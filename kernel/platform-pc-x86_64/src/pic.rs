//! Legacy 8259 PIC shutdown.

use x86_64::instructions::{interrupts, port::Port};

/// Vector base the PICs are remapped to before being masked, so that even a spurious IRQ7/15
/// cannot alias a CPU exception (0x00..0x20). Spurious IRQs still arrive at BASE+7 / BASE+15.
const PIC_VECTOR_BASE: u8 = 0xE0;

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;
const IO_WAIT_PORT: u16 = 0x80;

const ICW1_INIT_ICW4: u8 = 0x11;
const ICW3_MASTER_SLAVE_ON_IRQ2: u8 = 1 << 2;
const ICW3_SLAVE_ID: u8 = 2;
const ICW4_8086: u8 = 0x01;
const MASK_ALL: u8 = 0xFF;

/// each chip must see ICW1..ICW4 in sequence, then the mask.
const SEQUENCE: [(u16, u8); 10] = [
    (PIC1_CMD, ICW1_INIT_ICW4),
    (PIC2_CMD, ICW1_INIT_ICW4),
    (PIC1_DATA, PIC_VECTOR_BASE),
    (PIC2_DATA, PIC_VECTOR_BASE + 8),
    (PIC1_DATA, ICW3_MASTER_SLAVE_ON_IRQ2),
    (PIC2_DATA, ICW3_SLAVE_ID),
    (PIC1_DATA, ICW4_8086),
    (PIC2_DATA, ICW4_8086),
    (PIC1_DATA, MASK_ALL),
    (PIC2_DATA, MASK_ALL),
];

/// Remap and mask both 8259s.
///
/// This is done unconditionally: on machines without PICs the writes are harmless, and on
/// machines that report them (MADT `PCAT_COMPAT`) we must not leave them live.
/// If `route_imcr` is set, the IMCR is also switched so that external interrupts reach the APIC
/// instead of being wired straight to the CPU INTR pin.
pub(crate) fn disable(route_imcr: bool) {
    interrupts::without_interrupts(|| {
        let mut wait = Port::<u8>::new(IO_WAIT_PORT);

        for (port, value) in SEQUENCE {
            unsafe {
                Port::<u8>::new(port).write(value);
                wait.write(0);
            }
        }

        if route_imcr {
            const IMCR_SELECT: u16 = 0x22;
            const IMCR_DATA: u16 = 0x23;
            unsafe {
                Port::<u8>::new(IMCR_SELECT).write(0x70);
                Port::<u8>::new(IMCR_DATA).write(0x01);
            }
        }
    });
}
