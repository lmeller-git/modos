use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptStackFrame, PageFaultErrorCode},
};

use crate::{pic::end_interrupt, serial_println};

static TOTAL_TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub(super) extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    // println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
    panic!("breakpoint hit, but not supported: {stack_frame:?}");
}

pub(super) extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{stack_frame:#?}");
}

pub(super) extern "x86-interrupt" fn timer_interrupt(_stack_frame: InterruptStackFrame) {
    TOTAL_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    end_interrupt();
}

pub(crate) fn current_tick() -> u64 {
    TOTAL_TIMER_TICKS.load(Ordering::Acquire)
}

pub(super) extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let mut port = Port::<u8>::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    serial_println!("received scancode {}", scancode);
    end_interrupt();
}

pub(super) extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;
    panic!(
        "EXCEPTION Page fault:\naccessed address: {:?}\nerror code: {:?}\nstack_frame: {:?}",
        Cr2::read(),
        error_code,
        stack_frame
    )
}

pub(super) extern "x86-interrupt" fn gpf_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT\n{stack_frame:#?}\nError Code: {error_code:b}"
    );
}

pub(super) const SPURIOUS_VECTOR: u8 = 0xFF;

pub(super) extern "x86-interrupt" fn spurious_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // nothing to do
    serial_println!("spurious interrupt");
    end_interrupt();
}
