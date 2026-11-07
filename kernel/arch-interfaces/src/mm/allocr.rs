use crate::mm::{FRAME_4KIB_SIZE, map::PhysMemoryFrame};

pub trait FrameAllocator<const SIZE: usize> {
    fn allocate(&mut self) -> Option<PhysMemoryFrame<SIZE>>;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum FrameDeallocationError {
    #[error("Unknown deallocation error: {}", msg)]
    Unknown { msg: &'static str },
    #[error("The passed frame was invalid")]
    InvalidFrame,
    #[error("The passed frame was already freed")]
    DoubleFree,
}

pub trait FrameDeallocator<const SIZE: usize> {
    /// # Safety
    /// The caller has to ensure that the deallocated frame is not in use anywhere.
    unsafe fn deallocate(
        &mut self,
        frame: PhysMemoryFrame<SIZE>,
    ) -> Result<(), FrameDeallocationError>;
}

pub trait FrameAllocatorCapability<const SIZE: usize = FRAME_4KIB_SIZE> {
    fn allocate() -> Option<PhysMemoryFrame<SIZE>>;
}

pub trait FrameDeallocatorCapability<const SIZE: usize = FRAME_4KIB_SIZE> {
    /// # Safety
    /// The caller has to ensure that the deallocated frame is not in use anywhere.
    unsafe fn deallocate(frame: PhysMemoryFrame<SIZE>) -> Result<(), FrameDeallocationError>;
}
