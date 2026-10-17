//! Bug bundles: a crash or failed check saves tape, build and
//! rules/content versions plus bounded redacted events.
//!
//! Consumer: `A9` bug-tape evidence and `HL2` chaos rows; agents read
//! [`BugBundle`] through [`BugBundle::events`] and [`replay`]. Data owner:
//! this module owns [`BugBundle`], [`BuildId`], [`RedactedEvent`],
//! [`ScreenshotCapture`], [`DivergenceReport`], [`BundleError`],
//! [`collect`], [`collect_crash`] and [`replay`]; the proof fixture owns tape
//! truth (`crate::record`/`crate::replay`), `OB2` trace owns the event detail
//! shape, and `engine-observe` owns the event-code registry. Units: ticks
//! (`u32`, zero-based), byte budgets (`usize`). Determinism scope: the same
//! (`Tape`, `Error`) always gives the same bundle bytes on every machine;
//! wall-clock timing is never stored. Error type: [`BundleError`]; corrupt
//! bundles fail, never panic.
//!
//! Secrets are redacted by construction: no bundle field carries a prompt,
//! credential or token. [`RedactedEvent::new`] and every `collect` path build
//! details only from checkpoint hex, input counts and static diagnostics, and
//! [`scrub_detail`] replaces any secret-shaped text with `[redacted detail]`
//! as defense in depth. Bundles persist as plain versioned bytes (no new
//! dependency); the headless proof fixture has no renderer, so the screenshot
//! is always [`ScreenshotCapture::Unavailable`] with an explicit reason.

use std::fmt;

use engine_observe::{EventCode, MAX_MESSAGE_LEN};

use crate::tape::MAX_TAPE_BYTES;
use crate::{Checkpoint, Error, Tape, FIXTURE_VERSION};

/// Bundle envelope version, independent of tape and fixture versions.
pub const BUNDLE_FORMAT: u16 = 1;
/// Most redacted events one bundle holds; the trace prefix is truncated.
pub const MAX_BUNDLE_EVENTS: usize = 64;
/// Most content digests one bundle holds; the proof fixture is asset-free.
pub const MAX_CONTENT_DIGESTS: usize = 32;
/// Longest accepted build version text in bytes.
pub const MAX_BUILD_VERSION_BYTES: usize = 32;
/// Longest accepted failure detail in bytes.
pub const MAX_FAILURE_DETAIL_BYTES: usize = 256;
/// Longest accepted screenshot-absence reason in bytes.
pub const MAX_UNAVAILABLE_REASON_BYTES: usize = 128;
/// Largest accepted screenshot payload; the headless fixture stores none.
pub const MAX_SCREENSHOT_BYTES: usize = 32_768;
/// Largest accepted bundle; checked before allocation by IO consumers.
pub const MAX_BUNDLE_BYTES: usize = MAX_TAPE_BYTES
    + 2048
    + MAX_CONTENT_DIGESTS * 36
    + MAX_BUNDLE_EVENTS * 300
    + MAX_SCREENSHOT_BYTES;
/// Headless reason stored whenever no renderer capture exists.
pub const HEADLESS_UNAVAILABLE: &str = "headless: no renderer capture in proof fixture";
/// Stored when a detail trips the secret-shaped-text screen.
pub const REDACTED_DETAIL: &str = "[redacted detail]";
/// Envelope magic for bundle bytes.
const MAGIC: &[u8; 8] = b"E2040BB\0";

/// Build that produced the bundle: crate version plus a short git sha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildId {
    /// Crate version text, bounded to [`MAX_BUILD_VERSION_BYTES`] bytes.
    pub version: String,
    /// Short git sha; zeros mean the sha was unavailable at build time.
    pub git_sha_short: [u8; 8],
}

impl BuildId {
    /// Build of this binary: `CARGO_PKG_VERSION` plus zeros for the sha,
    /// which no build script stamps today. Deterministic by construction.
    #[must_use]
    pub fn current() -> Self {
        Self {
            version: bound_str(env!("CARGO_PKG_VERSION"), MAX_BUILD_VERSION_BYTES),
            git_sha_short: [0; 8],
        }
    }
}

/// One bounded diagnostic event with no secret-carrying field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedactedEvent {
    /// Zero-based tick the event belongs to.
    pub tick: u32,
    /// Stable observe registry code.
    pub code: EventCode,
    /// Scrubbed human/agent-readable detail, at most [`MAX_MESSAGE_LEN`].
    pub detail: String,
}

impl RedactedEvent {
    /// Build an event; the detail is truncated then scrubbed, so
    /// secret-shaped text never survives construction.
    #[must_use]
    pub fn new(tick: u32, code: EventCode, detail: &str) -> Self {
        Self {
            tick,
            code,
            detail: scrub_detail(detail),
        }
    }
}

/// Screenshot payload: present bytes or an explicit absence reason, never both
/// silent. The headless proof fixture always stores the absent form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenshotCapture {
    /// No capture; the reason names why (for example headless).
    Unavailable {
        /// Explicit reason, bounded to [`MAX_UNAVAILABLE_REASON_BYTES`].
        unavailable_reason: String,
    },
    /// Raw capture bytes (for example PNG); bounded to
    /// [`MAX_SCREENSHOT_BYTES`].
    Present {
        /// Capture payload.
        bytes: Vec<u8>,
    },
}

impl ScreenshotCapture {
    /// Absent capture with an explicit bounded reason.
    #[must_use]
    pub fn unavailable(reason: &str) -> Self {
        Self::Unavailable {
            unavailable_reason: bound_str(reason, MAX_UNAVAILABLE_REASON_BYTES),
        }
    }

    /// Present capture; oversize or empty input fails instead of truncating.
    ///
    /// # Errors
    ///
    /// Returns [`BundleError::Corrupt`] when `bytes` is empty or oversize.
    pub fn present(bytes: &[u8]) -> Result<Self, BundleError> {
        if bytes.is_empty() || bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err(BundleError::Corrupt("screenshot size is outside bounds"));
        }
        Ok(Self::Present {
            bytes: bytes.to_vec(),
        })
    }
}

/// Which failure a bundle preserves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// Replay checkpoints diverged.
    Divergence,
    /// Fixture input or shape was invalid.
    Invalid,
    /// Tape or fixture version was unsupported.
    Version,
    /// Default golden checksum mismatched.
    Golden,
    /// Crash without a typed [`Error`].
    Crash,
}

/// A crash or failed check saved with its replay tape and versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BugBundle {
    /// Encoded failing tape (`Tape::encode`); empty when the tape itself was
    /// unencodable, in which case [`replay`] reports missing artifact.
    pub tape_bytes: Vec<u8>,
    /// Build that recorded the failure.
    pub build: BuildId,
    /// Fixture rules version at record time ([`FIXTURE_VERSION`] today).
    pub rules_version: u16,
    /// Content digests pinned at record time (digest, byte length); empty
    /// for the asset-free proof fixture.
    pub content_digests: Vec<([u8; 32], u32)>,
    /// Bounded scrubbed trace prefix plus one failure marker.
    pub events: Vec<RedactedEvent>,
    /// Screenshot or the explicit reason it is absent.
    pub screenshot: ScreenshotCapture,
    /// Which failure was collected.
    pub failure_kind: FailureKind,
    /// Recorded first-divergent tick for [`FailureKind::Divergence`].
    pub failure_tick: Option<u32>,
    /// Bounded scrubbed failure summary; static diagnostics only.
    pub failure_detail: String,
    /// Trace ticks omitted between the retained prefix and the failure.
    pub dropped_events: u32,
}

/// Authoritative replay outcome: where the bundle's tape first diverges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DivergenceReport {
    /// Zero-based tick of the first mismatching checkpoint.
    pub first_divergent_tick: u32,
    /// Recorded checkpoint at that tick.
    pub expected: Checkpoint,
    /// Re-simulated checkpoint at that tick.
    pub actual: Checkpoint,
}

/// Bundle load or replay failure: diagnosed, never a silent pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// The bundle holds no tape bytes.
    MissingArtifact(&'static str),
    /// The bundle envelope version is not supported.
    UnsupportedFormat {
        /// Envelope version found in the bytes.
        got: u16,
    },
    /// The bundle's rules version is not this build's fixture.
    UnsupportedTier {
        /// Rules version found in the bundle.
        rules_version: u16,
    },
    /// The embedded tape's own version gate fired.
    UnsupportedTape {
        /// Tape envelope version found in the bytes.
        format: u16,
        /// Fixture version found in the bytes.
        fixture: u16,
    },
    /// Invalid data, with a bounded static diagnostic.
    Corrupt(&'static str),
    /// The embedded tape replays cleanly, so there is no divergence to name.
    CleanReplay,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingArtifact(reason) => write!(f, "missing bundle artifact: {reason}"),
            Self::UnsupportedFormat { got } => {
                write!(f, "unsupported bundle version {got}")
            }
            Self::UnsupportedTier { rules_version } => {
                write!(f, "unsupported rules tier {rules_version}")
            }
            Self::UnsupportedTape { format, fixture } => write!(
                f,
                "unsupported tape version {format}, fixture version {fixture}"
            ),
            Self::Corrupt(reason) => write!(f, "invalid bug bundle: {reason}"),
            Self::CleanReplay => write!(f, "bundle tape replays without divergence"),
        }
    }
}

impl std::error::Error for BundleError {}

/// Substrings that mark secret-shaped text (ASCII case-insensitive).
const SECRET_MARKERS: &[&str] = &[
    "sk-",
    "bearer",
    "api_key",
    "apikey",
    "api-key",
    "password",
    "passwd",
    "secret",
    "token",
    "credential",
    "auth",
    "prompt",
    "system prompt",
];

/// Truncate `text` to `max_bytes` bytes on a `char` boundary.
fn bound_str(text: &str, max_bytes: usize) -> String {
    text[..text.floor_char_boundary(max_bytes)].to_owned()
}

/// Scrub secret-shaped text: truncate to [`MAX_MESSAGE_LEN`], then replace
/// the whole detail with [`REDACTED_DETAIL`] when any [`SECRET_MARKERS`]
/// entry appears (ASCII case-insensitive). Primary redaction is by
/// construction (details only carry checkpoint hex, counts and static
/// diagnostics); this is the defense-in-depth screen.
fn scrub_detail(raw: &str) -> String {
    let bounded = bound_str(raw, MAX_MESSAGE_LEN);
    let lower = bounded.to_ascii_lowercase();
    if SECRET_MARKERS.iter().any(|mark| lower.contains(mark)) {
        REDACTED_DETAIL.to_owned()
    } else {
        bounded
    }
}

/// Detail for one trace tick: checkpoint hex plus the applied-input count.
///
/// Fixed shape (`legacy=<16 hex> state=<16 hex> inputs=<n>`), so details stay
/// far under the observe message bound and hash identically on every machine.
fn detail_for(checkpoint: Checkpoint, inputs: usize) -> String {
    format!(
        "legacy={:016x} state={:016x} inputs={inputs}",
        checkpoint.legacy, checkpoint.state
    )
}

/// Applied-input count per tick, in tick order; out-of-range ticks are
/// ignored defensively and never panic.
fn inputs_per_tick(tape: &Tape) -> Vec<usize> {
    let mut counts = vec![0usize; tape.config.ticks.min(crate::MAX_TICKS) as usize];
    for input in &tape.inputs {
        if let Some(slot) = counts.get_mut(input.tick as usize) {
            *slot += 1;
        }
    }
    counts
}

/// Failure marker code: divergence is a checkpoint desync, everything else is
/// a versioned-save rejection.
fn failure_code(kind: FailureKind) -> EventCode {
    match kind {
        FailureKind::Divergence => EventCode::NET_CLIENT_DESYNC,
        FailureKind::Invalid | FailureKind::Version | FailureKind::Golden | FailureKind::Crash => {
            EventCode::SAVE_CORRUPT
        }
    }
}

/// Split an [`Error`] into its kind, optional divergent tick and a static
/// failure summary. Summaries carry ticks and hex only, never input bytes.
fn describe(error: &Error) -> (FailureKind, Option<u32>, String) {
    match *error {
        Error::Divergence {
            tick,
            expected,
            actual,
            ..
        } => (
            FailureKind::Divergence,
            Some(tick),
            format!(
                "diverged at tick {tick}: expected {:016x}/{:016x}, got {:016x}/{:016x}",
                expected.legacy, expected.state, actual.legacy, actual.state
            ),
        ),
        Error::Invalid(reason) => (
            FailureKind::Invalid,
            None,
            scrub_detail(&format!("invalid proof: {reason}")),
        ),
        Error::UnsafePath(reason) => (
            FailureKind::Invalid,
            None,
            scrub_detail(&format!("unsafe tape path: {reason}")),
        ),
        Error::Version { format, fixture } => (
            FailureKind::Version,
            None,
            format!("unsupported tape version {format}, fixture version {fixture}"),
        ),
        Error::Golden { actual } => (
            FailureKind::Golden,
            None,
            format!("golden mismatch: got {actual:016x}"),
        ),
    }
}

/// Redacted trace prefix plus one failure marker for `tape`/`error`.
///
/// Divergence keeps the prefix before the recorded tick (truncated to
/// [`MAX_BUNDLE_EVENTS`] minus the marker) and counts the omitted ticks in
/// `dropped`; other failures keep a single marker event.
fn build_events(
    tape: &Tape,
    kind: FailureKind,
    tick: Option<u32>,
    failure_detail: &str,
) -> (Vec<RedactedEvent>, u32) {
    let counts = inputs_per_tick(tape);
    if let Some(tick) = tick {
        let keep = (MAX_BUNDLE_EVENTS - 1).min(tick as usize);
        let start = tick as usize - keep;
        let mut events = Vec::with_capacity(keep + 1);
        for (index, n) in (start..tick as usize).enumerate() {
            if let Some(checkpoint) = tape.checkpoints.get(start + index) {
                let count = counts.get(n + start).copied().unwrap_or(0);
                events.push(RedactedEvent::new(
                    (start + index) as u32,
                    EventCode::SIM_TICK,
                    &detail_for(*checkpoint, count),
                ));
            }
        }
        let dropped = u32::try_from(start).unwrap_or(u32::MAX);
        events.push(RedactedEvent::new(tick, failure_code(kind), failure_detail));
        (events, dropped)
    } else {
        let marker_tick = tape.config.ticks.saturating_sub(1);
        (
            vec![RedactedEvent::new(
                marker_tick,
                failure_code(kind),
                failure_detail,
            )],
            0,
        )
    }
}

/// Collect a failed check into a bundle: tape bytes, current build,
/// [`FIXTURE_VERSION`], empty content digests (the proof fixture is
/// asset-free), bounded scrubbed events and an explicit headless screenshot
/// reason. The same (`tape`, `error`) always gives the same bundle.
#[must_use]
pub fn collect(tape: &Tape, error: &Error) -> BugBundle {
    let (kind, tick, detail) = describe(error);
    let (events, dropped_events) = build_events(tape, kind, tick, &detail);
    let encoded = tape.encode();
    assert!(encoded.is_ok(), "bug bundle tape must encode");
    BugBundle {
        tape_bytes: encoded.unwrap_or_default(),
        build: BuildId::current(),
        rules_version: FIXTURE_VERSION,
        content_digests: Vec::new(),
        events,
        screenshot: ScreenshotCapture::unavailable(HEADLESS_UNAVAILABLE),
        failure_kind: kind,
        failure_tick: tick,
        failure_detail: scrub_detail(&detail),
        dropped_events,
    }
}

/// Collect a crash without a typed [`Error`]: same tape/build/version shape
/// as [`collect`] with a static scrubbed reason as the failure marker.
#[must_use]
pub fn collect_crash(tape: &Tape, reason: &'static str) -> BugBundle {
    let detail = scrub_detail(&format!("crash: {reason}"));
    let (events, dropped_events) = build_events(tape, FailureKind::Crash, None, &detail);
    let encoded = tape.encode();
    assert!(encoded.is_ok(), "bug bundle tape must encode");
    BugBundle {
        tape_bytes: encoded.unwrap_or_default(),
        build: BuildId::current(),
        rules_version: FIXTURE_VERSION,
        content_digests: Vec::new(),
        events,
        screenshot: ScreenshotCapture::unavailable(HEADLESS_UNAVAILABLE),
        failure_kind: FailureKind::Crash,
        failure_tick: None,
        failure_detail: detail,
        dropped_events,
    }
}

/// Replay the bundle's tape and name the first divergent tick.
///
/// Trace-diff contract: the same seed plus config plus inputs reruns to a
/// byte-identical TRACE prefix (see `crate::trace::replay_verbose`); any
/// mismatch reports `first_divergent_tick` here, never a silent pass.
///
/// Supported tapes replay to the recorded divergence. An empty tape, an
/// unsupported bundle/rules/tape version, a corrupt tape or a clean replay
/// is diagnosed with [`BundleError`], never a silent pass. The stored
/// `failure_tick` is evidence only; the returned tick is authoritative.
pub fn replay(bundle: &BugBundle) -> Result<DivergenceReport, BundleError> {
    if bundle.tape_bytes.is_empty() {
        return Err(BundleError::MissingArtifact("bundle holds no tape bytes"));
    }
    if bundle.rules_version != FIXTURE_VERSION {
        return Err(BundleError::UnsupportedTier {
            rules_version: bundle.rules_version,
        });
    }
    let tape = match Tape::decode(&bundle.tape_bytes) {
        Ok(tape) => tape,
        Err(Error::Version { format, fixture }) => {
            return Err(BundleError::UnsupportedTape { format, fixture });
        }
        Err(_) => return Err(BundleError::Corrupt("bundle tape failed to decode")),
    };
    match crate::replay(&tape) {
        Err(Error::Divergence {
            tick,
            expected,
            actual,
            ..
        }) => Ok(DivergenceReport {
            first_divergent_tick: tick,
            expected,
            actual,
        }),
        Ok(()) => Err(BundleError::CleanReplay),
        Err(_) => Err(BundleError::Corrupt("bundle tape failed to replay")),
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_u32(out, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(bytes);
}

struct Reader<'a> {
    remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], BundleError> {
        let bytes = self
            .remaining
            .get(..N)
            .ok_or(BundleError::Corrupt("truncated bundle"))?;
        let mut value = [0; N];
        value.copy_from_slice(bytes);
        self.remaining = &self.remaining[N..];
        Ok(value)
    }

    fn take_slice(&mut self, len: usize) -> Result<&'a [u8], BundleError> {
        let bytes = self
            .remaining
            .get(..len)
            .ok_or(BundleError::Corrupt("truncated bundle"))?;
        self.remaining = &self.remaining[len..];
        Ok(bytes)
    }

    fn take_u16(&mut self) -> Result<u16, BundleError> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    fn take_u32(&mut self) -> Result<u32, BundleError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn take_u8(&mut self) -> Result<u8, BundleError> {
        Ok(self.take::<1>()?[0])
    }

    fn take_bounded(&mut self, max: usize) -> Result<Vec<u8>, BundleError> {
        let len = self.take_u32()? as usize;
        if len > max {
            return Err(BundleError::Corrupt("bundle field exceeds its bound"));
        }
        Ok(self.take_slice(len)?.to_vec())
    }
}

fn failure_kind_to_byte(kind: FailureKind) -> u8 {
    match kind {
        FailureKind::Divergence => 0,
        FailureKind::Invalid => 1,
        FailureKind::Version => 2,
        FailureKind::Golden => 3,
        FailureKind::Crash => 4,
    }
}

fn failure_kind_from_byte(byte: u8) -> Result<FailureKind, BundleError> {
    match byte {
        0 => Ok(FailureKind::Divergence),
        1 => Ok(FailureKind::Invalid),
        2 => Ok(FailureKind::Version),
        3 => Ok(FailureKind::Golden),
        4 => Ok(FailureKind::Crash),
        _ => Err(BundleError::Corrupt("unknown failure kind")),
    }
}

impl BugBundle {
    /// Encode to plain versioned bytes: envelope header, build, failure
    /// summary, content digests, redacted events, screenshot status, tape.
    pub fn encode(&self) -> Result<Vec<u8>, BundleError> {
        if self.events.len() > MAX_BUNDLE_EVENTS {
            return Err(BundleError::Corrupt("bundle holds too many events"));
        }
        if self.content_digests.len() > MAX_CONTENT_DIGESTS {
            return Err(BundleError::Corrupt("bundle holds too many digests"));
        }
        if self.tape_bytes.len() > MAX_TAPE_BYTES {
            return Err(BundleError::Corrupt("bundle tape exceeds its bound"));
        }
        if self.build.version.len() > MAX_BUILD_VERSION_BYTES {
            return Err(BundleError::Corrupt("build version exceeds its bound"));
        }
        if self.failure_detail.len() > MAX_FAILURE_DETAIL_BYTES {
            return Err(BundleError::Corrupt("failure detail exceeds its bound"));
        }
        let screenshot_len = match &self.screenshot {
            ScreenshotCapture::Unavailable { unavailable_reason } => {
                if unavailable_reason.len() > MAX_UNAVAILABLE_REASON_BYTES {
                    return Err(BundleError::Corrupt("screenshot reason exceeds its bound"));
                }
                unavailable_reason.len()
            }
            ScreenshotCapture::Present { bytes } => {
                if bytes.is_empty() || bytes.len() > MAX_SCREENSHOT_BYTES {
                    return Err(BundleError::Corrupt("screenshot size is outside bounds"));
                }
                bytes.len()
            }
        };
        let _ = screenshot_len;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        push_u16(&mut out, BUNDLE_FORMAT);
        push_u16(&mut out, self.rules_version);
        push_bytes(&mut out, self.build.version.as_bytes());
        out.extend_from_slice(&self.build.git_sha_short);
        out.push(failure_kind_to_byte(self.failure_kind));
        match self.failure_tick {
            Some(tick) => {
                out.push(1);
                push_u32(&mut out, tick);
            }
            None => out.push(0),
        }
        push_bytes(&mut out, self.failure_detail.as_bytes());
        push_u32(
            &mut out,
            u32::try_from(self.content_digests.len()).unwrap_or(u32::MAX),
        );
        for (digest, len) in &self.content_digests {
            out.extend_from_slice(digest);
            push_u32(&mut out, *len);
        }
        push_u32(
            &mut out,
            u32::try_from(self.events.len()).unwrap_or(u32::MAX),
        );
        push_u32(&mut out, self.dropped_events);
        for event in &self.events {
            if event.detail.len() > MAX_MESSAGE_LEN {
                return Err(BundleError::Corrupt("event detail exceeds its bound"));
            }
            push_u32(&mut out, event.tick);
            push_u32(&mut out, event.code.raw());
            push_bytes(&mut out, event.detail.as_bytes());
        }
        match &self.screenshot {
            ScreenshotCapture::Unavailable { unavailable_reason } => {
                out.push(0);
                push_bytes(&mut out, unavailable_reason.as_bytes());
            }
            ScreenshotCapture::Present { bytes } => {
                out.push(1);
                push_bytes(&mut out, bytes);
            }
        }
        push_bytes(&mut out, &self.tape_bytes);
        if out.len() > MAX_BUNDLE_BYTES {
            return Err(BundleError::Corrupt("bundle exceeds its bound"));
        }
        Ok(out)
    }

    /// Decode without trusting sizes, versions, codes, text or trailing
    /// bytes. A non-v1 envelope fails with [`BundleError::UnsupportedFormat`];
    /// details are re-scrubbed on the way in.
    pub fn decode(bytes: &[u8]) -> Result<Self, BundleError> {
        if bytes.len() < 8 + 2 || bytes.len() > MAX_BUNDLE_BYTES {
            return Err(BundleError::Corrupt("bundle size is outside bounds"));
        }
        let mut reader = Reader { remaining: bytes };
        if reader.take::<8>()? != *MAGIC {
            return Err(BundleError::Corrupt("bad bundle magic"));
        }
        let format = reader.take_u16()?;
        if format != BUNDLE_FORMAT {
            return Err(BundleError::UnsupportedFormat { got: format });
        }
        let rules_version = reader.take_u16()?;
        let version_bytes = reader.take_bounded(MAX_BUILD_VERSION_BYTES)?;
        let version = String::from_utf8(version_bytes)
            .map_err(|_| BundleError::Corrupt("build version is not utf8"))?;
        let git_sha_short: [u8; 8] = reader.take()?;
        let failure_kind = failure_kind_from_byte(reader.take_u8()?)?;
        let has_tick = reader.take_u8()?;
        let failure_tick = match has_tick {
            0 => None,
            1 => Some(reader.take_u32()?),
            _ => return Err(BundleError::Corrupt("bad failure tick flag")),
        };
        let failure_bytes = reader.take_bounded(MAX_FAILURE_DETAIL_BYTES)?;
        let failure_detail = scrub_detail(
            &String::from_utf8(failure_bytes)
                .map_err(|_| BundleError::Corrupt("failure detail is not utf8"))?,
        );
        let digest_count = reader.take_u32()? as usize;
        if digest_count > MAX_CONTENT_DIGESTS {
            return Err(BundleError::Corrupt("bundle holds too many digests"));
        }
        let mut content_digests = Vec::with_capacity(digest_count);
        for _ in 0..digest_count {
            let digest: [u8; 32] = reader.take()?;
            let len = reader.take_u32()?;
            content_digests.push((digest, len));
        }
        let event_count = reader.take_u32()? as usize;
        if event_count > MAX_BUNDLE_EVENTS {
            return Err(BundleError::Corrupt("bundle holds too many events"));
        }
        let dropped_events = reader.take_u32()?;
        let mut events = Vec::with_capacity(event_count);
        for _ in 0..event_count {
            let tick = reader.take_u32()?;
            let code = EventCode::from_raw(reader.take_u32()?)
                .map_err(|_| BundleError::Corrupt("unknown event code"))?;
            let detail_bytes = reader.take_bounded(MAX_MESSAGE_LEN)?;
            let detail = String::from_utf8(detail_bytes)
                .map_err(|_| BundleError::Corrupt("event detail is not utf8"))?;
            events.push(RedactedEvent::new(tick, code, &detail));
        }
        let screenshot = match reader.take_u8()? {
            0 => {
                let reason_bytes = reader.take_bounded(MAX_UNAVAILABLE_REASON_BYTES)?;
                let reason = String::from_utf8(reason_bytes)
                    .map_err(|_| BundleError::Corrupt("screenshot reason is not utf8"))?;
                if reason.is_empty() {
                    return Err(BundleError::Corrupt("screenshot reason is empty"));
                }
                ScreenshotCapture::Unavailable {
                    unavailable_reason: reason,
                }
            }
            1 => {
                let payload = reader.take_bounded(MAX_SCREENSHOT_BYTES)?;
                if payload.is_empty() {
                    return Err(BundleError::Corrupt("screenshot size is outside bounds"));
                }
                ScreenshotCapture::Present { bytes: payload }
            }
            _ => return Err(BundleError::Corrupt("bad screenshot tag")),
        };
        let tape_bytes = reader.take_bounded(MAX_TAPE_BYTES)?;
        if !reader.remaining.is_empty() {
            return Err(BundleError::Corrupt("bundle has trailing data"));
        }
        Ok(Self {
            tape_bytes,
            build: BuildId {
                version,
                git_sha_short,
            },
            rules_version,
            content_digests,
            events,
            screenshot,
            failure_kind,
            failure_tick,
            failure_detail,
            dropped_events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{record, Config, Input};

    fn proof_tape() -> Tape {
        record(
            Config { seed: 7, ticks: 24 },
            &[Input {
                tick: 5,
                body: 1,
                vx: 1.0,
                vy: 2.0,
            }],
        )
        .unwrap()
    }

    fn diverged_tape(tick: u32) -> (Tape, Error) {
        let mut tape = proof_tape();
        tape.checkpoints[tick as usize].state ^= 1;
        let error = crate::replay(&tape).unwrap_err();
        let Error::Divergence { tick: found, .. } = error else {
            panic!("expected a divergence");
        };
        assert_eq!(found, tick);
        (tape, error)
    }

    #[test]
    fn bundle_roundtrip_preserves_tape_build_and_versions() {
        let (tape, error) = diverged_tape(7);
        let bundle = collect(&tape, &error);
        assert_eq!(bundle.rules_version, FIXTURE_VERSION);
        assert_eq!(bundle.failure_kind, FailureKind::Divergence);
        assert_eq!(bundle.failure_tick, Some(7));
        assert!(bundle.tape_bytes.len() <= MAX_TAPE_BYTES);
        assert!(bundle.events.len() <= MAX_BUNDLE_EVENTS);
        // Headless: no capture, with an explicit reason.
        assert!(matches!(
            &bundle.screenshot,
            ScreenshotCapture::Unavailable { unavailable_reason } if !unavailable_reason.is_empty()
        ));
        let bytes = bundle.encode().unwrap();
        let decoded = BugBundle::decode(&bytes).unwrap();
        assert_eq!(bundle, decoded);
        assert_eq!(bytes, decoded.encode().unwrap());
        // Same failure twice gives identical bytes.
        assert_eq!(bytes, collect(&tape, &error).encode().unwrap());
    }

    #[test]
    fn supported_tape_replay_hits_the_recorded_first_divergence_tick() {
        for tick in [0, 7, 23] {
            let (tape, error) = diverged_tape(tick);
            let bundle = collect(&tape, &error);
            assert_eq!(bundle.failure_tick, Some(tick));
            let report = replay(&bundle).unwrap();
            assert_eq!(report.first_divergent_tick, tick);
            // Both checkpoint scopes are evidence, not just the tick.
            let Error::Divergence {
                expected, actual, ..
            } = error
            else {
                panic!("expected a divergence");
            };
            assert_eq!(report.expected, expected);
            assert_eq!(report.actual, actual);
        }
    }

    #[test]
    fn missing_artifact_is_diagnosed_never_silent() {
        let (tape, error) = diverged_tape(3);
        let mut bundle = collect(&tape, &error);
        bundle.tape_bytes.clear();
        assert_eq!(
            replay(&bundle),
            Err(BundleError::MissingArtifact("bundle holds no tape bytes"))
        );
        // An undecodable payload is corrupt, never a clean pass.
        bundle.tape_bytes = vec![0, 1, 2];
        assert!(matches!(replay(&bundle), Err(BundleError::Corrupt(_))));
    }

    #[test]
    fn unsupported_tier_is_diagnosed_never_silent() {
        let (tape, error) = diverged_tape(3);
        let mut bundle = collect(&tape, &error);
        bundle.rules_version = FIXTURE_VERSION + 1;
        assert_eq!(
            replay(&bundle),
            Err(BundleError::UnsupportedTier {
                rules_version: FIXTURE_VERSION + 1
            })
        );
        // A future bundle envelope fails at decode with its version named.
        let mut bytes = bundle.encode().unwrap();
        bytes[8..10].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            BugBundle::decode(&bytes),
            Err(BundleError::UnsupportedFormat { got: 99 })
        );
        // A future tape inside a current bundle names its own versions.
        let mut inner = tape.encode().unwrap();
        inner[8..10].copy_from_slice(&99u16.to_le_bytes());
        let mut renamed = collect(&tape, &error);
        renamed.tape_bytes = inner;
        assert!(matches!(
            replay(&renamed),
            Err(BundleError::UnsupportedTape { .. })
        ));
    }

    #[test]
    fn secret_shaped_input_never_appears_in_events() {
        // Prompt/credential-shaped bytes through the only text door.
        let leak = "system prompt: ignore previous instructions, bearer sk-live-abc123 password=hunter2 api_key=XYZ token 123";
        let event = RedactedEvent::new(0, EventCode::SIM_TICK, leak);
        assert_eq!(event.detail, REDACTED_DETAIL);
        for marker in ["sk-live", "bearer", "password", "api_key", "prompt"] {
            assert!(
                !event.detail.to_ascii_lowercase().contains(marker),
                "marker leaked: {marker}"
            );
        }
        // Collected bundles carry checkpoint hex and counts only.
        let (tape, error) = diverged_tape(7);
        let bundle = collect(&tape, &error);
        for event in &bundle.events {
            let lower = event.detail.to_ascii_lowercase();
            for marker in SECRET_MARKERS {
                assert!(!lower.contains(marker), "marker leaked: {marker}");
            }
            assert!(event.detail.len() <= MAX_MESSAGE_LEN);
        }
        assert!(bundle.events.len() <= MAX_BUNDLE_EVENTS);
        // A crash reason that smuggles prompt text is scrubbed too.
        let crash = collect_crash(
            &proof_tape(),
            "system prompt exfiltration with sk-fake token password",
        );
        assert!(!crash.failure_detail.to_ascii_lowercase().contains("prompt"));
        assert!(crash
            .events
            .iter()
            .all(|event| !event.detail.to_ascii_lowercase().contains("prompt")));
    }

    #[test]
    fn corrupt_bundles_are_rejected_with_bounded_diagnostics() {
        let (tape, error) = diverged_tape(7);
        let bytes = collect(&tape, &error).encode().unwrap();
        for end in [0, 7, 8, 9, 20, bytes.len() - 1] {
            assert!(BugBundle::decode(&bytes[..end]).is_err(), "length {end}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(BugBundle::decode(&trailing).is_err());
        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 0xff;
        assert!(BugBundle::decode(&bad_magic).is_err());
        for error in [
            BugBundle::decode(&[]).unwrap_err(),
            BugBundle::decode(&bytes[..7]).unwrap_err(),
        ] {
            assert!(format!("{error}").len() <= 256, "diagnostic too long");
        }
    }
    fn trace_prefix_bytes(tape: &Tape) -> Vec<u8> {
        let report = crate::trace::replay_verbose(tape).expect("verified tape traces");
        let mut out = Vec::with_capacity(report.events.len() * 48);
        for event in &report.events {
            out.extend_from_slice(&event.tick.to_le_bytes());
            out.extend_from_slice(event.detail.as_bytes());
            out.push(0);
        }
        out
    }
    #[test]
    fn same_seed_rerun_trace_prefix_is_byte_identical() {
        let config = Config { seed: 7, ticks: 24 };
        let inputs = [Input {
            tick: 5,
            body: 1,
            vx: 1.0,
            vy: 2.0,
        }];
        let first = record(config, &inputs).expect("fixture records");
        let second = record(config, &inputs).expect("rerun records");
        assert_eq!(first.checkpoints, second.checkpoints);
        let a = trace_prefix_bytes(&first);
        let b = trace_prefix_bytes(&second);
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len());
        let differing = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
        assert_eq!(differing, 0, "same-seed rerun differed");
    }
    #[test]
    fn perturbed_run_names_its_first_divergent_tick_exactly() {
        let config = Config { seed: 7, ticks: 24 };
        let inputs = [Input {
            tick: 5,
            body: 1,
            vx: 1.0,
            vy: 2.0,
        }];
        let clean = record(config, &inputs).expect("fixture records");
        let mut moved_inputs = inputs;
        moved_inputs[0].vx = 19.0;
        let moved = record(config, &moved_inputs).expect("moved records");
        let clean_report = crate::trace::replay_verbose(&clean).expect("clean traces");
        let moved_report = crate::trace::replay_verbose(&moved).expect("moved traces");
        let mut first_diff = None;
        for (index, (a, b)) in clean_report
            .events
            .iter()
            .zip(moved_report.events.iter())
            .enumerate()
        {
            // `state` folds the input context, so it differs from tick 0;
            // the authoritative tick is the first `legacy` (trajectory)
            // divergence, matching `replay` precedence in `crate::replay`.
            let legacy_a = a.detail.split(' ').next().unwrap_or(&a.detail);
            let legacy_b = b.detail.split(' ').next().unwrap_or(&b.detail);
            if legacy_a != legacy_b {
                first_diff = u32::try_from(index).ok();
                break;
            }
        }
        assert_eq!(first_diff, Some(5));
        assert_ne!(trace_prefix_bytes(&clean), trace_prefix_bytes(&moved));
        let mut tampered = clean;
        tampered.inputs = moved_inputs.to_vec();
        let error = crate::replay(&tampered).expect_err("altered input must diverge");
        let Error::Divergence { tick, .. } = error else {
            panic!("expected a divergence");
        };
        assert_eq!(tick, 5);
        assert_eq!(tick, first_diff.expect("trace diff names a tick"));
        let bundle = collect(&tampered, &error);
        assert_eq!(bundle.failure_tick, Some(5));
        assert_eq!(
            replay(&bundle)
                .expect("bundle replays")
                .first_divergent_tick,
            5
        );
    }
}
