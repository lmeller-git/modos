#![expect(clippy::todo)]

// TODO: currently just uses the bootloaders hhdm. Write back MMIO will need uncached mapping equivalent to apic setup.

use core::{hint::spin_loop, ptr::NonNull};

use acpi::{Handler, PciAddress, PhysicalMapping};
use x86_64::instructions::port::Port;

use crate::NanosFn;

/// Volatile access to `$ty` at a raw address. Firmware tables are not aligned to their access
/// width, so an unaligned address is accessed bytewise (still volatile, little endian).
macro_rules! volatile_access {
    ($read:ident, $write:ident, $ty:ty) => {
        unsafe fn $read(addr: usize) -> $ty {
            let ptr = addr as *const $ty;
            if ptr.is_aligned() {
                return unsafe { ptr.read_volatile() };
            }
            let mut bytes = [0u8; size_of::<$ty>()];
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = unsafe { (addr as *const u8).add(i).read_volatile() };
            }
            <$ty>::from_le_bytes(bytes)
        }

        unsafe fn $write(addr: usize, value: $ty) {
            let ptr = addr as *mut $ty;
            if ptr.is_aligned() {
                return unsafe { ptr.write_volatile(value) };
            }
            for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                unsafe { (addr as *mut u8).add(i).write_volatile(byte) };
            }
        }
    };
}

volatile_access!(read_u16, write_u16, u16);
volatile_access!(read_u32, write_u32, u32);
volatile_access!(read_u64, write_u64, u64);

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
        unsafe { read_u16(self.virt(a)) }
    }

    fn read_u32(&self, a: usize) -> u32 {
        unsafe { read_u32(self.virt(a)) }
    }

    fn read_u64(&self, a: usize) -> u64 {
        unsafe { read_u64(self.virt(a)) }
    }

    fn write_u8(&self, a: usize, v: u8) {
        unsafe { (self.virt(a) as *mut u8).write_volatile(v) }
    }

    fn write_u16(&self, a: usize, v: u16) {
        unsafe { write_u16(self.virt(a), v) }
    }

    fn write_u32(&self, a: usize, v: u32) {
        unsafe { write_u32(self.virt(a), v) }
    }

    fn write_u64(&self, a: usize, v: u64) {
        unsafe { write_u64(self.virt(a), v) }
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

    // #[cfg(feature = "aml")]
    fn create_mutex(&self) -> acpi::Handle {
        todo!("AML mutex")
    }

    // #[cfg(feature = "aml")]
    fn acquire(&self, _m: acpi::Handle, _timeout: u16) -> Result<(), acpi::aml::AmlError> {
        todo!("AML mutex")
    }

    // #[cfg(feature = "aml")]
    fn release(&self, _m: acpi::Handle) {
        todo!("AML mutex")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    const PHYS: usize = 0x1000;

    /// A handler whose "physical memory" at `PHYS..` is `mem`.
    fn over(mem: &mut [u64]) -> KernelAcpiHandler {
        KernelAcpiHandler::new(mem.as_mut_ptr() as usize - PHYS, || 0)
    }

    fn bytes(mem: &[u64]) -> alloc::vec::Vec<u8> {
        mem.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    proptest! {
        #![proptest_config(config())]

        /// Reads and writes of every width, aligned or not, go to `phys + hhdm`, little endian,
        /// and touch only their own bytes.
        #[test]
        fn access(
            mut mem in prop::array::uniform4(any::<u64>()),
            at in (0..32usize, 0..31usize, 0..29usize, 0..25usize),
            value in any::<u64>(),
        ) {
            let mut model = bytes(&mem);
            let h = over(&mut mem);
            let (a8, a16, a32, a64) = at;

            prop_assert_eq!(h.read_u8(PHYS + a8), model[a8]);
            prop_assert_eq!(h.read_u16(PHYS + a16), u16::from_le_bytes(model[a16..][..2].try_into().unwrap()));
            prop_assert_eq!(h.read_u32(PHYS + a32), u32::from_le_bytes(model[a32..][..4].try_into().unwrap()));
            prop_assert_eq!(h.read_u64(PHYS + a64), u64::from_le_bytes(model[a64..][..8].try_into().unwrap()));

            h.write_u8(PHYS + a8, value as u8);
            model[a8] = value as u8;
            h.write_u16(PHYS + a16, value as u16);
            model[a16..][..2].copy_from_slice(&(value as u16).to_le_bytes());
            h.write_u32(PHYS + a32, value as u32);
            model[a32..][..4].copy_from_slice(&(value as u32).to_le_bytes());
            h.write_u64(PHYS + a64, value);
            model[a64..][..8].copy_from_slice(&value.to_le_bytes());

            prop_assert_eq!(bytes(&mem), model);
        }
    }

    #[test]
    #[should_panic(expected = "overflows the HHDM")]
    fn address_overflowing_hhdm() {
        KernelAcpiHandler::new(usize::MAX, || 0).read_u8(1);
    }

    #[test]
    #[should_panic(expected = "wraps the physical address space")]
    fn region_wrapping() {
        let h = KernelAcpiHandler::new(0, || 0);
        let _ = unsafe { h.map_physical_region::<u8>(usize::MAX - 3, 8) };
    }
}
