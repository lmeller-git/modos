#![no_std]

pub use platform_interfaces as interface;
use platform_interfaces::{power::ShutdownCapability, serial::SerialWriterCapability};
#[cfg(feature = "platform-pc-qemu")]
use platform_pc_qemu::PcQemu;

#[cfg(feature = "platform-pc-qemu")]
pub type TargetPlatform = PcQemu;
#[cfg(not(feature = "platform-pc-qemu"))]
pub struct TargetPlatform;

#[expect(dead_code)]
fn t_supports<T>()
where
    T: ShutdownCapability + SerialWriterCapability,
{
}

#[expect(dead_code)]
fn target_paltform_supported() {
    t_supports::<TargetPlatform>();
}

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
