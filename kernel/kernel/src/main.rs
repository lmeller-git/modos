#![no_std]
#![no_main]

#[cfg(not(test))]
use core::panic::PanicInfo;

#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    loop {}
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(_: &PanicInfo<'_>) -> ! {
    loop {}
}
