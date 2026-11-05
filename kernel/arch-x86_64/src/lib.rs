#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{
    fmt::Arguments,
    hint::spin_loop,
    mem::ManuallyDrop,
    ptr::{NonNull, null},
    sync::atomic::{AtomicU8, Ordering},
};

use arch_interfaces::{
    cpu::{CpuDataCapability, SmpCapability},
    init::InitCapability,
    interrupt::InterruptHandlingCapability,
    mm::{
        FrameAllocatorCapability,
        FrameDeallocatorCapability,
        MemMapperCapability,
        MemUnmapperCapability,
        PageTableError,
        PageTableErrorKind,
        PhysAddr,
        PhysMemoryFrame,
        TLBFlushCapability,
    },
};
use boot::{EarlyBootInfo, LateBootInfo, mp::BootCpu};
use spin::{Mutex, lazy::Lazy};
use uart_16550::SerialPort;
use x86_64::{
    instructions::interrupts::{self, without_interrupts},
    registers::{
        control::{Cr3, Cr3Flags, Cr4, Cr4Flags},
        model_specific::GsBase,
    },
    structures::{
        gdt::GlobalDescriptorTable,
        paging::{
            FrameAllocator,
            FrameDeallocator,
            Mapper,
            Page,
            PageSize,
            PageTableFlags,
            PhysFrame,
            Size4KiB,
            mapper::MapperFlush,
        },
        tss::TaskStateSegment,
    },
};

use crate::{
    frame::X86FrameAllocView,
    gdt::{Selectors, build_gdt, build_tss, load_gdt_tss},
    pic::enable_timer,
};

pub mod frame;
mod gdt;
mod idt;
mod irq;
pub mod page;
mod pic;
mod qemu;

/// Capability provider for x86_64 targets
pub struct X86_64;

impl InitCapability for X86_64 {
    fn early_init(boot_info: &EarlyBootInfo<'_>) -> Result<(), arch_interfaces::init::InitError> {
        early_init(boot_info);
        Ok(())
    }

    fn late_init(boot_info: &LateBootInfo) -> Result<(), arch_interfaces::init::InitError> {
        late_init(boot_info);
        Ok(())
    }
}

impl CpuDataCapability for X86_64 {
    fn core_init(
        _boot_info: &LateBootInfo,
        _core_id: usize,
    ) -> Option<arch_interfaces::cpu::CpuId> {
        None
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CPUCoreState {
    Init,
    Ready,
    Running,
}

#[repr(C)]
struct CPUData {
    self_ptr: *const CPUData,
    cpu: BootCpu,
    stack: ManuallyDrop<NonNull<u8>>,
    gdt: GlobalDescriptorTable,
    selectors: Selectors,
    tss: &'static TaskStateSegment,
}

#[repr(C)]
struct CPUStartupData {
    data: &'static mut CPUData,
    entry: fn(BootCpu) -> !,
    cr3: (PhysFrame<Size4KiB>, Cr3Flags),
    state: Arc<AtomicU8>,
}

#[derive(Clone, Debug)]
pub struct APBringupSync {
    cores: Vec<Arc<AtomicU8>>,
}

const AP_STACK_SIZE: usize = 10 * Size4KiB::SIZE as usize;
const AP_STACK_START: usize = 0xffff_f000_c000_0000;

impl SmpCapability for X86_64 {
    type Communicator = APBringupSync;

    fn comm_init(boot_info: &LateBootInfo) -> Self::Communicator {
        APBringupSync {
            cores: boot_info
                .mp_info
                .as_ref()
                .map(|mp_dat| {
                    mp_dat
                        .cpus
                        .iter()
                        .filter_map(|cpu| {
                            (!cpu.is_bsp)
                                .then_some(Arc::new(AtomicU8::new(CPUCoreState::Init as u8)))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn ap_init(boot_info: &LateBootInfo, entry: fn(BootCpu) -> !, comm: Self::Communicator) {
        // for each ap processor enumerated in boot_info do
        // allocate a stack for that core and set it up properly
        // allocate GDT and TSS for the core and initialize
        // store ptr to global pagetable such that we can then write it immediately on the new ap
        // wait until bsp is ready
        //
        // I.e. we build a CPUStartupData, pass it to the start_cpu() function of the next core, who will then switch stack, install important structures and busy wait.

        // for this we actuallly need to share some atomic flags, gotta think about dataflow here

        let Some(mp_info) = &boot_info.mp_info else {
            serial_println!("No smp");
            // TODO should return err such that we know that we are single core. Not fatal, but should be known
            return;
        };

        serial_println!("cores: {}", mp_info.cpus.len());

        let cr3 = Cr3::read();

        for (_i, (cpu, state)) in mp_info
            .cpus
            .iter()
            .filter(|core| !core.is_bsp)
            .zip(comm.cores.iter())
            .enumerate()
        {
            // allocate a stack
            // one guard page between each stack
            // layout is
            // START_ADDRESS
            // ....
            // STACK_SIZE
            // ....
            // GUARD_PAGE
            // NEXT
            // ....
            let our_base =
                AP_STACK_START + cpu.logical_id * (AP_STACK_SIZE + Size4KiB::SIZE as usize);

            let page_range = Page::<Size4KiB>::range(
                Page::containing_address(x86_64::VirtAddr::new(our_base as u64)),
                Page::containing_address(x86_64::VirtAddr::new((our_base + AP_STACK_SIZE) as u64)),
            );

            let mut frame_alloc = frame::get_frame_alloc().lock();
            let mut pagetable = page::get_pagetable().lock();

            for page in page_range {
                let frame = frame_alloc.allocate_frame().unwrap();
                unsafe {
                    pagetable.map_to(
                        page,
                        frame,
                        PageTableFlags::PRESENT
                            | PageTableFlags::WRITABLE
                            | PageTableFlags::NO_EXECUTE,
                        &mut *frame_alloc,
                    )
                }
                .unwrap()
                .flush();
            }

            drop(frame_alloc);
            drop(pagetable);

            let stack = our_base + AP_STACK_SIZE;

            let tss = build_tss(cpu.logical_id);
            let tss = Box::leak(Box::new(tss));

            let (gdt, selectors) = build_gdt(tss);

            let data = CPUData {
                self_ptr: null(),
                cpu: *cpu,
                stack: ManuallyDrop::new(NonNull::new(stack as *mut u8).unwrap()),
                gdt,
                tss,
                selectors,
            };
            let data = Box::leak(Box::new(data));
            data.self_ptr = data as *const CPUData;

            let my_packet = Box::leak(Box::new(CPUStartupData {
                data,
                entry,
                cr3,
                state: state.clone(),
            }));

            (mp_info.start_cpu)(
                cpu,
                core_startup_trampoline,
                my_packet as *mut CPUStartupData as usize,
            );
        }
    }

    fn ap_release(comm: Self::Communicator) {
        for core in comm.cores.iter() {
            while core.load(Ordering::Acquire) != CPUCoreState::Ready as u8 {
                spin_loop();
            }
        }

        for core in comm.cores {
            core.store(CPUCoreState::Running as u8, Ordering::Release);
        }
    }
}

fn core_startup_trampoline(cpu: &BootCpu, arg: usize) -> ! {
    assert_ne!(arg, 0);
    let my_packet = unsafe { &*(arg as *const CPUStartupData) };

    unsafe {
        Cr3::write(my_packet.cr3.0, my_packet.cr3.1);

        core::arch::asm!(
            "mov rsp, {top}", "xor ebp, ebp", "call {next}", "ud2",
            top = in(reg) my_packet.data.stack.as_ptr(),
            next = sym ap_main, in("rdi") cpu, in("rsi") my_packet,
            options(noreturn),
        );
    }
}

extern "C" fn ap_main(cpu: &BootCpu, my_packet: &'static CPUStartupData) -> ! {
    load_gdt_tss(&my_packet.data.gdt, &my_packet.data.selectors);

    idt::IDT.get().unwrap().load();

    let my_data_ptr = x86_64::VirtAddr::from_ptr(my_packet.data as *const CPUData);
    GsBase::write(my_data_ptr);

    my_packet
        .state
        .store(CPUCoreState::Ready as u8, Ordering::Release);

    while my_packet.state.load(Ordering::Acquire) != CPUCoreState::Running as u8 {
        spin_loop();
    }

    (my_packet.entry)(*cpu)
}

impl FrameAllocatorCapability<{ Size4KiB::SIZE as usize }> for X86_64 {
    fn allocate() -> Option<PhysMemoryFrame<{ Size4KiB::SIZE as usize }>> {
        frame::get_frame_alloc()
            .lock()
            .allocate_frame()
            .and_then(|frame| {
                PhysMemoryFrame::from_addr(PhysAddr::new(frame.start_address().as_u64() as usize))
            })
    }
}

impl FrameDeallocatorCapability<{ Size4KiB::SIZE as usize }> for X86_64 {
    unsafe fn deallocate(frame: PhysMemoryFrame<{ Size4KiB::SIZE as usize }>) -> Result<(), ()> {
        unsafe {
            frame::get_frame_alloc().lock().deallocate_frame(
                PhysFrame::<Size4KiB>::from_start_address(x86_64::PhysAddr::new(
                    frame.start().as_usize() as u64,
                ))
                .map_err(|_| {})?,
            );
        };
        Ok(())
    }
}

struct FrameAllocView<'a, T>(&'a mut T);

unsafe impl<'a, T> FrameAllocator<Size4KiB> for FrameAllocView<'a, T>
where
    T: arch_interfaces::mm::FrameAllocator<{ Size4KiB::SIZE as usize }>,
{
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.0.allocate().map(|frame| {
            PhysFrame::from_start_address(x86_64::PhysAddr::new(frame.start().as_usize() as u64))
                .unwrap()
        })
    }
}

impl MemMapperCapability<{ Size4KiB::SIZE as usize }> for X86_64 {
    unsafe fn map_to(
        page: arch_interfaces::mm::VirtMemoryFrame<{ Size4KiB::SIZE as usize }>,
        frame: PhysMemoryFrame<{ Size4KiB::SIZE as usize }>,
        flags: arch_interfaces::mm::PageFlags,
        frame_allocator: &mut impl arch_interfaces::mm::FrameAllocator<{ Size4KiB::SIZE as usize }>,
    ) -> Result<Self::Flush, PageTableError> {
        unsafe {
            page::get_pagetable()
                .lock()
                .map_to(
                    Page::from_start_address(x86_64::VirtAddr::new(page.start().as_usize() as u64))
                        .unwrap(),
                    PhysFrame::<Size4KiB>::from_start_address(x86_64::PhysAddr::new(
                        frame.start().as_usize() as u64,
                    ))
                    .unwrap(),
                    PageTableFlags::from_bits(flags.bits()).unwrap(),
                    &mut FrameAllocView(frame_allocator),
                )
                .map_err(|_| PageTableError::Simple(PageTableErrorKind::None))
        }
    }

    unsafe fn map_to_global(
        page: arch_interfaces::mm::VirtMemoryFrame<{ Size4KiB::SIZE as usize }>,
        frame: PhysMemoryFrame<{ Size4KiB::SIZE as usize }>,
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

impl MemUnmapperCapability<{ Size4KiB::SIZE as usize }> for X86_64 {
    fn unmap(
        page: arch_interfaces::mm::VirtMemoryFrame<{ Size4KiB::SIZE as usize }>,
        _frame_deallocator: &mut impl arch_interfaces::mm::FrameDeallocator<{ Size4KiB::SIZE as usize }>,
    ) -> Result<(PhysMemoryFrame<{ Size4KiB::SIZE as usize }>, Self::Flush), PageTableError> {
        page::get_pagetable()
            .lock()
            .unmap(
                Page::<Size4KiB>::from_start_address(x86_64::VirtAddr::new(
                    page.start().as_usize() as u64,
                ))
                .unwrap(),
            )
            .map(|(frame, flush)| {
                (
                    PhysMemoryFrame::from_addr(PhysAddr::new(
                        frame.start_address().as_u64() as usize
                    ))
                    .unwrap(),
                    flush,
                )
            })
            .map_err(|_| PageTableError::Simple(PageTableErrorKind::None))
    }
}

impl TLBFlushCapability for X86_64 {
    type Flush = MapperFlush<Size4KiB>;

    fn flush(flush: Self::Flush) -> Result<(), ()> {
        flush.flush();
        Ok(())
    }
}

impl InterruptHandlingCapability for X86_64 {
    fn enable() {
        interrupts::enable();
    }

    fn disable() {
        interrupts::disable();
    }

    fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
        without_interrupts(f)
    }
}

static SERIAL: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    port.into()
});

pub fn exit(code: usize) -> ! {
    qemu::exit_qemu(qemu::QemuExitCode::from_usize(code));
}

pub fn early_init(boot_info: &EarlyBootInfo<'_>) {
    page::init_pagetable(boot_info.hhdm_offset.unwrap());
    frame::init_frame_alloc(boot_info.memmap.unwrap(), boot_info.hhdm_offset.unwrap());
    init_xmm();
}

pub fn late_init(boot_info: &LateBootInfo) {
    let my_cpu = boot_info
        .mp_info
        .as_ref()
        .map(|info| {
            info.cpus
                .iter()
                .find(|cpu| cpu.is_bsp)
                .and_then(|cpu| Some(cpu))
        })
        .flatten()
        .unwrap_or(&BootCpu {
            logical_id: 0,
            hw_id: 0,
            is_bsp: true,
        });

    let tss = build_tss(my_cpu.logical_id);
    let tss = Box::leak(Box::new(tss));
    let (gdt, selectors) = build_gdt(tss);

    let my_data = CPUData {
        self_ptr: null(),
        cpu: *my_cpu,
        stack: ManuallyDrop::new(NonNull::dangling()), // TODO switch stack?
        gdt,
        selectors,
        tss,
    };

    let my_data = Box::leak(Box::new(my_data));
    my_data.self_ptr = my_data as *const CPUData;

    let my_data_ptr = x86_64::VirtAddr::from_ptr(my_data as *mut CPUData);

    GsBase::write(my_data_ptr);
    load_gdt_tss(&my_data.gdt, &my_data.selectors);

    idt::init_idt();
    pic::init_apic(
        boot_info.hhdm_offset.unwrap(),
        boot_info.rsdp_addr.unwrap().as_ptr() as usize,
    );

    interrupts::enable();
    enable_timer();
}

fn init_xmm() {
    unsafe {
        Cr4::update(|cr4| {
            cr4.insert(Cr4Flags::OSFXSR);
            cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
        });
    }
}

pub fn _serial_print(args: Arguments<'_>) {
    use core::fmt::Write;
    SERIAL
        .lock()
        .write_fmt(args)
        .expect("Printing to serial failed");
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial_print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::serial_print!("{}\n", format_args!($($arg)*))
    };
}
#[macro_export]
macro_rules! serial_print {
    () => {};
    ($($arg:tt)*) => {
        $crate::_serial_print(format_args!("\x1b[34m[KINFO]\x1b[0m {}", format_args!($($arg)*)))
    };
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
