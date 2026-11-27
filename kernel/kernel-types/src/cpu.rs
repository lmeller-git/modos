/// Logical ID of a CPU core.
///
/// The ids of a system with `n` cores are exactly `0..n`.
// TODO: the above is not enforced by this type currently and is checked by consumers instead. This is kinda bad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct CpuId(usize);

impl CpuId {
    /// Returns the raw logical id.
    pub const fn value(self) -> usize {
        self.0
    }

    /// Constructs a [`CpuId`] from a raw logical id.
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }
}
