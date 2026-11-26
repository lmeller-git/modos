//! Irq descriptor table.
//!
//! Irqs are named by `cpu * MAX_VECTORS + vector`, with exclusivity guaranteed by arch allocator.

use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use arch::{TargetArch, interface::interrupt::IrqDeliveryCapability};
use core_local::CLSLazyPtr;

use crate::{CpuId, Desc, IrqError, IrqNum, IrqTarget, sync::ReadGuard};

const VECTORS: usize = <TargetArch as IrqDeliveryCapability>::MAX_VECTORS;

type Table = [AtomicPtr<Desc>; VECTORS];

/// This CPU's slots, by vector. Written by whoever owns the vector, read lock-free by `entry`.
#[core_local::core_local(remote)]
static IRQ_TABLE: CLSLazyPtr<Table> = CLSLazyPtr::new();

/// The number naming the interrupt delivered at `target`.
///
/// Panics if `target.vector` is not a vector of its CPU.
pub(crate) fn irq_of(target: IrqTarget) -> IrqNum {
    let vector = target.vector as usize;
    assert!(vector < VECTORS, "vector {vector} out of range");

    let raw = target
        .cpu
        .value()
        .checked_mul(VECTORS)
        .and_then(|base| u32::try_from(base + vector).ok())
        .expect("irq number space exhausted");
    IrqNum::new(raw)
}

/// The delivery target `irq` names.
pub(crate) fn target_of(irq: IrqNum) -> IrqTarget {
    let raw = irq.raw() as usize;
    IrqTarget {
        cpu: CpuId::new(raw / VECTORS),
        vector: (raw % VECTORS) as u32,
    }
}

/// The slot of `target`, creating its CPU's table if needed.
///
/// Only the owner of `target` may write the slot.
pub(crate) fn slot(target: IrqTarget) -> Result<&'static AtomicPtr<Desc>, IrqError> {
    let table = IRQ_TABLE
        .on(target.cpu)
        .ok_or(IrqError::NotInitialized)?
        .get_or_init(|| {
            alloc::boxed::Box::new([const { AtomicPtr::new(ptr::null_mut()) }; VECTORS])
        });
    table
        .get(target.vector as usize)
        .ok_or(IrqError::InvalidTarget)
}

/// The descriptor published for `vector` on this CPU.
///
/// The descriptor stays valid for the lifetime of the read section: it is only freed after
/// `synchronize` saw this CPU leave it.
pub(crate) fn get<'a>(_section: &'a ReadGuard<'_>, vector: u32) -> Option<&'a Desc> {
    let slot = IRQ_TABLE.get()?.get(vector as usize)?;
    unsafe { slot.load(Ordering::Acquire).as_ref() }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::{config, panics};

    use super::*;

    /// Highest cpu id whose irq numbers still fit a `u32`.
    const MAX_CPU: usize = u32::MAX as usize / VECTORS;

    mod irq_num {
        use super::*;

        proptest! {
            #![proptest_config(config())]

            /// Targets and numbers are in bijection.
            #[test]
            fn target_roundtrip(
                cpu in prop_oneof![0..64usize, MAX_CPU - 64..=MAX_CPU],
                vector in 0..VECTORS as u32,
                raw in any::<u32>(),
            ) {
                let target = IrqTarget { cpu: CpuId::new(cpu), vector };
                prop_assert_eq!(target_of(irq_of(target)), target);

                let irq = IrqNum::new(raw);
                prop_assert_eq!(irq_of(target_of(irq)), irq);
            }
        }

        /// Targets with no irq number panic instead of aliasing another target's number.
        #[test]
        fn unnameable_targets() {
            let target = |cpu, vector| IrqTarget {
                cpu: CpuId::new(cpu),
                vector,
            };
            for t in [
                target(0, VECTORS as u32),
                target(0, u32::MAX),
                target(MAX_CPU, VECTORS as u32),
                target(MAX_CPU + 1, 0),
                target(usize::MAX, 0),
            ] {
                assert!(panics(|| irq_of(t)), "{t:?}");
            }
        }
    }
}
