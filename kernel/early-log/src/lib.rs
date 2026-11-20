#![no_std]

use core::fmt::Arguments;

use spin::Once;

pub type Sink = fn(Arguments<'_>);

static SINK: Once<Sink> = Once::new();

/// Install a logging backend once.
pub fn set_sink(sink: Sink) {
    SINK.call_once(|| sink);
}

/// Writes to installed sink.
/// If none is installed, the output is dropped.
pub fn print(args: Arguments<'_>) {
    if let Some(sink) = SINK.get() {
        sink(args);
    }
}

#[macro_export]
macro_rules! early_print {
    ($($arg:tt)*) => {
        $crate::print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! early_println {
    () => {
        $crate::early_print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::early_print!("{}\n", format_args!($($arg)*))
    };
}
