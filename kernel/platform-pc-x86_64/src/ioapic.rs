//! I/O APIC management.

use alloc::vec::Vec;

use irq_interfaces::{
    IrqError,
    IrqSpec,
    IrqTarget,
    Polarity,
    TriggerMode,
    x86::{IoApicInfo, IsaOverride},
};
use kernel_acpi::{InterruptConfig, MapMmioFn};
use kernel_types::cpu::CpuId;
use spin::{Mutex, Once};
use x2apic::ioapic::{IoApic, IrqFlags, IrqMode, RedirectionTableEntry};
use x86_64::instructions::interrupts;

const IOAPIC_MMIO_LEN: usize = 0x1000;
/// The architectural spurious vector. Firmware may leave pins unmasked, so park them on a
/// harmless vector until a driver claims the GSI.
const PARKING_VECTOR: u8 = 0xFF;

static IOAPIC: Once<IoApicManager> = Once::new();

/// Discover and park every I/O APIC, then publish the manager.
pub(crate) fn init(
    config: &InterruptConfig,
    bsp_id: u32,
    cpus: &[(CpuId, u32)],
    map_mmio: MapMmioFn,
) {
    interrupts::without_interrupts(|| {
        let mgr = unsafe { IoApicManager::new(config, cpus, map_mmio) };
        for nmi in &config.nmi_sources {
            if let Err(err) = mgr.route_nmi(nmi.line, bsp_id) {
                panic!("failed to route NMI source GSI {}: {err:?}", nmi.line.hwirq);
            }
        }
        IOAPIC.call_once(|| mgr);
    });
}

struct ManagedIoApic {
    ioapic: IoApic,
    info: IoApicInfo,
    max_entry: u8,
}
// SAFETY: IoApic wraps an MMIO pointer; every access goes through the Mutex below.
unsafe impl Send for ManagedIoApic {}

pub(crate) struct IoApicManager {
    ioapics: Mutex<Vec<ManagedIoApic>>,
    overrides: Vec<IsaOverride>,
    /// GSIs firmware reserved for NMI delivery. Line uniqueness among drivers is the `irq` core's
    /// job, but only the platform knows these.
    reserved: Vec<u32>,
    cpu_apic_ids: Vec<(CpuId, u32)>,
}

/// A masked entry on the harmless parking vector.
fn parked() -> RedirectionTableEntry {
    let mut entry = RedirectionTableEntry::default();
    entry.set_mode(IrqMode::Fixed);
    entry.set_flags(IrqFlags::MASKED);
    entry.set_vector(PARKING_VECTOR);
    entry
}

/// Finds the I/O APIC covering `gsi` among `(gsi_base, max_entry)` ranges: its index and the pin.
fn resolve_gsi(ranges: impl Iterator<Item = (u32, u8)>, gsi: u32) -> Result<(usize, u8), IrqError> {
    ranges
        .enumerate()
        .find_map(|(index, (base, max_entry))| {
            let pin = gsi.checked_sub(base)?;
            (pin <= u32::from(max_entry)).then_some((index, pin as u8))
        })
        .ok_or(IrqError::NoController)
}

/// The masked redirection entry delivering `line` to `dest`; NMI deliveries have no `vector`.
fn build_entry(
    line: IrqSpec,
    mode: IrqMode,
    vector: Option<u8>,
    dest: u8,
) -> RedirectionTableEntry {
    let mut flags = IrqFlags::MASKED;
    if line.polarity == Polarity::ActiveLow {
        flags |= IrqFlags::LOW_ACTIVE;
    }
    if line.trigger == TriggerMode::Level {
        flags |= IrqFlags::LEVEL_TRIGGERED;
    }

    let mut entry = RedirectionTableEntry::default();
    entry.set_mode(mode);
    entry.set_flags(flags);
    if let Some(vector) = vector {
        entry.set_vector(vector);
    }
    entry.set_dest(dest);
    entry
}

impl IoApicManager {
    pub(crate) fn global() -> Result<&'static Self, IrqError> {
        IOAPIC.get().ok_or(IrqError::NotInitialized)
    }

    unsafe fn new(config: &InterruptConfig, cpus: &[(CpuId, u32)], map_mmio: MapMmioFn) -> Self {
        let ioapics = config
            .ioapics
            .iter()
            .map(|info| unsafe {
                let virt = map_mmio(info.phys, IOAPIC_MMIO_LEN);
                let mut ioapic = IoApic::new(virt);
                let max_entry = ioapic.max_table_entry();

                // Firmware may have left pins unmasked: park every entry masked on a harmless vector.
                for pin in 0..=max_entry {
                    ioapic.set_table_entry(pin, parked());
                }

                ManagedIoApic {
                    ioapic,
                    info: *info,
                    max_entry,
                }
            })
            .collect();

        Self {
            ioapics: Mutex::new(ioapics),
            overrides: config.overrides.clone(),
            reserved: config.nmi_sources.iter().map(|n| n.line.hwirq).collect(),
            cpu_apic_ids: cpus.to_vec(),
        }
    }

    /// Resolve a legacy ISA IRQ, honouring MADT interrupt source overrides
    /// (default: identity-mapped GSI, edge triggered, active high).
    pub(crate) fn isa_irq_line(&self, isa_irq: u8) -> IrqSpec {
        self.overrides
            .iter()
            .find(|o| o.isa_irq == isa_irq)
            .map_or_else(
                || IrqSpec {
                    hwirq: u32::from(isa_irq),
                    trigger: TriggerMode::Edge,
                    polarity: Polarity::ActiveHigh,
                },
                |o| o.line,
            )
    }

    /// Program `spec`'s redirection entry for `target`, leaving it masked.
    pub(crate) fn route(&self, spec: IrqSpec, target: IrqTarget) -> Result<(), IrqError> {
        if self.reserved.contains(&spec.hwirq) {
            return Err(IrqError::AlreadyClaimed);
        }
        let vector = u8::try_from(target.vector).map_err(|_| IrqError::InvalidTarget)?;
        let dest_apic_id = self.apic_id_for(target.cpu)?;
        self.program_gsi(spec, IrqMode::Fixed, Some(vector), dest_apic_id, true)
    }

    fn route_nmi(&self, line: IrqSpec, dest_apic_id: u32) -> Result<(), IrqError> {
        self.program_gsi(line, IrqMode::NonMaskable, None, dest_apic_id, false)
    }

    pub(crate) fn set_masked(&self, spec: IrqSpec, masked: bool) {
        let res = self.with_gsi(spec.hwirq, |ioapic, pin| unsafe {
            if masked {
                ioapic.disable_irq(pin);
            } else {
                ioapic.enable_irq(pin);
            }
        });
        debug_assert!(res.is_ok(), "masking a line no I/O APIC covers");
    }

    /// Masks the line and points it back at the parking vector.
    pub(crate) fn park(&self, spec: IrqSpec) {
        let res = self.with_gsi(spec.hwirq, |ioapic, pin| unsafe {
            ioapic.set_table_entry(pin, parked());
        });
        debug_assert!(res.is_ok(), "parking a line no I/O APIC covers");
    }

    fn apic_id_for(&self, cpu: CpuId) -> Result<u32, IrqError> {
        self.cpu_apic_ids
            .iter()
            .find(|(id, _)| *id == cpu)
            .map(|(_, apic_id)| *apic_id)
            .ok_or(IrqError::InvalidTarget)
    }

    /// Runs `f` on the I/O APIC covering `gsi`, with interrupts disabled.
    ///
    /// The mask path runs from interrupt context, so the list lock must never be held while a
    /// core can take an interrupt.
    fn with_gsi<R>(&self, gsi: u32, f: impl FnOnce(&mut IoApic, u8) -> R) -> Result<R, IrqError> {
        interrupts::without_interrupts(|| {
            let mut list = self.ioapics.lock();
            let (index, pin) =
                resolve_gsi(list.iter().map(|io| (io.info.gsi_base, io.max_entry)), gsi)?;
            Ok(f(&mut list[index].ioapic, pin))
        })
    }

    fn program_gsi(
        &self,
        line: IrqSpec,
        mode: IrqMode,
        vector: Option<u8>,
        dest_apic_id: u32,
        masked: bool,
    ) -> Result<(), IrqError> {
        let dest = u8::try_from(dest_apic_id)
            .ok()
            .filter(|&d| d != 0xFF)
            .ok_or(IrqError::InvalidTarget)?;

        let entry = build_entry(line, mode, vector, dest);

        self.with_gsi(line.hwirq, |ioapic, pin| unsafe {
            ioapic.set_table_entry(pin, entry);
            if !masked {
                ioapic.enable_irq(pin);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};

    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    fn trigger() -> impl Strategy<Value = TriggerMode> {
        prop_oneof![Just(TriggerMode::Edge), Just(TriggerMode::Level)]
    }

    fn polarity() -> impl Strategy<Value = Polarity> {
        prop_oneof![Just(Polarity::ActiveHigh), Just(Polarity::ActiveLow)]
    }

    fn spec() -> impl Strategy<Value = IrqSpec> {
        (0..64u32, trigger(), polarity()).prop_map(|(hwirq, trigger, polarity)| IrqSpec {
            hwirq,
            trigger,
            polarity,
        })
    }

    fn manager(
        overrides: Vec<IsaOverride>,
        reserved: Vec<u32>,
        cpus: &[(usize, u32)],
    ) -> IoApicManager {
        IoApicManager {
            ioapics: Mutex::new(Vec::new()),
            overrides,
            reserved,
            cpu_apic_ids: cpus
                .iter()
                .map(|&(id, apic)| (CpuId::new(id), apic))
                .collect(),
        }
    }

    mod resolve_gsi {
        use super::*;

        proptest! {
            #![proptest_config(config())]

            /// A GSI resolves to an I/O APIC whose `[base, base + max_entry]` contains it, at the
            /// offset into it; otherwise there is no controller. Which of several overlapping
            /// controllers wins (a firmware bug) is not pinned.
            #[test]
            fn covering(
                ranges in prop::collection::vec(
                    (prop_oneof![0..40u32, u32::MAX - 30..=u32::MAX], 0..24u8),
                    0..4,
                ),
                gsi in prop_oneof![0..70u32, u32::MAX - 40..=u32::MAX],
            ) {
                let covers = |&(base, max): &(u32, u8)| {
                    gsi >= base && u64::from(gsi) <= u64::from(base) + u64::from(max)
                };
                match resolve_gsi(ranges.iter().copied(), gsi) {
                    Ok((index, pin)) => {
                        prop_assert!(covers(&ranges[index]));
                        prop_assert_eq!(u32::from(pin), gsi - ranges[index].0);
                    }
                    Err(error) => {
                        prop_assert_eq!(error, IrqError::NoController);
                        prop_assert!(!ranges.iter().any(covers));
                    }
                }
            }
        }
    }

    mod build_entry {
        use super::*;

        /// Trigger and polarity map onto the redirection flags and the entry starts masked; only a
        /// fixed delivery carries a vector.
        #[test]
        fn flags() {
            for trigger in [TriggerMode::Edge, TriggerMode::Level] {
                for polarity in [Polarity::ActiveHigh, Polarity::ActiveLow] {
                    let line = IrqSpec {
                        hwirq: 4,
                        trigger,
                        polarity,
                    };
                    let mut flags = IrqFlags::MASKED;
                    flags.set(IrqFlags::LEVEL_TRIGGERED, trigger == TriggerMode::Level);
                    flags.set(IrqFlags::LOW_ACTIVE, polarity == Polarity::ActiveLow);

                    let fixed = build_entry(line, IrqMode::Fixed, Some(0x41), 7);
                    let nmi = build_entry(line, IrqMode::NonMaskable, None, 3);
                    assert_eq!((fixed.flags(), fixed.vector()), (flags, 0x41));
                    assert_eq!((nmi.flags(), nmi.vector()), (flags, 0));
                }
            }
        }
    }

    mod manager {
        use super::*;

        /// Each way a request can be invalid is rejected before any I/O APIC is touched.
        #[test]
        fn route_rejects() {
            let m = manager(Vec::new(), vec![9], &[(0, 0), (1, 1), (2, 0xFF), (3, 256)]);
            let line = |hwirq| IrqSpec {
                hwirq,
                trigger: TriggerMode::Edge,
                polarity: Polarity::ActiveHigh,
            };
            let target = |cpu, vector| IrqTarget {
                cpu: CpuId::new(cpu),
                vector,
            };

            let cases = [
                // Reserved by firmware for NMI delivery.
                (line(9), target(0, 0x30), IrqError::AlreadyClaimed),
                // Does not fit the 8 bit vector field.
                (line(1), target(0, 0x100), IrqError::InvalidTarget),
                (line(1), target(0, u32::MAX), IrqError::InvalidTarget),
                // Unknown cpu.
                (line(1), target(9, 0x30), IrqError::InvalidTarget),
                // The destination is 8 bits wide and 0xFF is broadcast: large x2APIC ids are
                // rejected rather than misrouted.
                (line(1), target(2, 0x30), IrqError::InvalidTarget),
                (line(1), target(3, 0x30), IrqError::InvalidTarget),
            ];
            for (spec, target, error) in cases {
                assert_eq!(m.route(spec, target), Err(error), "{spec:?} {target:?}");
            }
        }

        proptest! {
            #![proptest_config(config())]

            /// A firmware override replaces the identity mapping (edge, active high) of its legacy
            /// IRQ. Duplicate overrides (a firmware bug) are not generated.
            #[test]
            fn isa_irq_line(
                overrides in prop::collection::btree_map(0..16u8, spec(), 0..5),
                irq in 0..20u8,
            ) {
                let expected = overrides.get(&irq).copied().unwrap_or(IrqSpec {
                    hwirq: u32::from(irq),
                    trigger: TriggerMode::Edge,
                    polarity: Polarity::ActiveHigh,
                });
                let overrides = overrides
                    .into_iter()
                    .map(|(isa_irq, line)| IsaOverride { isa_irq, line })
                    .collect();
                prop_assert_eq!(manager(overrides, Vec::new(), &[]).isa_irq_line(irq), expected);
            }
        }
    }
}
