#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use core::{
    alloc::Layout,
    marker::PhantomData,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

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

// Note: we depend on x86_64-primitives directly, since arch and arch-x86_64 use core_local.
#[cfg(target_arch = "x86_64")]
type CpuDataProvider = arch_x86_64_primitives::Primitive;
#[cfg(not(target_arch = "x86_64"))]
struct CpuDataProvider;

#[doc(hidden)]
pub mod __private {
    use core::{
        cell::UnsafeCell,
        sync::atomic::{
            AtomicBool,
            AtomicI8,
            AtomicI16,
            AtomicI32,
            AtomicI64,
            AtomicIsize,
            AtomicPtr,
            AtomicU8,
            AtomicU16,
            AtomicU32,
            AtomicU64,
            AtomicUsize,
        },
    };

    use crate::CpuLocalData;

    /// Sealed: implemented for the `core::sync::atomic` types only.
    pub trait Atomic {}

    macro_rules! atomics {
        ($($ty:ty),* $(,)?) => { $(impl Atomic for $ty {})* };
    }
    atomics!(
        AtomicBool,
        AtomicI8,
        AtomicI16,
        AtomicI32,
        AtomicI64,
        AtomicIsize,
        AtomicU8,
        AtomicU16,
        AtomicU32,
        AtomicU64,
        AtomicUsize,
    );
    impl<T> Atomic for AtomicPtr<T> {}

    #[repr(transparent)]
    pub struct CoreLocalStorage<T: CpuLocalData>(pub(crate) UnsafeCell<T>);

    // SAFETY: This is only accessed via controlled core_local structs.
    unsafe impl<T: CpuLocalData + Send> Sync for CoreLocalStorage<T> {}

    impl<T: CpuLocalData> CoreLocalStorage<T> {
        pub const fn new(val: T) -> Self {
            Self(UnsafeCell::new(val))
        }
    }
}

use __private::CoreLocalStorage;
use arch_interfaces::cpu::{CpuDataCapability, CpuId};

/// # Safety
/// Must be safe to bitwise-copy from the `.cpu_local` template at core boot.
pub unsafe trait CpuLocalData {}

unsafe impl<T: Copy> CpuLocalData for T {}
unsafe impl<T> CpuLocalData for CLSLazyPtr<T> {}

/// An atomic usable as (remote) core local data, e.g. `CLSAtomic<AtomicUsize>`. Derefs to the atomic.
#[repr(transparent)]
pub struct CLSAtomic<A: __private::Atomic>(A);

impl<A: __private::Atomic> CLSAtomic<A> {
    pub const fn new(atomic: A) -> Self {
        Self(atomic)
    }
}

impl<A: __private::Atomic> core::ops::Deref for CLSAtomic<A> {
    type Target = A;

    #[inline]
    fn deref(&self) -> &A {
        &self.0
    }
}

// SAFETY: sealed to atomics, which own nothing and are valid when bitwise-copied from a never-mutated template.
unsafe impl<A: __private::Atomic> CpuLocalData for CLSAtomic<A> {}

pub struct CLSLazyPtr<T>(AtomicPtr<T>);

unsafe impl<T: Send + Sync> Sync for CLSLazyPtr<T> {}
unsafe impl<T: Send> Send for CLSLazyPtr<T> {}

impl<T> CLSLazyPtr<T> {
    pub const fn new() -> Self {
        Self(AtomicPtr::new(ptr::null_mut()))
    }

    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        let ptr = self.0.load(Ordering::Acquire);
        unsafe { ptr.as_mut() }
    }

    #[inline]
    pub fn get(&self) -> Option<&T> {
        let ptr = self.0.load(Ordering::Acquire);
        unsafe { ptr.as_ref() }
    }

    #[inline]
    pub fn get_or_init(&self, init: impl FnOnce() -> Box<T>) -> &T {
        if let Some(val) = self.get() {
            return val;
        }
        self.init_slow(init)
    }

    #[cold]
    fn init_slow(&self, init: impl FnOnce() -> Box<T>) -> &T {
        let new_ptr = Box::into_raw(init());
        match self.0.compare_exchange(
            ptr::null_mut(),
            new_ptr,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => unsafe { &*new_ptr },
            Err(existing) => {
                unsafe { drop(Box::from_raw(new_ptr)) };
                unsafe { &*existing }
            }
        }
    }
}

impl<T> Default for CLSLazyPtr<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "C" {
    static __cpu_local_start: u8;
    static __cpu_local_end: u8;
}

/// Offset of a `.cpu_local` template item from the start of the section, which is also its offset inside every core's cls block.
#[inline]
fn template_offset<T: CpuLocalData>(template: *const CoreLocalStorage<T>) -> usize {
    let start = (&raw const __cpu_local_start) as usize;
    let addr = template as usize;

    debug_assert!(addr >= start && addr < (&raw const __cpu_local_end) as usize);

    addr - start
}

/// A core local variable.
pub struct CoreLocal<T: CpuLocalData> {
    // TODO:
    // we could circumvent storing this ptr by generating one type/method per core_local variable and just inlinign &raw const STATIC at the call site.
    // Makes the whole thing a bit more complex though and might need some trait plumbing.
    template: *const CoreLocalStorage<T>,
    _phantom: PhantomData<T>,
}

// SAFETY: The ptr here is never actually dereferenced. It is just used for offset calculation and access are redirected to core_local seciton.
unsafe impl<T: CpuLocalData> Sync for CoreLocal<T> {}

impl<T: CpuLocalData> CoreLocal<T> {
    /// # Safety
    /// the passed item pointed to by the passed ptr must sit in the `.cpu_local` section.
    pub const unsafe fn new(template: *const CoreLocalStorage<T>) -> Self {
        Self {
            template,
            _phantom: PhantomData,
        }
    }

    // TODO: maybe make this private
    #[inline]
    pub fn as_ptr(&self) -> *const CoreLocalStorage<T> {
        let core_base: *const () = CpuDataProvider::public_data_base();
        unsafe { core_base.byte_add(template_offset(self.template)) }.cast()
    }

    #[inline]
    pub fn with<'a, R>(&'a self, f: impl FnOnce(&'a T) -> R) -> R {
        // TODO: pin to this core
        let cell = unsafe { &*self.as_ptr() };
        unsafe { f(&*cell.0.get()) }
    }

    #[inline]
    pub fn with_mut<'a, R>(&'a self, f: impl FnOnce(&'a mut T) -> R) -> R {
        // TODO pin to this core and ensure noone else runs concurrently on this core.
        // I.e. disable interrupts.
        // Then this is safe, because only someone runnign on this core can (safely) access this core_local variable
        // and we are guaranteed to be the only one holding a ref for the duration of f.
        let cell = unsafe { &*self.as_ptr() };
        unsafe { f(&mut *cell.0.get()) }
    }
}

/// A core local variable that other cores may access.
pub struct RemoteCoreLocal<T: CpuLocalData + Sync> {
    template: *const CoreLocalStorage<T>,
    _phantom: PhantomData<T>,
}

unsafe impl<T: CpuLocalData + Sync> Sync for RemoteCoreLocal<T> {}

impl<T: CpuLocalData + Sync> RemoteCoreLocal<T> {
    /// # Safety
    /// the passed item pointed to by the passed ptr must sit in the `.cpu_local` section.
    pub const unsafe fn new(template: *const CoreLocalStorage<T>) -> Self {
        Self {
            template,
            _phantom: PhantomData,
        }
    }

    /// Returns this cores instance.
    #[inline]
    pub fn local(&self) -> &T {
        // we can just return a ref here, since this can be accessed from other cores anyways.
        let core_base: *const () = CpuDataProvider::public_data_base();
        // SAFETY: cls blocks are never freed and `T: Sync`.
        unsafe { self.at(core_base) }
    }

    /// Returns the instance of `cpu`, or `None` if it is unknown or not online yet.
    #[inline]
    pub fn on(&self, cpu: CpuId) -> Option<&T> {
        let core_base = CpuDataProvider::public_data_base_of(cpu)?;
        // SAFETY: a published base points to a live, initialized cls block and `T: Sync`.
        Some(unsafe { self.at(core_base) })
    }

    /// # Safety
    /// `core_base` must be the cls base of an online core.
    #[inline]
    unsafe fn at(&self, core_base: *const ()) -> &T {
        let cell = unsafe {
            &*core_base
                .byte_add(template_offset(self.template))
                .cast::<CoreLocalStorage<T>>()
        };
        unsafe { &*cell.0.get() }
    }
}

impl<T: Send + Sync> RemoteCoreLocal<CLSLazyPtr<T>> {
    /// Returns this core's value if it is initialized.
    #[inline]
    pub fn get(&self) -> Option<&T> {
        self.local().get()
    }

    /// Returns the value of `cpu` if that core is online and has initialized it.
    #[inline]
    pub fn get_on(&self, cpu: CpuId) -> Option<&T> {
        self.on(cpu)?.get()
    }

    #[inline]
    pub fn get_or_init(&self, init: impl FnOnce() -> Box<T>) -> &T {
        self.local().get_or_init(init)
    }
}

impl<T> CoreLocal<CLSLazyPtr<T>> {
    #[inline]
    pub fn get(&self) -> Option<&T> {
        self.with(|zelf| zelf.get())
    }

    #[inline]
    pub fn get_mut(&self) -> Option<&mut T> {
        self.with_mut(|zelf| zelf.get_mut())
    }

    #[inline]
    pub fn get_or_init(&self, init: impl FnOnce() -> Box<T>) -> &T {
        self.with(|zelf| zelf.get_or_init(init))
    }
}

const CACHE_LINE: usize = 64;

pub fn allocate_cls_block() -> *mut [u8] {
    let start = (&raw const __cpu_local_start) as usize;
    let end = (&raw const __cpu_local_end) as usize;
    let size = end - start;

    if size == 0 {
        return ptr::slice_from_raw_parts_mut(ptr::null_mut(), 0);
    }

    let padded = size.next_multiple_of(CACHE_LINE);
    let layout = Layout::from_size_align(padded, CACHE_LINE).unwrap();
    unsafe {
        let ptr = alloc::alloc::alloc(layout);
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }
        ptr::copy_nonoverlapping(start as *const u8, ptr, size);
        ptr.add(size).write_bytes(0, padded - size);
        ptr.cast_slice(padded)
    }
}
