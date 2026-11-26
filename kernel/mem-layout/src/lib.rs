#![no_std]

// TODO
// sizes, percpu, mmio, virtualization, ...

pub const KERNEL_HEAP_START: usize = 0xffff_c000_0000_0000;
pub const KERNEL_HEAP_SIZE: usize = 10 * 1024 * 1024; // 10 MiB
pub const MAX_KERNEL_HEAP_SIZE: usize = CORE_STACK_START - KERNEL_HEAP_START;

pub const CORE_STACK_SIZE: usize = 20 * 4096; // 80 KiB
pub const CORE_STACK_START: usize = 0xffff_d000_0000_0000;
pub const MAX_CORE_STACKS: usize = (IST_STACK_START - CORE_STACK_START) / (CORE_STACK_SIZE + 4096); // a lot

pub const IST_STACK_SIZE: usize = 5 * 4096; // 20 KiB
pub const IST_STACK_START: usize = 0xffff_e000_0000_0000;
pub const MAX_IST_STACKS: usize = (APPROX_KIMAGE_START - IST_STACK_START) / (IST_STACK_SIZE + 4096); // a lot

pub const MMIO_MAX_SIZE: usize = 100 * 4096;
pub const MMIO_START: usize = 0xffff_f000_0000_0000;

pub const APPROX_KIMAGE_START: usize = 0xffff_ffff_8000_0000;

unsafe extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
    static __cpu_local_start: u8;
    static __cpu_local_end: u8;
}

pub fn kimage_range() -> (usize, usize) {
    let start = unsafe { &__kernel_start as *const _ as usize };
    let end = unsafe { &__kernel_end as *const _ as usize };
    (start, end)
}

pub fn cpu_local_range() -> (usize, usize) {
    let start = unsafe { &__cpu_local_start as *const _ as usize };
    let end = unsafe { &__cpu_local_end as *const _ as usize };
    (start, end)
}
