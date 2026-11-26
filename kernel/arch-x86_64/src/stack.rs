//! Fixed stack regions. Every stack sits directly above its own unmapped guard page.

use mem_layout::{
    CORE_STACK_SIZE,
    CORE_STACK_START,
    IST_STACK_SIZE,
    IST_STACK_START,
    MAX_CORE_STACKS,
    MAX_IST_STACKS,
};
use x86_64::{
    VirtAddr,
    structures::paging::{FrameAllocator, Mapper, Page, PageSize, PageTableFlags, Size4KiB},
};

const PAGE: usize = Size4KiB::SIZE as usize;

/// `count` stacks of `size` bytes from `start`, each preceded by a guard page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StackRegion {
    start: usize,
    size: usize,
    count: usize,
}

pub(crate) const CORE_STACKS: StackRegion = StackRegion {
    start: CORE_STACK_START,
    size: CORE_STACK_SIZE,
    count: MAX_CORE_STACKS,
};

pub(crate) const IST_STACKS: StackRegion = StackRegion {
    start: IST_STACK_START,
    size: IST_STACK_SIZE,
    count: MAX_IST_STACKS,
};

impl StackRegion {
    /// Lowest mapped address of stack `index`; the page below it is its guard.
    pub(crate) fn base(self, index: usize) -> usize {
        assert!(index < self.count, "stack {index} is outside its region");
        self.start + index * (self.size + PAGE) + PAGE
    }

    /// Maps stack `index` (writable, non-executable) and returns its top.
    pub(crate) fn map(self, index: usize) -> VirtAddr {
        let base = self.base(index);
        let pages = Page::<Size4KiB>::range(
            Page::containing_address(VirtAddr::new(base as u64)),
            Page::containing_address(VirtAddr::new((base + self.size) as u64)),
        );

        let mut frame_alloc = crate::frame::get_frame_alloc().lock();
        let mut pagetable = crate::page::get_pagetable().lock();
        for page in pages {
            let frame = frame_alloc.allocate_frame().unwrap();
            unsafe {
                pagetable.map_to(
                    page,
                    frame,
                    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
                    &mut *frame_alloc,
                )
            }
            .unwrap()
            .flush();
        }

        VirtAddr::new((base + self.size) as u64)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::{config, panics};

    use super::*;

    /// Index pairs `(a, a + gap)` inside `region`, biased to both ends.
    fn pair(region: StackRegion) -> impl Strategy<Value = (usize, usize)> {
        let last = region.count - 1;
        (prop_oneof![0..8usize, last - 8..=last, 0..=last], 0..4usize)
            .prop_map(move |(a, gap)| (a, (a + gap).min(last)))
    }

    /// Stacks are page-aligned, lie inside the region with their guard page, and never touch
    /// the guard page or stack of another; indices past the region are rejected.
    fn layout(region: StackRegion, (a, b): (usize, usize)) -> Result<(), TestCaseError> {
        let end = region.start + region.count * (region.size + PAGE);
        let (base_a, base_b) = (region.base(a), region.base(b));

        prop_assert!(region.size.is_multiple_of(PAGE) && base_a.is_multiple_of(PAGE));
        prop_assert!(base_a - PAGE >= region.start && base_b + region.size <= end);
        if a < b {
            // `b`'s guard page lies between the two stacks.
            prop_assert!(base_a + region.size <= base_b - PAGE);
        }
        prop_assert!(panics(|| region.base(region.count)));
        Ok(())
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn core_stacks(pair in pair(CORE_STACKS)) {
            layout(CORE_STACKS, pair)?;
        }

        #[test]
        fn ist_stacks(pair in pair(IST_STACKS)) {
            layout(IST_STACKS, pair)?;
        }
    }
}
