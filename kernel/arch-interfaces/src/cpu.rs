use core::ops::Deref;

use boot::{LateBootInfo, mp::BootCpu};
use derive_more::{Binary, Display, LowerHex, Octal, UpperHex};

/// Logical ID of a CPU core.
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
pub struct CpuId(usize);

impl CpuId {
    /// returns the raw value of this [`CpuId`]
    pub fn value(&self) -> usize {
        self.0
    }

    /// Constructs a new [`CpuId`] from a raw logical id.
    pub fn new(raw: usize) -> Self {
        Self(raw)
    }
}

pub struct CpuLocal<T, S> {
    /// arch specific data stored per CPU. derefs to `BootCpu` in `CpuDataCapability`
    pub arch: T,
    /// Cpu local storage
    pub storage: S, // TODO sync primitives for this core.
                    // sync: M,
                    //...
}

pub trait Storage<T> {}

/// Architectures supporting this can be queried for various information regarding cpu topology and core state.
pub trait CpuDataCapability {
    type Data: Deref<Target = BootCpu>;
    type Storage;
    /// Returns the total count of active cores.
    fn cpu_count() -> usize;
    /// Is the core we are on the BSP?
    fn is_bootstrap_cpu() -> bool;
    /// Returns an iterator over all active logical core ids.
    fn cpus() -> impl Iterator<Item = CpuId>;
    /// Returns data representing the core we are on.
    fn my_cpu() -> &'static CpuLocal<Self::Data, Self::Storage>;
}

pub trait SmpCapability {
    /// A (potentially singleton) token that may restrict when [`Self::core_release`] can be called.
    type InitToken;
    /// Initializes and starts all cores on this system using smp.
    /// All bootloader information will be consumed by this call and should be released appropriately.
    /// All cores in the system will start in `entry` in a consistent state.
    ///
    /// Consumes the init phase. No more init after handoff into entry.
    /// .
    /// All cores are promised to be in a ready state. This is architecture dependent.
    fn core_release(token: Self::InitToken, boot_info: LateBootInfo, entry: fn() -> !) -> !;
}
