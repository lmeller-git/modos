//! I/O APIC management.

use alloc::vec::Vec;

use irq_interfaces::{
    CpuId,
    IrqError,
    IrqSpec,
    IrqTarget,
    Polarity,
    TriggerMode,
    x86::{IoApicInfo, IsaOverride},
};
use kernel_acpi::{InterruptConfig, MapMmioFn};
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
            let target = list
                .iter_mut()
                .find(|io| {
                    gsi.checked_sub(io.info.gsi_base)
                        .is_some_and(|pin| pin <= u32::from(io.max_entry))
                })
                .ok_or(IrqError::NoController)?;
            let pin = (gsi - target.info.gsi_base) as u8;
            Ok(f(&mut target.ioapic, pin))
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

        self.with_gsi(line.hwirq, |ioapic, pin| unsafe {
            ioapic.set_table_entry(pin, entry);
            if !masked {
                ioapic.enable_irq(pin);
            }
        })
    }
}
