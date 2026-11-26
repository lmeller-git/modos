use core::ops::Add;

use derive_more::{Binary, Display, LowerHex, Octal, UpperHex};

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
pub struct VirtAddr(usize);

impl VirtAddr {
    pub fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub fn null() -> Self {
        Self(0)
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }

    pub fn as_ptr<T>(&self) -> *const T {
        self.as_usize() as *const T
    }

    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.as_ptr::<T>() as *mut T
    }

    pub fn align_up(&mut self, align: usize) {
        self.0 = align_up(self.0, align);
    }

    pub fn align_down(&mut self, align: usize) {
        self.0 = align_down(self.0, align);
    }
}

impl Add<usize> for VirtAddr {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

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
pub struct PhysAddr(usize);

impl PhysAddr {
    pub fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub fn null() -> Self {
        Self(0)
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }

    pub fn as_ptr<T>(&self) -> *const T {
        self.as_usize() as *const T
    }

    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.as_ptr::<T>() as *mut T
    }

    pub fn align_up(&mut self, align: usize) {
        self.0 = align_up(self.0, align);
    }

    pub fn align_down(&mut self, align: usize) {
        self.0 = align_down(self.0, align);
    }
}

impl Add<usize> for PhysAddr {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

#[inline]
pub const fn align_down(addr: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    addr & !(align - 1)
}

#[inline]
pub const fn align_up(addr: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    let align_mask = align - 1;
    if addr & align_mask == 0 {
        addr
    } else {
        (addr | align_mask)
            .checked_add(1)
            .expect("attempt to add with overflow")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::{config, edge_addr, panics};

    use super::*;

    /// `align = 2^shift` with an address biased towards the edges of an `align` block.
    fn addr_and_align() -> impl Strategy<Value = (usize, usize)> {
        (0..usize::BITS).prop_flat_map(|shift| {
            let align = 1usize << shift;
            edge_addr(align).prop_map(move |addr| (addr, align))
        })
    }

    mod align {
        use super::*;

        proptest! {
            #![proptest_config(config())]

            #[test]
            fn down((addr, align) in addr_and_align()) {
                let down = align_down(addr, align);
                prop_assert_eq!(down, addr - addr % align);
                prop_assert_eq!(align_down(down, align), down);
            }

            #[test]
            fn up((addr, align) in addr_and_align()) {
                let expected = (addr as u128).div_ceil(align as u128) * align as u128;
                match usize::try_from(expected) {
                    Ok(expected) => {
                        let up = align_up(addr, align);
                        prop_assert_eq!(up, expected);
                        prop_assert_eq!(align_up(up, align), up);
                    }
                    Err(_) => prop_assert!(panics(|| align_up(addr, align))),
                }
            }
        }

        // Release builds skip the check and compute garbage, so only debug is pinned.
        #[cfg(debug_assertions)]
        #[test]
        fn rejects_non_power_of_two() {
            for align in [0, 3, 6, 4095] {
                assert!(panics(|| align_down(0x1234, align)), "down {align}");
                assert!(panics(|| align_up(0x1234, align)), "up {align}");
            }
        }
    }
}
