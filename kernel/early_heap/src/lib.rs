#![no_std]

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
extern crate std;

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
use mem_layout::{KERNEL_HEAP_SIZE, KERNEL_HEAP_START};

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

    #[cfg(test)]
    mod tests {
        use alloc::{
            alloc::{alloc as host_alloc, dealloc as host_dealloc},
            vec::Vec,
        };

        use proptest::prelude::*;
        use test_host::config;

        use super::*;

        #[cfg(any(miri, loom, shuttle))]
        const HEAP: usize = 2 * 1024;
        #[cfg(not(any(miri, loom, shuttle)))]
        const HEAP: usize = 16 * 1024;
        #[cfg(any(miri, loom, shuttle))]
        const OPS: usize = 16;
        #[cfg(not(any(miri, loom, shuttle)))]
        const OPS: usize = 200;

        /// A [`SafeHeap`] over host memory, freed on drop (also when an assertion fails).
        struct TestHeap {
            heap: SafeHeap,
            base: *mut u8,
        }

        impl TestHeap {
            const BACKING: Layout = match Layout::from_size_align(HEAP, 4096) {
                Ok(layout) => layout,
                Err(_) => panic!("invalid backing layout"),
            };

            fn new() -> Self {
                let base = unsafe { host_alloc(Self::BACKING) };
                assert!(!base.is_null());
                let heap = SafeHeap::new();
                unsafe { heap.init(base, HEAP) };
                Self { heap, base }
            }

            /// Allocates `layout`, checking the block is aligned and inside the heap.
            fn alloc(&self, layout: Layout) -> Option<*mut u8> {
                let ptr = unsafe { self.heap.alloc(layout) };
                if ptr.is_null() {
                    return None;
                }
                let (addr, base) = (ptr as usize, self.base as usize);
                assert!(
                    addr.is_multiple_of(layout.align()),
                    "{addr:#x} misaligned for {layout:?}"
                );
                assert!(
                    addr >= base && addr + layout.size() <= base + HEAP,
                    "{addr:#x} outside the heap"
                );
                Some(ptr)
            }

            /// Allocates `layout` until the heap is exhausted.
            fn exhaust(&self, layout: Layout) -> Vec<*mut u8> {
                core::iter::from_fn(|| self.alloc(layout)).collect()
            }
        }

        impl Drop for TestHeap {
            fn drop(&mut self) {
                unsafe { host_dealloc(self.base, Self::BACKING) };
            }
        }

        fn layout() -> impl Strategy<Value = Layout> {
            (1..512usize, 0..7u32)
                .prop_map(|(size, shift)| Layout::from_size_align(size, 1 << shift).unwrap())
        }

        #[derive(Clone, Copy, Debug)]
        enum Op {
            Alloc(Layout),
            Free(prop::sample::Index),
        }

        proptest! {
            #![proptest_config(config())]

            /// Exhaustion yields null rather than a panic, `dealloc(null)` is a no-op, and
            /// freeing everything gives the whole heap back.
            #[test]
            fn exhaustion_and_reuse(layout in layout()) {
                let heap = TestHeap::new();

                let first = heap.exhaust(layout);
                prop_assert!(!first.is_empty());

                unsafe { heap.heap.dealloc(null_mut(), layout) };
                for &ptr in &first {
                    unsafe { heap.heap.dealloc(ptr, layout) };
                }
                prop_assert_eq!(heap.exhaust(layout).len(), first.len());
            }

            /// Under mixed allocations and frees, live blocks are aligned, inside the heap and
            /// disjoint: each keeps the bytes written to it. Once all are freed the heap serves
            /// as many blocks as a fresh one.
            #[test]
            fn mixed(
                ops in prop::collection::vec(
                    prop_oneof![
                        3 => layout().prop_map(Op::Alloc),
                        2 => any::<prop::sample::Index>().prop_map(Op::Free),
                    ],
                    0..OPS,
                ),
                probe in layout(),
            ) {
                let heap = TestHeap::new();
                let mut live: Vec<(*mut u8, Layout, u8)> = Vec::new();
                let intact = |live: &[(*mut u8, Layout, u8)]| {
                    live.iter().all(|&(ptr, layout, tag)| {
                        unsafe { core::slice::from_raw_parts(ptr, layout.size()) }.iter().all(|&b| b == tag)
                    })
                };

                for (step, op) in ops.into_iter().enumerate() {
                    match op {
                        Op::Alloc(layout) => {
                            if let Some(ptr) = heap.alloc(layout) {
                                let tag = step as u8;
                                unsafe { ptr.write_bytes(tag, layout.size()) };
                                live.push((ptr, layout, tag));
                            }
                        }
                        Op::Free(index) if !live.is_empty() => {
                            let (ptr, layout, _) = live.swap_remove(index.index(live.len()));
                            unsafe { heap.heap.dealloc(ptr, layout) };
                        }
                        Op::Free(_) => {}
                    }
                    prop_assert!(intact(&live), "a live block was overwritten");
                }

                for (ptr, layout, _) in live.drain(..) {
                    unsafe { heap.heap.dealloc(ptr, layout) };
                }
                prop_assert_eq!(heap.exhaust(probe).len(), TestHeap::new().exhaust(probe).len());
            }
        }
    }
}

pub fn init(alloc: &SafeHeap) {
    let page_range = {
        let heap_start = VirtAddr::new(KERNEL_HEAP_START);
        let heap_end = heap_start + KERNEL_HEAP_SIZE;
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
    unsafe { alloc.init(KERNEL_HEAP_START as *mut u8, KERNEL_HEAP_SIZE) };
}
