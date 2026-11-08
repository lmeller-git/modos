use core::{
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

use acpi::{Handler, PciAddress, PhysicalMapping};
#[cfg(target_arch = "x86_64")]
use x86_64::instructions::port;

struct Port<T>(#[cfg(target_arch = "x86_64")] port::Port<T>);

impl Port {
    fn new(port: T) -> Self {
        #[cfg(target_arch = "x86_64")]
        Self(port::Port::new(port))
    }

    unsafe fn read(&mut self) -> T {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.0.read()
        }
    }

    unsafe fn write(&mut self, v: T) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.0.write(v);
        }
    }
}

pub static TSC_HZ: AtomicU64 = AtomicU64::new(0);
pub static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct KernelAcpiHandler {
    hhdm: usize,
}

impl KernelAcpiHandler {
    pub const fn new(hhdm: usize) -> Self {
        Self { hhdm }
    }

    #[inline]
    fn virt(&self, phys: usize) -> usize {
        phys + self.hhdm
    }
}

#[inline]
fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_lfence();
        let t = core::arch::x86_64::_rdtsc();
        core::arch::x86_64::_mm_lfence();
        t
    }
}

impl Handler for KernelAcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let virt = NonNull::new(self.virt(physical_address) as *mut T).expect("null ACPI mapping");
        unsafe { PhysicalMapping::new(physical_address, virt, size, size, *self) }
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
        let hz = TSC_HZ.load(Ordering::Relaxed);
        debug_assert!(hz != 0, "Need to calibrate TSC");
        let ticks = rdtsc().wrapping_sub(BOOT_TSC.load(Ordering::Relaxed));
        ((ticks as u128 * 1_000_000_000) / hz as u128) as u64
    }

    fn stall(&self, microseconds: u64) {
        let end = self.nanos_since_boot() + microseconds * 1_000;
        while self.nanos_since_boot() < end {
            core::hint::spin_loop();
        }
    }

    fn sleep(&self, milliseconds: u64) {
        // TODO yield
        self.stall(milliseconds * 1_000);
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
