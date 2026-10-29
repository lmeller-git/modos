#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
#[cfg(not(test))]
use core::panic::PanicInfo;
use core::str::FromStr;

use arch_x86_64::{exit, serial_println};

#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    let boot_info = boot::build_bootinfo();
    serial_println!("Hello there!");
    arch_x86_64::early_init(&boot_info);
    early_heap::init();
    let string = String::from_str("Hello world");
    serial_println!("We allocated this on the heap: {:?}", string);
    arch_x86_64::late_init(&boot_info);
    serial_println!("Completed late init");
    exit(0)
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("Panic: {:?}", info);
    exit(1)
}
