#![no_std]

use core::{mem::ManuallyDrop, ptr::NonNull};

use arch_interfaces::cpu::{CpuDataCapability, CpuId};
use boot::mp::BootCpu;
use x86_64::{
    registers::model_specific::GsBase,
    structures::{
        gdt::{GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
};

pub struct Primitive;

pub struct Selectors {
    pub code_selector: SegmentSelector,
    pub data_selector: SegmentSelector,
    pub tss_selector: SegmentSelector,
}

#[repr(C)]
pub struct CPUData {
    pub self_ptr: *const CPUData,
    pub cpu: BootCpu,
    pub stack: ManuallyDrop<NonNull<u8>>,
    pub gdt: GlobalDescriptorTable,
    pub selectors: Selectors,
    pub tss: &'static TaskStateSegment,
    pub cpus: &'static [CpuId],
    pub cls: *const [u8],
}

pub fn my_cpu() -> &'static CPUData {
    unsafe { &*GsBase::read().as_ptr() }
}

impl CpuDataCapability for Primitive {
    fn cpu_count() -> usize {
        my_cpu().cpus.len()
    }

    fn is_bootstrap_cpu() -> bool {
        my_cpu().cpu.is_bsp
    }

    fn my_cpu() -> BootCpu {
        my_cpu().cpu
    }

    fn public_data_base() -> *const () {
        my_cpu().cls.cast()
    }

    fn cpus() -> impl Iterator<Item = CpuId> {
        my_cpu().cpus.iter().copied()
    }
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
