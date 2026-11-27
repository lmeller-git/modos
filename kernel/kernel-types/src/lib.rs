//! Plain data types shared by every kernel crate.

#![no_std]

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod addr;
pub mod aligned;
pub mod cpu;
pub mod frame;
#[cfg(feature = "hhdm")]
pub mod hhdm;
