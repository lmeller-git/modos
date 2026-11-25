//! Per-CPU read-side tracking: lets a writer wait until no CPU can still hold a stale pointer.
//!
//! Readers are never blocked and write only their own CPU's counter. A writer first unpublishes
//! what it wants to free, then calls [`synchronize`] for the CPU the value was reachable from;
//! afterwards no reader that could have seen the old value is left.

use core::{
    hint::spin_loop,
    marker::PhantomData,
    sync::atomic::{AtomicU64, Ordering, fence},
};

use core_local::CLSAtomic;

use crate::CpuId;

/// Odd while this CPU is inside a read-side section.
///
/// Sections must not nest and must not be preempted or migrated, e.g. because they run in an
/// interrupt gate or with interrupts disabled. That makes each counter single-writer. Remote CPUs
/// only read it.
#[core_local::core_local(remote)]
static SEQ: CLSAtomic<AtomicU64> = CLSAtomic::new(AtomicU64::new(0));

/// Enters a read-side section on this CPU, which lasts until the guard is dropped.
///
/// Everything a reader loads from the structure it protects must be loaded after this call.
pub(crate) fn enter() -> ReadGuard<'static> {
    let seq: &'static AtomicU64 = SEQ.local();
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

/// Whether this CPU is currently inside a read-side section.
pub(crate) fn is_inside() -> bool {
    SEQ.local().load(Ordering::Acquire) & 1 == 1
}

/// Waits until `cpu`, if it was inside a section when called, has left it.
///
/// Must be called after unpublishing, and not from inside a section of `cpu` itself. A section
/// entered after the unpublish cannot see the old value, so it is not waited for.
pub(crate) fn synchronize(cpu: CpuId) {
    fence(Ordering::SeqCst);
    // A CPU that is not online cannot be inside a section.
    let Some(seq) = SEQ.on(cpu) else { return };
    let seen = seq.load(Ordering::Acquire);
    if seen & 1 == 1 {
        while seq.load(Ordering::Acquire) == seen {
            spin_loop();
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
