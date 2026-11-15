#![allow(non_snake_case, non_camel_case_types)]

//! Server-side WinRT ABI shims missing from `windows-core` on non-Windows targets
//! (`IActivationFactory`, [`WinRtArray`], and [`DummyFactoryCache`]).

use core::marker::PhantomData;
use core::ptr;
use windows_core::{Error, GUID, HRESULT, IInspectable, IInspectable_Vtbl, IUnknown, IUnknownImpl, Interface, Result, RuntimeName};
use winrt_cross_pal::abi::{co_task_mem_alloc, co_task_mem_free};

const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007_000Eu32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x8000_4001u32 as i32);

windows_core::imp::define_interface!(
    IActivationFactory,
    IActivationFactory_Vtbl,
    0x00000035_0000_0000_c000_000000000046,
    inspectable
);
windows_core::imp::interface_hierarchy!(IActivationFactory, IUnknown, IInspectable);

#[repr(C)]
pub struct IActivationFactory_Vtbl {
    pub base__: IInspectable_Vtbl,
    pub ActivateInstance: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> HRESULT,
}

/// Implementation trait for WinRT class activation factories (`IActivationFactory::ActivateInstance`).
pub trait IActivationFactory_Impl: IUnknownImpl {
    fn ActivateInstance(&self) -> Result<IInspectable>;
}

impl IActivationFactory_Vtbl {
    pub const fn new<Identity: IActivationFactory_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn ActivateInstance<
            Identity: IActivationFactory_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            instance: *mut *mut core::ffi::c_void,
        ) -> HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                match IActivationFactory_Impl::ActivateInstance(this) {
                    Ok(ok__) => {
                        instance.cast::<IInspectable>().write(ok__);
                        HRESULT(0)
                    }
                    Err(err) => {
                        instance.write_bytes(0, 1);
                        err.into()
                    }
                }
            }
        }
        Self {
            base__: IInspectable_Vtbl::new::<Identity, OFFSET>(),
            ActivateInstance: ActivateInstance::<Identity, OFFSET>,
        }
    }

    pub fn matches(iid: &GUID) -> bool {
        iid == &<IActivationFactory as Interface>::IID
    }
}

impl RuntimeName for IActivationFactory {}

#[windows_core::implement(IActivationFactory)]
struct Activatable(fn() -> Result<IInspectable>);

impl IActivationFactory_Impl for Activatable_Impl {
    fn ActivateInstance(&self) -> Result<IInspectable> {
        (self.0)()
    }
}

/// Creates a lightweight [`IActivationFactory`] for a WinRT class whose default constructor
/// maps `Impl::default()` into `Class` and upcasts to `IInspectable`.
pub fn activatable<Impl, Class>() -> IActivationFactory
where
    Impl: Default + Into<Class>,
    Class: Into<IInspectable>,
{
    Activatable(|| {
        let instance: Class = Impl::default().into();
        Ok(instance.into())
    })
    .into()
}

/// No-op placeholder for `windows_core::imp::FactoryCache` referenced by `windows-bindgen`
/// when emitting client stubs for classes with static/factory interfaces.
pub struct DummyFactoryCache<C, I>(PhantomData<(C, I)>);

unsafe impl<C, I> Sync for DummyFactoryCache<C, I> {}

impl<C, I> Default for DummyFactoryCache<C, I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C, I> DummyFactoryCache<C, I> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }

    /// Server-only components do not activate external factories through this cache.
    pub fn call<R, F: FnOnce(&I) -> Result<R>>(&self, _callback: F) -> Result<R> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

/// Ownership wrapper for CoTaskMem-allocated WinRT `out T[]` (`ReceiveArray`) buffers,
/// replacing `windows_core::Array<T>` on targets where `windows-core` omits it.
pub struct WinRtArray<T> {
    data: *mut T,
    len: u32,
}

impl<T> Default for WinRtArray<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T> WinRtArray<T> {
    /// Creates an empty array without allocating.
    pub const fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }

    /// Wraps an existing CoTaskMem-allocated buffer and element length.
    ///
    /// # Safety
    /// `data` must be null (when `len == 0`) or allocated via `CoTaskMemAlloc` with room for
    /// `len` initialized `T` elements.
    pub unsafe fn from_raw_parts(data: *mut T, len: u32) -> Self {
        Self { data, len }
    }

    /// Returns the number of elements in the array.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns `true` if the array contains no elements.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Views the array contents as a slice.
    pub fn as_slice(&self) -> &[T] {
        if self.data.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.data, self.len as usize) }
        }
    }

    /// Transfers ownership of the underlying CoTaskMem buffer to the WinRT ABI caller.
    pub fn into_abi(self) -> (*mut T, u32) {
        let abi = (self.data, self.len);
        core::mem::forget(self);
        abi
    }
}

impl<T: Copy> WinRtArray<T> {
    /// Allocates a CoTaskMem-backed array copied from `slice`.
    pub fn from_slice(slice: &[T]) -> Result<Self> {
        if slice.is_empty() {
            return Ok(Self::empty());
        }
        let len_u32 = u32::try_from(slice.len()).map_err(|_| Error::from_hresult(E_OUTOFMEMORY))?;
        let byte_len = slice
            .len()
            .checked_mul(core::mem::size_of::<T>())
            .ok_or_else(|| Error::from_hresult(E_OUTOFMEMORY))?;
        let ptr = unsafe { co_task_mem_alloc(byte_len) as *mut T };
        if ptr.is_null() {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        unsafe {
            ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            Ok(Self::from_raw_parts(ptr, len_u32))
        }
    }

    /// Allocates a CoTaskMem-backed array of `len` elements initialized to `value`.
    pub fn alloc_filled(len: usize, value: T) -> Result<Self> {
        if len == 0 {
            return Ok(Self::empty());
        }
        let len_u32 = u32::try_from(len).map_err(|_| Error::from_hresult(E_OUTOFMEMORY))?;
        let byte_len = len
            .checked_mul(core::mem::size_of::<T>())
            .ok_or_else(|| Error::from_hresult(E_OUTOFMEMORY))?;
        let ptr = unsafe { co_task_mem_alloc(byte_len) as *mut T };
        if ptr.is_null() {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        unsafe {
            for i in 0..len {
                ptr.add(i).write(value);
            }
            Ok(Self::from_raw_parts(ptr, len_u32))
        }
    }
}

impl<T> Drop for WinRtArray<T> {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe {
                co_task_mem_free(self.data.cast());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_core::implement;

    #[implement(IActivationFactory)]
    struct TestFactory;

    impl IActivationFactory_Impl for TestFactory_Impl {
        fn ActivateInstance(&self) -> Result<IInspectable> {
            Ok(TestFactory.into())
        }
    }

    #[test]
    fn array_from_slice_and_into_abi_roundtrip() {
        let arr = WinRtArray::from_slice(&[10u32, 20, 30, 40]).unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr.as_slice(), &[10, 20, 30, 40]);

        let (raw, len) = arr.into_abi();
        assert_eq!(len, 4);
        unsafe {
            let reclaimed = WinRtArray::from_raw_parts(raw, len);
            assert_eq!(reclaimed.as_slice(), &[10, 20, 30, 40]);
        }
    }

    #[test]
    fn array_alloc_filled_and_empty() {
        let empty = WinRtArray::<u8>::from_slice(&[]).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.as_slice(), &[]);

        let filled = WinRtArray::alloc_filled(16, 42u8).unwrap();
        assert_eq!(filled.as_slice(), &[42u8; 16]);
    }

    #[test]
    fn activation_factory_implements_and_activates() {
        let factory: IActivationFactory = TestFactory.into();
        let instance = factory.vtable().ActivateInstance;
        let mut raw = ptr::null_mut();
        let hr = unsafe { instance(factory.as_raw(), &mut raw) };
        assert_eq!(hr, HRESULT(0));
        assert!(!raw.is_null());
        unsafe { drop(IInspectable::from_raw(raw)) };
    }

    #[derive(Default)]
    struct DummyClassImpl;

    impl From<DummyClassImpl> for IInspectable {
        fn from(_: DummyClassImpl) -> Self {
            TestFactory.into()
        }
    }

    #[test]
    fn activatable_helper_creates_default_instance() {
        let factory = activatable::<DummyClassImpl, IInspectable>();
        let instance = factory.vtable().ActivateInstance;
        let mut raw = ptr::null_mut();
        let hr = unsafe { instance(factory.as_raw(), &mut raw) };
        assert_eq!(hr, HRESULT(0));
        assert!(!raw.is_null());
        unsafe { drop(IInspectable::from_raw(raw)) };
    }
}
