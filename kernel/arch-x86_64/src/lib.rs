#![no_std]
#![feature(abi_x86_interrupt)]

use core::fmt::Arguments;

use arch_interfaces::{
    cpu::CpuHandlingCapability,
    init::InitCapability,
    interrupt::InterruptHandlingCapability,
    mm::{
        FrameAllocatorCapability,
        FrameDeAllocatorCapability,
        MemMapperCapability,
        MemUnmapperCapability,
        PageTableError,
        PageTableErrorKind,
        PhysAddr,
        PhysMemoryFrame,
        TLBFlushCapability,
    },
};
use boot::BootInfo;
use spin::{Mutex, lazy::Lazy};
use uart_16550::SerialPort;
use x86_64::{
    instructions::interrupts::{self, without_interrupts},
    registers::control::{Cr4, Cr4Flags},
    structures::paging::{
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
};

use crate::pic::enable_timer;

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
    fn early_init(boot_info: &BootInfo) -> Result<(), arch_interfaces::init::InitError> {
        early_init(boot_info);
        Ok(())
    }

    fn late_init(boot_info: &BootInfo) -> Result<(), arch_interfaces::init::InitError> {
        late_init(boot_info);
        Ok(())
    }
}

impl CpuHandlingCapability for X86_64 {
    fn core_init() {
        todo!()
    }
}

impl FrameAllocatorCapability<{ Size4KiB::SIZE as usize }> for X86_64 {
    fn allocate() -> Option<PhysMemoryFrame<{ Size4KiB::SIZE as usize }>> {
        frame::get_frame_alloc()
            .lock()
            .allocate_frame()
            .map(|frame| {
                PhysMemoryFrame::from_addr(PhysAddr::new(frame.start_address().as_u64() as usize))
            })
            .flatten()
    }
}

impl FrameDeAllocatorCapability<{ Size4KiB::SIZE as usize }> for X86_64 {
    unsafe fn deallocate(frame: PhysMemoryFrame<{ Size4KiB::SIZE as usize }>) -> Result<(), ()> {
        unsafe {
            frame::get_frame_alloc().lock().deallocate_frame(
                PhysFrame::<Size4KiB>::from_start_address(x86_64::PhysAddr::new(
                    frame.start().as_usize() as u64,
                ))
                .map_err(|_| {})?,
            )
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

pub fn early_init(boot_info: &BootInfo) {
    page::init_pagetable(boot_info.hhdm_offset.unwrap());
    frame::init_frame_alloc(boot_info.memmap.unwrap(), boot_info.hhdm_offset.unwrap());
    init_xmm();
}

pub fn late_init(boot_info: &BootInfo) {
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
        .expect("Printing to serial failed")
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
