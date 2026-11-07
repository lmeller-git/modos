#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
#[cfg(not(test))]
use core::panic::PanicInfo;
use core::{hint::spin_loop, str::FromStr};

use arch::interface::{
    cpu::{CpuDataCapability, SmpCapability},
    init::InitCapability,
};
use kernel::serial_println;
use platform::{
    TargetPlatform,
    interface::power::{ExitCode, ShutdownCapability},
};

#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    let boot_info = boot::build_bootinfo();
    serial_println!("Hello there!");

    let init_once = arch::TargetArch::initialize().unwrap();

    arch::TargetArch::early_init(&init_once, &boot_info).unwrap();

    early_heap::init(&GLOBAL_ALLOCATOR);

    let string = String::from_str("Hello world");
    serial_println!("We allocated this on the heap: {:?}", string);

    let late_info = boot_info.into_late();

    arch::TargetArch::late_init(&init_once, &late_info).unwrap();
    serial_println!("Completed late init");

    arch::TargetArch::core_release(init_once, late_info, core_main)
}

fn core_main() -> ! {
    let cpu = arch::TargetArch::my_cpu();
    serial_println!("Hello from cpu {:?}", cpu);

    for _ in 0..1000000 {
        spin_loop();
    }

    TargetPlatform::shutdown(ExitCode::Success)
}

#[cfg(not(test))]
#[panic_handler]
fn kernel_panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("Panic: {:?}", info);
    TargetPlatform::shutdown(ExitCode::Failure)
}

#[global_allocator]
pub static GLOBAL_ALLOCATOR: early_heap::linked_list::SafeHeap =
    early_heap::linked_list::get_alloc();
