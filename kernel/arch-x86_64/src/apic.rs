use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use kernel_acpi::InterruptConfig;
use mem_layout::{MMIO_MAX_SIZE, MMIO_START};
use x86_64::{
    PhysAddr,
    VirtAddr,
    instructions::port::Port,
    structures::paging::{Mapper, Page, PageSize, PageTableFlags, PhysFrame, Size4KiB},
};

use crate::{frame::get_frame_alloc, idt::InterruptIndex, page::get_pagetable, serial_println};

mod consts {
    use x2apic::lapic::TimerDivide;

    pub(super) const TIMER_DIVIDE: TimerDivide = TimerDivide::Div16;

    pub(super) const PAGE_SIZE: usize = 0x1000;
    pub(super) const LAPIC_MMIO_LEN: usize = PAGE_SIZE;
    pub(super) const IOAPIC_MMIO_LEN: usize = PAGE_SIZE;
    pub(super) const IOAPIC_BROADCAST_ID: u8 = 0xFF;
    pub(super) const IOAPIC_PARK_VECTOR: u8 = 0x20;

    pub(super) const TICK_PERIOD_MS: u32 = 10;

    // i8254 PIT, channel 2, gated via port B
    pub const PIT_HZ: u64 = 1_193_182;
    pub const PIT_CMD: u16 = 0x43;
    pub const PIT_CH2: u16 = 0x42;
    pub const PIT_CMD_CH2_LOHI_MODE0: u8 = 0b10_11_000_0; // channel 2, lo/hi byte, mode 0, binary
    pub const PORT_B: u16 = 0x61;
    pub const PORT_B_GATE2: u8 = 1 << 0;
    pub const PORT_B_SPEAKER: u8 = 1 << 1;
    pub const PORT_B_OUT2: u8 = 1 << 5;

    pub const CAL_MS: u64 = 20;
    pub const CAL_TRIALS: usize = 3;
    pub const PIT_COUNT: u16 = {
        let c = PIT_HZ * CAL_MS / 1000;
        assert!(c <= u16::MAX as u64, "PIT count overflows 16 bits");
        c as u16
    };

    // 8259 PIC
    pub(super) const PIC_VECTOR_BASE: u8 = 0xE0;
    pub(super) const PIC1_CMD: u16 = 0x20;
    pub(super) const PIC1_DATA: u16 = 0x21;
    pub(super) const PIC2_CMD: u16 = 0xA0;
    pub(super) const PIC2_DATA: u16 = 0xA1;
    pub(super) const PIC_IO_WAIT: u16 = 0x80;
    pub(super) const ICW1_INIT_ICW4: u8 = 0x11;
    pub(super) const ICW3_MASTER_SLAVE_ON_IRQ2: u8 = 1 << 2;
    pub(super) const ICW3_SLAVE_ID: u8 = 2;
    pub(super) const ICW4_8086: u8 = 0x01;
    pub(super) const PIC_MASK_ALL: u8 = 0xFF;

    // i8042
    pub const KBD_DATA: u16 = 0x60;
    pub const KBD_STATUS: u16 = 0x64;
    pub const KBD_STATUS_OBF: u8 = 1 << 0;
}

use consts::*;

static NEXT_MMIO_VIRT: AtomicUsize = AtomicUsize::new(MMIO_START);
static LAPIC_VIRT_BASE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn map_no_cache(phys: PhysAddr, len: usize) -> VirtAddr {
    debug_assert!(len > 0);
    let page_offset = phys.as_u64() & (Size4KiB::SIZE - 1);
    let start_frame = PhysFrame::<Size4KiB>::containing_address(phys);
    let end_frame = PhysFrame::<Size4KiB>::containing_address(phys + (len as u64 - 1));

    let num_pages = (end_frame.start_address().as_u64() - start_frame.start_address().as_u64())
        / Size4KiB::SIZE
        + 1;
    let total_bytes = (num_pages * Size4KiB::SIZE) as usize;

    let virt_base = NEXT_MMIO_VIRT.fetch_add(total_bytes, Ordering::AcqRel) as u64;
    assert!(
        virt_base as usize + total_bytes <= MMIO_START + MMIO_MAX_SIZE,
        "MMIO window exhausted"
    );
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_base));
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(
        virt_base + (num_pages - 1) * Size4KiB::SIZE,
    ));

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH
        | PageTableFlags::NO_EXECUTE;

    let mut mapper = get_pagetable().lock();
    let mut allocr = get_frame_alloc().lock();
    for (page, frame) in Page::range_inclusive(start_page, end_page)
        .zip(PhysFrame::range_inclusive(start_frame, end_frame))
    {
        unsafe { mapper.map_to(page, frame, flags, &mut *allocr) }
            .unwrap()
            .flush();
    }

    VirtAddr::new(virt_base + page_offset)
}

/// Remap the 8259s to PIC_VECTOR_BASE..+16 (outside the CPU exception range 0x00..0x20 and
/// outside every vector we use), then mask all lines. Spurious IRQ7/15 still arrive at +7/+15.
unsafe fn disable_legacy_pic() {
    let mut cmd1 = Port::<u8>::new(PIC1_CMD);
    let mut data1 = Port::<u8>::new(PIC1_DATA);
    let mut cmd2 = Port::<u8>::new(PIC2_CMD);
    let mut data2 = Port::<u8>::new(PIC2_DATA);
    let mut wait = Port::<u8>::new(PIC_IO_WAIT);

    unsafe fn out(port: &mut Port<u8>, wait: &mut Port<u8>, v: u8) {
        unsafe {
            port.write(v);
            wait.write(0)
        }
    }

    unsafe {
        out(&mut cmd1, &mut wait, ICW1_INIT_ICW4);
        out(&mut cmd2, &mut wait, ICW1_INIT_ICW4);
        out(&mut data1, &mut wait, PIC_VECTOR_BASE);
        out(&mut data2, &mut wait, PIC_VECTOR_BASE + 8);
        out(&mut data1, &mut wait, ICW3_MASTER_SLAVE_ON_IRQ2);
        out(&mut data2, &mut wait, ICW3_SLAVE_ID);
        out(&mut data1, &mut wait, ICW4_8086);
        out(&mut data2, &mut wait, ICW4_8086);
        out(&mut data1, &mut wait, PIC_MASK_ALL);
        out(&mut data2, &mut wait, PIC_MASK_ALL);
    }
}

pub(crate) unsafe fn __init_bsp(config: &InterruptConfig) -> (u32, ioapic::IoApicManager) {
    unsafe { disable_legacy_pic() };

    let msr_phys = unsafe { x2apic::lapic::xapic_base() };
    if msr_phys != config.lapic_phys {
        serial_println!(
            "[WARN] MADT LAPIC {:#x} != IA32_APIC_BASE {:#x}; using the MSR",
            config.lapic_phys,
            msr_phys
        );
    }
    let lapic_virt = map_no_cache(PhysAddr::new(msr_phys), LAPIC_MMIO_LEN).as_u64();
    LAPIC_VIRT_BASE.store(lapic_virt, Ordering::Release);

    let bsp_id = unsafe { lapic::init_core_lapic(lapic_virt) };
    let ioapic_mgr = unsafe { ioapic::IoApicManager::new(config) };
    (bsp_id, ioapic_mgr)
}

pub(crate) unsafe fn init_ap() -> u32 {
    let lapic_virt = LAPIC_VIRT_BASE.load(Ordering::Acquire);
    assert!(lapic_virt != 0, "init_ap before init_bsp");
    unsafe { lapic::init_core_lapic(lapic_virt) }
}

pub(crate) mod lapic {
    use alloc::boxed::Box;

    use core_local::CLSLazyPtr;
    use x2apic::lapic::{LocalApic, LocalApicBuilder, TimerMode};
    use x86_64::instructions::interrupts;

    use super::*;
    use crate::idt::InterruptIndex;

    pub(crate) struct CoreLapic(LocalApic);
    // SAFETY: only ever created, stored and used on the core it programs.
    unsafe impl Send for CoreLapic {}

    #[core_local::core_local]
    pub(super) static LAPIC_ID: u32 = 0;

    #[core_local::core_local]
    pub(super) static LAPIC: CLSLazyPtr<CoreLapic> = CLSLazyPtr::new();

    pub(crate) fn with_lapic<R>(f: impl FnOnce(&mut LocalApic) -> R) -> R {
        interrupts::without_interrupts(|| LAPIC.with_mut(|slot| f(&mut slot.get_mut().unwrap().0)))
    }

    pub(crate) fn lapic_id() -> u32 {
        LAPIC_ID.with(|id| *id)
    }

    pub unsafe fn init_core_lapic(xapic_virt: u64) -> u32 {
        let mut lapic = LocalApicBuilder::new()
            .timer_vector(InterruptIndex::Timer as usize)
            .error_vector(InterruptIndex::Error as usize)
            .spurious_vector(InterruptIndex::Spurious as usize)
            .timer_mode(TimerMode::OneShot)
            .timer_divide(TIMER_DIVIDE)
            .timer_initial(0)
            .set_xapic_base(xapic_virt)
            .build()
            .unwrap();
        unsafe {
            lapic.enable();
            lapic.disable_timer();
        }
        let id = unsafe { lapic.id() };
        LAPIC.with(|slot| slot.get_or_init(|| Box::new(CoreLapic(lapic))));
        id
    }

    #[inline]
    pub fn end_interrupt() {
        with_lapic(|l| unsafe { l.end_of_interrupt() });
    }
}

pub(crate) mod ioapic {
    use alloc::vec::Vec;

    use conquer_once::spin::OnceCell;
    use kernel_acpi::{InterruptConfig, IoApicInfo, IsaOverride};
    use spin::Mutex;
    use x2apic::ioapic::{IoApic, IrqFlags, IrqMode, RedirectionTableEntry};
    use x86_64::{PhysAddr, instructions::interrupts};

    use super::{consts::*, map_no_cache};

    pub(crate) static IOAPIC: OnceCell<IoApicManager> = OnceCell::uninit();

    struct ManagedIoApic {
        ioapic: IoApic,
        info: IoApicInfo,
        max_entry: u8,
    }
    // SAFETY: IoApic wraps an MMIO pointer; every access goes through the Mutex below.
    unsafe impl Send for ManagedIoApic {}

    pub struct IoApicManager {
        ioapics: Mutex<Vec<ManagedIoApic>>,
        overrides: Vec<IsaOverride>,
    }

    impl IoApicManager {
        pub unsafe fn new(config: &InterruptConfig) -> Self {
            let ioapics = config
                .ioapics
                .iter()
                .map(|info| unsafe {
                    let virt = map_no_cache(PhysAddr::new(info.phys), IOAPIC_MMIO_LEN);
                    let mut ioapic = IoApic::new(virt.as_u64());
                    let max_entry = ioapic.max_table_entry();
                    for pin in 0..=max_entry {
                        let mut parked = RedirectionTableEntry::default();
                        parked.set_mode(IrqMode::Fixed);
                        parked.set_flags(IrqFlags::MASKED);
                        parked.set_vector(IOAPIC_PARK_VECTOR);
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

        pub fn route_isa_irq(&self, isa_irq: u8, vector: u8, dest_apic_id: u32, masked: bool) {
            let (gsi, active_low, level) = self
                .overrides
                .iter()
                .find(|o| o.isa_irq == isa_irq)
                .map(|o| (o.gsi, o.active_low, o.level))
                .unwrap_or((isa_irq as u32, false, false));
            self.route_gsi(gsi, vector, dest_apic_id, active_low, level, masked);
        }

        pub fn route_gsi(
            &self,
            gsi: u32,
            vector: u8,
            dest_apic_id: u32,
            active_low: bool,
            level: bool,
            masked: bool,
        ) {
            let dest = u8::try_from(dest_apic_id)
                .ok()
                .filter(|&d| d != IOAPIC_BROADCAST_ID)
                .expect("APIC id not addressable via I/O APIC (needs interrupt remapping)");

            interrupts::without_interrupts(|| {
                let mut list = self.ioapics.lock();
                let target = list
                    .iter_mut()
                    .find(|io| {
                        gsi >= io.info.gsi_base && gsi <= io.info.gsi_base + io.max_entry as u32
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

                let mut e = RedirectionTableEntry::default();
                e.set_mode(IrqMode::Fixed);
                e.set_flags(flags);
                e.set_vector(vector);
                e.set_dest(dest);

                unsafe {
                    target.ioapic.set_table_entry(pin, e);
                    if !masked {
                        target.ioapic.enable_irq(pin);
                    }
                }
            });
        }
    }
}

pub(crate) mod timer {
    use core::sync::atomic::{AtomicU32, Ordering};

    use x2apic::lapic::{LocalApic, TimerMode};
    use x86_64::instructions::port::Port;

    use super::{lapic::with_lapic, *};

    pub static LAPIC_TICKS_PER_MS: AtomicU32 = AtomicU32::new(0);

    #[inline]
    pub(crate) fn rdtsc_serialized() -> u64 {
        unsafe {
            core::arch::x86_64::_mm_lfence();
            let t = core::arch::x86_64::_rdtsc();
            core::arch::x86_64::_mm_lfence();
            t
        }
    }

    /// One PIT-gated measurement of exactly PIT_COUNT PIT ticks: (TSC ticks, LAPIC ticks).
    unsafe fn measure_once(l: &mut LocalApic) -> (u64, u32) {
        let mut port_b = Port::<u8>::new(PORT_B);
        let mut cmd = Port::<u8>::new(PIT_CMD);
        let mut ch2 = Port::<u8>::new(PIT_CH2);

        unsafe {
            // Mode 0 only counts while GATE2 is high, so hold it low while preparing.
            let b = port_b.read() & !(PORT_B_GATE2 | PORT_B_SPEAKER);
            port_b.write(b);
            cmd.write(PIT_CMD_CH2_LOHI_MODE0);
            ch2.write(PIT_COUNT as u8);
            ch2.write((PIT_COUNT >> 8) as u8);

            l.disable_timer(); // a masked timer still counts
            l.set_timer_mode(TimerMode::OneShot);
            l.set_timer_divide(TIMER_DIVIDE);

            // Start PIT, TSC and LAPIC back to back.
            port_b.write(b | PORT_B_GATE2);
            let t0 = rdtsc_serialized();
            l.set_timer_initial(u32::MAX);

            while port_b.read() & PORT_B_OUT2 == 0 {
                core::hint::spin_loop();
            }

            let t1 = rdtsc_serialized();
            let remaining = l.timer_current();
            l.set_timer_initial(0);
            (t1 - t0, u32::MAX - remaining)
        }
    }

    /// BSP only, once, interrupts off, after the BSP LAPIC is initialised.
    pub unsafe fn calibrate_timers() {
        let mut tsc = [0u64; CAL_TRIALS];
        let mut apic = [0u32; CAL_TRIALS];
        with_lapic(|l| {
            for i in 0..CAL_TRIALS {
                (tsc[i], apic[i]) = unsafe { measure_once(l) };
            }
        });
        tsc.sort_unstable();
        apic.sort_unstable();

        // The interval is exactly PIT_COUNT / PIT_HZ seconds, not the nominal CAL_MS.
        let tsc_hz = tsc[CAL_TRIALS / 2] * PIT_HZ / PIT_COUNT as u64;
        let apic_hz = apic[CAL_TRIALS / 2] as u64 * PIT_HZ / PIT_COUNT as u64;
        assert!(tsc_hz != 0 && apic_hz >= 1000, "timer calibration failed");

        kernel_acpi::TSC_HZ.store(tsc_hz, Ordering::Release);
        LAPIC_TICKS_PER_MS.store((apic_hz / 1000) as u32, Ordering::Release);
    }

    /// Every core, once it can take scheduler ticks.
    pub unsafe fn start_periodic_timer(period_ms: u32) {
        let per_ms = LAPIC_TICKS_PER_MS.load(Ordering::Acquire);
        assert!(per_ms != 0, "LAPIC timer not calibrated");
        let count = u32::try_from(per_ms as u64 * period_ms as u64)
            .expect("tick count exceeds 32 bits; use a larger divider");
        assert!(count != 0);

        with_lapic(|l| unsafe {
            l.set_timer_mode(TimerMode::Periodic);
            l.set_timer_divide(TIMER_DIVIDE); // must match calibration
            l.set_timer_initial(count);
            l.enable_timer();
        });
    }
}

fn drain_i8042() {
    let mut status = Port::<u8>::new(KBD_STATUS);
    let mut data = Port::<u8>::new(KBD_DATA);
    for _ in 0..16 {
        // bounded, in case the controller misbehaves
        if unsafe { status.read() } & KBD_STATUS_OBF == 0 {
            break;
        }
        let _ = unsafe { data.read() };
    }
}
pub(crate) fn init_bsp(config: &InterruptConfig) -> u32 {
    let (bsp_id, ioapic) = unsafe { __init_bsp(config) };
    lapic::LAPIC_ID.with_mut(|id| *id = bsp_id);

    unsafe { timer::calibrate_timers() };

    ioapic.route_isa_irq(1, InterruptIndex::Keyboard as u8, bsp_id, false);
    drain_i8042();
    ioapic::IOAPIC.init_once(|| ioapic);
    bsp_id
}

pub(crate) fn enable_timer() {
    unsafe { timer::start_periodic_timer(TICK_PERIOD_MS) };
}
