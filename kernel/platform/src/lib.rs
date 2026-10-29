#![no_std]

pub use platform_interfaces as interface;
use platform_interfaces::{power::ShutdownCapability, serial::SerialWriterCapability};
#[cfg(feature = "platform-pc-x86_64")]
use platform_pc_x86_64::PcQemu;

#[cfg(feature = "platform-pc-x86_64")]
pub type TargetPlatform = PcQemu;
#[cfg(not(feature = "platform-pc-x86_64"))]
pub struct TargetPlatform;

pub trait SupportedPlatform: ShutdownCapability + SerialWriterCapability {}

impl SupportedPlatform for TargetPlatform {}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
