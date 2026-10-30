use core::ptr::null_mut;

use boot::memmap::{Entry, MEMMAP_BOOTLOADER_RECLAIMABLE, MEMMAP_USABLE};
use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::{
    PhysAddr,
    align_down,
    align_up,
    structures::paging::{FrameAllocator, FrameDeallocator, PageSize, PhysFrame, Size4KiB},
};

pub(crate) static FRAME_ALLOC: OnceCell<Mutex<LinkedListFrameAllocator>> = OnceCell::uninit();

pub(crate) fn init_frame_alloc<'a>(memmap: &'a [&'a Entry], hhdm_offset: usize) {
    FRAME_ALLOC.init_once(|| Mutex::new(LinkedListFrameAllocator::new(memmap, hhdm_offset)));
}

pub fn get_frame_alloc<'a>() -> &'a Mutex<impl FrameAllocator<Size4KiB> + FrameDeallocator<Size4KiB>>
{
    FRAME_ALLOC.get().unwrap()
}

#[derive(Clone, Copy)]
struct Region {
    base: usize,
    len: usize,
}

pub(crate) struct LinkedListFrameAllocator {
    head: *mut u64,
    current_batch_end: usize,
    regions: &'static mut [Region], // we allocate this in an empty frame at init
    hhdm_offset: usize,
    count: usize,
}

impl LinkedListFrameAllocator {
    fn new<'a>(memmap: &'a [&'a Entry], hhdm_offset: usize) -> Self {
        let is_usable = |e: &&&Entry| e.type_ == MEMMAP_USABLE;
        let is_reclaimable = |e: &&&Entry| e.type_ == MEMMAP_BOOTLOADER_RECLAIMABLE;

        let capacity = memmap
            .iter()
            .filter(|e| is_usable(e) || is_reclaimable(e))
            .count();
        let table_bytes =
            (capacity * size_of::<Region>()).next_multiple_of(Size4KiB::SIZE as usize);

        let host = memmap
            .iter()
            .find(|e| is_usable(e) && e.length as usize >= table_bytes)
            .expect("no usable region can host the region table");

        let ptr = (hhdm_offset + host.base as usize) as *mut Region;
        unsafe { core::ptr::write_bytes(ptr as *mut u8, 0, table_bytes) };
        let regions = unsafe { core::slice::from_raw_parts_mut(ptr, capacity) };

        let mut count = 0;
        for e in memmap.iter().filter(|e| e.type_ == MEMMAP_USABLE) {
            let mut base = align_up(e.base, Size4KiB::SIZE) as usize;
            let end = align_down(e.base + e.length, Size4KiB::SIZE) as usize;

            if core::ptr::eq(*e, *host) {
                base += table_bytes;
            }

            if end <= base {
                continue;
            }

            // len is at least Size4Kib::SIZE

            regions[count] = Region {
                base,
                len: end - base,
            };
            count += 1;
        }

        let mut alloc = Self {
            head: null_mut(),
            current_batch_end: 0,
            regions,
            hhdm_offset,
            count,
        };
        alloc.add_batch();
        alloc
    }

    pub(crate) fn add_region(&mut self, base: usize, len: usize) {
        assert!(self.count < self.regions.len());
        let start = align_up(base as u64, Size4KiB::SIZE) as usize;
        let end = align_down((base + len) as u64, Size4KiB::SIZE) as usize;

        if end <= start {
            return;
        }

        // len is at least Size4Kib::SIZE

        self.regions[self.count] = Region {
            base: start,
            len: end - start,
        };
        self.count += 1;
    }

    fn add_batch(&mut self) -> bool {
        const FRAMES_PER_BATCH: usize = 10000;

        // all regions have multiple of Size4Kib::SIZE bytes
        let Some(region) = self.regions[..self.count].iter_mut().find(|r| r.len > 0) else {
            return false;
        };

        let bytes = (FRAMES_PER_BATCH * Size4KiB::SIZE as usize).min(region.len);

        debug_assert_eq!(bytes % Size4KiB::SIZE as usize, 0);
        debug_assert!(bytes >= Size4KiB::SIZE as usize);

        let start = region.base;
        region.base += bytes;
        region.len -= bytes;

        let mut addr = start + bytes;
        while addr > start {
            addr -= Size4KiB::SIZE as usize;
            unsafe {
                self.deallocate_frame(PhysFrame::containing_address(PhysAddr::new(addr as u64)));
            }
        }
        true
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
        if self.head.is_null() && !self.add_batch() {
            // tried to add more frames, but none are available
            return None;
        }

        let next_head_ptr = self.head;
        let next_head = unsafe { *next_head_ptr };
        let current_phys = self.head as u64 - self.hhdm_offset as u64;
        self.head = next_head as *mut u64;

        let frame = PhysFrame::containing_address(PhysAddr::new(current_phys));
        unsafe {
            core::ptr::write_bytes(next_head_ptr, 0, Size4KiB::SIZE as usize);
        }
        Some(frame)
    }
}

unsafe impl Send for LinkedListFrameAllocator {}
