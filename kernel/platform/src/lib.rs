#![no_std]

use core::fmt::Arguments;

pub use platform_interfaces as interface;
use platform_interfaces::{
    interrupt::InterruptControllerCapability,
    power::ShutdownCapability,
    serial::SerialWriterCapability,
};
#[cfg(target_arch = "x86_64")]
use platform_pc_x86_64::PcX86_64;

#[cfg(target_arch = "x86_64")]
pub type TargetPlatform = PcX86_64;
#[cfg(not(target_arch = "x86_64"))]
pub struct TargetPlatform;

#[cfg(target_arch = "x86_64")]
pub use platform_pc_x86_64::InterruptConfig;

pub trait SupportedPlatform:
    ShutdownCapability + SerialWriterCapability + InterruptControllerCapability
{
}

impl SupportedPlatform for TargetPlatform {}

#[doc(hidden)]
pub fn _serial_print(args: Arguments<'_>) {
    TargetPlatform::write_fmt(args).unwrap();
}
