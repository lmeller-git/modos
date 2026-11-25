//! Scoped guards.

use crate::IrqHandle;

/// Keeps an interrupt masked; unmasks it on drop.
#[must_use = "the interrupt is unmasked again when the guard is dropped"]
#[derive(Debug)]
pub struct MaskGuard<'a>(&'a IrqHandle);

impl<'a> MaskGuard<'a> {
    pub(crate) fn new(handle: &'a IrqHandle) -> Self {
        Self(handle)
    }
}

impl Drop for MaskGuard<'_> {
    fn drop(&mut self) {
        self.0.unmask();
    }
}
