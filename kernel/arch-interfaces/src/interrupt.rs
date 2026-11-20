pub trait InterruptHandlerCapability {
    fn enable();
    fn disable();
    fn without_interrupts<R>(f: impl FnOnce() -> R) -> R;
}
