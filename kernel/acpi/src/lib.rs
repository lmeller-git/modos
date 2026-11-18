#![no_std]

extern crate alloc;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("kernel_acpi currently only supports x86_64");

// TODO delegate the actual arch specific parser to arch specific crates via capabilities.

use alloc::vec::Vec;

use acpi::{
    AcpiError,
    AcpiTables,
    platform::{
        AcpiPlatform,
        InterruptModel,
        interrupt::{Apic, LocalInterruptLine, NmiProcessor},
    },
    sdt::madt::{Polarity, TriggerMode},
};

mod handler;
pub use handler::KernelAcpiHandler;

/// Clock that returns nanoseconds since boot.
/// This function may return anything before the arch `late_init` is done, i.e. before the arch has initialized its timers correctly.
pub type NanosFn = fn() -> u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicInfo {
    pub id: u8,
    pub phys: u64,
    pub gsi_base: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsaOverride {
    pub isa_irq: u8,
    pub gsi: u32,
    pub active_low: bool,
    pub level: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NmiSource {
    pub gsi: u32,
    pub active_low: bool,
    pub level: bool,
}

/// A processor as described by the MADT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuInfo {
    /// ACPI processor UID (nmi)
    pub uid: u32,
    /// Local APIC ID
    pub apic_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NmiTarget {
    All,
    Uid(u32),
}

/// "LINT`lint` of `target` is wired to NMI."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LapicNmi {
    pub target: NmiTarget,
    /// 0 for LINT0, 1 for LINT1.
    pub lint: u8,
    pub active_low: bool,
    pub level: bool,
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

pub struct InterruptConfig {
    pub lapic_phys: u64,
    pub lapic_nmi: LapicNmiTable,
    pub nmi_sources: Vec<NmiSource>,
    pub ioapics: Vec<IoApicInfo>,
    pub overrides: Vec<IsaOverride>,
    pub has_legacy_pic: bool,
}

#[derive(Debug)]
pub enum ParseError {
    Tables(AcpiError),
    Platform(AcpiError),
    /// The firmware describes no APIC interrupt model (PIC-only machine).
    NoApic,
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
        ..
    } = AcpiPlatform::new(tables, handler).map_err(ParseError::Platform)?;

    let InterruptModel::Apic(apic) = interrupt_model else {
        return Err(ParseError::NoApic);
    };

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

    Ok(apic_parse(apic, cpus))
}

fn apic_parse(apic: Apic, cpus: Vec<CpuInfo>) -> InterruptConfig {
    InterruptConfig {
        lapic_phys: apic.local_apic_address,
        lapic_nmi: LapicNmiTable {
            cpus,
            lines: apic
                .local_apic_nmi_lines
                .iter()
                .zip(apic.nmi_sources.iter())
                .map(|(n, s)| LapicNmi {
                    target: match n.processor {
                        NmiProcessor::All => NmiTarget::All,
                        NmiProcessor::ProcessorUid(u) => NmiTarget::Uid(u),
                    },
                    lint: match n.line {
                        LocalInterruptLine::Lint0 => 0,
                        LocalInterruptLine::Lint1 => 1,
                    },
                    active_low: matches!(s.polarity, Polarity::ActiveLow),
                    level: matches!(s.trigger_mode, TriggerMode::Level),
                })
                .collect(),
        },
        nmi_sources: apic
            .nmi_sources
            .iter()
            .map(|n| NmiSource {
                gsi: n.global_system_interrupt,
                active_low: matches!(n.polarity, Polarity::ActiveLow),
                level: matches!(n.trigger_mode, TriggerMode::Level),
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
                gsi: o.global_system_interrupt,
                active_low: matches!(o.polarity, Polarity::ActiveLow),
                level: matches!(o.trigger_mode, TriggerMode::Level),
            })
            .collect(),
        has_legacy_pic: apic.also_has_legacy_pics,
    }
}
