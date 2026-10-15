//! engine-app lifecycle: builder, registration, phase order, extraction.
//!
//! Owner: the app composition layer. Units: frames, milliseconds of
//! wall-clock startup, fixed ticks, scene nodes. Determinism: system
//! order is code (registration order inside fixed phases) and extraction
//! reads snapshots only, so neither ever enters sim hashes. Errors:
//! typed [`AppError`] per startup step with a recovery hint; any failure
//! leaves the app unstarted and retryable, never half-started.

pub mod app;
pub mod config;
pub mod error;
pub mod journal;
pub mod live_proof;
pub mod platform;
pub mod snapshot;
#[cfg(test)]
mod tests;

pub use app::{
    verify_config, App, AppBuilder, FrameCtx, FrameReceipt, ShutdownReceipt, StartupReceipt,
    SystemFn, SystemKind, VERIFY_TICKS,
};
pub use config::{AppConfig, QualityTier};
pub use engine_ecs::schedule::Phase;
pub use error::AppError;
pub use journal::{Journal, JournalEntry};
pub use platform::{
    LobbyId, MockPlatform, NullPlatform, Platform, PlatformCall, PlatformError, Presence,
    WorkshopId,
};
pub use snapshot::{
    build_hash_for, ActorFrame, Extraction, PresentationFrame, EXTRACTION_SCHEMA, MAX_FRAME_ACTORS,
    PRESENTATION_FRAME_SCHEMA,
};
