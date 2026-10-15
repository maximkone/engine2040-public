//! Typed startup/shutdown failures with a recovery hint on every variant.
//!
//! Each step of the startup sequence maps to one variant, so a refused
//! startup names the step and tells the caller how to recover. There is
//! no half-started app: any error leaves the app unstarted and retryable.

use engine_ecs::schedule::Phase;
use std::fmt;

/// One startup, frame or shutdown failure plus how to recover.
#[derive(Clone, Debug, PartialEq)]
pub enum AppError {
    /// Config rejected before anything started.
    BadConfig {
        /// Which field and the offending value.
        reason: String,
    },
    /// Frame dt rejected: NaN, infinite or negative.
    BadFrameDt {
        /// The offending dt in seconds.
        value: f32,
    },
    /// Component registry failed to construct.
    Registry {
        /// What failed to register.
        reason: String,
    },
    /// A system name registered twice; the first registration stands.
    Duplicate {
        /// The repeated system name.
        name: &'static str,
    },
    /// A system registered in a phase outside its lane's home phase.
    WrongPhase {
        /// The misplaced system name.
        name: &'static str,
        /// Phase it was registered in.
        got: Phase,
        /// Phase its lane belongs in.
        want: Phase,
    },
    /// Scene document refused: version, decode or hierarchy check.
    SceneLoad {
        /// The codec or hierarchy report.
        reason: String,
    },
    /// The verified headless frame diverged between its two runs.
    Verify {
        /// What diverged.
        reason: String,
    },
    /// An operation that needs a running app on an unstarted one.
    NotStarted {
        /// The refused operation.
        what: &'static str,
    },
    /// Startup or pre-start registration on an already running app.
    AlreadyStarted {
        /// The refused operation.
        what: &'static str,
    },
    /// Any operation on an app whose journal is sealed.
    AlreadyShutdown {
        /// The refused operation.
        what: &'static str,
    },
    /// Late system registration after handover; order is code, not data.
    RegisterAfterStart {
        /// The late system name.
        name: &'static str,
    },
    /// A worker's measured cost exceeded the shutdown deadline.
    WorkerTimeout {
        /// The worker that overran.
        name: &'static str,
        /// Its measured cost in milliseconds.
        cost_ms: u64,
        /// The deadline it missed in milliseconds.
        deadline_ms: u64,
    },
    /// A journal write after the shutdown seal.
    JournalSealed {
        /// The refused step.
        step: &'static str,
    },
    /// A journal write past the bounded capacity.
    JournalFull {
        /// The refused step.
        step: &'static str,
    },
}

impl AppError {
    /// What to do about this failure; quoted in the Display text.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::BadConfig { .. } => {
                "fix AppConfig (tick_hz must be 60, worker_deadline_ms 1..=30000, \
                 journal_capacity 16..=4096) and retry startup"
            }
            Self::BadFrameDt { .. } => {
                "pass a finite frame dt in seconds, zero or more; the accumulator \
                 clamps anything above 0.1s to the 5-step cap"
            }
            Self::Registry { .. } => {
                "re-run startup; if it persists, check engine-ecs Registry bits for \
                 Position and Velocity"
            }
            Self::Duplicate { .. } => {
                "rename the second system or remove it; registration order is code, \
                 never config"
            }
            Self::WrongPhase { .. } => {
                "move the system to the phase its lane belongs in (named want); \
                 render/audio/UI live in PostUpdate, sim lanes in Update"
            }
            Self::SceneLoad { .. } => {
                "migrate the save forward with Scene::migrate or re-export it at the \
                 current scene version; the previous scene is untouched"
            }
            Self::Verify { .. } => {
                "re-run the arena fixture directly; the same config plus tape must \
                 hash identically twice under the hash law"
            }
            Self::NotStarted { .. } => "run startup() first; frames need a handed-over app",
            Self::AlreadyStarted { .. } => {
                "register every system and load the scene before startup; order is code"
            }
            Self::AlreadyShutdown { .. } => "build a fresh app; a sealed journal never reopens",
            Self::RegisterAfterStart { .. } => {
                "register all systems before startup; reordering systems is a code \
                 change with a replay receipt"
            }
            Self::WorkerTimeout { .. } => {
                "raise worker_deadline_ms or shrink the worker's measured cost, then \
                 retry shutdown; the app is still running"
            }
            Self::JournalSealed { .. } => "build a fresh app; a sealed journal never reopens",
            Self::JournalFull { .. } => {
                "raise journal_capacity and rebuild; the journal is bounded by design"
            }
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadConfig { reason } => write!(f, "bad app config: {reason} ({})", self.hint()),
            Self::BadFrameDt { value } => {
                write!(f, "bad frame dt {value}s ({})", self.hint())
            }
            Self::Registry { reason } => {
                write!(f, "registry build failed: {reason} ({})", self.hint())
            }
            Self::Duplicate { name } => {
                write!(f, "duplicate system '{name}' ({})", self.hint())
            }
            Self::WrongPhase { name, got, want } => write!(
                f,
                "system '{name}' in phase {got:?} belongs in {want:?} ({})",
                self.hint()
            ),
            Self::SceneLoad { reason } => {
                write!(f, "scene load refused: {reason} ({})", self.hint())
            }
            Self::Verify { reason } => {
                write!(f, "verified frame diverged: {reason} ({})", self.hint())
            }
            Self::NotStarted { what } => {
                write!(f, "'{what}' needs a started app ({})", self.hint())
            }
            Self::AlreadyStarted { what } => {
                write!(f, "'{what}' after startup refused ({})", self.hint())
            }
            Self::AlreadyShutdown { what } => {
                write!(f, "'{what}' after shutdown refused ({})", self.hint())
            }
            Self::RegisterAfterStart { name } => {
                write!(f, "late registration of '{name}' refused ({})", self.hint())
            }
            Self::WorkerTimeout {
                name,
                cost_ms,
                deadline_ms,
            } => write!(
                f,
                "worker '{name}' cost {cost_ms}ms past deadline {deadline_ms}ms ({})",
                self.hint()
            ),
            Self::JournalSealed { step } => {
                write!(
                    f,
                    "journal write '{step}' after seal refused ({})",
                    self.hint()
                )
            }
            Self::JournalFull { step } => {
                write!(
                    f,
                    "journal write '{step}' past capacity refused ({})",
                    self.hint()
                )
            }
        }
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_names_its_culprit_and_hint() {
        let cases = vec![
            (
                AppError::Duplicate { name: "hud" },
                vec!["hud", "registration order is code"],
            ),
            (
                AppError::WrongPhase {
                    name: "hud",
                    got: Phase::Update,
                    want: Phase::PostUpdate,
                },
                vec!["hud", "Update", "PostUpdate"],
            ),
            (
                AppError::WorkerTimeout {
                    name: "bake",
                    cost_ms: 5000,
                    deadline_ms: 100,
                },
                vec!["bake", "5000", "100"],
            ),
            (
                AppError::BadConfig {
                    reason: "tick_hz 59".to_string(),
                },
                vec!["tick_hz 59", "must be 60"],
            ),
        ];
        for (error, needles) in cases {
            let report = error.to_string();
            for needle in needles {
                assert!(
                    report.contains(needle),
                    "report '{report}' misses '{needle}'"
                );
            }
            assert!(!error.hint().is_empty());
        }
    }

    #[test]
    fn error_is_clone_and_eq() {
        let first = AppError::NotStarted { what: "run_frame" };
        let second = first.clone();
        assert_eq!(first, second);
        assert_ne!(first, AppError::NotStarted { what: "shutdown" });
    }
}
