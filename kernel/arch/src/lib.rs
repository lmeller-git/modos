#![no_std]

pub mod mm;

// we reexport arch-interfaces, such that consumers doont have to depend on both crates always
pub use arch_interfaces as interface;
use arch_interfaces::{
    cpu::CpuHandlingCapability,
    init::InitCapability,
    interrupt::InterruptHandlingCapability,
    mm::{
        FrameAllocatorCapability,
        FrameDeAllocatorCapability,
        MemMapperCapability,
        MemUnmapperCapability,
    },
};

#[cfg(target_arch = "x86_64")]
pub type TargetArch = arch_x86_64::X86_64;
#[cfg(not(target_arch = "x86_64"))]
pub struct TargetArch;

pub trait SupportedArch:
    CpuHandlingCapability
    + InterruptHandlingCapability
    + FrameAllocatorCapability
    + FrameDeAllocatorCapability
    + MemMapperCapability
    + MemUnmapperCapability
    + InitCapability
{
}

impl SupportedArch for TargetArch {}

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
