use derive_more::{Binary, Display, LowerHex, Octal, UpperHex};

#[derive(
    Clone,
    Copy,
    Debug,
    Display,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Binary,
    Octal,
    LowerHex,
    UpperHex,
)]
#[repr(transparent)]
pub struct CpuId(u32);

impl CpuId {
    pub fn value(&self) -> u32 {
        self.0
    }
}

pub trait CpuHandlingCapability {
    fn core_init();
    fn cpu_count() -> u32 {
        1
    }

    fn bootstrap_cpu() -> Option<CpuId> {
        None
    }

    fn is_bootstrap_cpu() -> bool {
        true
    }

    fn current_cpu() -> CpuId {
        CpuId(0)
    }

    fn cpus() -> impl Iterator<Item = CpuId> {
        core::iter::once(Self::current_cpu())
    }
}
