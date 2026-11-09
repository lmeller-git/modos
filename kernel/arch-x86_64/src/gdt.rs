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

pub(crate) fn build_tss(core_id: usize) -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();

    // allocate ist
    // one guard page between each stack
    // layout is
    // START_ADDRESS
    // ....
    // STACK_SIZE
    // ....
    // GUARD_PAGE
    // NEXT
    // ....
    let our_ist_base = IST_STACK_START + core_id * (IST_STACK_SIZE + Size4KiB::SIZE as usize);
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

    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
        VirtAddr::new((our_ist_base + IST_STACK_SIZE) as u64);
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
