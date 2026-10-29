#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::str::FromStr;

use arch::interface::init::InitCapability;
use kernel::serial_println;
use platform::{
    TargetPlatform,
    interface::power::{ExitCode, ShutdownCapability},
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
