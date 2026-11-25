//! Line uniqueness for irq handlers.
//!
//! The platform keeps no claim state, so the core guarantees that a line is registered once. The
//! lock is only held for the insert/remove itself, never across arch, platform or waiting calls.

use alloc::collections::BTreeSet;

use arch::{TargetArch, interface::interrupt::InterruptHandlerCapability};
use spin::Mutex;

use crate::IrqError;

static CLAIMED: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

/// Exclusive claim on a hardware line. Released on drop.
#[derive(Debug)]
pub(crate) struct Claim(u32);

impl Claim {
    /// Claims `hwirq`.
    pub(crate) fn new(hwirq: u32) -> Result<Self, IrqError> {
        let inserted = TargetArch::without_interrupts(|| CLAIMED.lock().insert(hwirq));
        if inserted {
            Ok(Self(hwirq))
        } else {
            Err(IrqError::AlreadyClaimed)
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        TargetArch::without_interrupts(|| {
            CLAIMED.lock().remove(&self.0);
        });
    }
}
