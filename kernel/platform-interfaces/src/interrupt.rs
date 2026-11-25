//! Platform interrupt-controller capability.
//!
//! Interface as needed by the `irq` crate.

pub use irq_interfaces::{CpuId, IrqError, IrqSpec, IrqTarget, Polarity, TriggerMode};

/// Controller-side interrupt routing
///
/// The `irq` core guarantees line uniqueness and only calls `mask`/`unmask`/`unroute` on lines it
/// routed and still owns, so these operations are infallible and keep no claim state of their own.
pub trait InterruptControllerCapability {
    /// Routes `spec` to `target`, leaving the line masked.
    ///
    /// Fails with [`IrqError::NoController`] if no controller covers the line,
    /// [`IrqError::AlreadyClaimed`] if firmware reserved it, and [`IrqError::InvalidTarget`] if
    /// the target cannot be programmed.
    fn route(spec: IrqSpec, target: IrqTarget) -> Result<(), IrqError>;

    /// Masks a routed line. Callable from interrupt context.
    fn mask(spec: IrqSpec);

    /// Unmasks a routed line. Callable from interrupt context.
    fn unmask(spec: IrqSpec);

    /// Masks the line and parks it on a harmless target, so it can be routed again.
    fn unroute(spec: IrqSpec);
}
