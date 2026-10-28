#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

impl QemuExitCode {
    pub(crate) fn from_usize(code: usize) -> Self {
        match code {
            0 => Self::Success,
            _ => Self::Failed,
        }
    }
}

pub(crate) fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }

    unreachable!()
}
