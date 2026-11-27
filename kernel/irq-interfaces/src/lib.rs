//! backend-independent interrupt interfaces and types.
//!
//! - `arch-interfaces` defines the CPU side (`IrqDeliveryCapability`): where an interrupt arrives.
//! - `platform-interfaces` defines the controller side (`InterruptControllerCapability`): how a
//!   line is routed to that place and masked.
//! - `irq` is the generic core and the API drivers should use.
//!
//! This crate also contains some backend specific types.

#![no_std]

#[cfg(target_arch = "x86_64")]
extern crate alloc;

#[cfg(target_arch = "x86_64")]
pub mod x86;

use kernel_types::cpu::CpuId;

/// How the interrupt source is triggered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerMode {
    Edge,
    Level,
}

/// The polarity of the interrupt source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow,
}

/// Names a hardware interrupt line and how it is signalled.
///
/// `hwirq` is a flat, platform-defined line number (on ACPI platforms: the GSI).
// TODO: A domain id will  be added once a second controller class (MSI, ...) exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqSpec {
    pub hwirq: u32,
    pub trigger: TriggerMode,
    pub polarity: Polarity,
}

/// Identifies a registered interrupt in the `irq` core.
///
/// Assigned by the core and handed to the handler. It names, but grants no power: masking and
/// freeing require the owning `IrqHandle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct IrqNum(u32);

impl IrqNum {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Where the CPU side receives an interrupt: what a controller has to program for a line.
///
/// `vector` is the architecture's native delivery id (x86 vector, GIC INTID, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqTarget {
    pub cpu: CpuId,
    pub vector: u32,
}

/// Which CPU may receive an interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Any,
    Cpu(CpuId),
}

/// What a handler reports back to the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqReturn {
    /// The device did not raise this interrupt (shared or spurious delivery).
    NotMine,
    /// The interrupt was serviced.
    Handled,
    /// Serviced the top half; keep a level-triggered source masked until the owner calls
    /// `IrqHandle::unmask`. Equal to [`Self::Handled`] for edge sources.
    Defer,
}

/// A top-half interrupt handler.
///
/// Runs in interrupt context on the receiving CPU: it must be short, must not block, and must not
/// free its own `IrqHandle`. It must tolerate being called with no work pending (a late interrupt
/// can reach a recycled vector) and return [`IrqReturn::NotMine`].
///
/// The core owns masking and end-of-interrupt: a level-triggered source is masked before `handle`
/// and unmasked after unless the handler returns [`IrqReturn::Defer`].
pub trait IrqHandler: Send + Sync {
    fn handle(&self, irq: IrqNum) -> IrqReturn;
}

/// Errors reported by the interrupt stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqError {
    /// No free irq slot or delivery vector is left.
    NoFreeIrq,
    /// The line is already in use, or reserved by firmware.
    AlreadyClaimed,
    /// No interrupt controller covers the line.
    NoController,
    /// The target cannot be programmed into the controller.
    InvalidTarget,
    /// The interrupt stack has not been initialised.
    NotInitialized,
    /// The backend does not support the request.
    Unsupported,
}

#[cfg(test)]
extern crate std;
