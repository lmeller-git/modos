use arch_interfaces::mm::{FRAME_4KIB_SIZE, PhysMemoryFrame};

pub trait FrameAllocator<const SIZE: usize> {
    fn allocate(&mut self) -> Option<PhysMemoryFrame<SIZE>>;
}

pub trait FrameDeallocator<const SIZE: usize> {
    unsafe fn deallocate(&mut self, frame: PhysMemoryFrame<SIZE>) -> Result<(), ()>;
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
    buf: &'a mut [Option<PhysMemoryFrame<FRAME_SIZE>>],
    cursor: usize,
}

impl<'a, const FRAME_SIZE: usize> DirectCollect<'a, FRAME_SIZE> {
    pub fn new(buf: &'a mut [Option<PhysMemoryFrame<FRAME_SIZE>>]) -> Self {
        Self { buf, cursor: 0 }
    }
}

impl<'a, const FRAME_SIZE: usize> FrameDeallocator<FRAME_SIZE> for DirectCollect<'a, FRAME_SIZE> {
    unsafe fn deallocate(&mut self, frame: PhysMemoryFrame<FRAME_SIZE>) -> Result<(), ()> {
        loop {
            if self.cursor >= self.buf.len() {
                return Err(());
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
