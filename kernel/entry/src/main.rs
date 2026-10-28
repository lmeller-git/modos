#![no_std]
#![no_main]

#[cfg(not(test))]
use core::panic::PanicInfo;

pub fn kernel_main() {}

#[cfg(not(test))]
#[panic_handler]
fn panic_handler(_: &PanicInfo<'_>) -> ! {
    loop {}
}
