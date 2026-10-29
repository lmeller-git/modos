use conquer_once::spin::OnceCell;
use x86_64::structures::idt::InterruptDescriptorTable;

use crate::{
    gdt,
    irq::{
        SPURIOUS_VECTOR,
        breakpoint_handler,
        double_fault_handler,
        gpf_handler,
        keyboard_interrupt_handler,
        page_fault_handler,
        spurious_interrupt_handler,
        timer_interrupt,
    },
};

pub(crate) static IDT: OnceCell<InterruptDescriptorTable> = OnceCell::uninit();

pub(crate) fn init_idt() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);

    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt[InterruptIndex::Timer as u8].set_handler_fn(timer_interrupt);
    idt[InterruptIndex::Keyboard as u8].set_handler_fn(keyboard_interrupt_handler);
    idt[SPURIOUS_VECTOR].set_handler_fn(spurious_interrupt_handler);

    IDT.init_once(|| idt);

    IDT.get().unwrap().load();
}

#[repr(u8)]
pub(crate) enum InterruptIndex {
    Timer = 0x20,
    Keyboard = 0x21,
}
