#![no_std]

/// Cpu local data storage. Relies on corret setup from arch.

#[cfg(target_arch = "x86_64")]
type CpuDataProvider = arch_x86_64_primitives::Primitive;
#[cfg(not(target_arch = "x86_64"))]
struct CpuDataProvider;

#[macro_export]
macro_rules! core_local {
    () => {};
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
