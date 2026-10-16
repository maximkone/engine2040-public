//! stream: tick-isolated streaming model output with backpressure.
//!
//! Owner: provider crate; serves announcer/taunts and living-world mind asks.
//! Units: bytes/chunks for buffers, ms for timeouts, ticks for tape stamps.
//! Determinism: same SSE bytes give the same assembled text and the same
//! tape checkpoint for recorded and live; the source label is provenance
//! only, never hashed. Errors:
//! [`StreamError`], mapped to [`AiError`] at the call boundary. Budget: one
//! stream holds at most 64 chunks and 64 KiB assembled; `poll` drains at most
//! 64 chunks into pre-reserved storage and never blocks the tick.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use engine_core::Fnv64;

use super::announce::{
    fold_text, Announcer, EventKindLite, Line, MatchObservation, CHECKPOINT_START, MAX_LINE_LEN,
};
use super::protocol::AiError;

/// Default bounded inbox depth, in chunks.
pub const DEFAULT_STREAM_CAPACITY: usize = 16;
/// Largest inbox depth, in chunks.
pub const MAX_STREAM_CAPACITY: usize = 64;
/// Default assembled-text bound, in bytes.
pub const DEFAULT_STREAM_MAX_BYTES: usize = 8192;
/// Smallest assembled-text bound, in bytes.
pub const MIN_STREAM_BYTES: usize = 256;
/// Largest assembled-text bound, in bytes.
pub const MAX_STREAM_BYTES: usize = 65_536;
/// Default worker deadline, in milliseconds.
pub const DEFAULT_STREAM_TIMEOUT_MS: u64 = 5_000;
/// Largest worker deadline, in milliseconds.
pub const MAX_STREAM_TIMEOUT_MS: u64 = 30_000;
/// Default per-`poll` drain bound, in chunks.
pub const DEFAULT_STREAM_DRAIN: usize = 8;
/// Largest per-`poll` drain bound, in chunks.
pub const MAX_STREAM_DRAIN: usize = 64;
/// Worker retries while the inbox is full before reporting backpressure.
const PUSH_RETRIES_WHILE_FULL: u32 = 50;
/// Pause between inbox-full retries; also the cancel/timeout granularity.
const PUSH_RETRY_PAUSE_MS: u64 = 1;
/// Mind-side text cap, in characters. Matches the LW mind bound by value so
/// the provider never depends on the gameplay crate; the direction stays
/// provider-to-consumer by documentation, not by import.
pub const MIND_TEXT_CAP: usize = 280;

/// Provenance of streamed bytes: recorded fixtures or a live endpoint.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SourceLabel {
    /// Bytes from a recorded fixture or stub; replays offline.
    #[default]
    Recorded,
    /// Bytes from a live endpoint roundtrip.
    Live,
}

impl SourceLabel {
    /// True only for live endpoint bytes.
    pub const fn is_live(self) -> bool {
        matches!(self, SourceLabel::Live)
    }
}

impl std::fmt::Display for SourceLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceLabel::Recorded => f.write_str("recorded"),
            SourceLabel::Live => f.write_str("live"),
        }
    }
}

/// Streaming failure. Maps to [`AiError`] at the call boundary, so callers
/// keep one retry vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamError {
    /// Worker deadline elapsed before completion.
    Timeout,
    /// Cancelled mid-stream; late chunks are dropped, never applied.
    Cancelled,
    /// Worker vanished or the endpoint was unreachable.
    Unavailable,
    /// A chunk or SSE payload failed validation.
    MalformedOutput,
    /// Bounded inbox stayed full; the stream is refused, never grown.
    Backpressure,
    /// No provider network calls allowed.
    Offline,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Timeout => f.write_str("stream timed out"),
            StreamError::Cancelled => f.write_str("stream cancelled"),
            StreamError::Unavailable => f.write_str("stream unavailable"),
            StreamError::MalformedOutput => f.write_str("stream output malformed"),
            StreamError::Backpressure => f.write_str("stream buffer full"),
            StreamError::Offline => f.write_str("stream offline"),
        }
    }
}

impl StreamError {
    /// Contract error for this failure. Backpressure has no contract variant and
    /// surfaces as retryable [`AiError::Unavailable`]; offline is a privacy
    /// refusal, hence [`AiError::UnsupportedCapability`].
    pub const fn to_ai_error(self) -> AiError {
        match self {
            StreamError::Timeout => AiError::Timeout,
            StreamError::Cancelled => AiError::Cancelled,
            StreamError::Unavailable => AiError::Unavailable,
            StreamError::MalformedOutput => AiError::MalformedOutput,
            StreamError::Backpressure => AiError::Unavailable,
            StreamError::Offline => AiError::UnsupportedCapability,
        }
    }

    /// True when retrying with the same identity is allowed.
    pub const fn retryable(self) -> bool {
        match self {
            StreamError::Timeout | StreamError::Unavailable | StreamError::Backpressure => true,
            StreamError::Cancelled | StreamError::MalformedOutput | StreamError::Offline => false,
        }
    }
}

/// Bounded configuration for one stream. Every bound is validated up front;
/// invalid shapes are [`StreamError::MalformedOutput`], never silent clamps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StreamConfig {
    /// Bounded inbox depth, in chunks (`1..=MAX_STREAM_CAPACITY`).
    pub capacity_chunks: usize,
    /// Assembled-text bound, in bytes (`MIN_STREAM_BYTES..=MAX_STREAM_BYTES`).
    pub max_bytes: usize,
    /// Worker deadline, in milliseconds (`1..=MAX_STREAM_TIMEOUT_MS`).
    pub timeout_ms: u64,
    /// Per-`poll` drain bound, in chunks (`1..=MAX_STREAM_DRAIN`).
    pub max_drain_per_tick: usize,
}

impl StreamConfig {
    /// Build a bounded configuration; out-of-range fields are refused.
    pub fn new(
        capacity_chunks: usize,
        max_bytes: usize,
        timeout_ms: u64,
        max_drain_per_tick: usize,
    ) -> Result<Self, StreamError> {
        let config = Self {
            capacity_chunks,
            max_bytes,
            timeout_ms,
            max_drain_per_tick,
        };
        config.validate()?;
        Ok(config)
    }

    /// Refuse out-of-range bounds before any thread or byte moves.
    pub fn validate(&self) -> Result<(), StreamError> {
        if self.capacity_chunks == 0 || self.capacity_chunks > MAX_STREAM_CAPACITY {
            return Err(StreamError::MalformedOutput);
        }
        if self.max_bytes < MIN_STREAM_BYTES || self.max_bytes > MAX_STREAM_BYTES {
            return Err(StreamError::MalformedOutput);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_STREAM_TIMEOUT_MS {
            return Err(StreamError::MalformedOutput);
        }
        if self.max_drain_per_tick == 0 || self.max_drain_per_tick > MAX_STREAM_DRAIN {
            return Err(StreamError::MalformedOutput);
        }
        Ok(())
    }
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            capacity_chunks: DEFAULT_STREAM_CAPACITY,
            max_bytes: DEFAULT_STREAM_MAX_BYTES,
            timeout_ms: DEFAULT_STREAM_TIMEOUT_MS,
            max_drain_per_tick: DEFAULT_STREAM_DRAIN,
        }
    }
}

/// One parsed SSE frame: optional event name plus the data payload.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SseFrame {
    /// Event name from `event:`, if the server sent one.
    pub event: Option<String>,
    /// Joined `data:` payloads for this frame.
    pub data: String,
}

/// Split SSE bytes into frames. Normalizes CRLF, joins multi-`data:` lines
/// with `\n`, keeps the last `event:` name, and skips `:` heartbeats and
/// blocks without data. Live bodies are already bounded by the transport
/// (64 KiB); fixtures are small by construction.
pub fn parse_sse_stream(text: &str) -> Vec<SseFrame> {
    let normalized = text.replace("\r\n", "\n");
    let mut frames = Vec::new();
    for block in normalized.split("\n\n") {
        let mut event: Option<String> = None;
        let mut data: Vec<&str> = Vec::new();
        for line in block.lines() {
            if let Some(name) = line.strip_prefix("event:") {
                event = Some(name.trim().to_owned());
            } else if let Some(payload) = line.strip_prefix("data:") {
                data.push(payload.strip_prefix(' ').unwrap_or(payload));
            } else if line.starts_with(':') {
                // Comment or heartbeat: no payload, never a chunk.
            }
        }
        if data.is_empty() {
            continue;
        }
        frames.push(SseFrame {
            event,
            data: data.join("\n"),
        });
    }
    frames
}

/// True when the frame is the OpenAI `[DONE]` terminal marker.
pub fn is_done_marker(frame: &SseFrame) -> bool {
    frame.data.trim() == "[DONE]"
}

/// Text delta from an OpenAI-compatible SSE frame. Reads
/// `choices[0].delta.content` (streaming) and `choices[0].message.content`
/// (single-shot bodies); `[DONE]` and control frames yield `None`.
pub fn openai_sse_text(frame: &SseFrame) -> Option<String> {
    if is_done_marker(frame) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(frame.data.trim()).ok()?;
    for pointer in ["/choices/0/delta/content", "/choices/0/message/content"] {
        if let Some(text) = value.pointer(pointer).and_then(|item| item.as_str()) {
            if text.is_empty() {
                return None;
            }
            return Some(text.to_owned());
        }
    }
    None
}

/// Text delta from an Anthropic SSE frame. Reads `delta.text` on
/// `content_block_delta` (and any other event carrying one); control frames
/// such as `message_stop` yield `None`.
pub fn anthropic_sse_text(frame: &SseFrame) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(frame.data.trim()).ok()?;
    value
        .pointer("/delta/text")
        .and_then(|item| item.as_str())
        .filter(|text| !text.is_empty())
        .map(|text| text.to_owned())
}

/// Fold OpenAI SSE bytes into assembled text, bounded by `max_bytes`.
/// Truncates the tail on a char boundary; never fails, never grows past the
/// bound.
pub fn collect_openai_text(sse: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for frame in parse_sse_stream(sse) {
        if let Some(delta) = openai_sse_text(&frame) {
            let room = max_bytes.saturating_sub(out.len());
            if room == 0 {
                break;
            }
            out.push_str(truncate_str(&delta, room));
        }
    }
    out
}

/// Fold Anthropic SSE bytes into assembled text, bounded by `max_bytes`.
pub fn collect_anthropic_text(sse: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for frame in parse_sse_stream(sse) {
        if let Some(delta) = anthropic_sse_text(&frame) {
            let room = max_bytes.saturating_sub(out.len());
            if room == 0 {
                break;
            }
            out.push_str(truncate_str(&delta, room));
        }
    }
    out
}

/// Longest char-boundary prefix of `text` within `max` bytes.
pub(crate) fn truncate_str(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Bounded tick-side staging queue: chunk-count plus byte caps with counted
/// drops. Pushes past either bound are refused with
/// [`StreamError::Backpressure`]; the queue never grows past its bounds.
#[derive(Debug, Default)]
pub struct StreamBuffer {
    chunks: VecDeque<String>,
    bytes: usize,
    capacity: usize,
    max_bytes: usize,
    dropped: u64,
}

impl StreamBuffer {
    /// Build a staging queue. Zero bounds clamp to one so the queue always
    /// holds at least one byte of one chunk; real validation lives in
    /// [`StreamConfig::validate`].
    pub fn new(capacity_chunks: usize, max_bytes: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            bytes: 0,
            capacity: capacity_chunks.max(1),
            max_bytes: max_bytes.max(1),
            dropped: 0,
        }
    }

    /// Stage one chunk. Oversize chunks are [`StreamError::MalformedOutput`];
    /// full queues are [`StreamError::Backpressure`] with a counted drop.
    pub fn try_push(&mut self, chunk: String) -> Result<(), StreamError> {
        if chunk.len() > super::MAX_TEXT_LEN {
            return Err(StreamError::MalformedOutput);
        }
        if self.chunks.len() >= self.capacity {
            self.dropped += 1;
            return Err(StreamError::Backpressure);
        }
        if self.bytes + chunk.len() > self.max_bytes {
            self.dropped += 1;
            return Err(StreamError::Backpressure);
        }
        self.bytes += chunk.len();
        self.chunks.push_back(chunk);
        Ok(())
    }

    /// Oldest staged chunk, if any.
    pub fn pop(&mut self) -> Option<String> {
        let chunk = self.chunks.pop_front()?;
        self.bytes = self.bytes.saturating_sub(chunk.len());
        Some(chunk)
    }

    /// Staged chunk count.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// True when nothing is staged.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Staged bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Free chunk slots.
    pub fn space(&self) -> usize {
        self.capacity.saturating_sub(self.chunks.len())
    }

    /// Chunks refused and counted.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Worker-to-tick events over the bounded inbox channel.
#[derive(Debug)]
enum StreamEvent {
    /// One text delta.
    Chunk(String),
    /// Feed complete; tick assembles what drained.
    Done,
    /// Feed failed; tick reports the reason.
    Failed(StreamError),
}

/// Worker-side feed handle. Every method checks cancellation and the
/// deadline first, so a stuck producer surfaces promptly and the tick never
/// waits: all sends are `try_send` with a short bounded retry, never a
/// blocking send.
pub struct FeedTx {
    tx: SyncSender<StreamEvent>,
    cancel: std::sync::Arc<AtomicBool>,
    deadline: Instant,
    waits: std::sync::Arc<AtomicU64>,
}

impl FeedTx {
    /// True when the tick cancelled this stream.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// True when the worker deadline has passed.
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Backpressure waits so far (inbox-full retries).
    pub fn backpressure_waits(&self) -> u64 {
        self.waits.load(Ordering::SeqCst)
    }

    /// Push one text delta. Oversize input is refused without touching the
    /// channel; a full inbox retries briefly, then the stream fails with
    /// [`StreamError::Backpressure`] instead of growing.
    pub fn push(&self, chunk: &str) -> Result<(), StreamError> {
        if self.is_cancelled() {
            self.send_failed(StreamError::Cancelled);
            return Err(StreamError::Cancelled);
        }
        if self.is_expired() {
            self.send_failed(StreamError::Timeout);
            return Err(StreamError::Timeout);
        }
        if chunk.len() > super::MAX_TEXT_LEN {
            return Err(StreamError::MalformedOutput);
        }
        for _ in 0..PUSH_RETRIES_WHILE_FULL {
            match self.tx.try_send(StreamEvent::Chunk(chunk.to_owned())) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(event)) => {
                    let _ = event;
                    self.waits.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(PUSH_RETRY_PAUSE_MS));
                    if self.is_cancelled() {
                        self.send_failed(StreamError::Cancelled);
                        return Err(StreamError::Cancelled);
                    }
                    if self.is_expired() {
                        self.send_failed(StreamError::Timeout);
                        return Err(StreamError::Timeout);
                    }
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(StreamError::Unavailable);
                }
            }
        }
        self.send_failed(StreamError::Backpressure);
        Err(StreamError::Backpressure)
    }

    /// Complete the stream. A cancelled or expired feed reports its failure
    /// instead of a completion the tick must not trust.
    pub fn finish(&self) {
        if self.is_cancelled() {
            self.send_failed(StreamError::Cancelled);
        } else if self.is_expired() {
            self.send_failed(StreamError::Timeout);
        } else {
            let _ = self.tx.try_send(StreamEvent::Done);
        }
    }

    /// Fail the stream with `error`.
    pub fn fail(&self, error: StreamError) {
        self.send_failed(error);
    }

    fn send_failed(&self, error: StreamError) {
        // Best effort with a short retry: the tick drains regularly, and the
        // disconnect mapping below still reports a failure if it never lands.
        for _ in 0..10 {
            match self.tx.try_send(StreamEvent::Failed(error)) {
                Ok(()) => return,
                Err(TrySendError::Full(_)) => {
                    thread::sleep(Duration::from_millis(PUSH_RETRY_PAUSE_MS));
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    }
}

/// Tick-side view of one stream. `poll` drains at most `max_drain_per_tick`
/// chunks with `try_recv` and returns immediately: pending while the worker
/// runs, terminal exactly once, replayed verbatim afterwards.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamPoll {
    /// Worker still running; see [`StreamJob::assembled`] for text so far.
    Pending,
    /// Complete assembled text, byte-bounded.
    Done,
    /// Terminal failure; late chunks are dropped, never applied.
    Failed(StreamError),
}

/// Tick-isolated stream handle. The worker thread owns the feed; the tick
/// owns this handle and calls [`StreamJob::poll`] once per tick. Dropping
/// the handle disconnects the worker, whose next `push` then reports
/// [`StreamError::Unavailable`] so feeds must treat push errors as terminal.
pub struct StreamJob {
    rx: Receiver<StreamEvent>,
    cancel: std::sync::Arc<AtomicBool>,
    waits: std::sync::Arc<AtomicU64>,
    deadline: Instant,
    source: SourceLabel,
    max_drain: usize,
    max_bytes: usize,
    staging: StreamBuffer,
    assembled: String,
    chunks: u64,
    dropped_chunks: u64,
    truncated: bool,
    terminal: Option<Result<(), StreamError>>,
}

impl StreamJob {
    /// Spawn a stream: validate bounds, open the bounded inbox, and run
    /// `feed` on a worker thread. Returns the tick-side handle immediately;
    /// the worker never touches tick state. A worker that cannot start is
    /// `StreamError::Unavailable`, never a panic.
    pub fn spawn(
        config: &StreamConfig,
        source: SourceLabel,
        feed: impl FnOnce(&FeedTx) + Send + 'static,
    ) -> Result<Self, StreamError> {
        Self::spawn_via(config, source, feed, |builder, work| builder.spawn(work))
    }

    /// `spawn` with the OS spawn injected, so a test forces the failure
    /// through the same mapping the real spawn uses.
    fn spawn_via<S>(
        config: &StreamConfig,
        source: SourceLabel,
        feed: impl FnOnce(&FeedTx) + Send + 'static,
        spawn: S,
    ) -> Result<Self, StreamError>
    where
        S: FnOnce(
            thread::Builder,
            Box<dyn FnOnce() + Send>,
        ) -> std::io::Result<thread::JoinHandle<()>>,
    {
        config.validate()?;
        let (tx, rx) = mpsc::sync_channel(config.capacity_chunks);
        // One shared cancel flag, one shared wait counter and one shared
        // deadline: the tick sets cancellation here and the worker observes
        // it on its next chunk boundary, while the tick also enforces the
        // same deadline so cancel/timeout surface on the next `poll`
        // without waiting for a descheduled worker.
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let waits = std::sync::Arc::new(AtomicU64::new(0));
        let deadline = Instant::now() + Duration::from_millis(config.timeout_ms);
        let worker = FeedTx {
            tx,
            cancel: std::sync::Arc::clone(&cancel),
            deadline,
            waits: std::sync::Arc::clone(&waits),
        };
        spawn(
            thread::Builder::new().name("provider-stream".into()),
            Box::new(move || feed(&worker)),
        )
        .map_err(|_| StreamError::Unavailable)?;
        Ok(Self {
            rx,
            cancel,
            waits,
            deadline,
            source,
            max_drain: config.max_drain_per_tick,
            max_bytes: config.max_bytes,
            staging: StreamBuffer::new(config.capacity_chunks, config.max_bytes),
            assembled: String::with_capacity(config.max_bytes.min(MAX_STREAM_BYTES)),
            chunks: 0,
            dropped_chunks: 0,
            truncated: false,
            terminal: None,
        })
    }

    /// Drain at most `max_drain_per_tick` chunks without blocking. Never
    /// sleeps, never dials; reads only the shared cancel flag and its own
    /// deadline so cancel/timeout surface on the next `poll` without
    /// waiting for a descheduled worker. Drained text stays visible via
    /// [`StreamJob::assembled`] on every terminal.
    pub fn poll(&mut self) -> StreamPoll {
        if let Some(terminal) = &self.terminal {
            return match terminal {
                Ok(()) => StreamPoll::Done,
                Err(error) => StreamPoll::Failed(*error),
            };
        }
        // Share the worker's backpressure counter so the tick can report it.
        let mut drained = 0usize;
        while drained < self.max_drain {
            match self.rx.try_recv() {
                Ok(StreamEvent::Chunk(text)) => {
                    drained += 1;
                    self.chunks += 1;
                    if self.staging.try_push(text).is_err() {
                        self.dropped_chunks += 1;
                    }
                }
                Ok(StreamEvent::Done) => {
                    self.fold_staging();
                    self.terminal = Some(Ok(()));
                    return StreamPoll::Done;
                }
                Ok(StreamEvent::Failed(error)) => {
                    self.fold_staging();
                    self.terminal = Some(Err(error));
                    return StreamPoll::Failed(error);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.fold_staging();
                    // The worker vanished without a terminal event: a
                    // disconnect, never a silent completion. An explicit
                    // tick cancel still wins so immediate cancel stays
                    // under budget even when the worker never wakes.
                    if self.cancel.load(Ordering::SeqCst) {
                        self.terminal = Some(Err(StreamError::Cancelled));
                    } else if Instant::now() >= self.deadline {
                        self.terminal = Some(Err(StreamError::Timeout));
                    } else if self.terminal.is_none() {
                        self.terminal = Some(Err(StreamError::Unavailable));
                    }
                    let terminal = self
                        .terminal
                        .as_ref()
                        .unwrap_or(&Err(StreamError::Unavailable));
                    return match terminal {
                        Ok(()) => StreamPoll::Done,
                        Err(error) => StreamPoll::Failed(*error),
                    };
                }
            }
        }
        self.fold_staging();
        // Surface a terminal that arrived folded into staging order.
        if let Some(terminal) = &self.terminal {
            return match terminal {
                Ok(()) => StreamPoll::Done,
                Err(error) => StreamPoll::Failed(*error),
            };
        }
        // Tick-side cancel/timeout: the worker also enforces the same
        // deadline on its next chunk boundary, but the tick must not wait
        // for a descheduled worker. Folded text stays visible; late chunks
        // are dropped, never applied.
        if self.cancel.load(Ordering::SeqCst) {
            self.terminal = Some(Err(StreamError::Cancelled));
            return StreamPoll::Failed(StreamError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            self.terminal = Some(Err(StreamError::Timeout));
            return StreamPoll::Failed(StreamError::Timeout);
        }
        StreamPoll::Pending
    }

    fn fold_staging(&mut self) {
        while let Some(piece) = self.staging.pop() {
            let room = self.max_bytes.saturating_sub(self.assembled.len());
            if room == 0 {
                self.dropped_chunks += 1;
                continue;
            }
            if piece.len() <= room {
                self.assembled.push_str(&piece);
            } else {
                self.assembled.push_str(truncate_str(&piece, room));
                self.truncated = true;
                self.dropped_chunks += 1;
            }
        }
    }

    /// Cancel mid-stream. The next [`StreamJob::poll`] reports
    /// [`StreamError::Cancelled`] without waiting for the worker; the
    /// worker also observes the flag on its next chunk boundary and
    /// stops pushing, so late chunks are dropped, never applied.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Share the cancel flag with a feed that needs it directly. Test and
    /// adapter glue only; the tick itself uses [`StreamJob::cancel`].
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// True once [`StreamPoll::Done`] or [`StreamPoll::Failed`] was returned.
    pub fn is_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    /// Recorded fixtures or live endpoint bytes.
    pub fn source(&self) -> SourceLabel {
        self.source
    }

    /// Assembled text drained so far, always within `max_bytes`.
    pub fn assembled(&self) -> &str {
        &self.assembled
    }

    /// Chunks drained from the inbox so far.
    pub fn chunks(&self) -> u64 {
        self.chunks
    }

    /// Chunks dropped to hold the byte bound, plus staging overflows.
    pub fn dropped_chunks(&self) -> u64 {
        self.dropped_chunks + self.staging.dropped()
    }

    /// True when a chunk was cut to hold the byte bound.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Worker backpressure waits observed so far.
    pub fn backpressure_waits(&self) -> u64 {
        self.waits.load(Ordering::SeqCst)
    }
}

/// One accepted tick of streamed text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TapeEntry {
    /// Fixed tick the text was accepted at.
    pub tick: u64,
    /// Accepted text, bounded to [`MAX_STREAM_BYTES`] input via truncation.
    pub text: String,
}

/// Disconnected record of accepted stream text. The game pushes every
/// accepted chunk here; replay iterates the stored entries verbatim and never
/// touches the network, so a disconnected tape advances and replays offline.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StreamTape {
    entries: Vec<TapeEntry>,
    source: SourceLabel,
}

impl StreamTape {
    /// Empty tape with its provenance label.
    pub fn new(source: SourceLabel) -> Self {
        Self {
            entries: Vec::new(),
            source,
        }
    }

    /// Record one accepted text at its tick, truncated to the stream byte
    /// bound on a char boundary. Never fails, never dials.
    pub fn push(&mut self, tick: u64, text: &str) {
        self.entries.push(TapeEntry {
            tick,
            text: truncate_str(text, MAX_STREAM_BYTES).to_owned(),
        });
    }

    /// Recorded entries in acceptance order.
    pub fn entries(&self) -> &[TapeEntry] {
        &self.entries
    }

    /// Replay recorded entries verbatim: no worker, no network, no new text.
    pub fn replay(&self) -> impl Iterator<Item = &TapeEntry> {
        self.entries.iter()
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Recorded fixtures or live endpoint bytes.
    pub fn source(&self) -> SourceLabel {
        self.source
    }

    /// Deterministic FNV-1a checkpoint over ticks and text bytes in
    /// acceptance order. The same recorded sequence always yields the same
    /// checkpoint; any changed entry changes it.
    pub fn checkpoint(&self) -> u64 {
        let mut hash = Fnv64::with_state(CHECKPOINT_START);
        for entry in &self.entries {
            hash.write(&entry.tick.to_le_bytes());
            fold_text(&mut hash, &entry.text);
        }
        hash.finish()
    }
}

fn priority_for(obs: &MatchObservation) -> u8 {
    match &obs.last_event {
        Some(event) if event.kind == EventKindLite::Knockdown => 2,
        Some(_) => 1,
        None => 0,
    }
}

/// Serve one observation from completed stream text: the announcer call
/// site for this module. Non-blank assembled text becomes a provider line
/// (truncated to [`MAX_LINE_LEN`], never rejected mid-match); blank text
/// falls back to the announcer's scripted table with its priority intact.
pub fn announce_stream_line(
    announcer: &mut Announcer,
    obs: &MatchObservation,
    assembled: &str,
) -> Line {
    let clean = assembled.trim();
    if clean.is_empty() {
        return announcer.scripted_line(obs);
    }
    Line {
        text: clean.chars().take(MAX_LINE_LEN).collect(),
        priority: priority_for(obs),
        scripted: false,
    }
}

/// Read stream text the way an LW mind ask does: bounded non-blank text the
/// mind can store, or `None` so the mind takes its scripted fallback. Over-cap
/// text is refused (never silently cut) because mind memory is fixed-width.
pub fn mind_text_from_stream(assembled: &str) -> Option<String> {
    let clean = assembled.trim();
    if clean.is_empty() {
        return None;
    }
    let count = clean.chars().count();
    if count > MIND_TEXT_CAP {
        return None;
    }
    Some(clean.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Instant;

    /// Recorded OpenAI-compatible SSE fixture: two deltas plus `[DONE]`.
    const OPENAI_SSE_FIXTURE: &str = "data: {\"choices\": [{\"delta\": {\"content\": \"Steel \"}}]}\n\ndata: {\"choices\": [{\"delta\": {\"content\": \"glints!\"}}]}\n\ndata: [DONE]\n\n";
    /// Recorded Anthropic SSE fixture: two text deltas plus `message_stop`.
    const ANTHROPIC_SSE_FIXTURE: &str = "event: content_block_delta\ndata: {\"delta\": {\"text\": \"Steel \"}}\n\nevent: content_block_delta\ndata: {\"delta\": {\"text\": \"glints!\"}}\n\nevent: message_stop\ndata: {\"type\": \"message_stop\"}\n\n";

    fn config_for_test() -> StreamConfig {
        StreamConfig::new(16, 8192, 5_000, 8).expect("fixture config valid")
    }

    #[test]
    fn spawn_failure_is_err() {
        // A worker that cannot start is a typed Unavailable error; the forced
        // failure must not panic the caller.
        let forced = StreamJob::spawn_via(
            &config_for_test(),
            SourceLabel::Recorded,
            |_: &FeedTx| {},
            |_, _| Err(std::io::Error::other("forced by test")),
        );
        assert!(
            matches!(forced, Err(StreamError::Unavailable)),
            "forced spawn failure must be Unavailable"
        );
    }

    fn poll_until_terminal(job: &mut StreamJob) -> StreamPoll {
        let mut last = StreamPoll::Pending;
        for _ in 0..500 {
            last = job.poll();
            if last != StreamPoll::Pending {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        last
    }

    #[test]
    fn stream_openai_and_anthropic_from_recorded_and_live() {
        // Recorded fixtures parse with no network, labelled recorded.
        let openai_recorded = collect_openai_text(OPENAI_SSE_FIXTURE, 8192);
        let anthropic_recorded = collect_anthropic_text(ANTHROPIC_SSE_FIXTURE, 8192);
        assert_eq!(openai_recorded, "Steel glints!");
        assert_eq!(anthropic_recorded, "Steel glints!");
        assert_eq!(SourceLabel::Recorded.to_string(), "recorded");
        assert!(!SourceLabel::Recorded.is_live());

        // Live loopback roundtrips serve the same vendor SSE shapes over real
        // TCP with session auth, labelled live. When operator keys are
        // absent this stub path is the live proof; the vendor parsers above
        // consume identical bytes from a real endpoint.
        for (fixture, collect) in [
            (
                OPENAI_SSE_FIXTURE,
                collect_openai_text as fn(&str, usize) -> String,
            ),
            (ANTHROPIC_SSE_FIXTURE, collect_anthropic_text),
        ] {
            let stub = crate::stub::StubServer::spawn_completion(fixture.to_owned());
            let url = stub.url();
            let endpoint = crate::transport::parse_endpoint(&url).expect("stub endpoint parses");
            let response = crate::transport::post_json(
                &endpoint,
                "{\"stream\": true}",
                Some("suite-token"),
                5_000,
            )
            .expect("loopback SSE roundtrip succeeds");
            assert_eq!(stub.hits(), 1, "live bytes crossed real TCP");
            assert_eq!(stub.auth_hits(), 1, "live bytes carry session auth");
            assert_eq!(collect(&response.body, 8192), "Steel glints!");
        }
        assert!(SourceLabel::Live.is_live());
        assert_eq!(SourceLabel::Live.to_string(), "live");

        // A recorded stream job replays fixture deltas through the bounded
        // inbox and assembles the same text the live path parsed.
        let mut job = StreamJob::spawn(&config_for_test(), SourceLabel::Recorded, |feed| {
            for frame in parse_sse_stream(OPENAI_SSE_FIXTURE) {
                if let Some(delta) = openai_sse_text(&frame) {
                    if feed.push(&delta).is_err() {
                        return;
                    }
                }
            }
            feed.finish();
        })
        .expect("fixture config valid");
        assert_eq!(job.source(), SourceLabel::Recorded);
        assert_eq!(poll_until_terminal(&mut job), StreamPoll::Done);
        assert_eq!(job.assembled(), "Steel glints!");
        assert_eq!(job.chunks(), 2);
        assert_eq!(job.dropped_chunks(), 0);
        assert!(!job.truncated());
    }

    #[test]
    fn stream_cancel_completes_under_100ms() {
        let config = StreamConfig::new(16, 8192, 5_000, 8).expect("fixture config valid");
        let mut job = StreamJob::spawn(&config, SourceLabel::Live, |feed| {
            for n in 0..100 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("fixture config valid");
        // Let the worker start, then cancel mid-stream like the tick would.
        for _ in 0..50 {
            if !job.assembled().is_empty() {
                break;
            }
            let _ = job.poll();
            thread::sleep(Duration::from_millis(1));
        }
        let start = Instant::now();
        job.cancel();
        let terminal = poll_until_terminal(&mut job);
        let elapsed = start.elapsed();
        assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
        assert!(job.is_terminal());
        assert!(
            elapsed < Duration::from_millis(100),
            "cancel surfaces in {elapsed:?}, budget is 100 ms"
        );
        assert_eq!(
            StreamError::Cancelled.to_ai_error(),
            AiError::Cancelled,
            "cancel maps to the contract error"
        );
        assert!(!StreamError::Cancelled.retryable());
    }

    #[test]
    fn stream_blocked_adapter_never_stalls_tick() {
        // A feed parked behind a gate that never opens: the tick must still
        // advance on every poll, then cancel must release the worker.
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let config = StreamConfig::new(8, 1024, 5_000, 4).expect("fixture config valid");
        let mut job = StreamJob::spawn(&config, SourceLabel::Live, move |feed| {
            loop {
                if feed.is_cancelled() {
                    feed.fail(StreamError::Cancelled);
                    return;
                }
                match gate_rx.recv_timeout(Duration::from_millis(5)) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => continue,
                }
            }
            feed.finish();
        })
        .expect("fixture config valid");
        // The fixed tick advances 120 steps while the worker stays parked:
        // every poll returns immediately with Pending.
        let start = Instant::now();
        let mut ticks = 0u64;
        for _ in 0..120 {
            assert_eq!(
                job.poll(),
                StreamPoll::Pending,
                "parked worker stays pending"
            );
            ticks += 1;
        }
        let poll_time = start.elapsed();
        assert_eq!(ticks, 120, "the tick advanced despite the stuck stream");
        assert!(
            poll_time < Duration::from_secs(2),
            "120 non-blocking polls finish fast: {poll_time:?}"
        );
        job.cancel();
        assert_eq!(
            poll_until_terminal(&mut job),
            StreamPoll::Failed(StreamError::Cancelled),
            "cancel releases the parked worker"
        );
        drop(gate_tx);
    }

    #[test]
    fn stream_timeout_never_touches_tick() {
        // Worker deadline 80 ms; the feed stalls past it. The tick keeps
        // polling (Pending, text preserved) and learns Timeout, never waits.
        let config = StreamConfig::new(8, 1024, 80, 4).expect("fixture config valid");
        let mut job = StreamJob::spawn(&config, SourceLabel::Live, |feed| {
            if feed.push("part-one ").is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(200));
            // Past the deadline: refused, and the failure is reported.
            let _ = feed.push("part-two");
        })
        .expect("fixture config valid");
        let mut ticks = 0u64;
        let terminal = loop {
            let poll = job.poll();
            if poll != StreamPoll::Pending {
                break poll;
            }
            ticks += 1;
            thread::sleep(Duration::from_millis(2));
            assert!(ticks <= 500, "timeout must surface");
        };
        assert_eq!(terminal, StreamPoll::Failed(StreamError::Timeout));
        assert!(ticks > 2, "the tick advanced while the worker stalled");
        assert_eq!(job.assembled(), "part-one ", "pre-timeout text is kept");
        assert_eq!(StreamError::Timeout.to_ai_error(), AiError::Timeout);
        assert!(StreamError::Timeout.retryable());
    }

    #[test]
    fn stream_backpressure_bounds_buffers() {
        // Standalone staging queue: count and byte bounds refuse, never grow.
        let mut buffer = StreamBuffer::new(2, 16);
        assert!(buffer.is_empty());
        assert_eq!(buffer.space(), 2);
        assert!(buffer.try_push(String::from("ab")).is_ok());
        assert!(buffer.try_push(String::from("cd")).is_ok());
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.bytes(), 4);
        assert_eq!(
            buffer.try_push(String::from("ef")),
            Err(StreamError::Backpressure)
        );
        assert_eq!(buffer.dropped(), 1);
        assert_eq!(buffer.len(), 2, "refused chunks never stage");
        assert_eq!(buffer.pop().as_deref(), Some("ab"));
        assert!(buffer.try_push(String::from("ef")).is_ok());
        // Byte bound refuses while slots are free.
        let mut small = StreamBuffer::new(64, 4);
        assert!(small.try_push(String::from("ab")).is_ok());
        assert!(small.try_push(String::from("cd")).is_ok());
        assert_eq!(
            small.try_push(String::from("e")),
            Err(StreamError::Backpressure)
        );
        // Oversize chunks fail validation, not the queue.
        let big = "x".repeat(super::super::MAX_TEXT_LEN + 1);
        assert_eq!(small.try_push(big), Err(StreamError::MalformedOutput));
        // Job level: assembled text never passes max_bytes. Burst overflow
        // drops whole chunks at the staging bound, counted, never grown.
        let config = StreamConfig::new(8, 256, 5_000, 4).expect("fixture config valid");
        let mut job = StreamJob::spawn(&config, SourceLabel::Recorded, |feed| {
            let _ = feed.push(&"y".repeat(100));
            let _ = feed.push(&"y".repeat(100));
            let _ = feed.push(&"y".repeat(100));
            feed.finish();
        })
        .expect("fixture config valid");
        assert_eq!(poll_until_terminal(&mut job), StreamPoll::Done);
        assert_eq!(job.assembled().len(), 200, "staging holds the byte bound");
        assert!(job.dropped_chunks() > 0, "overflow is counted, not grown");
        // Paced overflow folds across polls and truncates the tail instead.
        let mut paced = StreamJob::spawn(&config, SourceLabel::Recorded, |feed| {
            let _ = feed.push(&"z".repeat(100));
            thread::sleep(Duration::from_millis(50));
            let _ = feed.push(&"z".repeat(100));
            thread::sleep(Duration::from_millis(50));
            let _ = feed.push(&"z".repeat(100));
            feed.finish();
        })
        .expect("fixture config valid");
        assert_eq!(poll_until_terminal(&mut paced), StreamPoll::Done);
        assert_eq!(
            paced.assembled().len(),
            256,
            "assembled holds the byte bound"
        );
        assert!(paced.truncated(), "overflow truncates on a char boundary");
        assert!(paced.dropped_chunks() > 0, "overflow is counted, not grown");
        assert!(job.dropped_chunks() > 0, "overflow is counted, not grown");
        assert_eq!(
            StreamError::Backpressure.to_ai_error(),
            AiError::Unavailable,
            "backpressure maps to retryable Unavailable"
        );
    }

    #[test]
    fn stream_disconnected_10k_tick_tape_replays_twice_offline() {
        use super::super::announce::{
            Announcer, EventLite, FigLite, MatchObservation, PresentationLog,
        };
        use super::super::announce::{EventKindLite, LineError};
        use super::super::mock::{MockStep, ProviderAdapter};
        use super::super::policy::ProviderCapabilities;

        fn offline(_obs: &MatchObservation, _budget: u64) -> Result<String, LineError> {
            Err(LineError::Offline)
        }

        // Advance a 10k-tick disconnected tape: scripted lines only, no
        // network, every accepted line recorded with its tick.
        let script: Vec<MatchObservation> = (0..10_000u64)
            .map(|tick| {
                let last_event = match tick % 6 {
                    2 => Some(EventLite {
                        kind: EventKindLite::Hit,
                        fighter: 1,
                    }),
                    4 => Some(EventLite {
                        kind: EventKindLite::Knockdown,
                        fighter: 2,
                    }),
                    _ => None,
                };
                MatchObservation {
                    tick,
                    fighters: vec![
                        FigLite {
                            id: 1,
                            hp_permille: 1000 - ((tick * 7) % 900) as u16,
                            down: false,
                        },
                        FigLite {
                            id: 2,
                            hp_permille: 1000 - ((tick * 11) % 950) as u16,
                            down: tick % 6 == 4,
                        },
                    ],
                    last_event,
                }
            })
            .collect();
        let run_tape = |seed: u64| {
            let mut announcer = Announcer::new(seed);
            let mut log = PresentationLog::new();
            let mut tape = StreamTape::new(SourceLabel::Recorded);
            for obs in &script {
                let line = announcer.on_observation(obs, &mut offline);
                tape.push(obs.tick, &line.text);
                log.push(obs.tick, line);
            }
            (log.checkpoint(), tape)
        };
        let (first_checkpoint, tape) = run_tape(11);
        assert_eq!(tape.len(), 10_000, "every tick recorded");
        assert_eq!(tape.source(), SourceLabel::Recorded);
        // Replay twice offline: fresh announcers plus verbatim tape reads.
        let (second_checkpoint, second) = run_tape(11);
        let (_, third) = run_tape(11);
        assert_eq!(
            second_checkpoint, first_checkpoint,
            "offline replay x2 matches"
        );
        assert_eq!(second.entries(), tape.entries());
        assert_eq!(third.entries(), tape.entries());
        assert_eq!(second.checkpoint(), tape.checkpoint());
        let replayed: Vec<&TapeEntry> = tape.replay().collect();
        assert_eq!(replayed.len(), 10_000);
        assert_eq!(replayed[0].tick, 0);
        assert_eq!(replayed[9_999].tick, 9_999);
        // No network anywhere on this path: an idle closed-port adapter
        // proves replay never dials, and the suite mock never dispatches.
        let port = crate::stub::closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let endpoint = crate::transport::parse_endpoint(&url).expect("test endpoint parses");
        let idle = crate::local::LocalAdapter::new(vec![MockStep::Succeed], endpoint)
            .expect("loopback endpoint accepted");
        assert_eq!(idle.socket_attempts(), 0, "replay made no socket attempt");
        let mut mock = crate::mock::MockAdapter::new(vec![MockStep::Succeed]);
        let policy = crate::mock::fixtures::policy();
        let caps = ProviderCapabilities::mock();
        let outcome = mock.call(&crate::mock::fixtures::request(917, 3), &policy, &caps);
        assert!(!outcome.dispatched_remote, "mock path never dispatches");
        drop(idle);
    }

    #[test]
    fn stream_announcer_and_mind_consume_completed_stream() {
        use super::super::announce::{Announcer, EventLite, FigLite, MatchObservation};

        let obs = MatchObservation {
            tick: 77,
            fighters: vec![FigLite {
                id: 1,
                hp_permille: 640,
                down: false,
            }],
            last_event: Some(EventLite {
                kind: super::super::announce::EventKindLite::Knockdown,
                fighter: 1,
            }),
        };
        // Completed stream text becomes a provider line with kept priority.
        let mut announcer = Announcer::new(3);
        let line = announce_stream_line(&mut announcer, &obs, "Steel glints twice! ");
        assert!(!line.scripted, "completed stream reads as a provider line");
        assert_eq!(line.text, "Steel glints twice!");
        assert_eq!(line.priority, 2, "knockdown priority survives the stream");
        // Blank streams fall back to the scripted table like any provider miss.
        let mut fallback = Announcer::new(3);
        let quiet = announce_stream_line(&mut fallback, &obs, "   ");
        assert!(quiet.scripted);
        assert_eq!(quiet.priority, 2);
        // The LW mind reads the same bytes under its fixed-width bound.
        assert_eq!(
            mind_text_from_stream("  guard the gate  "),
            Some(String::from("guard the gate"))
        );
        assert_eq!(mind_text_from_stream("   "), None, "blank means fallback");
        assert_eq!(
            mind_text_from_stream(&"x".repeat(MIND_TEXT_CAP + 1)),
            None,
            "over-cap text is refused, never cut"
        );
        assert!(
            mind_text_from_stream(&"x".repeat(MIND_TEXT_CAP)).is_some(),
            "cap-sized text stores"
        );
    }

    #[test]
    fn stream_malformed_and_disconnect_surface_failures() {
        // Non-JSON SSE data yields no delta, never a chunk.
        let bad = SseFrame {
            event: None,
            data: String::from("this is not json"),
        };
        assert_eq!(openai_sse_text(&bad), None);
        assert_eq!(anthropic_sse_text(&bad), None);
        assert_eq!(collect_openai_text("this is not json", 64), String::new());
        // Control frames yield no text on either vendor shape.
        let done = SseFrame {
            event: None,
            data: String::from("[DONE]"),
        };
        assert!(is_done_marker(&done));
        assert_eq!(openai_sse_text(&done), None);
        let stop = SseFrame {
            event: Some(String::from("message_stop")),
            data: String::from("{\"type\": \"message_stop\"}"),
        };
        assert_eq!(anthropic_sse_text(&stop), None);
        // Oversize worker input fails validation on the worker side.
        let config = config_for_test();
        let mut malformed = StreamJob::spawn(&config, SourceLabel::Recorded, |feed| {
            let big = "x".repeat(super::super::MAX_TEXT_LEN + 1);
            if feed.push(&big).is_err() {
                feed.fail(StreamError::MalformedOutput);
            } else {
                feed.finish();
            }
        })
        .expect("fixture config valid");
        assert_eq!(
            poll_until_terminal(&mut malformed),
            StreamPoll::Failed(StreamError::MalformedOutput)
        );
        // A worker that vanishes without finishing is a disconnect, and an
        // offline request never opens a stream at all.
        let mut dropped =
            StreamJob::spawn(&config, SourceLabel::Live, |_feed| {}).expect("fixture config valid");
        assert_eq!(
            poll_until_terminal(&mut dropped),
            StreamPoll::Failed(StreamError::Unavailable)
        );
        assert_eq!(
            StreamError::Offline.to_ai_error(),
            AiError::UnsupportedCapability
        );
        assert!(!StreamError::Offline.retryable());
        // Config validation refuses unbounded shapes up front.
        assert_eq!(
            StreamConfig::new(0, 8192, 5_000, 8),
            Err(StreamError::MalformedOutput)
        );
        assert_eq!(
            StreamConfig::new(16, 128, 5_000, 8),
            Err(StreamError::MalformedOutput)
        );
        assert_eq!(
            StreamConfig::new(16, 8192, 0, 8),
            Err(StreamError::MalformedOutput)
        );
    }
}
