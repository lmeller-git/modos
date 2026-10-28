#![no_std]

use core::ptr::NonNull;

use crate::memmap::Entry;

mod limine;

pub struct BootLoaderInfo {
    pub name: &'static str,
    pub version: &'static str,
}

pub struct BootInfo {
    pub hhdm_offset: Option<usize>,
    pub bootloader_info: Option<BootLoaderInfo>,
    pub rsdp_addr: Option<NonNull<()>>,
    pub memmap: Option<&'static [&'static Entry]>,
}

pub mod memmap {
    // Copyright © 2026, Julian Scheffers
    // SPDX-License-Identifier: MIT OR Apache-2.0

    pub const MEMMAP_USABLE: u64 = 0;
    pub const MEMMAP_RESERVED: u64 = 1;
    pub const MEMMAP_ACPI_RECLAIMABLE: u64 = 2;
    pub const MEMMAP_ACPI_NVS: u64 = 3;
    pub const MEMMAP_BAD_MEMORY: u64 = 4;
    pub const MEMMAP_BOOTLOADER_RECLAIMABLE: u64 = 5;
    pub const MEMMAP_EXECUTABLE_AND_MODULES: u64 = 6;
    pub const MEMMAP_FRAMEBUFFER: u64 = 7;
    pub const MEMMAP_MAPPED_RESERVED: u64 = 8;

    #[repr(C)]
    pub struct Entry {
        pub base: u64,
        pub length: u64,
        pub type_: u64,
    }
}

pub fn build_bootinfo() -> BootInfo {
    limine::build_bootinfo()
}
