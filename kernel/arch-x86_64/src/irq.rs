use core::{
    ops::Range,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

use arch_interfaces::{
    cpu::CpuDataCapability,
    interrupt::{Affinity, IrqDeliveryCapability, IrqError, IrqNum, IrqTarget},
};
use early_log::early_println;
use spin::{Mutex, Once};
use x86_64::{
    instructions::interrupts,
    structures::idt::{InterruptStackFrame, PageFaultErrorCode},
};

use crate::{X86_64, apic::lapic::end_interrupt};

/// Marks a vector no irq is bound to.
const UNBOUND: u32 = u32::MAX;

// TODO: ENTRY and VECTOR_IRQ, ... should be core_local. Needs cross core core_local access.

/// Vector -> irq. Currently globa.
static VECTOR_IRQ: [AtomicU32; 256] = [const { AtomicU32::new(UNBOUND) }; 256];

/// The core's entry point, installed once by `irq`.
static ENTRY: Once<fn(IrqNum)> = Once::new();

// TODO: use an atomic map
/// Vectors handed out by [`IrqDeliveryCapability::alloc_target`]. Currently global like [`VECTOR_IRQ`].
static VECTORS: Mutex<VectorMap> = Mutex::new(VectorMap::new());

/// [`Affinity::Any`] scheduler.
static NEXT_CPU: AtomicUsize = AtomicUsize::new(0);

/// First and last vector available to external interrupts.
/// `0x00..=0x1F`: exceptions,
/// `0x20`/`0x21`: local APIC timer / error entries,
/// `0xFF`: spurious entry.
const EXTERNAL_RANGE: Range<usize> = 0x22..0xFF;

/// valid external vector.
const fn is_external(vector: usize) -> bool {
    EXTERNAL_RANGE.contains(&vector) && !(vector >= 0xE0 && vector <= 0xEF)
}

/// Bitmap of vectors. A set bit is in use or not available to external interrupts.
struct VectorMap([u64; 4]);

impl VectorMap {
    const fn new() -> Self {
        let mut bits = [0u64; 4];
        let mut vector = 0;
        while vector < 256 {
            if !is_external(vector) {
                bits[vector / 64] |= 1 << (vector % 64);
            }
            vector += 1;
        }
        Self(bits)
    }

    fn alloc(&mut self) -> Option<u8> {
        for (index, word) in self.0.iter_mut().enumerate() {
            if *word != u64::MAX {
                let bit = word.trailing_ones() as usize;
                *word |= 1 << bit;
                return Some((index * 64 + bit) as u8);
            }
        }
        None
    }

    fn free(&mut self, vector: u8) {
        let vector = usize::from(vector);
        if is_external(vector) {
            self.0[vector / 64] &= !(1 << (vector % 64));
        }
    }
}

/// IDT stub for external vector `VEC`: forward to the core, then send EOI.
pub(crate) extern "x86-interrupt" fn irq_dispatch<const VEC: u8>(_f: InterruptStackFrame) {
    match VECTOR_IRQ[VEC as usize].load(Ordering::Acquire) {
        UNBOUND => early_println!("[irq] unbound vector {:#x}", VEC),
        raw => {
            if let Some(entry) = ENTRY.get() {
                entry(IrqNum::new(raw));
            }
        }
    }
    end_interrupt();
}

impl IrqDeliveryCapability for X86_64 {
    fn set_entry(entry: fn(IrqNum)) {
        ENTRY.call_once(|| entry);
    }

    /// Picks a CPU allowed by `affinity` and a free vector for it.
    fn alloc_target(affinity: Affinity) -> Result<IrqTarget, IrqError> {
        let cpu = match affinity {
            Affinity::Cpu(cpu) if Self::cpus().any(|id| id == cpu) => cpu,
            Affinity::Cpu(_) => return Err(IrqError::InvalidTarget),
            Affinity::Any => Self::cpus()
                .nth(NEXT_CPU.fetch_add(1, Ordering::Relaxed) % Self::cpu_count().max(1))
                .ok_or(IrqError::InvalidTarget)?,
        };
        // TODO: this lock should either be a no-block lock or no lock at all.
        let vector =
            interrupts::without_interrupts(|| VECTORS.lock().alloc()).ok_or(IrqError::NoFreeIrq)?;
        Ok(IrqTarget {
            cpu,
            vector: u32::from(vector),
        })
    }

    fn bind(target: IrqTarget, irq: IrqNum) {
        VECTOR_IRQ[target.vector as usize].store(irq.raw(), Ordering::Release);
    }

    fn unbind(target: IrqTarget) {
        VECTOR_IRQ[target.vector as usize].store(UNBOUND, Ordering::Release);
    }

    fn free_target(target: IrqTarget) {
        // TODO: this lock should either be a no-block lock or no lock at all.
        interrupts::without_interrupts(|| VECTORS.lock().free(target.vector as u8));
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

    pub(crate) extern "x86-interrupt" fn nmi_handler(stack_frame: InterruptStackFrame) {
        panic!("EXCEPTION: NON-MASKABLE INTERRUPT\n{stack_frame:#?}");
    }

    pub(crate) extern "x86-interrupt" fn timer_interrupt(_stack_frame: InterruptStackFrame) {
        TOTAL_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        end_interrupt();
    }

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
        let flags = crate::apic::lapic::take_error_flags();
        early_println!("[apic] error interrupt: {:?}", flags);
        end_interrupt();
    }
}
