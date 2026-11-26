use alloc::boxed::Box;
use core::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

use arch_interfaces::{
    cpu::CpuDataCapability,
    interrupt::{Affinity, IrqDeliveryCapability, IrqError, IrqTarget},
};
use core_local::CLSLazyPtr;
use early_log::early_println;
use spin::Mutex;
use x86_64::{
    instructions::interrupts,
    structures::idt::{InterruptStackFrame, PageFaultErrorCode},
};

use crate::{X86_64, apic::lapic::end_interrupt};

// TODO: this should also be core_local such that different entries can be installed per core, which might be cool in some usecases.
// currently blocked by the init sequence.
// Note that we can use a CLSLazyPtr anyways, as it is functionalyy just a oncecell.
/// Each cores entry point, installed by `irq`.
static ENTRY: CLSLazyPtr<fn(IrqTarget)> = CLSLazyPtr::new();

// TODO: use an atomic map
/// Vectors handed out by [`IrqDeliveryCapability::alloc_target`].
#[core_local::core_local(remote)]
static VECTORS: CLSLazyPtr<Mutex<VectorMap>> = CLSLazyPtr::new();

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

/// initializes arch backend irq handlers on this core
pub(crate) fn init() {
    VECTORS.get_or_init(|| Box::new(VectorMap::new().into()));
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
    if let Some(entry) = ENTRY.get() {
        entry(IrqTarget {
            cpu: X86_64::my_cpu().logical_id,
            vector: u32::from(VEC),
        });
    }
    end_interrupt();
}

impl IrqDeliveryCapability for X86_64 {
    const MAX_VECTORS: usize = 256;

    fn set_entry(entry: fn(IrqTarget)) {
        ENTRY.get_or_init(|| Box::new(entry));
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
        let vector = VECTORS.get_on(cpu).ok_or(IrqError::NotInitialized)?;
        let vector =
            interrupts::without_interrupts(|| vector.lock().alloc()).ok_or(IrqError::NoFreeIrq)?;
        Ok(IrqTarget {
            cpu,
            vector: u32::from(vector),
        })
    }

    fn free_target(target: IrqTarget) {
        interrupts::without_interrupts(|| {
            VECTORS
                .get_on(target.cpu)
                .unwrap()
                .lock()
                .free(target.vector as u8);
        });
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

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeSet, vec::Vec};

    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    /// `0x22..0xFF` is 221 vectors, minus the 16 in `0xE0..=0xEF`.
    const EXTERNAL: usize = 205;

    #[test]
    fn external_vectors() {
        let boundaries = [
            (0x00, false),
            (0x20, false),
            (0x21, false),
            (0x22, true),
            (0xDF, true),
            (0xE0, false),
            (0xEF, false),
            (0xF0, true),
            (0xFE, true),
            (0xFF, false),
        ];
        for (vector, external) in boundaries {
            assert_eq!(is_external(vector), external, "{vector:#x}");
        }
        assert_eq!((0..256).filter(|&v| is_external(v)).count(), EXTERNAL);
    }

    mod vector_map {
        use super::*;

        #[cfg(any(miri, loom, shuttle))]
        const OPS: usize = 40;
        #[cfg(not(any(miri, loom, shuttle)))]
        const OPS: usize = 600;

        #[derive(Clone, Copy, Debug)]
        enum Op {
            Alloc,
            /// Frees one of the allocated vectors.
            Free(prop::sample::Index),
            /// Frees a vector nobody holds: reserved, or currently free.
            FreeUnowned(u8),
        }

        fn ops() -> impl Strategy<Value = Vec<Op>> {
            let op = prop_oneof![
                4 => Just(Op::Alloc),
                3 => any::<prop::sample::Index>().prop_map(Op::Free),
                1 => any::<u8>().prop_map(Op::FreeUnowned),
            ];
            prop::collection::vec(op, 0..OPS)
        }

        #[test]
        fn exhaustion() {
            let mut map = VectorMap::new();
            let all: Vec<u8> = core::iter::from_fn(|| map.alloc()).collect();

            let external: Vec<u8> = (0..=255u8).filter(|&v| is_external(v.into())).collect();
            assert_eq!(all, external);
            assert_eq!(map.alloc(), None);
        }

        proptest! {
            #![proptest_config(config())]

            /// `alloc` returns the lowest free external vector and `free` returns an allocated
            /// one; freeing a vector nobody holds changes nothing.
            #[test]
            fn matches_set_model(ops in ops()) {
                let mut map = VectorMap::new();
                let mut free: BTreeSet<u8> =
                    (0..=255u8).filter(|&v| is_external(v.into())).collect();
                let mut held: Vec<u8> = Vec::new();

                for op in ops {
                    match op {
                        Op::Alloc => {
                            let vector = map.alloc();
                            prop_assert_eq!(vector, free.pop_first());
                            held.extend(vector);
                        }
                        Op::Free(index) if !held.is_empty() => {
                            let vector = held.swap_remove(index.index(held.len()));
                            map.free(vector);
                            free.insert(vector);
                        }
                        Op::Free(_) => {}
                        Op::FreeUnowned(vector) if !held.contains(&vector) => map.free(vector),
                        Op::FreeUnowned(_) => {}
                    }
                }
                // Whatever the history, the remaining vectors come out in order.
                let rest: Vec<u8> = core::iter::from_fn(|| map.alloc()).collect();
                prop_assert_eq!(rest, free.into_iter().collect::<Vec<_>>());
            }
        }
    }
}
