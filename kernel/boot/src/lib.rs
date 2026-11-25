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
    pub tsc_hz: Option<u64>,
}

impl<'a> EarlyBootInfo<'a> {
    pub fn into_late(&self) -> LateBootInfo {
        build_late_bootinfo(self)
    }
}

pub struct LateBootInfo {
    pub hhdm_offset: Option<usize>,
    pub bootloader_info: Option<BootLoaderInfo<'static>>,
    pub rsdp_addr: Option<NonNull<()>>,
    pub memmap: Option<Box<[Entry]>>,
    pub mp_info: Option<MpBootInfo>,
    pub tsc_hz: Option<u64>,
}

unsafe impl Send for LateBootInfo {}
unsafe impl Sync for LateBootInfo {}

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
    use alloc::{boxed::Box, vec::Vec};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    #[repr(C)]
    pub struct BootCpu {
        pub logical_id: CpuId,
        pub hw_id: u32,
        pub is_bsp: bool,
    }

    /// Logical ID of a CPU core.
    ///
    /// The ids of a system with `n` cores are exactly `0..n`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(transparent)]
    pub struct CpuId(usize);

    impl CpuId {
        /// Returns the raw logical id.
        pub const fn value(self) -> usize {
            self.0
        }

        /// Constructs a [`CpuId`] from a raw logical id.
        pub const fn new(raw: usize) -> Self {
            Self(raw)
        }
    }

    pub type CpuEntry = fn(usize) -> !;

    // TODO: we may want to put the bsp at a known logical_id (like 0) for init purposes, ...
    /// Builds the cpu list from raw bootloader data, assigning dense [`CpuId`]s.
    pub fn assign_logical_ids(
        raw: impl IntoIterator<Item = (u64, u32)>,
        bsp_hw_id: u32,
    ) -> Box<[BootCpu]> {
        let mut raw: Vec<_> = raw.into_iter().collect();
        raw.sort_unstable();
        raw.into_iter()
            .enumerate()
            .map(|(index, (_, hw_id))| BootCpu {
                logical_id: CpuId::new(index),
                hw_id,
                is_bsp: hw_id == bsp_hw_id,
            })
            .collect()
    }

    pub struct MpBootInfo {
        pub flags: u32,
        /// Cores, indexed by logical id.
        pub cpus: Box<[BootCpu]>,
        pub start_cpu: fn(&BootCpu, CpuEntry, usize),
    }
}

pub fn build_bootinfo<'a>() -> EarlyBootInfo<'a> {
    #[cfg(feature = "limine")]
    return limine::build_early_bootinfo();
    #[cfg(not(feature = "limine"))]
    todo!()
}

fn build_late_bootinfo(early_boot: &EarlyBootInfo<'_>) -> LateBootInfo {
    #[cfg(feature = "limine")]
    return limine::build_late_bootinfo(early_boot);
    #[cfg(not(feature = "limine"))]
    todo!()
}
