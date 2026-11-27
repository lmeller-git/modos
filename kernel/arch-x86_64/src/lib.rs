#![no_std]
#![feature(abi_x86_interrupt, const_range, const_trait_impl)]

extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{
    hint::spin_loop,
    mem::ManuallyDrop,
    ptr::{self, NonNull, null},
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
};

use arch_interfaces::{
    cpu::{CpuDataCapability, SmpCapability},
    init::InitCapability,
    interrupt::InterruptHandlerCapability,
    mm::{
        FlushError,
        FrameAllocatorCapability,
        FrameDeallocationError,
        FrameDeallocatorCapability,
        MemMapperCapability,
        MemUnmapperCapability,
        PageTableError,
        PageTableErrorKind,
        TLBFlushCapability,
    },
};
use arch_x86_64_primitives::{CPUData, Primitive};
use boot::{EarlyBootInfo, LateBootInfo, memmap::RegionKind, mp::BootCpu};
use conquer_once::spin::OnceCell;
use core_local::allocate_cls_block;
use early_log::early_println;
use kernel_types::{
    cpu::CpuId,
    frame::{FRAME_4KIB_SIZE, PhysMemoryFrame, VirtMemoryFrame},
};
use platform::TargetPlatform;
use x86_64::{
    instructions::interrupts::{self, without_interrupts},
    registers::{
        control::{Cr0, Cr3, Cr4, Cr4Flags},
        model_specific::{GsBase, Msr},
    },
    structures::paging::{
        FrameAllocator,
        FrameDeallocator,
        Mapper,
        PageTableFlags,
        PhysFrame,
        Size4KiB,
        mapper::MapperFlush,
    },
};

use crate::{
    frame::X86FrameAllocView,
    gdt::{build_gdt, build_tss, load_gdt_tss},
};

mod apic;
mod conv;
pub(crate) mod frame;
mod gdt;
mod idt;
mod irq;
pub(crate) mod mmio;
pub(crate) mod page;
mod stack;

/// Capability provider for x86_64 targets
pub struct X86_64;

pub struct X86_64Init(());

impl InitCapability for X86_64 {
    type InitToken = X86_64Init;

    fn initialize() -> Option<Self::InitToken> {
        static SINGLETON: AtomicBool = AtomicBool::new(false);
        if SINGLETON.swap(true, Ordering::AcqRel) {
            None
        } else {
            Some(X86_64Init(()))
        }
    }

    fn early_init(
        _token: &Self::InitToken,
        boot_info: &EarlyBootInfo<'_>,
    ) -> Result<(), arch_interfaces::init::InitError> {
        early_init(boot_info);
        Ok(())
    }

    fn late_init(
        _token: &Self::InitToken,
        boot_info: &LateBootInfo,
    ) -> Result<(), arch_interfaces::init::InitError> {
        late_init(boot_info);
        Ok(())
    }
}

impl CpuDataCapability for X86_64 {
    fn cpu_count() -> usize {
        Primitive::cpu_count()
    }

    fn my_cpu() -> BootCpu {
        Primitive::my_cpu()
    }

    fn public_data_base() -> *const () {
        Primitive::public_data_base()
    }

    fn public_data_base_of(cpu: CpuId) -> Option<*const ()> {
        Primitive::public_data_base_of(cpu)
    }

    fn cpus() -> impl Iterator<Item = CpuId> {
        Primitive::cpus()
    }
}

#[repr(C)]
struct CPUStartupData {
    data: &'static mut CPUData,
    entry: fn() -> !,
    cs: ControlState,
    state: Arc<AtomicBool>,
    gate: Arc<AtomicBool>,
}

struct BSPReleaseData {
    entry: fn() -> !,
    comm: APBringupSync,
    my_cpu: &'static CPUData,
    boot_info: LateBootInfo,
}

#[derive(Clone, Debug)]
struct APBringupSync {
    cores: Vec<Arc<AtomicBool>>,
    gate: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ControlState {
    efer: u64,
    cr0: u64,
    cr4: u64,
    pat: u64,
    cr3: (PhysFrame<Size4KiB>, u16),
}

fn capture_cs() -> ControlState {
    ControlState {
        efer: unsafe { Msr::new(0xC000_0080).read() },
        cr0: Cr0::read_raw(),
        cr4: Cr4::read_raw(),
        pat: unsafe { Msr::new(0x277).read() },
        cr3: Cr3::read_raw(),
    }
}

unsafe fn apply_cs(s: &ControlState) {
    unsafe {
        Msr::new(0xC000_0080).write(s.efer);
        Cr0::write_raw(s.cr0);
        Cr4::write_raw(s.cr4);
        Msr::new(0x277).write(s.pat);
        Cr3::write_raw(s.cr3.0, s.cr3.1);
    }
}

fn comm_init(boot_info: &LateBootInfo) -> APBringupSync {
    APBringupSync {
        cores: boot_info
            .mp_info
            .as_ref()
            .map(|mp_dat| {
                mp_dat
                    .cpus
                    .iter()
                    .filter_map(|cpu| (!cpu.is_bsp).then_some(Arc::new(AtomicBool::new(false))))
                    .collect()
            })
            .unwrap_or_default(),
        gate: Arc::new(AtomicBool::new(false)),
    }
}

fn build_core_stack(logical_id: usize) -> NonNull<u8> {
    NonNull::new(stack::CORE_STACKS.map(logical_id).as_mut_ptr()).unwrap()
}

impl SmpCapability for X86_64 {
    type InitToken = X86_64Init;

    fn core_release(_token: Self::InitToken, boot_info: LateBootInfo, entry: fn() -> !) -> ! {
        // for each ap processor enumerated in boot_info do
        // allocate a stack for that core and set it up properly
        // allocate GDT and TSS for the core and initialize
        // store ptr to global pagetable such that we can then write it immediately on the new ap
        // wait until bsp is ready
        //
        // I.e. we build a CPUStartupData, pass it to the start_cpu() function of the next core, who will then switch stack, install important structures and busy wait.

        let comm = comm_init(&boot_info);
        let bsp_cpu: &'static CPUData = unsafe { &*GsBase::read().as_ptr() };

        let Some(mp_info) = &boot_info.mp_info else {
            early_println!("No smp");

            let packet = Box::leak(Box::new(BSPReleaseData {
                entry,
                comm,
                my_cpu: bsp_cpu,
                boot_info,
            }));

            bsp_release(packet)
        };

        early_println!("cores: {}", mp_info.cpus.len());

        let cs = capture_cs();

        for (cpu, state) in mp_info
            .cpus
            .iter()
            .filter(|core| !core.is_bsp)
            .zip(comm.cores.iter())
        {
            let data = build_cpu(cpu, bsp_cpu.cls_table);

            let my_packet = Box::leak(Box::new(CPUStartupData {
                data,
                entry,
                cs,
                state: state.clone(),
                gate: comm.gate.clone(),
            }));

            (mp_info.start_cpu)(
                cpu,
                core_startup_trampoline,
                my_packet as *mut CPUStartupData as usize,
            );
        }

        let packet = Box::leak(Box::new(BSPReleaseData {
            entry,
            comm,
            my_cpu: bsp_cpu,
            boot_info,
        }));

        bsp_release(packet)
    }
}

fn bsp_release(packet: &'static mut BSPReleaseData) -> ! {
    unsafe {
        core::arch::asm!(
            "mov rsp, {top}", "xor ebp, ebp", "call {next}", "ud2",
            top = in(reg) packet.my_cpu.stack.as_ptr(),
            next = sym bsp_main, in("rdi") packet,
            options(noreturn),
        );
    }
}

fn core_startup_trampoline(arg: usize) -> ! {
    assert_ne!(arg, 0);
    let my_packet = unsafe { &mut *(arg as *mut CPUStartupData) };

    unsafe {
        apply_cs(&my_packet.cs);

        core::arch::asm!(
            "mov rsp, {top}", "xor ebp, ebp", "call {next}", "ud2",
            top = in(reg) my_packet.data.stack.as_ptr(),
            next = sym ap_main, in("rdi") my_packet,
            options(noreturn),
        );
    }
}

static BOOTDATA: OnceCell<LateBootInfo> = OnceCell::uninit();

extern "C" fn bsp_main(packet: &'static mut BSPReleaseData) -> ! {
    let BSPReleaseData {
        entry,
        comm,
        my_cpu: _,
        boot_info,
    } = *unsafe { Box::from_raw(packet as *mut BSPReleaseData) };
    for core in comm.cores.iter() {
        while !core.load(Ordering::Acquire) {
            // TODO: bounded wait per core and one second pass, then note which cores never came up
            spin_loop();
        }
    }
    // At this point we know that all cores including ours have done their init and are on their own stacks, ready for handoff

    // we can reclaim bootloader memory now
    if let Some(memmap) = boot_info.memmap.as_ref() {
        let mut frame_alloc = frame::FRAME_ALLOC.get().unwrap().lock();
        for region in memmap
            .iter()
            .filter(|region| region.kind() == RegionKind::BootloaderReclaimable)
        {
            frame_alloc.add_region(region.frames());
        }
    }

    BOOTDATA.init_once(|| boot_info);

    comm.gate.store(true, Ordering::Release);
    drop(comm);

    (entry)()
}

extern "C" fn ap_main(my_packet: &'static mut CPUStartupData) -> ! {
    let CPUStartupData {
        data,
        entry,
        cs: _,
        state,
        gate,
    } = *unsafe { Box::from_raw(my_packet as *mut CPUStartupData) };

    load_gdt_tss(&data.gdt, &data.selectors);

    let my_data_ptr = x86_64::VirtAddr::from_ptr(data as *const CPUData);
    GsBase::write(my_data_ptr);
    data.publish_cls();

    irq::init();
    idt::IDT.get().unwrap().load();

    apic::init_ap();

    state.store(true, Ordering::Release);
    while !gate.load(Ordering::Acquire) {
        spin_loop();
    }

    drop(state);
    drop(gate);

    interrupts::enable();
    apic::enable_timer();

    (entry)()
}

impl FrameAllocatorCapability<FRAME_4KIB_SIZE> for X86_64 {
    fn allocate() -> Option<PhysMemoryFrame> {
        frame::get_frame_alloc()
            .lock()
            .allocate_frame()
            .map(conv::from_x86_frame)
    }
}

impl FrameDeallocatorCapability<FRAME_4KIB_SIZE> for X86_64 {
    unsafe fn deallocate(frame: PhysMemoryFrame) -> Result<(), FrameDeallocationError> {
        unsafe {
            frame::get_frame_alloc()
                .lock()
                .deallocate_frame(conv::to_x86_frame(frame));
        };
        Ok(())
    }
}

struct FrameAllocView<'a, T>(&'a mut T);

unsafe impl<'a, T> FrameAllocator<Size4KiB> for FrameAllocView<'a, T>
where
    T: arch_interfaces::mm::FrameAllocator<FRAME_4KIB_SIZE>,
{
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.0.allocate().map(conv::to_x86_frame)
    }
}

impl MemMapperCapability<FRAME_4KIB_SIZE> for X86_64 {
    unsafe fn map_to(
        page: VirtMemoryFrame,
        frame: PhysMemoryFrame,
        flags: arch_interfaces::mm::PageFlags,
        frame_allocator: &mut impl arch_interfaces::mm::FrameAllocator<FRAME_4KIB_SIZE>,
    ) -> Result<Self::Flush, PageTableError> {
        unsafe {
            page::get_pagetable()
                .lock()
                .map_to(
                    conv::to_x86_page(page),
                    conv::to_x86_frame(frame),
                    page_table_flags(flags),
                    &mut FrameAllocView(frame_allocator),
                )
                .map_err(|_| PageTableError::Simple(PageTableErrorKind::None))
        }
    }

    unsafe fn map_to_global(
        page: VirtMemoryFrame,
        frame: PhysMemoryFrame,
        flags: arch_interfaces::mm::PageFlags,
    ) -> Result<Self::Flush, PageTableError> {
        unsafe {
            Self::map_to(
                page,
                frame,
                flags,
                &mut X86FrameAllocView(&mut *frame::get_frame_alloc().lock()),
            )
        }
    }
}

impl MemUnmapperCapability<FRAME_4KIB_SIZE> for X86_64 {
    fn unmap(
        page: VirtMemoryFrame,
        _frame_deallocator: &mut impl arch_interfaces::mm::FrameDeallocator<FRAME_4KIB_SIZE>,
    ) -> Result<(PhysMemoryFrame, Self::Flush), PageTableError> {
        page::get_pagetable()
            .lock()
            .unmap(conv::to_x86_page(page))
            .map(|(frame, flush)| (conv::from_x86_frame(frame), flush))
            .map_err(|_| PageTableError::Simple(PageTableErrorKind::None))
    }
}

impl TLBFlushCapability for X86_64 {
    type Flush = MapperFlush<Size4KiB>;

    fn flush(flush: Self::Flush) -> Result<(), FlushError> {
        flush.flush();
        Ok(())
    }
}

impl InterruptHandlerCapability for X86_64 {
    fn enable() {
        interrupts::enable();
    }

    fn disable() {
        interrupts::disable();
    }

    fn is_enabled() -> bool {
        interrupts::are_enabled()
    }

    fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
        without_interrupts(f)
    }
}

fn early_init(boot_info: &EarlyBootInfo<'_>) {
    let hhdm = boot_info.hhdm.unwrap();
    page::init_pagetable(hhdm);
    frame::init_frame_alloc(&boot_info.memmap.unwrap(), hhdm);
    init_xmm();
}

fn build_cpu(cpu: &BootCpu, cls_table: &'static [AtomicPtr<u8>]) -> &'static mut CPUData {
    let bsp_stack = build_core_stack(cpu.logical_id.value());
    let tss = build_tss(cpu.logical_id.value());
    let tss = Box::leak(Box::new(tss));
    let (gdt, selectors) = build_gdt(tss);
    let cls = allocate_cls_block();

    let my_data = CPUData {
        self_ptr: null(),
        cpu: *cpu,
        stack: ManuallyDrop::new(bsp_stack),
        gdt,
        selectors,
        tss,
        cls,
        cls_table,
    };

    let my_data = Box::leak(Box::new(my_data));
    my_data.self_ptr = my_data as *const CPUData;
    my_data
}

fn late_init(boot_info: &LateBootInfo) {
    let fallback_bsp = BootCpu {
        logical_id: CpuId::new(0),
        hw_id: apic::cpuid::initial_apic_id() as _,
        is_bsp: true,
    };
    let my_cpu = boot_info
        .mp_info
        .as_ref()
        .map_or(&fallback_bsp, |info| info.cpus.bsp());

    let cpu_count = boot_info.mp_info.as_ref().map_or(1, |info| info.cpus.len());

    let cls_table: &'static [AtomicPtr<u8>] = Box::leak(
        (0..cpu_count)
            .map(|_| AtomicPtr::new(ptr::null_mut()))
            .collect::<Box<_>>(),
    );

    let my_data = build_cpu(my_cpu, cls_table);
    let my_data_ptr = x86_64::VirtAddr::from_ptr(my_data as *mut CPUData);

    GsBase::write(my_data_ptr);
    my_data.publish_cls();
    load_gdt_tss(&my_data.gdt, &my_data.selectors);

    irq::init();
    idt::init_idt();

    apic::timer::init_clock(boot_info.tsc_hz);

    let hhdm = boot_info.hhdm.unwrap();
    let rsdp = boot_info.rsdp.unwrap();

    let config = unsafe { TargetPlatform::parse_config(hhdm, rsdp, apic::timer::nanos_since_boot) }
        .expect("failed to discover the platform interrupt topology");

    let bsp_id = apic::init_bsp(config.lapic_phys, &config.lapic_nmi);
    if bsp_id != my_cpu.hw_id {
        early_println!(
            "[WARN] BSP APIC ID {} != bootloader hw_id {}",
            bsp_id,
            my_cpu.hw_id
        );
    }

    let cpu_map: Vec<(CpuId, u32)> = boot_info
        .mp_info
        .as_ref()
        .map(|info| {
            info.cpus
                .iter()
                .map(|cpu| (cpu.logical_id, cpu.hw_id))
                .collect()
        })
        .unwrap_or_else(|| Vec::from([(my_cpu.logical_id, my_cpu.hw_id)]));

    unsafe {
        TargetPlatform::init_interrupt_controllers(&config, bsp_id, &cpu_map, mmio::map_mmio);
    }

    interrupts::enable();
    apic::enable_timer();
}

fn init_xmm() {
    unsafe {
        Cr4::update(|cr4| {
            cr4.insert(Cr4Flags::OSFXSR);
            cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
        });
    }
}

/// Converts to the x86_64 flags
fn page_table_flags(flags: arch_interfaces::mm::PageFlags) -> PageTableFlags {
    PageTableFlags::from_bits(flags.bits()).expect("PageFlags are a subset of the hardware flags")
}

#[cfg(test)]
mod tests {
    use arch_interfaces::mm::PageFlags;

    use super::*;

    /// Every flag we define (including the OS bits) converts to the same hardware bits.
    #[test]
    fn page_flags_convert() {
        assert_eq!(
            page_table_flags(PageFlags::all()).bits(),
            PageFlags::all().bits()
        );
    }
}
