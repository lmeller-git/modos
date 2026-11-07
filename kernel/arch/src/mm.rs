use arch_interfaces::mm::{FRAME_4KIB_SIZE, PhysMemoryFrame};

pub trait FrameAllocator<const SIZE: usize> {
    /// Allocates a currently free frame of physical memory, if a frame of the requested size is available.
    fn allocate(&mut self) -> Option<PhysMemoryFrame<SIZE>>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameDeallocationError {
    #[error("The passed frame is invalid.")]
    InvalidFrame,
    #[error("The passed frame was already freed")]
    DoubleFree,
    #[error("Unknown deallocation error {}", msg)]
    Other { msg: &'static str },
}

pub trait FrameDeallocator<const SIZE: usize> {
    /// Frees a frame of physical memory, making it available for future [`FrameAllocator::allocate`]'s.
    /// Returns an Err if the frame was not freed.
    /// # Safety
    /// The callerr must guarantee that the memory frame that is freed here is not used anymore.
    /// I.e. no references to that memory are allowed to exist at this point.
    unsafe fn deallocate(
        &mut self,
        frame: PhysMemoryFrame<SIZE>,
    ) -> Result<(), FrameDeallocationError>;
}

impl<I, const SIZE: usize> FrameAllocator<SIZE> for I
where
    I: Iterator<Item = PhysMemoryFrame<SIZE>>,
{
    fn allocate(&mut self) -> Option<PhysMemoryFrame<SIZE>> {
        self.next()
    }
}

pub struct DirectCollect<'a, const FRAME_SIZE: usize = FRAME_4KIB_SIZE> {
    // TODO: to support dynamic growth, we could use impl Index or similar
    // this is not a Vec, because the usecase for this is to save a roundtrip to frame alloc. a Vec would defeat that purpose.
    buf: &'a mut [Option<PhysMemoryFrame<FRAME_SIZE>>],
    cursor: usize,
}

impl<'a, const FRAME_SIZE: usize> DirectCollect<'a, FRAME_SIZE> {
    pub fn new(buf: &'a mut [Option<PhysMemoryFrame<FRAME_SIZE>>]) -> Self {
        Self { buf, cursor: 0 }
    }
}

impl<'a, const FRAME_SIZE: usize> FrameDeallocator<FRAME_SIZE> for DirectCollect<'a, FRAME_SIZE> {
    unsafe fn deallocate(
        &mut self,
        frame: PhysMemoryFrame<FRAME_SIZE>,
    ) -> Result<(), FrameDeallocationError> {
        loop {
            if self.cursor >= self.buf.len() {
                return Err(FrameDeallocationError::Other {
                    msg: "Cannot store more deallocated frames",
                });
            }

            if self.buf[self.cursor].is_some() {
                self.cursor += 1;
                continue;
            }

            self.buf[self.cursor].replace(frame);
            self.cursor += 1;
            return Ok(());
        }
    }
}

impl<'a, const FRAME_SIZE: usize> FrameAllocator<FRAME_SIZE> for DirectCollect<'a, FRAME_SIZE> {
    fn allocate(&mut self) -> Option<PhysMemoryFrame<FRAME_SIZE>> {
        while self.cursor > 0 {
            self.cursor -= 1;
            if let Some(f) = self.buf[self.cursor].take() {
                return Some(f);
            }
        }

        None
    }
}
