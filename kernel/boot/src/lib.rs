#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use core::ptr::NonNull;

use crate::{memmap::Entry, mp::MpBootInfo};

#[cfg(feature = "limine")]
mod limine;

#[cfg(test)]
extern crate std;

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

    /// Builds the cpu list from raw bootloader data, assigning dense [`CpuId`]s.
    ///
    /// The BSP (if present) gets id 0; the other cores follow ordered by `(processor_id, hw_id)`.
    pub fn assign_logical_ids(
        raw: impl IntoIterator<Item = (u64, u32)>,
        bsp_hw_id: u32,
    ) -> Box<[BootCpu]> {
        let mut raw: Vec<_> = raw.into_iter().collect();
        raw.sort_unstable();
        // bsp gets index 0
        raw.sort_by_key(|&(_, hw_id)| hw_id != bsp_hw_id);
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

    #[cfg(test)]
    mod tests {
        use alloc::{vec, vec::Vec};

        use proptest::prelude::*;
        use test_host::config;

        use super::*;

        proptest! {
            #![proptest_config(config())]

            /// Ids are exactly `0..n`: the BSP (if present) is 0, the others follow ordered by
            /// `(processor_id, hw_id)`. The result is independent of the input order.
            #[test]
            fn assign_ids(
                (raw, shuffled) in prop::collection::btree_map(any::<u32>(), 0..4u64, 0..12)
                    .prop_flat_map(|cores| {
                        let raw: Vec<(u64, u32)> = cores.into_iter().map(|(hw, p)| (p, hw)).collect();
                        (Just(raw.clone()), Just(raw).prop_shuffle())
                    }),
                bsp_pick in any::<prop::sample::Index>(),
                bsp_present in any::<bool>(),
            ) {
                let bsp_present = bsp_present && !raw.is_empty();
                let bsp = if bsp_present {
                    raw[bsp_pick.index(raw.len())].1
                } else {
                    // Not part of the system.
                    (0..=u32::MAX).find(|h| raw.iter().all(|r| r.1 != *h)).unwrap()
                };

                let cpus = assign_logical_ids(raw.iter().copied(), bsp);

                let ids: Vec<_> = cpus.iter().map(|c| c.logical_id.value()).collect();
                prop_assert_eq!(ids, (0..raw.len()).collect::<Vec<_>>());
                let mut hw: Vec<_> = cpus.iter().map(|c| c.hw_id).collect();
                let mut expected_hw: Vec<_> = raw.iter().map(|r| r.1).collect();
                hw.sort_unstable();
                expected_hw.sort_unstable();
                prop_assert_eq!(hw, expected_hw);

                let bsps: Vec<_> = cpus.iter().filter(|c| c.is_bsp).map(|c| c.logical_id.value()).collect();
                prop_assert_eq!(bsps, if bsp_present { vec![0] } else { vec![] });
                let processor = |hw: u32| raw.iter().find(|r| r.1 == hw).unwrap().0;
                let others: Vec<_> = cpus.iter().filter(|c| !c.is_bsp).map(|c| (processor(c.hw_id), c.hw_id)).collect();
                prop_assert!(others.is_sorted());

                prop_assert_eq!(assign_logical_ids(shuffled, bsp), cpus);
            }
        }
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
