use conquer_once::spin::OnceCell;
use kernel_types::{addr::PhysAddr, hhdm::Hhdm};
use spin::Mutex;
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

use crate::conv;

pub(crate) static PAGETABLE: OnceCell<Mutex<OffsetPageTable<'static>>> = OnceCell::uninit();

pub(crate) fn get_pagetable<'a>() -> &'a Mutex<OffsetPageTable<'static>> {
    PAGETABLE.get().unwrap()
}

pub(crate) fn init_pagetable(hhdm: Hhdm) {
    let level_4_table = unsafe { active_level_4_table(hhdm) };

    PAGETABLE.init_once(|| unsafe {
        Mutex::new(OffsetPageTable::new(
            level_4_table,
            VirtAddr::new(hhdm.offset() as u64),
        ))
    });
}

// reads current p4 rom cpu (CR3) and returns pointer
unsafe fn active_level_4_table(hhdm: Hhdm) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();
    let phys = PhysAddr::new(conv::to_usize(level_4_table_frame.start_address().as_u64()));
    let mut page_table_ptr = hhdm
        .ptr::<PageTable>(phys)
        .expect("level 4 table outside the HHDM");
    unsafe { page_table_ptr.as_mut() }
}
