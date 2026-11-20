//! Firmware-discovered interrupt topology and the platform interrupt-controller capability.
//!
//! For now this is somewhat targeted at x86_64, TODO: make it more generic if needed.

use alloc::vec::Vec;

/// Clock returning nanoseconds since boot.
///
/// May return meaningless values before the arch has initialised its timers.
pub type NanosFn = fn() -> u64;

/// Maps `len` bytes of device memory starting at `phys` and returns the virtual address of `phys`.
///
/// # Safety
///
/// `phys` must reference device memory owned by the caller, and the mapping must stay valid for
/// as long as the device is in use.
pub type MapMmioFn = unsafe fn(phys: u64, len: usize) -> u64;

/// One I/O APIC as described by firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicInfo {
    pub id: u8,
    pub phys: u64,
    pub gsi_base: u32,
}

/// A firmware override for a legacy ISA IRQ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsaOverride {
    pub isa_irq: u8,
    pub gsi: u32,
    pub active_low: bool,
    pub level: bool,
}

/// A firmware-described NMI source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NmiSource {
    pub gsi: u32,
    pub active_low: bool,
    pub level: bool,
}

/// A processor as described by the MADT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuInfo {
    /// ACPI processor UID.
    pub uid: u32,
    /// Local APIC ID.
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

/// Firmware-described interrupt topology of the machine.
pub struct InterruptConfig {
    pub lapic_phys: u64,
    pub lapic_nmi: LapicNmiTable,
    pub nmi_sources: Vec<NmiSource>,
    pub ioapics: Vec<IoApicInfo>,
    pub overrides: Vec<IsaOverride>,
    pub has_legacy_pic: bool,
}

/// firmware discovery failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptConfigError {
    /// The firmware tables are missing or malformed.
    Firmware,
    /// Firmware describes no APIC interrupt model.
    NoApic,
}

/// Platform-owned interrupt-controller discovery and routing.
///
/// Concerns are ACPI MADT, legacy PIC and I/O APICs.
pub trait InterruptControllerCapability {
    /// Parse the firmware interrupt topology.
    ///
    /// # Safety
    ///
    /// `hhdm` must be the correct offset for the hhdm and the hhdm must stay valid for this call.
    /// `rsdp_phys` must be a physical address pointing to the rsdp.
    unsafe fn parse_config(
        hhdm: usize,
        rsdp_phys: usize,
        nanos_since_boot: NanosFn,
    ) -> Result<InterruptConfig, InterruptConfigError>;

    /// Bring up the platform interrupt controllers.
    ///
    /// # Safety
    ///
    /// Must run on the BSP with interrupts disabled, after `discover_interrupts` and after the
    /// arch has brought up the local APIC. `map_mmio` must map device memory.
    unsafe fn init_interrupt_controllers(
        config: &InterruptConfig,
        bsp_id: u32,
        map_mmio: MapMmioFn,
    );

    /// Route legacy ISA IRQ `isa_irq`, honouring firmware interrupt-source overrides.
    fn route_isa_irq(isa_irq: u8, vector: u8, dest_apic_id: u32, masked: bool);

    /// Route global system interrupt `gsi` to `vector` on `dest_apic_id`.
    fn route_gsi(
        gsi: u32,
        vector: u8,
        dest_apic_id: u32,
        active_low: bool,
        level: bool,
        masked: bool,
    );

    /// Mask or unmask an already-routed GSI.
    fn set_gsi_masked(gsi: u32, masked: bool);
}
