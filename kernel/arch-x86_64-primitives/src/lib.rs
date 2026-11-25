#![no_std]

use core::{
    mem::ManuallyDrop,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, Ordering},
};

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
    pub cls_table: &'static [AtomicPtr<u8>],
}

impl CPUData {
    /// Makes this core's cls visible to other cores.
    pub fn publish_cls(&self) {
        let idx = cpu_index(self.cpus, self.cpu.logical_id)
            .expect("core is not part of its own cpu list");
        self.cls_table[idx].store(self.cls.cast::<u8>().cast_mut(), Ordering::Release);
    }
}

/// Position of `id` in `cpus`.
fn cpu_index(cpus: &[CpuId], id: CpuId) -> Option<usize> {
    // dense, sorted logical ids
    if cpus.get(id.value()) == Some(&id) {
        return Some(id.value());
    }
    cpus.iter().position(|&cpu| cpu == id)
}

pub fn my_cpu() -> &'static CPUData {
    unsafe { &*GsBase::read().as_ptr() }
}

impl CpuDataCapability for Primitive {
    fn cpu_count() -> usize {
        my_cpu().cpus.len()
    }

    fn my_cpu() -> BootCpu {
        my_cpu().cpu
    }

    fn public_data_base() -> *const () {
        my_cpu().cls.cast()
    }

    fn public_data_base_of(cpu: CpuId) -> Option<*const ()> {
        let this = my_cpu();
        let idx = cpu_index(this.cpus, cpu)?;
        let base = this.cls_table[idx].load(Ordering::Acquire);
        (!base.is_null()).then_some(base.cast_const().cast())
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
