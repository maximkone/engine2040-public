//! Door v2 verb packs: a small core (`api.call`, `api.load`) stays advertised, thirteen packs load on
//! demand in contract order (`scene`, `break_`, `anim`, `material`, `asset`, `rules`, `geo`, `gym`, `view`, `net`, `perf`, `play`, `replay` in [`PACK_NAMES`]), and every verb registers through one typed API.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) lists its core tools plus
//! the packs a session loaded (`api.load`), dispatches any verb by exact
//! name (`tools/call` or `api.call`) and seeds the Insight index so
//! `api.search` finds packs and verbs. Data owner stays the editor command
//! service; this module owns only the registry: names, capability cards,
//! advertised schemas and the typed dispatch thunks. Units: chars for
//! discovery cost, counts for verbs. Determinism: packs and verbs sit in
//! contract order ([`PACK_NAMES`]) and lookups go through a `BTreeMap`, so
//! the same build always lists the same tools in the same order. Errors:
//! [`PackError`] at registration (a verb without an annotation or a
//! capability card never registers), [`McpToolError`] at dispatch
//! (`unknown_verb`, `unknown_pack`, `bad_args`), every message naming the
//! legal values plus one recovery. Threading: the registry is immutable
//! after its one-time build; the loaded set lives on the session state
//! behind the door lock.
//!
//! Features: `mcp` opt-in; `geo`, `intake`, `rules` off by default so the
//! default build carries no donor libs or scripting engine. Budget: tool-list
//! [`TOOL_LIST_BUDGET_CHARS`] (43_514 chars). Entry points: [`PACK_NAMES`],
//! [`PackRegistry::shipped`], `pack`/`verb` lookups.
//!
//! Adding a verb (any pack owner): write a `SyncTool<McpServer>` (the
//! `e2_tool!` macro or a manual `ToolBase` impl) in your pack module, then
//! add one `pack.add(VerbSpec::of::<YourTool>(NAME, PACK, card)?)` line to
//! that module's `pack()`. Nothing in this file or in `mcp.rs` changes; the
//! verb is reachable by name at once and listed after `api.load`.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, OnceLock};

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::{JsonObject, Tool};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::insight::Index;
use super::mcp::{McpServer, McpToolError, ServerState, TOOL_LIST_BUDGET_CHARS};
use super::tool_capabilities::CapabilityCard;

pub mod anim;
pub mod asset;
pub mod break_;
pub mod geo;
pub mod gym;
pub mod material;
pub mod net;
pub mod perf;
pub mod play;
pub mod replay;
pub mod rules;
pub mod scene;
pub mod view;

/// Every pack name, in contract order. Each name has one module below.
pub const PACK_NAMES: [&str; 13] = [
    scene::PACK,
    break_::PACK,
    anim::PACK,
    material::PACK,
    asset::PACK,
    rules::PACK,
    geo::PACK,
    gym::PACK,
    view::PACK,
    net::PACK,
    perf::PACK,
    play::PACK,
    replay::PACK,
];

/// Core tool: call one verb from any pack by exact name.
pub const TOOL_API_CALL: &str = "api.call";
/// Core tool: load (advertise) or unload one verb pack for this session.
pub const TOOL_API_LOAD: &str = "api.load";

/// Words a verb name may never contain: the door has no arbitrary
/// execute, shell or file surface, by name as well as by construction.
const FORBIDDEN_NAME_WORDS: [&str; 5] = ["exec", "shell", "file", "eval", "terminal"];

/// Why a verb or pack refused to register. Every variant names the verb so
/// the pack owner can quote it; the guard test drives each one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// The tool advertises no `ToolAnnotations` or no `read_only_hint`:
    /// agents could not tell a reader from a writer.
    MissingAnnotation {
        /// The verb that lacks its annotation.
        verb: &'static str,
    },
    /// The verb was registered under the `Deny` card: a verb that denies
    /// itself is never advertised, so the registration is a mistake.
    MissingCapability {
        /// The verb registered without a usable capability card.
        verb: &'static str,
    },
    /// The tool advertises no input schema: a missing schema must refuse at
    /// registration, never fall back to a permissive shape.
    MissingSchema {
        /// The verb that lacks its input schema.
        verb: &'static str,
    },
    /// The registered name differs from the tool's own `ToolBase::name`.
    NameMismatch {
        /// The name passed at registration.
        verb: &'static str,
    },
    /// The name is empty, has no `pack.verb` dot, or carries a forbidden
    /// word (`exec`, `shell`, `file`, `eval`, `terminal`).
    BadName {
        /// The refused name.
        verb: &'static str,
    },
    /// Two packs (or one pack twice) registered the same verb name.
    DuplicateVerb {
        /// The repeated name.
        verb: &'static str,
    },
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAnnotation { verb } => write!(
                f,
                "verb `{verb}` has no read-only annotation; set `read_only_hint` in its annotations"
            ),
            Self::MissingCapability { verb } => write!(
                f,
                "verb `{verb}` has no capability card; register it as observe, propose, commit or revert"
            ),
            Self::MissingSchema { verb } => write!(
                f,
                "verb `{verb}` has no input schema; a verb without a schema never registers"
            ),
            Self::NameMismatch { verb } => write!(
                f,
                "verb `{verb}` was registered under a name its tool does not carry"
            ),
            Self::BadName { verb } => write!(
                f,
                "verb name `{verb}` is not `pack.verb` or carries a forbidden word \
                 ({})",
                FORBIDDEN_NAME_WORDS.join(", ")
            ),
            Self::DuplicateVerb { verb } => {
                write!(f, "verb `{verb}` is registered twice; names are unique across packs")
            }
        }
    }
}

impl std::error::Error for PackError {}

/// Dispatch thunk type: typed args in as JSON, typed output out as JSON.
type InvokeFn = fn(&McpServer, Value) -> Result<Value, McpToolError>;

/// One registered verb: its pack, capability card, read-only flag, the
/// advertised tool model and the typed dispatch thunk.
pub struct VerbSpec {
    name: &'static str,
    pack: &'static str,
    card: CapabilityCard,
    readonly: bool,
    description: Cow<'static, str>,
    model: fn() -> Tool,
    invoke: InvokeFn,
}

impl fmt::Debug for VerbSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerbSpec")
            .field("name", &self.name)
            .field("pack", &self.pack)
            .field("card", &self.card)
            .field("readonly", &self.readonly)
            .finish_non_exhaustive()
    }
}

impl VerbSpec {
    /// Register one `SyncTool` as a pack verb. The guard refuses a tool
    /// without annotations or a read-only hint, a `Deny` card, a tool with
    /// no input schema, a name the tool does not carry, and a name outside
    /// `pack.verb` or with a forbidden word.
    ///
    /// ```
    /// use engine_endpoint::packs::{PackError, VerbSpec};
    /// use engine_endpoint::tool_capabilities::CapabilityCard;
    /// use engine_endpoint::scene_tools::{SceneBriefTool, TOOL_SCENE_BRIEF};
    ///
    /// let verb = VerbSpec::of::<SceneBriefTool>(TOOL_SCENE_BRIEF, "scene", CapabilityCard::Observe)
    ///     .expect("brief registers");
    /// assert_eq!(verb.name(), "scene.brief");
    /// assert!(verb.readonly());
    /// let denied = VerbSpec::of::<SceneBriefTool>(TOOL_SCENE_BRIEF, "scene", CapabilityCard::Deny);
    /// assert_eq!(denied.err(), Some(PackError::MissingCapability { verb: "scene.brief" }));
    /// ```
    pub fn of<T>(
        name: &'static str,
        pack: &'static str,
        card: CapabilityCard,
    ) -> Result<Self, PackError>
    where
        T: SyncTool<McpServer, Error = McpToolError> + 'static,
    {
        if T::name().as_ref() != name {
            return Err(PackError::NameMismatch { verb: name });
        }
        if !name_is_sane(name) {
            return Err(PackError::BadName { verb: name });
        }
        let readonly = T::annotations()
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            .ok_or(PackError::MissingAnnotation { verb: name })?;
        if card == CapabilityCard::Deny {
            return Err(PackError::MissingCapability { verb: name });
        }
        if T::input_schema().is_none() {
            return Err(PackError::MissingSchema { verb: name });
        }
        Ok(Self {
            name,
            pack,
            card,
            readonly,
            description: T::description().unwrap_or(Cow::Borrowed("")),
            model: model_of::<T>,
            invoke: invoke_json::<T>,
        })
    }

    /// Exact verb name (`scene.brief`).
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Pack this verb belongs to.
    pub fn pack(&self) -> &'static str {
        self.pack
    }

    /// Capability card the door gates this verb under.
    pub fn card(&self) -> CapabilityCard {
        self.card
    }

    /// True when the tool promises no side effects.
    pub fn readonly(&self) -> bool {
        self.readonly
    }

    /// One-line description, as advertised.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The advertised tool model: name, description, schemas, annotations.
    pub fn model(&self) -> Tool {
        (self.model)()
    }

    /// Dispatch this verb with JSON args; the verb's own tool path runs
    /// (door lock, capability gate, audit record), so `api.call` and a
    /// direct `tools/call` land identically.
    pub fn invoke(&self, server: &McpServer, args: Value) -> Result<Value, McpToolError> {
        (self.invoke)(server, args)
    }
}

/// `pack.verb` shape with no forbidden word.
fn name_is_sane(name: &str) -> bool {
    let (head, tail) = match name.split_once('.') {
        Some(parts) => parts,
        None => return false,
    };
    if head.is_empty() || tail.is_empty() {
        return false;
    }
    let lower = name.to_lowercase();
    !FORBIDDEN_NAME_WORDS.iter().any(|word| lower.contains(word))
}

/// Closed empty-object schema for the fail-closed path: accepts only a JSON
/// object with no declared properties, matching the router's own empty-input
/// shape. A bare empty map would mean "any value" in JSON Schema.
fn closed_empty_schema() -> Arc<JsonObject> {
    let mut object = JsonObject::new();
    object.insert("type".into(), Value::String("object".into()));
    object.insert("properties".into(), Value::Object(JsonObject::new()));
    Arc::new(object)
}

/// The advertised model of one tool type, built the way the router builds
/// its own rows (name, title, description, both schemas, annotations).
/// Fail-closed: a missing input schema advertises the closed empty-object
/// shape (`type: object`, no properties), never a permissive empty map.
/// Registration refuses schema-less tools outright (see `VerbSpec::of`),
/// so this fallback is defense in depth only.
fn model_of<T: ToolBase>() -> Tool {
    let input = T::input_schema().unwrap_or_else(closed_empty_schema);
    let mut tool = Tool::new_with_raw(T::name(), T::description(), input);
    tool.title = T::title();
    tool.output_schema = T::output_schema();
    tool.annotations = T::annotations();
    tool.icons = T::icons();
    tool.meta = T::meta();
    tool
}

/// Typed dispatch through JSON: args must fit the tool's input schema, the
/// output serializes through its output schema.
fn invoke_json<T>(server: &McpServer, args: Value) -> Result<Value, McpToolError>
where
    T: SyncTool<McpServer, Error = McpToolError>,
{
    let name = T::name();
    let param: T::Parameter = rmcp::serde_json::from_value(args).map_err(|error| {
        McpToolError::new(
            "bad_args",
            format!(
                "args for `{name}` do not fit its input schema ({error}); \
                 re-run after api.describe `{name}` for the schema"
            ),
        )
    })?;
    let output = T::invoke(server, param)?;
    rmcp::serde_json::to_value(output).map_err(|error| {
        McpToolError::new(
            "bad_output",
            format!(
                "`{name}` produced output outside its schema ({error}); re-run and report the verb"
            ),
        )
    })
}

/// One verb pack: a name, a one-line summary, whether a fresh session
/// advertises it, and its verbs in registration order.
#[derive(Debug)]
pub struct PackDef {
    name: &'static str,
    summary: &'static str,
    default_loaded: bool,
    verbs: Vec<VerbSpec>,
}

impl PackDef {
    /// Empty pack with its summary; `default_loaded` packs are advertised
    /// before any `api.load`.
    pub fn new(name: &'static str, summary: &'static str, default_loaded: bool) -> Self {
        Self {
            name,
            summary,
            default_loaded,
            verbs: Vec::new(),
        }
    }

    /// Add one registered verb (duplicates refuse at registry build).
    pub fn add(&mut self, verb: VerbSpec) {
        self.verbs.push(verb);
    }

    /// Pack name (`scene`, `break`, ...).
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// One-line summary for `api.search` and `api.describe`.
    pub fn summary(&self) -> &'static str {
        self.summary
    }

    /// True when a fresh session lists this pack's verbs.
    pub fn default_loaded(&self) -> bool {
        self.default_loaded
    }

    /// Verbs in registration order.
    pub fn verbs(&self) -> &[VerbSpec] {
        &self.verbs
    }

    /// Verb names in registration order.
    pub fn verb_names(&self) -> Vec<&'static str> {
        self.verbs.iter().map(VerbSpec::name).collect()
    }
}

/// Every pack with every verb, plus any registration errors. Built once
/// per process ([`PackRegistry::shipped`]); never mutated afterwards.
#[derive(Debug, Default)]
pub struct PackRegistry {
    packs: Vec<PackDef>,
    by_verb: BTreeMap<&'static str, (usize, usize)>,
    errors: Vec<PackError>,
}

impl PackRegistry {
    /// Build a registry from pack results in contract order. A failed pack
    /// records its error and contributes nothing; a duplicate verb name
    /// records [`PackError::DuplicateVerb`] and keeps the first owner.
    pub fn build(packs: Vec<Result<PackDef, PackError>>) -> Self {
        let mut registry = Self::default();
        for pack in packs {
            match pack {
                Ok(pack) => registry.push_pack(pack),
                Err(error) => registry.errors.push(error),
            }
        }
        registry
    }

    fn push_pack(&mut self, pack: PackDef) {
        let pack_index = self.packs.len();
        for (verb_index, verb) in pack.verbs.iter().enumerate() {
            if self.by_verb.contains_key(verb.name) {
                self.errors
                    .push(PackError::DuplicateVerb { verb: verb.name });
                continue;
            }
            self.by_verb.insert(verb.name, (pack_index, verb_index));
        }
        self.packs.push(pack);
    }

    /// The shipped registry: the thirteen packs of [`PACK_NAMES`], built once.
    /// Registration errors never panic here; the guard test asserts the
    /// shipped list is clean.
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<PackRegistry> = OnceLock::new();
        SHIPPED.get_or_init(|| {
            Self::build(vec![
                scene::pack(),
                break_::pack(),
                anim::pack(),
                material::pack(),
                asset::pack(),
                rules::pack(),
                geo::pack(),
                gym::pack(),
                view::pack(),
                net::pack(),
                perf::pack(),
                play::pack(),
                replay::pack(),
            ])
        })
    }

    /// Registration errors recorded at build (empty for a clean registry).
    pub fn errors(&self) -> &[PackError] {
        &self.errors
    }

    /// Packs in contract order.
    pub fn packs(&self) -> &[PackDef] {
        &self.packs
    }

    /// One pack by name.
    pub fn pack(&self, name: &str) -> Option<&PackDef> {
        self.packs.iter().find(|pack| pack.name == name)
    }

    /// One verb by exact name, whatever its pack.
    pub fn verb(&self, name: &str) -> Option<&VerbSpec> {
        let (pack_index, verb_index) = *self.by_verb.get(name)?;
        self.packs.get(pack_index)?.verbs.get(verb_index)
    }

    /// Every verb name, pack by pack in contract order.
    pub fn verb_names(&self) -> Vec<&'static str> {
        let mut names = Vec::with_capacity(self.by_verb.len());
        for pack in &self.packs {
            for verb in &pack.verbs {
                if self.by_verb.get(verb.name).is_some_and(|slot| {
                    self.packs
                        .get(slot.0)
                        .is_some_and(|owner| owner.name == pack.name)
                }) {
                    names.push(verb.name);
                }
            }
        }
        names
    }

    /// Packs a fresh session advertises.
    pub fn default_loaded(&self) -> BTreeSet<&'static str> {
        self.packs
            .iter()
            .filter(|pack| pack.default_loaded)
            .map(|pack| pack.name)
            .collect()
    }

    /// Advertised models of the loaded packs, in contract order.
    pub fn models_for(&self, loaded: &BTreeSet<&'static str>) -> Vec<Tool> {
        let mut models = Vec::new();
        for pack in &self.packs {
            if !loaded.contains(pack.name) {
                continue;
            }
            for verb in &pack.verbs {
                if self
                    .verb(verb.name)
                    .is_some_and(|owner| owner.pack == pack.name)
                {
                    models.push(verb.model());
                }
            }
        }
        models
    }

    /// Comma list of pack names for refusal messages.
    pub fn pack_list(&self) -> String {
        self.packs
            .iter()
            .map(|pack| pack.name)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Dispatch one verb by exact name. Unknown names refuse with
    /// `unknown_verb`, the pack list and the search-first recovery.
    pub fn invoke(
        &self,
        server: &McpServer,
        verb: &str,
        args: Value,
    ) -> Result<Value, McpToolError> {
        let spec = self.verb(verb).ok_or_else(|| self.unknown_verb(verb))?;
        spec.invoke(server, args)
    }

    /// Typed refusal for a verb no pack serves.
    pub fn unknown_verb(&self, verb: &str) -> McpToolError {
        McpToolError::new(
            "unknown_verb",
            format!(
                "verb `{verb}` is served by no pack ({}); re-run api.search for the verb, \
                 then api.describe `pack:<name>` for its exact names",
                self.pack_list()
            ),
        )
    }

    /// Seed the Insight index with one entry per pack (`pack:<name>`) and
    /// one per verb, so `api.search` finds both and `api.describe` reads a
    /// pack's verb list at zero tool-list cost.
    pub fn seed_index(&self, index: &mut Index) {
        for pack in &self.packs {
            let verbs = pack.verb_names();
            let loaded = if pack.default_loaded {
                "loaded by default"
            } else {
                "load with api.load"
            };
            let docs = format!(
                "verb pack `{}` ({loaded}): {}. {} verbs: {}. Call any verb with api.call \
                 or by exact name; every verb keeps its capability card, audit record and undo.",
                pack.name,
                pack.summary,
                verbs.len(),
                if verbs.is_empty() {
                    "none yet".to_owned()
                } else {
                    verbs.join(", ")
                }
            );
            index.add_tool(
                &format!("pack:{}", pack.name),
                &format!(
                    "api.load {{pack: \"{}\"}} -> {} verbs",
                    pack.name,
                    verbs.len()
                ),
                &docs,
                "DOOR",
            );
            for verb in &pack.verbs {
                index.add_tool(
                    verb.name,
                    &format!(
                        "{} (pack {}, {} card, {})",
                        verb.name,
                        pack.name,
                        verb.card.name(),
                        if verb.readonly { "read-only" } else { "writes" }
                    ),
                    verb.description(),
                    "DOOR",
                );
            }
        }
        // Component schemas: `api.describe component:<kind>` lists the JSON
        // shape and ranges for the `set_component` / `remove_component` ops.
        for kind in engine_scene::component::ALL_KINDS {
            let (shape, docs) = scene::component_doc(kind);
            index.add_tool(
                &format!("component:{}", kind.name()),
                &format!(
                    "set_component {{id, component: \"{}\", value: {shape}}}",
                    kind.name()
                ),
                &format!(
                    "component schema `{}` v{}: {docs} Authored with tx.preview then tx.apply \
                     (ops set_component / remove_component); undoable; saves byte-identical.",
                    kind.name(),
                    engine_scene::component::SCHEMA_VERSION
                ),
                "DOOR",
            );
        }
    }
}

impl ServerState {
    /// Packs this session advertises.
    pub fn loaded_packs(&self) -> &BTreeSet<&'static str> {
        &self.loaded_packs
    }

    /// Load (advertise) or unload one pack for this session. Unknown pack
    /// names refuse with `unknown_pack` and the legal names. Returns the
    /// pack definition so the caller can list its verbs.
    pub fn load_pack(
        &mut self,
        pack: &str,
        unload: bool,
    ) -> Result<&'static PackDef, McpToolError> {
        let registry = PackRegistry::shipped();
        let def = registry.pack(pack).ok_or_else(|| {
            McpToolError::new(
                "unknown_pack",
                format!(
                    "pack `{pack}` is unknown; the door serves packs [{}]; \
                     re-run with one of those names",
                    registry.pack_list()
                ),
            )
        })?;
        if unload {
            self.loaded_packs.remove(def.name);
        } else {
            self.loaded_packs.insert(def.name);
        }
        Ok(def)
    }
}

// ---------------------------------------------------------------------------
// api.call: one verb from any pack by exact name.
// ---------------------------------------------------------------------------

/// Args for `api.call`: the exact verb name plus its typed args as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiCallArgs {
    /// Exact verb name (`scene.brief`, `geo.preview`, ...).
    pub verb: String,
    /// The verb's own args (its input schema); omitted means `{}`.
    #[serde(default)]
    pub args: Option<Value>,
}

/// `api.call` result: the verb, its pack and the verb's own output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiCallOutput {
    /// The verb that ran.
    pub verb: String,
    /// The pack that serves it.
    pub pack: String,
    /// The verb's output (its output schema).
    pub result: Value,
}

/// `api.call` handler: dispatch one pack verb by exact name.
fn api_call(service: &McpServer, param: &ApiCallArgs) -> Result<ApiCallOutput, McpToolError> {
    let registry = PackRegistry::shipped();
    let Some(verb) = registry.verb(&param.verb) else {
        let refusal = registry.unknown_verb(&param.verb);
        return service.dispatch(TOOL_API_CALL, |_| Err(refusal));
    };
    // The verb runs on its own tool path first (its lock, gate and audit
    // record), then this call seals one `api.call` record of its own.
    let args = param
        .args
        .clone()
        .unwrap_or_else(|| Value::Object(JsonObject::new()));
    let outcome = verb.invoke(service, args);
    let pack = verb.pack().to_owned();
    service.dispatch(TOOL_API_CALL, |_| {
        outcome.map(|result| ApiCallOutput {
            verb: param.verb.clone(),
            pack,
            result,
        })
    })
}

crate::e2_tool!(
    /// `api.call` tool: dispatch one pack verb by exact name.
    ApiCallTool,
    name = TOOL_API_CALL,
    description = "Call one pack verb by exact name with its typed args; the verb's own gates, audit record and undo apply. Find verbs with api.search, schemas with api.describe.",
    readonly = false,
    args = ApiCallArgs,
    output = ApiCallOutput,
    invoke = api_call
);

// ---------------------------------------------------------------------------
// api.load: advertise or hide one pack for this session.
// ---------------------------------------------------------------------------

/// Args for `api.load`: the pack to advertise, or to hide with `unload`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiLoadArgs {
    /// Pack name (`scene`, `break`, `anim`, `material`, `asset`, `rules`, `geo`, `gym`, `view`).
    pub pack: String,
    /// True hides the pack from tools/list again (verbs stay callable).
    #[serde(default)]
    pub unload: Option<bool>,
}

/// `api.load` result: the pack, its verbs, what the session now lists and
/// the tool-list cost after the change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiLoadOutput {
    /// The pack named.
    pub pack: String,
    /// True when the pack is now advertised.
    pub loaded: bool,
    /// The pack's verb names.
    pub verbs: Vec<String>,
    /// Every pack this session now advertises.
    pub loaded_packs: Vec<String>,
    /// Tool-list cost in chars after the change.
    pub list_chars: u64,
    /// Tool-list budget in chars.
    pub budget_chars: u64,
}

/// `api.load` handler: advertise or hide one verb pack for this session.
fn api_load(service: &McpServer, param: &ApiLoadArgs) -> Result<ApiLoadOutput, McpToolError> {
    let unload = param.unload.unwrap_or(false);
    let (verbs, loaded_packs) = service.dispatch(TOOL_API_LOAD, |state| {
        let def = state.load_pack(&param.pack, unload)?;
        let verbs: Vec<String> = def
            .verb_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let loaded: Vec<String> = state
            .loaded_packs()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        Ok((verbs, loaded))
    })?;
    // The list cost reads the advertised list after the lock is released.
    let list_chars = u64::try_from(service.tool_list_budget_chars()).unwrap_or(u64::MAX);
    Ok(ApiLoadOutput {
        loaded: !unload,
        pack: param.pack.clone(),
        verbs,
        loaded_packs,
        list_chars,
        budget_chars: u64::try_from(TOOL_LIST_BUDGET_CHARS).unwrap_or(u64::MAX),
    })
}

crate::e2_tool!(
    /// `api.load` tool: advertise or hide one verb pack for this session.
    ApiLoadTool,
    name = TOOL_API_LOAD,
    description = "Load one verb pack so tools/list advertises its verbs (unload hides them again); verbs stay callable by name either way. Packs: api.search \"pack\".",
    readonly = false,
    args = ApiLoadArgs,
    output = ApiLoadOutput,
    invoke = api_load
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiSearchArgs, ApiSearchTool};
    use crate::mcp::tool_names;
    use crate::scene_tools::{SceneBriefTool, TOOL_SCENE_BRIEF};
    use engine_editor::capability::Capability;
    use rmcp::model::ToolAnnotations;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// A tool with no annotations: the guard must refuse it.
    struct BareTool;

    /// A tool with no input schema: the guard must refuse it fail-closed
    /// instead of advertising a permissive shape.
    struct NoSchemaTool;

    impl ToolBase for NoSchemaTool {
        type Parameter = ApiLoadArgs;
        type Output = ApiLoadOutput;
        type Error = McpToolError;

        fn name() -> Cow<'static, str> {
            "test.noschema".into()
        }

        fn input_schema() -> Option<Arc<JsonObject>> {
            None
        }

        fn annotations() -> Option<ToolAnnotations> {
            let mut annotations = ToolAnnotations::default();
            annotations.read_only_hint = Some(true);
            Some(annotations)
        }
    }

    impl SyncTool<McpServer> for NoSchemaTool {
        fn invoke(
            _service: &McpServer,
            _param: Self::Parameter,
        ) -> Result<Self::Output, Self::Error> {
            Ok(ApiLoadOutput::default())
        }
    }

    impl ToolBase for BareTool {
        type Parameter = ApiLoadArgs;
        type Output = ApiLoadOutput;
        type Error = McpToolError;

        fn name() -> Cow<'static, str> {
            "bare.verb".into()
        }
    }

    impl SyncTool<McpServer> for BareTool {
        fn invoke(
            _service: &McpServer,
            _param: Self::Parameter,
        ) -> Result<Self::Output, Self::Error> {
            Ok(ApiLoadOutput::default())
        }
    }

    /// A tool whose name carries a forbidden word.
    struct ShellTool;

    impl ToolBase for ShellTool {
        type Parameter = ApiLoadArgs;
        type Output = ApiLoadOutput;
        type Error = McpToolError;

        fn name() -> Cow<'static, str> {
            "pack.shell".into()
        }

        fn annotations() -> Option<ToolAnnotations> {
            let mut annotations = ToolAnnotations::default();
            annotations.read_only_hint = Some(true);
            Some(annotations)
        }
    }

    impl SyncTool<McpServer> for ShellTool {
        fn invoke(
            _service: &McpServer,
            _param: Self::Parameter,
        ) -> Result<Self::Output, Self::Error> {
            Ok(ApiLoadOutput::default())
        }
    }

    #[test]
    fn guard_refuses_verbs_without_annotation_or_capability() {
        assert_eq!(
            VerbSpec::of::<BareTool>("bare.verb", "scene", CapabilityCard::Observe).err(),
            Some(PackError::MissingAnnotation { verb: "bare.verb" })
        );
        assert_eq!(
            VerbSpec::of::<SceneBriefTool>(TOOL_SCENE_BRIEF, "scene", CapabilityCard::Deny).err(),
            Some(PackError::MissingCapability {
                verb: TOOL_SCENE_BRIEF
            })
        );
        assert_eq!(
            VerbSpec::of::<SceneBriefTool>("scene.other", "scene", CapabilityCard::Observe).err(),
            Some(PackError::NameMismatch {
                verb: "scene.other"
            })
        );
        assert_eq!(
            VerbSpec::of::<ShellTool>("pack.shell", "scene", CapabilityCard::Observe).err(),
            Some(PackError::BadName { verb: "pack.shell" })
        );
        for error in [
            PackError::MissingAnnotation { verb: "x.y" },
            PackError::MissingCapability { verb: "x.y" },
            PackError::MissingSchema { verb: "x.y" },
            PackError::NameMismatch { verb: "x.y" },
            PackError::BadName { verb: "x.y" },
            PackError::DuplicateVerb { verb: "x.y" },
        ] {
            assert!(error.to_string().contains("x.y"), "{error}");
        }
        // A duplicate registration records its error and keeps the first.
        let mut first = PackDef::new("scene", "one", true);
        first.add(
            VerbSpec::of::<SceneBriefTool>(TOOL_SCENE_BRIEF, "scene", CapabilityCard::Observe)
                .expect("brief registers"),
        );
        let mut second = PackDef::new("view", "two", false);
        second.add(
            VerbSpec::of::<SceneBriefTool>(TOOL_SCENE_BRIEF, "view", CapabilityCard::Observe)
                .expect("brief registers"),
        );
        let registry = PackRegistry::build(vec![Ok(first), Ok(second)]);
        assert_eq!(
            registry.errors(),
            &[PackError::DuplicateVerb {
                verb: TOOL_SCENE_BRIEF
            }]
        );
        assert_eq!(
            registry.verb(TOOL_SCENE_BRIEF).map(VerbSpec::pack),
            Some("scene")
        );
        assert_eq!(registry.verb_names(), vec![TOOL_SCENE_BRIEF]);
        assert_eq!(
            registry.models_for(&["view"].into_iter().collect()).len(),
            0,
            "the losing duplicate advertises nothing"
        );
    }

    #[test]
    fn guard_refuses_schema_less_verbs_fail_closed() {
        // Registration refuses: no permissive advertisement, no dispatch.
        assert_eq!(
            VerbSpec::of::<NoSchemaTool>("test.noschema", "scene", CapabilityCard::Observe).err(),
            Some(PackError::MissingSchema {
                verb: "test.noschema"
            })
        );
        // Defense in depth: the fallback advertises a closed empty-object
        // shape, never an unconstrained empty map.
        let fallback = closed_empty_schema();
        assert_eq!(
            fallback.get("type").and_then(Value::as_str),
            Some("object"),
            "fallback constrains to objects"
        );
        assert!(
            fallback.get("properties").is_some_and(Value::is_object),
            "fallback declares (empty) properties"
        );
        // Shipped verbs all carry schemas, so none hit the fallback.
        for verb in PackRegistry::shipped().verb_names() {
            let model = PackRegistry::shipped()
                .verb(&verb)
                .expect("verb lists")
                .model();
            let schema = &model.input_schema;
            assert_eq!(
                schema.get("type").and_then(Value::as_str),
                Some("object"),
                "{verb} advertises an object schema"
            );
        }
    }

    #[test]
    fn shipped_registry_is_clean_and_covers_every_pack() {
        let registry = PackRegistry::shipped();
        assert!(
            registry.errors().is_empty(),
            "shipped packs register without error: {:?}",
            registry.errors()
        );
        let names: Vec<&str> = registry.packs().iter().map(PackDef::name).collect();
        assert_eq!(names, PACK_NAMES);
        for pack in registry.packs() {
            assert!(
                !pack.summary().is_empty(),
                "pack `{}` has a summary",
                pack.name()
            );
            for verb in pack.verbs() {
                assert_eq!(verb.pack(), pack.name());
                assert_ne!(verb.card(), CapabilityCard::Deny);
                assert!(
                    tool_names().contains(&verb.name()),
                    "{} is a contract name",
                    verb.name()
                );
                let model = verb.model();
                assert_eq!(model.name, verb.name());
                assert!(
                    model.output_schema.is_some(),
                    "{} advertises its output schema",
                    verb.name()
                );
            }
        }
        // The old scene tools live in the default-loaded scene pack.
        assert!(registry.default_loaded().contains("scene"));
        for old in [
            "scene.brief",
            "scene.spatial",
            "scene.place",
            "level.build",
            "scene.query",
            "scene.watch",
            "sim.plan_goals",
            "play.step",
            "log.digest",
            "log.query",
            "code.callers",
            "asset.refs",
            "rpc.discover",
        ] {
            assert!(
                registry.verb(old).is_some(),
                "old tool `{old}` is reachable as a verb"
            );
        }
        let server = server();
        let packs = registry.packs().len();
        let seeded = server.lock().expect("test lock holds").insight.len();
        assert!(
            seeded >= packs + registry.verb_names().len(),
            "index seeds one entry per pack and per verb: {seeded}"
        );
    }

    #[test]
    fn api_search_finds_at_least_six_packs() {
        let server = server();
        let found = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "pack".to_owned(),
                limit: Some(32),
            },
        )
        .expect("search serves");
        let packs: Vec<&str> = found
            .hits
            .iter()
            .filter(|hit| hit.path.starts_with("pack:"))
            .map(|hit| hit.path.as_str())
            .collect();
        assert!(
            packs.len() >= 6,
            "api.search finds {} packs: {packs:?}",
            packs.len()
        );
        for name in PACK_NAMES {
            assert!(
                packs.contains(&format!("pack:{name}").as_str()),
                "pack `{name}` is found: {packs:?}"
            );
        }
        // A verb search lands on the verb entry with its pack named.
        let brief = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "scene brief".to_owned(),
                limit: None,
            },
        )
        .expect("search serves");
        assert!(
            brief.hits.iter().any(|hit| hit.path == TOOL_SCENE_BRIEF),
            "scene.brief is found: {:?}",
            brief.hits
        );
    }

    #[test]
    fn api_call_dispatches_old_tools_and_refuses_unknown_verbs() {
        let server = server();
        let called = ApiCallTool::invoke(
            &server,
            ApiCallArgs {
                verb: TOOL_SCENE_BRIEF.to_owned(),
                args: None,
            },
        )
        .expect("brief runs through api.call");
        assert_eq!(called.verb, TOOL_SCENE_BRIEF);
        assert_eq!(called.pack, "scene");
        assert_eq!(called.result["revision"].as_u64(), Some(0));
        // Direct and indirect calls answer the same bytes.
        let direct = SceneBriefTool::invoke(&server, Default::default()).expect("brief runs");
        let direct_json = rmcp::serde_json::to_value(direct).expect("output serializes");
        assert_eq!(called.result, direct_json);
        // Unknown verbs refuse with the pack list and a recovery.
        let refused = ApiCallTool::invoke(
            &server,
            ApiCallArgs {
                verb: "shell.exec".to_owned(),
                args: None,
            },
        )
        .expect_err("unknown verb refuses");
        assert_eq!(refused.code, "unknown_verb");
        assert!(refused.message.contains("scene, break"));
        assert!(refused.message.contains("re-run"));
        // Bad args refuse typed, naming the verb.
        let bad = ApiCallTool::invoke(
            &server,
            ApiCallArgs {
                verb: "scene.query".to_owned(),
                args: Some(Value::String("nope".to_owned())),
            },
        )
        .expect_err("bad args refuse");
        assert_eq!(bad.code, "bad_args");
        assert!(bad.message.contains("scene.query"));
        // Every api.call seals one audit record of its own beside the verb's.
        let log = server.audit_log().expect("audit reads");
        let names: Vec<&str> = log.iter().map(|record| record.tool.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "scene.brief",
                "api.call",
                "scene.brief",
                "api.call",
                "api.call"
            ]
        );
    }

    #[test]
    fn api_load_advertises_packs_on_demand_and_unload_hides_them() {
        let server = server();
        let listed = |server: &McpServer| -> Vec<String> {
            server
                .tools()
                .iter()
                .map(|tool| tool.name.to_string())
                .collect()
        };
        let before = listed(&server);
        assert!(
            before.contains(&"scene.brief".to_owned()),
            "scene pack is default-loaded"
        );
        assert!(
            !before.contains(&"log.digest".to_owned()),
            "view pack waits for api.load"
        );
        let loaded = ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: "view".to_owned(),
                unload: None,
            },
        )
        .expect("view loads");
        assert!(loaded.loaded);
        assert!(loaded.verbs.contains(&"log.digest".to_owned()));
        assert!(loaded.loaded_packs.contains(&"view".to_owned()));
        assert!(loaded.list_chars > 0 && loaded.list_chars <= loaded.budget_chars);
        let after = listed(&server);
        assert!(after.contains(&"log.digest".to_owned()));
        assert!(after.len() > before.len());
        let unloaded = ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: "view".to_owned(),
                unload: Some(true),
            },
        )
        .expect("view unloads");
        assert!(!unloaded.loaded);
        assert_eq!(listed(&server), before);
        // Unknown packs refuse with the legal names.
        let refused = ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: "shell".to_owned(),
                unload: None,
            },
        )
        .expect_err("unknown pack refuses");
        assert_eq!(refused.code, "unknown_pack");
        assert!(refused.message.contains("geo"));
    }
}
