#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::{hint::spin_loop, str::FromStr};

use arch::interface::{cpu::SmpCapability, init::InitCapability};
use boot::mp::BootCpu;
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

    early_heap::init();

    let string = String::from_str("Hello world");
    serial_println!("We allocated this on the heap: {:?}", string);

    let late_info = boot_info.into_late();

    arch::TargetArch::late_init(&init_once, &late_info).unwrap();
    serial_println!("Completed late init");

    arch::TargetArch::core_release(init_once, late_info, core_main)
}

fn core_main(cpu: BootCpu) -> ! {
    serial_println!("Hello from cpu {:?}", cpu);
    if cpu.is_bsp {
        // TODO
        // cleanup
    }

    for _ in 0..1000000 {
        spin_loop();
    }

    TargetPlatform::shutdown(ExitCode::Success)
}
