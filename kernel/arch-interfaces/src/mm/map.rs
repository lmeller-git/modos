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
        addr.as_usize()
            .is_multiple_of(SIZE)
            .then_some(Self { start: addr })
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
        addr.as_usize()
            .is_multiple_of(SIZE)
            .then_some(Self { start: addr })
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
            (self.end.start.as_usize() - self.start.start.as_usize()) / SIZE
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
        if n >= self.0.len() {
            self.0.start = self.0.end;
            return None;
        }
        // n < len, so n * SIZE stays inside the range.
        self.0.start.start = self.0.start.start + n * SIZE;
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
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::*;
    use crate::testing::{config, edge_addr};

    const KIB4: usize = FRAME_4KIB_SIZE;
    const MIB2: usize = 2 << 20;
    const GIB1: usize = 1 << 30;

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

    mod frame {
        use super::*;

        fn containing<const SIZE: usize>(raw: usize) {
            let start = VirtMemoryFrame::<SIZE>::containing_address(VirtAddr::new(raw))
                .start()
                .as_usize();
            assert_eq!(start, raw - raw % SIZE);
        }

        /// Aligned addresses are accepted unchanged, unaligned ones rejected.
        fn from_addr<const SIZE: usize>(raw: usize) {
            let virt = VirtMemoryFrame::<SIZE>::from_addr(VirtAddr::new(raw));
            let phys = PhysMemoryFrame::<SIZE>::from_addr(PhysAddr::new(raw));
            let expected = raw.is_multiple_of(SIZE).then_some(raw);

            assert_eq!(virt.map(|f| f.start().as_usize()), expected);
            assert_eq!(phys.map(|f| f.start().as_usize()), expected);
        }

        proptest! {
            #![proptest_config(config())]

            #[test]
            fn containing_address(
                raw in (edge_addr(KIB4), edge_addr(MIB2), edge_addr(GIB1)),
            ) {
                containing::<KIB4>(raw.0);
                containing::<MIB2>(raw.1);
                containing::<GIB1>(raw.2);
            }

            #[test]
            fn from_addr_alignment(
                raw in (edge_addr(KIB4), edge_addr(MIB2)),
            ) {
                from_addr::<KIB4>(raw.0);
                from_addr::<MIB2>(raw.1);
            }
        }
    }

    mod range {
        use super::*;

        const MAX_SPAN: usize = 64;

        /// `[start, end)` as frame indices: anywhere in the low address space or at the very
        /// top, with `a`/`b` unordered so empty and inverted ranges occur.
        fn range<const SIZE: usize>(
            top: bool,
            base: usize,
            a: usize,
            b: usize,
        ) -> VirtMemoryFrameRange<SIZE> {
            let last = usize::MAX / SIZE;
            let base = if top {
                last - MAX_SPAN - base % 4
            } else {
                base % (1 << 20)
            };
            let frame =
                |i: usize| VirtMemoryFrame::<SIZE>::from_addr(VirtAddr::new((base + i) * SIZE));
            VirtMemoryFrame::range(frame(a).unwrap(), frame(b).unwrap())
        }

        fn starts<const SIZE: usize>(
            iter: impl Iterator<Item = VirtMemoryFrame<SIZE>>,
        ) -> Vec<usize> {
            iter.map(|f| f.start().as_usize()).collect()
        }

        fn reference<const SIZE: usize>(r: &VirtMemoryFrameRange<SIZE>) -> Vec<usize> {
            let (s, e) = (r.start.start.as_usize(), r.end.start.as_usize());
            (s..e).step_by(SIZE).collect()
        }

        fn yields_in_order<const SIZE: usize>(r: VirtMemoryFrameRange<SIZE>) {
            let expected = reference(&r);
            assert_eq!(r.is_empty(), expected.is_empty());
            assert_eq!(starts(r.into_iter()), expected);
        }

        fn exact_size<const SIZE: usize>(r: VirtMemoryFrameRange<SIZE>) {
            let count = reference(&r).len();
            assert_eq!(r.len(), count);
            assert_eq!(r.into_iter().size_hint(), (count, Some(count)));
        }

        /// `nth(n)` is `skip(n).next()`, and iteration resumes after the skipped frames.
        fn nth<const SIZE: usize>(r: VirtMemoryFrameRange<SIZE>, n: usize) {
            let all = reference(&r);
            let mut iter = r.into_iter();

            assert_eq!(
                iter.nth(n).map(|f| f.start().as_usize()),
                all.get(n).copied()
            );
            assert_eq!(
                starts(iter),
                all.into_iter().skip(n + 1).collect::<Vec<_>>()
            );
        }

        proptest! {
            #![proptest_config(config())]

            #[test]
            fn iteration(
                top: bool, base: usize, a in 0..MAX_SPAN, b in 0..MAX_SPAN, n in 0..MAX_SPAN + 6,
            ) {
                yields_in_order(range::<KIB4>(top, base, a, b));
                yields_in_order(range::<MIB2>(top, base, a, b));
                exact_size(range::<KIB4>(top, base, a, b));
                exact_size(range::<MIB2>(top, base, a, b));
                nth(range::<KIB4>(top, base, a, b), n);
                nth(range::<MIB2>(top, base, a, b), n);
            }
        }
    }
}
