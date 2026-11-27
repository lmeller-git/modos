//! Conversions between `kernel-types` and the `x86_64` crate.
//!
//! Both sides are already aligned, and `usize` is 64 bits here, so every conversion is lossless.

use kernel_types::{
    addr::PhysAddr,
    frame::{FRAME_4KIB_SIZE, PhysMemoryFrame, VirtMemoryFrame},
};
use x86_64::structures::paging::{Page, PageSize, PhysFrame, Size4KiB};

const _: () = {
    assert!(size_of::<usize>() == size_of::<u64>());
    assert!(FRAME_4KIB_SIZE as u64 == Size4KiB::SIZE);
};

/// Lossless: `usize` is 64 bits on x86_64.
#[expect(
    clippy::cast_possible_truncation,
    reason = "usize is 64 bits on x86_64"
)]
pub(crate) const fn to_usize(value: u64) -> usize {
    value as usize
}

pub(crate) fn to_x86_frame(frame: PhysMemoryFrame) -> PhysFrame<Size4KiB> {
    PhysFrame::containing_address(x86_64::PhysAddr::new(frame.get().as_usize() as u64))
}

pub(crate) fn from_x86_frame(frame: PhysFrame<Size4KiB>) -> PhysMemoryFrame {
    PhysMemoryFrame::containing(PhysAddr::new(to_usize(frame.start_address().as_u64())))
}

pub(crate) fn to_x86_page(page: VirtMemoryFrame) -> Page<Size4KiB> {
    Page::containing_address(x86_64::VirtAddr::new(page.get().as_usize() as u64))
}
