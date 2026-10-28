#![no_std]

mod qemu;

pub fn exit(code: usize) -> ! {
    qemu::exit_qemu(qemu::QemuExitCode::from_usize(code));
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
