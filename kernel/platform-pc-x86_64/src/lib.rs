#![no_std]

extern crate alloc;

mod interrupt;
mod ioapic;
mod pic;

use core::fmt::Write;

pub use kernel_acpi::{InterruptConfig, InterruptConfigError, MapMmioFn, NanosFn};
use platform_interfaces::{
    power::{ExitCode, ShutdownCapability},
    serial::{SerialWriteError, SerialWriterCapability},
};
use spin::{Lazy, Mutex};
use uart_16550::SerialPort;

pub struct PcX86_64;

#[cfg(feature = "qemu-platform")]
impl ShutdownCapability for PcX86_64 {
    fn shutdown(code: ExitCode) -> ! {
        unsafe {
            use x86_64::instructions::port::Port;
            let mut port = Port::<u32>::new(0xf4);
            let value = match code {
                ExitCode::Success => 0x10,
                ExitCode::Failure => 0x11,
            };
            port.write(value);
        }
        unreachable!()
    }

    fn restart() -> ! {
        panic!("not supported")
    }
}

#[cfg(not(feature = "qemu-platform"))]
impl ShutdownCapability for PcX86_64 {
    fn shutdown(_: ExitCode) -> ! {
        todo!()
    }

    fn restart() -> ! {
        todo!()
    }
}

impl SerialWriterCapability for PcX86_64 {
    fn write_str(msg: &str) -> Result<(), SerialWriteError> {
        SERIAL
            .lock()
            .write_str(msg)
            .map_err(|_| SerialWriteError::Unknown { msg: "unknown" })
    }

    fn write_fmt(args: core::fmt::Arguments<'_>) -> Result<(), SerialWriteError> {
        SERIAL
            .lock()
            .write_fmt(args)
            .map_err(|_| SerialWriteError::Unknown { msg: "unknown" })
    }
}

static SERIAL: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    port.into()
});
