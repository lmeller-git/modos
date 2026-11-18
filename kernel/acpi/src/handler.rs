#![expect(clippy::todo)]

// TODO: currently just uses the bootloaders hhdm. Write back MMIO will need uncached mapping equivalent to apic setup.

use core::{hint::spin_loop, ptr::NonNull};

use acpi::{Handler, PciAddress, PhysicalMapping};
use x86_64::instructions::port::Port;

use crate::NanosFn;

#[derive(Clone, Copy)]
pub struct KernelAcpiHandler {
    hhdm: usize,
    nanos_since_boot: NanosFn,
}

impl KernelAcpiHandler {
    pub const fn new(hhdm: usize, nanos_since_boot: NanosFn) -> Self {
        Self {
            hhdm,
            nanos_since_boot,
        }
    }

    #[inline]
    fn virt(&self, phys: usize) -> usize {
        phys.checked_add(self.hhdm)
            .expect("ACPI physical address overflows the HHDM")
    }
}

impl Handler for KernelAcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        // TODO callback into arch for proper mapping. Needs indirection again.
        physical_address
            .checked_add(size)
            .expect("ACPI region wraps the physical address space");
        let virt = NonNull::new(self.virt(physical_address) as *mut T).expect("null ACPI mapping");
        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: virt,
            region_length: size,
            mapped_length: size,
            handler: *self,
        }
    }

    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}

    fn read_u8(&self, a: usize) -> u8 {
        unsafe { (self.virt(a) as *const u8).read_volatile() }
    }

    fn read_u16(&self, a: usize) -> u16 {
        unsafe { (self.virt(a) as *const u16).read_volatile() }
    }

    fn read_u32(&self, a: usize) -> u32 {
        unsafe { (self.virt(a) as *const u32).read_volatile() }
    }

    fn read_u64(&self, a: usize) -> u64 {
        unsafe { (self.virt(a) as *const u64).read_volatile() }
    }

    fn write_u8(&self, a: usize, v: u8) {
        unsafe { (self.virt(a) as *mut u8).write_volatile(v) }
    }

    fn write_u16(&self, a: usize, v: u16) {
        unsafe { (self.virt(a) as *mut u16).write_volatile(v) }
    }

    fn write_u32(&self, a: usize, v: u32) {
        unsafe { (self.virt(a) as *mut u32).write_volatile(v) }
    }

    fn write_u64(&self, a: usize, v: u64) {
        unsafe { (self.virt(a) as *mut u64).write_volatile(v) }
    }

    fn read_io_u8(&self, p: u16) -> u8 {
        unsafe { Port::<u8>::new(p).read() }
    }

    fn read_io_u16(&self, p: u16) -> u16 {
        unsafe { Port::<u16>::new(p).read() }
    }

    fn read_io_u32(&self, p: u16) -> u32 {
        unsafe { Port::<u32>::new(p).read() }
    }

    fn write_io_u8(&self, p: u16, v: u8) {
        unsafe { Port::<u8>::new(p).write(v) }
    }

    fn write_io_u16(&self, p: u16, v: u16) {
        unsafe { Port::<u16>::new(p).write(v) }
    }

    fn write_io_u32(&self, p: u16, v: u32) {
        unsafe { Port::<u32>::new(p).write(v) }
    }

    // needs MCFG/ECAM.
    // TODO implement with PCI system
    fn read_pci_u8(&self, _a: PciAddress, _o: u16) -> u8 {
        todo!("ACPI: PCI config read")
    }

    fn read_pci_u16(&self, _a: PciAddress, _o: u16) -> u16 {
        todo!("ACPI: PCI config read")
    }

    fn read_pci_u32(&self, _a: PciAddress, _o: u16) -> u32 {
        todo!("ACPI: PCI config read")
    }

    fn write_pci_u8(&self, _a: PciAddress, _o: u16, _v: u8) {
        todo!("ACPI: PCI config write")
    }

    fn write_pci_u16(&self, _a: PciAddress, _o: u16, _v: u16) {
        todo!("ACPI: PCI config write")
    }

    fn write_pci_u32(&self, _a: PciAddress, _o: u16, _v: u32) {
        todo!("ACPI: PCI config write")
    }

    fn nanos_since_boot(&self) -> u64 {
        (self.nanos_since_boot)()
    }

    fn stall(&self, microseconds: u64) {
        let end = self
            .nanos_since_boot()
            .saturating_add(microseconds.saturating_mul(1_000));
        while self.nanos_since_boot() < end {
            spin_loop();
        }
    }

    fn sleep(&self, milliseconds: u64) {
        // TODO: yield to the scheduler
        self.stall(milliseconds.saturating_mul(1_000));
    }

    #[cfg(feature = "aml")]
    fn create_mutex(&self) -> acpi::Handle {
        todo!("AML mutex")
    }

    #[cfg(feature = "aml")]
    fn acquire(&self, _m: acpi::Handle, _timeout: u16) -> Result<(), acpi::aml::AmlError> {
        todo!("AML mutex")
    }

    #[cfg(feature = "aml")]
    fn release(&self, _m: acpi::Handle) {
        todo!("AML mutex")
    }
}
