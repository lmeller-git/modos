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
        self.0 = align_up(self.0, align)
    }

    pub fn align_down(&mut self, align: usize) {
        self.0 = align_down(self.0, align)
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
        self.0 = align_up(self.0, align)
    }

    pub fn align_down(&mut self, align: usize) {
        self.0 = align_down(self.0, align)
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
