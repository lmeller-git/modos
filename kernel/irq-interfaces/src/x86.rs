//! Types shared between the x86 backend crates (firmware discovery, local APIC, I/O APIC).

use alloc::vec::Vec;

use crate::{IrqError, IrqSpec, Polarity, TriggerMode};

/// One I/O APIC as described by firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicInfo {
    pub id: u8,
    pub phys: u64,
    pub gsi_base: u32,
}

/// A firmware override for a legacy ISA IRQ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsaOverride {
    pub isa_irq: u8,
    pub line: IrqSpec,
}

/// A firmware-described NMI source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NmiSource {
    pub line: IrqSpec,
}

/// A processor as described by the MADT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuInfo {
    /// ACPI processor UID.
    pub uid: u32,
    /// Local APIC ID.
    pub apic_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NmiTarget {
    All,
    Uid(u32),
}

/// "LINT `lint` of `target` is wired to NMI."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LapicNmi {
    pub target: NmiTarget,
    /// 0 for LINT0, 1 for LINT1.
    pub lint: u8,
    pub trigger: TriggerMode,
    pub polarity: Polarity,
}

#[derive(Clone, Debug, Default)]
pub struct LapicNmiTable {
    pub cpus: Vec<CpuInfo>,
    pub lines: Vec<LapicNmi>,
}

impl LapicNmiTable {
    /// Returns `[LINT0, LINT1]` NMI configurations for the core with this APIC ID.
    pub fn lints_for(&self, apic_id: u32) -> [Option<LapicNmi>; 2] {
        let uid = self
            .cpus
            .iter()
            .find(|c| c.apic_id == apic_id)
            .map(|c| c.uid);

        let mut out = [None; 2];
        for &line in &self.lines {
            let applies = match line.target {
                NmiTarget::All => true,
                NmiTarget::Uid(u) => uid == Some(u),
            };
            if applies && let Some(slot) = out.get_mut(line.lint as usize) {
                *slot = Some(line);
            }
        }
        out
    }
}

/// Platforms that can resolve legacy ISA IRQs (honouring firmware overrides) to a line.
pub trait IsaIrqCapability {
    /// Resolves ISA IRQ `irq`. Defaults to an identity-mapped, edge-triggered, active-high line
    /// when firmware has no override.
    fn isa_irq(irq: u8) -> Result<IrqSpec, IrqError>;
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    mod lapic_nmi_table {
        use super::*;

        fn nmi() -> impl Strategy<Value = LapicNmi> {
            (
                prop_oneof![Just(NmiTarget::All), (0..4u32).prop_map(NmiTarget::Uid)],
                0..4u8,
                prop_oneof![Just(TriggerMode::Edge), Just(TriggerMode::Level)],
                prop_oneof![Just(Polarity::ActiveHigh), Just(Polarity::ActiveLow)],
            )
                .prop_map(|(target, lint, trigger, polarity)| LapicNmi {
                    target,
                    lint,
                    trigger,
                    polarity,
                })
        }

        proptest! {
            #![proptest_config(config())]

            /// Each LINT gets a line that targets this core (all cores, or its UID) and names that
            /// LINT, if there is one. Lines for LINT >= 2 are ignored; an unknown APIC id only
            /// matches `All` lines. Which of several matching lines wins is not pinned.
            #[test]
            fn lints_for(
                cpus in prop::collection::btree_map(0..4u32, 0..4u32, 0..4),
                lines in prop::collection::vec(nmi(), 0..8),
                apic_id in 0..5u32,
            ) {
                let uid = cpus.get(&apic_id).copied();
                let cpus = cpus.into_iter().map(|(apic_id, uid)| CpuInfo { uid, apic_id }).collect();
                let table = LapicNmiTable { cpus, lines };

                let got = table.lints_for(apic_id);
                for lint in 0..2u8 {
                    let matching: Vec<_> = table.lines.iter().copied().filter(|l| {
                        l.lint == lint
                            && match l.target {
                                NmiTarget::All => true,
                                NmiTarget::Uid(u) => uid == Some(u),
                            }
                    }).collect();
                    match got[usize::from(lint)] {
                        Some(line) => prop_assert!(matching.contains(&line)),
                        None => prop_assert!(matching.is_empty()),
                    }
                }
            }
        }
    }
}
