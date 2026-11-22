// TODO: may want an owned non-cloneable token instead of raw u8 vecs.

pub trait InterruptHandlerCapability {
    fn enable();
    fn disable();
    fn without_interrupts<R>(f: impl FnOnce() -> R) -> R;
    fn register_handler(handler: &'static dyn IrqHandler, vec: Option<u8>) -> Option<u8>;
    fn deregister_handler(vec: u8) -> Option<&'static dyn IrqHandler>;
}

pub trait IrqHandler: Send + Sync {
    fn handle(&self, vector: u8);
}
