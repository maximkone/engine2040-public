//! vendor: shared wire helpers for the HTTP adapters.
//! The four vendor adapters (OpenAI-compatible, Anthropic Messages, Gemini
//! and Ollama) share one prompt shape, one strict inner-proposal parser and
//! one adapter-local secret holder. Vendor differences stay at the envelope
//! edges: each adapter builds its own request body and extracts the assistant
//! text plus usage from its own response shape, then funnels through here.
//! [`HttpAdapter`] owns the contract flow, the roundtrip and the stream job
//! for all six HTTP adapters (the loopback and remote ones included).
//! Secrets resolve at call time from the adapter only and never enter errors,
//! events or diagnostics. Tests only use loopback stubs unless an operator
//! opts into live hosts with `E2040_AX1_*` variables.

use std::time::Instant;

use super::core::{
    validate_proposal, AdapterOutcome, ContractCore, MockStep, MAX_COORD_CM, MAX_LABEL_LEN,
};
use super::fallback::fallback_for;
use super::ladder::{
    descend, repair_text, Descent, LadderReport, Rung, RungDrop, RungFailure, RungSupport,
    MAX_LADDER_LOG, NATIVE_TOOL,
};
#[cfg(test)]
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, JobState, ProviderCapabilities};
use super::protocol::{AiError, AiRequest, PrivacyMode, Proposal, ProposalAction, Usage};
use super::stream::{
    parse_sse_stream, SourceLabel, SseFrame, StreamConfig, StreamError, StreamJob,
};
use super::transport::{self, Endpoint, TransportError};
use super::{MAX_ACTIONS, MAX_CONTEXT_REFS, MAX_TEXT_LEN};

/// Largest assistant-text payload accepted before parsing. Bounds memory on
/// hostile input the way the transport bounds response bodies; anything
/// larger is [`AiError::MalformedOutput`] without parsing.
const MAX_PROPOSAL_BYTES: usize = 65_536;

/// Minimal std-only JSON value for vendor envelopes and proposals. Objects
/// keep insertion order with last-wins on duplicates; numbers keep raw text
/// so integer checks match the strict grammar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JVal {
    /// JSON null.
    Null,
    /// JSON true or false.
    Bool(bool),
    /// JSON number, raw text as written.
    Num(String),
    /// JSON string, already unescaped.
    Str(String),
    /// JSON array.
    Arr(Vec<JVal>),
    /// JSON object as ordered pairs.
    Obj(Vec<(String, JVal)>),
}

impl JVal {
    /// Borrowed string when this value is one.
    pub(crate) fn as_str(&self) -> Option<&str> {
        if let Self::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Boolean when this value is one.
    pub(crate) fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    /// Array slice when this value is one.
    pub(crate) fn as_arr(&self) -> Option<&Vec<JVal>> {
        if let Self::Arr(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Unsigned integer for strict fields: plain decimal only, no fraction
    /// or exponent, no sign. Anything else stays unknown or malformed.
    pub(crate) fn as_u64(&self) -> Option<u64> {
        if let Self::Num(raw) = self {
            if raw.bytes().any(|b| b == b'.' || b == b'e' || b == b'E') {
                return None;
            }
            if raw.starts_with('-') {
                return None;
            }
            raw.parse::<u64>().ok()
        } else {
            None
        }
    }

    /// Signed integer for strict fields: plain decimal only.
    pub(crate) fn as_i64(&self) -> Option<i64> {
        if let Self::Num(raw) = self {
            if raw.bytes().any(|b| b == b'.' || b == b'e' || b == b'E') {
                return None;
            }
            raw.parse::<i64>().ok()
        } else {
            None
        }
    }

    /// Member lookup with last-wins, or `None` outside objects.
    pub(crate) fn get(&self, key: &str) -> Option<&JVal> {
        if let Self::Obj(pairs) = self {
            pairs.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }
}

/// Resolve a JSON pointer such as `/choices/0/message/content`. Missing
/// members, type mismatches and bad indices give `None`, never a guess.
pub(crate) fn jpointer<'a>(root: &'a JVal, pointer: &str) -> Option<&'a JVal> {
    if pointer.is_empty() {
        return Some(root);
    }
    if !pointer.starts_with('/') {
        return None;
    }
    let mut cur = root;
    for token in pointer.split('/').skip(1) {
        let key = token.replace("~1", "/").replace("~0", "~");
        if let JVal::Obj(pairs) = cur {
            let mut found: Option<&JVal> = None;
            for (k, v) in pairs.iter().rev() {
                if k == &key {
                    found = Some(v);
                    break;
                }
            }
            cur = found?;
        } else if let JVal::Arr(items) = cur {
            let idx: usize = key.parse().ok()?;
            cur = items.get(idx)?;
        } else {
            return None;
        }
    }
    Some(cur)
}

/// Parse one JSON document with only trailing whitespace allowed.
pub(crate) fn jparse(text: &str) -> Result<JVal, AiError> {
    let mut cur = JCur {
        b: text.as_bytes(),
        p: 0,
    };
    let val = cur.val(0)?;
    cur.ws();
    if cur.p != cur.b.len() {
        return Err(AiError::MalformedOutput);
    }
    Ok(val)
}

/// Byte cursor over one JSON document. All bounds misses are malformed,
/// never panics.
struct JCur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> JCur<'a> {
    fn ws(&mut self) {
        while self.p < self.b.len() {
            let c = self.b[self.p];
            if c == b' ' || c == b'\n' || c == b'\r' || c == b'\t' {
                self.p += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.p).copied()
    }

    fn val(&mut self, depth: usize) -> Result<JVal, AiError> {
        if depth > 64 {
            return Err(AiError::MalformedOutput);
        }
        self.ws();
        let c = self.peek().ok_or(AiError::MalformedOutput)?;
        match c {
            b'n' => self.lit(b"null", JVal::Null),
            b't' => self.lit(b"true", JVal::Bool(true)),
            b'f' => self.lit(b"false", JVal::Bool(false)),
            b'"' => Ok(JVal::Str(self.string()?)),
            b'[' => self.array(depth),
            b'{' => self.object(depth),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(AiError::MalformedOutput),
        }
    }

    fn lit(&mut self, word: &[u8], val: JVal) -> Result<JVal, AiError> {
        let end = self.p.saturating_add(word.len());
        if self.b.get(self.p..end) != Some(word) {
            return Err(AiError::MalformedOutput);
        }
        self.p = end;
        Ok(val)
    }

    fn hex4(&mut self) -> Result<u32, AiError> {
        let mut acc: u32 = 0;
        for _ in 0..4 {
            let c = self.peek().ok_or(AiError::MalformedOutput)?;
            let d = match c {
                b'0'..=b'9' => u32::from(c).saturating_sub(u32::from(b'0')),
                b'a'..=b'f' => u32::from(c)
                    .saturating_sub(u32::from(b'a'))
                    .saturating_add(10),
                b'A'..=b'F' => u32::from(c)
                    .saturating_sub(u32::from(b'A'))
                    .saturating_add(10),
                _ => return Err(AiError::MalformedOutput),
            };
            acc = acc.saturating_mul(16).saturating_add(d);
            self.p += 1;
        }
        Ok(acc)
    }

    fn string(&mut self) -> Result<String, AiError> {
        // Consume the opening quote; the caller peeked it.
        self.p += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or(AiError::MalformedOutput)?;
            if c == b'"' {
                self.p += 1;
                break;
            }
            if c < 0x20 {
                return Err(AiError::MalformedOutput);
            }
            if c != b'\\' {
                out.push(c);
                self.p += 1;
                continue;
            }
            self.p += 1;
            let e = self.peek().ok_or(AiError::MalformedOutput)?;
            match e {
                b'"' => {
                    out.push(b'"');
                    self.p += 1;
                }
                b'\\' => {
                    out.push(b'\\');
                    self.p += 1;
                }
                b'/' => {
                    out.push(b'/');
                    self.p += 1;
                }
                b'b' => {
                    out.push(0x08);
                    self.p += 1;
                }
                b'f' => {
                    out.push(0x0c);
                    self.p += 1;
                }
                b'n' => {
                    out.push(b'\n');
                    self.p += 1;
                }
                b'r' => {
                    out.push(b'\r');
                    self.p += 1;
                }
                b't' => {
                    out.push(b'\t');
                    self.p += 1;
                }
                b'u' => {
                    self.p += 1;
                    let hi = self.hex4()?;
                    let point = if (0xd800..0xdc00).contains(&hi) {
                        if self.peek() != Some(b'\\') {
                            return Err(AiError::MalformedOutput);
                        }
                        self.p += 1;
                        if self.peek() != Some(b'u') {
                            return Err(AiError::MalformedOutput);
                        }
                        self.p += 1;
                        let lo = self.hex4()?;
                        if !(0xdc00..0xe000).contains(&lo) {
                            return Err(AiError::MalformedOutput);
                        }
                        let high = hi.saturating_sub(0xd800);
                        let low = lo.saturating_sub(0xdc00);
                        high.saturating_mul(0x400)
                            .saturating_add(low)
                            .saturating_add(0x1_0000)
                    } else if (0xdc00..0xe000).contains(&hi) {
                        return Err(AiError::MalformedOutput);
                    } else {
                        hi
                    };
                    let ch = char::from_u32(point).ok_or(AiError::MalformedOutput)?;
                    let mut buf = [0u8; 4];
                    let text = ch.encode_utf8(&mut buf);
                    out.extend_from_slice(text.as_bytes());
                }
                _ => return Err(AiError::MalformedOutput),
            }
        }
        String::from_utf8(out).map_err(|_| AiError::MalformedOutput)
    }

    fn number(&mut self) -> Result<JVal, AiError> {
        let start = self.p;
        if self.peek() == Some(b'-') {
            self.p += 1;
        }
        let first = self.peek().ok_or(AiError::MalformedOutput)?;
        if first == b'0' {
            self.p += 1;
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(AiError::MalformedOutput);
            }
        } else if first.is_ascii_digit() {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.p += 1;
            }
        } else {
            return Err(AiError::MalformedOutput);
        }
        if self.peek() == Some(b'.') {
            self.p += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(AiError::MalformedOutput);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.p += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.p += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.p += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(AiError::MalformedOutput);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.p += 1;
            }
        }
        let raw = self.b.get(start..self.p).ok_or(AiError::MalformedOutput)?;
        let text = core::str::from_utf8(raw).map_err(|_| AiError::MalformedOutput)?;
        Ok(JVal::Num(text.to_owned()))
    }

    fn array(&mut self, depth: usize) -> Result<JVal, AiError> {
        self.p += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.p += 1;
            return Ok(JVal::Arr(items));
        }
        loop {
            items.push(self.val(depth.saturating_add(1))?);
            self.ws();
            let c = self.peek().ok_or(AiError::MalformedOutput)?;
            if c == b',' {
                self.p += 1;
                self.ws();
                if self.peek() == Some(b']') {
                    return Err(AiError::MalformedOutput);
                }
            } else if c == b']' {
                self.p += 1;
                break;
            } else {
                return Err(AiError::MalformedOutput);
            }
        }
        Ok(JVal::Arr(items))
    }

    fn object(&mut self, depth: usize) -> Result<JVal, AiError> {
        self.p += 1;
        let mut pairs: Vec<(String, JVal)> = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.p += 1;
            return Ok(JVal::Obj(pairs));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(AiError::MalformedOutput);
            }
            let key = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(AiError::MalformedOutput);
            }
            self.p += 1;
            let val = self.val(depth.saturating_add(1))?;
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == &key) {
                slot.1 = val;
            } else {
                pairs.push((key, val));
            }
            self.ws();
            let c = self.peek().ok_or(AiError::MalformedOutput)?;
            if c == b',' {
                self.p += 1;
                self.ws();
                if self.peek() == Some(b'}') {
                    return Err(AiError::MalformedOutput);
                }
            } else if c == b'}' {
                self.p += 1;
                break;
            } else {
                return Err(AiError::MalformedOutput);
            }
        }
        Ok(JVal::Obj(pairs))
    }
}

/// Prompt text for one engine request. Carries the task kind, revision and
/// context digests only: references are digests, never raw prompts, assets
/// or secrets.
pub(crate) fn prompt_text(request: &AiRequest) -> String {
    let mut prompt = format!(
        "task={:?} revision={} contexts={} budget_ms={}",
        request.kind,
        request.base_revision,
        request.context.len(),
        request.budget.max_ms
    );
    for reference in request.context.iter().take(MAX_CONTEXT_REFS) {
        prompt.push_str(" digest=");
        for byte in reference.digest {
            prompt.push_str(&format!("{byte:02x}"));
        }
    }
    if prompt.len() > MAX_TEXT_LEN {
        prompt.truncate(MAX_TEXT_LEN);
    }
    prompt
}

/// Strictly validate a proposal from assistant text. The text must itself be
/// a closed-grammar proposal object with known action shapes, bounded labels,
/// coordinates and diagnostics, then [`validate_proposal`]. Unknown keys,
/// extra fields, wrong types and out-of-range values are
/// [`AiError::MalformedOutput`]. Text parsing never claims native
/// structured output. `receipt` (a host label from the caller, never a
/// secret) is appended so a roundtrip receipt names where it ran.
pub(crate) fn parse_proposal_content(
    content: &str,
    request: &AiRequest,
    receipt: &str,
) -> Result<Proposal, AiError> {
    if content.len() > MAX_PROPOSAL_BYTES {
        return Err(AiError::MalformedOutput);
    }
    let inner = jparse(content.trim())?;
    parse_proposal_value(&inner, request, receipt)
}

/// Strict proposal check over an already parsed value. The text entry point
/// above parses first; the native tool rung calls this directly on the forced
/// call input so no second serialization is needed.
pub(crate) fn parse_proposal_value(
    inner: &JVal,
    request: &AiRequest,
    receipt: &str,
) -> Result<Proposal, AiError> {
    let JVal::Obj(pairs) = inner else {
        return Err(AiError::MalformedOutput);
    };
    for (key, _) in pairs {
        if key != "actions" && key != "diagnostics" {
            return Err(AiError::MalformedOutput);
        }
    }
    let mut actions = Vec::new();
    if let Some(list) = inner.get("actions") {
        let items = list.as_arr().ok_or(AiError::MalformedOutput)?;
        for item in items {
            actions.push(parse_action(item)?);
            if actions.len() > MAX_ACTIONS {
                return Err(AiError::MalformedOutput);
            }
        }
    }
    let mut diagnostics = Vec::new();
    if let Some(list) = inner.get("diagnostics") {
        let items = list.as_arr().ok_or(AiError::MalformedOutput)?;
        for item in items {
            let line = item.as_str().ok_or(AiError::MalformedOutput)?;
            if line.len() > MAX_TEXT_LEN {
                return Err(AiError::MalformedOutput);
            }
            diagnostics.push(line.to_owned());
            if diagnostics.len() > MAX_ACTIONS {
                return Err(AiError::MalformedOutput);
            }
        }
    }
    if diagnostics.len() < MAX_ACTIONS && is_receipt(receipt) {
        diagnostics.push(receipt.to_owned());
    }
    let proposal = Proposal {
        request: request.id,
        base_revision: request.base_revision,
        actions,
        object_versions: Vec::new(),
        diagnostics,
    };
    validate_proposal(&proposal)?;
    Ok(proposal)
}

/// True when a receipt may join the diagnostics: non-empty, bounded and
/// single-line. Receipts name the loopback or vendor host only.
fn is_receipt(receipt: &str) -> bool {
    if receipt.is_empty() || receipt.len() > MAX_TEXT_LEN {
        return false;
    }
    !receipt.chars().any(|c| c.is_control())
}

/// Parse one closed-grammar action. The object must hold exactly one known
/// key with its exact body shape; anything else is malformed.
fn parse_action(item: &JVal) -> Result<ProposalAction, AiError> {
    let JVal::Obj(pairs) = item else {
        return Err(AiError::MalformedOutput);
    };
    if pairs.len() != 1 {
        return Err(AiError::MalformedOutput);
    }
    if let Some(flag) = item.get("noop") {
        if flag.as_bool() == Some(true) {
            return Ok(ProposalAction::Noop);
        }
        return Err(AiError::MalformedOutput);
    }
    if let Some(target) = item.get("set_position") {
        let JVal::Obj(body) = target else {
            return Err(AiError::MalformedOutput);
        };
        if body.len() != 2 || target.get("object").is_none() || target.get("xyz_cm").is_none() {
            return Err(AiError::MalformedOutput);
        }
        let id = target
            .get("object")
            .and_then(JVal::as_u64)
            .ok_or(AiError::MalformedOutput)?;
        let axes = target
            .get("xyz_cm")
            .and_then(JVal::as_arr)
            .ok_or(AiError::MalformedOutput)?;
        if axes.len() != 3 {
            return Err(AiError::MalformedOutput);
        }
        let mut xyz_cm = [0i32; 3];
        for (slot, value) in xyz_cm.iter_mut().zip(axes.iter()) {
            let raw = value.as_i64().ok_or(AiError::MalformedOutput)?;
            let narrowed = i32::try_from(raw).map_err(|_| AiError::MalformedOutput)?;
            if i64::from(narrowed).abs() > i64::from(MAX_COORD_CM) {
                return Err(AiError::MalformedOutput);
            }
            *slot = narrowed;
        }
        return Ok(ProposalAction::SetPosition { object: id, xyz_cm });
    }
    if let Some(target) = item.get("set_label") {
        let JVal::Obj(body) = target else {
            return Err(AiError::MalformedOutput);
        };
        if body.len() != 2 || target.get("object").is_none() || target.get("label").is_none() {
            return Err(AiError::MalformedOutput);
        }
        let id = target
            .get("object")
            .and_then(JVal::as_u64)
            .ok_or(AiError::MalformedOutput)?;
        let label = target
            .get("label")
            .and_then(JVal::as_str)
            .ok_or(AiError::MalformedOutput)?;
        if label.is_empty()
            || label.len() > MAX_LABEL_LEN
            || label.chars().count() > MAX_LABEL_LEN
            || label.chars().any(|c| c.is_control())
        {
            return Err(AiError::MalformedOutput);
        }
        return Ok(ProposalAction::SetLabel {
            object: id,
            label: label.to_owned(),
        });
    }
    Err(AiError::MalformedOutput)
}

/// Adapter-local secret reference shared by the cloud vendor adapters. The
/// value leaves this struct only as header bytes on the wire: `Debug` prints
/// the reference kind (and the variable name for `EnvVar`), never the value.
#[derive(Clone, PartialEq, Eq)]
pub enum VendorSecret {
    /// Read the token from this environment variable at call time.
    EnvVar(String),
    /// Token held in memory (tests and operator tooling). Never printed.
    Direct(String),
}

impl std::fmt::Debug for VendorSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VendorSecret::EnvVar(name) => write!(f, "EnvVar({name:?})"),
            VendorSecret::Direct(_) => f.write_str("Direct(<redacted>)"),
        }
    }
}

impl VendorSecret {
    pub(crate) fn resolve(&self) -> Result<String, super::remote::SecretError> {
        match self {
            VendorSecret::EnvVar(name) => match std::env::var(name) {
                Ok(token) if !token.is_empty() => Ok(token),
                _ => Err(super::remote::SecretError::Unavailable),
            },
            VendorSecret::Direct(token) => {
                if token.is_empty() {
                    Err(super::remote::SecretError::Unavailable)
                } else {
                    Ok(token.clone())
                }
            }
        }
    }
}

/// Response fields of one vendor envelope: JSON pointers to the assistant
/// text and the two usage counters.
#[derive(Debug)]
pub(crate) struct Envelope {
    /// Pointer to the assistant text.
    pub(crate) text: &'static str,
    /// Pointer to the input-token count.
    pub(crate) input_tokens: &'static str,
    /// Pointer to the output-token count.
    pub(crate) output_tokens: &'static str,
    /// True when the vendor's native structured output arrives as a forced
    /// tool call (a `tool_use` content block named [`NATIVE_TOOL`]) rather
    /// than as text.
    pub(crate) native_tool: bool,
}

/// The chat-completions envelope shared by OpenAI, the local sidecar and the
/// configured remote host.
pub(crate) const CHAT_ENVELOPE: Envelope = Envelope {
    text: "/choices/0/message/content",
    input_tokens: "/usage/prompt_tokens",
    output_tokens: "/usage/completion_tokens",
    native_tool: false,
};

/// Extract and strictly validate a proposal from a vendor response body.
/// Usage is recorded only when the server reports it; otherwise it stays
/// unknown, never zero-claimed.
pub(crate) fn parse_envelope(
    envelope: &Envelope,
    body: &str,
    request: &AiRequest,
    receipt: &str,
) -> Parsed {
    parse_envelope_at(envelope, body, request, receipt, Rung::JsonMode)
}

/// The model's answer text in `outer` for text rungs.
fn answer_text(envelope: &Envelope, outer: &JVal) -> Result<String, AiError> {
    jpointer(outer, envelope.text)
        .and_then(JVal::as_str)
        .map(str::to_owned)
        .ok_or(AiError::MalformedOutput)
}

/// [`parse_envelope`] for one rung. Every rung ends in the strict grammar;
/// only the text rung may first run the bounded repair, and the repaired text
/// goes through the same strict parse.
pub(crate) fn parse_envelope_at(
    envelope: &Envelope,
    body: &str,
    request: &AiRequest,
    receipt: &str,
    rung: Rung,
) -> Parsed {
    let outer = jparse(body)?;
    if rung == Rung::Native && envelope.native_tool {
        let blocks = jpointer(&outer, "/content")
            .and_then(JVal::as_arr)
            .ok_or(AiError::MalformedOutput)?;
        let mut input: Option<&JVal> = None;
        for block in blocks {
            let is_call = block.get("type").and_then(JVal::as_str) == Some("tool_use")
                && block.get("name").and_then(JVal::as_str) == Some(NATIVE_TOOL);
            if is_call {
                input = block.get("input");
                break;
            }
        }
        let proposal =
            parse_proposal_value(input.ok_or(AiError::MalformedOutput)?, request, receipt)
                .map_err(|_| AiError::MalformedOutput)?;
        let usage = Usage {
            input_tokens: jpointer(&outer, envelope.input_tokens).and_then(JVal::as_u64),
            output_tokens: jpointer(&outer, envelope.output_tokens).and_then(JVal::as_u64),
        };
        return Ok((proposal, usage));
    }
    let content = answer_text(envelope, &outer)?;
    let proposal = match parse_proposal_content(&content, request, receipt) {
        Ok(proposal) => proposal,
        Err(error) if rung == Rung::Text => {
            let fixed = repair_text(&content).ok_or(error)?;
            parse_proposal_content(&fixed, request, receipt)?
        }
        Err(error) => return Err(error),
    };
    let usage = Usage {
        input_tokens: jpointer(&outer, envelope.input_tokens).and_then(JVal::as_u64),
        output_tokens: jpointer(&outer, envelope.output_tokens).and_then(JVal::as_u64),
    };
    Ok((proposal, usage))
}

/// A validated proposal plus the usage the server reported.
pub(crate) type Parsed = Result<(Proposal, Usage), AiError>;

/// How one HTTP adapter speaks on the wire.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wire {
    /// Receipt prefix naming the adapter (`anthropic`, `local`, ...).
    pub(crate) label: &'static str,
    /// Request body for one engine request under the adapter's pins.
    pub(crate) body: fn(&AiRequest, &RequestPins) -> String,
    /// Where a 200 body keeps the answer text and usage counters.
    pub(crate) envelope: &'static Envelope,
    /// True for cloud adapters: a completed exchange is a remote dispatch.
    pub(crate) remote: bool,
    /// Which structured-output rungs the vendor offers.
    pub(crate) rungs: RungSupport,
}

/// Streaming side of a vendor wire: the body and the SSE delta extractor.
#[derive(Clone, Copy)]
pub(crate) struct StreamWire {
    /// Streaming request body for one engine request under the adapter's pins.
    pub(crate) body: fn(&AiRequest, &RequestPins) -> String,
    /// Text delta carried by one SSE frame, if any.
    pub(crate) text: fn(&SseFrame) -> Option<String>,
}

/// Contract state, endpoint and dial counters shared by every HTTP adapter.
/// One owner for the roundtrip, the contract `call` flow and the stream job,
/// so adapters differ only in their [`Wire`] and credential.
#[derive(Clone, Debug)]
pub(crate) struct HttpAdapter {
    pub(crate) core: ContractCore,
    pub(crate) endpoint: Endpoint,
    pub(crate) pins: RequestPins,
    pub(crate) socket_attempts: u64,
    pub(crate) transport_calls: u64,
    /// What the latest roundtrip did on the ladder.
    pub(crate) last_report: Option<LadderReport>,
    /// Rung drops, oldest first, bounded by [`MAX_LADDER_LOG`].
    pub(crate) ladder_log: Vec<RungDrop>,
}

impl HttpAdapter {
    /// Fresh state that plays `script` against `endpoint`.
    pub(crate) fn new(script: Vec<MockStep>, endpoint: Endpoint) -> Self {
        Self {
            core: ContractCore::new(script),
            endpoint,
            pins: RequestPins::default(),
            socket_attempts: 0,
            transport_calls: 0,
            last_report: None,
            ladder_log: Vec::new(),
        }
    }

    /// Contract flow shared by every HTTP adapter: replay, offline refusal
    /// before any socket attempt, guards, then the script or one roundtrip.
    pub(crate) fn call(
        &mut self,
        wire: &Wire,
        auth: Option<&VendorSecret>,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        // Same request identity twice returns the recorded first result and
        // performs no second mutation, without consuming script.
        if let Some(replay) = self.core.replay(request.id) {
            return replay;
        }
        self.last_report = None;
        // Offline means no provider network calls at all, loopback included:
        // refuse before any socket attempt.
        if request.privacy == PrivacyMode::Offline {
            return self.core.fail(
                request,
                AiError::UnsupportedCapability,
                JobState::Done,
                false,
            );
        }
        if let Err((error, state)) = self.core.precheck(request, policy, capabilities) {
            return self.core.fail(request, error, state, false);
        }
        match self.core.next_step() {
            MockStep::Succeed => self.roundtrip(wire, auth, request, policy),
            step => match self.core.scripted(request, step) {
                Some(outcome) => outcome,
                None => self
                    .core
                    .fail(request, AiError::MalformedOutput, JobState::Done, false),
            },
        }
    }

    /// One request on the ladder. An unusable credential is a quota refusal
    /// with no socket attempt and no payload in the outcome. Each rung is one
    /// HTTP exchange under what is left of the request's budget; a malformed
    /// answer or a refused request shape drops one rung, anything else ends on
    /// the scripted floor, where callers apply scripted behavior on top.
    /// Loopback adapters pass no credential and hold no remote endpoint.
    fn roundtrip(
        &mut self,
        wire: &Wire,
        auth: Option<&VendorSecret>,
        request: &AiRequest,
        policy: &AiSessionPolicy,
    ) -> AdapterOutcome {
        let token = match auth.map(VendorSecret::resolve).transpose() {
            Ok(token) => token,
            Err(_) => {
                return self
                    .core
                    .fail(request, AiError::QuotaAuth, JobState::Done, false);
            }
        };
        let receipt = format!("{}:{}", wire.label, self.endpoint.host());
        let started = Instant::now();
        let plan = self.pins.ladder();
        let Descent { result, report } = descend(request.id, plan, wire.rungs, |rung| {
            let spent = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let left = request.budget.max_ms.saturating_sub(spent);
            self.exchange(wire, token.as_deref(), request, rung, left, &receipt)
        });
        let outcome = match result {
            Ok((proposal, usage)) => self
                .core
                .succeed_with(request, proposal, usage, wire.remote),
            Err(failure) => self.core.fail(
                request,
                failure.error,
                JobState::Done,
                wire.remote && failure.exchanged,
            ),
        };
        let scripted = match &outcome.result {
            Err(error) if report.used == Rung::Scripted => {
                fallback_for(*error, request, policy).ok()
            }
            _ => None,
        };
        self.note_ladder(LadderReport { scripted, ..report });
        outcome
    }

    /// One rung's HTTP exchange: the pinned body for `rung`, one POST within
    /// `left_ms`, and the strict parse of what came back.
    fn exchange(
        &mut self,
        wire: &Wire,
        token: Option<&str>,
        request: &AiRequest,
        rung: Rung,
        left_ms: u64,
        receipt: &str,
    ) -> Result<(Proposal, Usage), RungFailure> {
        if left_ms == 0 {
            return Err(RungFailure::fatal(AiError::Timeout, false));
        }
        let body = (wire.body)(request, &self.pins.at(rung));
        self.socket_attempts += 1;
        match transport::post_json(&self.endpoint, &body, token, left_ms) {
            Ok(response) => {
                self.transport_calls += 1;
                if response.status != 200 {
                    return Err(rung_status_failure(response.status));
                }
                parse_envelope_at(wire.envelope, &response.body, request, receipt, rung)
                    .map_err(|_| RungFailure::malformed())
            }
            Err(TransportError::Timeout) => Err(RungFailure::fatal(AiError::Timeout, false)),
            // Endpoint change without session authorization: refuse per
            // policy, never follow.
            Err(TransportError::Redirect) => {
                Err(RungFailure::fatal(AiError::UnsupportedCapability, false))
            }
            Err(_) => Err(RungFailure::fatal(AiError::Unavailable, false)),
        }
    }

    /// Keep the latest report and append its drops to the bounded log.
    pub(crate) fn note_ladder(&mut self, report: LadderReport) {
        for drop in &report.drops {
            if self.ladder_log.len() >= MAX_LADDER_LOG {
                self.ladder_log.remove(0);
            }
            self.ladder_log.push(*drop);
        }
        self.last_report = Some(report);
    }

    /// Completed HTTP exchanges as the `u32` remote-contact counter of a
    /// cloud adapter (saturating).
    pub(crate) fn remote_calls(&self) -> u32 {
        u32::try_from(self.transport_calls).unwrap_or(u32::MAX)
    }

    /// Spawn a tick-isolated SSE stream job against the endpoint. POSTs the
    /// streaming body on the worker thread and feeds text deltas through
    /// [`StreamJob`] so the tick only polls: no network call happens on the
    /// tick. Offline privacy is refused before spawning; loopback hosts are
    /// labelled recorded (a stub loopback is never called live), any other
    /// host is labelled live.
    pub(crate) fn stream_job(
        &self,
        auth: &VendorSecret,
        wire: StreamWire,
        request: &AiRequest,
        config: &StreamConfig,
    ) -> Result<StreamJob, StreamError> {
        if request.privacy == PrivacyMode::Offline {
            return Err(StreamError::Offline);
        }
        let endpoint = self.endpoint.clone();
        let auth = auth.clone();
        let body = (wire.body)(request, &self.pins);
        let transport_timeout = request.budget.max_ms.clamp(1, 30_000);
        let source = if transport::is_loopback(endpoint.host()) {
            SourceLabel::Recorded
        } else {
            SourceLabel::Live
        };
        StreamJob::spawn(config, source, move |feed| {
            // Cancel-before-dial fast path: when the tick already cancelled
            // (worker start delayed under load), report Cancelled without
            // dialing.
            if feed.is_cancelled() {
                feed.fail(StreamError::Cancelled);
                return;
            }
            let Ok(token) = auth.resolve() else {
                feed.fail(StreamError::Unavailable);
                return;
            };
            match transport::post_json(&endpoint, &body, Some(&token), transport_timeout) {
                Ok(response) => {
                    if response.status != 200 {
                        feed.fail(status_failure(response.status));
                        return;
                    }
                    let mut pushed = 0u32;
                    for frame in parse_sse_stream(&response.body) {
                        if let Some(delta) = (wire.text)(&frame) {
                            if feed.push(&delta).is_err() {
                                return;
                            }
                            pushed += 1;
                        }
                    }
                    if pushed == 0 {
                        feed.fail(StreamError::MalformedOutput);
                    } else {
                        feed.finish();
                    }
                }
                Err(error) => feed.fail(transport_failure(error)),
            }
        })
    }
}

/// Delegators every HTTP adapter exposes over its [`HttpAdapter`] state (the
/// `http` field): endpoint, contract controls and dial counters. One owner
/// for the shared surface, so adapters add only what differs.
macro_rules! http_adapter_api {
    () => {
        /// Configured endpoint.
        pub fn endpoint(&self) -> &$crate::transport::Endpoint {
            &self.http.endpoint
        }

        /// The pins every request this adapter sends carries.
        pub fn pins(&self) -> &$crate::pins::RequestPins {
            &self.http.pins
        }

        /// Replace the pins (system prompt, temperature, seed, tools).
        #[must_use]
        pub fn with_pins(mut self, pins: $crate::pins::RequestPins) -> Self {
            self.http.pins = pins;
            self
        }

        /// Pin the live document/world revision.
        pub fn set_revision(&mut self, revision: u64) {
            self.http.core.set_revision(revision);
        }

        /// Stop enforcing revision freshness.
        pub fn clear_revision_check(&mut self) {
            self.http.core.clear_revision_check();
        }

        /// Pretend `count` requests are already in flight.
        pub fn set_in_flight(&mut self, count: u32) {
            self.http.core.set_in_flight(count);
        }

        /// Track a job as running so cancel/late-output tests have a live job.
        pub fn begin_job(&mut self, id: $crate::protocol::RequestId) {
            self.http.core.begin_job(id);
        }

        /// Cancel a job with a reason.
        pub fn cancel_job(
            &mut self,
            id: $crate::protocol::RequestId,
            reason: $crate::policy::CancelReason,
        ) {
            self.http.core.cancel_job(id, reason);
        }

        /// Cancel as the calling user.
        pub fn cancel(&mut self, id: $crate::protocol::RequestId) {
            self.http.core.cancel(id);
        }

        /// Offer one event to a job after the fact.
        pub fn deliver_late(
            &self,
            id: $crate::protocol::RequestId,
            event: &$crate::protocol::AiEvent,
        ) -> $crate::core::LateOutcome {
            self.http.core.deliver_late(id, event)
        }

        /// Script steps not yet consumed.
        pub fn script_remaining(&self) -> usize {
            self.http.core.script_remaining()
        }

        /// Calls that newly produced a complete proposal.
        pub fn mutations(&self) -> u64 {
            self.http.core.mutations()
        }

        /// What the latest request did on the structured-output ladder:
        /// the rung that served and every drop. `None` before the first
        /// roundtrip and after a call refused before dispatch.
        pub fn ladder_report(&self) -> Option<&$crate::ladder::LadderReport> {
            self.http.last_report.as_ref()
        }

        /// The rung that served the latest request.
        pub fn last_rung(&self) -> Option<$crate::ladder::Rung> {
            self.http.last_report.as_ref().map(|report| report.used)
        }

        /// Every rung drop so far, oldest first (bounded).
        pub fn ladder_events(&self) -> &[$crate::ladder::RungDrop] {
            &self.http.ladder_log
        }

        /// TCP connect attempts. Refusals before the socket leave this at zero.
        pub fn socket_attempts(&self) -> u64 {
            self.http.socket_attempts
        }

        /// Completed HTTP exchanges with the configured endpoint.
        pub fn transport_calls(&self) -> u64 {
            self.http.transport_calls
        }
    };
}
pub(crate) use http_adapter_api;

/// One [`crate::suite::SuiteAdapter`] impl for every [`HttpAdapter`]-backed
/// adapter. The ten delegators only forward to the shared contract surface,
/// so they compile once here instead of once per adapter file. Test-only.
#[cfg(test)]
macro_rules! impl_suite_adapter {
    ($t:ty) => {
        impl $crate::suite::SuiteAdapter for $t {
            fn t_cancel(&mut self, id: $crate::protocol::RequestId) {
                self.cancel(id);
            }
            fn t_begin_job(&mut self, id: $crate::protocol::RequestId) {
                self.begin_job(id);
            }
            fn t_cancel_job(
                &mut self,
                id: $crate::protocol::RequestId,
                reason: $crate::policy::CancelReason,
            ) {
                self.cancel_job(id, reason);
            }
            fn t_deliver_late(
                &self,
                id: $crate::protocol::RequestId,
                event: &$crate::protocol::AiEvent,
            ) -> $crate::core::LateOutcome {
                self.deliver_late(id, event)
            }
            fn t_set_revision(&mut self, revision: u64) {
                self.set_revision(revision);
            }
            fn t_set_in_flight(&mut self, count: u32) {
                self.set_in_flight(count);
            }
            fn t_script_remaining(&self) -> usize {
                self.script_remaining()
            }
            fn t_mutations(&self) -> u64 {
                self.mutations()
            }
            fn t_socket_attempts(&self) -> u64 {
                self.socket_attempts()
            }
            fn t_remote_calls(&self) -> u32 {
                self.remote_calls()
            }
        }
    };
}
#[cfg(test)]
pub(crate) use impl_suite_adapter;

/// One shared shell for the HTTP vendor adapters: an owned [`HttpAdapter`]
/// plus one [`Wire`] and an optional adapter-local credential. Backends differ
/// only in their wire and credential, so one struct plus one suite impl covers
/// all six instead of one struct per vendor. Test-only: the shared shell
/// exists so the stub roundtrip exercises the generic path; production
/// adapters keep their own shells.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct GenericVendorAdapter {
    http: HttpAdapter,
    auth: Option<VendorSecret>,
    wire: Wire,
}

#[cfg(test)]
impl GenericVendorAdapter {
    /// Shell that plays `script` against `endpoint` with `wire`.
    pub(crate) fn new(
        script: Vec<MockStep>,
        endpoint: Endpoint,
        wire: Wire,
        auth: Option<VendorSecret>,
    ) -> Self {
        Self {
            http: HttpAdapter::new(script, endpoint),
            auth,
            wire,
        }
    }

    /// Cloud shell with an adapter-local env-var credential.
    pub(crate) fn with_env_token(
        script: Vec<MockStep>,
        endpoint: Endpoint,
        wire: Wire,
        env_var: &str,
    ) -> Self {
        Self::new(
            script,
            endpoint,
            wire,
            Some(VendorSecret::EnvVar(env_var.to_owned())),
        )
    }

    http_adapter_api!();

    /// Completed exchanges as remote contacts. Loopback wires stay at zero.
    pub(crate) fn remote_calls(&self) -> u32 {
        if self.wire.remote {
            self.http.remote_calls()
        } else {
            0
        }
    }

    /// Tick-isolated SSE stream job with `stream` (see [`HttpAdapter::stream_job`]).
    pub(crate) fn stream_job(
        &self,
        auth: &VendorSecret,
        stream: StreamWire,
        request: &AiRequest,
        config: &StreamConfig,
    ) -> Result<StreamJob, StreamError> {
        self.http.stream_job(auth, stream, request, config)
    }
}

#[cfg(test)]
impl ProviderAdapter for GenericVendorAdapter {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        self.http.call(
            &self.wire,
            self.auth.as_ref(),
            request,
            policy,
            capabilities,
        )
    }

    fn request_pins(&self) -> Option<&RequestPins> {
        Some(&self.http.pins)
    }
}

#[cfg(test)]
impl_suite_adapter!(GenericVendorAdapter);

/// A completed non-200 status as a rung failure: a refused request shape
/// (400, 422) may be served by a lower rung; auth, quota, timeout and server
/// trouble may not.
fn rung_status_failure(status: u16) -> RungFailure {
    let error = transport::map_status(status);
    match status {
        400 | 422 => RungFailure::rejected(error),
        _ => RungFailure::fatal(error, true),
    }
}

/// Map a completed non-200 status to its stream failure. Auth/quota refusals
/// have no stream variant and surface as retryable [`StreamError::Unavailable`];
/// endpoint changes refused per policy surface as [`StreamError::Offline`].
fn status_failure(status: u16) -> StreamError {
    match transport::map_status(status) {
        AiError::Timeout => StreamError::Timeout,
        AiError::UnsupportedCapability => StreamError::Offline,
        _ => StreamError::Unavailable,
    }
}

/// Map a transport failure to its stream failure. Redirects are refused per
/// policy as [`StreamError::Offline`]; timeouts stay timeouts.
fn transport_failure(error: TransportError) -> StreamError {
    match error {
        TransportError::Timeout => StreamError::Timeout,
        TransportError::Redirect => StreamError::Offline,
        _ => StreamError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::ProposalAction;
    use super::*;

    fn request() -> AiRequest {
        crate::mock::fixtures::request(7, 3)
    }

    #[test]
    fn grammar_accepts_known_shapes() {
        let base = request();
        let noop = parse_proposal_content(
            r#"{"actions": [{"noop": true}], "diagnostics": ["stub:ok"]}"#,
            &base,
            "mock:ok",
        )
        .expect("noop validates");
        assert_eq!(noop.actions, vec![ProposalAction::Noop]);
        assert!(noop.diagnostics.contains(&String::from("mock:ok")));

        let moved = parse_proposal_content(
            r#"{"actions": [{"set_position": {"object": 2, "xyz_cm": [10, -20, 30]}}], "diagnostics": []}"#,
            &base,
            "local:127.0.0.1",
        )
        .expect("position validates");
        assert_eq!(
            moved.actions,
            vec![ProposalAction::SetPosition {
                object: 2,
                xyz_cm: [10, -20, 30]
            }]
        );

        let labeled = parse_proposal_content(
            r#"{"actions": [{"set_label": {"object": 3, "label": "gate"}}], "diagnostics": []}"#,
            &base,
            "remote:127.0.0.1",
        )
        .expect("label validates");
        assert_eq!(
            labeled.actions,
            vec![ProposalAction::SetLabel {
                object: 3,
                label: String::from("gate")
            }]
        );

        let empty =
            parse_proposal_content(r#"{"actions": [], "diagnostics": []}"#, &base, "mock:ok")
                .expect("empty validates");
        assert!(empty.actions.is_empty());
    }

    #[test]
    fn grammar_rejects_unknown_extra_and_unbounded() {
        let base = request();
        let cases = [
            // Unknown action shape.
            r#"{"actions": [{"teleport": {"object": 1}}], "diagnostics": []}"#,
            // Extra outer key on the action.
            r#"{"actions": [{"noop": true, "extra": 1}], "diagnostics": []}"#,
            // Noop must be exactly true.
            r#"{"actions": [{"noop": false}], "diagnostics": []}"#,
            r#"{"actions": [{"noop": 1}], "diagnostics": []}"#,
            // Empty and multi-key actions.
            r#"{"actions": [{}], "diagnostics": []}"#,
            r#"{"actions": [{"noop": true, "set_label": {"object": 1, "label": "x"}}], "diagnostics": []}"#,
            // Position: extra body field, wrong arity, float, overflow, far.
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [0, 0, 0], "speed": 9}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [0, 0]}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [0.5, 0, 0]}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [2147483648, 0, 0]}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [1000001, 0, 0]}}], "diagnostics": []}"#,
            // Label: empty, overlong, control, extra field.
            r#"{"actions": [{"set_label": {"object": 1, "label": ""}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_label": {"object": 1, "label": "ok", "extra": 1}}], "diagnostics": []}"#,
            // Unknown top-level keys smuggle state.
            r#"{"actions": [], "diagnostics": [], "object_versions": [[1, 2]]}"#,
            r#"{"actions": [], "diagnostics": [], "free_text": "hello"}"#,
            // Diagnostics shape.
            r#"{"actions": [], "diagnostics": "ok"}"#,
            r#"{"actions": [], "diagnostics": [42]}"#,
            // Envelope shape.
            r#"[1, 2, 3]"#,
            r#"{"actions": [{"noop": true}], "diagnostics": []} trailing"#,
        ];
        for (index, body) in cases.iter().enumerate() {
            assert_eq!(
                parse_proposal_content(body, &base, "mock:ok"),
                Err(AiError::MalformedOutput),
                "case {index} must be malformed: {body}"
            );
        }
        let long_label = format!(
            r#"{{"actions": [{{"set_label": {{"object": 1, "label": "{}"}}}}], "diagnostics": []}}"#,
            "x".repeat(MAX_LABEL_LEN + 1)
        );
        assert_eq!(
            parse_proposal_content(&long_label, &base, "mock:ok"),
            Err(AiError::MalformedOutput)
        );
        let long_line = format!(
            r#"{{"actions": [], "diagnostics": ["{}"]}}"#,
            "y".repeat(MAX_TEXT_LEN + 1)
        );
        assert_eq!(
            parse_proposal_content(&long_line, &base, "mock:ok"),
            Err(AiError::MalformedOutput)
        );
        let huge = format!(
            r#"{{"actions": [], "diagnostics": ["{}"]}}"#,
            "z".repeat(MAX_PROPOSAL_BYTES)
        );
        assert_eq!(
            parse_proposal_content(&huge, &base, "mock:ok"),
            Err(AiError::MalformedOutput)
        );
    }

    #[test]
    fn grammar_20_tasks_identical_modulo_receipt() {
        let base = request();
        let shapes = [
            r#"{"actions": [{"noop": true}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [1, 2, 3]}}], "diagnostics": ["t"]}"#,
            r#"{"actions": [{"set_label": {"object": 2, "label": "gate"}}], "diagnostics": []}"#,
            r#"{"actions": [], "diagnostics": ["note"]}"#,
        ];
        for task in 0..20 {
            let body = shapes[task % shapes.len()];
            let mock = parse_proposal_content(body, &base, "mock:ok").expect("task validates");
            let local =
                parse_proposal_content(body, &base, "local:127.0.0.1").expect("task validates");
            let remote =
                parse_proposal_content(body, &base, "remote:127.0.0.1").expect("task validates");
            assert_eq!(mock.actions, local.actions, "task {task} actions match");
            assert_eq!(mock.actions, remote.actions, "task {task} actions match");
            assert_eq!(mock.request, base.id);
            assert_eq!(mock.base_revision, base.base_revision);
            for proposal in [&mock, &local, &remote] {
                validate_proposal(proposal).expect("shared validator accepts");
            }
            assert!(local.diagnostics.contains(&String::from("local:127.0.0.1")));
            assert!(remote
                .diagnostics
                .contains(&String::from("remote:127.0.0.1")));
        }
    }

    #[test]
    fn fuzz_malformed_applies_nothing() {
        use super::super::mock::{MockAdapter, ProviderAdapter};
        let base = request();
        let malformed = [
            "{",
            "null",
            "[]",
            "{} trailing words",
            r#"{"actions": [{"noop": true}]} trailing"#,
            r#"{"actions": "nope", "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": -1, "xyz_cm": [0, 0, 0]}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_label": {"object": 1}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_label": {"object": 1, "label": "a\nb"}}], "diagnostics": []}"#,
            r#"{"actions": [{"noop": null}], "diagnostics": []}"#,
            r#"{"actions": [null], "diagnostics": []}"#,
            r#"{"actions": [], "diagnostics": [null]}"#,
            r#"{"actions": [], "diagnostics": [], "actions2": []}"#,
        ];
        for body in malformed {
            let error = parse_proposal_content(body, &base, "mock:ok")
                .expect_err("fuzz input must be malformed");
            assert_eq!(error, AiError::MalformedOutput);
            let leaked = format!("{error:?}");
            assert!(!leaked.contains("a\nb"));
        }
        // Empty input is malformed without echoing anything.
        assert_eq!(
            parse_proposal_content("", &base, "mock:ok"),
            Err(AiError::MalformedOutput)
        );
        // Ledger proof: malformed script arms record errors, never mutations.
        let script = vec![super::super::core::MockStep::Malformed; 8];
        let mut adapter = MockAdapter::new(script);
        let policy = crate::mock::fixtures::policy();
        let caps = crate::mock::fixtures::caps();
        for n in 0..8u64 {
            let asked = crate::mock::fixtures::request(500 + n, 3);
            let outcome = adapter.call(&asked, &policy, &caps);
            assert_eq!(outcome.result, Err(AiError::MalformedOutput));
            assert!(!outcome.mutated);
        }
        assert_eq!(adapter.mutations(), 0);
    }

    #[test]
    fn generic_chat_shell_roundtrips_like_local() {
        use super::super::protocol::AdapterKind;
        const WIRE: Wire = Wire {
            label: "generic",
            body: transport::chat_body,
            envelope: &CHAT_ENVELOPE,
            remote: false,
            rungs: RungSupport {
                native: false,
                json_mode: false,
            },
        };
        let stub = crate::stub::StubServer::spawn_completion(crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["stub:ok"]}"#,
            Some((7, 3)),
        ));
        let url = stub.url();
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let policy = AiSessionPolicy {
            adapters: vec![AdapterKind::Local],
            mode: PrivacyMode::LocalOnly,
            destinations: Vec::new(),
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: crate::policy::Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        };
        let caps = crate::policy::ProviderCapabilities::local_http();
        let (outcome, adapter) = crate::stub::settle(
            |n| {
                let mut adapter = GenericVendorAdapter::new(
                    vec![MockStep::Succeed],
                    endpoint.clone(),
                    WIRE,
                    None,
                );
                let mut asked = crate::mock::fixtures::request(1 + n as u64, 3);
                asked.model.adapter = AdapterKind::Local;
                let outcome = adapter.call(&asked, &policy, &caps);
                (outcome, adapter)
            },
            |(o, _)| o.result.is_ok(),
        );
        assert!(outcome.result.is_ok(), "stub roundtrip succeeds");
        assert!(!outcome.dispatched_remote, "loopback is not remote");
        assert_eq!(adapter.remote_calls(), 0);
        assert_eq!(adapter.transport_calls(), 1);
        let result = outcome.result.as_ref().expect("success must complete");
        assert_eq!(result.usage.input_tokens, Some(7));
        assert_eq!(result.usage.output_tokens, Some(3));
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line == "generic:127.0.0.1"),
            "receipt labels the shell plus host: {proposal:?}"
        );
    }

    #[test]
    fn generic_shell_refuses_offline_and_missing_credential() {
        use super::super::protocol::AdapterKind;
        const WIRE: Wire = Wire {
            label: "generic",
            body: transport::chat_body,
            envelope: &CHAT_ENVELOPE,
            remote: true,
            rungs: RungSupport {
                native: false,
                json_mode: false,
            },
        };
        let url = String::from("http://127.0.0.1:9/v1/chat/completions");
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let policy = AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![url],
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: crate::policy::Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        };
        let caps = crate::policy::ProviderCapabilities::remote_http();
        // Offline is refused before any socket attempt.
        let mut adapter = GenericVendorAdapter::new(
            vec![MockStep::Succeed],
            endpoint.clone(),
            WIRE,
            Some(VendorSecret::Direct(String::from("token"))),
        );
        let mut offline = crate::mock::fixtures::request(2, 3);
        offline.model.adapter = AdapterKind::Remote;
        offline.privacy = PrivacyMode::Offline;
        let outcome = adapter.call(&offline, &policy, &caps);
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert_eq!(adapter.socket_attempts(), 0, "offline dials nothing");
        // Missing credential is a quota refusal with no dial.
        crate::test_env::remove("E2040_AX1_MISSING_GENERIC");
        let mut adapter = GenericVendorAdapter::with_env_token(
            vec![MockStep::Succeed],
            endpoint,
            WIRE,
            "E2040_AX1_MISSING_GENERIC",
        );
        let mut asked = crate::mock::fixtures::request(3, 3);
        asked.model.adapter = AdapterKind::Remote;
        asked.privacy = PrivacyMode::RemoteAllowed;
        let outcome = adapter.call(&asked, &policy, &caps);
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert_eq!(adapter.socket_attempts(), 0, "no credential, no dial");
        assert!(!outcome.dispatched_remote);
    }
}
