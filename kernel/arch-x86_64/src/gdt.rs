use arch_x86_64_primitives::Selectors;
use mem_layout::{IST_STACK_SIZE, IST_STACK_START};
use x86_64::{
    VirtAddr,
    instructions::tables::load_tss,
    registers::segmentation::{CS, SS, Segment},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable},
        paging::{FrameAllocator, Mapper, Page, PageSize, PageTableFlags, Size4KiB},
        tss::TaskStateSegment,
    },
};

pub(crate) const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub(crate) const NMI_IST_INDEX: u16 = 1;
const IST_STACKS_PER_CORE: usize = 2;

const _: () = {
    assert!((DOUBLE_FAULT_IST_INDEX as usize) < IST_STACKS_PER_CORE);
    assert!((NMI_IST_INDEX as usize) < IST_STACKS_PER_CORE);
    assert!(DOUBLE_FAULT_IST_INDEX != NMI_IST_INDEX);
};

/// Lowest address of IST stack `slot` of core `core_id`.
///
/// Stacks are laid out back to back with one unmapped guard page above each, which is the
/// guard page below the next stack.
fn ist_stack_base(core_id: usize, slot: u16) -> usize {
    let index = core_id * IST_STACKS_PER_CORE + slot as usize;
    IST_STACK_START + index * (IST_STACK_SIZE + Size4KiB::SIZE as usize) + Size4KiB::SIZE as usize
}

fn build_ist_stack(core_id: usize, slot: u16) -> VirtAddr {
    let our_ist_base = ist_stack_base(core_id, slot);
    let page_range = Page::<Size4KiB>::range(
        Page::containing_address(VirtAddr::new(our_ist_base as u64)),
        Page::containing_address(VirtAddr::new((our_ist_base + IST_STACK_SIZE) as u64)),
    );

    let mut frame_alloc = crate::frame::get_frame_alloc().lock();
    let mut pagetable = crate::page::get_pagetable().lock();

    for page in page_range {
        let frame = frame_alloc.allocate_frame().unwrap();
        unsafe {
            pagetable.map_to(
                page,
                frame,
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
                &mut *frame_alloc,
            )
        }
        .unwrap()
        .flush();
    }

    drop(frame_alloc);
    drop(pagetable);

    VirtAddr::new((our_ist_base + IST_STACK_SIZE) as u64)
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

#[cfg(test)]
mod tests {
    use mem_layout::MMIO_START;
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    const MAX_CORES: usize = 1 << 16;
    const PAGE: usize = Size4KiB::SIZE as usize;

    proptest! {
        #![proptest_config(config())]

        /// Stacks never overlap, are separated by a guard page, and stay clear of the regions
        /// above. Stacks are compared with their neighbours, where bugs show.
        #[test]
        fn ist_stacks(
            first in prop_oneof![0..8usize, 0..MAX_CORES * IST_STACKS_PER_CORE],
            gap in 0..4usize,
        ) {
            let base = |index: usize| {
                ist_stack_base(index / IST_STACKS_PER_CORE, (index % IST_STACKS_PER_CORE) as u16)
            };
            let (a, b) = (first, first + gap);

            prop_assert_eq!(base(a) % PAGE, 0);
            prop_assert!(base(a) >= IST_STACK_START);
            prop_assert!(base(b) + IST_STACK_SIZE + PAGE <= MMIO_START);
            prop_assert_eq!(base(a) == base(b), a == b);
            if a != b {
                prop_assert!(base(a) + IST_STACK_SIZE + PAGE <= base(b));
            }
        }
    }
}
