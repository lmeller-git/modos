//! Neutral interrupt-line types shared by the architecture and platform seams.
//!
//! The architecture owns vector allocation and dispatch; the platform owns
//! firmware routing (GSI, trigger mode, polarity). Keeping the shared value
//! types here lets both sides speak the same language without either depending
//! on the other's implementation crate.

/// An architecture-assigned interrupt vector.
///
/// The inner value is private so safe code cannot invent a vector. Architectures
/// construct one with [`IrqVector::from_raw_unchecked`] after validating that the
/// vector is dispatchable and not reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct IrqVector(u8);

impl IrqVector {
    /// Wraps a raw vector.
    ///
    /// # Safety
    ///
    /// `raw` must be a vector the architecture has installed a dispatch entry for
    /// and must not be reserved for a CPU exception, the local APIC, or another
    /// fixed purpose.
    #[inline]
    pub const unsafe fn from_raw_unchecked(raw: u8) -> Self {
        Self(raw)
    }

    /// Returns the raw architecture vector.
    #[inline]
    pub const fn raw(self) -> u8 {
        self.0
    }
}

/// How the interrupt source is triggered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerMode {
    Edge,
    Level,
}

/// The polarity of the interrupt source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow,
}

/// The GSI, trigger mode, and polarity of a platform interrupt line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqLineInfo {
    pub gsi: u32,
    pub trigger: TriggerMode,
    pub polarity: Polarity,
}

/// Callback used by a handler to mask or unmask its source.
///
/// The platform installs this when the line is routed. It is a plain function
/// pointer so descriptors stay allocation-free and usable from IRQ context.
pub type IrqMaskFn = fn(u32, bool) -> Result<(), IrqError>;

/// Errors reported by the interrupt registry and platform routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqError {
    /// The architecture has no free vectors left.
    NoFreeVector,
    /// The requested vector is already owned by another registration.
    VectorInUse,
    /// The requested vector is reserved for an exception or a fixed purpose.
    ReservedVector,
    /// The descriptor has no GSI to route.
    NoGsi,
    /// No I/O APIC covers the requested GSI.
    NoIoApic,
    /// The GSI is already claimed by a different vector.
    AlreadyClaimed,
    /// The GSI has not been claimed.
    NotClaimed,
    /// The registration token does not match the current table entry.
    StaleRegistration,
    /// The interrupt controller has not been initialised.
    NotInitialized,
    /// The destination APIC ID cannot be represented in a redirection entry.
    InvalidDestination,
    /// The operation is not supported for this descriptor.
    Unsupported,
}

/// The metadata a handler receives for the line it is servicing.
#[derive(Clone, Copy, Debug)]
pub struct IrqDescriptor {
    vector: IrqVector,
    gsi: Option<u32>,
    trigger: TriggerMode,
    polarity: Polarity,
    set_masked: Option<IrqMaskFn>,
}

impl IrqDescriptor {
    /// Builds a descriptor. Used by the architecture when a handler is registered.
    pub const fn new(
        vector: IrqVector,
        gsi: Option<u32>,
        trigger: TriggerMode,
        polarity: Polarity,
        set_masked: Option<IrqMaskFn>,
    ) -> Self {
        Self {
            vector,
            gsi,
            trigger,
            polarity,
            set_masked,
        }
    }

    #[inline]
    pub const fn vector(self) -> IrqVector {
        self.vector
    }

    #[inline]
    pub const fn gsi(self) -> Option<u32> {
        self.gsi
    }

    #[inline]
    pub const fn trigger(self) -> TriggerMode {
        self.trigger
    }

    #[inline]
    pub const fn polarity(self) -> Polarity {
        self.polarity
    }

    /// Masks the source.
    ///
    /// Returns [`IrqError::NoGsi`] when the line has no GSI and
    /// [`IrqError::Unsupported`] when the platform did not install a callback.
    pub fn mask(self) -> Result<(), IrqError> {
        match (self.gsi, self.set_masked) {
            (Some(gsi), Some(set_masked)) => set_masked(gsi, true),
            (None, _) => Err(IrqError::NoGsi),
            (_, None) => Err(IrqError::Unsupported),
        }
    }

    /// Unmasks the source.
    ///
    /// See [`IrqDescriptor::mask`] for the error cases.
    pub fn unmask(self) -> Result<(), IrqError> {
        match (self.gsi, self.set_masked) {
            (Some(gsi), Some(set_masked)) => set_masked(gsi, false),
            (None, _) => Err(IrqError::NoGsi),
            (_, None) => Err(IrqError::Unsupported),
        }
    }
}

/// A request to register an interrupt handler.
///
/// Use [`IrqRequest::any`] to let the architecture pick a vector, or
/// [`IrqRequest::at`] to request a specific one.
#[derive(Clone, Copy, Debug)]
pub struct IrqRequest {
    pub vector: Option<IrqVector>,
    pub gsi: Option<u32>,
    pub trigger: TriggerMode,
    pub polarity: Polarity,
    pub set_masked: Option<IrqMaskFn>,
}

impl IrqRequest {
    /// Requests any free vector with no line metadata.
    pub const fn any() -> Self {
        Self {
            vector: None,
            gsi: None,
            trigger: TriggerMode::Edge,
            polarity: Polarity::ActiveHigh,
            set_masked: None,
        }
    }

    /// Requests a specific vector.
    pub const fn at(vector: IrqVector) -> Self {
        Self {
            vector: Some(vector),
            gsi: None,
            trigger: TriggerMode::Edge,
            polarity: Polarity::ActiveHigh,
            set_masked: None,
        }
    }

    pub const fn with_gsi(mut self, gsi: u32) -> Self {
        self.gsi = Some(gsi);
        self
    }

    pub const fn with_line(mut self, line: IrqLineInfo) -> Self {
        self.gsi = Some(line.gsi);
        self.trigger = line.trigger;
        self.polarity = line.polarity;
        self
    }

    pub const fn with_trigger(mut self, trigger: TriggerMode) -> Self {
        self.trigger = trigger;
        self
    }

    pub const fn with_polarity(mut self, polarity: Polarity) -> Self {
        self.polarity = polarity;
        self
    }

    pub const fn with_mask_fn(mut self, set_masked: IrqMaskFn) -> Self {
        self.set_masked = Some(set_masked);
        self
    }
}

/// Owned proof that a handler is registered at a vector.
///
/// The token is deliberately not `Clone` or `Copy`: deregistration consumes it,
/// so safe code cannot deregister the same line twice. The generation guards
/// against stale or forged tokens.
#[derive(Debug)]
pub struct IrqRegistration {
    descriptor: IrqDescriptor,
    generation: u32,
}

impl IrqRegistration {
    /// Builds a registration token. Used by the architecture.
    #[doc(hidden)]
    pub const fn new(descriptor: IrqDescriptor, generation: u32) -> Self {
        Self {
            descriptor,
            generation,
        }
    }

    #[inline]
    pub const fn descriptor(&self) -> IrqDescriptor {
        self.descriptor
    }

    #[inline]
    pub const fn vector(&self) -> IrqVector {
        self.descriptor.vector()
    }

    /// The table generation this token was issued for.
    #[doc(hidden)]
    #[inline]
    pub const fn generation(&self) -> u32 {
        self.generation
    }
}
