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

pub fn core_init() {
    unimplemented!()
}

fn unimplemented<T>() -> T {
    unimplemented!()
}

// Returns an iterator over the available CPUs.
pub fn cpus() -> impl Iterator<Item = CpuId> {
    unimplemented::<core::iter::Once<CpuId>>()
}

/// Returns the number of CPUs (SMP cores) that exist and
/// are currently initialized on this system.
pub fn cpu_count() -> u32 {
    unimplemented!()
}

/// Returns the ID of the bootstrap CPU (if known), which
/// is the first CPU to run after system power-on.
pub fn bootstrap_cpu() -> Option<CpuId> {
    unimplemented!()
}

/// Returns true if the currently executing CPU is the bootstrap
/// CPU, i.e., the first CPU to run after system power-on.
pub fn is_bootstrap_cpu() -> bool {
    unimplemented!()
}

/// Returns the ID of the currently executing CPU.
pub fn current_cpu() -> CpuId {
    unimplemented!()
}
