//! MCP7 self-describing door: manifest, watch, one error, batch preview.
//!
//! Consumer: external MCP clients over `e2040-mcp` plus MCP9 declarative
//! tools (the manifest feeds tool generation). Data owner stays the editor
//! command service; this module owns only discovery DTOs, the read-only
//! watch hub and the dry-run batch preview. Units follow the scene crate
//! (meters) and arena hashes (`u64` digests). Determinism scope: manifest
//! order is name-sorted, watch delivery is slot-id ordered, batch results
//! are input ordered; no wall clock, no threads. Errors are one shape,
//! [`RpcError`] `{code, message, data}`.
//!
//! Budgets: each watch queue holds at most [`MAX_WATCH_QUEUE`] events,
//! each delivered log at most [`MAX_WATCH_DELIVERED`], one evidence blob at
//! most [`MAX_WATCH_EVIDENCE_BYTES`] bytes. `rpc.discover` and `scene.watch`
//! register on the endpoint router (thirteen core tools plus loaded packs);
//! the [`SyncTool`] shapes here serve through [`McpServer`] either way.

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard, OnceLock,
};

use engine_editor::commands::{begin_group, cancel_group, end_group, CommandMeta};
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::mcp::{
    McpServer, McpToolError, OpDto, SceneReadArgs, SceneReadTool, ServerState, TxApplyArgs,
    TxApplyTool, TxPreviewArgs, TxPreviewOutput, TxPreviewTool, TxUndoArgs, TxUndoTool,
};
use super::scene_tools::SceneBriefArgs;

// ---------------------------------------------------------------------------
// Tool names.
// ---------------------------------------------------------------------------

/// Tool: self-describing manifest with a schema per tool (read-only).
pub const TOOL_RPC_DISCOVER: &str = "rpc.discover";
/// Tool: read-only scene digest subscription (read-only).
pub const TOOL_SCENE_WATCH: &str = "scene.watch";

/// Watch queue bound per subscription: bursts beyond this drop the oldest
/// (counted, never silent) unless coalescing collapses them first.
pub const MAX_WATCH_QUEUE: usize = 64;
/// Delivered-log bound per subscription (the zero-poll read path).
pub const MAX_WATCH_DELIVERED: usize = 256;
/// Evidence-blob bound per subscription (one latest screenshot, bytes).
pub const MAX_WATCH_EVIDENCE_BYTES: usize = 1_048_576;

// ---------------------------------------------------------------------------
// One error shape: {code, message, data}.
// ---------------------------------------------------------------------------

/// The only wire error shape on this door: a machine `code`, a message that
/// names the legal values, and optional `data` for the offending payload.
/// This is the canonical door error shape `{code, message, data}`, shared
/// with [`McpToolError`]: codes always originate as `&'static str` constants
/// (owned here so refusals deserialize off the wire) and every conversion
/// preserves `data`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RpcError {
    /// Machine code (`stale_revision`, `denied_op`, ...).
    pub code: String,
    /// Human detail with the legal values embedded.
    pub message: String,
    /// Offending payload (item index, method name, ...), when it fits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<rmcp::serde_json::Value>,
}

impl RpcError {
    /// New error with a static code and no data.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            data: None,
        }
    }

    /// Attach a JSON payload to this error.
    pub fn with_data(mut self, data: rmcp::serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Machine code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Offending payload, when one fits.
    pub fn data(&self) -> Option<&rmcp::serde_json::Value> {
        self.data.as_ref()
    }

    /// Canonical payload carried in [`rmcp::ErrorData::data`]: the full
    /// `{code, message, data}` shape, so the JSON-RPC layer never drops the
    /// machine code or the culprit.
    pub fn wire_data(&self) -> rmcp::serde_json::Value {
        rmcp::serde_json::json!({
            "code": self.code,
            "message": self.message,
            "data": self.data,
        })
    }

    /// Decode a door refusal from [`rmcp::ErrorData`]: `None` when the
    /// payload did not come from this door (a foreign `data` shape).
    pub fn from_wire(error: &rmcp::ErrorData) -> Option<Self> {
        let object = error.data.as_ref()?.as_object()?;
        let code = object.get("code")?.as_str()?;
        let message = object.get("message")?.as_str()?;
        let data = object.get("data").cloned().filter(|data| !data.is_null());
        Some(Self {
            code: code.to_owned(),
            message: message.to_owned(),
            data,
        })
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

impl From<McpToolError> for RpcError {
    fn from(error: McpToolError) -> Self {
        match error.data {
            Some(data) => Self::new(error.code, error.message).with_data(data),
            None => Self::new(error.code, error.message),
        }
    }
}

impl From<RpcError> for rmcp::ErrorData {
    fn from(error: RpcError) -> Self {
        rmcp::ErrorData::invalid_params(error.to_string(), Some(error.wire_data()))
    }
}

// ---------------------------------------------------------------------------
// rpc.discover: manifest plus a schema per tool.
// ---------------------------------------------------------------------------

/// One manifest row: name, readonly flag, description and both schemas as
/// JSON text. Schemas generate from the registered tools, never twice.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DiscoverEntry {
    /// Tool name (`scene.brief`, `tx.preview`, ...).
    pub name: String,
    /// True when the tool promises no side effects.
    pub readonly: bool,
    /// One-line tool description.
    pub description: String,
    /// Input schema as JSON text (always present on this door).
    pub input_schema: String,
    /// Output schema as JSON text, when the tool advertises one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
}

/// The `rpc.discover` manifest: every advertised tool with its schemas.
/// Name-sorted, so the same server always serves the same manifest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DiscoverManifest {
    /// Protocol revision served (`2026-07-28`).
    pub protocol: String,
    /// Server name (`engine-endpoint`).
    pub server: String,
    /// Crate version serving this manifest.
    pub version: String,
    /// One row per advertised tool, name-sorted.
    pub tools: Vec<DiscoverEntry>,
    /// Hidden system tools omitted from this manifest.
    pub hidden_omitted: u64,
}

impl DiscoverManifest {
    /// Build the manifest from the live router. Hidden `system.*` tools are
    /// omitted unless `include_hidden` passes (they stay reachable by exact
    /// name, never by default listing).
    pub fn from_server(server: &McpServer, include_hidden: bool) -> Self {
        let mut entries: Vec<DiscoverEntry> = server.tools().iter().map(entry_of).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let (tools, hidden_omitted) = filter_hidden(entries, include_hidden);
        Self {
            protocol: "2026-07-28".to_owned(),
            server: "engine-endpoint".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            tools,
            hidden_omitted,
        }
    }

    /// Tools in this manifest.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Schema row for one tool by exact name.
    pub fn schema_for(&self, name: &str) -> Option<&DiscoverEntry> {
        self.tools.iter().find(|entry| entry.name == name)
    }

    /// Discovery cost in chars: names, descriptions and both schemas.
    /// Shared with `mcp::tool_cost_row` through [`discovery_cost_chars`] so
    /// the rows always sum to the total.
    pub fn manifest_chars(&self) -> usize {
        self.tools
            .iter()
            .map(|entry| {
                discovery_cost_chars(
                    entry.name.len(),
                    entry.description.len(),
                    entry.input_schema.len(),
                    entry.output_schema.as_ref().map_or(0, String::len),
                )
            })
            .sum()
    }
}

/// Shared discovery-cost sum in chars: name plus description plus both
/// schemas. Used by [`DiscoverManifest::manifest_chars`] and by the
/// `mcp` tool-cost table so the rows always sum to the total.
pub(crate) fn discovery_cost_chars(
    name_len: usize,
    desc_len: usize,
    input_len: usize,
    output_len: usize,
) -> usize {
    name_len + desc_len + input_len + output_len
}

/// Hidden system tools never appear in the default manifest. The `system.`
/// prefix is reserved for them; every contract tool today is visible.
pub fn is_hidden_tool(name: &str) -> bool {
    name.starts_with("system.")
}

/// Drop hidden rows from a manifest row list, counting the omitted.
pub(crate) fn filter_hidden(
    entries: Vec<DiscoverEntry>,
    include_hidden: bool,
) -> (Vec<DiscoverEntry>, u64) {
    if include_hidden {
        return (entries, 0);
    }
    let mut kept = Vec::with_capacity(entries.len());
    let mut omitted = 0u64;
    for entry in entries {
        if is_hidden_tool(&entry.name) {
            omitted += 1;
        } else {
            kept.push(entry);
        }
    }
    (kept, omitted)
}

fn entry_of(tool: &rmcp::model::Tool) -> DiscoverEntry {
    let input_schema =
        rmcp::serde_json::to_string(&tool.input_schema).unwrap_or_else(|_| "{}".to_owned());
    let output_schema = tool
        .output_schema
        .as_ref()
        .and_then(|schema| rmcp::serde_json::to_string(schema).ok());
    DiscoverEntry {
        name: tool.name.to_string(),
        readonly: tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false),
        description: tool.description.as_deref().unwrap_or("").to_owned(),
        input_schema,
        output_schema,
    }
}

/// Args for `rpc.discover`: hidden tools stay omitted unless asked for.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DiscoverArgs {
    /// Include hidden `system.*` tools (default false).
    #[serde(default)]
    pub include_hidden: Option<bool>,
}

/// `rpc.discover` tool: the self-describing manifest, read-only.
pub struct DiscoverTool;

impl ToolBase for DiscoverTool {
    type Parameter = DiscoverArgs;
    type Output = DiscoverManifest;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_RPC_DISCOVER.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Self-describing manifest: every tool with its input and output schemas. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for DiscoverTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        // The manifest reads the advertised list (which takes the session
        // lock), so it builds before the audited dispatch takes that lock.
        let manifest =
            DiscoverManifest::from_server(service, param.include_hidden.unwrap_or(false));
        service.dispatch(TOOL_RPC_DISCOVER, |_| Ok(manifest))
    }
}

// ---------------------------------------------------------------------------
// scene.watch: read-only digest subscription. Dies on disconnect (Drop).
// ---------------------------------------------------------------------------

/// What a watch event reports: scene digests or play-step digests.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum WatchKind {
    /// A scene brief digest at a document revision.
    Scene,
    /// A play-step digest at a duel tick.
    Play,
}

/// One digest notification: kind, ordering key and the digest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WatchEvent {
    /// Scene or play digest.
    pub kind: WatchKind,
    /// Document revision (scene) or duel tick (play): orders the stream.
    pub revision: u64,
    /// The digest at that revision.
    pub digest: u64,
    /// One-line note (what moved).
    pub note: String,
}

/// Watch subscription policy. Bursts coalesce to the latest digest, the
/// subscriber's own session is suppressed, runtime-error capture is opt-in
/// (default off) and the latest evidence blob rides along when attached.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WatchConfig {
    /// Collapse undrained bursts to the latest digest (default true).
    pub coalesce: bool,
    /// Skip events from the subscriber's own session (default true).
    pub suppress_own: bool,
    /// Record runtime errors into the subscription (default false).
    pub capture_errors: bool,
    /// Keep the latest evidence blob for reads (default false).
    pub include_evidence: bool,
}

impl WatchConfig {
    /// Default policy: coalesce on, own-session suppressed, error capture
    /// off, evidence off.
    pub fn default_policy() -> Self {
        Self {
            coalesce: true,
            suppress_own: true,
            capture_errors: false,
            include_evidence: false,
        }
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self::default_policy()
    }
}

#[derive(Debug)]
struct WatchSlot {
    session: u64,
    config: WatchConfig,
    alive: Arc<AtomicBool>,
    queue: VecDeque<WatchEvent>,
    delivered: Vec<WatchEvent>,
    polls: u64,
    coalesced: u64,
    overflow: u64,
    error_count: u64,
    last_error: Option<RpcError>,
    evidence: Vec<u8>,
}

#[derive(Debug)]
struct HubInner {
    next_id: u64,
    slots: BTreeMap<u64, WatchSlot>,
}

/// The watch hub: subscriptions plus fan-out. Slots are id-ordered, so the
/// same publish sequence always fans out the same way. Clone shares the hub.
#[derive(Debug, Clone)]
pub struct WatchHub {
    inner: Arc<Mutex<HubInner>>,
}

impl WatchHub {
    /// Empty hub; subscription ids start at 1 so 0 stays missing.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HubInner {
                next_id: 1,
                slots: BTreeMap::new(),
            })),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, HubInner>, RpcError> {
        self.inner.lock().map_err(|_| {
            RpcError::new(
                "lock_poisoned",
                "watch hub lock poisoned by a failed call; restart the server",
            )
        })
    }

    /// Open a subscription for one session. The subscription dies on
    /// disconnect ([`Drop`] clears its alive flag; the next publish or
    /// [`WatchHub::prune`] removes the slot). Close it with
    /// [`WatchHub::unsubscribe`]: subscribe -> use -> unsubscribe.
    pub fn subscribe(
        &self,
        session: u64,
        config: WatchConfig,
    ) -> Result<WatchSubscription, RpcError> {
        let mut inner = self.lock()?;
        let id = inner.next_id;
        inner.next_id += 1;
        let alive = Arc::new(AtomicBool::new(true));
        inner.slots.insert(
            id,
            WatchSlot {
                session,
                config,
                alive: Arc::clone(&alive),
                queue: VecDeque::new(),
                delivered: Vec::new(),
                polls: 0,
                coalesced: 0,
                overflow: 0,
                error_count: 0,
                last_error: None,
                evidence: Vec::new(),
            },
        );
        Ok(WatchSubscription {
            hub: self.clone(),
            id,
            session,
            alive,
            detached: false,
        })
    }

    /// Live subscription slots (dead ones prune on publish/prune).
    pub fn live_count(&self) -> Result<usize, RpcError> {
        Ok(self.lock()?.slots.len())
    }

    /// Drop dead slots (subscriptions dropped by their owners).
    pub fn prune(&self) -> Result<usize, RpcError> {
        let mut inner = self.lock()?;
        let before = inner.slots.len();
        inner
            .slots
            .retain(|_, slot| slot.alive.load(Ordering::SeqCst));
        Ok(before - inner.slots.len())
    }

    /// Close one subscription by id: the slot is removed at once, so later
    /// uses answer `unknown_subscription`. Unknown ids are refused the same
    /// way (never silent). This is the end of the subscribe -> use ->
    /// unsubscribe flow; dropping the handle without it only marks the
    /// slot dead until the next publish or prune reaps it.
    pub fn unsubscribe(&self, id: u64) -> Result<(), RpcError> {
        let mut inner = self.lock()?;
        inner
            .slots
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| missing_subscription(id))
    }

    /// Fan one digest out to every live slot. Returns deliveries. Own-session
    /// events skip suppressed slots; bursts coalesce to the latest digest;
    /// full queues drop the oldest and count it. Dead slots are pruned.
    pub fn publish(
        &self,
        kind: WatchKind,
        revision: u64,
        digest: u64,
        note: &str,
        origin: Option<u64>,
    ) -> Result<usize, RpcError> {
        let event = WatchEvent {
            kind,
            revision,
            digest,
            note: note.to_owned(),
        };
        let mut inner = self.lock()?;
        inner
            .slots
            .retain(|_, slot| slot.alive.load(Ordering::SeqCst));
        let mut deliveries = 0usize;
        for slot in inner.slots.values_mut() {
            if slot.config.suppress_own && origin == Some(slot.session) {
                continue;
            }
            if slot.config.coalesce && !slot.queue.is_empty() {
                if let Some(back) = slot.queue.back_mut() {
                    *back = event.clone();
                }
                slot.coalesced += 1;
            } else {
                if slot.queue.len() >= MAX_WATCH_QUEUE {
                    slot.queue.pop_front();
                    slot.overflow += 1;
                }
                slot.queue.push_back(event.clone());
            }
            if slot.delivered.len() >= MAX_WATCH_DELIVERED {
                slot.delivered.remove(0);
            }
            slot.delivered.push(event.clone());
            deliveries += 1;
        }
        Ok(deliveries)
    }

    /// Record a runtime error into opt-in slots only. Slots with
    /// `capture_errors` off never see it (default off).
    pub fn publish_error(&self, code: &'static str, message: &str) -> Result<usize, RpcError> {
        let mut inner = self.lock()?;
        inner
            .slots
            .retain(|_, slot| slot.alive.load(Ordering::SeqCst));
        let mut recorded = 0usize;
        for slot in inner.slots.values_mut() {
            if !slot.config.capture_errors {
                continue;
            }
            slot.error_count += 1;
            slot.last_error = Some(RpcError::new(code, message));
            recorded += 1;
        }
        Ok(recorded)
    }
}

impl Default for WatchHub {
    fn default() -> Self {
        Self::new()
    }
}

fn missing_subscription(id: u64) -> RpcError {
    RpcError::new(
        "unknown_subscription",
        format!("subscription {id} is gone; subscribe again through scene.watch"),
    )
}

/// One read-only digest subscription. Disconnect kills it: [`Drop`] clears
/// the alive flag so the hub stops delivering and prunes the slot.
#[derive(Debug)]
pub struct WatchSubscription {
    hub: WatchHub,
    id: u64,
    session: u64,
    alive: Arc<AtomicBool>,
    /// Set by [`WatchSubscription::detach`]: the hub owns the slot, so drop
    /// leaves it alive until [`WatchHub::unsubscribe`].
    detached: bool,
}

impl WatchSubscription {
    /// Subscription id.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Hand the slot to the hub and return its id: the subscription stays
    /// live after this handle drops, until an explicit
    /// [`WatchHub::unsubscribe`]. For connectionless callers (the MCP tool)
    /// that answer with an id instead of holding the handle.
    pub fn detach(mut self) -> u64 {
        self.detached = true;
        self.id
    }

    /// Owning session.
    pub fn session(&self) -> u64 {
        self.session
    }

    /// Explicit long-polls so far. The delivered log needs zero polls: every
    /// publish lands there directly, so a watcher that only reads
    /// [`WatchSubscription::delivered`] keeps this at 0.
    pub fn polls(&self) -> Result<u64, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.polls)
    }

    /// Queued (undrained) events.
    pub fn queue_len(&self) -> Result<usize, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.queue.len())
    }

    /// Bursts collapsed to the latest digest.
    pub fn coalesced(&self) -> Result<u64, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.coalesced)
    }

    /// Oldest drops on a full queue.
    pub fn overflow(&self) -> Result<u64, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.overflow)
    }

    /// Every digest fanned out to this subscription, in publish order. Reads
    /// without polling: this is the zero-poll path.
    pub fn delivered(&self) -> Result<Vec<WatchEvent>, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.delivered.clone())
    }

    /// Drain the queue like an explicit long-poll. Counts one poll.
    pub fn poll(&self) -> Result<Vec<WatchEvent>, RpcError> {
        let mut inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get_mut(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        slot.polls += 1;
        Ok(slot.queue.drain(..).collect())
    }

    /// Runtime errors recorded (opt-in only).
    pub fn error_count(&self) -> Result<u64, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.error_count)
    }

    /// Latest recorded runtime error, if any.
    pub fn last_error(&self) -> Result<Option<RpcError>, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.last_error.clone())
    }

    /// Attach one evidence blob (a screenshot PNG from a play step). Bounded;
    /// oversize blobs are refused and the old blob stays.
    pub fn attach_evidence(&self, png: Vec<u8>) -> Result<(), RpcError> {
        if png.len() > MAX_WATCH_EVIDENCE_BYTES {
            return Err(RpcError::new(
                "bad_evidence",
                format!(
                    "evidence holds {} bytes; at most {MAX_WATCH_EVIDENCE_BYTES}",
                    png.len()
                ),
            ));
        }
        let mut inner = self.hub.lock()?;
        inner
            .slots
            .get_mut(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?
            .evidence = png;
        Ok(())
    }

    /// Evidence bytes held.
    pub fn evidence_len(&self) -> Result<usize, RpcError> {
        let inner = self.hub.lock()?;
        let slot = inner
            .slots
            .get(&self.id)
            .ok_or_else(|| missing_subscription(self.id))?;
        Ok(slot.evidence.len())
    }
}

impl Drop for WatchSubscription {
    fn drop(&mut self) {
        if !self.detached {
            self.alive.store(false, Ordering::SeqCst);
        }
    }
}

/// Process-wide hub for the tool path (the MCP tool takes no hub argument),
/// mirroring the play-step GPU hook cell. Tests use per-test hubs instead.
static WATCH_HUB: OnceLock<WatchHub> = OnceLock::new();

/// The process-wide watch hub, created on first use.
pub fn global_hub() -> WatchHub {
    WATCH_HUB.get_or_init(WatchHub::new).clone()
}

impl ServerState {
    /// Adopt a new watch subscription into the server-owned slot.
    ///
    /// Replaces any prior watch (the old slot unsubscribes at once), then
    /// records the id plus the baseline digest so `scene.brief` serves the
    /// session-local cache without re-subscribing.
    pub(crate) fn remember_watch(&mut self, id: u64, digest: u64) {
        if let Some(prior) = self.watch_subscription {
            if prior != id {
                let _ = global_hub().unsubscribe(prior);
            }
        }
        self.watch_subscription = Some(id);
        self.watch_digest = Some(digest);
    }

    /// Refresh the session-local digest cache after a scene mutation.
    ///
    /// No-op without a live watch. Otherwise recomputes the brief digest,
    /// caches it, and fans the event out on the process-wide hub (own-session
    /// suppression still applies to hub delivery; the cache always moves).
    /// Hub errors never fail the mutation that already landed.
    pub(crate) fn refresh_watch(&mut self) {
        if self.watch_subscription.is_none() {
            return;
        }
        let digest = match self.scene_brief(&SceneBriefArgs::default()) {
            Ok(brief) => brief.digest,
            Err(_) => return,
        };
        self.watch_digest = Some(digest);
        let _ = global_hub().publish(
            WatchKind::Scene,
            self.doc.revision,
            digest,
            "scene",
            Some(self.session),
        );
    }
}

/// Args for `scene.watch`: open a subscription for one session.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WatchSubscribeArgs {
    /// Session owning the subscription (own-session events suppress).
    pub session: u64,
    /// Collapse undrained bursts (default true).
    #[serde(default)]
    pub coalesce: Option<bool>,
    /// Skip the session's own events (default true).
    #[serde(default)]
    pub suppress_own: Option<bool>,
    /// Record runtime errors (default false).
    #[serde(default)]
    pub capture_errors: Option<bool>,
}

/// `scene.watch` result: the handle plus the live baseline digest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WatchSubscribeOutput {
    /// Subscription handle on the process-wide hub.
    pub subscription_id: u64,
    /// Owning session.
    pub session: u64,
    /// Document revision at subscribe time.
    pub revision: u64,
    /// Scene brief digest at subscribe time.
    pub digest: u64,
}

fn watch_config_of(args: &WatchSubscribeArgs) -> WatchConfig {
    let defaults = WatchConfig::default_policy();
    WatchConfig {
        coalesce: args.coalesce.unwrap_or(defaults.coalesce),
        suppress_own: args.suppress_own.unwrap_or(defaults.suppress_own),
        capture_errors: args.capture_errors.unwrap_or(defaults.capture_errors),
        include_evidence: defaults.include_evidence,
    }
}

/// `scene.watch` tool: open a read-only digest subscription.
pub struct WatchTool;

impl ToolBase for WatchTool {
    type Parameter = WatchSubscribeArgs;
    type Output = WatchSubscribeOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_WATCH.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Read-only scene digest subscription: bursts coalesce to the latest digest. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for WatchTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_SCENE_WATCH, |state| {
            let hub = global_hub();
            let subscription = hub
                .subscribe(param.session, watch_config_of(&param))
                .map_err(|error| McpToolError::wrap("subscribe_failed", error))?;
            let baseline = state.scene_brief(&SceneBriefArgs::default())?;
            let output = WatchSubscribeOutput {
                subscription_id: subscription.id(),
                session: param.session,
                revision: baseline.revision,
                digest: baseline.digest,
            };
            // The tool holds no connection, so the hub adopts the slot:
            // detaching keeps the alive flag set until an explicit
            // unsubscribe. A plain drop would mark the slot dead and the
            // prune below would remove it, returning an id that answers
            // unknown_subscription on next use. The id plus baseline digest
            // also land in the server-owned slot, replacing any prior
            // watch, so scene.brief serves the session-local cache without
            // re-subscribing.
            let digest = baseline.digest;
            let id = subscription.detach();
            state.remember_watch(id, digest);
            hub.prune()
                .map_err(|error| McpToolError::wrap("subscribe_failed", error))?;
            Ok(output)
        })
    }
}

// ---------------------------------------------------------------------------
// Batch preview: per-item {ok, error} with default deny, no side effects.
// ---------------------------------------------------------------------------

/// Methods that map onto the single state-patch path. Anything else is
/// denied by default (default deny, never an open set).
pub const STATE_PATCH_METHODS: &[&str] = &["list", "query", "get", "set", "spawn", "watch"];

/// The single patch path every mapped method lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchKind {
    /// The one state.patch operation.
    StatePatch,
}

/// Map one method name onto the single patch path. Unknown methods are
/// refused with the legal set plus the closest fuzzy suggestion attached
/// (default deny).
pub fn map_method(name: &str) -> Result<PatchKind, RpcError> {
    if STATE_PATCH_METHODS.contains(&name) {
        return Ok(PatchKind::StatePatch);
    }
    let mut message = format!(
        "method `{name}` is denied; mapped methods: [{}] \
         (every mapped method lands on the single state.patch path)",
        STATE_PATCH_METHODS.join(", ")
    );
    if let Some(close) = suggest_method(name) {
        message.push_str(&format!("; did you mean `{close}`?"));
    }
    Err(RpcError::new("denied_op", message)
        .with_data(rmcp::serde_json::Value::String(name.to_owned())))
}

/// One batch preview item: a named method plus one command batch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewItem {
    /// Method name, mapped through [`map_method`] (default deny).
    pub method: String,
    /// Must equal the document revision, else this item errors.
    pub expected_revision: u64,
    /// Retry identity for this item.
    pub idempotency_key: u64,
    /// Edits in order.
    pub ops: Vec<OpDto>,
}

/// The dry-run result for one item: effects without side effects.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewOk {
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// `base_revision + 1`; the revision an apply would publish.
    pub resulting_revision: u64,
    /// Canonical hash of the exact batch.
    pub action_hash: u64,
}

/// Single owner for the `tx.preview` success seal: moves the dry-run
/// `effects` plus the revision/hash the apply would publish. Call sites
/// must call this instead of re-spelling the three-field copy.
pub(crate) fn preview_ok_from_tx(preview: TxPreviewOutput) -> PreviewOk {
    PreviewOk {
        effects: preview.effects,
        resulting_revision: preview.resulting_revision,
        action_hash: preview.action_hash,
    }
}

/// Per-item preview answer: exactly one of `ok` / `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewResult {
    /// Item index in the request.
    pub index: usize,
    /// Dry-run effects, when the item validates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<PreviewOk>,
    /// The refusal, when the item fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl PreviewResult {
    /// True when exactly one of `ok` / `error` is set.
    pub fn is_shaped(&self) -> bool {
        self.ok.is_some() != self.error.is_some()
    }
}

/// Dry-run a batch of items against the live document: per-item `{ok,
/// error}`, input ordered, no side effects. A denied method, a stale
/// revision, an empty batch or a spatially-rejected batch errors its item
/// and writes nothing.
pub fn preview_batch(server: &McpServer, items: &[PreviewItem]) -> Vec<PreviewResult> {
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if let Err(denied) = map_method(&item.method) {
            out.push(PreviewResult {
                index,
                ok: None,
                error: Some(denied),
            });
            continue;
        }
        let args = TxPreviewArgs {
            expected_revision: item.expected_revision,
            idempotency_key: item.idempotency_key,
            ops: item.ops.clone(),
        };
        match TxPreviewTool::invoke(server, args) {
            Ok(preview) => out.push(PreviewResult {
                index,
                ok: Some(preview_ok_from_tx(preview)),
                error: None,
            }),
            Err(tool) => out.push(PreviewResult {
                index,
                ok: None,
                error: Some(RpcError::from(tool)),
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Domain rollup: manage(op) school over preview/apply/undo, fuzzy suggestions,
// atomic batch_execute. Twenty tools unchanged, no second write path.
// Sampling is deprecated: this rollup runs on the loopback server with the
// session token only and never samples a model; replay uses recorded accepted
// actions only.
// ---------------------------------------------------------------------------

/// Furthest typo still offered as a suggestion (chars of edit distance).
/// Units: chars. Small so `lst` finds `list` but `exec` offers nothing.
pub const MAX_SUGGEST_DISTANCE: usize = 2;

/// One-line rollup hint for weak models. Short by construction: the full
/// manifest stays on `rpc.discover`, this hint only names the six managed
/// methods plus the atomic entry point, so tool selection costs chars down.
pub const ROLLUP_HELP: &str = "manage(op): list|query|get|set|spawn|watch -> tx.preview, then tx.apply/tx.undo; batch_execute is atomic via preview/apply/undo";

/// Chars of [`ROLLUP_HELP`]: the selection hint budget, always far below the
/// full manifest cost.
pub fn rollup_help_chars() -> usize {
    ROLLUP_HELP.len()
}

/// Edit distance over lowercased chars (insert/delete/substitute cost 1).
/// Pure over its inputs; inputs are short tool/method names.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let deletion = prev[j + 1].saturating_add(1);
            let insertion = curr[j].saturating_add(1);
            let substitution = prev[j].saturating_add(cost);
            curr[j + 1] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Closest option within [`MAX_SUGGEST_DISTANCE`], case-insensitive.
/// Trims whitespace; empty input offers nothing. Pure.
fn best_match(name: &str, options: &[&'static str]) -> Option<&'static str> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let mut best: Option<(&'static str, usize)> = None;
    for option in options {
        let distance = edit_distance(&needle, &option.to_lowercase());
        let is_better = best.is_none_or(|(_, known)| distance < known);
        if is_better {
            best = Some((option, distance));
        }
    }
    best.and_then(|(option, distance)| {
        if distance <= MAX_SUGGEST_DISTANCE {
            Some(option)
        } else {
            None
        }
    })
}

/// Closest managed method for a typo (`lst` -> `list`). `None` when nothing
/// is close (`exec` offers nothing). Pure.
pub fn suggest_method(name: &str) -> Option<&'static str> {
    best_match(name, STATE_PATCH_METHODS)
}

/// Closest reachable tool for a typo (`scene.red` -> `scene.read`).
/// `None` when nothing is close. Pure; reads only the static tool table.
pub fn suggest_tool(name: &str) -> Option<&'static str> {
    let tools = super::mcp::tool_names();
    best_match(name, &tools)
}

/// Resolve one tool name exactly. Unknown names refuse with code
/// `unknown_tool`, the legal reachable names plus the closest fuzzy
/// suggestion when one is close (tool-selection accuracy up, one round trip
/// saved). Pure.
pub fn resolve_tool(name: &str) -> Result<&'static str, RpcError> {
    for known in super::mcp::tool_names() {
        if known == name {
            return Ok(known);
        }
    }
    let mut message = format!(
        "unknown tool `{name}`; the door serves these tools: {}",
        super::mcp::tool_names().join(", ")
    );
    if let Some(close) = suggest_tool(name) {
        message.push_str(&format!("; did you mean `{close}`?"));
    } else {
        message.push_str("; re-run with one of those names");
    }
    Err(RpcError::new("unknown_tool", message)
        .with_data(rmcp::serde_json::Value::String(name.to_owned())))
}

/// Managed op school (the `manage(op)` domain): the six method names that all
/// land on the single `state.patch` path. Typed so callers stop passing raw
/// strings; parsing keeps default deny plus the fuzzy suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManageOp {
    /// Read-shaped patch intent.
    List,
    /// Spatial-question patch intent.
    Query,
    /// Single-node fetch patch intent.
    Get,
    /// Mutating patch intent.
    Set,
    /// Fresh-node patch intent.
    Spawn,
    /// Watch-subscription patch intent.
    Watch,
}

impl ManageOp {
    /// Canonical method name.
    pub fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Query => "query",
            Self::Get => "get",
            Self::Set => "set",
            Self::Spawn => "spawn",
            Self::Watch => "watch",
        }
    }

    /// Every managed method name, in contract order.
    pub fn all_names() -> [&'static str; 6] {
        ["list", "query", "get", "set", "spawn", "watch"]
    }

    /// Parse one method name (exact only). Unknown names refuse with code
    /// `denied_op`, the legal six plus the closest suggestion when close.
    pub fn parse(name: &str) -> Result<Self, RpcError> {
        match name {
            "list" => Ok(Self::List),
            "query" => Ok(Self::Query),
            "get" => Ok(Self::Get),
            "set" => Ok(Self::Set),
            "spawn" => Ok(Self::Spawn),
            "watch" => Ok(Self::Watch),
            _ => Err(map_method(name).expect_err("unknown manage op denies")),
        }
    }
}

/// Domain rollup dispatch: one `manage(op)` entry over the existing
/// `tx.preview` path. Maps `method` through [`map_method`] (default deny
/// with suggestion), then dry-runs the batch with no side effects. Approvals
/// keep the session, revision, hash and expiry binding; `tx.apply` still lands
/// the batch and `tx.undo` still reverts it. No second write path: this never
/// touches the document or journal directly. Sampling is deprecated:
/// loopback server plus session token only.
pub fn dispatch_manage(
    server: &McpServer,
    method: &str,
    expected_revision: u64,
    idempotency_key: u64,
    ops: Vec<OpDto>,
) -> Result<PreviewOk, RpcError> {
    map_method(method)?;
    match TxPreviewTool::invoke(
        server,
        TxPreviewArgs {
            expected_revision,
            idempotency_key,
            ops,
        },
    ) {
        Ok(preview) => Ok(preview_ok_from_tx(preview)),
        Err(tool) => Err(RpcError::from(tool)),
    }
}

/// Atomic batch outcome: per-item results plus how many stay landed and
/// the live revision after the call. On full success `applied == items.len()`
/// and `revision == base + applied`. On any failure nothing stays landed
/// (`applied == 0`): the failing item carries the refusal and every landed
/// batch is reverted with `tx.undo`, which appends undo revisions, so
/// `revision` moves past base while the document content matches base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchExecuteOutput {
    /// Per-item answers, input ordered, exactly one of `ok`/`error` set.
    pub results: Vec<PreviewResult>,
    /// Batches still landed after rollback (0 on any failure).
    pub applied: usize,
    /// Live document revision after the call (content matches base on failure,
    /// at a moved revision since undo appends).
    pub revision: u64,
}

/// Atomically execute a batch of managed items over the existing
/// preview/apply/undo path: dry-run every item first (no side effects), then
/// preview plus apply each item in order at its chained revision, reverting
/// every landed batch with `tx.undo` when any item fails (undo appends new
/// revisions, so the content matches base at a moved revision). Each item
/// keeps the session, revision, hash and expiry approval binding (fresh
/// preview approval per chained revision); denied, stale, duplicate or
/// spatially-rejected items land nothing. Sampling is deprecated: loopback
/// server plus session token only, no model in the loop.
pub fn batch_execute(server: &McpServer, items: &[PreviewItem]) -> BatchExecuteOutput {
    let base = match SceneReadTool::invoke(server, SceneReadArgs::default()) {
        Ok(page) => page.revision,
        Err(tool) => {
            let error = RpcError::from(tool);
            let results = items
                .iter()
                .enumerate()
                .map(|(index, _)| PreviewResult {
                    index,
                    ok: None,
                    error: Some(error.clone()),
                })
                .collect();
            return BatchExecuteOutput {
                results,
                applied: 0,
                revision: 0,
            };
        }
    };
    if items.is_empty() {
        return BatchExecuteOutput {
            results: Vec::new(),
            applied: 0,
            revision: base,
        };
    }
    // Gate first: the dry run writes nothing, so a refusal here lands none.
    let dry = preview_batch(server, items);
    if dry.iter().any(|result| result.error.is_some()) {
        return BatchExecuteOutput {
            results: dry,
            applied: 0,
            revision: base,
        };
    }
    apply_chained(server, items, base)
}

/// Chained preview plus apply for a fully dry-clean batch: item `index`
/// previews and applies at `base + applied`, so each approval binds its live
/// revision. First failure reverts every landed batch and reports the tail
/// as never-ran. Never exceeds one screen: failure shaping lives in
/// [`rolled_back_output`].
fn apply_chained(server: &McpServer, items: &[PreviewItem], base: u64) -> BatchExecuteOutput {
    let mut results: Vec<PreviewResult> = Vec::with_capacity(items.len());
    let mut applied: usize = 0;
    for (index, item) in items.iter().enumerate() {
        if map_method(&item.method).is_err() {
            let denied = map_method(&item.method).expect_err("denied method errors");
            return rolled_back_output(server, items, &results, index, denied, base, applied);
        }
        let applied_u64 = u64::try_from(applied).unwrap_or(u64::MAX);
        let current = base.saturating_add(applied_u64);
        let preview = match TxPreviewTool::invoke(
            server,
            TxPreviewArgs {
                expected_revision: current,
                idempotency_key: item.idempotency_key,
                ops: item.ops.clone(),
            },
        ) {
            Ok(preview) => preview,
            Err(tool) => {
                return rolled_back_output(
                    server,
                    items,
                    &results,
                    index,
                    RpcError::from(tool),
                    base,
                    applied,
                );
            }
        };
        let approval = preview.approval.clone();
        match TxApplyTool::invoke(
            server,
            TxApplyArgs {
                expected_revision: current,
                idempotency_key: item.idempotency_key,
                ops: item.ops.clone(),
                approval,
                delta: Some(false),
            },
        ) {
            Ok(_) => {
                applied += 1;
                results.push(PreviewResult {
                    index,
                    ok: Some(PreviewOk {
                        effects: preview.effects,
                        resulting_revision: current.saturating_add(1),
                        action_hash: preview.action_hash,
                    }),
                    error: None,
                });
            }
            Err(tool) => {
                return rolled_back_output(
                    server,
                    items,
                    &results,
                    index,
                    RpcError::from(tool),
                    base,
                    applied,
                );
            }
        }
    }
    BatchExecuteOutput {
        results,
        applied,
        revision: live_revision(server, base),
    }
}

/// Shape a rollback outcome: keep the landed-then-undone prefix, report the
/// refusal at `failed`, and mark the rest never-ran. Reverts `landed`
/// batches first so nothing stays applied.
fn rolled_back_output(
    server: &McpServer,
    items: &[PreviewItem],
    done: &[PreviewResult],
    failed: usize,
    error: RpcError,
    base: u64,
    landed: usize,
) -> BatchExecuteOutput {
    rollback_applied(server, landed);
    let mut out = Vec::with_capacity(items.len());
    for result in done {
        out.push(result.clone());
    }
    out.push(PreviewResult {
        index: failed,
        ok: None,
        error: Some(error),
    });
    for (rest_index, _) in items.iter().enumerate().skip(failed + 1) {
        out.push(PreviewResult {
            index: rest_index,
            ok: None,
            error: Some(never_ran_error(rest_index, base)),
        });
    }
    BatchExecuteOutput {
        results: out,
        applied: 0,
        revision: live_revision(server, base),
    }
}

/// Never-ran tail marker: the batch rolled back, so this item never executed.
fn never_ran_error(rest_index: usize, base: u64) -> RpcError {
    RpcError::new(
        "stale_revision",
        format!("item {rest_index} never ran; the batch rolled back to revision {base}"),
    )
}

/// Undo `count` newest batches; best effort, failures stop the rollback.
/// Crate-private helper for [`batch_execute`]: the caller re-reads the live
/// revision afterwards, so a stuck journal surfaces as a moved revision.
fn rollback_applied(server: &McpServer, count: usize) {
    for _ in 0..count {
        if TxUndoTool::invoke(server, TxUndoArgs { undo_token: None }).is_err() {
            break;
        }
    }
}

/// Live revision, falling back to `base` when the read itself refuses.
/// Never panics; the batch output always names a revision.
fn live_revision(server: &McpServer, base: u64) -> u64 {
    match SceneReadTool::invoke(server, SceneReadArgs::default()) {
        Ok(page) => page.revision,
        Err(_) => base,
    }
}

/// Fyrox group-seal predicate: one node, one value kind, single-op writes only.
/// PORT(Fyrox@048d395 editor/src/command/mod.rs CommandGroup execute-in-order plus end_group merge) -> seal-at-batch-end shape only, idea only. Structural ops never fold.
pub fn gesture_sealable(items: &[PreviewItem]) -> bool {
    if items.is_empty() {
        return false;
    }
    let (first_id, first_kind) = match &*items[0].ops {
        [OpDto::SetLocal { id, .. }] => (id, 0u8),
        [OpDto::SetMaterial { id, .. }] => (id, 1u8),
        _ => return false,
    };
    for item in items {
        if map_method(&item.method).is_err() {
            return false;
        }
        match &*item.ops {
            [OpDto::SetLocal { id, .. }] if id == first_id && first_kind == 0 => {}
            [OpDto::SetMaterial { id, .. }] if id == first_id && first_kind == 1 => {}
            _ => return false,
        }
    }
    true
}

/// Batch tagged as one gesture folds to a single undo entry on the session path.
/// Caller: `crate::remote_shim::execute_via_shim`. Keeps first inverse via `end_group`, forward ends at newest value; zero-delta annuls; pinned lane (key 0 or journaled key) never folds.
pub fn batch_execute_grouped(
    server: &McpServer,
    items: &[PreviewItem],
    gesture_key: u64,
) -> BatchExecuteOutput {
    if gesture_key == 0 || !gesture_sealable(items) {
        return batch_execute(server, items);
    }
    if let Ok(state) = server.lock() {
        if state.journal.past.iter().any(|e| e.key == gesture_key) {
            drop(state);
            return batch_execute(server, items);
        }
    }
    if items.first().map(|i| &i.ops) == items.last().map(|i| &i.ops) {
        let dry = preview_batch(server, items);
        return BatchExecuteOutput {
            results: dry,
            applied: 0,
            revision: live_revision(server, 0),
        };
    }
    if let Ok(mut state) = server.lock() {
        begin_group(
            &mut state.journal,
            &CommandMeta::significant("gesture drag"),
            gesture_key,
        );
    } else {
        return batch_execute(server, items);
    }
    let out = batch_execute(server, items);
    if out.applied == items.len() {
        if let Ok(mut state) = server.lock() {
            let _ = end_group(&mut state.journal);
        }
    } else if let Ok(mut state) = server.lock() {
        cancel_group(&mut state.journal);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::mcp::{
        SceneReadArgs, SceneReadTool, TxApplyArgs, TxApplyTool, TOOL_LIST_BUDGET_CHARS,
    };
    use super::super::play::play_session::PlaySession;
    use super::super::play::PlayInputDto;
    use super::super::scene_tools::SceneBriefTool;
    use super::*;
    use engine_editor::capability::Capability;
    use engine_scene::ids::ObjectId;
    use rmcp::handler::server::router::tool::{SyncTool, ToolBase};

    const DUEL_SEED: u64 = 2040;

    fn test_server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn revision_of(server: &McpServer) -> u64 {
        SceneReadTool::invoke(server, SceneReadArgs::default())
            .expect("scene reads")
            .revision
    }

    /// One duel step's inputs: fighter 0 pushes +x (attack every fourth
    /// step), fighter 1 pushes -x. Mirrors the VP1 session shape.
    fn step_inputs(base: u32, step: u32) -> Vec<PlayInputDto> {
        let mut out = Vec::new();
        for tick in base..base + 2 {
            out.push(PlayInputDto {
                tick,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: u8::from(step.is_multiple_of(4)),
            });
            out.push(PlayInputDto {
                tick,
                fighter: 1,
                mx: -60,
                mz: 0,
                facing: 0,
                buttons: 0,
            });
        }
        out
    }

    fn fresh_budget() -> (usize, DiscoverManifest) {
        let server = test_server();
        // The shared tool-list budget bounds what a fresh session advertises
        // (the core plus the default scene pack); packs a session loads on
        // demand add to its list and are not part of that bound.
        let fresh = DiscoverManifest::from_server(&server, false).manifest_chars();
        assert!(fresh > 0, "manifest counts something");
        assert!(
            fresh <= TOOL_LIST_BUDGET_CHARS,
            "fresh manifest costs {fresh} chars, budget is {TOOL_LIST_BUDGET_CHARS}"
        );
        // The manifest lists what the session advertises: load the view and
        // rules packs so every old tool is on it.
        {
            let mut state = server.lock().expect("test lock holds");
            state.load_pack("view", false).expect("view loads");
            state.load_pack("rules", false).expect("rules loads");
        }
        let manifest = DiscoverManifest::from_server(&server, false);
        assert_eq!(
            manifest.tool_count(),
            server.tools().len(),
            "the manifest covers the advertised door"
        );
        assert!(
            manifest.tool_count() >= 21,
            "core plus scene, view and rules reach past the old 21"
        );
        assert_eq!(manifest.hidden_omitted, 0, "no hidden tools exist yet");
        assert_eq!(manifest.protocol, "2026-07-28");
        assert_eq!(manifest.server, "engine-endpoint");
        assert!(!manifest.version.is_empty());
        let names: Vec<&str> = manifest
            .tools
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "manifest order is name-sorted");
        (fresh, manifest)
    }

    fn manifest_shape(manifest: &DiscoverManifest) {
        for entry in &manifest.tools {
            assert!(
                !entry.input_schema.is_empty(),
                "tool `{}` must advertise its input schema",
                entry.name
            );
            assert!(
                entry.output_schema.is_some(),
                "tool `{}` must advertise its output schema",
                entry.name
            );
            assert!(
                !entry.description.is_empty(),
                "tool `{}` must carry its one-line description",
                entry.name
            );
        }
        // Readonly flags ride the rmcp annotations.
        let brief = manifest
            .schema_for("scene.brief")
            .expect("brief is advertised");
        assert!(brief.readonly);
        let apply = manifest
            .schema_for("tx.apply")
            .expect("apply is advertised");
        assert!(!apply.readonly);
        // Schema per tool: the tx.preview schema names its ops field.
        let preview = manifest
            .schema_for("tx.preview")
            .expect("preview is advertised");
        assert!(
            preview.input_schema.contains("ops"),
            "got: {}",
            preview.input_schema
        );
        assert!(manifest.schema_for("no-such-tool").is_none());
    }

    fn pack_growth(fresh: usize, manifest: &DiscoverManifest) {
        // MCP9 registers the self-description tools on the router itself.
        let discover = manifest
            .schema_for("rpc.discover")
            .expect("discover is advertised");
        assert!(discover.readonly);
        assert!(discover.input_schema.contains("include_hidden"));
        let watch = manifest
            .schema_for("scene.watch")
            .expect("watch is advertised");
        assert!(watch.readonly);
        assert!(watch.input_schema.contains("session"));
        let query = manifest
            .schema_for("scene.query")
            .expect("query is advertised");
        assert!(query.readonly);
        assert!(query.input_schema.contains("kind"));
        let goals = manifest
            .schema_for("sim.plan_goals")
            .expect("plan_goals is advertised");
        assert!(goals.readonly);
        assert!(goals.input_schema.contains("brief"));
        // The level builder rides the same manifest: a gated writer, so its
        // row is present and marked as mutating.
        let level = manifest
            .schema_for("level.build")
            .expect("level is advertised");
        assert!(!level.readonly);
        assert!(level.input_schema.contains("brief"));
        assert!(level.output_schema.is_some());
        // Loading packs grows the list, and every grown row is counted.
        let chars = manifest.manifest_chars();
        eprintln!("manifest: fresh {fresh} chars, view and rules loaded {chars} chars");
        assert!(chars > fresh, "loaded packs add to the manifest");
    }

    #[test]
    fn discover_manifest_covers_all_twenty_one_tools_with_schemas() {
        let (fresh, manifest) = fresh_budget();
        manifest_shape(&manifest);
        pack_growth(fresh, &manifest);
    }

    #[test]
    fn discover_tool_is_readonly_and_hidden_filter_counts() {
        let server = test_server();
        let manifest = DiscoverTool::invoke(
            &server,
            DiscoverArgs {
                include_hidden: None,
            },
        )
        .expect("discover serves");
        assert_eq!(manifest.tool_count(), server.tools().len());
        let annotations = DiscoverTool::annotations().expect("annotations ride along");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert!(
            DiscoverTool::input_schema().is_some(),
            "discover has a schema"
        );
        assert!(
            DiscoverTool::output_schema().is_some(),
            "discover answers a schema"
        );

        // Hidden filtering: system tools omit by default, count when hidden.
        assert!(is_hidden_tool("system.trace"));
        assert!(!is_hidden_tool("scene.brief"));
        let rows = vec![
            DiscoverEntry {
                name: "scene.brief".to_owned(),
                readonly: true,
                description: "brief".to_owned(),
                input_schema: "{}".to_owned(),
                output_schema: None,
            },
            DiscoverEntry {
                name: "system.trace".to_owned(),
                readonly: true,
                description: "trace".to_owned(),
                input_schema: "{}".to_owned(),
                output_schema: None,
            },
        ];
        let (visible, omitted) = filter_hidden(rows.clone(), false);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "scene.brief");
        assert_eq!(omitted, 1);
        let (all, omitted) = filter_hidden(rows, true);
        assert_eq!(all.len(), 2);
        assert_eq!(omitted, 0);
    }

    #[test]
    fn one_error_shape_roundtrips_code_message_data() {
        let tool = McpToolError::new("stale_revision", "document moved; re-read first");
        let error = RpcError::from(tool);
        assert_eq!(error.code(), "stale_revision");
        assert!(error.message.contains("re-read"));
        assert!(error.data.is_none(), "tool errors convert with no data");
        // Batch denials attach the culprit method as data.
        let denied = map_method("exec").expect_err("exec is denied");
        assert_eq!(denied.code(), "denied_op");
        assert!(denied.message.contains("list"));
        assert_eq!(
            denied.data,
            Some(rmcp::serde_json::Value::String("exec".to_owned()))
        );
        // The shape serializes as exactly {code, message, data}.
        let text = rmcp::serde_json::to_string(&denied).expect("error serializes");
        let value: rmcp::serde_json::Value =
            rmcp::serde_json::from_str(&text).expect("error is JSON");
        let object = value.as_object().expect("error is an object");
        assert!(object.contains_key("code"));
        assert!(object.contains_key("message"));
        assert!(object.contains_key("data"));
        let back: RpcError = rmcp::serde_json::from_str(&text).expect("error round-trips");
        assert_eq!(back, denied);
        // Data-less errors omit the data key on the wire.
        let plain = rmcp::serde_json::to_string(&error).expect("plain error serializes");
        let plain_value: rmcp::serde_json::Value =
            rmcp::serde_json::from_str(&plain).expect("plain error is JSON");
        assert!(
            !plain_value
                .as_object()
                .expect("object")
                .contains_key("data"),
            "unset data stays off the wire: {plain}"
        );
    }

    /// Every machine code the door emits, in one table: the wire test below
    /// round-trips each of these with and without `data`. Enumerated from
    /// every `McpToolError::new` / `McpToolError::wrap` plus `RpcError::new`
    /// literal on HEAD, miss none, plus every dynamic passthrough that keeps
    /// the source code: [`LogError::code`](super::super::log_tools::LogError)
    /// (`unknown_code`), [`ExplainRefusal::code`](super::super::explain::ExplainRefusal)
    /// (`bad_caps`, `explain_over_budget`), [`error_code`](super::super::insight::index::error_code)
    /// (`unknown_path`, `empty_query`, `walk_failed`),
    /// [`BriefError::code`](super::super::build_encounter::BriefError)
    /// (`empty_brief`, `brief_too_long` via `sim.plan_goals`),
    /// [`QueryError::new`](super::super::scene_query::QueryError) sites
    /// (`bad_kind`, `bad_radius`, `bad_point`, `bad_capsule`, `no_space`,
    /// `bad_gap`) and [`Verdict::as_tool_error`](super::super::tool_capabilities::Verdict)
    /// (`unknown_tool` on the dispatch seam, `destructive_tier` under
    /// `check_write` below door tier).
    const TOOL_ERROR_CODES: &[&str] = &[
        "apply_in_flight",
        "audit_exhausted",
        "bad_action",
        "bad_brief",
        "bad_budget",
        "bad_caps",
        "bad_capsule",
        "bad_check",
        "bad_cursor",
        "bad_evidence",
        "bad_extent",
        "bad_fighters",
        "bad_gap",
        "bad_id",
        "bad_job_id",
        "bad_kind",
        "bad_limit",
        "bad_material",
        "bad_participants",
        "bad_point",
        "bad_pose",
        "bad_radius",
        "bad_receipt",
        "bad_relation",
        "bad_size",
        "bad_ticks",
        "bad_ttl",
        "bad_wall",
        "bad_want",
        "bad_word",
        "black_capture",
        "brief_too_long",
        "budget_headroom_exhausted",
        "denied_op",
        "destructive_tier",
        "duplicate_input",
        "duplicate_key",
        "empty_batch",
        "empty_brief",
        "empty_checks",
        "empty_query",
        "expired_approval",
        "explain_over_budget",
        "forged_approval",
        "injection_refused",
        "invalid_batch",
        "invalid_config",
        "invalid_tape",
        "job_already_done",
        "job_not_running",
        "job_table_full",
        "lock_poisoned",
        "manifest_mismatch",
        "needs_nav_view",
        "no_space",
        "not_a_root",
        "nothing_to_undo",
        "op_not_allowed",
        "out_of_scope",
        "over_budget",
        "reused_approval",
        "snapshot_failed",
        "spatial_rejected",
        "stale_revision",
        "stale_tick",
        "subscribe_failed",
        "too_many_checks",
        "too_many_inputs",
        "too_many_ops",
        "too_many_wants",
        "tool_list_bloated",
        "undo_mismatch",
        "undone_key",
        "unknown_actor",
        "unknown_check",
        "unknown_code",
        "unknown_job",
        "unknown_path",
        "unknown_subscription",
        "unknown_tool",
        "unsorted_tape",
        "unsupported_capture",
        "walk_failed",
    ];

    /// One hop of the wire: `McpToolError` converts, serializes and decodes
    /// back with code, message and data equal at every step.
    fn roundtrip_tool_error(code: &'static str, data: Option<rmcp::serde_json::Value>) {
        let message = format!("{code} refuses the Fixture probe; re-read first");
        let tool = match data {
            Some(ref data) => McpToolError::new(code, message.clone()).with_data(data.clone()),
            None => McpToolError::new(code, message.clone()),
        };
        assert_eq!(tool.code(), code);
        // Tool shape converts to the wire shape with data preserved.
        let wire = RpcError::from(tool);
        assert_eq!(wire.code(), code);
        assert_eq!(wire.message, message);
        assert_eq!(wire.data, data);
        // The wire shape serializes as exactly {code, message, data}.
        let text = rmcp::serde_json::to_string(&wire).expect("error serializes");
        let back: RpcError = rmcp::serde_json::from_str(&text).expect("error round-trips");
        assert_eq!(back, wire, "code `{code}` survives JSON");
        // The transport envelope carries the full canonical shape in `data`.
        let envelope = rmcp::ErrorData::from(wire.clone());
        assert!(
            envelope.message.contains(code),
            "envelope names the code: {}",
            envelope.message
        );
        let decoded = RpcError::from_wire(&envelope).expect("door envelopes decode back");
        assert_eq!(decoded, wire, "code `{code}` survives the envelope");
    }

    #[test]
    fn every_tool_error_roundtrips_the_wire_with_and_without_data() {
        let mut sorted = TOOL_ERROR_CODES.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, TOOL_ERROR_CODES, "the table stays sorted");
        assert!(
            !TOOL_ERROR_CODES.contains(&""),
            "no empty code rides the wire"
        );
        for code in TOOL_ERROR_CODES {
            roundtrip_tool_error(code, Some(rmcp::serde_json::json!({"item": code})));
            roundtrip_tool_error(code, None);
        }
        // A foreign envelope never decodes as a door refusal.
        let foreign = rmcp::ErrorData::invalid_params("gone", None);
        assert!(RpcError::from_wire(&foreign).is_none());
        let shaped =
            rmcp::ErrorData::invalid_params("gone", Some(rmcp::serde_json::json!({"code": 7})));
        assert!(RpcError::from_wire(&shaped).is_none());
    }

    /// Every dynamic passthrough keeps its source code in the table above:
    /// each assertion below derives the code from the source type, so adding
    /// a variant without tabling it fails here instead of silently narrowing
    /// the wire contract.
    #[test]
    fn every_dynamic_code_source_is_tabled() {
        use super::super::build_encounter::BriefError;
        use super::super::explain::{ExplainCaps, ExplainRefusal};
        use super::super::insight::index::{error_code, InsightError};
        use super::super::log_tools::LogError;
        use super::super::scene_query::QueryError;
        use super::super::tool_capabilities::{check_write, refuse_tool_call};
        use engine_editor::commands::{Batch, Document, Op};

        fn assert_tabled(code: &str) {
            assert!(
                TOOL_ERROR_CODES.contains(&code),
                "code `{code}` rides the wire but is not tabled"
            );
        }
        // `From<LogError>`: the log tools keep the log code.
        assert_tabled(
            LogError::BadCursor {
                cursor: 9,
                oldest: 0,
                newest: 0,
            }
            .code(),
        );
        assert_tabled(LogError::BadLimit { limit: 0 }.code());
        assert_tabled(LogError::BadBudget { budget: 0 }.code());
        assert_tabled(LogError::UnknownCode { code: 9999 }.code());
        // `ExplainRefusal::to_tool_error`: the EXPLAIN walk keeps its code.
        let refusal = |metric: &'static str| ExplainRefusal {
            metric,
            value: 1,
            bound: 0,
            invariant: format!("{metric} 1 exceeds cap 0"),
            recovery: "shrink the batch".to_owned(),
        };
        assert_eq!(refusal("caps").code(), "bad_caps");
        assert_tabled(refusal("caps").code());
        assert_eq!(refusal("objects").code(), "explain_over_budget");
        assert_tabled(refusal("objects").code());
        // `error_code`: the Insight tools keep the index code.
        assert_tabled(error_code(&InsightError::UnknownPath {
            path: "missing".to_owned(),
        }));
        assert_tabled(error_code(&InsightError::EmptyQuery));
        assert_tabled(error_code(&InsightError::BadLimit { got: 0 }));
        assert_tabled(error_code(&InsightError::Walk {
            path: "missing".to_owned(),
            reason: "gone".to_owned(),
        }));
        // `BriefError::code`: `sim.plan_goals` keeps the brief code.
        assert_tabled(BriefError::Empty.code());
        assert_tabled(BriefError::TooLong { len: 1 }.code());
        // `QueryError::new` sites: `scene.query` keeps the query code.
        for code in [
            "bad_kind",
            "bad_budget",
            "bad_limit",
            "bad_radius",
            "bad_point",
            "unknown_actor",
            "bad_capsule",
            "no_space",
            "bad_gap",
        ] {
            assert_tabled(QueryError::new(code, "probe", "probe").code);
        }
        // `Verdict::as_tool_error`: the dispatch seam keeps `unknown_tool`,
        // and `check_write` below door tier keeps `destructive_tier`.
        let denied = refuse_tool_call("shell.exec").expect("unknown tool denies");
        assert_eq!(denied.code(), "unknown_tool");
        assert_tabled(denied.code());
        let batch = Batch {
            ops: vec![Op::Remove {
                id: ObjectId::new(),
            }],
            base_revision: 0,
            idempotency_key: 1,
            provenance: None,
        };
        let verdict = check_write(
            &Document::new(),
            &batch,
            &Capability::unrestricted(7),
            &ExplainCaps::default_caps(),
            0,
        );
        assert_eq!(verdict.code, "destructive_tier");
        let tiered = verdict.as_tool_error().expect("tier refuses");
        assert_tabled(tiered.code());
    }

    /// Live refusals ride the same round-trip: each refusal below converts
    /// to the wire shape and decodes back from the transport envelope with
    /// code, message and data equal, and its code sits in the table above.
    #[test]
    fn live_refusals_roundtrip_with_data_preserved() {
        fn assert_live(wire: RpcError) {
            assert!(
                TOOL_ERROR_CODES.contains(&wire.code()),
                "code `{}` is tabled",
                wire.code()
            );
            let text = rmcp::serde_json::to_string(&wire).expect("error serializes");
            let back: RpcError = rmcp::serde_json::from_str(&text).expect("error round-trips");
            assert_eq!(back, wire);
            let envelope = rmcp::ErrorData::from(wire.clone());
            assert_eq!(
                RpcError::from_wire(&envelope).expect("door envelopes decode"),
                wire
            );
        }

        let server = test_server();
        // `empty_batch`: a preview with no ops refuses before validation.
        let empty = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops: Vec::new(),
            },
        )
        .expect_err("empty batch refuses");
        assert_eq!(empty.code(), "empty_batch");
        assert_live(RpcError::from(empty));
        // `bad_limit`: a zero page refuses with the legal range attached.
        let over = SceneReadTool::invoke(
            &server,
            SceneReadArgs {
                limit: Some(0),
                cursor: None,
            },
        )
        .expect_err("zero limit refuses");
        assert_eq!(over.code(), "bad_limit");
        assert_live(RpcError::from(over));
        // `stale_revision`: a preview against a moved revision refuses.
        let fresh = OpDto::Insert {
            id: ObjectId::new().to_string(),
            parent: ObjectId::NIL.to_string(),
            local: super::super::mcp::PoseDto {
                pos: [3.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        };
        let stale = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 99,
                idempotency_key: 2,
                ops: vec![fresh],
            },
        )
        .expect_err("stale preview refuses");
        assert_eq!(stale.code(), "stale_revision");
        assert_live(RpcError::from(stale));
        // `denied_op`: an unmapped method refuses with the method as data.
        let denied = map_method("exec").expect_err("exec is denied");
        assert_eq!(
            denied.data,
            Some(rmcp::serde_json::Value::String("exec".to_owned()))
        );
        assert_live(denied);
    }

    /// Offline stage tags never ride the wire: the shared level envelope maps
    /// `BadStage` through `LevelError::BadLine` to the single tabled
    /// `bad_brief` code, so the stage spelling stays out of the table while
    /// the wire code it collapses to stays in. (`BadBudget` shares its
    /// spelling with the tabled log/query `bad_budget`, which rides via
    /// those paths.)
    #[test]
    fn offline_brief_variants_stay_off_the_wire() {
        use super::super::brief::BriefError;
        let stage = BriefError::BadStage {
            stage: "siege".to_owned(),
        }
        .code();
        assert_eq!(stage, "bad_stage");
        assert!(
            !TOOL_ERROR_CODES.contains(&stage),
            "offline code `{stage}` must not ride the wire"
        );
        assert!(
            TOOL_ERROR_CODES.contains(&"bad_brief"),
            "the collapsed `bad_brief` wire code stays tabled"
        );
        assert!(
            TOOL_ERROR_CODES.contains(&BriefError::BadBudget { budget: 9 }.code()),
            "`bad_budget` rides via the log/query paths"
        );
    }

    #[test]
    fn method_map_covers_list_query_get_set_spawn_watch_with_default_deny() {
        for method in ["list", "query", "get", "set", "spawn", "watch"] {
            assert_eq!(
                map_method(method).expect("mapped method passes"),
                PatchKind::StatePatch,
                "every mapped method lands on the single state.patch path"
            );
        }
        for method in ["", "exec", "delete", "shell", "LIST", "patch"] {
            let denied = map_method(method).expect_err("default deny refuses");
            assert_eq!(denied.code(), "denied_op", "method `{method}`");
            assert!(denied.message.contains("state.patch"), "method `{method}`");
        }
    }

    #[test]
    fn batch_preview_returns_per_item_ok_error_with_no_side_effects() {
        let server = test_server();
        let seed = ObjectId::new();
        let seed_ops = vec![OpDto::Insert {
            id: seed.to_string(),
            parent: ObjectId::NIL.to_string(),
            local: super::super::mcp::PoseDto {
                pos: [0.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        }];
        let drafted = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops: seed_ops.clone(),
            },
        )
        .expect("seed preview");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops: seed_ops,
                approval: drafted.approval,
                delta: None,
            },
        )
        .expect("seed apply");
        assert_eq!(revision_of(&server), 1);

        let fresh = || OpDto::Insert {
            id: ObjectId::new().to_string(),
            parent: ObjectId::NIL.to_string(),
            local: super::super::mcp::PoseDto {
                pos: [3.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        };
        let items = vec![
            PreviewItem {
                method: "set".to_owned(),
                expected_revision: 1,
                idempotency_key: 11,
                ops: vec![fresh()],
            },
            PreviewItem {
                method: "exec".to_owned(),
                expected_revision: 1,
                idempotency_key: 12,
                ops: vec![fresh()],
            },
            PreviewItem {
                method: "get".to_owned(),
                expected_revision: 1,
                idempotency_key: 13,
                ops: Vec::new(),
            },
            PreviewItem {
                method: "watch".to_owned(),
                expected_revision: 0,
                idempotency_key: 14,
                ops: vec![fresh()],
            },
        ];
        let results = preview_batch(&server, &items);
        assert_eq!(results.len(), 4);
        for (index, result) in results.iter().enumerate() {
            assert_eq!(result.index, index);
            assert!(
                result.is_shaped(),
                "item {index} sets exactly one of ok/error"
            );
        }
        let ok = results[0].ok.as_ref().expect("valid batch previews");
        assert_eq!(ok.effects.len(), 1);
        assert_eq!(ok.resulting_revision, 2);
        assert_eq!(
            results[1].error.as_ref().expect("denied method").code(),
            "denied_op"
        );
        assert_eq!(
            results[2].error.as_ref().expect("empty batch").code(),
            "empty_batch"
        );
        assert_eq!(
            results[3].error.as_ref().expect("stale revision").code(),
            "stale_revision"
        );
        assert_eq!(revision_of(&server), 1, "batch preview writes nothing");
    }

    #[test]
    fn watch_config_defaults_keep_error_capture_off() {
        let defaults = WatchConfig::default_policy();
        assert!(defaults.coalesce, "bursts coalesce by default");
        assert!(
            defaults.suppress_own,
            "own-session changes suppress by default"
        );
        assert!(!defaults.capture_errors, "runtime-error capture is opt-in");
        assert!(!defaults.include_evidence);
        assert_eq!(WatchConfig::default(), defaults);
    }

    #[test]
    fn watch_subscription_collects_digests_with_zero_polls() {
        let hub = WatchHub::new();
        let sub = hub
            .subscribe(7, WatchConfig::default_policy())
            .expect("subscribe opens");
        assert_eq!(hub.live_count().expect("counts"), 1);
        // Twenty VP1 digests fan out with no poll call.
        let mut session = PlaySession::new(DUEL_SEED, 2, 20.0).expect("duel opens");
        let mut expected = Vec::with_capacity(20);
        for step in 0..20 {
            let base = session.cursor();
            let out = session
                .advance(&step_inputs(base, step as u32), 2, Some(96), Some(64))
                .expect("duel step advances");
            hub.publish(
                WatchKind::Play,
                u64::from(out.tick),
                out.digest,
                "step",
                None,
            )
            .expect("publish fans out");
            expected.push(out.digest);
        }
        assert_eq!(sub.polls().expect("poll counter reads"), 0, "zero polls");
        let digests: Vec<u64> = sub
            .delivered()
            .expect("delivered reads")
            .iter()
            .map(|event| event.digest)
            .collect();
        assert_eq!(digests, expected, "watch sees every step digest");
        // Default policy coalesces the undrained burst: one queued event,
        // nineteen collapses, all twenty in the delivered log.
        assert_eq!(sub.queue_len().expect("queue reads"), 1, "burst coalesces");
        assert_eq!(sub.coalesced().expect("coalesced reads"), 19);
    }

    /// MCP7 acceptance: a 20-step VP1 session with 0 polls gives the same
    /// digests as polling, twice. The polling path drains per step; the
    /// watch path never polls and reads the delivered log instead.
    #[test]
    fn twenty_step_session_watch_matches_polling_twice() {
        let run_polling = || {
            let hub = WatchHub::new();
            let sub = hub
                .subscribe(7, WatchConfig::default_policy())
                .expect("subscribe");
            let mut session = PlaySession::new(DUEL_SEED, 2, 20.0).expect("duel opens");
            let mut digests = Vec::with_capacity(20);
            for step in 0..20 {
                let base = session.cursor();
                let out = session
                    .advance(&step_inputs(base, step as u32), 2, Some(96), Some(64))
                    .expect("duel step advances");
                hub.publish(
                    WatchKind::Play,
                    u64::from(out.tick),
                    out.digest,
                    "step",
                    None,
                )
                .expect("publish fans out");
                let polled = sub.poll().expect("poll drains one");
                assert_eq!(polled.len(), 1, "one digest per step");
                digests.push(polled[0].digest);
            }
            assert_eq!(sub.polls().expect("polls read"), 20);
            digests
        };
        let run_watching = || {
            let hub = WatchHub::new();
            let sub = hub
                .subscribe(7, WatchConfig::default_policy())
                .expect("subscribe");
            let mut session = PlaySession::new(DUEL_SEED, 2, 20.0).expect("duel opens");
            for step in 0..20 {
                let base = session.cursor();
                let out = session
                    .advance(&step_inputs(base, step as u32), 2, Some(96), Some(64))
                    .expect("duel step advances");
                hub.publish(
                    WatchKind::Play,
                    u64::from(out.tick),
                    out.digest,
                    "step",
                    None,
                )
                .expect("publish fans out");
            }
            assert_eq!(sub.polls().expect("polls read"), 0, "zero polls");
            sub.delivered()
                .expect("delivered reads")
                .iter()
                .map(|event| event.digest)
                .collect::<Vec<_>>()
        };
        let polling_first = run_polling();
        let watching_first = run_watching();
        assert_eq!(polling_first, watching_first, "watch matches polling");
        assert_ne!(polling_first[0], polling_first[19], "the duel moves");
        // Twice: the same inputs give the same digests on a fresh run.
        assert_eq!(run_polling(), polling_first, "polling repeats");
        assert_eq!(run_watching(), watching_first, "watch repeats");
    }

    #[test]
    fn watch_scene_digests_track_brief_polling() {
        let server = test_server();
        let hub = WatchHub::new();
        let sub = hub
            .subscribe(7, WatchConfig::default_policy())
            .expect("subscribe");
        let brief_digest = || {
            SceneBriefTool::invoke(&server, SceneBriefArgs::default())
                .expect("brief reads")
                .digest
        };
        let mut expected = Vec::new();
        for key in 100..105u64 {
            let id = ObjectId::new();
            let ops = vec![OpDto::Insert {
                id: id.to_string(),
                parent: ObjectId::NIL.to_string(),
                local: super::super::mcp::PoseDto {
                    pos: [key as f32, 0.0, 0.0],
                    rot: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            }];
            let rev = revision_of(&server);
            let drafted = TxPreviewTool::invoke(
                &server,
                TxPreviewArgs {
                    expected_revision: rev,
                    idempotency_key: key,
                    ops: ops.clone(),
                },
            )
            .expect("insert previews");
            TxApplyTool::invoke(
                &server,
                TxApplyArgs {
                    expected_revision: rev,
                    idempotency_key: key,
                    ops,
                    approval: drafted.approval,
                    delta: None,
                },
            )
            .expect("insert applies");
            let digest = brief_digest();
            hub.publish(WatchKind::Scene, rev + 1, digest, "insert", Some(7))
                .expect("publish fans out");
            expected.push(digest);
        }
        // Own-session events suppress by default: nothing was delivered.
        assert!(
            sub.delivered().expect("delivered reads").is_empty(),
            "own-session changes suppress"
        );
        // Another session's origin delivers the same digests polling sees.
        for (index, digest) in expected.iter().enumerate() {
            hub.publish(
                WatchKind::Scene,
                100 + index as u64,
                *digest,
                "peer insert",
                Some(9),
            )
            .expect("peer publish fans out");
        }
        let digests: Vec<u64> = sub
            .delivered()
            .expect("delivered reads")
            .iter()
            .map(|event| event.digest)
            .collect();
        assert_eq!(digests, expected, "peer watch matches brief polling");
    }

    #[test]
    fn watch_bursts_coalesce_suppression_and_drop_kill_delivery() {
        let hub = WatchHub::new();
        // Bursts coalesce to the latest digest when the queue is undrained.
        let bursting = hub
            .subscribe(7, WatchConfig::default_policy())
            .expect("burst subscribe");
        for digest in [11u64, 22, 33, 44, 55] {
            hub.publish(WatchKind::Scene, 1, digest, "burst", Some(9))
                .expect("burst publishes");
        }
        assert_eq!(
            bursting.queue_len().expect("queue reads"),
            1,
            "burst collapses"
        );
        assert_eq!(bursting.coalesced().expect("coalesced reads"), 4);
        let drained = bursting.poll().expect("drain reads the burst");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].digest, 55, "coalescing keeps the latest digest");
        // Without coalescing every burst event queues.
        let raw = hub
            .subscribe(
                7,
                WatchConfig {
                    coalesce: false,
                    ..WatchConfig::default_policy()
                },
            )
            .expect("raw subscribe");
        for digest in [11u64, 22, 33] {
            hub.publish(WatchKind::Scene, 1, digest, "burst", Some(9))
                .expect("raw publishes");
        }
        assert_eq!(raw.queue_len().expect("queue reads"), 3);
        assert_eq!(raw.coalesced().expect("coalesced reads"), 0);
        // Full queues drop the oldest and count it.
        for digest in 0..(MAX_WATCH_QUEUE as u64 + 4) {
            hub.publish(WatchKind::Scene, 2, digest, "flood", Some(9))
                .expect("flood publishes");
        }
        assert_eq!(raw.queue_len().expect("queue reads"), MAX_WATCH_QUEUE);
        assert!(
            raw.overflow().expect("overflow reads") > 0,
            "flood overflows"
        );
        // Runtime errors record only on opt-in slots.
        assert_eq!(bursting.error_count().expect("errors read"), 0);
        hub.publish_error("sim_panic", "tick blew up")
            .expect("error publishes");
        assert_eq!(
            bursting.error_count().expect("errors read"),
            0,
            "default off"
        );
        assert!(bursting.last_error().expect("last error reads").is_none());
        let watching_errors = hub
            .subscribe(
                7,
                WatchConfig {
                    capture_errors: true,
                    ..WatchConfig::default_policy()
                },
            )
            .expect("opt-in subscribe");
        hub.publish_error("sim_panic", "tick blew up")
            .expect("error publishes");
        assert_eq!(watching_errors.error_count().expect("errors read"), 1);
        let last = watching_errors
            .last_error()
            .expect("last error reads")
            .expect("recorded");
        assert_eq!(last.code(), "sim_panic");
        // Evidence blobs attach bounded.
        watching_errors
            .attach_evidence(vec![1, 2, 3])
            .expect("evidence attaches");
        assert_eq!(watching_errors.evidence_len().expect("evidence reads"), 3);
        let too_big = watching_errors
            .attach_evidence(vec![0u8; MAX_WATCH_EVIDENCE_BYTES + 1])
            .expect_err("oversize evidence refuses");
        assert_eq!(too_big.code(), "bad_evidence");
        assert_eq!(watching_errors.evidence_len().expect("evidence reads"), 3);
        // Drop kills delivery: the slot prunes and publishes skip it.
        assert_eq!(hub.live_count().expect("counts"), 3);
        drop(bursting);
        assert_eq!(hub.prune().expect("prunes"), 1, "one dead slot prunes");
        assert_eq!(hub.live_count().expect("counts"), 2);
        hub.publish(WatchKind::Scene, 3, 99, "after drop", Some(9))
            .expect("publish skips the dead slot");
        drop(raw);
        drop(watching_errors);
        assert_eq!(hub.prune().expect("prunes"), 2, "both dead slots prune");
        assert_eq!(hub.live_count().expect("counts"), 0);
    }

    #[test]
    fn watch_subscribe_use_unsubscribe_flow_stays_live_until_close() {
        let hub = WatchHub::new();
        // Subscribe: the slot is live.
        let sub = hub
            .subscribe(7, WatchConfig::default_policy())
            .expect("subscribe opens");
        let id = sub.id();
        assert_ne!(id, 0, "subscription ids start at 1");
        assert_eq!(hub.live_count().expect("counts"), 1);
        // Use: a peer publish lands on this subscription (own-session
        // origins suppress, so the origin is another session).
        hub.publish(WatchKind::Scene, 1, 0xBEEF, "peer insert", Some(9))
            .expect("publish fans out");
        let digests: Vec<u64> = sub
            .delivered()
            .expect("delivered reads")
            .iter()
            .map(|event| event.digest)
            .collect();
        assert_eq!(digests, vec![0xBEEF], "use sees the publish");
        // Unsubscribe: the slot is gone at once, later uses refuse it.
        hub.unsubscribe(id).expect("unsubscribe closes");
        assert_eq!(hub.live_count().expect("counts"), 0);
        assert_eq!(
            sub.delivered().expect_err("use after close refuses").code(),
            "unknown_subscription"
        );
        assert_eq!(
            hub.unsubscribe(id)
                .expect_err("second close refuses")
                .code(),
            "unknown_subscription"
        );
    }

    #[test]
    fn watch_tool_opens_readonly_with_live_baseline() {
        let server = test_server();
        let annotations = WatchTool::annotations().expect("annotations ride along");
        assert_eq!(annotations.read_only_hint, Some(true));
        let out = WatchTool::invoke(
            &server,
            WatchSubscribeArgs {
                session: 7,
                coalesce: None,
                suppress_own: None,
                capture_errors: None,
            },
        )
        .expect("watch subscribes");
        assert_ne!(out.subscription_id, 0);
        assert_eq!(out.session, 7);
        let baseline =
            SceneBriefTool::invoke(&server, SceneBriefArgs::default()).expect("brief reads");
        assert_eq!(out.revision, baseline.revision);
        assert_eq!(out.digest, baseline.digest);
        // The returned id is live: the tool adopts the handle (no
        // drop+prune), so a peer publish reaches it until unsubscribe.
        let hub = global_hub();
        let deliveries = hub
            .publish(WatchKind::Scene, out.revision, out.digest, "peer", Some(9))
            .expect("publish fans out");
        assert!(
            deliveries >= 1,
            "the tool subscription receives until unsubscribe"
        );
        hub.unsubscribe(out.subscription_id)
            .expect("unsubscribe closes the tool subscription");
    }

    #[test]
    fn watch_brief_cache_tracks_scene_edits_and_stays_live() {
        let server = test_server();
        let out = WatchTool::invoke(
            &server,
            WatchSubscribeArgs {
                session: 7,
                coalesce: None,
                suppress_own: None,
                capture_errors: None,
            },
        )
        .expect("watch subscribes");
        let before =
            SceneBriefTool::invoke(&server, SceneBriefArgs::default()).expect("brief reads");
        assert_eq!(before.watch_subscription, Some(out.subscription_id));
        assert_eq!(before.watch_digest, Some(out.digest));

        // One scene edit through the command gate.
        let id = ObjectId::new();
        let ops = vec![OpDto::Insert {
            id: id.to_string(),
            parent: ObjectId::NIL.to_string(),
            local: super::super::mcp::PoseDto {
                pos: [3.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        }];
        let drafted = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: before.revision,
                idempotency_key: 6101,
                ops: ops.clone(),
            },
        )
        .expect("insert previews");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: before.revision,
                idempotency_key: 6101,
                ops,
                approval: drafted.approval,
                delta: None,
            },
        )
        .expect("insert applies");

        // The brief serves the fresh digest from the session-local cache, and
        // the id is still live on a later call.
        let after =
            SceneBriefTool::invoke(&server, SceneBriefArgs::default()).expect("brief reads");
        assert_eq!(after.revision, before.revision + 1);
        assert_ne!(after.digest, before.digest);
        assert_eq!(after.watch_digest, Some(after.digest));
        assert_eq!(after.watch_subscription, Some(out.subscription_id));
        let deliveries = global_hub()
            .publish(
                WatchKind::Scene,
                after.revision,
                after.digest,
                "peer",
                Some(9),
            )
            .expect("publish fans out");
        assert!(
            deliveries >= 1,
            "the tool subscription stays live across the edit"
        );

        // A second watch replaces the first: the old id is gone at once.
        let second = WatchTool::invoke(
            &server,
            WatchSubscribeArgs {
                session: 7,
                coalesce: None,
                suppress_own: None,
                capture_errors: None,
            },
        )
        .expect("second watch subscribes");
        assert_ne!(second.subscription_id, out.subscription_id);
        assert_eq!(
            global_hub()
                .unsubscribe(out.subscription_id)
                .expect_err("replaced watch is gone")
                .code(),
            "unknown_subscription"
        );
        global_hub()
            .unsubscribe(second.subscription_id)
            .expect("second watch closes");
    }

    fn insert_item(method: &str, revision: u64, key: u64) -> PreviewItem {
        PreviewItem {
            method: method.to_owned(),
            expected_revision: revision,
            idempotency_key: key,
            ops: vec![OpDto::Insert {
                id: ObjectId::new().to_string(),
                parent: ObjectId::NIL.to_string(),
                local: super::super::mcp::PoseDto {
                    pos: [key as f32, 0.0, 0.0],
                    rot: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            }],
        }
    }

    #[test]
    fn fuzzy_suggestions_name_the_close_tool_and_method() {
        assert_eq!(suggest_method("lst"), Some("list"));
        assert_eq!(suggest_method("sett"), Some("set"));
        assert_eq!(suggest_method("quary"), Some("query"));
        assert_eq!(suggest_method("exec"), None);
        assert_eq!(suggest_method(""), None);
        assert_eq!(suggest_tool("scene.red"), Some("scene.read"));
        assert_eq!(suggest_tool("tx.prevew"), Some("tx.preview"));
        assert_eq!(suggest_tool("zzz-no-such-tool"), None);
        // Denials carry the suggestion text for weak models.
        let denied = map_method("lstt").expect_err("typo denies");
        assert_eq!(denied.code(), "denied_op");
        assert!(
            denied.message.contains("did you mean `list`"),
            "got: {}",
            denied.message
        );
        let unknown = resolve_tool("scene.red").expect_err("typo refuses");
        assert_eq!(unknown.code(), "unknown_tool");
        assert!(
            unknown.message.contains("scene.read"),
            "got: {}",
            unknown.message
        );
        assert_eq!(
            resolve_tool("scene.read").expect("exact resolves"),
            "scene.read"
        );
        // The manage op-enum round-trips its six names.
        for name in ManageOp::all_names() {
            assert_eq!(ManageOp::parse(name).expect("managed parses").name(), name);
        }
        assert_eq!(
            ManageOp::parse("exec").expect_err("exec denies").code(),
            "denied_op"
        );
    }

    #[test]
    fn dispatch_manage_previews_without_side_effects() {
        let server = test_server();
        let item = insert_item("set", 0, 41);
        let ok = dispatch_manage(
            &server,
            &item.method,
            item.expected_revision,
            item.idempotency_key,
            item.ops.clone(),
        )
        .expect("managed set previews");
        assert_eq!(ok.effects.len(), 1);
        assert_eq!(ok.resulting_revision, 1);
        assert_eq!(revision_of(&server), 0, "dispatch_manage writes nothing");
        let denied =
            dispatch_manage(&server, "exec", 0, 42, item.ops).expect_err("unmapped denies");
        assert_eq!(denied.code(), "denied_op");
        assert_eq!(revision_of(&server), 0, "denials write nothing");
    }

    #[test]
    fn batch_execute_lands_atomically_and_replays_identically() {
        let run_once = || {
            let server = test_server();
            let items = vec![insert_item("set", 0, 51), insert_item("spawn", 0, 52)];
            let out = batch_execute(&server, &items);
            assert_eq!(out.applied, 2, "both items land");
            assert_eq!(out.revision, 2);
            assert_eq!(out.results.len(), 2);
            for result in &out.results {
                assert!(result.is_shaped());
                assert!(result.error.is_none(), "both preview clean");
            }
            let text = rmcp::serde_json::to_string(&out.results).expect("results serialize");
            (text, out.revision)
        };
        let (first_text, first_rev) = run_once();
        let (second_text, second_rev) = run_once();
        // Same shape on fresh loopback servers replays identically: the
        // revision matches and each item previews one effect (ids and hashes
        // differ per run, so shape compares, not bytes, across runs).
        assert_eq!(first_rev, second_rev);
        let first_back: Vec<PreviewResult> =
            rmcp::serde_json::from_str(&first_text).expect("results decode");
        let second_back: Vec<PreviewResult> =
            rmcp::serde_json::from_str(&second_text).expect("results decode");
        assert_eq!(first_back.len(), second_back.len());
        for (first, second) in first_back.iter().zip(second_back.iter()) {
            assert_eq!(first.error.is_some(), second.error.is_some());
            assert_eq!(
                first.ok.as_ref().map(|ok| ok.effects.len()),
                second.ok.as_ref().map(|ok| ok.effects.len())
            );
        }
        // One live list/preview/apply/undo cycle replays its own bytes x2.
        let server = test_server();
        let items = vec![insert_item("get", 0, 61), insert_item("watch", 0, 62)];
        let out = batch_execute(&server, &items);
        assert_eq!(out.applied, 2);
        let bytes = rmcp::serde_json::to_vec(&out.results).expect("results encode");
        let back: Vec<PreviewResult> =
            rmcp::serde_json::from_slice(&bytes).expect("results decode");
        assert_eq!(back, out.results, "replay x2 is byte-identical");
        let bytes_again = rmcp::serde_json::to_vec(&back).expect("results re-encode");
        assert_eq!(
            bytes, bytes_again,
            "second replay matches the first byte for byte"
        );
        // The hint stays chars-down next to the full manifest.
        let manifest = DiscoverManifest::from_server(&server, false);
        assert_eq!(
            manifest.tool_count(),
            server.tools().len(),
            "the manifest follows the advertised door"
        );
        assert!(
            rollup_help_chars() < manifest.manifest_chars(),
            "hint {} chars vs manifest {} chars",
            rollup_help_chars(),
            manifest.manifest_chars()
        );
    }

    #[test]
    fn batch_execute_rolls_back_on_failure_and_writes_nothing() {
        let server = test_server();
        // Second item reuses the first id, so its chained apply must fail
        // after the first lands; the rollback undoes the first (undo appends
        // a new revision, so the content matches base at a moved revision).
        let id = ObjectId::new().to_string();
        let mk = |method: &str, id: &str, key: u64, x: f32| PreviewItem {
            method: method.to_owned(),
            expected_revision: 0,
            idempotency_key: key,
            ops: vec![OpDto::Insert {
                id: id.to_owned(),
                parent: ObjectId::NIL.to_string(),
                local: super::super::mcp::PoseDto {
                    pos: [x, 0.0, 0.0],
                    rot: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            }],
        };
        let items = vec![mk("set", &id, 71, 2.0), mk("set", &id, 72, 9.0)];
        let out = batch_execute(&server, &items);
        assert_eq!(out.applied, 0, "rollback lands nothing");
        assert_eq!(out.results.len(), 2);
        assert!(
            out.results[0].ok.is_some(),
            "first chained apply landed then undid"
        );
        assert!(out.results[1].error.is_some(), "duplicate id refuses");
        let page = SceneReadTool::invoke(&server, SceneReadArgs::default()).expect("reads");
        assert!(
            page.objects.is_empty(),
            "content matches base after rollback"
        );
        // A denied method up front also lands nothing.
        let mixed = vec![insert_item("set", 0, 73), insert_item("exec", 0, 74)];
        let refused = batch_execute(&server, &mixed);
        assert_eq!(refused.applied, 0);
        assert!(refused.results[1].error.is_some());
        let page = SceneReadTool::invoke(&server, SceneReadArgs::default()).expect("reads");
        assert!(page.objects.is_empty(), "denied batch writes nothing");
    }

    #[test]
    fn batch_preview_remote_shim_coalesced_drag_is_one_undo() {
        use super::super::remote_shim::{preview_via_shim, BrpRequest};
        use engine_editor::commands as cmds;
        let server = test_server();
        let node = ObjectId::new();
        let seed = vec![OpDto::Insert {
            id: node.to_string(),
            parent: ObjectId::NIL.to_string(),
            local: super::super::mcp::PoseDto {
                pos: [0.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        }];
        let rev = revision_of(&server);
        let draft = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: rev,
                idempotency_key: 9001,
                ops: seed.clone(),
            },
        )
        .expect("seed preview");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: rev,
                idempotency_key: 9001,
                ops: seed,
                approval: draft.approval,
                delta: None,
            },
        )
        .expect("seed apply");
        let base = revision_of(&server);
        let pre = server.lock().expect("lock").doc.hierarchy.clone();
        let base_past = server.lock().expect("lock").journal.past_len();
        let mut batch = Vec::new();
        for i in 0..50u64 {
            let x = 0.02 * (i + 1) as f32;
            batch.push(BrpRequest {
                method: "set".to_owned(),
                expected_revision: base,
                idempotency_key: 9100 + i,
                ops: vec![OpDto::SetLocal {
                    id: node.to_string(),
                    local: super::super::mcp::PoseDto {
                        pos: [x, 0.0, 0.0],
                        rot: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0, 1.0, 1.0],
                    },
                }],
            });
        }
        let dry = preview_via_shim(&server, &batch);
        assert!(dry.len() == 50 && dry.iter().all(|r| r.ok.is_some()));
        assert_eq!(revision_of(&server), base);
        let out = server.brp_batch(&batch, 9999);
        assert_eq!(out.applied, 50);
        let past = server.lock().expect("lock").journal.past_len();
        assert_eq!(past, base_past + 1);
        let landed = server.lock().expect("lock").doc.hierarchy.clone();
        let x = landed.local(node).expect("node").position[0];
        assert!((x - 1.0).abs() <= 1e-3, "folded value is +1.00m, got {x}");
        TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
        assert_eq!(server.lock().expect("lock").doc.hierarchy, pre);
        let mut s = server.lock().expect("lock");
        let state = &mut *s;
        cmds::redo(&mut state.doc, &mut state.journal).expect("redo");
        drop(s);
        assert_eq!(server.lock().expect("lock").doc.hierarchy, landed);
    }
}
