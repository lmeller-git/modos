#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("kernel_acpi currently only supports x86_64");

use alloc::vec::Vec;

use acpi::{
    AcpiError,
    AcpiTables,
    platform::{AcpiPlatform, InterruptModel, interrupt::Apic},
    sdt::madt::{
        Madt,
        MadtEntry,
        Polarity as AcpiPolarity,
        TriggerMode as AcpiTriggerMode,
        parse_mps_inti_flags,
    },
};
use irq_interfaces::{
    IrqSpec,
    Polarity,
    TriggerMode,
    x86::{CpuInfo, IoApicInfo, IsaOverride, LapicNmi, LapicNmiTable, NmiSource, NmiTarget},
};

mod handler;
pub use handler::KernelAcpiHandler;

/// Clock returning nanoseconds since boot.
///
/// Undefined before timer init.
pub type NanosFn = fn() -> u64;

/// Maps `len` bytes of device memory starting at `phys` and returns the virtual address of `phys`.
///
/// # Safety
///
/// `phys` must reference device memory owned by the caller, and the mapping must stay valid for the lifetime of the device.
pub type MapMmioFn = unsafe fn(phys: u64, len: usize) -> u64;

/// Firmware-described interrupt topology of a PC.
///
/// Currently targetted at x86_64 apic.
pub struct InterruptConfig {
    pub lapic_phys: u64,
    pub lapic_nmi: LapicNmiTable,
    pub nmi_sources: Vec<NmiSource>,
    pub ioapics: Vec<IoApicInfo>,
    pub overrides: Vec<IsaOverride>,
    pub has_legacy_pic: bool,
}

/// Firmware discovery failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptConfigError {
    /// The firmware tables are missing or malformed.
    Firmware,
    /// Firmware describes no APIC interrupt model.
    NoApic,
}

#[derive(Debug)]
pub enum ParseError {
    Tables(AcpiError),
    Platform(AcpiError),
    /// The firmware describes no APIC interrupt model (PIC-only machine).
    NoApic,
}

impl From<ParseError> for InterruptConfigError {
    fn from(error: ParseError) -> Self {
        match error {
            ParseError::Tables(_) | ParseError::Platform(_) => Self::Firmware,
            ParseError::NoApic => Self::NoApic,
        }
    }
}

/// # Safety
///
/// `rsdp_phys` must be the physical address of a valid RSDP and all tables it references must be
/// readable through the HHDM at `hhdm`.
pub unsafe fn parse(
    hhdm: usize,
    rsdp_phys: usize,
    nanos_since_boot: NanosFn,
) -> Result<InterruptConfig, ParseError> {
    let handler = KernelAcpiHandler::new(hhdm, nanos_since_boot);
    let tables =
        unsafe { AcpiTables::from_rsdp(handler, rsdp_phys) }.map_err(ParseError::Tables)?;
    let AcpiPlatform {
        interrupt_model,
        processor_info,
        tables,
        ..
    } = AcpiPlatform::new(tables, handler).map_err(ParseError::Platform)?;

    let InterruptModel::Apic(apic) = interrupt_model else {
        return Err(ParseError::NoApic);
    };
    // The APIC model was read from the MADT, so it exists.
    let madt = tables.find_table::<Madt>().ok_or(ParseError::NoApic)?;
    let lapic_nmis = local_nmis(madt.get().entries()).map_err(ParseError::Platform)?;

    let cpus = processor_info
        .map(|info| {
            core::iter::once(info.boot_processor)
                .chain(info.application_processors.iter().copied())
                .map(|p| CpuInfo {
                    uid: p.processor_uid,
                    apic_id: p.local_apic_id,
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(apic_parse(apic, cpus, lapic_nmis))
}

fn polarity(p: AcpiPolarity) -> Polarity {
    match p {
        AcpiPolarity::ActiveHigh | AcpiPolarity::SameAsBus => Polarity::ActiveHigh,
        AcpiPolarity::ActiveLow => Polarity::ActiveLow,
    }
}

fn trigger_mode(t: AcpiTriggerMode) -> TriggerMode {
    match t {
        AcpiTriggerMode::Edge | AcpiTriggerMode::SameAsBus => TriggerMode::Edge,
        AcpiTriggerMode::Level => TriggerMode::Level,
    }
}

fn line(gsi: u32, p: AcpiPolarity, t: AcpiTriggerMode) -> IrqSpec {
    IrqSpec {
        hwirq: gsi,
        trigger: trigger_mode(t),
        polarity: polarity(p),
    }
}

/// LINT `lint` of `processor` (`None`: every processor) wired to NMI with MPS INTI `flags`.
fn local_nmi(processor: Option<u32>, lint: u8, flags: u16) -> Result<LapicNmi, AcpiError> {
    let (p, t) = parse_mps_inti_flags(flags)?;
    Ok(LapicNmi {
        target: processor.map_or(NmiTarget::All, NmiTarget::Uid),
        lint,
        trigger: trigger_mode(t),
        polarity: polarity(p),
    })
}

/// The (x2)APIC NMI entries of the MADT. Read raw, since the `acpi` crate drops their flags.
fn local_nmis<'a>(
    entries: impl IntoIterator<Item = MadtEntry<'a>>,
) -> Result<Vec<LapicNmi>, AcpiError> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            MadtEntry::LocalApicNmi(e) => {
                let processor = (e.processor_id != 0xFF).then_some(u32::from(e.processor_id));
                Some(local_nmi(processor, e.nmi_line, e.flags))
            }
            MadtEntry::X2ApicNmi(e) => {
                let processor = (e.processor_uid != u32::MAX).then_some(e.processor_uid);
                Some(local_nmi(processor, e.nmi_line, e.flags))
            }
            _ => None,
        })
        .collect()
}

fn apic_parse(apic: Apic, cpus: Vec<CpuInfo>, lapic_nmis: Vec<LapicNmi>) -> InterruptConfig {
    InterruptConfig {
        lapic_phys: apic.local_apic_address,
        lapic_nmi: LapicNmiTable {
            cpus,
            lines: lapic_nmis,
        },
        nmi_sources: apic
            .nmi_sources
            .iter()
            .map(|n| NmiSource {
                line: line(n.global_system_interrupt, n.polarity, n.trigger_mode),
            })
            .collect(),
        ioapics: apic
            .io_apics
            .iter()
            .map(|io| IoApicInfo {
                id: io.id,
                phys: u64::from(io.address),
                gsi_base: io.global_system_interrupt_base,
            })
            .collect(),
        overrides: apic
            .interrupt_source_overrides
            .iter()
            .map(|o| IsaOverride {
                isa_irq: o.isa_source,
                line: line(o.global_system_interrupt, o.polarity, o.trigger_mode),
            })
            .collect(),
        has_legacy_pic: apic.also_has_legacy_pics,
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use acpi::{
        platform::interrupt::{InterruptSourceOverride, NmiSource as AcpiNmiSource},
        sdt::madt::{EntryHeader, LocalApicNmiEntry, NmiSourceEntry},
    };
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    fn acpi_polarity() -> impl Strategy<Value = (AcpiPolarity, Polarity)> {
        prop_oneof![
            Just((AcpiPolarity::ActiveHigh, Polarity::ActiveHigh)),
            Just((AcpiPolarity::ActiveLow, Polarity::ActiveLow)),
            Just((AcpiPolarity::SameAsBus, Polarity::ActiveHigh)),
        ]
    }

    fn acpi_trigger() -> impl Strategy<Value = (AcpiTriggerMode, TriggerMode)> {
        prop_oneof![
            Just((AcpiTriggerMode::Edge, TriggerMode::Edge)),
            Just((AcpiTriggerMode::Level, TriggerMode::Level)),
            Just((AcpiTriggerMode::SameAsBus, TriggerMode::Edge)),
        ]
    }

    proptest! {
        #![proptest_config(config())]

        /// Overrides and NMI sources keep their line and map the polarity and trigger of the
        /// firmware entry onto ours ("same as bus" being ISA).
        #[test]
        fn lines(
            overrides in prop::collection::vec(
                (0..16u8, any::<u32>(), acpi_polarity(), acpi_trigger()), 0..5),
            nmis in prop::collection::vec(
                (any::<u32>(), acpi_polarity(), acpi_trigger()), 0..5),
        ) {
            let apic = Apic {
                local_apic_address: 0xFEE0_0000,
                io_apics: Vec::new(),
                local_apic_nmi_lines: Vec::new(),
                interrupt_source_overrides: overrides.iter().map(|&(isa, gsi, (p, _), (t, _))| InterruptSourceOverride {
                    isa_source: isa,
                    global_system_interrupt: gsi,
                    polarity: p,
                    trigger_mode: t,
                }).collect(),
                nmi_sources: nmis.iter().map(|&(gsi, (p, _), (t, _))| AcpiNmiSource {
                    global_system_interrupt: gsi,
                    polarity: p,
                    trigger_mode: t,
                }).collect(),
                also_has_legacy_pics: false,
            };
            let config = apic_parse(apic, Vec::new(), Vec::new());

            let spec = |hwirq, polarity, trigger| IrqSpec { hwirq, trigger, polarity };
            let expected_overrides: Vec<_> = overrides.iter().map(|&(isa, gsi, (_, p), (_, t))| {
                IsaOverride { isa_irq: isa, line: spec(gsi, p, t) }
            }).collect();
            let expected_nmis: Vec<_> = nmis.iter().map(|&(gsi, (_, p), (_, t))| {
                NmiSource { line: spec(gsi, p, t) }
            }).collect();
            prop_assert_eq!(config.overrides, expected_overrides);
            prop_assert_eq!(config.nmi_sources, expected_nmis);
        }
    }

    mod local_nmi {
        use super::*;

        /// MPS INTI flags: polarity in bits 0-1, trigger in bits 2-3 (`01` high / edge, `11` low /
        /// level). "Same as bus" is edge / active high; the reserved `10` is an error.
        #[test]
        fn flags() {
            use Polarity::*;
            use TriggerMode::*;

            let cases = [
                (0b00_00, Some((Edge, ActiveHigh))),
                (0b01_01, Some((Edge, ActiveHigh))),
                (0b11_11, Some((Level, ActiveLow))),
                (0b01_11, Some((Edge, ActiveLow))),
                (0b11_00, Some((Level, ActiveHigh))),
                (0b00_10, None),
                (0b10_00, None),
            ];
            for (flags, expected) in cases {
                let got = local_nmi(None, 1, flags)
                    .ok()
                    .map(|n| (n.trigger, n.polarity));
                assert_eq!(got, expected, "{flags:#06b}");
            }
        }

        /// Each local APIC NMI entry keeps its own target, LINT and flags; processor `0xFF` is
        /// every processor, and NMI source entries are not local NMIs.
        #[test]
        fn entries() {
            let lapic = |processor_id, nmi_line, flags| LocalApicNmiEntry {
                header: EntryHeader {
                    entry_type: 4,
                    length: 6,
                },
                processor_id,
                flags,
                nmi_line,
            };
            let all_lint1 = lapic(0xFF, 1, 0b01_01);
            let cpu3_lint0 = lapic(3, 0, 0b11_11);
            let source = NmiSourceEntry {
                header: EntryHeader {
                    entry_type: 3,
                    length: 8,
                },
                flags: 0b11_11,
                global_system_interrupt: 2,
            };

            let got = local_nmis([
                MadtEntry::LocalApicNmi(&all_lint1),
                MadtEntry::NmiSource(&source),
                MadtEntry::LocalApicNmi(&cpu3_lint0),
            ])
            .unwrap();
            let expected = [
                LapicNmi {
                    target: NmiTarget::All,
                    lint: 1,
                    trigger: TriggerMode::Edge,
                    polarity: Polarity::ActiveHigh,
                },
                LapicNmi {
                    target: NmiTarget::Uid(3),
                    lint: 0,
                    trigger: TriggerMode::Level,
                    polarity: Polarity::ActiveLow,
                },
            ];
            assert_eq!(got, expected);
        }
    }
}
