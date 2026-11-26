//! Uncached mapping for device memory

use core::sync::atomic::{AtomicUsize, Ordering};

use mem_layout::{MMIO_MAX_SIZE, MMIO_START};
use x86_64::{
    PhysAddr,
    VirtAddr,
    structures::paging::{Mapper, Page, PageSize, PageTableFlags, PhysFrame, Size4KiB},
};

use crate::{frame::get_frame_alloc, page::get_pagetable};

static NEXT_MMIO_VIRT: AtomicUsize = AtomicUsize::new(MMIO_START);

/// A planned slice of the MMIO window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    /// First (page-aligned) virtual address of the mapping.
    base: usize,
    /// Virtual address of `phys` itself.
    virt: usize,
    pages: usize,
    /// Value of the cursor after this mapping.
    next: usize,
}

/// Places the `len` bytes at `phys` at `cursor`, or `None` if the window would be exceeded.
fn plan_window(cursor: usize, phys: u64, len: usize) -> Option<Window> {
    let page_size = Size4KiB::SIZE as usize;
    let page_offset = (phys & (Size4KiB::SIZE - 1)) as usize;
    let total_bytes = page_offset
        .checked_add(len)?
        .checked_next_multiple_of(page_size)?;

    let next = cursor.checked_add(total_bytes)?;
    (next <= MMIO_START + MMIO_MAX_SIZE).then_some(Window {
        base: cursor,
        virt: cursor + page_offset,
        pages: total_bytes / page_size,
        next,
    })
}

/// Map `len` bytes of memory at `phys` as uncached and return the virtual address of `phys`.
///
/// # Safety
///
/// `phys` must point to device memory of sufficient length that the caller owns and may access.
pub(crate) unsafe fn map_no_cache(phys: PhysAddr, len: usize) -> VirtAddr {
    debug_assert!(len > 0);

    let mut window = None;
    NEXT_MMIO_VIRT
        .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
            window = plan_window(cur, phys.as_u64(), len);
            window.map(|w| w.next)
        })
        .expect("MMIO virtual window exhausted");
    let window = window.expect("the successful update planned a window");

    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(window.base as u64));
    let start_frame = PhysFrame::<Size4KiB>::containing_address(phys);

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH
        | PageTableFlags::NO_EXECUTE;

    let mut mapper = get_pagetable().lock();
    let mut allocator = get_frame_alloc().lock();
    for i in 0..window.pages as u64 {
        unsafe { mapper.map_to(start_page + i, start_frame + i, flags, &mut *allocator) }
            .unwrap()
            .flush();
    }

    VirtAddr::new(window.virt as u64)
}

/// Platform-interfaces compatible [`map_no_cache`].
///
/// # Safety
///
/// See [`map_no_cache`].
pub(crate) unsafe fn map_mmio(phys: u64, len: usize) -> u64 {
    unsafe { map_no_cache(PhysAddr::new(phys), len) }.as_u64()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    const PAGE: usize = 4096;

    proptest! {
        #![proptest_config(config())]

        /// A mapping is placed at the cursor and covers `[phys, phys + len)` with the fewest
        /// pages; it fails exactly when it would leave the window (or overflow).
        #[test]
        fn plan_window(
            cursor in prop_oneof![
                (0..=MMIO_MAX_SIZE / PAGE).prop_map(|pages| MMIO_START + pages * PAGE),
                // Flush against the end of the window.
                (0..=8usize).prop_map(|pages| MMIO_START + MMIO_MAX_SIZE - pages * PAGE),
                any::<usize>(),
            ],
            phys in any::<u64>(),
            len in prop_oneof![1..=8 * PAGE, any::<usize>()],
        ) {
            let offset = (phys % PAGE as u64) as usize;
            let pages = (offset as u128 + len as u128).div_ceil(PAGE as u128);
            let next = cursor as u128 + pages * PAGE as u128;
            let fits = next <= (MMIO_START + MMIO_MAX_SIZE) as u128;

            let expected = fits.then(|| Window {
                base: cursor,
                virt: cursor + offset,
                pages: pages as usize,
                next: next as usize,
            });
            prop_assert_eq!(super::plan_window(cursor, phys, len), expected);
        }
    }
}
