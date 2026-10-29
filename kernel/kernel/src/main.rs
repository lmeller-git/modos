#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
#[cfg(not(test))]
use core::panic::PanicInfo;
use core::{fmt::Arguments, str::FromStr};

use arch::interface::init::InitCapability;
use platform::{
    TargetPlatform,
    interface::{
        power::{ExitCode, ShutdownCapability},
        serial::SerialWriterCapability,
    },
};

#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    let boot_info = boot::build_bootinfo();
    serial_println!("Hello there!");
    arch::TargetArch::early_init(&boot_info).unwrap();
    early_heap::init();
    let string = String::from_str("Hello world");
    serial_println!("We allocated this on the heap: {:?}", string);
    arch::TargetArch::late_init(&boot_info).unwrap();
    serial_println!("Completed late init");
    TargetPlatform::shutdown(ExitCode::Success)
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("Panic: {:?}", info);
    TargetPlatform::shutdown(ExitCode::Failure)
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

fn _serial_print(args: Arguments<'_>) {
    TargetPlatform::write_fmt(args).unwrap();
}
