#![no_std]

extern crate alloc;

use alloc::boxed::Box;

use kernel_types::{addr::PhysAddr, hhdm::Hhdm};

use crate::{
    memmap::{MemoryMap, Region},
    mp::MpBootInfo,
};

#[cfg(feature = "limine")]
mod limine;
pub mod memmap;
pub mod mp;

#[cfg(test)]
extern crate std;

pub struct BootLoaderInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

pub struct EarlyBootInfo<'a> {
    pub hhdm: Option<Hhdm>,
    pub bootloader_info: Option<BootLoaderInfo<'a>>,
    pub rsdp: Option<PhysAddr>,
    pub memmap: Option<MemoryMap<'a>>,
    pub tsc_hz: Option<u64>,
}

impl<'a> EarlyBootInfo<'a> {
    pub fn into_late(&self) -> LateBootInfo {
        build_late_bootinfo(self)
    }
}

pub struct LateBootInfo {
    pub hhdm: Option<Hhdm>,
    pub bootloader_info: Option<BootLoaderInfo<'static>>,
    pub rsdp: Option<PhysAddr>,
    pub memmap: Option<Box<[Region]>>,
    pub mp_info: Option<MpBootInfo>,
    pub tsc_hz: Option<u64>,
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
