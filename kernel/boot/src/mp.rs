use alloc::{boxed::Box, vec::Vec};

use derive_more::{Deref, Display, Error};
use kernel_types::cpu::CpuId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct BootCpu {
    pub logical_id: CpuId,
    pub hw_id: u32,
    pub is_bsp: bool,
}

pub type CpuEntry = fn(usize) -> !;

/// Why raw bootloader data does not describe a [`CpuTable`].
#[derive(Clone, Copy, Debug, Display, Error, PartialEq, Eq)]
pub enum CpuTableError {
    #[display("two cpus share hardware id {_0}")]
    DuplicateHwId(#[error(not(source))] u32),
    #[display("the BSP (hardware id {_0}) is not in the cpu list")]
    MissingBsp(#[error(not(source))] u32),
}

/// All cpus of the system, indexed by logical id, with unique hardware ids. The BSP is cpu 0.
#[derive(Clone, Debug, PartialEq, Eq, Deref)]
pub struct CpuTable(Box<[BootCpu]>);

impl CpuTable {
    /// Builds the table from `(processor_id, hw_id)` pairs, assigning dense [`CpuId`]s.
    ///
    /// The BSP gets id 0; the other cores follow ordered by `(processor_id, hw_id)`.
    pub fn from_raw(
        raw: impl IntoIterator<Item = (u64, u32)>,
        bsp_hw_id: u32,
    ) -> Result<Self, CpuTableError> {
        let mut raw: Vec<_> = raw.into_iter().collect();
        let mut hw_ids: Vec<u32> = raw.iter().map(|&(_, hw_id)| hw_id).collect();
        hw_ids.sort_unstable();
        if let Some(pair) = hw_ids.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(CpuTableError::DuplicateHwId(pair[0]));
        }
        if hw_ids.binary_search(&bsp_hw_id).is_err() {
            return Err(CpuTableError::MissingBsp(bsp_hw_id));
        }

        raw.sort_unstable();
        raw.sort_by_key(|&(_, hw_id)| hw_id != bsp_hw_id);
        let cpus = raw
            .into_iter()
            .enumerate()
            .map(|(index, (_, hw_id))| BootCpu {
                logical_id: CpuId::new(index),
                hw_id,
                is_bsp: hw_id == bsp_hw_id,
            })
            .collect();
        Ok(Self(cpus))
    }

    pub fn bsp(&self) -> &BootCpu {
        // Non-empty and sorted BSP first: checked in `from_raw`.
        &self.0[0]
    }

    pub fn get(&self, cpu: CpuId) -> Option<&BootCpu> {
        self.0.get(cpu.value())
    }
}

pub struct MpBootInfo {
    pub flags: u32,
    pub cpus: CpuTable,
    pub start_cpu: fn(&BootCpu, CpuEntry, usize),
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    mod from_raw {
        use super::*;

        proptest! {
            #![proptest_config(config())]

            /// Ids are `0..n` and `get` finds each cpu by its id. The BSP is 0 and the only one
            /// marked, the others follow ordered by `(processor_id, hw_id)`, and the result is
            /// independent of the input order. Without the BSP in the list there is no table.
            #[test]
            fn order(
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

                let cpus = match CpuTable::from_raw(raw.iter().copied(), bsp) {
                    Ok(cpus) => cpus,
                    Err(err) => {
                        prop_assert_eq!((err, bsp_present), (CpuTableError::MissingBsp(bsp), false));
                        return Ok(());
                    }
                };
                prop_assert!(bsp_present);

                for (index, cpu) in cpus.iter().enumerate() {
                    prop_assert_eq!(cpu.logical_id.value(), index);
                    prop_assert_eq!(cpus.get(cpu.logical_id), Some(cpu));
                }
                prop_assert_eq!(cpus.get(CpuId::new(cpus.len())), None);

                let mut hw: Vec<_> = cpus.iter().map(|c| c.hw_id).collect();
                let mut expected_hw: Vec<_> = raw.iter().map(|r| r.1).collect();
                hw.sort_unstable();
                expected_hw.sort_unstable();
                prop_assert_eq!(hw, expected_hw);

                let bsps: Vec<_> = cpus.iter().filter(|c| c.is_bsp).collect();
                prop_assert_eq!(bsps, [cpus.bsp()]);
                prop_assert_eq!((cpus.bsp().logical_id.value(), cpus.bsp().hw_id), (0, bsp));
                let processor = |hw: u32| raw.iter().find(|r| r.1 == hw).unwrap().0;
                let others: Vec<_> = cpus.iter().filter(|c| !c.is_bsp).map(|c| (processor(c.hw_id), c.hw_id)).collect();
                prop_assert!(others.is_sorted());

                prop_assert_eq!(CpuTable::from_raw(shuffled, bsp), Ok(cpus));
            }

            /// A hardware id listed twice (under any processor ids) is rejected.
            #[test]
            fn duplicate_hw_id(
                raw in prop::collection::vec((0..4u64, any::<u32>()), 1..12),
                pick in any::<prop::sample::Index>(),
                processor in 0..4u64,
            ) {
                let mut raw = raw;
                let duplicate = raw[pick.index(raw.len())].1;
                raw.push((processor, duplicate));
                prop_assert_eq!(
                    CpuTable::from_raw(raw, duplicate),
                    Err(CpuTableError::DuplicateHwId(duplicate))
                );
            }
        }
    }
}
