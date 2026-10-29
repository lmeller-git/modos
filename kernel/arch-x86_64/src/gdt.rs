use core::ptr;

use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::{
    VirtAddr,
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
};

pub(crate) const DOUBLE_FAULT_IST_INDEX: u16 = 0;

struct Selectors {
    code_selector: SegmentSelector,
    data_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

static TSS: OnceCell<Mutex<TaskStateSegment>> = OnceCell::uninit();

static GDT: OnceCell<(GlobalDescriptorTable, Selectors)> = OnceCell::uninit();

pub(crate) fn init_tss() -> &'static TaskStateSegment {
    TSS.init_once(|| {
        Mutex::new({
            let mut tss = TaskStateSegment::new();
            tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
                const STACK_SIZE: usize = 4096 * 5;
                static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

                let stack_start = VirtAddr::from_ptr(&raw const STACK);
                stack_start + STACK_SIZE as u64
            };
            tss
        })
    });
    unsafe { &*(&*TSS.get_unchecked().lock() as *const TaskStateSegment) }
}

pub(crate) fn init_gdt(tss: &'static TaskStateSegment) {
    GDT.init_once(|| {
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
    });
}

pub(crate) fn init() {
    use x86_64::instructions::{
        segmentation::{CS, SS, Segment},
        tables::load_tss,
    };

    let tss = init_tss();

    assert!(ptr::eq(tss, &*TSS.get().unwrap().lock()));

    init_gdt(tss);

    unsafe {
        let gdt = GDT.get_unchecked();
        gdt.0.load();
        CS::set_reg(gdt.1.code_selector);
        SS::set_reg(gdt.1.data_selector);
        load_tss(gdt.1.tss_selector);
    }
}
