//! Higher-half direct map: all physical memory mapped at one fixed virtual offset.

use core::{num::NonZeroUsize, ptr::NonNull};

use derive_more::{Display, LowerHex, UpperHex};

use crate::addr::{PhysAddr, VirtAddr};

/// Translates between physical addresses and their HHDM virtual addresses.
#[derive(Clone, Copy, Debug, Display, PartialEq, Eq, Hash, LowerHex, UpperHex)]
#[repr(transparent)]
pub struct Hhdm(NonZeroUsize);

impl Hhdm {
    /// # Safety
    ///
    /// `offset` must be the HHDM offset of the running system: every physical address the kernel
    /// accesses through this `Hhdm` must be mapped at `offset + phys`.
    pub const unsafe fn new(offset: NonZeroUsize) -> Self {
        Self(offset)
    }

    pub const fn offset(self) -> usize {
        self.0.get()
    }

    /// The virtual address of `phys`, or `None` if it lies past the top of the address space.
    pub fn to_virt(self, phys: PhysAddr) -> Option<VirtAddr> {
        self.ptr::<u8>(phys)
            .map(|ptr| VirtAddr::new(ptr.addr().get()))
    }

    /// Pointer to `phys` through the HHDM. Non-null because the offset is non-zero.
    pub fn ptr<T>(self, phys: PhysAddr) -> Option<NonNull<T>> {
        let addr = self.0.checked_add(phys.as_usize())?;
        Some(NonNull::with_exposed_provenance(addr))
    }

    /// The physical address `virt` maps, or `None` if it lies below the HHDM.
    pub fn to_phys(self, virt: VirtAddr) -> Option<PhysAddr> {
        virt.as_usize()
            .checked_sub(self.offset())
            .map(PhysAddr::new)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_host::config;

    use super::*;

    proptest! {
        #![proptest_config(config())]

        /// `to_virt` adds the offset without wrapping and `to_phys` inverts it; addresses below the
        /// map have no physical address.
        #[test]
        fn translation(
            offset in prop_oneof![1..=usize::MAX, Just(usize::MAX), Just(1usize)],
            raw in prop_oneof![any::<usize>(), 0..0x1000usize, usize::MAX - 0x1000..=usize::MAX],
        ) {
            // SAFETY: never dereferenced.
            let hhdm = unsafe { Hhdm::new(NonZeroUsize::new(offset).unwrap()) };

            let virt = hhdm.to_virt(PhysAddr::new(raw));
            prop_assert_eq!(virt.map(|v| v.as_usize()), offset.checked_add(raw));
            if let Some(virt) = virt {
                prop_assert_eq!(hhdm.to_phys(virt), Some(PhysAddr::new(raw)));
            }
            let expected = raw.checked_sub(offset).map(PhysAddr::new);
            prop_assert_eq!(hhdm.to_phys(VirtAddr::new(raw)), expected);
        }
    }
}
