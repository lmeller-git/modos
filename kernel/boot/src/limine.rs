use core::ptr::NonNull;

use limine::{
    BaseRevision,
    RequestsEndMarker,
    RequestsStartMarker,
    paging,
    request::{
        BootloaderInfoRequest,
        FramebufferRequest,
        HhdmRequest,
        MemmapRequest,
        PagingModeRequest,
        RsdpRequest,
    },
};

use crate::{BootInfo, BootLoaderInfo};

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

pub(crate) fn build_bootinfo() -> BootInfo {
    assert!(BASE_REVISION.is_supported());

    let boot_info = BOOT_INFO.response().map(|response| BootLoaderInfo {
        name: response.name(),
        version: response.version(),
    });

    let memmap = MEMORY_MAP_REQUEST.response().map(
        |response| // SAFETY: crate::Entry is the same repr(C) struct as used in limine
    unsafe { core::mem::transmute(response.entries()) },
    );

    BootInfo {
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
