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
    pub cls: *const [u8],
    /// cls bases of all cores indexed by `logical_id`.
    pub cls_table: &'static [AtomicPtr<u8>],
}

impl CPUData {
    /// Total count of cores.
    pub fn cpu_count(&self) -> usize {
        self.cls_table.len()
    }

    /// Makes this cores cls visible to other cores.
    pub fn publish_cls(&self) {
        self.cls_table[self.cpu.logical_id.value()]
            .store(self.cls.cast::<u8>().cast_mut(), Ordering::Release);
    }
}

pub fn my_cpu() -> &'static CPUData {
    unsafe { &*GsBase::read().as_ptr() }
}

impl CpuDataCapability for Primitive {
    fn cpu_count() -> usize {
        my_cpu().cpu_count()
    }

    fn my_cpu() -> BootCpu {
        my_cpu().cpu
    }

    fn public_data_base() -> *const () {
        my_cpu().cls.cast()
    }

    fn public_data_base_of(cpu: CpuId) -> Option<*const ()> {
        let base = my_cpu().cls_table.get(cpu.value())?.load(Ordering::Acquire);
        (!base.is_null()).then_some(base.cast_const().cast())
    }

    fn cpus() -> impl Iterator<Item = CpuId> {
        (0..my_cpu().cpu_count()).map(CpuId::new)
    }
}
