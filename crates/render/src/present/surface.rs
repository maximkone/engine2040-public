use std::fmt;

use engine_observe::EventCode;

use super::MAX_FRAME_DIM;

// ---------------------------------------------------------------------------
// Errors: typed, with a recovery action; resize never panics.
// ---------------------------------------------------------------------------

/// What the caller should do after a `PresentError`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoveryAction {
    /// Retry the same frame (transient surface state).
    RetryFrame,
    /// Reconfigure the surface/swapchain, keeping the last-good pipeline.
    ReconfigureSurface,
    /// Drop this presenter and build a new one (adapter gone).
    ReacquireDevice,
    /// Keep rendering with the last-good pipeline and targets.
    KeepLastGood,
    /// Skip this frame; try again next frame.
    SkipFrame,
    /// The caller input is bad; fix the draw list or upload and retry.
    FixCallerInput,
}

/// Presenter failures. `Lost`/`Outdated` keep the last-good pipeline;
/// `NoAdapter` is skip-not-green on headless machines, never a fake pass.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PresentError {
    /// No usable adapter; headless CI skips, never fails.
    NoAdapter,
    /// Adapter or device setup failed; message carried.
    Device(String),
    /// The GPU device was lost; build a new presenter (AH2 seam).
    DeviceLost(String),
    /// Swapchain lost; reconfigure, then retry the frame.
    SurfaceLost {
        /// Last-good generation kept across the loss.
        generation: u64,
    },
    /// Swapchain outdated; reconfigure, then retry the frame.
    SurfaceOutdated {
        /// Last-good generation kept across the loss.
        generation: u64,
    },
    /// Acquire timed out; skip this frame.
    SurfaceTimeout,
    /// Window occluded/minimized at the OS level; skip, keep last-good.
    SurfaceOccluded,
    /// Allocation failed; keep last-good, lower quality (AH2 seam: the heal
    /// reports the frame-budget row and steps down to the quality floor).
    OutOfMemory {
        /// Last-good generation kept across the failure.
        generation: u64,
    },
    /// A wgpu validation error fired inside the frame scope; last-good kept.
    Validation(String),
    /// Readback map/poll failure.
    Readback(String),
    /// Frame size rejected (`0` suspends instead; over-bound is an error).
    InvalidSize {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// One draw is unusable; index plus reason carried.
    InvalidDraw {
        /// Draw index in the submitted list.
        index: usize,
        /// Why the draw was refused.
        reason: String,
    },
    /// A mesh upload is unusable; reason carried.
    InvalidMesh(String),
    /// A texture upload is unusable; reason carried.
    InvalidTexture(String),
    /// A ship-path texture lookup found nothing resident: the content gate
    /// fails loudly with the asset id, never a silent magenta fallback.
    /// Explicitly-marked preview paths use `texture_view` (`None` means the
    /// caller may substitute its own placeholder); ship paths use
    /// `require_texture_view` and get this error.
    MissingTexture {
        /// Stable asset identity of the absent texture.
        id: u64,
        /// Why the lookup was refused.
        reason: String,
    },
    /// Surface frame asked for with no surface attached.
    NoSurface,
    /// Digest hex is not 64 hex chars.
    BadDigest(String),
}

impl PresentError {
    /// The caller action that recovers from this error.
    pub fn recovery(&self) -> RecoveryAction {
        match self {
            Self::NoAdapter
            | Self::Validation(_)
            | Self::SurfaceOccluded
            | Self::OutOfMemory { .. } => RecoveryAction::KeepLastGood,
            Self::Device(_) | Self::DeviceLost(_) => RecoveryAction::ReacquireDevice,
            Self::SurfaceLost { .. } | Self::SurfaceOutdated { .. } => {
                RecoveryAction::ReconfigureSurface
            }
            Self::SurfaceTimeout => RecoveryAction::SkipFrame,
            Self::Readback(_) => RecoveryAction::RetryFrame,
            Self::InvalidSize { .. }
            | Self::InvalidDraw { .. }
            | Self::InvalidMesh(_)
            | Self::InvalidTexture(_)
            | Self::MissingTexture { .. }
            | Self::NoSurface
            | Self::BadDigest(_) => RecoveryAction::FixCallerInput,
        }
    }
}

impl fmt::Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no wgpu adapter available"),
            Self::Device(e) => write!(f, "presenter device: {e}"),
            Self::DeviceLost(e) => write!(f, "presenter device lost: {e}"),
            Self::SurfaceLost { generation } => {
                write!(f, "surface lost, last-good generation {generation}")
            }
            Self::SurfaceOutdated { generation } => {
                write!(f, "surface outdated, last-good generation {generation}")
            }
            Self::SurfaceTimeout => write!(f, "surface acquire timed out"),
            Self::SurfaceOccluded => write!(f, "surface occluded"),
            Self::OutOfMemory { generation } => {
                write!(f, "out of memory, last-good generation {generation}")
            }
            Self::Validation(e) => write!(f, "presenter validation: {e}"),
            Self::Readback(e) => write!(f, "presenter readback: {e}"),
            Self::InvalidSize { width, height } => {
                write!(f, "presenter size {width}x{height} rejected")
            }
            Self::InvalidDraw { index, reason } => {
                write!(f, "presenter draw {index}: {reason}")
            }
            Self::InvalidMesh(e) => write!(f, "presenter mesh: {e}"),
            Self::InvalidTexture(e) => write!(f, "presenter texture: {e}"),
            Self::MissingTexture { id, reason } => {
                write!(f, "missing texture {id}: {reason}")
            }
            Self::NoSurface => write!(f, "no surface attached"),
            Self::BadDigest(e) => write!(f, "bad digest: {e}"),
        }
    }
}

impl std::error::Error for PresentError {}

// ---------------------------------------------------------------------------
// Frame sizes: zero area suspends (minimize path), over-bound rejects.
// The state itself stays in `v1::SwapState`; this is the input predicate.
// ---------------------------------------------------------------------------

/// What a requested frame size means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeClass {
    /// Zero area: suspend drawing, keep last-good targets.
    Suspend,
    /// Valid size: (re)build targets.
    Sizes,
    /// Over `MAX_FRAME_DIM`: reject, keep last-good.
    Reject,
}

impl SizeClass {
    /// Classify `width` x `height` in pixels without touching any GPU state.
    pub fn classify(width: u32, height: u32) -> Self {
        if width == 0 || height == 0 {
            return Self::Suspend;
        }
        if width > MAX_FRAME_DIM || height > MAX_FRAME_DIM {
            return Self::Reject;
        }
        Self::Sizes
    }
}

// ---------------------------------------------------------------------------
// Surface events: pure acquire-outcome mapping, no GPU needed. The windowed
// presenter feeds `CurrentSurfaceTexture` through `SurfaceEvent::from_*` and
// acts on the returned `SurfaceAction`, keeping last-good on every path.
// ---------------------------------------------------------------------------

/// One surface-acquire outcome in presenter terms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceEvent {
    /// Fresh frame, surface matches the configuration.
    AcquiredClean,
    /// Fresh frame, but the surface no longer matches; present then fix.
    AcquiredSuboptimal,
    /// Swapchain outdated; reconfigure, then retry.
    Outdated,
    /// Swapchain lost; reconfigure, then retry.
    Lost,
    /// Acquire timed out; skip this frame.
    Timeout,
    /// Window occluded; skip, keep last-good.
    Occluded,
    /// Validation fired on acquire; keep last-good, do not present.
    ValidationError,
}

/// What the presenter does for one `SurfaceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceAction {
    /// Render into the acquired frame and present it.
    Render,
    /// Render, present, then reconfigure to the surface properties.
    RenderThenReconfigure,
    /// Reconfigure the surface, then retry the frame (no present).
    ReconfigureThenRetry,
    /// Skip the frame; retry next frame.
    SkipRetryNext,
    /// Skip the frame; keep last-good targets.
    SkipKeepLastGood,
    /// Do not present; keep last-good pipeline and targets.
    AbortKeepLastGood,
}

/// Pure mapping from acquire outcome to presenter action.
pub fn surface_action(event: SurfaceEvent) -> SurfaceAction {
    match event {
        SurfaceEvent::AcquiredClean => SurfaceAction::Render,
        SurfaceEvent::AcquiredSuboptimal => SurfaceAction::RenderThenReconfigure,
        SurfaceEvent::Outdated | SurfaceEvent::Lost => SurfaceAction::ReconfigureThenRetry,
        SurfaceEvent::Timeout => SurfaceAction::SkipRetryNext,
        SurfaceEvent::Occluded => SurfaceAction::SkipKeepLastGood,
        SurfaceEvent::ValidationError => SurfaceAction::AbortKeepLastGood,
    }
}

// ---------------------------------------------------------------------------
// Supervisor seams (AH2): surface outcomes and presenter errors map to the
// observe catalog rows the render heal reports. Non-fault outcomes map to
// `None` and consume no supervisor budget.
// ---------------------------------------------------------------------------

/// Supervisor fault code for one surface-acquire outcome, if it is a fault.
/// `Lost` reconnects with backoff, `Outdated` and validation failures keep
/// last-good; clean, suboptimal, timeout and occluded outcomes are not
/// faults. Consumer: [`crate::heal::RenderHeal`].
#[must_use]
pub fn surface_fault_code(event: SurfaceEvent) -> Option<EventCode> {
    match event {
        SurfaceEvent::Outdated | SurfaceEvent::ValidationError => {
            Some(EventCode::RENDER_PIPELINE_FAILED)
        }
        SurfaceEvent::Lost => Some(EventCode::RENDER_DEVICE_LOST),
        SurfaceEvent::AcquiredClean
        | SurfaceEvent::AcquiredSuboptimal
        | SurfaceEvent::Timeout
        | SurfaceEvent::Occluded => None,
    }
}

impl PresentError {
    /// Supervisor fault code for this failure, if it is a fault. Device and
    /// surface loss reconnect with backoff, outdated and validation failures
    /// keep last-good, out-of-memory degrades quality to the floor; caller
    /// input, timeouts, occlusion and missing adapters are not faults.
    /// Consumer: [`crate::heal::RenderHeal`].
    #[must_use]
    pub fn fault_code(&self) -> Option<EventCode> {
        match self {
            Self::Device(_) | Self::DeviceLost(_) | Self::SurfaceLost { .. } => {
                Some(EventCode::RENDER_DEVICE_LOST)
            }
            Self::SurfaceOutdated { .. } | Self::Validation(_) => {
                Some(EventCode::RENDER_PIPELINE_FAILED)
            }
            Self::OutOfMemory { .. } => Some(EventCode::RENDER_FRAME_OVER_BUDGET),
            Self::MissingTexture { .. } => Some(EventCode::ASSETS_MISSING),
            Self::NoAdapter
            | Self::SurfaceTimeout
            | Self::SurfaceOccluded
            | Self::Readback(_)
            | Self::InvalidSize { .. }
            | Self::InvalidDraw { .. }
            | Self::InvalidMesh(_)
            | Self::InvalidTexture(_)
            | Self::NoSurface
            | Self::BadDigest(_) => None,
        }
    }
}
