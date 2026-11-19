//! Interrupt-controller bring-up: legacy PIC shutdown, local APIC (x2APIC when the CPU has it),
//! I/O APIC and the timers.

use core::sync::atomic::{AtomicUsize, Ordering};

use kernel_acpi::InterruptConfig;
use mem_layout::{MMIO_MAX_SIZE, MMIO_START};
use platform::serial_println;
use x86_64::{
    PhysAddr,
    VirtAddr,
    structures::paging::{Mapper, Page, PageSize, PageTableFlags, PhysFrame, Size4KiB},
};

use crate::{frame::get_frame_alloc, page::get_pagetable};

const TICK_PERIOD_MS: u32 = 10;

static NEXT_MMIO_VIRT: AtomicUsize = AtomicUsize::new(MMIO_START);

// TODO: proper allocator once pci is added
/// Map `len` bytes of device memory at `phys` as uncached and return the virtual address of
/// `phys` itself (i.e. including the in-page offset).
///
/// Never unmaps: the MMIO window is a bump allocator currently
///
/// # SAFETY
/// the phys must be point to correct device memory that we actually may use here/own.
pub(crate) unsafe fn map_no_cache(phys: PhysAddr, len: usize) -> VirtAddr {
    debug_assert!(len > 0, "zero-length MMIO mapping");

    let page_size = Size4KiB::SIZE as usize;
    let page_offset = (phys.as_u64() & (Size4KiB::SIZE - 1)) as usize;
    let total_bytes = (page_offset + len).next_multiple_of(page_size);
    let num_pages = (total_bytes / page_size) as u64;

    let virt_base = NEXT_MMIO_VIRT
        .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
            let end = cur.checked_add(total_bytes)?;
            (end <= MMIO_START + MMIO_MAX_SIZE).then_some(end)
        })
        .expect("MMIO virtual window exhausted");

    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_base as u64));
    let start_frame = PhysFrame::<Size4KiB>::containing_address(phys);

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH
        | PageTableFlags::NO_EXECUTE;

    let mut mapper = get_pagetable().lock();
    let mut allocator = get_frame_alloc().lock();
    for i in 0..num_pages {
        unsafe { mapper.map_to(start_page + i, start_frame + i, flags, &mut *allocator) }
            .unwrap()
            .flush();
    }

    VirtAddr::new((virt_base + page_offset) as u64)
}

/// Bring up the interrupt hardware on the bootstrap processor. Returns the BSP's APIC ID.
///
/// # SAFETY
/// Interrupts must still be disabled, and `timer::init_clock()` must already have run.
pub(crate) unsafe fn init_bsp(config: &InterruptConfig) -> u32 {
    unsafe { pic::disable(config.has_legacy_pic) };

    let msr_phys = unsafe { x2apic::lapic::xapic_base() };
    if msr_phys != config.lapic_phys {
        serial_println!(
            "[WARN] MADT LAPIC {:#x} != IA32_APIC_BASE {:#x}; using MSR",
            config.lapic_phys,
            msr_phys
        );
    }

    lapic::init_global(msr_phys, &config.lapic_nmi);
    let bsp_id = unsafe { lapic::init_core_lapic() };
    timer::calibrate_lapic_timer();

    ioapic::init(config, bsp_id);

    bsp_id
}

/// Bring up the local APIC on an ap. Returns its APIC ID.
///
/// # Safety
/// `init_bsp` must have completed, and the calling core's per-core data (GS base) must be set up.
pub(crate) unsafe fn init_ap() -> u32 {
    unsafe { lapic::init_core_lapic() }
}

/// Start the periodic scheduler tick on the calling core.
pub(crate) fn enable_timer() {
    unsafe { timer::start_periodic_timer(TICK_PERIOD_MS) };
}

pub(crate) mod lapic {
    //! Per-core local APIC handling on top of the `x2apic` crate.

    use alloc::boxed::Box;

    use conquer_once::spin::OnceCell;
    use core_local::CLSLazyPtr;
    use kernel_acpi::LapicNmiTable;
    use platform::serial_println;
    use x2apic::lapic::{LocalApic, LocalApicBuilder, TimerDivide, TimerMode};
    use x86_64::{PhysAddr, VirtAddr, instructions::interrupts, registers::model_specific::Msr};

    use super::{cpuid, map_no_cache};
    use crate::idt::InterruptIndex;

    pub(super) const TIMER_DIVIDE: TimerDivide = TimerDivide::Div16;

    const XAPIC_MMIO_LEN: usize = 0x1000;
    /// x2APIC registers live at MSR 0x800 + (xAPIC MMIO offset >> 4).
    const X2APIC_MSR_BASE: u32 = 0x800;

    /// xAPIC MMIO register offsets.
    mod reg {
        pub(crate) const TPR: u32 = 0x080;
        pub(crate) const LVT_LINT0: u32 = 0x350;
        pub(crate) const LVT_LINT1: u32 = 0x360;
    }

    /// LVT entry: delivery mode NMI, edge triggered, active high, not masked.
    const LVT_NMI: u32 = 0b100 << 8;

    /// LVT entry: delivery mode NMI (100b), unmasked.
    const LVT_NMI_BASE: u32 = 0b100 << 8;
    const LVT_PIN_POLARITY_LOW: u32 = 1 << 13;
    const LVT_TRIGGER_LEVEL: u32 = 1 << 15;

    fn lvt_nmi_value(nmi: kernel_acpi::LapicNmi) -> u32 {
        let mut val = LVT_NMI_BASE;
        if nmi.active_low {
            val |= LVT_PIN_POLARITY_LOW;
        }
        if nmi.level {
            val |= LVT_TRIGGER_LEVEL;
        }
        val
    }

    #[derive(Clone, Copy, Debug)]
    enum Mode {
        X2Apic,
        XApic { virt: VirtAddr },
    }

    static MODE: OnceCell<Mode> = OnceCell::uninit();
    static NMI_TABLE: OnceCell<LapicNmiTable> = OnceCell::uninit();

    pub(crate) struct CoreLapic {
        lapic: LocalApic,
        id: u32,
    }
    // SAFETY: only ever created, stored, and accessed on the owning CPU.
    unsafe impl Send for CoreLapic {}

    #[core_local::core_local]
    static LAPIC: CLSLazyPtr<CoreLapic> = CLSLazyPtr::new();

    /// One-time, system-wide setup. Must run on the BSP before any `init_core_lapic`.
    pub(super) fn init_global(msr_phys: u64, nmi: &LapicNmiTable) {
        let mode = *MODE.get_or_init(|| {
            if cpuid::has_x2apic() {
                Mode::X2Apic
            } else {
                Mode::XApic {
                    virt: unsafe { map_no_cache(PhysAddr::new(msr_phys), XAPIC_MMIO_LEN) },
                }
            }
        });
        serial_println!("[apic] local APIC mode: {:?}", mode);
        NMI_TABLE.init_once(|| nmi.clone());
    }

    /// Write a local-APIC register (by xAPIC MMIO offset) in the active mode.
    unsafe fn write_reg(offset: u32, value: u32) {
        match MODE.get().expect("local APIC mode not initialised") {
            Mode::X2Apic => unsafe {
                Msr::new(X2APIC_MSR_BASE + (offset >> 4)).write(u64::from(value));
            },
            Mode::XApic { virt } => unsafe {
                virt.as_mut_ptr::<u32>()
                    .byte_add(offset as usize)
                    .write_volatile(value);
            },
        }
    }

    /// Enable the local APIC of the calling core and register it in core local storage.
    /// Returns the core's APIC ID.
    ///
    /// # Safety
    /// core local must be set up, and `init_global` must have run.
    pub(crate) unsafe fn init_core_lapic() -> u32 {
        let mut builder = LocalApicBuilder::new();
        builder
            .timer_vector(InterruptIndex::Timer as usize)
            .error_vector(InterruptIndex::Error as usize)
            .spurious_vector(InterruptIndex::Spurious as usize)
            .timer_mode(TimerMode::OneShot)
            .timer_divide(TIMER_DIVIDE)
            .timer_initial(0);

        if let Some(Mode::XApic { virt }) = MODE.get() {
            builder.set_xapic_base(virt.as_u64());
        }
        let mut lapic = builder.build().expect("failed to build the local APIC");

        unsafe {
            // Also masks LINT0/LINT1 and the timer.
            lapic.enable();
            lapic.disable_timer();
        }
        let id = unsafe { lapic.id() };

        unsafe {
            write_reg(reg::TPR, 0);

            let [lint0, lint1] = NMI_TABLE
                .get()
                .expect("local APIC NMI table not initialised")
                .lints_for(id);
            if let Some(nmi) = lint0 {
                write_reg(reg::LVT_LINT0, lvt_nmi_value(nmi));
            }
            if let Some(nmi) = lint1 {
                write_reg(reg::LVT_LINT1, lvt_nmi_value(nmi));
            }
        }

        LAPIC.with(|slot| slot.get_or_init(|| Box::new(CoreLapic { lapic, id })));
        id
    }

    pub(crate) fn with_lapic<R>(f: impl FnOnce(&mut LocalApic) -> R) -> R {
        interrupts::without_interrupts(|| {
            LAPIC.with_mut(|slot| f(&mut slot.get_mut().unwrap().lapic))
        })
    }

    pub(crate) fn lapic_id() -> u32 {
        LAPIC.with(|slot| slot.get().unwrap().id)
    }

    #[inline]
    pub(crate) fn end_interrupt() {
        with_lapic(|l| unsafe { l.end_of_interrupt() });
    }
}

pub(crate) mod ioapic {
    //! I/O APIC management.

    use alloc::vec::Vec;

    use conquer_once::spin::OnceCell;
    use kernel_acpi::{InterruptConfig, IoApicInfo, IsaOverride};
    use spin::Mutex;
    use x2apic::ioapic::{IoApic, IrqFlags, IrqMode, RedirectionTableEntry};
    use x86_64::{PhysAddr, instructions::interrupts};

    use super::map_no_cache;
    use crate::idt::InterruptIndex;

    const IOAPIC_MMIO_LEN: usize = 0x1000;

    static IOAPIC: OnceCell<IoApicManager> = OnceCell::uninit();

    /// Discover and park every I/O APIC, then publish the manager
    pub(super) fn init(config: &InterruptConfig, bsp_id: u32) {
        let mgr = unsafe { IoApicManager::new(config) };
        for nmi in &config.nmi_sources {
            mgr.route_nmi(nmi.gsi, bsp_id, nmi.active_low, nmi.level);
        }
        IOAPIC.init_once(|| mgr);
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
        /// Panics if `apic::init_bsp` has not run yet.
        pub(crate) fn global() -> &'static Self {
            IOAPIC.get().expect("I/O APIC not initialised")
        }

        unsafe fn new(config: &InterruptConfig) -> Self {
            let ioapics = config
                .ioapics
                .iter()
                .map(|info| unsafe {
                    let virt = map_no_cache(PhysAddr::new(info.phys), IOAPIC_MMIO_LEN);
                    let mut ioapic = IoApic::new(virt.as_u64());
                    let max_entry = ioapic.max_table_entry();

                    // Firmware may have left pins unmasked: park every entry masked on a harmless vector.
                    for pin in 0..=max_entry {
                        let mut parked = RedirectionTableEntry::default();
                        parked.set_mode(IrqMode::Fixed);
                        parked.set_flags(IrqFlags::MASKED);
                        parked.set_vector(InterruptIndex::Spurious as u8);
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
        pub(crate) fn route_isa_irq(
            &self,
            isa_irq: u8,
            vector: u8,
            dest_apic_id: u32,
            masked: bool,
        ) {
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

                unsafe {
                    target.ioapic.set_table_entry(pin, entry);
                    if !masked {
                        target.ioapic.enable_irq(pin);
                    }
                }
            });
        }
    }
}

pub(crate) mod timer {
    //! Time sources: the TSC (monotonic clock) and the LAPIC timer (scheduler tick).
    //!
    //! Calibration strategy:
    //! * the TSC is calibrated once, against PIT channel 2, with a bounded wait
    //! * the LAPIC timer is then calibrated against the already-calibrated TSC, so the PIT is only
    //!   touched once and this step can be reused unchanged if the TSC source is swapped for
    //!   CPUID 0x15/0x16, HPET or the ACPI PM timer later.

    use core::{
        hint::spin_loop,
        sync::atomic::{AtomicU32, AtomicU64, Ordering},
    };

    use platform::serial_println;
    use x2apic::lapic::{LocalApic, TimerMode};
    use x86_64::instructions::port::Port;

    use super::{
        cpuid,
        lapic::{TIMER_DIVIDE, with_lapic},
    };

    const PIT_HZ: u64 = 1_193_182;
    const TSC_CAL_MS: u64 = 10;
    const LAPIC_CAL_MS: u64 = 10;
    const CAL_TRIALS: usize = 3;
    const PIT_COUNT: u16 = ((PIT_HZ * TSC_CAL_MS) / 1000) as u16;
    /// Give up on the PIT after this many TSC cycles (>= 1 s on any CPU up to 4 GHz, and the
    /// calibration window is only 10 ms).
    const PIT_TIMEOUT_CYCLES: u64 = 4_000_000_000;
    /// Sanity bounds for a calibrated TSC frequency.
    const TSC_HZ_RANGE: core::ops::RangeInclusive<u64> = 50_000_000..=20_000_000_000;

    static TSC_HZ: AtomicU64 = AtomicU64::new(0);
    /// TSC value on the BSP at `init_clock`. Reading `nanos_since_boot` from other cores assumes the
    /// firmware left the TSCs synchronised (true on virtually all invariant-TSC machines).
    pub(crate) static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

    pub(crate) static LAPIC_TICKS_PER_MS: AtomicU32 = AtomicU32::new(0);

    #[inline]
    pub(crate) fn rdtsc_serialized() -> u64 {
        unsafe {
            core::arch::x86_64::_mm_lfence();
            let t = core::arch::x86_64::_rdtsc();
            core::arch::x86_64::_mm_lfence();
            t
        }
    }

    /// Start the monotonic clock: record the boot TSC and calibrate its frequency.
    ///
    /// # Panics
    /// If the PIT does not respond or yields an implausible frequency.
    pub(crate) fn init_clock(tsc_hz: Option<u64>) {
        let hz = if let Some(tsc_hz) = tsc_hz {
            tsc_hz
        } else {
            BOOT_TSC.store(rdtsc_serialized(), Ordering::Relaxed);

            if !cpuid::has_invariant_tsc() {
                serial_println!(
                    "[WARN] TSC is not invariant: nanos_since_boot() may drift or jump"
                );
            }
            if !cpuid::has_arat() {
                serial_println!("[WARN] no ARAT: the LAPIC timer may stop in deep C-states");
            }

            let hz = calibrate_tsc_with_pit()
                .expect("TSC calibration failed: PIT channel 2 did not complete (no usable PIT?)");
            if !TSC_HZ_RANGE.contains(&hz) {
                serial_println!("[WARN]: implausible calibrated tsc frequency: {}", hz);
            }
            hz
        };
        TSC_HZ.store(hz, Ordering::Release);
        serial_println!("[timer] TSC: {} MHz", hz / 1_000_000);
    }

    /// Nanoseconds since `init_clock`.
    ///
    /// Assumes previous call to `init_clock`.
    pub(crate) fn nanos_since_boot() -> u64 {
        let hz = TSC_HZ.load(Ordering::Acquire);

        let ticks = rdtsc_serialized().wrapping_sub(BOOT_TSC.load(Ordering::Relaxed));

        let secs = ticks / hz;
        let rem = ticks % hz;
        secs * 1_000_000_000 + rem * 1_000_000_000 / hz
    }

    /// One PIT measurement: TSC cycles elapsed during `PIT_COUNT` PIT ticks, or `None` on timeout.
    fn pit_measure_tsc_once() -> Option<u64> {
        let mut port_b = Port::<u8>::new(0x61);
        let mut cmd = Port::<u8>::new(0x43);
        let mut ch2 = Port::<u8>::new(0x42);

        unsafe {
            let b = port_b.read() & !0x03; // GATE2 = 0, speaker off
            port_b.write(b);
            cmd.write(0xb0); // channel 2, lobyte/hibyte, mode 0, binary
            ch2.write(PIT_COUNT as u8);
            ch2.write((PIT_COUNT >> 8) as u8);

            port_b.write(b | 0x01); // raise GATE2: start counting
            let t0 = rdtsc_serialized();

            let result = loop {
                if port_b.read() & 0x20 != 0 {
                    // OUT2 went high: count expired.
                    break Some(rdtsc_serialized().wrapping_sub(t0));
                }
                if rdtsc_serialized().wrapping_sub(t0) > PIT_TIMEOUT_CYCLES {
                    break None;
                }
                spin_loop();
            };

            port_b.write(b); // gate low again
            result
        }
    }

    fn calibrate_tsc_with_pit() -> Option<u64> {
        let mut samples = [0u64; CAL_TRIALS];
        for sample in &mut samples {
            *sample = pit_measure_tsc_once()?;
        }
        samples.sort_unstable();
        Some(samples[CAL_TRIALS / 2] * PIT_HZ / u64::from(PIT_COUNT))
    }

    /// Count LAPIC timer ticks over `window_tsc` TSC cycles.
    unsafe fn measure_lapic_once(l: &mut LocalApic, window_tsc: u64) -> u32 {
        unsafe {
            // A masked LVT still counts down; we only need the counter, not interrupts.
            l.disable_timer();
            l.set_timer_mode(TimerMode::OneShot);
            l.set_timer_divide(TIMER_DIVIDE);

            let t0 = rdtsc_serialized();
            l.set_timer_initial(u32::MAX);
            while rdtsc_serialized().wrapping_sub(t0) < window_tsc {
                spin_loop();
            }
            let remaining = l.timer_current();
            l.set_timer_initial(0);
            u32::MAX - remaining
        }
    }

    /// Calibrate the LAPIC timer of the BSP against the TSC. APs reuse the result.
    ///
    /// assumes `init_clock` has run.
    pub(crate) fn calibrate_lapic_timer() {
        let tsc_hz = TSC_HZ.load(Ordering::Acquire);

        let window_tsc = tsc_hz * LAPIC_CAL_MS / 1000;

        let mut samples = [0u32; CAL_TRIALS];
        with_lapic(|l| {
            for sample in &mut samples {
                *sample = unsafe { measure_lapic_once(l, window_tsc) };
            }
        });
        samples.sort_unstable();

        let per_ms = samples[CAL_TRIALS / 2] / LAPIC_CAL_MS as u32;
        assert!(per_ms != 0, "LAPIC timer calibration failed");
        LAPIC_TICKS_PER_MS.store(per_ms, Ordering::Release);
        serial_println!("[timer] LAPIC timer: {} ticks/ms (divide 16)", per_ms);
    }

    /// Start the periodic tick on the calling core.
    ///
    /// # Safety
    /// The caller must have a handler installed for the timer vector.
    pub(crate) unsafe fn start_periodic_timer(period_ms: u32) {
        let per_ms = LAPIC_TICKS_PER_MS.load(Ordering::Acquire);
        let count = per_ms.checked_mul(period_ms).expect("timer count overflow");
        assert!(count != 0);

        with_lapic(|l| unsafe {
            l.set_timer_mode(TimerMode::Periodic);
            l.set_timer_divide(TIMER_DIVIDE);
            l.set_timer_initial(count);
            l.enable_timer();
        });
    }
}

mod pic {
    //! Legacy 8259 PIC shutdown.

    use x86_64::instructions::port::Port;

    /// Vector base the PICs are remapped to before being masked, so that even a spurious IRQ7/15
    /// cannot alias a CPU exception (0x00..0x20). Spurious IRQs still arrive at BASE+7 / BASE+15.
    const PIC_VECTOR_BASE: u8 = 0xE0;

    const PIC1_CMD: u16 = 0x20;
    const PIC1_DATA: u16 = 0x21;
    const PIC2_CMD: u16 = 0xA0;
    const PIC2_DATA: u16 = 0xA1;
    const IO_WAIT_PORT: u16 = 0x80;

    const ICW1_INIT_ICW4: u8 = 0x11;
    const ICW3_MASTER_SLAVE_ON_IRQ2: u8 = 1 << 2;
    const ICW3_SLAVE_ID: u8 = 2;
    const ICW4_8086: u8 = 0x01;
    const MASK_ALL: u8 = 0xFF;

    /// each chip must see ICW1..ICW4 in sequence, then the mask.
    const SEQUENCE: [(u16, u8); 10] = [
        (PIC1_CMD, ICW1_INIT_ICW4),
        (PIC2_CMD, ICW1_INIT_ICW4),
        (PIC1_DATA, PIC_VECTOR_BASE),
        (PIC2_DATA, PIC_VECTOR_BASE + 8),
        (PIC1_DATA, ICW3_MASTER_SLAVE_ON_IRQ2),
        (PIC2_DATA, ICW3_SLAVE_ID),
        (PIC1_DATA, ICW4_8086),
        (PIC2_DATA, ICW4_8086),
        (PIC1_DATA, MASK_ALL),
        (PIC2_DATA, MASK_ALL),
    ];

    /// Remap and mask both 8259s.
    ///
    /// This is done unconditionally: on machines without PICs the writes are harmless, and on
    /// machines that report them (MADT `PCAT_COMPAT`) we must not leave them live.
    /// If `route_imcr` is set, the IMCR is also switched so that external interrupts reach the APIC
    /// instead of being wired straight to the CPU INTR pin.
    ///
    /// # Safety
    /// Raw port I/O; must run with interrupts disabled during early boot.
    pub(super) unsafe fn disable(route_imcr: bool) {
        let mut wait = Port::<u8>::new(IO_WAIT_PORT);

        for (port, value) in SEQUENCE {
            unsafe {
                Port::<u8>::new(port).write(value);
                wait.write(0);
            }
        }

        if route_imcr {
            const IMCR_SELECT: u16 = 0x22;
            const IMCR_DATA: u16 = 0x23;
            unsafe {
                Port::<u8>::new(IMCR_SELECT).write(0x70);
                Port::<u8>::new(IMCR_DATA).write(0x01);
            }
        }
    }
}

pub(crate) mod cpuid {
    use raw_cpuid::CpuId;
    pub(crate) fn has_x2apic() -> bool {
        CpuId::new()
            .get_feature_info()
            .is_some_and(|f| f.has_x2apic())
    }

    pub(crate) fn has_invariant_tsc() -> bool {
        CpuId::new()
            .get_advanced_power_mgmt_info()
            .is_some_and(|apm| apm.has_invariant_tsc())
    }

    pub(crate) fn has_arat() -> bool {
        CpuId::new()
            .get_thermal_power_info()
            .is_some_and(|tp| tp.has_arat())
    }

    pub(crate) fn initial_apic_id() -> u32 {
        let cpuid = CpuId::new();
        if let Some(mut topo) = cpuid.get_extended_topology_info()
            && let Some(level) = topo.next()
        {
            return level.x2apic_id();
        }
        cpuid
            .get_feature_info()
            .map_or(0, |f| u32::from(f.initial_local_apic_id()))
    }
}
