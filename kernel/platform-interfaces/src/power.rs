pub enum ExitCode {
    Success,
    Failure,
}

pub trait ShutdownCapability {
    fn shutdown(code: ExitCode) -> !;
    fn restart() -> !;
}
