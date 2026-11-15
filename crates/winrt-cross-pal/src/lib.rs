#![allow(non_snake_case, non_camel_case_types)]

//! Cross-platform WinRT ABI platform abstraction layer (PAL).
//!
//! Provides the native WinRT C-ABI primitives (`HSTRING`, `CoTaskMemAlloc`/`CoTaskMemFree`,
//! MTA lifetime shims, restricted error stubs, and `RoGetActivationFactory` dispatch) required by
//! CsWinRT 3.0 (`WinRT.Runtime.dll`) when hosting Rust WinRT components on non-Windows OSes.
//!
//! In a `cdylib` crate implementing WinRT classes, invoke [`export_winrt_component!`] to expose the
//! standard PAL exports (`#[cfg(not(windows))]`) and factory activation entrypoints.

#[doc(hidden)]
pub mod __private {
    pub use windows_core;
}

/// Low-level ABI implementations for the WinRT platform surface.
pub mod abi {
    use core::ffi::c_void;
    use windows_core::{GUID, HRESULT, HSTRING, IUnknown, Interface};

    /// `E_POINTER` (`0x80004003`).
    pub const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);
    /// `E_INVALIDARG` (`0x80070057`).
    pub const E_INVALIDARG: HRESULT = HRESULT(0x8007_0057u32 as i32);
    /// `REGDB_E_CLASSNOTREG` (`0x80040154`).
    pub const REGDB_E_CLASSNOTREG: HRESULT = HRESULT(0x8004_0154u32 as i32);
    /// `S_OK` (`0x00000000`).
    pub const S_OK: HRESULT = HRESULT(0);
    /// `S_FALSE` (`0x00000001`).
    pub const S_FALSE: HRESULT = HRESULT(1);

    /// Standard COM/WinRT IID for `IActivationFactory` (`{00000035-0000-0000-C000-000000000046}`).
    pub const IACTIVATION_FACTORY_IID: GUID =
        GUID::from_u128(0x0000_0035_0000_0000_c000_0000_0000_0046);

    const HSTRING_REFERENCE_FLAG: u32 = 1;

    /// Binary-compatible header matching Windows `HSTRING_HEADER` for fast-pass reference strings.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct RawHStringHeader {
        pub flags: u32,
        pub length: u32,
        pub padding1: u32,
        pub padding2: u32,
        pub data: *const u16,
    }

    #[cfg(not(windows))]
    unsafe extern "C" {
        fn malloc(size: usize) -> *mut c_void;
        fn free(ptr: *mut c_void);
    }

    #[cfg(windows)]
    unsafe extern "system" {
        #[link_name = "CoTaskMemAlloc"]
        fn win_co_task_mem_alloc(cb: usize) -> *mut c_void;
        #[link_name = "CoTaskMemFree"]
        fn win_co_task_mem_free(pv: *mut c_void);
    }

    /// Allocates `cb` bytes from the COM task allocator (`malloc` on non-Windows).
    ///
    /// # Safety
    /// Caller owns the returned pointer and must release it with [`co_task_mem_free`].
    #[inline]
    pub unsafe extern "system" fn co_task_mem_alloc(cb: usize) -> *mut c_void {
        #[cfg(not(windows))]
        unsafe {
            malloc(cb.max(1))
        }
        #[cfg(windows)]
        unsafe {
            win_co_task_mem_alloc(cb)
        }
    }

    /// Frees memory previously allocated by [`co_task_mem_alloc`]. Null pointers are ignored.
    ///
    /// # Safety
    /// `pv` must be null or have been returned by [`co_task_mem_alloc`] / `CoTaskMemAlloc`.
    #[inline]
    pub unsafe extern "system" fn co_task_mem_free(pv: *mut c_void) {
        if !pv.is_null() {
            #[cfg(not(windows))]
            unsafe {
                free(pv);
            }
            #[cfg(windows)]
            unsafe {
                win_co_task_mem_free(pv);
            }
        }
    }

    /// Creates a heap-allocated, reference-counted `HSTRING` from a UTF-16 buffer.
    ///
    /// # Safety
    /// If `length > 0`, `source_string` must be valid for `length` `u16` elements.
    /// `string` must be a valid writable pointer.
    pub unsafe extern "system" fn windows_create_string(
        source_string: *const u16,
        length: u32,
        string: *mut *mut c_void,
    ) -> HRESULT {
        if string.is_null() {
            return E_INVALIDARG;
        }
        if length == 0 {
            unsafe { *string = core::ptr::null_mut() };
            return S_OK;
        }
        if source_string.is_null() {
            return E_POINTER;
        }
        let slice = unsafe { core::slice::from_raw_parts(source_string, length as usize) };
        let hstr = HSTRING::from_wide(slice);
        unsafe {
            *string = core::mem::transmute::<HSTRING, *mut c_void>(hstr);
        }
        S_OK
    }

    /// Initializes a stack/caller-allocated fast-pass `HSTRING` without heap allocation.
    ///
    /// # Safety
    /// `hstring_header` and `string` must be valid writable pointers, and `source_string`
    /// must outlive the resulting `HSTRING` reference when `length > 0`.
    pub unsafe extern "system" fn windows_create_string_reference(
        source_string: *const u16,
        length: u32,
        hstring_header: *mut RawHStringHeader,
        string: *mut *mut c_void,
    ) -> HRESULT {
        if string.is_null() || hstring_header.is_null() {
            return E_INVALIDARG;
        }
        if source_string.is_null() && length > 0 {
            return E_POINTER;
        }
        if length == 0 {
            unsafe { *string = core::ptr::null_mut() };
            return S_OK;
        }
        unsafe {
            hstring_header.write(RawHStringHeader {
                flags: HSTRING_REFERENCE_FLAG,
                length,
                padding1: 0,
                padding2: 0,
                data: source_string,
            });
            *string = hstring_header.cast();
        }
        S_OK
    }

    /// Decrements the reference count of a heap `HSTRING` and frees it if zero.
    /// Fast-pass reference strings and null handles are safely ignored.
    ///
    /// # Safety
    /// `string` must be null or a valid `HSTRING` handle.
    pub unsafe extern "system" fn windows_delete_string(string: *mut c_void) -> HRESULT {
        unsafe {
            let _hstr: HSTRING = core::mem::transmute(string);
        }
        S_OK
    }

    /// Returns a pointer to the UTF-16 buffer of `string` and optionally writes its unit length.
    ///
    /// # Safety
    /// `string` must be null or a valid `HSTRING` handle; `length`, if non-null, must be writable.
    pub unsafe extern "system" fn windows_get_string_raw_buffer(
        string: *mut c_void,
        length: *mut u32,
    ) -> *const u16 {
        let hstr: &HSTRING = unsafe { &*(&raw const string).cast::<HSTRING>() };
        if !length.is_null() {
            unsafe { *length = hstr.len() as u32 };
        }
        hstr.as_ptr()
    }

    /// Marks COM Multi-Threaded Apartment usage as active (stub returning a dummy cookie).
    ///
    /// # Safety
    /// `cookie`, if non-null, must be writable.
    pub unsafe extern "system" fn co_increment_mta_usage(cookie: *mut *mut c_void) -> HRESULT {
        if !cookie.is_null() {
            unsafe { *cookie = 1usize as *mut c_void };
        }
        S_OK
    }

    /// Releases an MTA usage cookie obtained from [`co_increment_mta_usage`].
    pub extern "system" fn co_decrement_mta_usage(_cookie: *mut c_void) -> HRESULT {
        S_OK
    }

    /// Records a language exception (no-op stub returning `TRUE`).
    pub extern "system" fn ro_originate_language_exception(
        _error: i32,
        _message: *mut c_void,
        _language_exception: *mut c_void,
    ) -> i32 {
        1
    }

    /// Queries the thread's restricted error info (returns `S_FALSE` with null out pointer).
    ///
    /// # Safety
    /// `pp_restricted_error_info`, if non-null, must be writable.
    pub unsafe extern "system" fn get_restricted_error_info(
        pp_restricted_error_info: *mut *mut c_void,
    ) -> HRESULT {
        if !pp_restricted_error_info.is_null() {
            unsafe { *pp_restricted_error_info = core::ptr::null_mut() };
        }
        S_FALSE
    }

    /// Sets the thread's restricted error info (no-op stub returning `S_OK`).
    pub extern "system" fn set_restricted_error_info(_p_restricted_error_info: *mut c_void) -> HRESULT {
        S_OK
    }

    /// COM `oleaut32!SetErrorInfo` stub returning `S_OK`.
    pub extern "system" fn set_error_info(_dw_reserved: u32, _perrinfo: *mut c_void) -> HRESULT {
        S_OK
    }

    /// Win32 `kernel32!FormatMessageW` stub for non-Windows hosts.
    ///
    /// Returning `0` signals no OS-localized string is available so `WinRT.Runtime` falls back
    /// cleanly to `WellKnownExceptionMappings` (`DivideByZeroException`, `ArgumentException`, etc.)
    /// without failing with `DllNotFoundException('kernel32.dll')`.
    ///
    /// # Safety
    /// If `lp_buffer` is non-null when `dw_flags & 0x0100 != 0` (`FORMAT_MESSAGE_ALLOCATE_BUFFER`),
    /// `*lp_buffer` is zeroed.
    pub unsafe extern "system" fn format_message_w(
        dw_flags: u32,
        _lp_source: *const c_void,
        _dw_message_id: u32,
        _dw_language_id: u32,
        lp_buffer: *mut *mut u16,
        _n_size: u32,
        _p_arguments: *const c_void,
    ) -> u32 {
        const FORMAT_MESSAGE_ALLOCATE_BUFFER: u32 = 0x0000_0100;
        if (dw_flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) != 0 && !lp_buffer.is_null() {
            unsafe { *lp_buffer = core::ptr::null_mut() };
        }
        0
    }

    /// Win32 `kernel32!LocalFree` counterpart forwarding to [`co_task_mem_free`].
    ///
    /// # Safety
    /// `h_mem` must be null or a pointer previously allocated by `malloc` / [`co_task_mem_alloc`].
    pub unsafe extern "system" fn local_free(h_mem: *mut c_void) -> *mut c_void {
        unsafe { co_task_mem_free(h_mem) };
        core::ptr::null_mut()
    }

    /// Shared activation dispatcher used by [`crate::export_winrt_factories!`] /
    /// [`crate::export_winrt_component!`].
    ///
    /// # Safety
    /// `activatable_class_id` must be null or a valid `HSTRING`; `iid` and `factory` must be
    /// valid pointers if non-null.
    pub unsafe fn dispatch_activation_factory<F>(
        activatable_class_id: *mut c_void,
        iid: *const GUID,
        factory: *mut *mut c_void,
        resolver: F,
    ) -> HRESULT
    where
        F: FnOnce(&HSTRING) -> Option<IUnknown>,
    {
        if factory.is_null() || iid.is_null() {
            return E_POINTER;
        }
        unsafe { *factory = core::ptr::null_mut() };

        let class_id: &HSTRING =
            unsafe { &*(&raw const activatable_class_id).cast::<HSTRING>() };
        match resolver(class_id) {
            Some(com_factory) => unsafe { com_factory.query(iid, factory) },
            None => REGDB_E_CLASSNOTREG,
        }
    }
}

/// Exports the standard non-Windows WinRT platform ABI functions (`HSTRING`, `CoTaskMem*`, MTA,
/// and `RestrictedErrorInfo` shims) from the invoking `cdylib` crate.
///
/// Guarded with `#[cfg(not(windows))]` so the same crate continues to link against `combase.dll`
/// and `ole32.dll` on Windows without symbol collisions.
#[macro_export]
macro_rules! export_winrt_pal {
    () => {
        #[cfg(not(windows))]
        mod __winrt_cross_pal_exports {
            use core::ffi::c_void;
            use $crate::__private::windows_core::HRESULT;

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn CoTaskMemAlloc(cb: usize) -> *mut c_void {
                unsafe { $crate::abi::co_task_mem_alloc(cb) }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn CoTaskMemFree(pv: *mut c_void) {
                unsafe { $crate::abi::co_task_mem_free(pv) }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn WindowsCreateString(
                source_string: *const u16,
                length: u32,
                string: *mut *mut c_void,
            ) -> HRESULT {
                unsafe { $crate::abi::windows_create_string(source_string, length, string) }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn WindowsCreateStringReference(
                source_string: *const u16,
                length: u32,
                hstring_header: *mut $crate::abi::RawHStringHeader,
                string: *mut *mut c_void,
            ) -> HRESULT {
                unsafe {
                    $crate::abi::windows_create_string_reference(
                        source_string,
                        length,
                        hstring_header,
                        string,
                    )
                }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn WindowsDeleteString(string: *mut c_void) -> HRESULT {
                unsafe { $crate::abi::windows_delete_string(string) }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn WindowsGetStringRawBuffer(
                string: *mut c_void,
                length: *mut u32,
            ) -> *const u16 {
                unsafe { $crate::abi::windows_get_string_raw_buffer(string, length) }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn CoIncrementMTAUsage(
                cookie: *mut *mut c_void,
            ) -> HRESULT {
                unsafe { $crate::abi::co_increment_mta_usage(cookie) }
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn CoDecrementMTAUsage(cookie: *mut c_void) -> HRESULT {
                $crate::abi::co_decrement_mta_usage(cookie)
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn RoOriginateLanguageException(
                error: i32,
                message: *mut c_void,
                language_exception: *mut c_void,
            ) -> i32 {
                $crate::abi::ro_originate_language_exception(error, message, language_exception)
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn GetRestrictedErrorInfo(
                pp_restricted_error_info: *mut *mut c_void,
            ) -> HRESULT {
                unsafe { $crate::abi::get_restricted_error_info(pp_restricted_error_info) }
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn SetRestrictedErrorInfo(
                p_restricted_error_info: *mut c_void,
            ) -> HRESULT {
                $crate::abi::set_restricted_error_info(p_restricted_error_info)
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn SetErrorInfo(
                dw_reserved: u32,
                perrinfo: *mut c_void,
            ) -> HRESULT {
                $crate::abi::set_error_info(dw_reserved, perrinfo)
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn FormatMessageW(
                dw_flags: u32,
                lp_source: *const c_void,
                dw_message_id: u32,
                dw_language_id: u32,
                lp_buffer: *mut *mut u16,
                n_size: u32,
                p_arguments: *const c_void,
            ) -> u32 {
                unsafe {
                    $crate::abi::format_message_w(
                        dw_flags,
                        lp_source,
                        dw_message_id,
                        dw_language_id,
                        lp_buffer,
                        n_size,
                        p_arguments,
                    )
                }
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn LocalFree(h_mem: *mut c_void) -> *mut c_void {
                unsafe { $crate::abi::local_free(h_mem) }
            }
        }
    };
}

/// Exports `DllGetActivationFactory` (all targets) and `RoGetActivationFactory` (`#[cfg(not(windows))]`)
/// dispatching each WinRT runtime class name to its Rust factory expression.
///
/// ```rust,ignore
/// winrt_cross_pal::export_winrt_factories! {
///     "MyComponent.RustEngine" => RustEngineFactory,
/// }
/// ```
#[macro_export]
macro_rules! export_winrt_factories {
    ($($class_id:expr => $factory:expr),* $(,)?) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn DllGetActivationFactory(
            activatable_class_id: *mut core::ffi::c_void,
            factory: *mut *mut core::ffi::c_void,
        ) -> $crate::__private::windows_core::HRESULT {
            unsafe {
                $crate::abi::dispatch_activation_factory(
                    activatable_class_id,
                    &$crate::abi::IACTIVATION_FACTORY_IID,
                    factory,
                    |class_id| {
                        $(
                            if class_id == $class_id {
                                let com_factory: $crate::__private::windows_core::IUnknown =
                                    ($factory).into();
                                return Some(com_factory);
                            }
                        )*
                        let _ = class_id;
                        None
                    },
                )
            }
        }

        #[cfg(not(windows))]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn RoGetActivationFactory(
            activatable_class_id: *mut core::ffi::c_void,
            iid: *const $crate::__private::windows_core::GUID,
            factory: *mut *mut core::ffi::c_void,
        ) -> $crate::__private::windows_core::HRESULT {
            unsafe {
                $crate::abi::dispatch_activation_factory(
                    activatable_class_id,
                    iid,
                    factory,
                    |class_id| {
                        $(
                            if class_id == $class_id {
                                let com_factory: $crate::__private::windows_core::IUnknown =
                                    ($factory).into();
                                return Some(com_factory);
                            }
                        )*
                        let _ = class_id;
                        None
                    },
                )
            }
        }
    };
}

/// Convenience macro combining [`export_winrt_pal!`] and [`export_winrt_factories!`].
#[macro_export]
macro_rules! export_winrt_component {
    ($($class_id:expr => $factory:expr),* $(,)?) => {
        $crate::export_winrt_pal!();
        $crate::export_winrt_factories! {
            $($class_id => $factory),*
        }
    };
}

#[cfg(test)]
mod tests {
    use super::abi::*;
    use core::ffi::c_void;
    use core::ptr;
    use windows_core::{GUID, HSTRING, IUnknown, Interface, implement};

    #[implement]
    struct DummyFactory;

    #[test]
    fn hstring_heap_roundtrip_and_empty() {
        unsafe {
            let mut handle: *mut c_void = ptr::null_mut();
            let wide: Vec<u16> = "Hello WinRT!".encode_utf16().collect();
            assert_eq!(
                windows_create_string(wide.as_ptr(), wide.len() as u32, &mut handle),
                S_OK
            );
            assert!(!handle.is_null());

            let mut out_len: u32 = 0;
            let raw_ptr = windows_get_string_raw_buffer(handle, &mut out_len);
            assert_eq!(out_len as usize, wide.len());
            let read_back = core::slice::from_raw_parts(raw_ptr, out_len as usize);
            assert_eq!(read_back, wide.as_slice());

            assert_eq!(windows_delete_string(handle), S_OK);

            // Empty string produces null handle
            let mut empty: *mut c_void = 1usize as *mut c_void;
            assert_eq!(windows_create_string(ptr::null(), 0, &mut empty), S_OK);
            assert!(empty.is_null());
            assert_eq!(windows_delete_string(empty), S_OK);
        }
    }

    #[test]
    fn hstring_fast_pass_reference_does_not_free_borrowed_buffer() {
        unsafe {
            let wide: Vec<u16> = "FastPass".encode_utf16().collect();
            let mut header = RawHStringHeader {
                flags: 0,
                length: 0,
                padding1: 0,
                padding2: 0,
                data: ptr::null(),
            };
            let mut handle: *mut c_void = ptr::null_mut();
            assert_eq!(
                windows_create_string_reference(
                    wide.as_ptr(),
                    wide.len() as u32,
                    &mut header,
                    &mut handle
                ),
                S_OK
            );
            assert_eq!(handle, (&raw mut header).cast());
            assert_eq!(header.flags, 1);

            let mut len = 0u32;
            let buf = windows_get_string_raw_buffer(handle, &mut len);
            assert_eq!(len as usize, wide.len());
            assert_eq!(buf, wide.as_ptr());

            // Must not crash or free stack header/Vec buffer.
            assert_eq!(windows_delete_string(handle), S_OK);
        }
    }

    #[test]
    fn co_task_mem_alloc_and_free() {
        unsafe {
            let ptr = co_task_mem_alloc(64) as *mut u8;
            assert!(!ptr.is_null());
            ptr.write_bytes(0xAB, 64);
            co_task_mem_free(ptr.cast());
            co_task_mem_free(ptr::null_mut());
        }
    }

    #[test]
    fn activation_dispatch_resolves_and_rejects_unknown() {
        unsafe {
            let class_ok = HSTRING::from("Demo.TestClass");
            let class_raw: *mut c_void = core::mem::transmute_copy(&class_ok);
            let mut out: *mut c_void = ptr::null_mut();

            let hr = dispatch_activation_factory(
                class_raw,
                &IUnknown::IID,
                &mut out,
                |id| (id == "Demo.TestClass").then(|| DummyFactory.into()),
            );
            assert_eq!(hr, S_OK);
            assert!(!out.is_null());
            drop(IUnknown::from_raw(out));

            let class_missing = HSTRING::from("Demo.Missing");
            let missing_raw: *mut c_void = core::mem::transmute_copy(&class_missing);
            let mut out_missing: *mut c_void = 1usize as *mut c_void;
            let hr_missing = dispatch_activation_factory(
                missing_raw,
                &GUID::zeroed(),
                &mut out_missing,
                |id| (id == "Demo.TestClass").then(|| DummyFactory.into()),
            );
            assert_eq!(hr_missing, REGDB_E_CLASSNOTREG);
            assert!(out_missing.is_null());
        }
    }
}
