//! I/O APIC management.

use alloc::vec::Vec;

use platform_interfaces::interrupt::{InterruptConfig, IoApicInfo, IsaOverride, MapMmioFn};
use spin::{Mutex, Once};
use x2apic::ioapic::{IoApic, IrqFlags, IrqMode, RedirectionTableEntry};
use x86_64::instructions::interrupts;

const IOAPIC_MMIO_LEN: usize = 0x1000;
/// The architectural spurious vector. Firmware may leave pins unmasked, so park them on a
/// harmless vector until a driver claims the GSI.
const PARKING_VECTOR: u8 = 0xFF;

static IOAPIC: Once<IoApicManager> = Once::new();

/// Discover and park every I/O APIC, then publish the manager.
pub(crate) fn init(config: &InterruptConfig, bsp_id: u32, map_mmio: MapMmioFn) {
    interrupts::without_interrupts(|| {
        let mgr = unsafe { IoApicManager::new(config, map_mmio) };
        for nmi in &config.nmi_sources {
            mgr.route_nmi(nmi.gsi, bsp_id, nmi.active_low, nmi.level);
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
}

impl IoApicManager {
    /// Panics if the platform interrupt controllers have not been initialised yet.
    pub(crate) fn global() -> &'static Self {
        IOAPIC.get().expect("I/O APIC not initialised")
    }

    unsafe fn new(config: &InterruptConfig, map_mmio: MapMmioFn) -> Self {
        let ioapics = config
            .ioapics
            .iter()
            .map(|info| unsafe {
                let virt = map_mmio(info.phys, IOAPIC_MMIO_LEN);
                let mut ioapic = IoApic::new(virt);
                let max_entry = ioapic.max_table_entry();

                // Firmware may have left pins unmasked: park every entry masked on a harmless vector.
                for pin in 0..=max_entry {
                    let mut parked = RedirectionTableEntry::default();
                    parked.set_mode(IrqMode::Fixed);
                    parked.set_flags(IrqFlags::MASKED);
                    parked.set_vector(PARKING_VECTOR);
                    ioapic.set_table_entry(pin, parked);
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
        }
    }

    /// Route legacy ISA IRQ `isa_irq`, honouring MADT interrupt source overrides
    /// (default: identity-mapped GSI, edge triggered, active high).
    pub(crate) fn route_isa_irq(&self, isa_irq: u8, vector: u8, dest_apic_id: u32, masked: bool) {
        let (gsi, active_low, level) = self
            .overrides
            .iter()
            .find(|o| o.isa_irq == isa_irq)
            .map(|o| (o.gsi, o.active_low, o.level))
            .unwrap_or((u32::from(isa_irq), false, false));
        self.route_gsi(gsi, vector, dest_apic_id, active_low, level, masked);
    }

    pub(crate) fn route_gsi(
        &self,
        gsi: u32,
        vector: u8,
        dest_apic_id: u32,
        active_low: bool,
        level: bool,
        masked: bool,
    ) {
        self.program_gsi(
            gsi,
            IrqMode::Fixed,
            vector,
            dest_apic_id,
            active_low,
            level,
            masked,
        );
    }

    pub(crate) fn route_nmi(&self, gsi: u32, dest_apic_id: u32, active_low: bool, level: bool) {
        self.program_gsi(
            gsi,
            IrqMode::NonMaskable,
            0,
            dest_apic_id,
            active_low,
            level,
            false,
        );
    }

    fn with_gsi<R>(&self, gsi: u32, f: impl FnOnce(&mut IoApic, u8) -> R) -> R {
        interrupts::without_interrupts(|| {
            let mut list = self.ioapics.lock();
            let target = list
                .iter_mut()
                .find(|io| {
                    gsi.checked_sub(io.info.gsi_base)
                        .is_some_and(|pin| pin <= u32::from(io.max_entry))
                })
                .expect("no I/O APIC covers this GSI");
            let pin = (gsi - target.info.gsi_base) as u8;
            f(&mut target.ioapic, pin)
        })
    }

    #[expect(clippy::too_many_arguments)]
    fn program_gsi(
        &self,
        gsi: u32,
        mode: IrqMode,
        vector: u8,
        dest_apic_id: u32,
        active_low: bool,
        level: bool,
        masked: bool,
    ) {
        let dest = u8::try_from(dest_apic_id)
            .ok()
            .filter(|&d| d != 0xFF)
            .expect("APIC ID > 254 requires VT-d interrupt remapping");

        let mut flags = IrqFlags::MASKED;
        if active_low {
            flags |= IrqFlags::LOW_ACTIVE;
        }
        if level {
            flags |= IrqFlags::LEVEL_TRIGGERED;
        }

        let mut entry = RedirectionTableEntry::default();
        entry.set_mode(mode);
        entry.set_flags(flags);
        entry.set_vector(vector);
        entry.set_dest(dest);

        self.with_gsi(gsi, |ioapic, pin| unsafe {
            ioapic.set_table_entry(pin, entry);
            if !masked {
                ioapic.enable_irq(pin);
            }
        });
    }

    pub(crate) fn set_gsi_masked(&self, gsi: u32, masked: bool) {
        self.with_gsi(gsi, |ioapic, pin| unsafe {
            if masked {
                ioapic.disable_irq(pin);
            } else {
                ioapic.enable_irq(pin);
            }
        });
    }
}
