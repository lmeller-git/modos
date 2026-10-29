#![no_std]

use core::fmt::Arguments;
#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
use platform::interface::power::{ExitCode, ShutdownCapability};
use platform::{TargetPlatform, interface::serial::SerialWriterCapability};

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

pub fn _serial_print(args: Arguments<'_>) {
    TargetPlatform::write_fmt(args).unwrap();
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("Panic: {:?}", info);
    TargetPlatform::shutdown(ExitCode::Failure)
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
