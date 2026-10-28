#![no_std]
#![no_main]

#[cfg(not(test))]
use core::panic::PanicInfo;

use arch_x86_64::exit;

#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    let _boot_info = boot::build_bootinfo();
    exit(0)
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(_: &PanicInfo<'_>) -> ! {
    exit(1)
}
