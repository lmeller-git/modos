use alloc::boxed::Box;
use core::ptr::NonNull;

use limine::{
    BaseRevision,
    RequestsEndMarker,
    RequestsStartMarker,
    mp::MpInfo,
    paging,
    request::{
        BootloaderInfoRequest,
        FramebufferRequest,
        HhdmRequest,
        MemmapRequest,
        ModulesRequest,
        MpRequest,
        PagingModeRequest,
        RsdpRequest,
    },
};

use crate::{
    BootLoaderInfo,
    EarlyBootInfo,
    LateBootInfo,
    mp::{BootCpu, CpuEntry, MpBootInfo},
};

/// Sets the base revision to the latest revision supported by the crate.
/// See specification for further info.
/// Be sure to mark all limine requests with #[used], otherwise they may be removed by the compiler.
#[used]
// The .requests section allows limine to find the requests faster and more safely.
#[unsafe(link_section = ".requests")]
pub(crate) static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static BOOT_INFO: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

// Request a paging mode
#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static PAGING_MODE_REQUEST: PagingModeRequest = PagingModeRequest::new(
    paging::PagingMode::X86_64_4LVL,
    paging::PagingMode::X86_64_4LVL,
    paging::PagingMode::X86_64_4LVL,
);

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static MODULES_REQUEST: ModulesRequest = ModulesRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static MP_REQUEST: MpRequest = MpRequest::new(0);

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

/// Define the start and end markers for Limine requests.
#[used]
#[unsafe(link_section = ".requests_start_marker")]
pub(crate) static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
pub(crate) static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

pub(crate) fn build_early_bootinfo<'a>() -> EarlyBootInfo<'a> {
    assert!(BASE_REVISION.is_supported());

    let boot_info = BOOT_INFO.response().map(|response| BootLoaderInfo {
        name: response.name(),
        version: response.version(),
    });

    let memmap = MEMORY_MAP_REQUEST.response().map(
        |response| // SAFETY: crate::Entry is the same repr(C) struct as used in limine
    unsafe { core::mem::transmute(response.entries()) },
    );

    EarlyBootInfo {
        hhdm_offset: HHDM_REQUEST
            .response()
            .map(|response| response.offset as usize),
        bootloader_info: boot_info,
        rsdp_addr: RSDP_REQUEST
            .response()
            .and_then(|response| NonNull::new(response.address)),
        memmap,
    }
}

pub(crate) fn build_late_bootinfo(early_boot: &EarlyBootInfo<'_>) -> LateBootInfo {
    let mp_info = MP_REQUEST.response().map(|response| MpBootInfo {
        flags: response.flags,
        cpus: response
            .cpus()
            .iter()
            .map(|core_data| BootCpu {
                logical_id: core_data.processor_id as usize,
                hw_id: core_data.lapic_id,
                is_bsp: core_data.lapic_id == response.bsp_lapic_id,
            })
            .collect(),
        start_cpu: start_cpu,
    });

    let bootloader_info = early_boot
        .bootloader_info
        .as_ref()
        .map(|info| BootLoaderInfo {
            name: Box::leak(Box::from(info.name)),
            version: Box::leak(Box::from(info.version)),
        });

    LateBootInfo {
        hhdm_offset: early_boot.hhdm_offset,
        bootloader_info,
        rsdp_addr: early_boot.rsdp_addr,
        memmap: early_boot
            .memmap
            .map(|memap| memap.iter().map(|&&x| x).collect()),
        mp_info,
    }
}

struct StartPacket {
    cpu: BootCpu,
    entry: CpuEntry,
    arg: usize,
}

unsafe extern "C" fn ap_trampoline(info: &MpInfo) -> ! {
    let packet = unsafe { &*(info.extra_argument() as *const StartPacket) };
    (packet.entry)(&packet.cpu, packet.arg)
}

fn start_cpu(cpu: &BootCpu, entry: CpuEntry, arg: usize) {
    let info = MP_REQUEST
        .response()
        .unwrap()
        .cpus()
        .iter()
        .find(|i| i.lapic_id == cpu.hw_id)
        .unwrap();
    let packet = Box::leak(Box::new(StartPacket {
        cpu: *cpu,
        entry,
        arg,
    }));
    info.bootstrap(ap_trampoline, packet as *mut _ as u64);
}
