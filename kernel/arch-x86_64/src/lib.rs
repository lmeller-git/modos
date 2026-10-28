#![no_std]

use core::fmt::Arguments;

use boot::BootInfo;
use spin::{Mutex, lazy::Lazy};
use uart_16550::SerialPort;
use x86_64::registers::control::{Cr4, Cr4Flags};

pub mod frame;
pub mod page;
mod qemu;

static SERIAL: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    port.into()
});

pub fn exit(code: usize) -> ! {
    qemu::exit_qemu(qemu::QemuExitCode::from_usize(code));
}

pub fn early_init(boot_info: &BootInfo) {
    frame::init_frame_alloc(boot_info.memmap.unwrap(), boot_info.hhdm_offset.unwrap());
    init_xmm();
}

fn init_xmm() {
    unsafe {
        Cr4::update(|cr4| {
            cr4.insert(Cr4Flags::OSFXSR);
            cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
        });
    }
}

pub fn _serial_print(args: Arguments<'_>) {
    use core::fmt::Write;
    SERIAL
        .lock()
        .write_fmt(args)
        .expect("Printing to serial failed")
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial_print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::serial_print!("{}\n", format_args!($($arg)*))
    };
}
#[macro_export]
macro_rules! serial_print {
    () => {};
    ($($arg:tt)*) => {
        $crate::_serial_print(format_args!("\x1b[34m[KINFO]\x1b[0m {}", format_args!($($arg)*)))
    };
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
