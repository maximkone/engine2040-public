//! protocol: versioned provider-neutral request/event/result/error types.
//! Engine-owned and serializable; adapters translate vendor formats. No
//! provider SDK types, no network, no credentials here. Serialization is
//! canonical little-endian bytes (no serde dependency); strings are
//! `u16`-length-prefixed and bound-checked, never echoed unbounded.

use super::{MAX_CONTEXT_REFS, MAX_TEXT_LEN, PROTOCOL_VERSION};

/// Request identity, echoed by every event for the request.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RequestId(pub u64);

/// Background job identity for cancel/poll/expiry contracts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct JobId(pub u64);

/// First-task kinds the contract covers; nothing else is routable yet.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TaskKind {
    /// Read-only inspection with a bounded summary result.
    Inspect,
    /// Propose a typed action set without applying it.
    Propose,
    /// Diagnose a staged finding (misplaced prop, invalid value).
    Diagnose,
    /// Repair proposal for a diagnosed finding.
    Repair,
    /// Presentation-only announcer line.
    Announce,
    /// Presentation-only combat taunt.
    Taunt,
}

/// Where the model runs. Local never fails over to cloud (see [`PrivacyMode`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AdapterKind {
    /// Deterministic mock for CI and tests.
    Mock,
    /// Configured local endpoint only (sidecar).
    Local,
    /// Configured remote endpoint (explicit session authorization).
    Remote,
}

/// Model selector: adapter plus a bounded model name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModelSelector {
    /// Which adapter serves this request.
    pub adapter: AdapterKind,
    /// Model name, `1..=128` chars.
    pub model: String,
}

/// Privacy mode. Local-only cannot escalate to any remote destination.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PrivacyMode {
    /// No provider network calls at all.
    Offline,
    /// Configured local endpoints only.
    LocalOnly,
    /// Remote endpoints allowed by explicit session authorization.
    RemoteAllowed,
}

impl PrivacyMode {
    /// True only for [`PrivacyMode::RemoteAllowed`].
    pub const fn allows_remote(self) -> bool {
        matches!(self, PrivacyMode::RemoteAllowed)
    }
}

/// Data classification of one context reference.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DataClass {
    /// Public engine data (schemas, docs).
    Public,
    /// Project data that stays local unless the session allows remote.
    Project,
    /// Secrets-adjacent data: never leaves the machine, never logged.
    Restricted,
}

/// One context reference: digest + revision, never raw bytes or prompts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContextRef {
    /// Data classification of the referenced context.
    pub class: DataClass,
    /// Blake3-style digest of the referenced bytes.
    pub digest: [u8; 32],
    /// Document/world revision the digest was taken at.
    pub revision: u64,
}

/// Bounded budgets for one request. Unknown cost stays unknown (`None`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budget {
    /// Maximum output tokens the caller accepts.
    pub max_output_tokens: u32,
    /// Maximum wall-clock milliseconds for the whole request.
    pub max_ms: u64,
    /// Spend cap in minor units; `None` means cost is unknown.
    pub spend_limit: Option<u64>,
}

/// One provider request: identity, task, model, bounded context, budget.
#[derive(Clone, PartialEq, Debug)]
pub struct AiRequest {
    /// Request identity.
    pub id: RequestId,
    /// What the provider must do.
    pub kind: TaskKind,
    /// Which adapter and model serve it.
    pub model: ModelSelector,
    /// Bounded context references (`<= MAX_CONTEXT_REFS`).
    pub context: Vec<ContextRef>,
    /// Expected output schema version (must equal [`PROTOCOL_VERSION`]).
    pub schema_version: u16,
    /// Token/time/spend bounds.
    pub budget: Budget,
    /// Absolute deadline in wall-clock milliseconds.
    pub deadline_ms: u64,
    /// Privacy mode; remote adapters need [`PrivacyMode::RemoteAllowed`].
    pub privacy: PrivacyMode,
    /// Base document/world revision the request was built against.
    pub base_revision: u64,
}

/// Token usage; `None` fields mean usage is unknown, never zero-claimed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Usage {
    /// Input tokens, or `None` when the adapter does not report it.
    pub input_tokens: Option<u64>,
    /// Output tokens, or `None` when the adapter does not report it.
    pub output_tokens: Option<u64>,
}

/// Why a request finished.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FinishReason {
    /// A complete validated proposal is attached.
    Complete,
    /// Cancelled; late output is ineligible.
    Cancelled,
    /// Budget or deadline exhausted.
    BudgetExceeded,
    /// Structured failure attached.
    Error,
}

/// One typed proposal action. Object handles are stable `u64` ids.
#[derive(Clone, PartialEq, Debug)]
pub enum ProposalAction {
    /// Move an object; centimeters, integer-exact.
    SetPosition {
        /// Stable object id.
        object: u64,
        /// Target `[x, y, z]` in centimeters.
        xyz_cm: [i32; 3],
    },
    /// Relabel an object; bounded text.
    SetLabel {
        /// Stable object id.
        object: u64,
        /// New label, `1..=128` chars.
        label: String,
    },
    /// Explicit no-op, keeps empty proposals well-formed.
    Noop,
}

/// Canonical verb schema the door publishes as `door.grammar_verbs`.
/// PORT guidance-ai/llguidance@6cef6ed TopLevelGrammar::from_json_schema pattern.
pub fn verb_schema_json() -> &'static str {
    r#"{"type":"object","properties":{"actions":{"type":"array","items":{"anyOf":[{"type":"object","properties":{"noop":{"type":"boolean","enum":[true]}},"required":["noop"],"additionalProperties":false},{"type":"object","properties":{"set_position":{"type":"object","properties":{"object":{"type":"integer"},"xyz_cm":{"type":"array","items":{"type":"integer"}}},"required":["object","xyz_cm"],"additionalProperties":false}},"required":["set_position"],"additionalProperties":false},{"type":"object","properties":{"set_label":{"type":"object","properties":{"object":{"type":"integer"},"label":{"type":"string"}}},"required":["object","label"],"additionalProperties":false}},"required":["set_label"],"additionalProperties":false}]}},"diagnostics":{"type":"array","items":{"type":"string"}}},"required":["actions","diagnostics"],"additionalProperties":false}"#
}

/// Malformed-output refusals seen since process start.
static MALFORMED_OUTPUTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Count of outputs refused as `MalformedOutput`; the spike reads it.
pub fn malformed_count() -> u64 {
    MALFORMED_OUTPUTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Record one `MalformedOutput` refusal.
pub fn note_malformed() {
    MALFORMED_OUTPUTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Complete validated proposal for one request.
#[derive(Clone, PartialEq, Debug)]
pub struct Proposal {
    /// Source request identity.
    pub request: RequestId,
    /// Document/world revision the proposal applies to.
    pub base_revision: u64,
    /// Typed actions (`<= MAX_ACTIONS`).
    pub actions: Vec<ProposalAction>,
    /// Referenced object/asset versions as `(id, version)` pairs.
    pub object_versions: Vec<(u64, u64)>,
    /// Bounded diagnostics, one line each.
    pub diagnostics: Vec<String>,
}

/// Complete result for one request.
#[derive(Clone, PartialEq, Debug)]
pub struct AiResult {
    /// Source request identity.
    pub request: RequestId,
    /// How the request finished.
    pub reason: FinishReason,
    /// Proposal when `reason` is [`FinishReason::Complete`].
    pub proposal: Option<Proposal>,
    /// Usage; unknown stays unknown.
    pub usage: Usage,
}

/// Streaming event for one request. Text is preview only, never a command.
#[derive(Clone, PartialEq, Debug)]
pub struct AiEvent {
    /// Source request identity.
    pub request: RequestId,
    /// Monotonic sequence number per request.
    pub seq: u32,
    /// Event payload.
    pub kind: EventKind,
}

/// Event payloads.
#[derive(Clone, PartialEq, Debug)]
pub enum EventKind {
    /// Progress preview text (bounded, never a partial command).
    Progress(String),
    /// Terminal completion with the validated result.
    Completed(AiResult),
    /// Terminal structured failure.
    Failed(AiError),
}

/// Provider failures. Retryability is explicit via [`AiError::retryable`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AiError {
    /// Capability, privacy mode or adapter cannot serve this request.
    /// Privacy refusals (e.g. remote adapter under local-only policy) land
    /// here: the session was not authorized for that capability.
    UnsupportedCapability,
    /// Deadline or budget exhausted.
    Timeout,
    /// Cancelled by caller, scene close, provider switch or expiry.
    Cancelled,
    /// Adapter unreachable or offline without fallback.
    Unavailable,
    /// Authentication, quota or spend-cap refusal.
    QuotaAuth,
    /// Output failed validation (schema, bounds, unknown action).
    MalformedOutput,
    /// Base revision moved; the request was built on a stale context.
    StaleContext,
}

impl AiError {
    /// True when retrying with the same request identity is allowed.
    /// Retries share the original deadline and budget and can never silently
    /// duplicate a world mutation.
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            AiError::Timeout | AiError::Unavailable | AiError::StaleContext
        )
    }
}

/// Contract failures for malformed bytes or violated bounds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProtocolError {
    /// Version field is not [`PROTOCOL_VERSION`].
    BadVersion(u16),
    /// Truncated or trailing bytes.
    Truncated,
    /// A bounded collection or string exceeds its cap.
    TooLong,
    /// String is not valid UTF-8.
    BadUtf8,
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_text(bytes: &mut Vec<u8>, text: &str) {
    push_u16(bytes, text.len() as u16);
    bytes.extend_from_slice(text.as_bytes());
}

fn check_text(text: &str, max: usize) -> Result<(), ProtocolError> {
    if text.len() > max || text.len() > u16::MAX as usize {
        return Err(ProtocolError::TooLong);
    }
    Ok(())
}

/// Canonical bytes for [`AiRequest`]: version tag first, then fields.
pub fn encode_request(request: &AiRequest) -> Result<Vec<u8>, ProtocolError> {
    if request.context.len() > MAX_CONTEXT_REFS {
        return Err(ProtocolError::TooLong);
    }
    check_text(&request.model.model, 128)?;
    let mut bytes = Vec::new();
    push_u16(&mut bytes, PROTOCOL_VERSION);
    push_u64(&mut bytes, request.id.0);
    bytes.push(request.kind as u8);
    bytes.push(request.model.adapter as u8);
    push_text(&mut bytes, &request.model.model);
    push_u16(&mut bytes, request.context.len() as u16);
    for reference in &request.context {
        bytes.push(reference.class as u8);
        bytes.extend_from_slice(&reference.digest);
        push_u64(&mut bytes, reference.revision);
    }
    push_u16(&mut bytes, request.schema_version);
    push_u64(&mut bytes, request.budget.max_output_tokens as u64);
    push_u64(&mut bytes, request.budget.max_ms);
    match request.budget.spend_limit {
        Some(limit) => {
            bytes.push(1);
            push_u64(&mut bytes, limit);
        }
        None => bytes.push(0),
    }
    push_u64(&mut bytes, request.deadline_ms);
    bytes.push(request.privacy as u8);
    push_u64(&mut bytes, request.base_revision);
    Ok(bytes)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self.pos.saturating_add(count);
        if end > self.bytes.len() {
            return Err(ProtocolError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    /// The next `N` bytes as an array; a short input is `Truncated`.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn text(&mut self, max: usize) -> Result<String, ProtocolError> {
        let len = self.u16()? as usize;
        if len > max {
            return Err(ProtocolError::TooLong);
        }
        let bytes = self.take(len)?;
        core::str::from_utf8(bytes)
            .map_err(|_| ProtocolError::BadUtf8)
            .map(|text| text.to_owned())
    }
}

/// Decode [`encode_request`] output; version, bounds and UTF-8 are enforced.
pub fn decode_request(bytes: &[u8]) -> Result<AiRequest, ProtocolError> {
    let mut reader = Reader { bytes, pos: 0 };
    let version = reader.u16()?;
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::BadVersion(version));
    }
    let id = RequestId(reader.u64()?);
    let kind = match reader.u8()? {
        0 => TaskKind::Inspect,
        1 => TaskKind::Propose,
        2 => TaskKind::Diagnose,
        3 => TaskKind::Repair,
        4 => TaskKind::Announce,
        5 => TaskKind::Taunt,
        _ => return Err(ProtocolError::Truncated),
    };
    let adapter = match reader.u8()? {
        0 => AdapterKind::Mock,
        1 => AdapterKind::Local,
        2 => AdapterKind::Remote,
        _ => return Err(ProtocolError::Truncated),
    };
    let model = reader.text(128)?;
    let contexts = reader.u16()? as usize;
    if contexts > MAX_CONTEXT_REFS {
        return Err(ProtocolError::TooLong);
    }
    let mut context = Vec::with_capacity(contexts);
    for _ in 0..contexts {
        let class = match reader.u8()? {
            0 => DataClass::Public,
            1 => DataClass::Project,
            2 => DataClass::Restricted,
            _ => return Err(ProtocolError::Truncated),
        };
        let mut digest = [0u8; 32];
        digest.copy_from_slice(reader.take(32)?);
        context.push(ContextRef {
            class,
            digest,
            revision: reader.u64()?,
        });
    }
    let schema_version = reader.u16()?;
    let budget = Budget {
        max_output_tokens: reader.u64()? as u32,
        max_ms: reader.u64()?,
        spend_limit: match reader.u8()? {
            0 => None,
            1 => Some(reader.u64()?),
            _ => return Err(ProtocolError::Truncated),
        },
    };
    let deadline_ms = reader.u64()?;
    let privacy = match reader.u8()? {
        0 => PrivacyMode::Offline,
        1 => PrivacyMode::LocalOnly,
        2 => PrivacyMode::RemoteAllowed,
        _ => return Err(ProtocolError::Truncated),
    };
    let base_revision = reader.u64()?;
    if reader.pos != bytes.len() {
        return Err(ProtocolError::Truncated);
    }
    Ok(AiRequest {
        id,
        kind,
        model: ModelSelector { adapter, model },
        context,
        schema_version,
        budget,
        deadline_ms,
        privacy,
        base_revision,
    })
}

/// Validate a request against session policy: privacy escalation is refused
/// as [`AiError::UnsupportedCapability`]; unknown spend with a hard cap need
/// is refused as [`AiError::QuotaAuth`]; bounds violations fail fast.
pub fn validate_request(
    request: &AiRequest,
    capabilities: &super::ProviderCapabilities,
    policy: &super::AiSessionPolicy,
) -> Result<(), AiError> {
    if request.schema_version != PROTOCOL_VERSION {
        return Err(AiError::MalformedOutput);
    }
    if request.context.len() > MAX_CONTEXT_REFS {
        return Err(AiError::MalformedOutput);
    }
    if request.model.model.is_empty() || request.model.model.len() > 128 {
        return Err(AiError::MalformedOutput);
    }
    if !policy.adapters.contains(&request.model.adapter) {
        return Err(AiError::UnsupportedCapability);
    }
    if request.model.adapter == AdapterKind::Remote && !request.privacy.allows_remote() {
        return Err(AiError::UnsupportedCapability);
    }
    if request.model.adapter == AdapterKind::Remote && !policy.mode.allows_remote() {
        return Err(AiError::UnsupportedCapability);
    }
    if request.budget.max_ms == 0 || request.budget.max_output_tokens == 0 {
        return Err(AiError::MalformedOutput);
    }
    if request.budget.spend_limit.is_none()
        && policy.spend_limit.is_some()
        && request.model.adapter == AdapterKind::Remote
    {
        // A hard session cap plus unknown remote cost: refuse, never guess.
        return Err(AiError::QuotaAuth);
    }
    if !capabilities.cancellation && request.budget.max_ms > MAX_TEXT_LEN as u64 * 1000 {
        return Err(AiError::UnsupportedCapability);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn request() -> AiRequest {
        AiRequest {
            id: RequestId(7),
            kind: TaskKind::Diagnose,
            model: ModelSelector {
                adapter: AdapterKind::Local,
                model: "sidecar-1".into(),
            },
            context: vec![ContextRef {
                class: DataClass::Project,
                digest: [3u8; 32],
                revision: 12,
            }],
            schema_version: PROTOCOL_VERSION,
            budget: Budget {
                max_output_tokens: 512,
                max_ms: 5_000,
                spend_limit: None,
            },
            deadline_ms: 60_000,
            privacy: PrivacyMode::LocalOnly,
            base_revision: 12,
        }
    }

    #[test]
    fn request_roundtrips_bit_exact() {
        let bytes = encode_request(&request()).unwrap();
        let (again, twice) = (
            decode_request(&bytes).unwrap(),
            decode_request(&bytes).unwrap(),
        );
        assert_eq!(again, request());
        assert_eq!(encode_request(&again).unwrap(), bytes);
        assert_eq!(again, twice);
    }

    #[test]
    fn corrupt_encodings_fail_bounded() {
        assert_eq!(decode_request(&[]), Err(ProtocolError::Truncated));
        let mut bad = encode_request(&request()).unwrap();
        bad[0] = 0xFF;
        assert!(matches!(
            decode_request(&bad),
            Err(ProtocolError::BadVersion(_))
        ));
        let mut truncated = encode_request(&request()).unwrap();
        truncated.pop();
        assert_eq!(decode_request(&truncated), Err(ProtocolError::Truncated));
        let mut trailing = encode_request(&request()).unwrap();
        trailing.push(0);
        assert_eq!(decode_request(&trailing), Err(ProtocolError::Truncated));
    }

    #[test]
    fn error_retryability_is_explicit() {
        assert!(AiError::Timeout.retryable());
        assert!(AiError::Unavailable.retryable());
        assert!(AiError::StaleContext.retryable());
        assert!(!AiError::UnsupportedCapability.retryable());
        assert!(!AiError::Cancelled.retryable());
        assert!(!AiError::QuotaAuth.retryable());
        assert!(!AiError::MalformedOutput.retryable());
    }

    #[test]
    fn unknown_usage_stays_unknown() {
        let usage = Usage::default();
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
    }
}
