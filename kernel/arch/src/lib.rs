#![no_std]

// we reexport arch-interfaces, such that consumers doont have to depend on both crates always
pub use arch_interfaces as interface;
use arch_interfaces::{
    cpu::CpuDataCapability,
    init::InitCapability,
    interrupt::{InterruptHandlerCapability, IrqDeliveryCapability},
    mm::{
        FrameAllocatorCapability,
        FrameDeallocatorCapability,
        MemMapperCapability,
        MemUnmapperCapability,
    },
};

#[cfg(target_arch = "x86_64")]
pub type TargetArch = arch_x86_64::X86_64;
#[cfg(not(target_arch = "x86_64"))]
pub struct TargetArch;

pub trait SupportedArch:
    CpuDataCapability
    + InterruptHandlerCapability
    + IrqDeliveryCapability
    + FrameAllocatorCapability
    + FrameDeallocatorCapability
    + MemMapperCapability
    + MemUnmapperCapability
    + InitCapability
{
}

impl SupportedArch for TargetArch {}
