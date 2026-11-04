use derive_more::Display;
use thiserror::Error;

use crate::mm::{
    FrameDeallocator,
    addr::{PhysAddr, VirtAddr},
    allocr::FrameAllocator,
};

// PageFlags adapted from https://github.com/rust-osdev/x86_64/blob/master/src/addr.rs. Licensed under MIT https://github.com/rust-osdev/x86_64/blob/master/LICENSE-MIT.

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
    /// Possible flags for a page table entry.
    pub struct PageFlags: u64 {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT =         1;
        /// Controls whether writes to the mapped frames are allowed.
        ///
        /// If this bit is unset in a level 1 page table entry, the mapped frame is read-only.
        /// If this bit is unset in a higher level page table entry the complete range of mapped
        /// pages is read-only.
        const WRITABLE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
        const USER_ACCESSIBLE = 1 << 2;
        /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
        /// policy is used.
        const WRITE_THROUGH =   1 << 3;
        /// Disables caching for the pointed entry is cacheable.
        const NO_CACHE =        1 << 4;
        /// Set by the CPU when the mapped frame or page table is accessed.
        const ACCESSED =        1 << 5;
        /// Set by the CPU on a write to the mapped frame.
        const DIRTY =           1 << 6;
        /// Specifies that the entry maps a huge frame instead of a page table. Only allowed in
        /// P2 or P3 tables.
        const HUGE_PAGE =       1 << 7;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_9 =           1 << 9;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_10 =          1 << 10;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_11 =          1 << 11;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_52 =          1 << 52;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_53 =          1 << 53;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_54 =          1 << 54;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_55 =          1 << 55;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_56 =          1 << 56;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_57 =          1 << 57;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_58 =          1 << 58;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_59 =          1 << 59;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_60 =          1 << 60;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_61 =          1 << 61;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_62 =          1 << 62;
        /// Forbid code execution from the mapped frames.
        ///
        /// Can be only used when the no-execute page protection feature is enabled in the EFER
        /// register.
        const NO_EXECUTE =      1 << 63;
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

#[derive(Clone, Debug, Copy)]
pub struct VirtMemoryFrame<const SIZE: usize = FRAME_4KIB_SIZE> {
    start: VirtAddr,
}

impl<const SIZE: usize> VirtMemoryFrame<SIZE> {
    pub fn from_addr(addr: VirtAddr) -> Option<Self> {
        // TODO: check if addr is aligned to SIZE
        Some(Self { start: addr })
    }

    pub fn start(&self) -> VirtAddr {
        self.start
    }

    pub fn containing_address(mut addr: VirtAddr) -> Self {
        addr.align_down(SIZE);
        Self { start: addr }
    }

    pub fn range(
        start: VirtMemoryFrame<SIZE>,
        end: VirtMemoryFrame<SIZE>,
    ) -> VirtMemoryFrameRange<SIZE> {
        VirtMemoryFrameRange { start, end }
    }
}

#[derive(Clone, Debug)]
pub struct VirtMemoryFrameRange<const SIZE: usize = FRAME_4KIB_SIZE> {
    start: VirtMemoryFrame<SIZE>,
    end: VirtMemoryFrame<SIZE>,
}

impl<const SIZE: usize> VirtMemoryFrameRange<SIZE> {
    pub fn is_empty(&self) -> bool {
        self.end.start <= self.start.start
    }

    pub fn len(&self) -> usize {
        if !self.is_empty() {
            self.end.start.as_usize() - self.start.start.as_usize()
        } else {
            0
        }
    }
}

impl<const SIZE: usize> IntoIterator for VirtMemoryFrameRange<SIZE> {
    type IntoIter = VirtMemoryFrameRangeIter<SIZE>;
    type Item = VirtMemoryFrame<SIZE>;

    fn into_iter(self) -> Self::IntoIter {
        VirtMemoryFrameRangeIter(self)
    }
}

#[derive(Clone, Debug)]
pub struct VirtMemoryFrameRangeIter<const SIZE: usize = FRAME_4KIB_SIZE>(
    VirtMemoryFrameRange<SIZE>,
);

impl<const SIZE: usize> Iterator for VirtMemoryFrameRangeIter<SIZE> {
    type Item = VirtMemoryFrame<SIZE>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.start.start < self.0.end.start {
            let page = self.0.start;
            self.0.start.start = self.0.start.start + SIZE;
            Some(page)
        } else {
            None
        }
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        for _ in 1..n {
            self.next()?;
        }
        self.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.0.len();
        (len, Some(len))
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
pub enum PageTableErrorKind {
    #[error("No error")]
    None,
}

pub trait MemMapperCapability<const FRAME_SIZE: usize = FRAME_4KIB_SIZE>:
    TLBFlushCapability
{
    unsafe fn map_to(
        page: VirtMemoryFrame<FRAME_SIZE>,
        frame: PhysMemoryFrame<FRAME_SIZE>,
        flags: PageFlags,
        frame_allocator: &mut impl FrameAllocator<FRAME_SIZE>,
    ) -> Result<Self::Flush, PageTableError>;

    unsafe fn map_to_global(
        page: VirtMemoryFrame<FRAME_SIZE>,
        frame: PhysMemoryFrame<FRAME_SIZE>,
        flags: PageFlags,
    ) -> Result<Self::Flush, PageTableError>;
}

pub trait MemUnmapperCapability<const FRAME_SIZE: usize = FRAME_4KIB_SIZE>:
    TLBFlushCapability
{
    fn unmap(
        page: VirtMemoryFrame<FRAME_SIZE>,
        frame_deallocator: &mut impl FrameDeallocator<FRAME_SIZE>,
    ) -> Result<(PhysMemoryFrame<FRAME_SIZE>, Self::Flush), PageTableError>;
}

pub trait TLBFlushCapability {
    type Flush;
    fn flush(flush: Self::Flush) -> Result<(), ()>;
}
