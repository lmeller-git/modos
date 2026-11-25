//! PS/2 keyboard demo driver
//! TODO: move elsewhere

use alloc::boxed::Box;

use arch::{TargetArch, interface::cpu::CpuDataCapability};
use early_log::early_println;
use irq::{Affinity, IrqHandler, IrqNum, IrqReturn, x86::IsaIrqCapability};
use platform::TargetPlatform;
use x86_64::instructions::port::Port;

const STATUS_PORT: u16 = 0x64;
const DATA_PORT: u16 = 0x60;
const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_AUX: u8 = 1 << 5;

struct Keyboard;

impl IrqHandler for Keyboard {
    fn handle(&self, irq: IrqNum) -> IrqReturn {
        let status = unsafe { Port::<u8>::new(STATUS_PORT).read() };
        // spurious interrupt
        if status & STATUS_OUTPUT_FULL == 0 {
            return IrqReturn::NotMine;
        }
        let scancode = unsafe { Port::<u8>::new(DATA_PORT).read() };

        if status & STATUS_AUX == 0 {
            early_println!("received scancode: {} (irq {})", scancode, irq.raw());
        }
        IrqReturn::Handled
    }
}

/// Registers the keyboard on the BSP and leaves it registered for the kernel's lifetime.
pub(crate) fn init() {
    let spec = TargetPlatform::isa_irq(1).expect("failed to resolve the keyboard irq");
    let bsp = TargetArch::my_cpu().logical_id;
    let handle = irq::request_irq(spec, Box::new(Keyboard), Affinity::Cpu(bsp))
        .expect("failed to register the keyboard handler");

    // Drain whatever the controller buffered before the line was routed.
    for _ in 0..16 {
        if unsafe { Port::<u8>::new(STATUS_PORT).read() } & STATUS_OUTPUT_FULL == 0 {
            break;
        }
        _ = unsafe { Port::<u8>::new(DATA_PORT).read() };
    }

    handle.unmask();
    Box::leak(Box::new(handle));
}
