//! Architecture defined interrupt handling including local interrupt state and interrupt delivery.

pub use irq_interfaces::{Affinity, IrqError, IrqNum, IrqTarget};

/// State of interrupt delivery on the calling CPU.
pub trait InterruptHandlerCapability {
    fn enable();
    fn disable();
    fn is_enabled() -> bool;
    fn without_interrupts<R>(f: impl FnOnce() -> R) -> R;
}

/// API for routing interrupts on a cpu core.
///
/// The arch backend should map from [`IrqTarget`] to [`IrqNum`] as well as handle dispatching and ending interrupts.
pub trait IrqDeliveryCapability {
    /// Installs the core's entry point, called with the bound [`IrqNum`] for every delivered
    /// interrupt. Must be called once, before the first target is bound.
    fn set_entry(entry: fn(IrqNum));

    /// Reserves a free delivery target on a CPU allowed by `affinity`.
    fn alloc_target(affinity: Affinity) -> Result<IrqTarget, IrqError>;

    /// Makes interrupts delivered at `target` call the entry with `irq`.
    fn bind(target: IrqTarget, irq: IrqNum);

    /// Stops delivering `target` to the entry. Does not wait for an entry already running.
    fn unbind(target: IrqTarget);

    /// Releases `target` for reuse. Only valid after [`Self::unbind`] and once no CPU is still
    /// inside the entry for it.
    fn free_target(target: IrqTarget);
}
