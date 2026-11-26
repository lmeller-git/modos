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
pub const MAX_IST_STACKS: usize = (MMIO_START - IST_STACK_START) / (IST_STACK_SIZE + 4096); // a lot

pub const MMIO_MAX_SIZE: usize = 100 * 4096;
pub const MMIO_START: usize = 0xffff_f000_0000_0000;

pub const APPROX_KIMAGE_START: usize = 0xffff_ffff_8000_0000;

const _: () = {
    const PAGE: usize = 4096;
    const HIGH_HALF: usize = 0xffff_8000_0000_0000;

    const REGIONS: [(usize, usize); 5] = [
        (KERNEL_HEAP_START, MAX_KERNEL_HEAP_SIZE),
        (CORE_STACK_START, MAX_CORE_STACKS * (CORE_STACK_SIZE + PAGE)),
        (IST_STACK_START, MAX_IST_STACKS * (IST_STACK_SIZE + PAGE)),
        (MMIO_START, MMIO_MAX_SIZE),
        (APPROX_KIMAGE_START, 0),
    ];

    let mut i = 0;
    while i < REGIONS.len() {
        let (start, size) = REGIONS[i];
        assert!(start >= HIGH_HALF && start.is_multiple_of(PAGE) && size.is_multiple_of(PAGE));
        if i + 1 < REGIONS.len() {
            assert!(
                start + size <= REGIONS[i + 1].0,
                "regions overlap or are unordered"
            );
        }
        i += 1;
    }

    assert!(KERNEL_HEAP_SIZE > 0 && KERNEL_HEAP_SIZE.is_multiple_of(PAGE));
    assert!(KERNEL_HEAP_SIZE <= MAX_KERNEL_HEAP_SIZE);
    assert!(CORE_STACK_SIZE.is_multiple_of(PAGE) && IST_STACK_SIZE.is_multiple_of(PAGE));
    assert!(MAX_CORE_STACKS > 0 && MAX_IST_STACKS > 0 && MMIO_MAX_SIZE > 0);
};

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
