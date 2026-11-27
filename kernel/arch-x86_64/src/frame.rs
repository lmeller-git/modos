use core::ptr::NonNull;

use arch_interfaces::mm::FrameDeallocationError;
use boot::memmap::{MemoryMap, RegionKind};
use conquer_once::spin::OnceCell;
use kernel_types::{
    addr::{PhysAddr, VirtAddr},
    frame::{FRAME_4KIB_SIZE, PhysMemoryFrame, PhysMemoryFrameRange},
    hhdm::Hhdm,
};
use spin::Mutex;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, PhysFrame, Size4KiB};

use crate::conv;

pub(crate) static FRAME_ALLOC: OnceCell<Mutex<LinkedListFrameAllocator>> = OnceCell::uninit();

pub(crate) fn init_frame_alloc(memmap: &MemoryMap<'_>, hhdm: Hhdm) {
    FRAME_ALLOC.init_once(|| Mutex::new(LinkedListFrameAllocator::new(memmap, hhdm)));
}

pub(crate) fn get_frame_alloc<'a>()
-> &'a Mutex<impl FrameAllocator<Size4KiB> + FrameDeallocator<Size4KiB>> {
    FRAME_ALLOC.get().unwrap()
}

pub(crate) struct LinkedListFrameAllocator {
    /// Free list through the frames themselves, by HHDM address.
    head: Option<NonNull<usize>>,
    /// Frames not yet on the free list. Allocated in an empty frame at init.
    regions: &'static mut [PhysMemoryFrameRange],
    hhdm: Hhdm,
    count: usize,
}

impl LinkedListFrameAllocator {
    fn new(memmap: &MemoryMap<'_>, hhdm: Hhdm) -> Self {
        let is_usable = |kind| kind == RegionKind::Usable;

        let capacity = memmap
            .regions()
            .filter(|r| {
                matches!(
                    r.kind(),
                    RegionKind::Usable | RegionKind::BootloaderReclaimable
                )
            })
            .count();
        let table_frames = (capacity * size_of::<PhysMemoryFrameRange>()).div_ceil(FRAME_4KIB_SIZE);

        // TODO:
        // We reverse the memmap here, because for some reason low regions are marked as useable while being used by limine at the same time.
        // If we allocate these regions to heap, ... while we still use tehm (bootinfo, ...) we may crash.
        // Should consult docs for a more robust fix here.
        let (host_index, host) = memmap
            .regions()
            .rev()
            .enumerate()
            .find(|(_, r)| is_usable(r.kind()) && r.frames().len() >= table_frames)
            .expect("no usable region can host the region table");
        let table = host.frames().take_front(table_frames).start().get();

        let ptr = hhdm
            .ptr::<PhysMemoryFrameRange>(table)
            .expect("region table outside the HHDM");
        // SAFETY: the table frames are usable memory, mapped by the HHDM, and handed out by no one
        // else. An all-zero `PhysMemoryFrameRange` is a valid (empty) range.
        let regions = unsafe {
            ptr.cast::<u8>()
                .write_bytes(0, table_frames * FRAME_4KIB_SIZE);
            core::slice::from_raw_parts_mut(ptr.as_ptr(), capacity)
        };

        let mut count = 0;
        for (index, region) in memmap
            .regions()
            .rev()
            .enumerate()
            .filter(|(_, r)| is_usable(r.kind()))
        {
            let mut frames = region.frames();

            if index == host_index {
                frames.take_front(table_frames);
            }

            if frames.len() > 0 {
                regions[count] = frames;
                count += 1;
            }
        }

        let mut alloc = Self {
            head: None,
            regions,
            hhdm,
            count,
        };
        alloc.add_batch();
        alloc
    }

    /// Hands `frames` to the allocator. At most one call per region.
    // TODO: could be unsafe marked
    pub(crate) fn add_region(&mut self, frames: PhysMemoryFrameRange) {
        if frames.len() == 0 {
            return;
        }
        assert!(self.count < self.regions.len(), "region table is full");
        self.regions[self.count] = frames;
        self.count += 1;
    }

    fn add_batch(&mut self) -> bool {
        const FRAMES_PER_BATCH: usize = 1000;

        let Some(region) = self.regions[..self.count].iter_mut().find(|r| r.len() > 0) else {
            return false;
        };

        // Pushed from the top, so the batch is handed out in ascending order.
        for frame in region.take_front(FRAMES_PER_BATCH).rev() {
            unsafe { self.deallocate_frame(conv::to_x86_frame(frame)) };
        }
        true
    }

    fn virt(&self, phys: PhysAddr) -> NonNull<usize> {
        self.hhdm.ptr(phys).expect("frame outside the HHDM")
    }
}

impl FrameDeallocator<Size4KiB> for LinkedListFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        // write current head into frame and point head to frame

        // TODO: we may wan to detect double frees here. Cant err right now anyways though.
        // TODO thus we should maybe just depend this in terms of arch-interfaces::mm::FrameDeallocator.

        let addr = self.virt(conv::from_x86_frame(frame).get());
        let next = self
            .head
            .map_or(0, |head| head.as_ptr().expose_provenance());
        unsafe { addr.write(next) };
        self.head = Some(addr);
    }
}

unsafe impl FrameAllocator<Size4KiB> for LinkedListFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        // get current frame from head and update head to point to next
        if self.head.is_none() && !self.add_batch() {
            // tried to add more frames, but none are available
            return None;
        }
        let head = self.head?;

        let next = unsafe { head.read() };
        self.head = NonNull::new(core::ptr::with_exposed_provenance_mut(next));

        let phys = self
            .hhdm
            .to_phys(VirtAddr::new(head.addr().get()))
            .expect("free list entry outside the HHDM");
        unsafe { head.cast::<u8>().write_bytes(0, FRAME_4KIB_SIZE) };
        Some(conv::to_x86_frame(PhysMemoryFrame::containing(phys)))
    }
}

pub(crate) struct X86FrameAllocView<'a, A>(pub(crate) &'a mut A);

impl<'a, A> arch_interfaces::mm::FrameAllocator<FRAME_4KIB_SIZE> for X86FrameAllocView<'a, A>
where
    A: FrameAllocator<Size4KiB>,
{
    fn allocate(&mut self) -> Option<PhysMemoryFrame> {
        self.0.allocate_frame().map(conv::from_x86_frame)
    }
}

impl<'a, A> arch_interfaces::mm::FrameDeallocator<FRAME_4KIB_SIZE> for X86FrameAllocView<'a, A>
where
    A: FrameDeallocator<Size4KiB>,
{
    unsafe fn deallocate(&mut self, frame: PhysMemoryFrame) -> Result<(), FrameDeallocationError> {
        unsafe { self.0.deallocate_frame(conv::to_x86_frame(frame)) };
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
    use core::{alloc::Layout, num::NonZeroUsize};

    use boot::memmap::{
        Entry,
        MEMMAP_ACPI_RECLAIMABLE,
        MEMMAP_BOOTLOADER_RECLAIMABLE,
        MEMMAP_RESERVED,
        MEMMAP_USABLE,
    };
    use kernel_types::aligned::AlignedRange;
    use proptest::prelude::*;
    use test_host::{config, panics};

    use super::*;

    const PAGE: usize = FRAME_4KIB_SIZE;
    /// Fake physical address of the first arena byte.
    const PHYS_BASE: usize = 0x10_0000;
    const FILL: u8 = 0xAB;

    #[cfg(any(miri, loom, shuttle))]
    const LIMITS: (usize, usize, usize) = (3, 3, 16); // entries, pages per entry, ops
    #[cfg(not(any(miri, loom, shuttle)))]
    const LIMITS: (usize, usize, usize) = (7, 8, 120);

    /// Host memory standing in for physical memory, mapped at `hhdm`.
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

        fn hhdm(&self) -> Hhdm {
            let offset = NonZeroUsize::new(self.ptr.expose_provenance() - PHYS_BASE).unwrap();
            // SAFETY: the arena backs `[PHYS_BASE, PHYS_BASE + pages * PAGE)` at this offset.
            unsafe { Hhdm::new(offset) }
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
        /// Bytes cut from the start and end when this entry is reclaimed via `add_region`.
        trim: (usize, usize),
    }

    impl Span {
        fn entry(&self) -> Entry {
            Entry {
                base: (PHYS_BASE + self.first * PAGE) as u64,
                length: (self.pages * PAGE) as u64,
                type_: self.ty,
            }
        }

        /// `(base, len)` handed to `add_region` on reclaim; not page-aligned when trimmed.
        fn reclaimed(&self) -> (usize, usize) {
            let len = self.pages * PAGE;
            let head = self.trim.0.min(len);
            let tail = self.trim.1.min(len - head);
            (PHYS_BASE + self.first * PAGE + head, len - head - tail)
        }

        /// Pages lying wholly inside `[base, base + len)`.
        fn pages_within(base: usize, len: usize) -> impl Iterator<Item = usize> {
            let first = base.next_multiple_of(PAGE);
            (first..(base + len) / PAGE * PAGE).step_by(PAGE)
        }
    }

    /// Entries in order with gaps between them, at least one usable (as the allocator needs).
    fn spans() -> impl Strategy<Value = Vec<Span>> {
        let ty = prop_oneof![
            4 => Just(MEMMAP_USABLE),
            1 => Just(MEMMAP_RESERVED),
            2 => Just(MEMMAP_BOOTLOADER_RECLAIMABLE),
            1 => Just(MEMMAP_ACPI_RECLAIMABLE),
        ];
        let cut = || prop_oneof![Just(0), Just(1), Just(PAGE - 1), 0..2 * PAGE];
        prop::collection::vec((0..3usize, 1..LIMITS.1, ty, (cut(), cut())), 1..LIMITS.0).prop_map(
            |raw| {
                let mut next = 0;
                let mut spans: Vec<Span> = raw
                    .into_iter()
                    .map(|(gap, pages, ty, trim)| {
                        let span = Span {
                            first: next + gap,
                            pages,
                            ty,
                            trim,
                        };
                        next = span.first + pages;
                        span
                    })
                    .collect();
                if spans.iter().all(|s| s.ty != MEMMAP_USABLE) {
                    spans.last_mut().unwrap().ty = MEMMAP_USABLE;
                }
                spans
            },
        )
    }

    #[derive(Clone, Copy, Debug)]
    enum Op {
        Alloc,
        Free(prop::sample::Index),
        /// Hands the next bootloader-reclaimable entry to `add_region`.
        Reclaim,
    }

    fn ops() -> impl Strategy<Value = Vec<Op>> {
        let op = prop_oneof![
            6 => Just(Op::Alloc),
            4 => any::<prop::sample::Index>().prop_map(Op::Free),
            1 => Just(Op::Reclaim),
        ];
        prop::collection::vec(op, 0..LIMITS.2)
    }

    /// Runs `ops` against an allocator over `spans`, then returns everything and drains it.
    ///
    /// The allocator hands out only usable and reclaimed pages, each at most once at a time and
    /// zeroed, and writes nothing else. All of them but the pages of its region table (one entry
    /// per usable or reclaimable map entry, placed in usable memory) come back.
    fn check(spans: &[Span], ops: &[Op]) {
        let arena = Arena::new(spans.last().map_or(0, |s| s.first + s.pages));
        let entries: Vec<Entry> = spans.iter().map(Span::entry).collect();
        let map: Vec<&Entry> = entries.iter().collect();
        let mut alloc = LinkedListFrameAllocator::new(&MemoryMap::new(&map), arena.hhdm());

        let usable: BTreeSet<usize> = spans
            .iter()
            .filter(|s| s.ty == MEMMAP_USABLE)
            .flat_map(|s| Span::pages_within(PHYS_BASE + s.first * PAGE, s.pages * PAGE))
            .collect();
        let tracked = spans
            .iter()
            .filter(|s| matches!(s.ty, MEMMAP_USABLE | MEMMAP_BOOTLOADER_RECLAIMABLE))
            .count();
        let table_pages = (tracked * size_of::<PhysMemoryFrameRange>()).div_ceil(PAGE);
        let mut reclaimable = spans
            .iter()
            .filter(|s| s.ty == MEMMAP_BOOTLOADER_RECLAIMABLE);
        let mut candidates = usable.clone();

        let take = |alloc: &mut LinkedListFrameAllocator, candidates: &BTreeSet<usize>| {
            alloc.allocate_frame().map(|frame| {
                let phys = frame.start_address().as_u64() as usize;
                assert!(candidates.contains(&phys), "{phys:#x} is not allocatable");
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
                Op::Alloc => match take(&mut alloc, &candidates) {
                    Some(phys) => {
                        assert!(!outstanding.contains(&phys), "{phys:#x} handed out twice");
                        outstanding.push(phys);
                    }
                    None => assert!(
                        outstanding.len() + table_pages >= candidates.len(),
                        "exhausted early"
                    ),
                },
                Op::Free(index) if !outstanding.is_empty() => {
                    let phys = outstanding.swap_remove(index.index(outstanding.len()));
                    // Freed frames keep stale data; allocation must clear it.
                    unsafe { arena.ptr.add(phys - PHYS_BASE).write_bytes(0xCD, PAGE) };
                    unsafe { alloc.deallocate_frame(frame_at(phys)) };
                }
                Op::Free(_) => {}
                Op::Reclaim => {
                    if let Some(span) = reclaimable.next() {
                        let (base, len) = span.reclaimed();
                        alloc.add_region(frames_within(base, len));
                        candidates.extend(Span::pages_within(base, len));
                    }
                }
            }
        }

        for phys in outstanding.drain(..) {
            unsafe { alloc.deallocate_frame(frame_at(phys)) };
        }
        let drained: BTreeSet<usize> =
            core::iter::from_fn(|| take(&mut alloc, &candidates)).collect();
        let kept: Vec<usize> = candidates.difference(&drained).copied().collect();
        assert_eq!(
            kept.len(),
            table_pages,
            "pages lost besides the region table"
        );
        assert!(
            kept.iter().all(|p| usable.contains(p)),
            "table outside usable memory"
        );
        assert!(alloc.allocate_frame().is_none());

        for page in 0..arena.pages {
            let phys = PHYS_BASE + page * PAGE;
            if !candidates.contains(&phys) {
                assert!(
                    arena.page(phys).iter().all(|&b| b == FILL),
                    "{phys:#x} was written"
                );
            }
        }
    }

    fn frame_at(phys: usize) -> PhysFrame<Size4KiB> {
        conv::to_x86_frame(PhysMemoryFrame::new(PhysAddr::new(phys)).unwrap())
    }

    /// The whole frames inside `[base, base + len)`, as handed over on reclaim.
    fn frames_within(base: usize, len: usize) -> PhysMemoryFrameRange {
        AlignedRange::within(PhysAddr::new(base), PhysAddr::new(base + len))
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn allocator(spans in spans(), ops in ops()) {
            check(&spans, &ops);
        }
    }

    fn span(first: usize, pages: usize, ty: u64) -> Span {
        Span {
            first,
            pages,
            ty,
            trim: (0, 0),
        }
    }

    /// The region table has room for exactly the usable and reclaimable entries.
    #[test]
    fn region_capacity() {
        let spans = [
            span(0, 4, MEMMAP_USABLE),
            span(4, 2, MEMMAP_BOOTLOADER_RECLAIMABLE),
        ];
        let arena = Arena::new(6);
        let entries: Vec<Entry> = spans.iter().map(Span::entry).collect();
        let map: Vec<&Entry> = entries.iter().collect();
        let mut alloc = LinkedListFrameAllocator::new(&MemoryMap::new(&map), arena.hhdm());

        let (base, len) = spans[1].reclaimed();
        alloc.add_region(frames_within(base, len));
        assert!(panics(|| alloc.add_region(frames_within(base, len))));
    }

    /// Freeing a frame twice is reported instead of corrupting the free list.
    #[test]
    #[ignore = "frame alloc currently ignores double free"]
    fn double_free() {
        let arena = Arena::new(4);
        let entries = [span(0, 4, MEMMAP_USABLE).entry()];
        let map: Vec<&Entry> = entries.iter().collect();
        let mut alloc = LinkedListFrameAllocator::new(&MemoryMap::new(&map), arena.hhdm());
        let mut view = X86FrameAllocView(&mut alloc);

        let start = arch_interfaces::mm::FrameAllocator::allocate(&mut view)
            .unwrap()
            .get();
        let free = |view: &mut X86FrameAllocView<'_, _>| unsafe {
            let frame = PhysMemoryFrame::new(start).unwrap();
            arch_interfaces::mm::FrameDeallocator::deallocate(view, frame)
        };
        assert_eq!(free(&mut view), Ok(()));
        assert_eq!(free(&mut view), Err(FrameDeallocationError::DoubleFree));
    }

    /// A region larger than one refill batch.
    #[cfg_attr(miri, ignore = "too slow under miri")]
    #[test]
    fn batches() {
        let pages = 2 * 1000 + 100;
        check(&[span(0, pages, MEMMAP_USABLE)], &[]);
    }
}
