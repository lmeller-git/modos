//! lapic bring-up and lapic timer.

use early_log::early_println;
use platform::interface::interrupt::LapicNmiTable;
use x86_64::instructions::interrupts;

const TICK_PERIOD_MS: u32 = 10;

/// Bring up the lapic on the bsp. Returns its apic ID.
///
/// May panic if the timer has not been initialized.
///
pub(crate) fn init_bsp(lapic_phys: u64, nmi: &LapicNmiTable) -> u32 {
    interrupts::without_interrupts(|| {
        let msr_phys = unsafe { x2apic::lapic::xapic_base() };
        if msr_phys != lapic_phys {
            early_println!(
                "[WARN] MADT Lapic {:#x} != IA32_apic_BASE {:#x}; using MSR",
                lapic_phys,
                msr_phys
            );
        }

        lapic::init_global(msr_phys, nmi);
        let bsp_id = lapic::init_core_lapic();
        timer::calibrate_lapic_timer();

        bsp_id
    })
}

/// Bring up the lapic on an ap. Returns its apic ID.
pub(crate) fn init_ap() -> u32 {
    lapic::init_core_lapic()
}

/// Start the periodic timer tick on the calling core.
pub(crate) fn enable_timer() {
    unsafe { timer::start_periodic_timer(TICK_PERIOD_MS) };
}

pub(crate) mod lapic {
    use alloc::boxed::Box;

    use conquer_once::spin::OnceCell;
    use core_local::CLSLazyPtr;
    use early_log::early_println;
    use platform::interface::interrupt::{LapicNmi, LapicNmiTable};
    use x2apic::lapic::{LocalApic, LocalApicBuilder, TimerDivide, TimerMode};
    use x86_64::{PhysAddr, VirtAddr, instructions::interrupts, registers::model_specific::Msr};

    use super::cpuid;
    use crate::{idt::InterruptIndex, mmio::map_no_cache};

    pub(super) const TIMER_DIVIDE: TimerDivide = TimerDivide::Div16;

    const XAPIC_MMIO_LEN: usize = 0x1000;
    /// x2apic registers live at MSR 0x800 + (xapic MMIO offset >> 4).
    const X2APIC_MSR_BASE: u32 = 0x800;

    /// xapic MMIO register offsets.
    mod reg {
        pub(crate) const TPR: u32 = 0x080;
        pub(crate) const LVT_LINT0: u32 = 0x350;
        pub(crate) const LVT_LINT1: u32 = 0x360;
    }

    /// LVT entry: delivery mode NMI (100b), unmasked.
    const LVT_NMI_BASE: u32 = 0b100 << 8;
    const LVT_PIN_POLARITY_LOW: u32 = 1 << 13;
    const LVT_TRIGGER_LEVEL: u32 = 1 << 15;

    fn lvt_nmi_value(nmi: LapicNmi) -> u32 {
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
        #[expect(dead_code)]
        id: u32,
    }
    // SAFETY: only ever created, stored, and accessed on the owning CPU.
    unsafe impl Send for CoreLapic {}

    #[core_local::core_local]
    static LAPIC: CLSLazyPtr<CoreLapic> = CLSLazyPtr::new();

    /// Parses and stores some information needed for all cores lapics, ...
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
        early_println!("[apic] local apic mode: {:?}", mode);
        NMI_TABLE.init_once(|| nmi.clone());
    }

    /// Write a lapic register (by xapic MMIO offset) in the active mode.
    unsafe fn write_reg(offset: u32, value: u32) {
        match MODE.get().expect("local apic mode not initialised") {
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

    /// Enable the lapic of the calling core. Returns its apic ID.
    pub(super) fn init_core_lapic() -> u32 {
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
        let mut lapic = builder.build().expect("failed to build the local apic");

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
                .expect("local apic NMI table not initialised")
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

    #[expect(dead_code)]
    pub(crate) fn lapic_id() -> u32 {
        LAPIC.with(|slot| slot.get().unwrap().id)
    }

    #[inline]
    pub(crate) fn end_interrupt() {
        with_lapic(|l| unsafe { l.end_of_interrupt() });
    }
}

pub(crate) mod timer {
    //! Time sources: the TSC (monotonic clock) and the Lapic timer (scheduler tick).
    //!
    //! Calibration strategy:
    //! * the TSC is calibrated once, against PIT channel 2, with a bounded wait
    //! * the Lapic timer is then calibrated against the already-calibrated TSC, so the PIT is only
    //!   touched once and this step can be reused unchanged if the TSC source is swapped for
    //!   CPUID 0x15/0x16, HPET or the ACPI PM timer later.

    use core::{
        hint::spin_loop,
        sync::atomic::{AtomicU32, AtomicU64, Ordering},
    };

    use early_log::early_println;
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
    pub(crate) fn init_clock(tsc_hz: Option<u64>) {
        let hz = if let Some(tsc_hz) = tsc_hz {
            tsc_hz
        } else {
            BOOT_TSC.store(rdtsc_serialized(), Ordering::Relaxed);

            if !cpuid::has_invariant_tsc() {
                early_println!("[WARN] TSC is not invariant: nanos_since_boot() may drift or jump");
            }
            if !cpuid::has_arat() {
                early_println!("[WARN] no ARAT: the Lapic timer may stop in deep C-states");
            }

            let hz = calibrate_tsc_with_pit()
                .expect("TSC calibration failed: PIT channel 2 did not complete");
            if !TSC_HZ_RANGE.contains(&hz) {
                early_println!("[WARN]: implausible calibrated tsc frequency: {}", hz);
            }
            hz
        };

        TSC_HZ.store(hz, Ordering::Release);
        early_println!("[timer] TSC: {} MHz", hz / 1_000_000);
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

    /// Count Lapic timer ticks over `window_tsc` TSC cycles.
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

    /// Calibrate the Lapic timer of the BSP against the TSC. APs reuse the result.
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
        assert!(per_ms != 0, "Lapic timer calibration failed");
        LAPIC_TICKS_PER_MS.store(per_ms, Ordering::Release);
        early_println!("[timer] Lapic timer: {} ticks/ms (divide 16)", per_ms);
    }

    /// Start the periodic tick on the calling core.
    ///
    /// # Safety
    ///
    /// The caller must have a handler installed for the timer vector.
    pub(super) unsafe fn start_periodic_timer(period_ms: u32) {
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
