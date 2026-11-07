use core::fmt::Arguments;

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum SerialWriteError {
    #[error("Unknown serial writer error: {}", msg)]
    Unknown { msg: &'static str },
}

pub trait SerialWriterCapability {
    fn write_str(msg: &str) -> Result<(), SerialWriteError>;
    fn write_fmt(args: Arguments<'_>) -> Result<(), SerialWriteError>;
}
