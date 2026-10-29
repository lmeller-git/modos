use crate::mm::{FRAME_4KIB_SIZE, map::PhysMemoryFrame};

pub trait FrameAllocator<const SIZE: usize> {
    fn allocate(&mut self) -> Option<PhysMemoryFrame<SIZE>>;
}

pub trait FrameDeallocator<const SIZE: usize> {
    unsafe fn deallocate(&mut self, frame: PhysMemoryFrame<SIZE>) -> Result<(), ()>;
}

pub trait FrameAllocatorCapability<const SIZE: usize = FRAME_4KIB_SIZE> {
    fn allocate() -> Option<PhysMemoryFrame<SIZE>>;
}

pub trait FrameDeAllocatorCapability<const SIZE: usize = FRAME_4KIB_SIZE> {
    unsafe fn deallocate(frame: PhysMemoryFrame<SIZE>) -> Result<(), ()>;
}
