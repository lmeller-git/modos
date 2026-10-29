#![no_std]

use x86_64::{
    VirtAddr,
    structures::paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, Size4KiB},
};

#[global_allocator]
pub static GLOBAL_ALLOCATOR: linked_list::SafeHeap = linked_list::get_alloc();

pub mod linked_list {
    use core::{
        alloc::{GlobalAlloc, Layout},
        ptr::{NonNull, null_mut},
    };

    use linked_list_allocator::Heap;
    use spin::{Mutex, MutexGuard};

    pub(super) const fn get_alloc() -> SafeHeap {
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

pub fn init() {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START as u64);
        let heap_end = heap_start + HEAP_SIZE as u64;
        let heap_start_page = Page::containing_address(heap_start);
        let heap_end_page = Page::containing_address(heap_end);
        Page::range(heap_start_page, heap_end_page)
    };
    for page in page_range {
        let mut allocator = arch_x86_64::frame::get_frame_alloc().lock();
        let frame = allocator.allocate_frame().unwrap();
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::GLOBAL;
        unsafe {
            arch_x86_64::page::get_pagetable()
                .lock()
                .map_to(page, frame, flags, &mut *allocator)
                .unwrap()
                .flush();
        };
    }
    unsafe { GLOBAL_ALLOCATOR.init(HEAP_START as *mut u8, HEAP_SIZE) };
}

pub fn map_heap(tbl: &mut OffsetPageTable<'_>) {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START as u64);
        let heap_end = heap_start + HEAP_SIZE as u64 - 1u64;
        let heap_start_page: Page<Size4KiB> = Page::containing_address(heap_start);
        let heap_end_page: Page<Size4KiB> = Page::containing_address(heap_end);
        Page::range_inclusive(heap_start_page, heap_end_page)
    };

    let mapper = arch_x86_64::page::get_pagetable().lock();
    let mut allocator = arch_x86_64::frame::get_frame_alloc().lock();
    for page in page_range {
        let frame = mapper.translate_page(page).unwrap();
        unsafe {
            tbl.map_to(
                page,
                frame,
                PageTableFlags::WRITABLE | PageTableFlags::PRESENT | PageTableFlags::GLOBAL,
                &mut *allocator,
            )
            .unwrap()
            .flush();
        }
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
