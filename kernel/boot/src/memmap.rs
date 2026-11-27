//! Physical memory map handed over by the bootloader.

use derive_more::Display;
use kernel_types::{addr::PhysAddr, aligned::AlignedRange, frame::PhysMemoryFrameRange};

// These consts are limine constants.
// TODO generalize if necessary.

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

/// A raw bootloader entry. Read it through [`MemoryMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct Entry {
    pub base: u64,
    pub length: u64,
    pub type_: u64,
}

#[derive(Clone, Copy, Debug, Display, PartialEq, Eq, Hash)]
pub enum RegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    ExecutableAndModules,
    Framebuffer,
    MappedReserved,
    #[display("Unknown({_0})")]
    Unknown(u64),
}

impl RegionKind {
    pub fn from_raw(raw: u64) -> Self {
        match raw {
            MEMMAP_USABLE => Self::Usable,
            MEMMAP_RESERVED => Self::Reserved,
            MEMMAP_ACPI_RECLAIMABLE => Self::AcpiReclaimable,
            MEMMAP_ACPI_NVS => Self::AcpiNvs,
            MEMMAP_BAD_MEMORY => Self::BadMemory,
            MEMMAP_BOOTLOADER_RECLAIMABLE => Self::BootloaderReclaimable,
            MEMMAP_EXECUTABLE_AND_MODULES => Self::ExecutableAndModules,
            MEMMAP_FRAMEBUFFER => Self::Framebuffer,
            MEMMAP_MAPPED_RESERVED => Self::MappedReserved,
            other => Self::Unknown(other),
        }
    }
}

/// Physical memory `[start, end)` of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    start: PhysAddr,
    end: PhysAddr,
    kind: RegionKind,
}

impl Region {
    /// `None` if `end < start`.
    pub fn new(start: PhysAddr, end: PhysAddr, kind: RegionKind) -> Option<Self> {
        (start <= end).then_some(Self { start, end, kind })
    }

    /// `None` if the entry starts beyond the address space; an end beyond it is clipped.
    pub fn parse(entry: &Entry) -> Option<Self> {
        let start = usize::try_from(entry.base).ok()?;
        let len = usize::try_from(entry.length).unwrap_or(usize::MAX);
        let end = start.saturating_add(len);
        Self::new(
            PhysAddr::new(start),
            PhysAddr::new(end),
            RegionKind::from_raw(entry.type_),
        )
    }

    pub fn start(&self) -> PhysAddr {
        self.start
    }

    /// Exclusive.
    pub fn end(&self) -> PhysAddr {
        self.end
    }

    pub fn kind(&self) -> RegionKind {
        self.kind
    }

    /// The whole frames inside the region.
    pub fn frames(&self) -> PhysMemoryFrameRange {
        AlignedRange::within(self.start, self.end)
    }
}

/// The bootloader's memory map. Borrowed and parsed on iteration, so it needs no heap.
#[derive(Clone, Copy, Debug)]
pub struct MemoryMap<'a>(&'a [&'a Entry]);

impl<'a> MemoryMap<'a> {
    pub fn new(entries: &'a [&'a Entry]) -> Self {
        Self(entries)
    }

    /// Entries in bootloader order, skipping those outside the address space.
    pub fn regions(&self) -> impl DoubleEndedIterator<Item = Region> + Clone + 'a {
        self.0.iter().filter_map(|entry| Region::parse(entry))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    /// Limine's memory map types, in order.
    #[test]
    fn kinds() {
        let known = [
            RegionKind::Usable,
            RegionKind::Reserved,
            RegionKind::AcpiReclaimable,
            RegionKind::AcpiNvs,
            RegionKind::BadMemory,
            RegionKind::BootloaderReclaimable,
            RegionKind::ExecutableAndModules,
            RegionKind::Framebuffer,
            RegionKind::MappedReserved,
        ];
        for (raw, kind) in (0..).zip(known) {
            assert_eq!(RegionKind::from_raw(raw), kind);
        }
        assert_eq!(RegionKind::from_raw(9), RegionKind::Unknown(9));
        assert_eq!(
            RegionKind::from_raw(u64::MAX),
            RegionKind::Unknown(u64::MAX)
        );
    }

    proptest! {
        #![proptest_config(config())]

        /// An entry keeps its start and covers `length` bytes, clipped at the top of the address
        /// space.
        #[test]
        fn parse(
            base in prop_oneof![any::<u64>(), u64::MAX - 0x2000..=u64::MAX],
            length in prop_oneof![0..0x10_000u64, any::<u64>()],
            type_ in 0..12u64,
        ) {
            let entry = Entry { base, length, type_ };
            let region = Region::parse(&entry).unwrap();
            let end = u128::from(base) + u128::from(length);

            prop_assert_eq!(region.start().as_usize() as u64, base);
            prop_assert_eq!(region.end().as_usize() as u128, end.min(usize::MAX as u128));
            prop_assert_eq!(region.kind(), RegionKind::from_raw(type_));
        }
    }
}
