//! Platform interrupt-controller capability for the PC.
//!
//! Firmware discovery and controller bring-up are inherent PC operations, not part of the generic
//! platform interface.

use irq_interfaces::x86::IsaIrqCapability;
use kernel_acpi::{InterruptConfig, InterruptConfigError, MapMmioFn, NanosFn};
use platform_interfaces::interrupt::{
    CpuId,
    InterruptControllerCapability,
    IrqError,
    IrqSpec,
    IrqTarget,
};

use crate::{PcX86_64, ioapic, pic};

impl PcX86_64 {
    /// Parse the firmware interrupt topology.
    ///
    /// # Safety
    ///
    /// `hhdm` must be the correct offset for the HHDM and must stay valid for
    /// this call. `rsdp_phys` must point at a valid RSDP.
    pub unsafe fn parse_config(
        hhdm: usize,
        rsdp_phys: usize,
        nanos_since_boot: NanosFn,
    ) -> Result<InterruptConfig, InterruptConfigError> {
        unsafe { kernel_acpi::parse(hhdm, rsdp_phys, nanos_since_boot) }.map_err(Into::into)
    }

    /// Bring up the platform interrupt controllers.
    ///
    /// # Safety
    ///
    /// Must run on the BSP with interrupts disabled, after `parse_config` and
    /// after the arch has brought up the local APIC. `map_mmio` must map device
    /// memory. `cpus` maps logical CPU ids to hardware APIC ids.
    pub unsafe fn init_interrupt_controllers(
        config: &InterruptConfig,
        bsp_id: u32,
        cpus: &[(CpuId, u32)],
        map_mmio: MapMmioFn,
    ) {
        pic::disable(config.has_legacy_pic);
        ioapic::init(config, bsp_id, cpus, map_mmio);
    }
}

impl InterruptControllerCapability for PcX86_64 {
    fn route(spec: IrqSpec, target: IrqTarget) -> Result<(), IrqError> {
        ioapic::IoApicManager::global()?.route(spec, target)
    }

    fn mask(spec: IrqSpec) {
        if let Ok(mgr) = ioapic::IoApicManager::global() {
            mgr.set_masked(spec, true);
        }
    }

    fn unmask(spec: IrqSpec) {
        if let Ok(mgr) = ioapic::IoApicManager::global() {
            mgr.set_masked(spec, false);
        }
    }

    fn unroute(spec: IrqSpec) {
        if let Ok(mgr) = ioapic::IoApicManager::global() {
            mgr.park(spec);
        }
    }
}

impl IsaIrqCapability for PcX86_64 {
    fn isa_irq(irq: u8) -> Result<IrqSpec, IrqError> {
        Ok(ioapic::IoApicManager::global()?.isa_irq_line(irq))
    }
}
