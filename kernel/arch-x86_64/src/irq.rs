use alloc::boxed::Box;
use core::{
    ops::Index,
    sync::atomic::{AtomicPtr, AtomicU64, Ordering},
};

use arch_interfaces::interrupt::IrqHandler;
use core_local::CLSLazyPtr;
use early_log::early_println;
use spin::Mutex;
use x86_64::{
    instructions::interrupts,
    structures::idt::{InterruptStackFrame, PageFaultErrorCode},
};

use crate::{apic::lapic::end_interrupt, idt::InterruptIndex};

const RESERVED_IRQ: [u8; 3] = [
    InterruptIndex::Timer as u8,
    InterruptIndex::Error as u8,
    InterruptIndex::Spurious as u8,
];

#[core_local::core_local]
pub(crate) static IRQ_CONTROLLER: CLSLazyPtr<CoreIrqTable> = CLSLazyPtr::new();

pub(crate) extern "x86-interrupt" fn irq_dispatch<const VEC: u8>(_f: InterruptStackFrame) {
    if !IRQ_CONTROLLER.with(|c| c.get().unwrap().dispatch(VEC)) {
        early_println!("[irq] Unhandled interrupt: {:#x}", VEC);
    }
    end_interrupt();
}

struct IrqAction {
    handler: &'static dyn IrqHandler,
}

struct Bitmap {
    used: [u64; 4],
}

impl Bitmap {
    const fn new() -> Self {
        let mut used = [0u64; 4];
        used[0] = 0x0000_0000_FFFF_FFFF; // 0x00..=0x1F CPU exceptions

        let mut i = 0;
        while i < RESERVED_IRQ.len() {
            let v = RESERVED_IRQ[i] as usize;
            used[v / 64] |= 1u64 << (v % 64);
            i += 1;
        }
        Self { used }
    }

    fn alloc(&mut self) -> Option<u8> {
        for (word_idx, word) in self.used.iter_mut().enumerate() {
            if *word != u64::MAX {
                let bit = word.trailing_ones();
                *word |= 1u64 << bit;
                return Some((word_idx as u8) * 64 + (bit as u8));
            }
        }
        None
    }

    fn alloc_at(&mut self, vec: u8) {
        let word = &mut self.used[(vec / 64) as usize];
        let mask = 1u64 << (vec % 64);
        *word |= mask;
    }

    fn free(&mut self, vec: u8) {
        let idx = (vec / 64) as usize;
        let bit = 1u64 << (vec % 64);
        debug_assert_ne!(
            self.used[idx] & bit,
            0,
            "double-free of IRQ vector {vec:#x}"
        );
        self.used[idx] &= !bit;
    }
}

impl Index<u8> for Bitmap {
    type Output = bool;

    #[inline]
    fn index(&self, index: u8) -> &Self::Output {
        let word = self.used[(index / 64) as usize];
        if (word & (1u64 << (index % 64))) != 0 {
            &true
        } else {
            &false
        }
    }
}

pub(crate) struct CoreIrqTable {
    bitmap: Mutex<Bitmap>,
    slots: [AtomicPtr<IrqAction>; 256],
}

impl CoreIrqTable {
    pub(crate) const fn new() -> Self {
        Self {
            bitmap: Mutex::new(Bitmap::new()),
            slots: [const { AtomicPtr::new(core::ptr::null_mut()) }; 256],
        }
    }

    #[inline]
    pub(crate) fn dispatch(&self, vector: u8) -> bool {
        let ptr = self.slots[vector as usize].load(Ordering::Acquire);
        (!ptr.is_null())
            .then(|| unsafe { (&*ptr).handler.handle(vector) })
            .is_some()
    }

    pub(crate) fn register(&self, handler: &'static dyn IrqHandler) -> Option<u8> {
        let vec = interrupts::without_interrupts(|| self.bitmap.lock().alloc())?;
        let action = Box::into_raw(Box::new(IrqAction { handler }));
        self.slots[vec as usize].store(action, Ordering::Release);
        Some(vec)
    }

    pub(crate) fn register_at(&self, handler: &'static dyn IrqHandler, vec: u8) -> Option<()> {
        let action = Box::into_raw(Box::new(IrqAction { handler }));

        let mut bitmap = self.bitmap.lock();
        if bitmap[vec] {
            return None;
        }
        bitmap.alloc_at(vec);

        debug_assert!(self.slots[vec as usize].load(Ordering::Acquire).is_null());
        self.slots[vec as usize].store(action, Ordering::Release);
        Some(())
    }

    pub(crate) unsafe fn unregister(&self, vec: u8) -> Option<&'static dyn IrqHandler> {
        let old = self.slots[vec as usize].swap(core::ptr::null_mut(), Ordering::AcqRel);
        interrupts::without_interrupts(|| self.bitmap.lock().free(vec));
        (!old.is_null()).then(|| unsafe { Box::from_raw(old) }.handler)
    }
}

static TOTAL_TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub(crate) mod handlers {
    use core::sync::atomic::Ordering;

    use super::{InterruptStackFrame, PageFaultErrorCode, early_println, end_interrupt};
    use crate::irq::TOTAL_TIMER_TICKS;

    pub(crate) extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
        // println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
        panic!("breakpoint hit, but not supported: {stack_frame:?}");
    }

    pub(crate) extern "x86-interrupt" fn double_fault_handler(
        stack_frame: InterruptStackFrame,
        _error_code: u64,
    ) -> ! {
        panic!("EXCEPTION: DOUBLE FAULT\n{stack_frame:#?}");
    }

    pub(crate) extern "x86-interrupt" fn timer_interrupt(_stack_frame: InterruptStackFrame) {
        TOTAL_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        end_interrupt();
    }

    // #[expect(dead_code)]
    // pub(crate) fn current_tick() -> u64 {
    //     TOTAL_TIMER_TICKS.load(Ordering::Acquire)
    // }

    pub(crate) extern "x86-interrupt" fn page_fault_handler(
        stack_frame: InterruptStackFrame,
        error_code: PageFaultErrorCode,
    ) {
        use x86_64::registers::control::Cr2;
        panic!(
            "EXCEPTION Page fault:\naccessed address: {:?}\nerror code: {:?}\nstack_frame: {:?}",
            Cr2::read(),
            error_code,
            stack_frame
        )
    }

    pub(crate) extern "x86-interrupt" fn gpf_handler(
        stack_frame: InterruptStackFrame,
        error_code: u64,
    ) {
        panic!("EXCEPTION: GENERAL PROTECTION FAULT\n{stack_frame:#?}\nError Code: {error_code:b}");
    }

    pub(crate) extern "x86-interrupt" fn spurious_interrupt_handler(
        _stack_frame: InterruptStackFrame,
    ) {
        // nothing to do
        early_println!("spurious interrupt");
    }

    pub(crate) extern "x86-interrupt" fn error_interrupt_handler(
        _stack_frame: InterruptStackFrame,
    ) {
        early_println!("error interrupt");
        end_interrupt();
    }
}
