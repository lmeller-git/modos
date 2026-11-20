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

/// Map `len` bytes of memory at `phys` as uncached and return the virtual address of `phys`.
///
/// # Safety
///
/// `phys` must point to device memory of sufficient length that the caller owns and may access.
pub(crate) unsafe fn map_no_cache(phys: PhysAddr, len: usize) -> VirtAddr {
    debug_assert!(len > 0);

    let page_size = Size4KiB::SIZE as usize;
    let page_offset = (phys.as_u64() & (Size4KiB::SIZE - 1)) as usize;
    let total_bytes = (page_offset + len).next_multiple_of(page_size);
    let num_pages = (total_bytes / page_size) as u64;

    let virt_base = NEXT_MMIO_VIRT
        .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
            let end = cur.checked_add(total_bytes)?;
            (end <= MMIO_START + MMIO_MAX_SIZE).then_some(end)
        })
        .expect("MMIO virtual window exhausted");

    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_base as u64));
    let start_frame = PhysFrame::<Size4KiB>::containing_address(phys);

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH
        | PageTableFlags::NO_EXECUTE;

    let mut mapper = get_pagetable().lock();
    let mut allocator = get_frame_alloc().lock();
    for i in 0..num_pages {
        unsafe { mapper.map_to(start_page + i, start_frame + i, flags, &mut *allocator) }
            .unwrap()
            .flush();
    }

    VirtAddr::new((virt_base + page_offset) as u64)
}

/// Platform-interfaces compatible [`map_no_cache`].
///
/// # Safety
///
/// See [`map_no_cache`].
pub(crate) unsafe fn map_mmio(phys: u64, len: usize) -> u64 {
    unsafe { map_no_cache(PhysAddr::new(phys), len) }.as_u64()
}
