use core::fmt::Display;

use boot::EarlyBootInfo;
use thiserror::Error;

#[derive(Debug, Error)]
pub struct InitError {
    pub is_deadly: bool,
    pub kind: InitErrorKind,
    pub msg: Option<&'static str>,
}

impl Display for InitError {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum InitErrorKind {
    #[error("No error")]
    None,
}

pub trait InitCapability {
    fn early_init(boot_info: &EarlyBootInfo<'_>) -> Result<(), InitError>;

    fn late_init(boot_info: &EarlyBootInfo<'_>) -> Result<(), InitError>;
}
