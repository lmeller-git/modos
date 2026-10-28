#![no_std]

use core::ptr::NonNull;

mod limine;

pub struct BootLoaderInfo {
    pub name: &'static str,
    pub version: &'static str,
}

pub struct BootInfo {
    pub hhdm_offset: Option<usize>,
    pub bootloader_info: Option<BootLoaderInfo>,
    pub rsdp_addr: Option<NonNull<()>>,
}

pub fn build_bootinfo() -> BootInfo {
    limine::build_bootinfo()
}
