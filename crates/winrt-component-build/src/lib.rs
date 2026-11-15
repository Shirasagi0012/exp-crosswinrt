//! Build-script helper wrapping `windows-rdl` and `windows-bindgen` to compile `.rdl` contracts
//! into `.winmd` metadata (optional) and emit patched WinRT server bindings for `winrt-server-shims`
//! on non-Windows targets.

use std::path::{Path, PathBuf};

mod server_gen;

/// Builder for generating `.winmd` metadata (from `.rdl`) and Rust WinRT bindings in `build.rs`.
///
/// Defaults:
/// - `input_default = true`
/// - `flat = true`, `minimal = true`
/// - `output = "src/bindings.rs"`
/// - `shims_path = "::winrt_server_shims"`
/// - `patch_array = true`, `patch_factory_cache = true`, `server = true`
pub struct WinrtComponentBindgen {
    rdl_inputs: Vec<PathBuf>,
    rdl_references: Vec<PathBuf>,
    rdl_output_winmd: Option<PathBuf>,
    inputs: Vec<PathBuf>,
    input_default: bool,
    output: PathBuf,
    filters: Vec<String>,
    implements: Vec<String>,
    flat: bool,
    minimal: bool,
    shims_path: String,
    future_path: String,
    patch_array: bool,
    patch_factory_cache: bool,
    patch_future: bool,
    generate_server: bool,
    emit_rerun_if_changed: bool,
}

impl Default for WinrtComponentBindgen {
    fn default() -> Self {
        Self::new()
    }
}

impl WinrtComponentBindgen {
    /// Creates a new builder configured with standard WinRT server-component defaults.
    pub fn new() -> Self {
        Self {
            rdl_inputs: Vec::new(),
            rdl_references: Vec::new(),
            rdl_output_winmd: None,
            inputs: Vec::new(),
            input_default: true,
            output: PathBuf::from("src/bindings.rs"),
            filters: Vec::new(),
            implements: Vec::new(),
            flat: true,
            minimal: true,
            shims_path: "::winrt_server_shims".to_string(),
            future_path: "::winrt_future".to_string(),
            patch_array: true,
            patch_factory_cache: true,
            patch_future: true,
            generate_server: true,
            emit_rerun_if_changed: true,
        }
    }

    /// Compiles an `.rdl` contract file into `out_winmd` via `windows-rdl`, automatically feeding
    /// the resulting `.winmd` into `windows-bindgen` and emitting `cargo:rerun-if-changed`.
    pub fn rdl(mut self, rdl_path: impl AsRef<Path>, out_winmd: impl AsRef<Path>) -> Self {
        self.rdl_inputs.push(rdl_path.as_ref().to_path_buf());
        self.rdl_output_winmd = Some(out_winmd.as_ref().to_path_buf());
        self
    }

    /// Adds an extra `.rdl` source file to compile alongside [`Self::rdl`].
    pub fn rdl_input(mut self, rdl_path: impl AsRef<Path>) -> Self {
        self.rdl_inputs.push(rdl_path.as_ref().to_path_buf());
        self
    }

    /// Adds an external `.winmd` reference for RDL compilation (in addition to `reference_default()`).
    pub fn rdl_reference(mut self, winmd_path: impl AsRef<Path>) -> Self {
        self.rdl_references.push(winmd_path.as_ref().to_path_buf());
        self
    }

    /// Adds a `.winmd` metadata file or directory input.
    pub fn input(mut self, path: impl AsRef<Path>) -> Self {
        self.inputs.push(path.as_ref().to_path_buf());
        self
    }

    /// Controls whether `windows_bindgen::Bindgen::input_default()` and
    /// `windows_rdl::Reader::reference_default()` are called (default: `true`).
    pub fn input_default(mut self, enabled: bool) -> Self {
        self.input_default = enabled;
        self
    }

    /// Sets the generated Rust output path (default: `src/bindings.rs`).
    pub fn output(mut self, path: impl AsRef<Path>) -> Self {
        self.output = path.as_ref().to_path_buf();
        self
    }

    /// Adds a `windows-bindgen` filter rule (e.g. `"MyComponent"` or `"!MyComponent.Foo"`).
    pub fn filter(mut self, expr: impl Into<String>) -> Self {
        self.filters.push(expr.into());
        self
    }

    /// Adds a `windows-bindgen` implement rule (e.g. `"MyComponent"`).
    pub fn implement(mut self, expr: impl Into<String>) -> Self {
        self.implements.push(expr.into());
        self
    }

    /// Controls flat layout (default: `true`).
    pub fn flat(mut self, enabled: bool) -> Self {
        self.flat = enabled;
        self
    }

    /// Controls minimal bindings mode (default: `true`).
    pub fn minimal(mut self, enabled: bool) -> Self {
        self.minimal = enabled;
        self
    }

    /// Sets the Rust path prefix used when rewriting `WinRtArray` and `DummyFactoryCache`
    /// (default: `"::winrt_server_shims"`).
    pub fn shims_path(mut self, path: impl Into<String>) -> Self {
        self.shims_path = path.into();
        self
    }

    /// Controls replacing `windows_core::Array` with `<shims_path>::WinRtArray` (default: `true`).
    pub fn patch_array(mut self, enabled: bool) -> Self {
        self.patch_array = enabled;
        self
    }

    /// Controls replacing `windows_core::imp::FactoryCache` with `<shims_path>::DummyFactoryCache`
    /// (default: `true`).
    pub fn patch_factory_cache(mut self, enabled: bool) -> Self {
        self.patch_factory_cache = enabled;
        self
    }

    /// Sets the Rust path prefix used when rewriting `windows_future::` (default: `"::winrt_future"`).
    pub fn future_path(mut self, path: impl Into<String>) -> Self {
        self.future_path = path.into();
        self
    }

    /// Controls replacing `windows_future::` with `<future_path>::` (default: `true`).
    pub fn patch_future(mut self, enabled: bool) -> Self {
        self.patch_future = enabled;
        self
    }

    /// Controls generating `pub mod server` factory traits/proxies for runtime classes
    /// when `flat = true` and `implement(...)` rules are configured (default: `true`).
    pub fn server(mut self, enabled: bool) -> Self {
        self.generate_server = enabled;
        self
    }

    /// Controls emitting `cargo:rerun-if-changed` directives for input `.rdl`/`.winmd` files (default: `true`).
    pub fn emit_rerun_if_changed(mut self, enabled: bool) -> Self {
        self.emit_rerun_if_changed = enabled;
        self
    }

    /// Compiles `.rdl` inputs (if configured), executes `windows-bindgen`, applies configured
    /// server-shim patches, and writes the output file.
    pub fn generate(mut self) {
        if let Some(winmd_out) = self.rdl_output_winmd.take() {
            if self.emit_rerun_if_changed {
                for rdl in &self.rdl_inputs {
                    println!("cargo:rerun-if-changed={}", rdl.display());
                }
                for rdl_ref in &self.rdl_references {
                    println!("cargo:rerun-if-changed={}", rdl_ref.display());
                }
            }

            compile_rdl(
                &self.rdl_inputs,
                &self.rdl_references,
                self.input_default,
                &winmd_out,
            );
            self.inputs.push(winmd_out);
        }

        if self.emit_rerun_if_changed {
            for input in &self.inputs {
                println!("cargo:rerun-if-changed={}", input.display());
            }
        }

        let mut bindgen = windows_bindgen::Bindgen::new();
        for input in &self.inputs {
            bindgen.input(input);
        }
        if self.input_default {
            bindgen.input_default();
        }
        bindgen.output(&self.output);
        if self.flat {
            bindgen.flat();
        }
        if self.minimal {
            bindgen.minimal();
        }
        for filter in &self.filters {
            bindgen.filter(filter);
        }
        for implement in &self.implements {
            bindgen.implement(implement);
        }
        bindgen.write();

        if self.patch_array || self.patch_factory_cache || self.patch_future || self.generate_server {
            let content = std::fs::read_to_string(&self.output).unwrap_or_else(|e| {
                panic!(
                    "Failed to read generated WinRT bindings '{}': {e}",
                    self.output.display()
                )
            });
            let mut updated = if self.patch_array || self.patch_factory_cache {
                apply_shims_patches(
                    &content,
                    &self.shims_path,
                    self.patch_array,
                    self.patch_factory_cache,
                )
            } else {
                content.clone()
            };
            if self.patch_future {
                updated = apply_future_patch(&updated, &self.future_path);
            }
            if self.generate_server && self.flat && !self.implements.is_empty() {
                let server_mod =
                    server_gen::generate_server_module(&self.inputs, &updated, &self.shims_path);
                if !server_mod.is_empty() {
                    updated.push_str(&server_mod);
                }
            }
            if updated != content {
                std::fs::write(&self.output, updated).unwrap_or_else(|e| {
                    panic!(
                        "Failed to write patched WinRT bindings '{}': {e}",
                        self.output.display()
                    )
                });
            }
        }
    }
}

/// Compiles one or more `.rdl` contract files into a `.winmd` metadata file via `windows-rdl`.
pub fn compile_rdl<P1: AsRef<Path>, P2: AsRef<Path>>(
    rdl_inputs: &[P1],
    references: &[P2],
    reference_default: bool,
    out_winmd: impl AsRef<Path>,
) {
    let out_winmd = out_winmd.as_ref();
    if let Some(parent) = out_winmd.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            panic!(
                "Failed to create WinMD output directory '{}': {e}",
                parent.display()
            )
        });
    }

    let mut reader = windows_rdl::reader();
    for rdl in rdl_inputs {
        reader.input(rdl);
    }
    for reference in references {
        reader.reference(reference);
    }
    if reference_default {
        reader.reference_default();
    }
    reader.output(out_winmd);
    if let Err(err) = reader.write() {
        panic!("Failed to compile RDL to '{}': {err}", out_winmd.display());
    }
}

/// Applies non-Windows server-shim type substitutions to generated `windows-bindgen` source text.
pub fn apply_shims_patches(
    source: &str,
    shims_path: &str,
    patch_array: bool,
    patch_factory_cache: bool,
) -> String {
    let mut out = source.to_string();
    if patch_array {
        let replacement = format!("{shims_path}::WinRtArray");
        out = out.replace("windows_core::Array", &replacement);
    }
    if patch_factory_cache {
        let replacement = format!("{shims_path}::DummyFactoryCache");
        out = out.replace("windows_core::imp::FactoryCache", &replacement);
    }
    out
}

/// Rewrites `windows_future::` references in generated `windows-bindgen` source text
/// to the configured cross-platform future crate path (e.g. `"::winrt_future"`).
pub fn apply_future_patch(source: &str, future_path: &str) -> String {
    let replacement = format!("{future_path}::");
    source.replace("windows_future::", &replacement)
}

#[cfg(test)]
mod tests {
    use super::{apply_future_patch, apply_shims_patches, compile_rdl};

    #[test]
    fn patches_array_and_factory_cache() {
        let input = "\
            static SHARED: windows_core::imp::FactoryCache<Foo, IFoo> = windows_core::imp::FactoryCache::new();\n\
            pub fn Gen(&self) -> windows_core::Result<windows_core::Array<u8>> {}\n\
            let x = windows_core::Array::<u8>::from_raw_parts(p, n);\n";

        let patched = apply_shims_patches(input, "::winrt_server_shims", true, true);
        assert!(!patched.contains("windows_core::imp::FactoryCache"));
        assert!(!patched.contains("windows_core::Array"));
        assert!(patched.contains("::winrt_server_shims::DummyFactoryCache::new()"));
        assert!(patched.contains("::winrt_server_shims::WinRtArray::<u8>::from_raw_parts"));

        let future_src = "fn Run(&self) -> windows_core::Result<windows_future::IAsyncOperation<i32>>";
        let future_patched = apply_future_patch(future_src, "::winrt_future");
        assert!(!future_patched.contains("windows_future::"));
        assert!(future_patched.contains("::winrt_future::IAsyncOperation<i32>"));
    }

    #[test]
    fn compiles_rdl_winrt_contract_to_winmd() {
        let temp_dir = std::env::temp_dir().join("winrt_component_build_rdl_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let rdl_path = temp_dir.join("test.rdl");
        let winmd_path = temp_dir.join("TestDemo.winmd");
        std::fs::write(
            &rdl_path,
            r#"
use Windows::Foundation::Metadata::*;

#[winrt]
mod TestDemo {
    #[guid(auto)]
    #[ExclusiveTo(Counter)]
    interface ICounter {
        fn Add(&self, #[in] amount: i32) -> i32;
    }

    #[Activatable(1)]
    class Counter {
        ICounter,
    }
}
"#,
        )
        .unwrap();

        compile_rdl(&[&rdl_path], &[] as &[&std::path::Path], true, &winmd_path);
        assert!(winmd_path.exists());
        assert!(std::fs::metadata(&winmd_path).unwrap().len() > 0);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn generates_server_module_for_default_and_factory_classes() {
        let temp_dir = std::env::temp_dir().join("winrt_component_build_server_gen_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let rdl_path = temp_dir.join("server_test.rdl");
        let winmd_path = temp_dir.join("ServerDemo.winmd");
        let bindings_path = temp_dir.join("bindings.rs");
        std::fs::write(
            &rdl_path,
            r#"
use Windows::Foundation::Metadata::*;

#[winrt]
mod ServerDemo {
    #[guid(auto)]
    #[ExclusiveTo(Simple)]
    interface ISimple {
        fn Ping(&self) -> i32;
    }

    #[Activatable(1)]
    class Simple {
        ISimple,
    }

    #[guid(auto)]
    #[ExclusiveTo(Fancy)]
    interface IFancy {
        fn Value(&self) -> i32;
    }

    #[guid(auto)]
    #[ExclusiveTo(Fancy)]
    interface IFancyFactory {
        fn WithValue(&self, #[in] val: i32) -> Fancy;
    }

    #[guid(auto)]
    #[ExclusiveTo(Fancy)]
    interface IFancyStatics {
        fn DoubleIt(&self, #[in] val: i32) -> i32;
    }

    #[Activatable(1)]
    #[Activatable(IFancyFactory, 1)]
    #[Static(IFancyStatics, 1)]
    class Fancy {
        IFancy,
    }
}
"#,
        )
        .unwrap();

        super::WinrtComponentBindgen::new()
            .rdl(&rdl_path, &winmd_path)
            .output(&bindings_path)
            .filter("ServerDemo")
            .implement("ServerDemo")
            .emit_rerun_if_changed(false)
            .generate();

        let generated = std::fs::read_to_string(&bindings_path).unwrap();
        assert!(generated.contains("pub mod server"));
        assert!(generated.contains("pub fn simple_factory"));
        assert!(generated.contains("activatable :: < T , super :: Simple >"));
        assert!(generated.contains("pub trait Fancy_Factory"));
        assert!(generated.contains("fn ActivateInstance ()"));
        assert!(generated.contains("fn WithValue (val : i32)"));
        assert!(generated.contains("fn DoubleIt (val : i32)"));
        assert!(generated.contains("pub struct Fancy_FactoryProxy"));
        assert!(generated.contains("pub fn fancy_factory"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
