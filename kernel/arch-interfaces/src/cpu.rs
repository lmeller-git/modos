pub use boot::mp::CpuId;
use boot::{LateBootInfo, mp::BootCpu};

/// Architectures supporting this can be queried for various information regarding cpu topology and core state.
pub trait CpuDataCapability {
    /// Returns the total count of active cores.
    fn cpu_count() -> usize;
    /// Returns an iterator over all active logical core ids, which are exactly `0..cpu_count()`.
    fn cpus() -> impl Iterator<Item = CpuId>;
    /// Returns data representing the core we are on.
    fn my_cpu() -> BootCpu;
    /// Returns the base address at which this cores public data is stored.
    fn public_data_base() -> *const ();
    /// Returns the base address at which the public data of `cpu` is stored.
    ///
    /// Returns `None` if `cpu` is unknown or has not published its data yet.
    /// The returned base stays valid for the rest of the kernel's lifetime.
    fn public_data_base_of(cpu: CpuId) -> Option<*const ()>;
}

pub trait SmpCapability {
    /// A (potentially singleton) token that may restrict when [`Self::core_release`] can be called.
    type InitToken;
    /// Initializes and starts all cores on this system using smp.
    /// All bootloader information will be consumed by this call and should be released appropriately.
    /// All cores in the system will start in `entry` in a consistent state.
    ///
    /// Consumes the init phase. No more init after handoff into entry.
    ///
    /// All cores are promised to be in a ready state. This is architecture dependant.
    fn core_release(token: Self::InitToken, boot_info: LateBootInfo, entry: fn() -> !) -> !;
}
