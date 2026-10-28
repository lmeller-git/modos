#![no_std]
#![no_main]

#[cfg(not(test))]
use core::panic::PanicInfo;

use arch_x86_64::{exit, serial_println};

#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    let _boot_info = boot::build_bootinfo();
    serial_println!("Hello there!");
    exit(0)
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("Panic: {:?}", info);
    exit(1)
}
