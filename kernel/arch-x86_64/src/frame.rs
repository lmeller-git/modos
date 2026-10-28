use core::ptr::null_mut;

use boot::memmap::{Entry, MEMMAP_USABLE};
use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::{
    PhysAddr,
    align_down,
    align_up,
    structures::paging::{FrameAllocator, FrameDeallocator, PageSize, PhysFrame, Size4KiB},
};

pub(crate) static FRAME_ALLOC: OnceCell<Mutex<LinkedListFrameAllocator>> = OnceCell::uninit();

pub(crate) fn init_frame_alloc(memmap: &'static [&'static Entry], hhdm_offset: usize) {
    FRAME_ALLOC.init_once(|| Mutex::new(LinkedListFrameAllocator::new(memmap, hhdm_offset)));
}

pub fn get_frame_alloc<'a>() -> &'a Mutex<impl FrameAllocator<Size4KiB> + FrameDeallocator<Size4KiB>>
{
    FRAME_ALLOC.get().unwrap()
}

pub(crate) struct LinkedListFrameAllocator {
    head: *mut u64,
    current_batch_end: usize,
    memmap: &'static [&'static Entry],
    hhdm_offset: usize,
}

impl LinkedListFrameAllocator {
    fn new(memmap: &'static [&'static Entry], hhdm_offset: usize) -> Self {
        let initial = null_mut();
        let mut alloc = Self {
            head: initial,
            current_batch_end: 0,
            memmap,
            hhdm_offset,
        };
        alloc.add_batch();
        alloc
    }

    fn add_batch(&mut self) {
        let offset = self.hhdm_offset as u64;
        let usable_regions = self.memmap.iter().filter_map(|e| match e.type_ {
            MEMMAP_USABLE => Some((
                if e.base >= 0x100000000 {
                    e.base + offset
                } else {
                    e.base
                },
                e.length,
            )),
            _ => None,
        });
        let frames = usable_regions
            .map(|r| align_up(r.0, Size4KiB::SIZE)..align_down(r.0 + r.1, Size4KiB::SIZE))
            .flat_map(|r| r.step_by(Size4KiB::SIZE as usize))
            .map(|r| PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(r)));

        let next_batch = frames.skip(self.current_batch_end).take(10000);
        for frame in next_batch {
            unsafe {
                self.deallocate_frame(frame);
            }
            self.current_batch_end += 1;
        }
    }
}

impl FrameDeallocator<Size4KiB> for LinkedListFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        // write current head into frame and point head to frame

        let addr = (frame.start_address().as_u64() + self.hhdm_offset as u64) as *mut u64;
        unsafe { addr.write(self.head as u64) };
        self.head = addr;
    }
}

unsafe impl FrameAllocator<Size4KiB> for LinkedListFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        // get current frame from head and update head to point to next
        if self.head.is_null() {
            self.add_batch();
            if self.head.is_null() {
                // tried to add more frames, but none are available
                return None;
            }
        }

        let next_head = unsafe { *self.head };
        let current_phys = self.head as u64 - self.hhdm_offset as u64;
        self.head = next_head as *mut u64;

        let frame = PhysFrame::containing_address(PhysAddr::new(current_phys));
        unsafe {
            core::ptr::write_bytes(
                (frame.start_address().as_u64() + self.hhdm_offset as u64) as *mut u8,
                0,
                Size4KiB::SIZE as usize,
            );
        }
        Some(frame)
    }
}

unsafe impl Send for LinkedListFrameAllocator {}
