use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

pub(crate) static PAGETABLE: OnceCell<Mutex<OffsetPageTable<'static>>> = OnceCell::uninit();

pub fn get_pagetable<'a>() -> &'a Mutex<OffsetPageTable<'static>> {
    PAGETABLE.get().unwrap()
}

pub(crate) fn init_pagetable(hhdm_offset: usize) {
    let level_4_table = unsafe { active_level_4_table(hhdm_offset as u64) };

    PAGETABLE.init_once(|| unsafe {
        Mutex::new(OffsetPageTable::new(
            level_4_table,
            VirtAddr::new(hhdm_offset as u64),
        ))
    });
}

// reads current p4 rom cpu (CR3) and returns pointer
unsafe fn active_level_4_table(hhdm_offset: u64) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address().as_u64();
    let virt = VirtAddr::new(hhdm_offset + phys);
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();
    unsafe { &mut *page_table_ptr }
}
