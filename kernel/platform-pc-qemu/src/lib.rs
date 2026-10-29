#![no_std]

use core::fmt::Write;

use platform_interfaces::{
    power::{ExitCode, ShutdownCapability},
    serial::SerialWriterCapability,
};
use spin::{Lazy, Mutex};
use uart_16550::SerialPort;

pub struct PcQemu;

impl ShutdownCapability for PcQemu {
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

impl SerialWriterCapability for PcQemu {
    fn write_str(msg: &str) -> Result<(), ()> {
        SERIAL.lock().write_str(msg).map_err(|_| {})
    }

    fn write_fmt(args: core::fmt::Arguments<'_>) -> Result<(), ()> {
        SERIAL.lock().write_fmt(args).map_err(|_| {})
    }
}

static SERIAL: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    port.into()
});

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
