use core::ptr::null_mut;

use arch_interfaces::mm::{FrameDeallocationError, PhysMemoryFrame};
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

pub(crate) fn get_frame_alloc<'a>()
-> &'a Mutex<impl FrameAllocator<Size4KiB> + FrameDeallocator<Size4KiB>> {
    FRAME_ALLOC.get().unwrap()
}

#[derive(Clone, Copy)]
struct Region {
    base: usize,
    len: usize,
}

pub(crate) struct LinkedListFrameAllocator {
    head: *mut u64,
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

        // TODO:
        // We reverse the memmap here, because for some reason low regions are marked as useable while being used by limine at the same time.
        // If we allocate these regions to heap, ... while we still use tehm (bootinfo, ...) we may crash.
        // Should consult docs for a more robust fix here.
        let host = memmap
            .iter()
            .rev()
            .find(|e| is_usable(e) && e.length as usize >= table_bytes)
            .expect("no usable region can host the region table");

        assert_eq!(host.base, align_up(host.base, Size4KiB::SIZE));
        let ptr = (hhdm_offset + host.base as usize) as *mut Region;
        unsafe { core::ptr::write_bytes(ptr.cast::<u8>(), 0, table_bytes) };
        let regions = unsafe { core::slice::from_raw_parts_mut(ptr, capacity) };

        let mut count = 0;
        for e in memmap.iter().rev().filter(is_usable) {
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
        const FRAMES_PER_BATCH: usize = 1000;

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

        // TODO: we may wan to detect double frees here. Cant err right now anyways though.
        // TODO thus we should maybe just depend this in terms of arch-interfaces::mm::FrameDeallocator.

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
            core::ptr::write_bytes(next_head_ptr.cast::<u8>(), 0, Size4KiB::SIZE as usize);
        }
        Some(frame)
    }
}

pub(crate) struct X86FrameAllocView<'a, A>(pub(crate) &'a mut A);

impl<'a, A> arch_interfaces::mm::FrameAllocator<{ Size4KiB::SIZE as usize }>
    for X86FrameAllocView<'a, A>
where
    A: FrameAllocator<Size4KiB>,
{
    fn allocate(&mut self) -> Option<PhysMemoryFrame<{ Size4KiB::SIZE as usize }>> {
        self.0.allocate_frame().and_then(|frame| {
            PhysMemoryFrame::from_addr(arch_interfaces::mm::PhysAddr::new(
                frame.start_address().as_u64() as usize,
            ))
        })
    }
}

impl<'a, A> arch_interfaces::mm::FrameDeallocator<{ Size4KiB::SIZE as usize }>
    for X86FrameAllocView<'a, A>
where
    A: FrameDeallocator<Size4KiB>,
{
    unsafe fn deallocate(
        &mut self,
        frame: PhysMemoryFrame<{ Size4KiB::SIZE as usize }>,
    ) -> Result<(), FrameDeallocationError> {
        unsafe {
            self.0.deallocate_frame(
                PhysFrame::from_start_address(PhysAddr::new(frame.start().as_usize() as u64))
                    .map_err(|_| FrameDeallocationError::InvalidFrame)?,
            );
        }
        Ok(())
    }
}

unsafe impl Send for LinkedListFrameAllocator {}

#[cfg(test)]
mod tests {
    use alloc::{
        alloc::{alloc, dealloc},
        collections::BTreeSet,
        vec::Vec,
    };
    use core::alloc::Layout;

    use boot::memmap::{MEMMAP_ACPI_RECLAIMABLE, MEMMAP_RESERVED};
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    const PAGE: usize = Size4KiB::SIZE as usize;
    /// Fake physical address of the first arena byte.
    const PHYS_BASE: usize = 0x10_0000;
    const FILL: u8 = 0xAB;

    #[cfg(any(miri, loom, shuttle))]
    const LIMITS: (usize, usize, usize) = (3, 3, 16); // entries, pages per entry, ops
    #[cfg(not(any(miri, loom, shuttle)))]
    const LIMITS: (usize, usize, usize) = (7, 8, 120);

    /// Host memory standing in for physical memory, mapped at `hhdm_offset`.
    struct Arena {
        ptr: *mut u8,
        pages: usize,
    }

    impl Arena {
        fn new(pages: usize) -> Self {
            let layout = Layout::from_size_align(pages * PAGE, PAGE).unwrap();
            let ptr = unsafe { alloc(layout) };
            assert!(!ptr.is_null() && ptr as usize > PHYS_BASE);
            unsafe { ptr.write_bytes(FILL, pages * PAGE) };
            Self { ptr, pages }
        }

        fn hhdm_offset(&self) -> usize {
            self.ptr as usize - PHYS_BASE
        }

        fn page(&self, phys: usize) -> &[u8] {
            assert!(phys >= PHYS_BASE && phys + PAGE <= PHYS_BASE + self.pages * PAGE);
            unsafe { core::slice::from_raw_parts(self.ptr.add(phys - PHYS_BASE), PAGE) }
        }
    }

    impl Drop for Arena {
        fn drop(&mut self) {
            let layout = Layout::from_size_align(self.pages * PAGE, PAGE).unwrap();
            unsafe { dealloc(self.ptr, layout) };
        }
    }

    /// One memory map entry, in pages from the start of the arena.
    #[derive(Clone, Copy, Debug)]
    struct Span {
        first: usize,
        pages: usize,
        ty: u64,
    }

    /// Entries in order with gaps between them, at least one usable (as the allocator needs).
    fn spans() -> impl Strategy<Value = Vec<Span>> {
        let ty = prop_oneof![
            4 => Just(MEMMAP_USABLE),
            1 => Just(MEMMAP_RESERVED),
            2 => Just(MEMMAP_BOOTLOADER_RECLAIMABLE),
            1 => Just(MEMMAP_ACPI_RECLAIMABLE),
        ];
        prop::collection::vec((0..3usize, 1..LIMITS.1, ty), 1..LIMITS.0).prop_map(|raw| {
            let mut next = 0;
            let mut spans: Vec<Span> = raw
                .into_iter()
                .map(|(gap, pages, ty)| {
                    let span = Span {
                        first: next + gap,
                        pages,
                        ty,
                    };
                    next = span.first + pages;
                    span
                })
                .collect();
            if spans.iter().all(|s| s.ty != MEMMAP_USABLE) {
                spans.last_mut().unwrap().ty = MEMMAP_USABLE;
            }
            spans
        })
    }

    #[derive(Clone, Copy, Debug)]
    enum Op {
        Alloc,
        Free(prop::sample::Index),
    }

    fn ops() -> impl Strategy<Value = Vec<Op>> {
        let op = prop_oneof![
            3 => Just(Op::Alloc),
            2 => any::<prop::sample::Index>().prop_map(Op::Free),
        ];
        prop::collection::vec(op, 0..LIMITS.2)
    }

    /// Runs `ops` against an allocator over `spans`, then returns everything and drains it.
    ///
    /// The allocator must hand out exactly the usable pages, except the first page of the
    /// last usable entry (which hosts its region table), each at most once at a time and
    /// zeroed, and must not write anywhere else.
    fn check(spans: &[Span], ops: &[Op]) {
        let arena = Arena::new(spans.last().map_or(0, |s| s.first + s.pages));
        let entries: Vec<Entry> = spans
            .iter()
            .map(|s| Entry {
                base: (PHYS_BASE + s.first * PAGE) as u64,
                length: (s.pages * PAGE) as u64,
                type_: s.ty,
            })
            .collect();
        let map: Vec<&Entry> = entries.iter().collect();
        let mut alloc = LinkedListFrameAllocator::new(&map, arena.hhdm_offset());

        let host = spans.iter().rfind(|s| s.ty == MEMMAP_USABLE).unwrap();
        let table_page = PHYS_BASE + host.first * PAGE;
        let expected: BTreeSet<usize> = spans
            .iter()
            .filter(|s| s.ty == MEMMAP_USABLE)
            .flat_map(|s| (s.first..s.first + s.pages).map(|p| PHYS_BASE + p * PAGE))
            .filter(|&phys| phys != table_page)
            .collect();

        let take = |alloc: &mut LinkedListFrameAllocator| {
            alloc.allocate_frame().map(|frame| {
                let phys = frame.start_address().as_u64() as usize;
                assert!(expected.contains(&phys), "{phys:#x} is not allocatable");
                assert!(
                    arena.page(phys).iter().all(|&b| b == 0),
                    "{phys:#x} not zeroed"
                );
                phys
            })
        };

        let mut outstanding: Vec<usize> = Vec::new();
        for op in ops {
            match op {
                Op::Alloc => match take(&mut alloc) {
                    Some(phys) => {
                        assert!(!outstanding.contains(&phys), "{phys:#x} handed out twice");
                        outstanding.push(phys);
                    }
                    None => assert_eq!(outstanding.len(), expected.len(), "exhausted early"),
                },
                Op::Free(index) if !outstanding.is_empty() => {
                    let phys = outstanding.swap_remove(index.index(outstanding.len()));
                    // Freed frames keep stale data; allocation must clear it.
                    unsafe { arena.ptr.add(phys - PHYS_BASE).write_bytes(0xCD, PAGE) };
                    unsafe { alloc.deallocate_frame(frame_at(phys)) };
                }
                Op::Free(_) => {}
            }
        }

        for phys in outstanding.drain(..) {
            unsafe { alloc.deallocate_frame(frame_at(phys)) };
        }
        let drained: BTreeSet<usize> = core::iter::from_fn(|| take(&mut alloc)).collect();
        assert_eq!(drained, expected);
        assert!(alloc.allocate_frame().is_none());

        for page in 0..arena.pages {
            let phys = PHYS_BASE + page * PAGE;
            if phys != table_page && !expected.contains(&phys) {
                assert!(
                    arena.page(phys).iter().all(|&b| b == FILL),
                    "{phys:#x} was written"
                );
            }
        }
    }

    fn frame_at(phys: usize) -> PhysFrame<Size4KiB> {
        PhysFrame::containing_address(PhysAddr::new(phys as u64))
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn allocator(spans in spans(), ops in ops()) {
            check(&spans, &ops);
        }
    }

    /// A region larger than one refill batch.
    #[cfg_attr(miri, ignore = "too slow under miri")]
    #[test]
    fn batches() {
        let pages = 2 * 1000 + 100;
        let span = Span {
            first: 0,
            pages,
            ty: MEMMAP_USABLE,
        };
        check(&[span], &[]);
    }
}
