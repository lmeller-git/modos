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
    arch::TargetArch::early_init(&boot_info).unwrap();
    early_heap::init();
    let string = String::from_str("Hello world");
    serial_println!("We allocated this on the heap: {:?}", string);
    let late_info = boot_info.into_late();
    arch::TargetArch::late_init(&late_info).unwrap();
    serial_println!("Completed late init");
    let comm = arch::TargetArch::comm_init(&late_info);
    arch::TargetArch::ap_init(&late_info, core_main, comm.clone());
    arch::TargetArch::ap_release(comm);

    for _ in 0..10000 {
        spin_loop();
    }

    TargetPlatform::shutdown(ExitCode::Success)
}

fn core_main(cpu: BootCpu) -> ! {
    serial_println!("Hello from cpu {:?}", cpu);
    loop {
        spin_loop();
    }
}
