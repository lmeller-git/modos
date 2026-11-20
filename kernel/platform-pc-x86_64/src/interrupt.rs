//! Platform interrupt-controller capability for the PC.

use platform_interfaces::interrupt::{
    InterruptConfig,
    InterruptConfigError,
    InterruptControllerCapability,
    MapMmioFn,
    NanosFn,
};

use crate::{PcX86_64, ioapic, pic};

impl InterruptControllerCapability for PcX86_64 {
    unsafe fn parse_config(
        hhdm: usize,
        rsdp_phys: usize,
        nanos_since_boot: NanosFn,
    ) -> Result<InterruptConfig, InterruptConfigError> {
        unsafe { kernel_acpi::parse(hhdm, rsdp_phys, nanos_since_boot) }.map_err(Into::into)
    }

    unsafe fn init_interrupt_controllers(
        config: &InterruptConfig,
        bsp_id: u32,
        map_mmio: MapMmioFn,
    ) {
        pic::disable(config.has_legacy_pic);
        ioapic::init(config, bsp_id, map_mmio);
    }

    fn route_isa_irq(isa_irq: u8, vector: u8, dest_apic_id: u32, masked: bool) {
        ioapic::IoApicManager::global().route_isa_irq(isa_irq, vector, dest_apic_id, masked);
    }

    fn route_gsi(
        gsi: u32,
        vector: u8,
        dest_apic_id: u32,
        active_low: bool,
        level: bool,
        masked: bool,
    ) {
        ioapic::IoApicManager::global().route_gsi(
            gsi,
            vector,
            dest_apic_id,
            active_low,
            level,
            masked,
        );
    }

    fn set_gsi_masked(gsi: u32, masked: bool) {
        ioapic::IoApicManager::global().set_gsi_masked(gsi, masked);
    }
}
