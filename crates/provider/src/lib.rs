//! provider: provider-neutral AI service contract plus mock, local and
//! remote adapters and the vendor adapters (OpenAI-compatible,
//! Anthropic Messages, Gemini, Ollama). Engine-owned types for requests,
//! capabilities, session policy and jobs per `EDITOR_AI_CONTROL.md` §2:
//! versioned capability/privacy/budget/cancel/job contracts. Adapters
//! translate vendor formats into these types; no provider SDK types and no
//! credentials leave the adapter that owns them. Transport is an owned
//! `std::net::TcpStream` HTTP/1.1 client; `serde_json` parses chat responses
//! strictly.
//!
//! Three layers make a model call repeatable: [`pins`] fixes how a model is
//! asked (system prompt, temperature 0, seed, tools), [`ladder`] descends
//! from the vendor's native structured output through JSON mode and repaired
//! text to the scripted fallback, and [`cassette`] records answers so an eval
//! or CI run replays them offline. Every layer ends in the one strict
//! proposal grammar.
//!
//! [`prompt`] makes each system prompt a versioned, content-addressed asset
//! with an eval gate, and its digest rides every exchange record.
//!
//! [`router`] picks a model per task class from the eval scorecard under
//! deadline and cost caps, and records every decision so it replays.
//!
//! [`arbiter`] decides where local inference runs while the game shares the
//! GPU (GPU, CPU, deferred or a configured remote), and [`LocalAdapter`]
//! applies that decision on its request path.

#![warn(missing_docs)]

pub mod agent_loop;
pub mod announce;
pub mod anthropic;
pub mod arbiter;
pub mod cassette;
pub mod config;
pub mod core;
pub mod fallback;
pub mod gemini;
pub mod ladder;
pub mod local;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod pins;
pub mod policy;
pub mod prompt;
pub mod protocol;
pub mod quota;
pub mod reader;
pub mod remote;
pub mod retry;
pub mod router;
mod serving;
pub mod stream;
pub mod transport;
pub mod vendor;

#[cfg(test)]
mod stub;
#[cfg(test)]
mod suite;
mod test_env;

pub use mock::{
    fixtures, validate_proposal, AdapterOutcome, LateOutcome, MockAdapter, MockOutcome, MockStep,
    ProviderAdapter, VISION_MODEL_PREFIX,
};

pub use announce::{
    announce_request, provider_text, Announcer, EventKindLite, EventLite, FigLite, Line, LineError,
    LogEntry, MatchObservation, PresentationLog, TauntBook, DEFAULT_ANNOUNCE_BUDGET_MS,
    MAX_FIGHTERS_PER_OBSERVATION, MAX_LINE_LEN,
};
pub use anthropic::AnthropicAdapter;
pub use arbiter::{
    Arbiter, ArbiterConfig, ArbiterError, ArbiterNotice, Hold, LocalRoute, MoveReason, Placement,
    PlacementReader, PressureSample, RouteError, VramHeadroom, MAX_DEFERRED,
};
pub use cassette::{
    Answer, CassetteAdapter, CassetteError, CassetteEvent, CassetteKey, CassetteMode,
    CassetteStore, ChangedParts, MissReason, RequestImage,
};
pub use config::{adapter_for, AdapterHandle, AdapterTarget, ConfigError};
pub use gemini::GeminiAdapter;
pub use ladder::{
    action_schema, descend, proposal_schema, repair_text, Descent, DropCause, LadderPlan,
    LadderReport, Rung, RungDrop, RungFailure, RungSupport, ACTION_NAMES,
};
pub use local::{LocalAdapter, LocalConfigError};
pub use ollama::{OllamaAdapter, OllamaConfigError};
pub use openai::OpenAiAdapter;
pub use pins::{
    default_system_prompt, PinError, RequestPins, ToolDef, MAX_TEMPERATURE_MILLI, MAX_TOOLS,
    PINNED_SEED,
};
pub use remote::{RemoteAdapter, SecretError, SecretRef};
pub use transport::{is_loopback, parse_endpoint, Endpoint, EndpointError, TransportError};

pub use fallback::{
    fallback_for, FallbackCause, FallbackErr, FallbackNotice, ScriptedFallback, MAX_NOTICE_LEN,
};
pub use policy::{AiSessionPolicy, CancelReason, Fallback, JobState, ProviderCapabilities};
pub use prompt::{
    gate_edit, score_permille, text_digest, EditReport, EvalModel, PromptAsset, PromptError,
    PromptEval, BUILTIN_KINDS, MAX_EVALS,
};
pub use protocol::{
    AdapterKind, AiError, AiEvent, AiRequest, AiResult, Budget, ContextRef, DataClass, EventKind,
    FinishReason, JobId, ModelSelector, PrivacyMode, Proposal, ProposalAction, ProtocolError,
    RequestId, TaskKind, Usage,
};
pub use quota::{
    resolve_key, scan_package, KeyFinding, KeyKind, KeyRoute, KeySafetyError, KeyScanReport,
    KeyStoreError, KeyUse, MemoryKeyStore, OsKeyStore, PlayerId, QuotaBook, QuotaCharge,
    QuotaConfigError, QuotaLimit, QuotaLimits, QuotaPlan, QuotaReceipt, QuotaRefusal, QuotaScope,
    QuotaUsage, RateLimit, ScanError, ScanLimits, SecretToken, SessionId, MAX_ACCOUNT_LEN,
    MAX_TOKEN_BYTES, MAX_TRACKED_PLAYERS, MAX_TRACKED_SESSIONS, MIN_TOKEN_BYTES,
};
pub use reader::{
    ReaderAdapter, ReaderAnswer, ReaderError, ReaderLabel, ReaderReceipt, ReaderRelation,
    ScriptedRead,
};
pub use router::{
    replay, Ask, Choice, Decision, Router, RouterError, ScoreRow, Skip, TaskClass, Tier,
};

/// Contract version every type in this crate speaks.
pub const PROTOCOL_VERSION: u16 = 1;
/// Maximum context references per request.
pub const MAX_CONTEXT_REFS: usize = 16;
/// Maximum actions per proposal.
pub const MAX_ACTIONS: usize = 64;
/// Maximum preview/diagnostic/label text length.
pub const MAX_TEXT_LEN: usize = 4096;

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_version_is_pinned() {
        assert_eq!(super::PROTOCOL_VERSION, 1);
    }
}
