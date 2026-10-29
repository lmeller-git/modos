use arch::interface::mm::VirtAddr;

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy)]
    #[repr(transparent)]
    pub struct PageFaultFlags: u32 {
        const PRESENT     = 1 << 0;
        const WRITE       = 1 << 1;
        const USER        = 1 << 2;
        const INSTRUCTION = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PageFaultInfo {
    pub fault_address: VirtAddr,
    pub instruction_pointer: VirtAddr,
    pub flags: PageFaultFlags,
}
