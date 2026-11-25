//! The interrupt core: the one backend-agnostic API drivers use.
//!
//! The core owns the descriptor table, line uniqueness, dispatch policy and teardown. Architecture
//! and platform only provide mechanism through `IrqDeliveryCapability` and
//! `InterruptControllerCapability`, so nothing here names a concrete backend.
//!
//! # Dataflow
//!
//! **Request** ([`request_irq`]), serialised by one control-path lock with local interrupts off:
//! 1. Take a free [`IrqNum`] slot and reject a duplicate `spec` ([`IrqError::AlreadyClaimed`]).
//! 2. Arch: `alloc_target`, then `bind(target, irq)`.
//! 3. Platform: `route(spec, target)`, leaving the line masked. Roll back 2 and 1 on failure.
//! 4. Publish the descriptor `{spec, target, handler}` in its slot.
//!
//! The returned [`IrqHandle`] is masked; the driver unmasks it once its device is ready.
//!
//! **Dispatch** ([`entry`], lock-free): the arch maps the delivered vector to an [`IrqNum`] and calls
//! `entry`, then sends EOI.
//! 1. An empty slot is a spurious interrupt: return.
//! 2. A level-triggered line is masked first.
//! 3. `handler.handle(irq)`.
//! 4. `Handled` or `NotMine` unmasks a level line; `Defer` leaves it masked for the owner.
//!
//! **Free** (`Drop` of [`IrqHandle`], blocks):
//! 1. Platform `mask`; arch `unbind`.
//! 2. Clear the slot.
//! 3. Wait until no CPU is still inside `entry` (per-CPU in-IRQ sequence numbers).
//! 4. Platform `unroute`. This must come after the wait: a handler still running on a level line
//!    unmasks it when it returns, which would otherwise undo the parking.
//! 5. Drop the handler, then arch `free_target`.
//!
//! Freeing must not happen in interrupt context or from the handler itself.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use arch::TargetArch;
pub use irq_interfaces::{
    Affinity,
    CpuId,
    IrqError,
    IrqHandler,
    IrqNum,
    IrqReturn,
    IrqSpec,
    IrqTarget,
    Polarity,
    TriggerMode,
    x86,
};
use platform::TargetPlatform;
use spin::Mutex;

mod guard;
mod sync;

pub use guard::MaskGuard;
use sync::ReadTracker;

/// Upper bound on registered interrupts.
const MAX_IRQS: usize = 256;
/// Upper bound on CPUs tracked by [`READERS`].
const MAX_CPUS: usize = 256;

/// A registered interrupt, published in [`SLOTS`] at its [`IrqNum`].
struct Desc {
    spec: IrqSpec,
    target: IrqTarget,
    handler: Box<dyn IrqHandler>,
}

/// The descriptor table. A slot is written only under [`CONTROL`]; [`entry`] reads it lock-free.
static SLOTS: [AtomicPtr<Desc>; MAX_IRQS] = [const { AtomicPtr::new(ptr::null_mut()) }; MAX_IRQS];

/// Serialises request and free.
static CONTROL: Mutex<()> = Mutex::new(());

/// CPUs currently inside [`entry`].
static READERS: ReadTracker<MAX_CPUS> = ReadTracker::new();

fn this_cpu() -> usize {
    use arch::interface::cpu::CpuDataCapability;
    TargetArch::my_cpu().logical_id.value()
}

/// Installs the core's entry point into the architecture.
pub fn init() {
    use arch::interface::{cpu::CpuDataCapability, interrupt::IrqDeliveryCapability};
    assert!(
        TargetArch::cpu_count() <= MAX_CPUS,
        "more cpus than the irq core can track"
    );
    TargetArch::set_entry(entry);
}

/// Registers `handler` for the line `spec` and returns its owner handle, masked.
///
/// The handler is owned by the core until the handle is dropped.
pub fn request_irq(
    spec: IrqSpec,
    handler: Box<dyn IrqHandler>,
    affinity: Affinity,
) -> Result<IrqHandle, IrqError> {
    use arch::interface::interrupt::{InterruptHandlerCapability, IrqDeliveryCapability};
    use platform::interface::interrupt::InterruptControllerCapability;

    TargetArch::without_interrupts(|| {
        let _control = CONTROL.lock();

        let mut free = None;
        for (index, slot) in SLOTS.iter().enumerate() {
            // SAFETY: descriptors are only freed under `CONTROL`, which we hold.
            match unsafe { slot.load(Ordering::Acquire).as_ref() } {
                Some(desc) if desc.spec.hwirq == spec.hwirq => {
                    return Err(IrqError::AlreadyClaimed);
                }
                Some(_) => {}
                None => {
                    free.get_or_insert(index);
                }
            }
        }
        let index = free.ok_or(IrqError::NoFreeIrq)?;

        let target = TargetArch::alloc_target(affinity)?;
        TargetArch::bind(target, IrqNum::new(index as u32));
        // The line stays masked until the owner unmasks it, so nothing can be delivered yet.
        if let Err(err) = TargetPlatform::route(spec, target) {
            TargetArch::unbind(target);
            TargetArch::free_target(target);
            return Err(err);
        }

        let desc = Box::into_raw(Box::new(Desc {
            spec,
            target,
            handler,
        }));
        SLOTS[index].store(desc, Ordering::Release);
        Ok(IrqHandle {
            irq: IrqNum::new(index as u32),
            spec,
        })
    })
}

/// Upcall target for the architecture: dispatches one delivered interrupt.
#[doc(hidden)]
pub fn entry(irq: IrqNum) {
    use platform::interface::interrupt::InterruptControllerCapability;

    let _reader = READERS.enter(this_cpu());

    let slot = SLOTS.get(irq.raw() as usize);
    // SAFETY: a descriptor is only freed after `synchronize` saw this CPU leave `entry`.
    if let Some(desc) = slot.and_then(|slot| unsafe { slot.load(Ordering::Acquire).as_ref() }) {
        let level = desc.spec.trigger == TriggerMode::Level;
        if level {
            TargetPlatform::mask(desc.spec);
        }
        let ret = desc.handler.handle(irq);
        if level && ret != IrqReturn::Defer {
            TargetPlatform::unmask(desc.spec);
        }
    }
}

/// Ownership of one registered interrupt.
///
/// Dropping the handle frees the interrupt, which blocks until no CPU is running its handler.
#[must_use = "dropping the handle frees the interrupt"]
#[derive(Debug)]
pub struct IrqHandle {
    irq: IrqNum,
    spec: IrqSpec,
}

impl IrqHandle {
    /// The number the handler is called with.
    pub fn irq(&self) -> IrqNum {
        self.irq
    }

    /// Masks the source. Masks are not counted.
    pub fn mask(&self) {
        use platform::interface::interrupt::InterruptControllerCapability;
        TargetPlatform::mask(self.spec);
    }

    /// Unmasks the source. Also releases a masked line left by [`IrqReturn::Defer`].
    pub fn unmask(&self) {
        use platform::interface::interrupt::InterruptControllerCapability;
        TargetPlatform::unmask(self.spec);
    }

    /// Masks the source until the returned guard is dropped.
    pub fn masked(&self) -> MaskGuard<'_> {
        self.mask();
        MaskGuard::new(self)
    }
}

impl Drop for IrqHandle {
    fn drop(&mut self) {
        use arch::interface::{
            cpu::CpuDataCapability,
            interrupt::{InterruptHandlerCapability, IrqDeliveryCapability},
        };
        use platform::interface::interrupt::InterruptControllerCapability;

        TargetArch::without_interrupts(|| {
            assert!(
                !READERS.is_inside(this_cpu()),
                "an IrqHandle must not be dropped from interrupt context"
            );
            let _control = CONTROL.lock();

            let slot = &SLOTS[self.irq.raw() as usize];
            let desc = slot.load(Ordering::Acquire);
            // SAFETY: this handle is the descriptor's only owner, and it is freed only below.
            let (spec, target) = unsafe { ((*desc).spec, (*desc).target) };

            TargetPlatform::mask(spec);
            TargetArch::unbind(target);

            slot.store(ptr::null_mut(), Ordering::Release);
            READERS.synchronize(TargetArch::cpus().map(CpuId::value));

            // Only now is the line final: a reader that was still inside `entry` may have
            // unmasked it.
            TargetPlatform::unroute(spec);

            // SAFETY: allocated by `request_irq`; no CPU can still reach it.
            drop(unsafe { Box::from_raw(desc) });
            TargetArch::free_target(target);
        });
    }
}
