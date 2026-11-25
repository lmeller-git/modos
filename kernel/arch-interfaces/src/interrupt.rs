//! Architecture defined interrupt handling including local interrupt state and interrupt delivery.

pub use irq_interfaces::{Affinity, IrqError, IrqNum, IrqTarget};

/// State of interrupt delivery on the calling CPU.
pub trait InterruptHandlerCapability {
    fn enable();
    fn disable();
    fn is_enabled() -> bool;
    fn without_interrupts<R>(f: impl FnOnce() -> R) -> R;
}

/// API for delivering interrupts to a cpu core.
pub trait IrqDeliveryCapability {
    /// Number of delivery vectors per cpu
    const MAX_VECTORS: usize;

    /// Installs the cores entry point, called with the [`IrqTarget`].
    /// Must be called once, before the first target is handed out.
    fn set_entry(entry: fn(IrqTarget));

    /// Reserves a free delivery target on a CPU allowed by `affinity`.
    fn alloc_target(affinity: Affinity) -> Result<IrqTarget, IrqError>;

    /// Releases `target` for reuse. Only valid once no CPU can still be inside the entry for it.
    fn free_target(target: IrqTarget);
}
