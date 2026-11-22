use alloc::boxed::Box;

use conquer_once::spin::OnceCell;
use x86_64::structures::idt::InterruptDescriptorTable;

use crate::{
    gdt,
    irq::{
        CoreIrqTable,
        IRQ_CONTROLLER,
        handlers::{
            breakpoint_handler,
            double_fault_handler,
            error_interrupt_handler,
            gpf_handler,
            page_fault_handler,
            spurious_interrupt_handler,
            timer_interrupt,
        },
        irq_dispatch,
    },
};

macro_rules! populate_external_vectors {
    ($idt:expr, $($hi:expr),*) => {
        $(
            $idt[($hi << 4) | 0x0].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x0 }>);
            $idt[($hi << 4) | 0x1].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x1 }>);
            $idt[($hi << 4) | 0x2].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x2 }>);
            $idt[($hi << 4) | 0x3].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x3 }>);
            $idt[($hi << 4) | 0x4].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x4 }>);
            $idt[($hi << 4) | 0x5].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x5 }>);
            $idt[($hi << 4) | 0x6].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x6 }>);
            $idt[($hi << 4) | 0x7].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x7 }>);
            $idt[($hi << 4) | 0x8].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x8 }>);
            $idt[($hi << 4) | 0x9].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0x9 }>);
            $idt[($hi << 4) | 0xA].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0xA }>);
            $idt[($hi << 4) | 0xB].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0xB }>);
            $idt[($hi << 4) | 0xC].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0xC }>);
            $idt[($hi << 4) | 0xD].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0xD }>);
            $idt[($hi << 4) | 0xE].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0xE }>);
            $idt[($hi << 4) | 0xF].set_handler_fn(irq_dispatch::<{ ($hi << 4) | 0xF }>);
        )*
    };
}

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

    populate_external_vectors!(
        idt, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xD, 0xE, 0xF
    );

    idt[InterruptIndex::Timer as u8].set_handler_fn(timer_interrupt);
    idt[InterruptIndex::Error as u8].set_handler_fn(error_interrupt_handler);
    idt[InterruptIndex::Spurious as u8].set_handler_fn(spurious_interrupt_handler);

    IDT.init_once(|| idt);
    IDT.get().unwrap().load();
}

pub(crate) fn init_core() {
    IRQ_CONTROLLER.with(|c| c.get_or_init(|| Box::new(CoreIrqTable::new())));
}

#[repr(u8)]
pub(crate) enum InterruptIndex {
    Timer = 0x20,
    // Keyboard = 0x21,
    Error = 0x21,
    Spurious = 0xFF,
}
