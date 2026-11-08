#![no_std]

extern crate alloc;

use acpi::{
    AcpiTables,
    platform::{AcpiPlatform, InterruptModel, interrupt::Apic},
    sdt::madt::{Polarity, TriggerMode},
};

use crate::handler::KernelAcpiHandler;

mod handler;

pub struct IoApicInfo {
    pub id: u8,
    pub phys: u64,
    pub gsi_base: u32,
}

pub struct IsaOverride {
    pub isa_irq: u8,
    pub gsi: u32,
    pub active_low: bool,
    pub level: bool,
}

pub struct InterruptConfig {
    pub lapic_phys: u64,
    pub ioapics: Vec<IoApicInfo>,
    pub overrides: Vec<IsaOverride>,
    pub has_legacy_pic: bool,
}

pub fn parse(hhdm: usize, rsdp_phys: usize) -> InterruptConfig {
    let mut handler = KernelAcpiHandler::new(hhdm);
    let tables = unsafe { AcpiTables::from_rsdp(&mut handler, rsdp_phys) }.unwrap();
    let platform = AcpiPlatform::new(tables, &mut handler).unwrap();

    match platform.interrupt_model {
        InterruptModel::Apic(apic) => apic_parse(apic),
        _ => todo!(),
    }
}

fn apic_parse(apic: Apic) -> InterruptConfig {
    InterruptConfig {
        lapic_phys: apic.local_apic_address,
        ioapics: apic
            .io_apics
            .iter()
            .map(|io| IoApicInfo {
                id: io.id,
                phys: io.address as u64,
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

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
