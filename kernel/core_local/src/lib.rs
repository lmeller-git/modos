#![no_std]

extern crate alloc;

use core::{alloc::Layout, marker::PhantomData, mem::MaybeUninit, ptr};

// Cpu local data storage. Relies on corret setup from arch.
//
// Layout:
// each core stores its core local info in GsBase. This includes arch specific info right now.
// The layout will be GsBase [arch --- ... --- core_local]. core_local here simply stores an array of integers, more complex types can be stored behind atomicptrs or similar.
// This requires lazy init for comple types.
// Setup is: a core_local is a static in core_local link section. At core init, this will e copied per cpu after its respective GsBase. We will have to ensure here that offsets to GsBase are coherent across all cores (and across linking -> setup), as offsets are computed at comp/link time.
// So to setup a core_local we query its GsBase addr, add the offset and return a ptr to that location.
// We must ensure though that this ptr stays on this core, i.e. suppsoe task k on core l queries some core_local var, but then switches cores (scheduler/await/...). It is now in general unsafe to use this ref.
// So we should guard this access somehow... Maybe core pinning to tell schedulers "this task must stay here for now"? Not clear yet, as we do not have a scheduler. right now every core has its own "thread" anyways, soe no rescheduling can happen currently.
pub use core_local_macros::core_local;

#[cfg(target_arch = "x86_64")]
type CpuDataProvider = arch_x86_64_primitives::Primitive;
#[cfg(not(target_arch = "x86_64"))]
struct CpuDataProvider;

#[doc(hidden)]
pub mod __private {
    use core::cell::UnsafeCell;

    #[repr(transparent)]
    pub struct CoreLocalStorage<T>(UnsafeCell<T>);

    // SAFETY: This is only accessed via controlled core_local structs.
    unsafe impl<T> Sync for CoreLocalStorage<T> {}

    impl<T> CoreLocalStorage<T> {
        pub const fn new(val: T) -> Self {
            Self(UnsafeCell::new(val))
        }
    }
}

use __private::CoreLocalStorage;
use arch_interfaces::cpu::CpuDataCapability;

unsafe extern "C" {
    static __cpu_local_start: u8;
    static __cpu_local_end: u8;
}

pub struct CoreLocal<T> {
    // TODO:
    // we could circumvent storing this ptr by generating one type/method per core_local variable and just inlinign &raw const STATIC at the call site.
    // Makes the whole thing a bit more complex though and might need some trait plumbing.
    template: *const CoreLocalStorage<T>,
    _phantom: PhantomData<T>,
}

// SAFETY: The ptr here is never actually dereferenced. It is just used for offset calculation and access are redirected to core_local seciton.
unsafe impl<T> Sync for CoreLocal<T> {}

impl<T> CoreLocal<T> {
    /// # Safety
    /// the passed item pointed to by the passed ptr must sit in the `.cpu_local` section.
    pub const unsafe fn new(template: *const CoreLocalStorage<T>) -> Self {
        Self {
            template,
            _phantom: PhantomData,
        }
    }

    // TODO: maybe make thius private
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        let start = (&raw const __cpu_local_start) as usize;
        let addr = self.template as usize;

        debug_assert!(addr >= start && addr < (&raw const __cpu_local_end) as usize,);

        let offset = addr - start;
        let core_base: *const () = CpuDataProvider::public_data_base();
        unsafe { core_base.byte_add(offset) }.cast()
    }

    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        // TODO: pin to this core
        let ptr = self.as_ptr();
        unsafe { f(&*ptr) }
    }

    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        // TODO pin to this core and ensure noone else runs concurrently on this core.
        // I.e. disable interrupts.
        // Then this is safe, because only someone runnign on this core can (safely) access this core_local variable
        // and we are guaranteed to be the only one holding a ref for the duration of f.
        let ptr = self.as_ptr().cast_mut();
        unsafe { f(&mut *ptr) }
    }
}

pub fn allocate_cls_block() -> *mut [u8] {
    let start = (&raw const __cpu_local_start) as usize;
    let end = (&raw const __cpu_local_end) as usize;
    let size = end - start;

    if size == 0 {
        return unsafe { MaybeUninit::<*mut [u8]>::zeroed().assume_init() };
    }

    let layout = Layout::from_size_align(size, 64).unwrap();
    unsafe {
        let ptr = alloc::alloc::alloc(layout);
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }
        ptr::copy_nonoverlapping(start as *const u8, ptr, size);
        ptr.cast_slice(size)
    }
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
