use derive_more::Display;
use kernel_types::frame::{FRAME_4KIB_SIZE, PhysMemoryFrame, VirtMemoryFrame};
use thiserror::Error;

use crate::mm::{FrameDeallocator, allocr::FrameAllocator};

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
    /// # Safety
    /// The caller needs to ensure that the frame we are mapping the virtual memory onto is not currently used.
    unsafe fn map_to(
        page: VirtMemoryFrame<FRAME_SIZE>,
        frame: PhysMemoryFrame<FRAME_SIZE>,
        flags: PageFlags,
        frame_allocator: &mut impl FrameAllocator<FRAME_SIZE>,
    ) -> Result<Self::Flush, PageTableError>;

    /// # Safety
    /// The caller needs to ensure that the frame we are mapping the virtual memory onto is not currently used.
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

#[derive(Debug, thiserror::Error, PartialEq, PartialOrd, Clone)]
pub enum FlushError {
    #[error("Unknown flush eror: {}", msg)]
    Unknonw { msg: &'static str },
}

pub trait TLBFlushCapability {
    type Flush;
    fn flush(flush: Self::Flush) -> Result<(), FlushError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    mod page_flags {
        use super::*;

        /// Bit positions must match the hardware PTE layout, and leave the address bits alone.
        #[test]
        fn pte_layout() {
            let layout = [
                (PageFlags::PRESENT, 0),
                (PageFlags::WRITABLE, 1),
                (PageFlags::USER_ACCESSIBLE, 2),
                (PageFlags::WRITE_THROUGH, 3),
                (PageFlags::NO_CACHE, 4),
                (PageFlags::ACCESSED, 5),
                (PageFlags::DIRTY, 6),
                (PageFlags::HUGE_PAGE, 7),
                (PageFlags::GLOBAL, 8),
                (PageFlags::NO_EXECUTE, 63),
            ];
            let os_bits = (9..=11).chain(52..=62);

            let mut seen = PageFlags::empty();
            for (flag, bit) in layout {
                assert_eq!(flag.bits(), 1 << bit, "{flag:?}");
                seen |= flag;
            }
            for bit in os_bits {
                let flag = PageFlags::from_bits(1 << bit).expect("OS-available bit");
                assert!(!seen.intersects(flag));
                seen |= flag;
            }

            assert_eq!(seen, PageFlags::all());
            // Physical address bits of an entry are never representable as flags.
            let addr_bits = ((1u64 << 52) - 1) & !0xfff;
            assert_eq!(PageFlags::all().bits() & addr_bits, 0);
        }
    }
}
