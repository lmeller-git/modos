//! Memory frames are their aligned start address.

use crate::{
    addr::{PhysAddr, VirtAddr},
    aligned::{Aligned, AlignedRange},
};

pub const FRAME_4KIB_SIZE: usize = 4096;

pub type PhysMemoryFrame<const SIZE: usize = FRAME_4KIB_SIZE> = Aligned<PhysAddr, SIZE>;
pub type VirtMemoryFrame<const SIZE: usize = FRAME_4KIB_SIZE> = Aligned<VirtAddr, SIZE>;
pub type VirtMemoryFrameRange<const SIZE: usize = FRAME_4KIB_SIZE> = AlignedRange<VirtAddr, SIZE>;
