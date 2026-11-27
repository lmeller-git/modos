//! Backend agnostic API for irq handlers.
//!
//! This crate owns the policy which is composed up from arch and platform components.
//!
//! # Dataflow
//!
//! **Request** ([`request_irq`]):
//! 1. Claim the requested line.
//! 2. Arch: `alloc_target`. This names the interrupt and its slot.
//! 3. Platform: `route(spec, target)`, leaving the line masked.
//! 4. Publish the descriptor `{spec, handler}` in its slot.
//!
//! **Dispatch** ([`entry`], lock-free): the arch calls `entry` with the cpu and vector an
//! interrupt arrived at, then sends EOI.
//! 1. An empty slot is a spurious interrupt: return.
//! 2. A level-triggered line is masked first.
//! 3. `handler.handle(irq)`.
//! 4. `Handled` or `NotMine` unmasks a level line; `Defer` leaves it masked for the owner.
//!
//! **Free** (`Drop` of [`IrqHandle`], blocks):
//! 1. Platform `mask`.
//! 2. Clear the slot.
//! 3. Wait until the target cpu is no longer inside `entry`.
//! 4. Platform `unroute`.
//! 5. Drop the descriptor (handler, line claim), then arch `free_target`.
//!
//! Freeing must not happen in interrupt context or from the handler itself.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use core::{ptr, sync::atomic::Ordering};

use arch::TargetArch;
pub use irq_interfaces::{
    Affinity,
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

#[cfg(test)]
extern crate std;

mod claims;
mod guard;
mod slots;
mod sync;

use claims::Claim;
pub use guard::MaskGuard;

/// A registered interrupt, published in its slot (see [`slots`]).
struct Desc {
    spec: IrqSpec,
    handler: Box<dyn IrqHandler>,
    /// Released with the descriptor.
    _claim: Claim,
}

/// Installs the core's entry point into the architecture.
pub fn init() {
    use arch::interface::interrupt::IrqDeliveryCapability;
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
    use arch::interface::interrupt::IrqDeliveryCapability;
    use platform::interface::interrupt::InterruptControllerCapability;

    let claim = Claim::new(spec.hwirq)?;

    let target = TargetArch::alloc_target(affinity)?;
    let slot = match slots::slot(target) {
        Ok(slot) => slot,
        Err(err) => {
            TargetArch::free_target(target);
            return Err(err);
        }
    };
    // The line stays masked until the owner unmasks it, so nothing can be delivered yet.
    if let Err(err) = TargetPlatform::route(spec, target) {
        TargetArch::free_target(target);
        return Err(err);
    }

    let desc = Box::into_raw(Box::new(Desc {
        spec,
        handler,
        _claim: claim,
    }));
    debug_assert!(slot.load(Ordering::Relaxed).is_null());
    slot.store(desc, Ordering::Release);
    Ok(IrqHandle {
        irq: slots::irq_of(target),
        spec,
    })
}

/// Upcall target for the architecture: dispatches one delivered interrupt.
#[doc(hidden)]
pub fn entry(target: IrqTarget) {
    use platform::interface::interrupt::InterruptControllerCapability;

    let section = sync::enter();

    // Only the cpu a vector is routed to can receive it.
    if let Some(desc) = slots::get(&section, target.vector) {
        let irq = slots::irq_of(target);
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
        use arch::interface::interrupt::{InterruptHandlerCapability, IrqDeliveryCapability};
        use platform::interface::interrupt::InterruptControllerCapability;

        TargetArch::without_interrupts(|| {
            debug_assert!(
                !sync::is_inside(),
                "an IrqHandle was dropped from interrupt context"
            );

            let target = slots::target_of(self.irq);
            let slot = slots::slot(target).expect("a registered interrupt has a slot");

            TargetPlatform::mask(self.spec);

            // Unpublish, then wait for the one cpu that could still be using the descriptor.
            let desc = slot.swap(ptr::null_mut(), Ordering::AcqRel);
            sync::synchronize(target.cpu);

            // Only now is the line final: a reader that was still inside `entry` may have
            // unmasked it.
            TargetPlatform::unroute(self.spec);

            // SAFETY: allocated by `request_irq`, owned by this handle; no CPU can still reach it.
            drop(unsafe { Box::from_raw(desc) });
            TargetArch::free_target(target);
        });
    }
}
