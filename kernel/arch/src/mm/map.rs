use derive_more::Display;
use thiserror::Error;

use crate::mm::{
    addr::{PhysAddr, VirtAddr},
    allocr::{FrameAllocator, FrameDeallocator},
};

bitflags::bitflags! {
    #[derive(
        Clone,
        Copy,
        Debug,
        Display,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Hash,
    )]
    #[repr(transparent)]
    pub struct PageFlags: u32 {
        const PRESENT  = 1 << 0;
        const WRITABLE = 1 << 1;
        const USER     = 1 << 2;
        const NO_CACHE = 1 << 3;
        const GLOBAL   = 1 << 4;
    }
}

pub const FRAME_4KIB_SIZE: usize = 4096;

pub struct PhysMemoryFrame<const SIZE: usize = FRAME_4KIB_SIZE> {
    start: PhysAddr,
}

impl<const SIZE: usize> PhysMemoryFrame<SIZE> {
    pub fn from_addr(addr: PhysAddr) -> Option<Self> {
        // TODO: check if addr is aligned to SIZE
        Some(Self { start: addr })
    }

    pub fn start(&self) -> PhysAddr {
        self.start
    }
}

#[derive(Error, Debug)]
pub enum PageTableError {
    #[error(transparent)]
    Simple(PageTableErrorKind),
    #[error("Custom error with kind: {}, msg: {}", kind, msg)]
    Custom {
        kind: PageTableErrorKind,
        msg: &'static str,
    },
}

#[derive(Error, Debug)]
pub enum PageTableErrorKind {}

pub fn map_region<const FRAME_SIZE: usize>(
    start: VirtAddr,
    size: usize,
    flags: PageFlags,
    frame_allocator: &mut impl FrameAllocator<FRAME_SIZE>,
) -> Result<(), PageTableError> {
    unimplemented!()
}

pub fn unmap_region<const FRAME_SIZE: usize>(
    start: VirtAddr,
    size: usize,
    frame_deallocator: &mut impl FrameDeallocator<FRAME_SIZE>,
) -> Result<(), PageTableError> {
    unimplemented!()
}
