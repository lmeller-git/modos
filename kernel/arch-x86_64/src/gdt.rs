use arch_x86_64_primitives::Selectors;
use x86_64::{
    VirtAddr,
    instructions::tables::load_tss,
    registers::segmentation::{CS, SS, Segment},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable},
        tss::TaskStateSegment,
    },
};

use crate::stack::IST_STACKS;

pub(crate) const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub(crate) const NMI_IST_INDEX: u16 = 1;
const IST_STACKS_PER_CORE: usize = 2;

const _: () = {
    assert!((DOUBLE_FAULT_IST_INDEX as usize) < IST_STACKS_PER_CORE);
    assert!((NMI_IST_INDEX as usize) < IST_STACKS_PER_CORE);
    assert!(DOUBLE_FAULT_IST_INDEX != NMI_IST_INDEX);
};

/// Maps IST stack `slot` of core `core_id` and returns its top.
fn build_ist_stack(core_id: usize, slot: u16) -> VirtAddr {
    IST_STACKS.map(core_id * IST_STACKS_PER_CORE + usize::from(slot))
}

pub(crate) fn build_tss(core_id: usize) -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();

    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
        build_ist_stack(core_id, DOUBLE_FAULT_IST_INDEX);
    tss.interrupt_stack_table[NMI_IST_INDEX as usize] = build_ist_stack(core_id, NMI_IST_INDEX);

    tss
}

pub(crate) fn build_gdt(tss: &'static TaskStateSegment) -> (GlobalDescriptorTable, Selectors) {
    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(tss));
    let kernel_data_selector = gdt.append(Descriptor::kernel_data_segment());

    (
        gdt,
        Selectors {
            code_selector,
            tss_selector,
            data_selector: kernel_data_selector,
        },
    )
}

pub(crate) fn load_gdt_tss(gdt: &'static GlobalDescriptorTable, selectors: &Selectors) {
    unsafe {
        gdt.load();
        CS::set_reg(selectors.code_selector);
        SS::set_reg(selectors.data_selector);
        load_tss(selectors.tss_selector);
    }
}
