use core::fmt::Display;

use boot::BootInfo;
use thiserror::Error;

#[derive(Debug, Error)]
pub struct InitError {
    is_deadly: bool,
    kind: InitErrorKind,
    msg: Option<&'static str>,
}

impl Display for InitError {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum InitErrorKind {}

pub trait InitCapability {
    fn early_init(boot_info: &BootInfo) -> Result<(), InitError>;

    fn late_init(boot_info: &BootInfo) -> Result<(), InitError>;
}
