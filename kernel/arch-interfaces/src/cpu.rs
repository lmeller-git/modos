use boot::{LateBootInfo, mp::BootCpu};
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
pub struct CpuId(pub usize);

impl CpuId {
    pub fn value(&self) -> usize {
        self.0
    }
}

pub trait CpuDataCapability {
    fn cpu_count() -> usize;
    fn is_bootstrap_cpu() -> bool;
    fn cpus() -> impl Iterator<Item = CpuId>;
    fn my_cpu() -> BootCpu;
}

pub trait SmpCapability {
    type InitToken;
    /// Consumes the init phase. No more init after handoff into entry.
    fn core_release(token: Self::InitToken, boot_info: LateBootInfo, entry: fn() -> !) -> !;
}
