#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use alloc::{boxed::Box, sync::Arc};
use core::{fmt::Arguments, mem::ManuallyDrop, ptr::NonNull, sync::atomic::AtomicU8};

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
    registers::control::{Cr4, Cr4Flags},
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
    gdt::{DOUBLE_FAULT_IST_INDEX, Selectors},
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

    fn late_init(boot_info: &EarlyBootInfo<'_>) -> Result<(), arch_interfaces::init::InitError> {
        late_init(boot_info);
        Ok(())
    }
}

impl CpuDataCapability for X86_64 {}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CPUCoreState {
    Init,
    Ready,
    Running,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CPUStartupData {
    cpu: &'static BootCpu,
    stack: ManuallyDrop<NonNull<u8>>,
    gdt: GlobalDescriptorTable,
    selectors: Selectors,
    tss: &'static mut TaskStateSegment,
    ready: Arc<AtomicU8>,
    entry: fn(&BootCpu) -> !,
}

const AP_STACK_SIZE: usize = 10 * Size4KiB::SIZE as usize;
const AP_STACK_START: usize = 0xffff_f000_c000_0000;

impl SmpCapability for X86_64 {
    fn ap_init(boot_info: &LateBootInfo, entry: fn(&BootCpu) -> !) {
        // for each ap processor enumerated in boot_info do
        // allocate a stack for that core and set it up properly
        // allocate GDT and TSS for the core and initialize
        // store ptr to global pagetable such that we can then write it immediately on the new ap
        // wait until bsp is ready
        //
        // I.e. we build a CPUStartupData, pass it to the start_cpu() function of the next core, who will then switch stack, install important structures and busy wait.

        // for this we actuallly need to share some atomic flags, gotta think about dataflow here

        let Some(mp_info) = boot_info.mp_info else {
            // TODO should return err such that we know that we are single core. Not fatal, but should be known
            return;
        };

        for (i, cpu) in mp_info.cpus.iter().enumerate() {
            if cpu.is_bsp {
                continue;
            }

            // allocate a stack
            // one guard page between each stack
            let our_base = AP_STACK_START + i * AP_STACK_SIZE + Size4KiB::SIZE as usize;

            let page_range = Page::<Size4KiB>::range_inclusive(
                Page::containing_address(x86_64::VirtAddr::new(our_base as u64)),
                Page::containing_address(x86_64::VirtAddr::new((our_base + AP_STACK_SIZE) as u64)),
            );

            let frame_alloc = frame::get_frame_alloc().lock();
            let pagetable = page::get_pagetable().lock();

            for page in page_range.into_iter() {
                let frame = frame_alloc.allocate_frame().unwrap();
                unsafe {
                    pagetable.map_to(
                        page,
                        frame,
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                        &mut *frame_alloc,
                    )
                }
                .unwrap()
                .flush();
            }

            let mut tss = TaskStateSegment::new();
            tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
                const STACK_SIZE: usize = 4096 * 5;
                static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

                let stack_start = VirtAddr::from_ptr(&raw const STACK);
                stack_start + STACK_SIZE as u64
            };

            let tss = Box::leak(Box::new(tss));

            let mut gdt = GlobalDescriptorTable::new();
            let code_selector = gdt.append(Descriptor::kernel_code_segment());
            let tss_selector = gdt.append(Descriptor::tss_segment(tss));
            let kernel_data_selector = gdt.append(Descriptor::kernel_data_segment());

            let selectors = Selectors {
                code_selector,
                tss_selector,
                data_selector: kernel_data_selector,
            };

            let my_packet = Box::leak(Box::new(CPUStartupData {
                cpu,
                stack: ManuallyDrop::new(NonNull::new(our_base as *mut u8).unwrap()),
                gdt,
                tss,
                selectors,
                ready: Arc::new(AtomicU8::new(0)),
                entry,
            }));

            (mp_info.start_cpu)(
                cpu,
                core_startup_trampoline,
                my_packet as *mut CPUStartupData as usize,
            );
        }

        todo!()
    }

    fn ap_release() {
        todo!()
    }
}

fn core_startup_trampoline(cpu: &BootCpu, arg: usize) -> ! {
    assert_ne!(arg, 0);
    let my_packet = unsafe { &*(arg as *const CPUStartupData) };
    unsafe { core::arch::asm!("mov rsp, {}", my_packet.stack) };
    (my_packet.entry)(cpu)
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
        _frame_deallocator: &mut impl arch_interfaces::mm::Framedeallocator<{ Size4KiB::SIZE as usize }>,
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

#[repr(C, align(128))]
pub struct PerCpu {
    self_ptr: *const PerCpu, // gs:[0]
    pub logical_id: usize,
    pub hw_id: u32,
    pub stack_top: usize,       // 16-byte aligned
    tss: TaskStateSegment,      // own IST stacks (#DF, NMI)
    gdt: GlobalDescriptorTable, // own GDT, one TSS descriptor
    selectors: Selectors,
}

fn ap_entry(_: &BootCpu, arg: usize) -> ! {
    // still on Limine's stack
    let pc = unsafe { &*(arg as *const PerCpu) };
    unsafe { switch_stack(pc.stack_top, ap_main, pc) }
}

unsafe fn switch_stack(top: usize, f: extern "C" fn(&'static PerCpu) -> !, arg: &PerCpu) -> ! {
    unsafe {
        core::arch::asm!(
            "mov rsp, {top}", "xor ebp, ebp", "call {f}", "ud2",
            top = in(reg) top, f = in(reg) f, in("rdi") arg,
            options(noreturn)
        )
    }
}

extern "C" fn ap_main(pc: &'static PerCpu) -> ! {
    apply_control_state(); // EFER.NXE, CR0.WP, CR4, PAT, before the CR3 switch
    unsafe { Cr3::write(kernel_cr3(), flags) };
    pc.load_gdt_and_tss(); // lgdt, reload CS/SS/DS, ltr
    IDT.load(); // one shared IDT is fine
    GsBase::write(VirtAddr::from_ptr(pc));
    lapic::init_this_core(); // x2APIC: SVR (enable + spurious vector), TPR = 0
    assert_eq!(lapic::id(), pc.hw_id);
    AP_ONLINE.fetch_add(1, Ordering::Release);
    serial_println!("CPU {} online", pc.logical_id);
    loop {
        x86_64::instructions::hlt()
    } // interrupts stay disabled for the first milestone
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

pub fn late_init(boot_info: &EarlyBootInfo<'_>) {
    gdt::init();
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
