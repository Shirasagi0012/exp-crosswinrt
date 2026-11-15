use std::collections::{HashMap, HashSet};
use std::path::Path;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, Item, Pat, TraitItem, TraitItemFn, Visibility};
use windows_metadata::reader::{File, Index, TypeCategory};
use windows_metadata::{HasAttributes, Value, trim_tick};

struct ClassInfo {
    namespace: String,
    name: String,
    has_default_ctor: bool,
    factory_interfaces: Vec<String>,
}

/// Generates `pub mod server { ... }` scaffolding (factory traits, `#[implement]` proxies,
/// and helper constructors) for all WinRT runtime classes present in `inputs` and `bindings_source`.
pub fn generate_server_module(
    inputs: &[impl AsRef<Path>],
    bindings_source: &str,
    shims_path: &str,
) -> String {
    let mut files = Vec::new();
    for input in inputs {
        collect_winmd_files(input.as_ref(), &mut files);
    }
    if files.is_empty() {
        return String::new();
    }

    let index = Index::new(files);
    let mut classes = collect_classes(&index);
    classes.sort_by(|a, b| {
        a.namespace
            .cmp(&b.namespace)
            .then_with(|| a.name.cmp(&b.name))
    });

    if classes.is_empty() {
        return String::new();
    }

    let Ok(parsed) = syn::parse_file(bindings_source) else {
        return String::new();
    };
    let (structs, impl_traits) = collect_bindings_symbols(&parsed);
    let shims: TokenStream = shims_path
        .parse()
        .unwrap_or_else(|e| panic!("Invalid shims_path '{shims_path}': {e}"));

    let mut class_blocks = Vec::new();
    for class in classes {
        if !structs.contains(&class.name) {
            continue;
        }
        if !class
            .factory_interfaces
            .iter()
            .all(|iface| impl_traits.contains_key(iface))
        {
            continue;
        }
        if !class.has_default_ctor && class.factory_interfaces.is_empty() {
            continue;
        }

        class_blocks.push(emit_class_server(&class, &impl_traits, &shims));
    }

    if class_blocks.is_empty() {
        return String::new();
    }

    let module = quote! {
        #[allow(non_snake_case, non_camel_case_types, dead_code, unused_imports)]
        pub mod server {
            use super::*;
            use #shims::{IActivationFactory, IActivationFactory_Impl, IActivationFactory_Vtbl};
            #(#class_blocks)*
        }
    };

    format!("\n{module}\n")
}

fn collect_winmd_files(path: &Path, out: &mut Vec<File>) {
    if path.is_file() {
        if let Some(file) = File::read(path) {
            out.push(file);
        }
    } else if path.is_dir()
        && let Ok(entries) = std::fs::read_dir(path)
    {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok().map(|entry| entry.path()))
            .collect();
        paths.sort();
        for entry in paths {
            if entry.is_file()
                && entry
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("winmd"))
                && let Some(file) = File::read(&entry)
            {
                out.push(file);
            }
        }
    }
}

fn collect_classes(index: &Index) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    for (namespace, name, def) in index.iter() {
        if def.category() != TypeCategory::Class {
            continue;
        }

        let mut has_default_ctor = false;
        let mut factory_interfaces = Vec::new();

        for attr in def.attributes() {
            match attr.name() {
                "ActivatableAttribute" => {
                    let mut iface_name = None;
                    for (_, arg) in attr.value() {
                        if let Value::TypeName(tn) = arg {
                            iface_name = Some(trim_tick(&tn.name).to_string());
                            break;
                        }
                    }
                    match iface_name {
                        Some(iface) => factory_interfaces.push(iface),
                        None => has_default_ctor = true,
                    }
                }
                "StaticAttribute" | "ComposableAttribute" => {
                    for (_, arg) in attr.value() {
                        if let Value::TypeName(tn) = arg {
                            factory_interfaces.push(trim_tick(&tn.name).to_string());
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        factory_interfaces.sort();
        factory_interfaces.dedup();

        classes.push(ClassInfo {
            namespace: namespace.to_string(),
            name: trim_tick(name).to_string(),
            has_default_ctor,
            factory_interfaces,
        });
    }
    classes
}

fn collect_bindings_symbols(
    file: &syn::File,
) -> (HashSet<String>, HashMap<String, Vec<TraitItemFn>>) {
    let mut structs = HashSet::new();
    let mut impl_traits = HashMap::new();

    for item in &file.items {
        match item {
            Item::Struct(s) if matches!(s.vis, Visibility::Public(_)) => {
                structs.insert(s.ident.to_string());
            }
            Item::Trait(t) if matches!(t.vis, Visibility::Public(_)) => {
                let name = t.ident.to_string();
                if let Some(iface) = name.strip_suffix("_Impl") {
                    let fns = t
                        .items
                        .iter()
                        .filter_map(|ti| match ti {
                            TraitItem::Fn(f) => Some(f.clone()),
                            _ => None,
                        })
                        .collect();
                    impl_traits.insert(iface.to_string(), fns);
                }
            }
            _ => {}
        }
    }

    (structs, impl_traits)
}

fn emit_class_server(
    class: &ClassInfo,
    impl_traits: &HashMap<String, Vec<TraitItemFn>>,
    shims: &TokenStream,
) -> TokenStream {
    let class_ident = syn::Ident::new(&class.name, Span::call_site());
    let factory_fn_ident = format_ident!("{}_factory", to_snake_case(&class.name));

    if class.factory_interfaces.is_empty() {
        return quote! {
            pub fn #factory_fn_ident<T>() -> #shims::IActivationFactory
            where
                T: Default + Into<super::#class_ident>,
            {
                #shims::activatable::<T, super::#class_ident>()
            }
        };
    }

    let factory_trait = format_ident!("{}_Factory", class.name);
    let proxy_ident = format_ident!("{}_FactoryProxy", class.name);
    let proxy_impl_ident = format_ident!("{}_FactoryProxy_Impl", class.name);

    let mut trait_methods = Vec::new();
    if class.has_default_ctor {
        trait_methods.push(quote! {
            fn ActivateInstance() -> ::windows_core::Result<super::#class_ident>;
        });
    }

    let mut seen_methods = HashSet::new();
    let mut iface_idents = Vec::new();
    let mut iface_impls = Vec::new();

    for iface_name in &class.factory_interfaces {
        let iface_ident = syn::Ident::new(iface_name, Span::call_site());
        let iface_impl_ident = format_ident!("{}_Impl", iface_name);
        iface_idents.push(iface_ident);

        let mut proxy_methods = Vec::new();
        for method in &impl_traits[iface_name] {
            let method_ident = &method.sig.ident;
            let mut static_sig = method.sig.clone();
            let mut arg_idents = Vec::new();
            let mut normalized_inputs = syn::punctuated::Punctuated::new();

            for (idx, input) in static_sig.inputs.iter().enumerate() {
                match input {
                    FnArg::Receiver(_) => {}
                    FnArg::Typed(pat_ty) => {
                        let mut pat_ty = pat_ty.clone();
                        let ident = match pat_ty.pat.as_ref() {
                            Pat::Ident(pi) => pi.ident.clone(),
                            _ => {
                                let fallback = format_ident!("__arg_{idx}");
                                pat_ty.pat = Box::new(Pat::Ident(syn::PatIdent {
                                    attrs: Vec::new(),
                                    by_ref: None,
                                    mutability: None,
                                    ident: fallback.clone(),
                                    subpat: None,
                                }));
                                fallback
                            }
                        };
                        arg_idents.push(ident);
                        normalized_inputs.push(FnArg::Typed(pat_ty));
                    }
                }
            }
            static_sig.inputs = normalized_inputs;

            if seen_methods.insert(method_ident.to_string()) {
                trait_methods.push(quote! {
                    #static_sig;
                });
            }

            let impl_sig = &method.sig;
            proxy_methods.push(quote! {
                #impl_sig {
                    <T as #factory_trait>::#method_ident(#(#arg_idents),*)
                }
            });
        }

        iface_impls.push(quote! {
            impl<T: #factory_trait + 'static> super::#iface_impl_ident for #proxy_impl_ident<T> {
                #(#proxy_methods)*
            }
        });
    }

    let activate_body = if class.has_default_ctor {
        quote! {
            Ok(<T as #factory_trait>::ActivateInstance()?.into())
        }
    } else {
        quote! {
            Err(::windows_core::Error::from_hresult(::windows_core::HRESULT(0x8000_4001u32 as i32)))
        }
    };

    quote! {
        pub trait #factory_trait: Sized {
            #(#trait_methods)*
        }

        #[::windows_core::implement(IActivationFactory, #(#iface_idents),*)]
        pub struct #proxy_ident<T>(core::marker::PhantomData<fn() -> T>)
        where
            T: #factory_trait + 'static;

        impl<T: #factory_trait + 'static> IActivationFactory_Impl for #proxy_impl_ident<T> {
            fn ActivateInstance(&self) -> ::windows_core::Result<::windows_core::IInspectable> {
                #activate_body
            }
        }

        #(#iface_impls)*

        pub fn #factory_fn_ident<T: #factory_trait + 'static>() -> #proxy_ident<T> {
            #proxy_ident(core::marker::PhantomData)
        }
    }
}

fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() {
            let prev_lower_or_digit = i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            let prev_upper = i > 0 && chars[i - 1].is_uppercase();
            if i > 0 && (prev_lower_or_digit || (prev_upper && next_lower)) {
                out.push('_');
            }
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
        } else {
            out.push(ch);
        }
    }
    out
}
