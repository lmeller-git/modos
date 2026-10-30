#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use core::ptr::NonNull;

use crate::{memmap::Entry, mp::MpBootInfo};

#[cfg(feature = "limine")]
mod limine;

pub struct BootLoaderInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

pub struct EarlyBootInfo<'a> {
    pub hhdm_offset: Option<usize>,
    pub bootloader_info: Option<BootLoaderInfo<'a>>,
    pub rsdp_addr: Option<NonNull<()>>,
    pub memmap: Option<&'a [&'a Entry]>,
}

impl<'a> EarlyBootInfo<'a> {
    pub fn into_late(&self) -> LateBootInfo {
        build_late_bootinfo(self)
    }

    pub unsafe fn release(self) {
        todo!()
    }
}

pub struct LateBootInfo {
    pub hhdm_offset: Option<usize>,
    pub bootloader_info: Option<BootLoaderInfo<'static>>,
    pub rsdp_addr: Option<NonNull<()>>,
    pub memmap: Option<Box<[Entry]>>,
    pub mp_info: Option<MpBootInfo>,
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(C)]
    pub struct Entry {
        pub base: u64,
        pub length: u64,
        pub type_: u64,
    }
}

pub mod mp {
    use alloc::boxed::Box;

    #[derive(Clone, Copy)]
    pub struct BootCpu {
        pub logical_id: usize,
        pub hw_id: u32,
        pub is_bsp: bool,
    }

    pub type CpuEntry = fn(&BootCpu, usize) -> !;

    pub struct MpBootInfo {
        pub flags: u32,
        pub cpus: Box<[BootCpu]>,
        pub start_cpu: fn(&BootCpu, CpuEntry, usize),
    }
}

pub fn build_bootinfo<'a>() -> EarlyBootInfo<'a> {
    #[cfg(feature = "limine")]
    limine::build_early_bootinfo()
}

fn build_late_bootinfo(early_boot: &EarlyBootInfo<'_>) -> LateBootInfo {
    #[cfg(feature = "limine")]
    limine::build_late_bootinfo(early_boot)
}
