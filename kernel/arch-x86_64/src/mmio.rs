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

/// One past the highest physical address the architecture can name (52 address bits).
const PHYS_LIMIT: u128 = 1 << 52;

/// End of the MMIO window.
const MMIO_END: usize = MMIO_START + MMIO_MAX_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanError {
    /// `[phys, phys + len)` is not a physical address range.
    PhysRange,
    /// The mapping does not fit the rest of the window.
    Exhausted,
}

/// Places the `len` bytes at `phys` at `cursor`, which lies inside the window.
fn plan_window(cursor: usize, phys: u64, len: usize) -> Result<Window, PlanError> {
    debug_assert!((MMIO_START..=MMIO_END).contains(&cursor));
    if u128::from(phys) + len as u128 > PHYS_LIMIT {
        return Err(PlanError::PhysRange);
    }

    let page_size = Size4KiB::SIZE as usize;
    let page_offset = (phys & (Size4KiB::SIZE - 1)) as usize;
    // Cannot overflow: `len < 2^52`.
    let total_bytes = (page_offset + len).next_multiple_of(page_size);

    let next = cursor
        .checked_add(total_bytes)
        .filter(|&next| next <= MMIO_END)
        .ok_or(PlanError::Exhausted)?;
    Ok(Window {
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

    // Holds the plan of the last attempt, which is the one that succeeded, if any did.
    let mut plan = Err(PlanError::Exhausted);
    let _ = NEXT_MMIO_VIRT.try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        plan = plan_window(cur, phys.as_u64(), len);
        plan.ok().map(|w| w.next)
    });
    let window = plan.unwrap_or_else(|err| panic!("cannot map MMIO {phys:?} + {len:#x}: {err:?}"));

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
        /// pages. It fails exactly when the range is not physical (beyond 52 bits) or would
        /// leave the window.
        #[test]
        fn plan_window(
            cursor in prop_oneof![
                (0..=MMIO_MAX_SIZE / PAGE).prop_map(|pages| MMIO_START + pages * PAGE),
                // Flush against the end of the window.
                (0..=8usize).prop_map(|pages| MMIO_END - pages * PAGE),
            ],
            phys in prop_oneof![
                0..1u64 << 52,
                // Around the top of the physical address space.
                (1u64 << 52) - 0x10_000..(1u64 << 52) + 0x10_000,
                any::<u64>(),
            ],
            len in prop_oneof![1..=8 * PAGE, any::<usize>()],
        ) {
            let offset = (phys % PAGE as u64) as usize;
            let pages = (offset as u128 + len as u128).div_ceil(PAGE as u128);
            let next = cursor as u128 + pages * PAGE as u128;

            let expected = if u128::from(phys) + len as u128 > 1 << 52 {
                Err(PlanError::PhysRange)
            } else if next > MMIO_END as u128 {
                Err(PlanError::Exhausted)
            } else {
                Ok(Window {
                    base: cursor,
                    virt: cursor + offset,
                    pages: pages as usize,
                    next: next as usize,
                })
            };
            prop_assert_eq!(super::plan_window(cursor, phys, len), expected);
        }
    }
}
