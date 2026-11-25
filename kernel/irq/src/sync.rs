//! Per-CPU read-side tracking: lets a writer wait until no CPU can still hold a stale pointer.
//!
//! Readers are never blocked and write only their own CPU's counter. A writer first unpublishes
//! what it wants to free, then calls [`ReadTracker::synchronize`]; afterwards no reader that
//! could have seen the old value is left.

use core::{
    hint::spin_loop,
    marker::PhantomData,
    sync::atomic::{AtomicU64, Ordering, fence},
};

/// One counter per CPU, odd while that CPU is inside a read-side section.
///
/// A CPU's sections must not nest and must not be preempted or migrated, e.g. because they run in
/// an interrupt gate or with interrupts disabled. That makes each counter single-writer.
pub(crate) struct ReadTracker<const CPUS: usize> {
    seq: [AtomicU64; CPUS],
}

impl<const CPUS: usize> ReadTracker<CPUS> {
    pub(crate) const fn new() -> Self {
        Self {
            seq: [const { AtomicU64::new(0) }; CPUS],
        }
    }

    /// Enters a read-side section on `cpu`, which lasts until the guard is dropped.
    ///
    /// Everything a reader loads from the structure it protects must be loaded after this call.
    pub(crate) fn enter(&self, cpu: usize) -> ReadGuard<'_> {
        let seq = &self.seq[cpu];
        let prev = seq.fetch_add(1, Ordering::AcqRel);
        debug_assert!(prev & 1 == 0, "read-side sections must not nest");
        // Pairs with the fence in `synchronize`: either the writer sees this section, or this
        // reader sees what the writer unpublished.
        fence(Ordering::SeqCst);
        ReadGuard {
            seq,
            _not_send: PhantomData,
        }
    }

    /// Whether `cpu` is currently inside a read-side section.
    pub(crate) fn is_inside(&self, cpu: usize) -> bool {
        self.seq[cpu].load(Ordering::Acquire) & 1 == 1
    }

    /// Waits until every CPU in `cpus` that was inside a section when called has left it.
    ///
    /// Must be called after unpublishing, and not from inside a section of one of `cpus`.
    /// A section entered after the unpublish cannot see the old value, so it is not waited for.
    pub(crate) fn synchronize(&self, cpus: impl IntoIterator<Item = usize>) {
        fence(Ordering::SeqCst);
        for cpu in cpus {
            let seq = &self.seq[cpu];
            let seen = seq.load(Ordering::Acquire);
            if seen & 1 == 1 {
                while seq.load(Ordering::Acquire) == seen {
                    spin_loop();
                }
            }
        }
    }
}

/// A read-side section of one CPU. Not `Send`: it belongs to the CPU that entered it.
#[must_use = "the section ends when the guard is dropped"]
pub(crate) struct ReadGuard<'a> {
    seq: &'a AtomicU64,
    _not_send: PhantomData<*mut ()>,
}

impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        // Publishes everything the reader did to the `Acquire` loads in `synchronize`.
        self.seq.fetch_add(1, Ordering::Release);
    }
}
