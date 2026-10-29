use core::fmt::Arguments;

pub trait SerialWriterCapability {
    fn write_str(msg: &str) -> Result<(), ()>;
    fn write_fmt(args: Arguments<'_>) -> Result<(), ()>;
}
