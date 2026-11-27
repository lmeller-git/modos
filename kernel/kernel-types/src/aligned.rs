//! Values with an alignment fixed at compile time.

use core::iter::FusedIterator;

use derive_more::{AsRef, Binary, Display, LowerHex, Octal, UpperHex};

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
    AsRef,
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

    /// The aligned value at or above `value`, if it is representable.
    pub fn round_up(value: T) -> Option<Self> {
        const { assert!(ALIGN.is_power_of_two(), "alignment must be a power of two") };
        let raw = value.to_usize().checked_next_multiple_of(ALIGN)?;
        Some(Self(T::from_usize(raw)))
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlignedRange<T, const ALIGN: usize> {
    start: Aligned<T, ALIGN>,
    end: Aligned<T, ALIGN>,
}

impl<T: Address, const ALIGN: usize> AlignedRange<T, ALIGN> {
    /// The largest aligned range inside `[start, end)`.
    pub fn within(start: T, end: T) -> Self {
        let end = Aligned::containing(end);
        let start = Aligned::round_up(start).unwrap_or(end);
        Self { start, end }
    }

    pub fn start(&self) -> Aligned<T, ALIGN> {
        self.start
    }

    /// Exclusive.
    pub fn end(&self) -> Aligned<T, ALIGN> {
        self.end
    }

    /// Splits off the first `n` values (or all, if fewer remain) as their own range.
    pub fn take_front(&mut self, n: usize) -> Self {
        let n = n.min(self.len());
        // `n <= len`, so this stays at or below `end`.
        let mid = Aligned(T::from_usize(self.start.0.to_usize() + n * ALIGN));
        let front = Self {
            start: self.start,
            end: mid,
        };
        self.start = mid;
        front
    }
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

impl<T: Address, const ALIGN: usize> DoubleEndedIterator for AlignedRange<T, ALIGN> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }
        // `start < end` and both are aligned, so `end - ALIGN >= start`.
        self.end = Aligned(T::from_usize(self.end.0.to_usize() - ALIGN));
        Some(self.end)
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

            /// The smallest multiple at or above `raw`, `None` past the top of `usize`.
            #[test]
            fn round_up(raw in (edge_addr(KIB4), edge_addr(MIB2))) {
                let expected = |raw: usize, align: usize| {
                    usize::try_from((raw as u128).div_ceil(align as u128) * align as u128).ok()
                };
                let up4 = Aligned::<usize, KIB4>::round_up(raw.0).map(Aligned::get);
                let up2m = Aligned::<usize, MIB2>::round_up(raw.1).map(Aligned::get);
                prop_assert_eq!(up4, expected(raw.0, KIB4));
                prop_assert_eq!(up2m, expected(raw.1, MIB2));
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

        /// Iterating from the back yields the same values in reverse, also when mixed with
        /// `next`.
        fn from_back<const ALIGN: usize>(mut r: AlignedRange<usize, ALIGN>, front: usize) {
            let mut expected = reference(&r);
            let taken: Vec<usize> = r.by_ref().take(front).map(Aligned::get).collect();
            assert_eq!(
                taken,
                expected
                    .drain(..front.min(expected.len()))
                    .collect::<Vec<_>>()
            );
            expected.reverse();
            assert_eq!(r.rev().map(Aligned::get).collect::<Vec<_>>(), expected);
        }

        /// `take_front(n)` splits off the first `n` values; the rest stays in the range.
        fn take_front<const ALIGN: usize>(mut r: AlignedRange<usize, ALIGN>, n: usize) {
            let all = reference(&r);
            let split = n.min(all.len());
            let front = r.take_front(n);
            assert_eq!(values(front), all[..split]);
            assert_eq!(values(r), all[split..]);
        }

        /// Exactly the multiples of `ALIGN` lying wholly inside `[start, end)`, so that one
        /// `ALIGN`-sized block starting at each fits.
        fn within<const ALIGN: usize>(start: usize, end: usize) {
            let first = (start as u128).div_ceil(ALIGN as u128);
            let last = (end as u128) / ALIGN as u128;
            let expected: Vec<usize> = (first..last)
                .take(MAX_SPAN + 1)
                .map(|i| usize::try_from(i * ALIGN as u128).unwrap())
                .collect();
            let got: Vec<usize> = AlignedRange::<usize, ALIGN>::within(start, end)
                .take(MAX_SPAN + 1)
                .map(Aligned::get)
                .collect();
            assert_eq!(got, expected, "[{start:#x}, {end:#x})");
        }

        proptest! {
            #![proptest_config(config())]

            #[test]
            fn split(
                top: bool, base: usize, a in 0..MAX_SPAN, b in 0..MAX_SPAN, n in 0..MAX_SPAN + 6,
            ) {
                from_back(range::<KIB4>(top, base, a, b), n);
                from_back(range::<MIB2>(top, base, a, b), n);
                take_front(range::<KIB4>(top, base, a, b), n);
                take_front(range::<MIB2>(top, base, a, b), n);
            }

            #[test]
            fn within_bounds(
                start in prop_oneof![edge_addr(KIB4), usize::MAX - 3 * KIB4..=usize::MAX],
                len in prop_oneof![0..8 * KIB4, any::<usize>()],
            ) {
                within::<KIB4>(start, start.saturating_add(len));
                within::<KIB4>(start.saturating_add(len), start);
                within::<MIB2>(start, start.saturating_add(len));
            }

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
