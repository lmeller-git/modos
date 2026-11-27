//! Values with an alignment fixed at compile time.

use core::iter::FusedIterator;

use derive_more::{Binary, Display, LowerHex, Octal, UpperHex};

use crate::addr::{PhysAddr, VirtAddr, align_down};

/// An address-like value that [`Aligned`] can hold.
pub trait Address: Copy + Ord {
    fn to_usize(self) -> usize;
    fn from_usize(raw: usize) -> Self;
}

impl Address for usize {
    fn to_usize(self) -> usize {
        self
    }

    fn from_usize(raw: usize) -> Self {
        raw
    }
}

impl Address for VirtAddr {
    fn to_usize(self) -> usize {
        self.as_usize()
    }

    fn from_usize(raw: usize) -> Self {
        Self::new(raw)
    }
}

impl Address for PhysAddr {
    fn to_usize(self) -> usize {
        self.as_usize()
    }

    fn from_usize(raw: usize) -> Self {
        Self::new(raw)
    }
}

/// A `T` that is a multiple of `ALIGN`. `ALIGN` must be a power of two.
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
    Binary,
    Octal,
    LowerHex,
    UpperHex,
)]
#[repr(transparent)]
pub struct Aligned<T, const ALIGN: usize>(T);

impl<T: Address, const ALIGN: usize> Aligned<T, ALIGN> {
    /// `value`, if it is aligned.
    pub fn new(value: T) -> Option<Self> {
        const { assert!(ALIGN.is_power_of_two(), "alignment must be a power of two") };
        value
            .to_usize()
            .is_multiple_of(ALIGN)
            .then_some(Self(value))
    }

    /// The aligned value at or below `value`.
    pub fn containing(value: T) -> Self {
        const { assert!(ALIGN.is_power_of_two(), "alignment must be a power of two") };
        Self(T::from_usize(align_down(value.to_usize(), ALIGN)))
    }

    pub fn get(self) -> T {
        self.0
    }

    /// `[start, end)` in steps of `ALIGN`; empty if `end <= start`.
    pub fn range(start: Self, end: Self) -> AlignedRange<T, ALIGN> {
        AlignedRange { start, end }
    }
}

/// Iterator over `[start, end)` in steps of `ALIGN`, see [`Aligned::range`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignedRange<T, const ALIGN: usize> {
    start: Aligned<T, ALIGN>,
    end: Aligned<T, ALIGN>,
}

impl<T: Address, const ALIGN: usize> Iterator for AlignedRange<T, ALIGN> {
    type Item = Aligned<T, ALIGN>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }
        let item = self.start;
        // `start < end` and both are aligned, so this stays at or below `end`.
        self.start = Aligned(T::from_usize(item.0.to_usize() + ALIGN));
        Some(item)
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        if n >= self.len() {
            self.start = self.end;
            return None;
        }
        // `n < len`, so `n * ALIGN` stays inside the range.
        self.start = Aligned(T::from_usize(self.start.0.to_usize() + n * ALIGN));
        self.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = if self.start < self.end {
            (self.end.0.to_usize() - self.start.0.to_usize()) / ALIGN
        } else {
            0
        };
        (len, Some(len))
    }
}

impl<T: Address, const ALIGN: usize> ExactSizeIterator for AlignedRange<T, ALIGN> {}

impl<T: Address, const ALIGN: usize> FusedIterator for AlignedRange<T, ALIGN> {}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use proptest::prelude::*;
    use test_host::{config, edge_addr};

    use super::*;

    const KIB4: usize = 4096;
    const MIB2: usize = 2 << 20;
    const GIB1: usize = 1 << 30;

    mod aligned {
        use super::*;

        fn containing<const ALIGN: usize>(raw: usize) {
            let start = Aligned::<VirtAddr, ALIGN>::containing(VirtAddr::new(raw));
            assert_eq!(start.get().as_usize(), raw - raw % ALIGN);
        }

        /// Aligned values are accepted unchanged, unaligned ones rejected.
        fn new<const ALIGN: usize>(raw: usize) {
            let expected = raw.is_multiple_of(ALIGN).then_some(raw);
            let virt = Aligned::<VirtAddr, ALIGN>::new(VirtAddr::new(raw));
            let phys = Aligned::<PhysAddr, ALIGN>::new(PhysAddr::new(raw));

            assert_eq!(virt.map(|a| a.get().as_usize()), expected);
            assert_eq!(phys.map(|a| a.get().as_usize()), expected);
        }

        proptest! {
            #![proptest_config(config())]

            #[test]
            fn containing_value(
                raw in (edge_addr(1), edge_addr(KIB4), edge_addr(MIB2), edge_addr(GIB1)),
            ) {
                containing::<1>(raw.0);
                containing::<KIB4>(raw.1);
                containing::<MIB2>(raw.2);
                containing::<GIB1>(raw.3);
            }

            #[test]
            fn new_alignment(raw in (edge_addr(KIB4), edge_addr(MIB2))) {
                new::<KIB4>(raw.0);
                new::<MIB2>(raw.1);
            }
        }
    }

    mod range {
        use super::*;

        const MAX_SPAN: usize = 64;

        /// `[start, end)` as multiples of `ALIGN`: anywhere in the low address space or at the
        /// very top, with `a`/`b` unordered so empty and inverted ranges occur.
        fn range<const ALIGN: usize>(
            top: bool,
            base: usize,
            a: usize,
            b: usize,
        ) -> AlignedRange<usize, ALIGN> {
            let last = usize::MAX / ALIGN;
            let base = if top {
                last - MAX_SPAN - base % 4
            } else {
                base % (1 << 20)
            };
            let at = |i: usize| Aligned::<usize, ALIGN>::new((base + i) * ALIGN).unwrap();
            Aligned::range(at(a), at(b))
        }

        fn reference<const ALIGN: usize>(r: &AlignedRange<usize, ALIGN>) -> Vec<usize> {
            (r.start.get()..r.end.get()).step_by(ALIGN).collect()
        }

        fn values<const ALIGN: usize>(iter: AlignedRange<usize, ALIGN>) -> Vec<usize> {
            iter.map(Aligned::get).collect()
        }

        fn yields_in_order<const ALIGN: usize>(r: AlignedRange<usize, ALIGN>) {
            let expected = reference(&r);
            assert_eq!(r.len(), expected.len());
            assert_eq!(values(r), expected);
        }

        /// `nth(n)` is `skip(n).next()`, and iteration resumes after the skipped values.
        fn nth<const ALIGN: usize>(mut r: AlignedRange<usize, ALIGN>, n: usize) {
            let all = reference(&r);

            assert_eq!(r.nth(n).map(Aligned::get), all.get(n).copied());
            assert_eq!(r.len(), all.len().saturating_sub(n + 1));
            assert_eq!(values(r), all.into_iter().skip(n + 1).collect::<Vec<_>>());
        }

        proptest! {
            #![proptest_config(config())]

            #[test]
            fn iteration(
                top: bool, base: usize, a in 0..MAX_SPAN, b in 0..MAX_SPAN, n in 0..MAX_SPAN + 6,
            ) {
                yields_in_order(range::<KIB4>(top, base, a, b));
                yields_in_order(range::<MIB2>(top, base, a, b));
                nth(range::<KIB4>(top, base, a, b), n);
                nth(range::<MIB2>(top, base, a, b), n);
            }
        }
    }
}
