#![no_std]

use arch::{
    TargetArch,
    interface::mm::{
        FrameAllocatorCapability,
        MemMapperCapability,
        PageFlags,
        VirtAddr,
        VirtMemoryFrame,
    },
};

use crate::linked_list::SafeHeap;

pub mod linked_list {
    use core::{
        alloc::{GlobalAlloc, Layout},
        ptr::{NonNull, null_mut},
    };

    use linked_list_allocator::Heap;
    use spin::{Mutex, MutexGuard};

    pub const fn get_alloc() -> SafeHeap {
        SafeHeap::new()
    }

    pub struct SafeHeap {
        inner: Mutex<Heap>,
    }

    impl SafeHeap {
        pub const fn new() -> Self {
            Self {
                inner: Mutex::new(Heap::empty()),
            }
        }

        /// # Safety
        /// `heap_bottom` needs to be a valid ptr pointing to unused and mapped virtual memory.
        /// heap size needs to be a valid size of unused memory, that is contiguous mapped free memory after `heap_bottom`.
        pub unsafe fn init(&self, heap_bottom: *mut u8, heap_size: usize) {
            unsafe {
                self.inner.lock().init(heap_bottom, heap_size);
            }
        }

        pub fn lock(&self) -> MutexGuard<'_, Heap> {
            self.inner.lock()
        }
    }

    impl Default for SafeHeap {
        fn default() -> Self {
            Self::new()
        }
    }

    unsafe impl GlobalAlloc for SafeHeap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            match self.lock().allocate_first_fit(layout) {
                Ok(ptr) => ptr.as_ptr(),
                Err(_) => null_mut(),
            }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if let Some(nn_ptr) = NonNull::new(ptr) {
                unsafe { self.lock().deallocate(nn_ptr, layout) };
            }
        }
    }
}

pub const HEAP_START: usize = 0x_7444_4444_0000;
pub const HEAP_SIZE: usize = 10000 * 1024; // 10 MiB

pub fn init(alloc: &SafeHeap) {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START);
        let heap_end = heap_start + HEAP_SIZE;
        let heap_start_page = VirtMemoryFrame::containing_address(heap_start);
        let heap_end_page = VirtMemoryFrame::containing_address(heap_end);
        VirtMemoryFrame::range(heap_start_page, heap_end_page)
    };

    for page in page_range {
        let frame = TargetArch::allocate().unwrap();
        let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::GLOBAL;
        unsafe { TargetArch::map_to_global(page, frame, flags) }
            .unwrap()
            .flush();
    }
    unsafe { alloc.init(HEAP_START as *mut u8, HEAP_SIZE) };
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
