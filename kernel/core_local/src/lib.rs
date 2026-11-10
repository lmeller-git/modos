#![no_std]

/// Cpu local data storage. Relies on corret setup from arch.
///
/// Layout:
/// each core stores its core local info in GsBase. This includes arch specific info right now.
/// The layout will be GsBase [arch --- ... --- core_local]. core_local here simply stores an array of integers, more complex types can be stored behind atomicptrs or similar.
/// This requires lazy init for comple types.
/// Setup is: a core_local is a static in core_local link section. At core init, this will e copied per cpu after its respective GsBase. We will have to ensure here that offsets to GsBase are coherent across all cores (and across linking -> setup), as offsets are computed at comp/link time.
/// So to setup a core_local we query its GsBase addr, add the offset and return a ptr to that location.
/// We must ensure though that this ptr stays on this core, i.e. suppsoe task k on core l queries some core_local var, but then switches cores (scheduler/await/...). It is now in general unsafe to use this ref.
/// So we should guard this access somehow... Maybe core pinning to tell schedulers "this task must stay here for now"? Not clear yet, as we do not have a scheduler. right now every core has its own "thread" anyways, soe no rescheduling can happen currently.

#[cfg(target_arch = "x86_64")]
type CpuDataProvider = arch_x86_64_primitives::Primitive;
#[cfg(not(target_arch = "x86_64"))]
struct CpuDataProvider;

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
