use alloc::boxed::Box;
use core::num::NonZeroUsize;

use kernel_types::{
    addr::{PhysAddr, VirtAddr},
    hhdm::Hhdm,
};
use limine::{
    BaseRevision,
    RequestsEndMarker,
    RequestsStartMarker,
    mp::{MP_FLAG_X2APIC, MpInfo},
    paging,
    request::{
        BootloaderInfoRequest,
        FramebufferRequest,
        HhdmRequest,
        MemmapRequest,
        MpRequest,
        PagingModeRequest,
        RsdpRequest,
        TscFrequencyRequest,
    },
};

use crate::{
    BootLoaderInfo,
    EarlyBootInfo,
    LateBootInfo,
    memmap::MemoryMap,
    mp::{BootCpu, CpuEntry, CpuTable, MpBootInfo},
};

/// Sets the base revision to the latest revision supported by the crate.
/// See specification for further info.
/// Be sure to mark all limine requests with #[used], otherwise they may be removed by the compiler.
#[used]
// The .requests section allows limine to find the requests faster and more safely.
#[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(REVISION);

/// Requested base revision; `build_early_bootinfo` refuses any other.
const REVISION: u64 = BaseRevision::MAX_SUPPORTED;

#[used]
#[unsafe(link_section = ".requests")]
static BOOT_INFO: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

// Request a paging mode
#[used]
#[unsafe(link_section = ".requests")]
static PAGING_MODE_REQUEST: PagingModeRequest = PagingModeRequest::new(
    paging::PagingMode::X86_64_4LVL,
    paging::PagingMode::X86_64_4LVL,
    paging::PagingMode::X86_64_4LVL,
);

#[used]
#[unsafe(link_section = ".requests")]
static MP_REQUEST: MpRequest = MpRequest::new(MP_FLAG_X2APIC);

#[used]
#[unsafe(link_section = ".requests")]
static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static TSC_REQUEST: TscFrequencyRequest = TscFrequencyRequest::new();

/// Define the start and end markers for Limine requests.
#[used]
#[unsafe(link_section = ".requests_start_marker")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

pub(crate) fn build_early_bootinfo<'a>() -> EarlyBootInfo<'a> {
    assert!(BASE_REVISION.is_supported());

    let boot_info = BOOT_INFO.response().map(|response| BootLoaderInfo {
        name: response.name(),
        version: response.version(),
    });

    let memmap = MEMORY_MAP_REQUEST.response().map(|response| {
        // SAFETY: crate::Entry is the same repr(C) struct as used in limine
        let entries = unsafe {
            core::mem::transmute::<&[&limine::memmap::Entry], &[&crate::memmap::Entry]>(
                response.entries(),
            )
        };
        MemoryMap::new(entries)
    });

    let tsc_hz = TSC_REQUEST.response().map(|r| r.frequency);

    let hhdm = HHDM_REQUEST.response().and_then(|response| {
        let offset = NonZeroUsize::new(usize::try_from(response.offset).ok()?)?;
        // SAFETY: Limine maps physical memory at its reported HHDM offset.
        Some(unsafe { Hhdm::new(offset) })
    });

    let rsdp = RSDP_REQUEST
        .response()
        .and_then(|response| rsdp_phys(response.address.addr(), hhdm));

    EarlyBootInfo {
        hhdm,
        bootloader_info: boot_info,
        rsdp,
        memmap,
        tsc_hz,
    }
}

/// The RSDP response is physical under base revision 3 and an HHDM address otherwise.
fn rsdp_phys(addr: usize, hhdm: Option<Hhdm>) -> Option<PhysAddr> {
    if addr == 0 {
        None
    } else if REVISION == 3 {
        Some(PhysAddr::new(addr))
    } else {
        hhdm?.to_phys(VirtAddr::new(addr))
    }
}

pub(crate) fn build_late_bootinfo(early_boot: &EarlyBootInfo<'_>) -> LateBootInfo {
    let mp_info = MP_REQUEST.response().map(|response| {
        let cpus = CpuTable::from_raw(
            response
                .cpus()
                .iter()
                .map(|core_data| (u64::from(core_data.processor_id), core_data.lapic_id)),
            response.bsp_lapic_id,
        )
        .unwrap_or_else(|err| panic!("invalid cpu list from the bootloader: {err}"));
        MpBootInfo {
            flags: response.flags,
            cpus,
            start_cpu,
        }
    });

    let bootloader_info = early_boot
        .bootloader_info
        .as_ref()
        .map(|info| BootLoaderInfo {
            name: Box::leak(Box::from(info.name)),
            version: Box::leak(Box::from(info.version)),
        });

    LateBootInfo {
        hhdm: early_boot.hhdm,
        bootloader_info,
        rsdp: early_boot.rsdp,
        memmap: early_boot.memmap.map(|memmap| memmap.regions().collect()),
        mp_info,
        tsc_hz: early_boot.tsc_hz,
    }
}

struct StartPacket {
    entry: CpuEntry,
    arg: usize,
}

unsafe extern "C" fn ap_trampoline(info: &MpInfo) -> ! {
    let StartPacket { entry, arg } =
        *unsafe { Box::from_raw(info.extra_argument() as *mut StartPacket) };
    (entry)(arg)
}

fn start_cpu(cpu: &BootCpu, entry: CpuEntry, arg: usize) {
    let info = MP_REQUEST
        .response()
        .unwrap()
        .cpus()
        .iter()
        .find(|i| i.lapic_id == cpu.hw_id)
        .unwrap();
    let packet = Box::leak(Box::new(StartPacket { entry, arg }));
    info.bootstrap(ap_trampoline, packet as *mut _ as u64);
}

// `build_early_bootinfo` transmutes limine's entries into ours, and the constants are copies.
const _: () = {
    use core::mem::{align_of, offset_of, size_of};

    use limine::memmap as lim;

    use crate::memmap as ours;

    assert!(size_of::<ours::Entry>() == size_of::<lim::Entry>());
    assert!(align_of::<ours::Entry>() == align_of::<lim::Entry>());
    assert!(offset_of!(ours::Entry, base) == offset_of!(lim::Entry, base));
    assert!(offset_of!(ours::Entry, length) == offset_of!(lim::Entry, length));
    assert!(offset_of!(ours::Entry, type_) == offset_of!(lim::Entry, type_));

    assert!(ours::MEMMAP_USABLE == lim::MEMMAP_USABLE);
    assert!(ours::MEMMAP_RESERVED == lim::MEMMAP_RESERVED);
    assert!(ours::MEMMAP_ACPI_RECLAIMABLE == lim::MEMMAP_ACPI_RECLAIMABLE);
    assert!(ours::MEMMAP_ACPI_NVS == lim::MEMMAP_ACPI_NVS);
    assert!(ours::MEMMAP_BAD_MEMORY == lim::MEMMAP_BAD_MEMORY);
    assert!(ours::MEMMAP_BOOTLOADER_RECLAIMABLE == lim::MEMMAP_BOOTLOADER_RECLAIMABLE);
    assert!(ours::MEMMAP_EXECUTABLE_AND_MODULES == lim::MEMMAP_EXECUTABLE_AND_MODULES);
    assert!(ours::MEMMAP_FRAMEBUFFER == lim::MEMMAP_FRAMEBUFFER);
    assert!(ours::MEMMAP_MAPPED_RESERVED == lim::MEMMAP_MAPPED_RESERVED);
};
