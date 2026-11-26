//! Shared helpers for host unit tests.

#![no_std]
#![cfg(not(target_os = "none"))]

extern crate std;

use proptest::{prelude::*, test_runner::Config};

#[cfg(any(miri, loom, shuttle))]
const CASES: u32 = 8;
#[cfg(not(any(miri, loom, shuttle)))]
const CASES: u32 = 512;

/// Property-test config: fewer cases under miri, loom and shuttle, no failure-persistence files.
pub fn config() -> Config {
    Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    }
}

/// Whether `f` panics.
pub fn panics<R>(f: impl FnOnce() -> R) -> bool {
    std::panic::catch_unwind(core::panic::AssertUnwindSafe(f)).is_err()
}

/// Addresses biased towards the edges of an `align`-sized block.
pub fn edge_addr(align: usize) -> impl Strategy<Value = usize> {
    (any::<usize>(), 0..4u8, any::<usize>()).prop_map(move |(raw, mode, any_off)| {
        let off = match mode {
            0 => 0,
            1 => 1.min(align - 1),
            2 => align - 1,
            _ => any_off % align,
        };
        (raw & !(align - 1)) | off
    })
}
