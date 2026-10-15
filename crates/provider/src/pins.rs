//! pins: what the engine pins about how a model is asked, on every wire.
//! One [`RequestPins`] value rides every request an HTTP adapter sends: a
//! system prompt (the task kind's own unless the caller overrides it), a
//! sampling temperature that defaults to zero, a seed where the vendor
//! offers one, and tool or function definitions when the caller gives them.
//! Nothing here reads a clock or a socket; the pins are plain data, so the
//! same request plus the same pins always serializes to the same body, and a
//! recorded answer can be replayed against it (see [`crate::cassette`]).
//!
//! The pins live with the adapter, not in [`AiRequest`]: the request type is
//! built field by field in other crates, and the pins are session policy,
//! not per-call data. Each vendor module turns the pins into its own body
//! shape through the helpers below, so the vendor differences stay at the
//! envelope edge.

use serde_json::{json, Map, Value};

use super::ladder::{proposal_schema, LadderPlan, Rung, NATIVE_TOOL, SCHEMA_NAME};
use super::protocol::{AiRequest, TaskKind};
use super::vendor::prompt_text;
use super::MAX_TEXT_LEN;

/// Seed every request carries where the vendor offers one. Small enough for
/// the narrowest vendor field (a signed 32-bit integer).
pub const PINNED_SEED: u64 = 2040;
/// Highest temperature a caller may pin, in thousandths (OpenAI's ceiling).
pub const MAX_TEMPERATURE_MILLI: u16 = 2000;
/// Most tool definitions one request may carry.
pub const MAX_TOOLS: usize = 16;
/// Longest tool name, in bytes (the strictest vendor limit).
pub const MAX_TOOL_NAME_LEN: usize = 64;
/// Longest tool description, in bytes.
pub const MAX_TOOL_DESCRIPTION_LEN: usize = 512;
/// Largest serialized tool parameter schema, in bytes.
pub const MAX_TOOL_SCHEMA_BYTES: usize = 8192;

/// The closed proposal grammar, as the model is told it. The strict parser
/// in `vendor.rs` is the authority; this text only states it.
macro_rules! grammar {
    () => {
        " Reply with exactly one JSON object and nothing else: {\"actions\": [...], \"diagnostics\": [...]}. \
Each action is one of {\"noop\": true}, \
{\"set_position\": {\"object\": <u64 object id>, \"xyz_cm\": [<i32>, <i32>, <i32>]}} \
(centimeters, each axis at most 1000000 in magnitude), or \
{\"set_label\": {\"object\": <u64 object id>, \"label\": \"<1 to 128 characters, no control characters>\"}}. \
Diagnostics are single-line strings. Add no other keys. The request lists digests of \
project context; never invent an object id you were not given."
    };
}

/// The system prompt a request of `kind` carries when the caller sets none.
/// Fixed text per kind: a prompt change is a code change, and a recorded
/// answer misses (see [`crate::cassette`]) instead of being reused.
#[must_use]
pub fn default_system_prompt(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Inspect => concat!(
            "You inspect a game-editor scene and report findings in diagnostics only; propose no changes.",
            grammar!()
        ),
        TaskKind::Propose => concat!(
            "You propose a typed edit to a game-editor scene. Propose only actions the context supports; you never apply them.",
            grammar!()
        ),
        TaskKind::Diagnose => concat!(
            "You diagnose one staged finding in a game-editor scene. Explain it in diagnostics and propose a fix only when sure.",
            grammar!()
        ),
        TaskKind::Repair => concat!(
            "You repair one diagnosed finding in a game-editor scene with the smallest typed action set.",
            grammar!()
        ),
        TaskKind::Announce => concat!(
            "You are the arena announcer. Put one short spoken line in the first diagnostics entry; actions stay empty.",
            grammar!()
        ),
        TaskKind::Taunt => concat!(
            "You write one short combat taunt. Put it in the first diagnostics entry; actions stay empty.",
            grammar!()
        ),
    }
}

/// Why a pin was refused. Carries no caller text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinError {
    /// The system prompt is empty, longer than [`MAX_TEXT_LEN`] bytes, or
    /// holds a NUL byte.
    BadSystem,
    /// The temperature is above [`MAX_TEMPERATURE_MILLI`].
    TemperatureTooHigh,
    /// More than [`MAX_TOOLS`] tools, or two tools share a name.
    BadToolSet,
    /// A tool name is empty, over [`MAX_TOOL_NAME_LEN`] bytes, or outside
    /// `[A-Za-z0-9_-]`.
    BadToolName,
    /// A tool description or parameter schema is over its byte bound, or the
    /// schema is not a JSON object.
    BadToolBody,
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PinError::BadSystem => "system prompt empty, too long or holds NUL",
            PinError::TemperatureTooHigh => "temperature above the pinned ceiling",
            PinError::BadToolSet => "tool set too large or names repeat",
            PinError::BadToolName => "tool name empty, too long or outside [A-Za-z0-9_-]",
            PinError::BadToolBody => "tool description or schema out of bounds",
        })
    }
}

impl std::error::Error for PinError {}

/// One tool (function) the model may name in its answer. The engine never
/// executes a tool: a tool call is a proposal, applied only through the
/// command service like any other.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ToolDef {
    name: String,
    description: String,
    parameters: Value,
}

impl ToolDef {
    /// Build one tool. `parameters` is a JSON-Schema object for the call
    /// arguments; name, description and schema size are bounded.
    pub fn new(name: &str, description: &str, parameters: Value) -> Result<Self, PinError> {
        let name_ok = !name.is_empty()
            && name.len() <= MAX_TOOL_NAME_LEN
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !name_ok {
            return Err(PinError::BadToolName);
        }
        let schema_len = parameters.to_string().len();
        if description.len() > MAX_TOOL_DESCRIPTION_LEN
            || !parameters.is_object()
            || schema_len > MAX_TOOL_SCHEMA_BYTES
        {
            return Err(PinError::BadToolBody);
        }
        Ok(Self {
            name: name.to_owned(),
            description: description.to_owned(),
            parameters,
        })
    }

    /// Tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tool description shown to the model.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// JSON-Schema object for the call arguments.
    #[must_use]
    pub fn parameters(&self) -> &Value {
        &self.parameters
    }
}

/// What shape of answer one attempt asks the vendor for. The ladder sets it
/// per rung (see [`RequestPins::at`]); a plain request asks for text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ResponseFormat {
    /// Plain text.
    Text,
    /// The vendor's JSON mode.
    JsonObject,
    /// The vendor's native structured output, constrained by the engine
    /// proposal schema.
    Schema,
}

/// The pinned way one adapter asks its model. Defaults are the repeatable
/// ones: the task kind's system prompt, temperature zero, [`PINNED_SEED`],
/// no tools, and the text rung of the structured-output ladder. A builder
/// method refuses out-of-bound input, so a held value is always sendable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RequestPins {
    system: Option<String>,
    temperature_milli: u16,
    seed: Option<u64>,
    tools: Vec<ToolDef>,
    ladder: LadderPlan,
    response: ResponseFormat,
}

impl Default for RequestPins {
    fn default() -> Self {
        Self {
            system: None,
            temperature_milli: 0,
            seed: Some(PINNED_SEED),
            tools: Vec::new(),
            ladder: LadderPlan::default(),
            response: ResponseFormat::Text,
        }
    }
}

impl RequestPins {
    /// Override the system prompt for every task kind.
    pub fn with_system(mut self, system: &str) -> Result<Self, PinError> {
        if system.is_empty() || system.len() > MAX_TEXT_LEN || system.contains('\0') {
            return Err(PinError::BadSystem);
        }
        self.system = Some(system.to_owned());
        Ok(self)
    }

    /// Pin the sampling temperature, in thousandths (0 is greedy).
    pub fn with_temperature_milli(mut self, milli: u16) -> Result<Self, PinError> {
        if milli > MAX_TEMPERATURE_MILLI {
            return Err(PinError::TemperatureTooHigh);
        }
        self.temperature_milli = milli;
        Ok(self)
    }

    /// Pin the seed, or `None` to send none (vendors without a seed field
    /// never send one regardless).
    #[must_use]
    pub fn with_seed(mut self, seed: Option<u64>) -> Self {
        self.seed = seed;
        self
    }

    /// Give the model a set of tools. Names must be unique.
    pub fn with_tools(mut self, tools: Vec<ToolDef>) -> Result<Self, PinError> {
        let repeats = tools
            .iter()
            .enumerate()
            .any(|(i, tool)| tools[..i].iter().any(|seen| seen.name == tool.name));
        if tools.len() > MAX_TOOLS || repeats {
            return Err(PinError::BadToolSet);
        }
        self.tools = tools;
        Ok(self)
    }

    /// Where on the structured-output ladder the adapter starts. The default
    /// is the text rung; a higher start asks the vendor for JSON mode or its
    /// native structured output first and falls back rung by rung.
    #[must_use]
    pub fn with_ladder(mut self, plan: LadderPlan) -> Self {
        self.ladder = plan;
        self
    }

    /// The ladder plan.
    #[must_use]
    pub fn ladder(&self) -> LadderPlan {
        self.ladder
    }

    /// These pins as one attempt on `rung` sends them: the same system
    /// prompt, sampling and tools, with the answer shape the rung asks for.
    #[must_use]
    pub(crate) fn at(&self, rung: Rung) -> Self {
        let mut pins = self.clone();
        pins.response = match rung {
            Rung::Native => ResponseFormat::Schema,
            Rung::JsonMode => ResponseFormat::JsonObject,
            Rung::Text | Rung::Scripted => ResponseFormat::Text,
        };
        pins
    }

    /// The system prompt a request of `kind` carries: the override when one
    /// is set, otherwise the kind's own [`default_system_prompt`].
    #[must_use]
    pub fn system_for(&self, kind: TaskKind) -> &str {
        match &self.system {
            Some(system) => system,
            None => default_system_prompt(kind),
        }
    }

    /// Pinned temperature, in thousandths.
    #[must_use]
    pub fn temperature_milli(&self) -> u16 {
        self.temperature_milli
    }

    /// Pinned seed, if any.
    #[must_use]
    pub fn seed(&self) -> Option<u64> {
        self.seed
    }

    /// Tools the model may name.
    #[must_use]
    pub fn tools(&self) -> &[ToolDef] {
        &self.tools
    }

    /// Temperature as the wire number. Vendor bodies are not hashed state,
    /// so a float on the wire is fine; the integer thousandths stay the
    /// canonical value everywhere else.
    fn temperature(&self) -> f64 {
        f64::from(self.temperature_milli) / 1000.0
    }

    /// OpenAI-shaped `tools` array (`type: function`), or `None` when the
    /// caller gave no tools. Chat-completions hosts and Ollama share it.
    fn function_tools(&self) -> Option<Value> {
        if self.tools.is_empty() {
            return None;
        }
        let list: Vec<Value> = self
            .tools
            .iter()
            .map(|tool| {
                json!({"type": "function", "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }})
            })
            .collect();
        Some(Value::Array(list))
    }

    /// The OpenAI-compatible chat body shared by chat-completions hosts, the
    /// stream body and (with its own sampling block) nothing else: model,
    /// system plus user messages, token bound, temperature, seed, tools.
    pub(crate) fn chat_fields(&self, request: &AiRequest) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert("model".into(), json!(request.model.model));
        body.insert(
            "messages".into(),
            json!([
                {"role": "system", "content": self.system_for(request.kind)},
                {"role": "user", "content": prompt_text(request)},
            ]),
        );
        body.insert("max_tokens".into(), json!(request.budget.max_output_tokens));
        body.insert("temperature".into(), json!(self.temperature()));
        if let Some(seed) = self.seed {
            body.insert("seed".into(), json!(seed));
        }
        if let Some(tools) = self.function_tools() {
            body.insert("tools".into(), tools);
        }
        match self.response {
            ResponseFormat::Text => {}
            ResponseFormat::JsonObject => {
                body.insert("response_format".into(), json!({"type": "json_object"}));
            }
            ResponseFormat::Schema => {
                body.insert(
                    "response_format".into(),
                    json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": SCHEMA_NAME,
                            "strict": true,
                            "schema": proposal_schema(),
                        },
                    }),
                );
            }
        }
        body
    }

    /// Anthropic Messages fields: top-level `system`, `temperature` capped
    /// at the vendor's 1.0, `tools` with `input_schema`. Anthropic offers no
    /// seed, so none is sent.
    pub(crate) fn messages_fields(&self, request: &AiRequest) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert("model".into(), json!(request.model.model));
        body.insert("max_tokens".into(), json!(request.budget.max_output_tokens));
        body.insert("system".into(), json!(self.system_for(request.kind)));
        body.insert(
            "temperature".into(),
            json!(f64::from(self.temperature_milli.min(1000)) / 1000.0),
        );
        body.insert(
            "messages".into(),
            json!([{"role": "user", "content": prompt_text(request)}]),
        );
        let mut list: Vec<Value> = self
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();
        if self.response == ResponseFormat::Schema {
            // Native structured output on Messages is a forced tool call
            // whose input is the proposal.
            list.push(json!({
                "name": NATIVE_TOOL,
                "description": "Return the proposal.",
                "input_schema": proposal_schema(),
            }));
            body.insert(
                "tool_choice".into(),
                json!({"type": "tool", "name": NATIVE_TOOL}),
            );
        }
        if !list.is_empty() {
            body.insert("tools".into(), Value::Array(list));
        }
        body
    }

    /// Gemini `generateContent` fields: `systemInstruction`, user contents,
    /// `generationConfig` with temperature and seed, `functionDeclarations`.
    pub(crate) fn generate_fields(&self, request: &AiRequest) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert(
            "systemInstruction".into(),
            json!({"parts": [{"text": self.system_for(request.kind)}]}),
        );
        body.insert(
            "contents".into(),
            json!([{"parts": [{"text": prompt_text(request)}]}]),
        );
        let mut config = Map::new();
        config.insert(
            "maxOutputTokens".into(),
            json!(request.budget.max_output_tokens),
        );
        config.insert("temperature".into(), json!(self.temperature()));
        if let Some(seed) = self.seed {
            config.insert("seed".into(), json!(seed));
        }
        match self.response {
            ResponseFormat::Text => {}
            ResponseFormat::JsonObject => {
                config.insert("responseMimeType".into(), json!("application/json"));
            }
            ResponseFormat::Schema => {
                config.insert("responseMimeType".into(), json!("application/json"));
                config.insert("responseJsonSchema".into(), proposal_schema());
            }
        }
        body.insert("generationConfig".into(), Value::Object(config));
        if !self.tools.is_empty() {
            let declarations: Vec<Value> = self
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    })
                })
                .collect();
            body.insert(
                "tools".into(),
                json!([{"functionDeclarations": declarations}]),
            );
        }
        body
    }

    /// Ollama `/api/chat` fields: system plus user messages, streaming off,
    /// `options` with temperature and seed, function tools.
    pub(crate) fn ollama_fields(&self, request: &AiRequest) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert("model".into(), json!(request.model.model));
        body.insert(
            "messages".into(),
            json!([
                {"role": "system", "content": self.system_for(request.kind)},
                {"role": "user", "content": prompt_text(request)},
            ]),
        );
        body.insert("stream".into(), json!(false));
        let mut options = Map::new();
        options.insert("temperature".into(), json!(self.temperature()));
        if let Some(seed) = self.seed {
            options.insert("seed".into(), json!(seed));
        }
        body.insert("options".into(), Value::Object(options));
        if let Some(tools) = self.function_tools() {
            body.insert("tools".into(), tools);
        }
        match self.response {
            ResponseFormat::Text => {}
            ResponseFormat::JsonObject => {
                body.insert("format".into(), json!("json"));
            }
            ResponseFormat::Schema => {
                body.insert("format".into(), proposal_schema());
            }
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::fixtures::request;

    fn sample_tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            "Move one object.",
            json!({"type": "object", "properties": {"object": {"type": "integer"}}}),
        )
        .expect("sample tool is in bounds")
    }

    #[test]
    fn defaults_are_the_repeatable_ones() {
        let pins = RequestPins::default();
        assert_eq!(pins.temperature_milli(), 0, "greedy by default");
        assert_eq!(pins.seed(), Some(PINNED_SEED));
        assert!(pins.tools().is_empty());
        for kind in [
            TaskKind::Inspect,
            TaskKind::Propose,
            TaskKind::Diagnose,
            TaskKind::Repair,
            TaskKind::Announce,
            TaskKind::Taunt,
        ] {
            let system = pins.system_for(kind);
            assert!(system.len() < MAX_TEXT_LEN, "{kind:?} prompt is bounded");
            for key in ["noop", "set_position", "set_label", "diagnostics"] {
                assert!(system.contains(key), "{kind:?} prompt states `{key}`");
            }
        }
        assert_ne!(
            pins.system_for(TaskKind::Propose),
            pins.system_for(TaskKind::Taunt),
            "each kind carries its own role line"
        );
    }

    #[test]
    fn override_replaces_every_kinds_prompt() {
        let pins = RequestPins::default()
            .with_system("Be terse.")
            .expect("in bounds");
        assert_eq!(pins.system_for(TaskKind::Propose), "Be terse.");
        assert_eq!(pins.system_for(TaskKind::Taunt), "Be terse.");
    }

    #[test]
    fn builders_refuse_out_of_bound_pins() {
        let base = RequestPins::default;
        assert_eq!(base().with_system("").unwrap_err(), PinError::BadSystem);
        assert_eq!(base().with_system("a\0b").unwrap_err(), PinError::BadSystem);
        assert_eq!(
            base()
                .with_system(&"x".repeat(MAX_TEXT_LEN + 1))
                .unwrap_err(),
            PinError::BadSystem
        );
        assert_eq!(
            base()
                .with_temperature_milli(MAX_TEMPERATURE_MILLI + 1)
                .unwrap_err(),
            PinError::TemperatureTooHigh
        );
        let twice = vec![sample_tool("move"), sample_tool("move")];
        assert_eq!(base().with_tools(twice).unwrap_err(), PinError::BadToolSet);
        let many: Vec<ToolDef> = (0..=MAX_TOOLS)
            .map(|i| sample_tool(&format!("tool{i}")))
            .collect();
        assert_eq!(base().with_tools(many).unwrap_err(), PinError::BadToolSet);
        let long = "n".repeat(MAX_TOOL_NAME_LEN + 1);
        for bad in ["", "has space", "slash/name", long.as_str()] {
            assert_eq!(
                ToolDef::new(bad, "d", json!({})).unwrap_err(),
                PinError::BadToolName,
                "{bad:?}"
            );
        }
        assert_eq!(
            ToolDef::new("t", &"d".repeat(MAX_TOOL_DESCRIPTION_LEN + 1), json!({})).unwrap_err(),
            PinError::BadToolBody
        );
        assert_eq!(
            ToolDef::new("t", "d", json!([1])).unwrap_err(),
            PinError::BadToolBody,
            "schema must be an object"
        );
        let huge = json!({"pad": "x".repeat(MAX_TOOL_SCHEMA_BYTES)});
        assert_eq!(
            ToolDef::new("t", "d", huge).unwrap_err(),
            PinError::BadToolBody
        );
    }

    #[test]
    fn chat_shape_carries_system_temperature_seed_and_tools() {
        let ask = request(1, 3);
        let plain = RequestPins::default().chat_fields(&ask);
        assert_eq!(plain["messages"][0]["role"], "system");
        assert_eq!(
            plain["messages"][0]["content"],
            default_system_prompt(ask.kind)
        );
        assert_eq!(plain["messages"][1]["role"], "user");
        assert_eq!(plain["temperature"].as_f64(), Some(0.0));
        assert_eq!(plain["seed"], PINNED_SEED);
        assert!(!plain.contains_key("tools"), "no tools unless given");

        let pins = RequestPins::default()
            .with_temperature_milli(700)
            .expect("in bounds")
            .with_seed(None)
            .with_tools(vec![sample_tool("move")])
            .expect("one tool");
        let rich = pins.chat_fields(&ask);
        assert_eq!(rich["temperature"].as_f64(), Some(0.7));
        assert!(!rich.contains_key("seed"), "seed unpinned means none sent");
        assert_eq!(rich["tools"][0]["type"], "function");
        assert_eq!(rich["tools"][0]["function"]["name"], "move");
    }

    #[test]
    fn anthropic_shape_has_top_level_system_no_seed_and_input_schema() {
        let ask = request(1, 3);
        let pins = RequestPins::default()
            .with_temperature_milli(1500)
            .expect("in bounds")
            .with_tools(vec![sample_tool("move")])
            .expect("one tool");
        let body = pins.messages_fields(&ask);
        assert_eq!(body["system"], default_system_prompt(ask.kind));
        assert_eq!(body["temperature"].as_f64(), Some(1.0), "vendor cap is 1.0");
        assert!(!body.contains_key("seed"), "anthropic has no seed field");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["tools"][0]["name"], "move");
        assert!(body["tools"][0]["input_schema"].is_object());
    }

    #[test]
    fn gemini_shape_nests_pins_in_system_instruction_and_generation_config() {
        let ask = request(1, 3);
        let pins = RequestPins::default()
            .with_tools(vec![sample_tool("move")])
            .expect("one tool");
        let body = pins.generate_fields(&ask);
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            default_system_prompt(ask.kind)
        );
        assert_eq!(body["generationConfig"]["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["generationConfig"]["seed"], PINNED_SEED);
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "move");
    }

    #[test]
    fn ollama_shape_puts_sampling_under_options() {
        let ask = request(1, 3);
        let body = RequestPins::default().ollama_fields(&ask);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["stream"], false);
        assert_eq!(body["options"]["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["options"]["seed"], PINNED_SEED);
    }

    #[test]
    fn answer_shape_follows_the_rung_on_every_vendor_shape() {
        use crate::ladder::{proposal_schema, LadderPlan, Rung, NATIVE_TOOL, SCHEMA_NAME};
        let ask = request(1, 3);
        let pins = RequestPins::default()
            .with_tools(vec![sample_tool("move")])
            .expect("one tool");
        assert_eq!(pins.ladder(), LadderPlan::text(), "text rung by default");
        assert_eq!(
            pins.clone().with_ladder(LadderPlan::native()).ladder(),
            LadderPlan::native()
        );
        // OpenAI chat shape.
        assert!(!pins
            .at(Rung::Text)
            .chat_fields(&ask)
            .contains_key("response_format"));
        assert_eq!(
            pins.at(Rung::JsonMode).chat_fields(&ask)["response_format"],
            json!({"type": "json_object"})
        );
        let native = pins.at(Rung::Native).chat_fields(&ask);
        assert_eq!(native["response_format"]["type"], "json_schema");
        assert_eq!(
            native["response_format"]["json_schema"]["name"],
            SCHEMA_NAME
        );
        assert_eq!(native["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            native["response_format"]["json_schema"]["schema"],
            proposal_schema()
        );
        assert_eq!(
            native["tools"][0]["function"]["name"], "move",
            "caller tools stay"
        );
        // Anthropic: a forced tool call beside the caller's tools.
        let names = |body: &Map<String, Value>| -> Vec<String> {
            body["tools"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|t| t["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let forced = pins.at(Rung::Native).messages_fields(&ask);
        assert_eq!(names(&forced), vec!["move", NATIVE_TOOL]);
        assert_eq!(
            forced["tool_choice"],
            json!({"type": "tool", "name": NATIVE_TOOL})
        );
        let plain = pins.at(Rung::Text).messages_fields(&ask);
        assert_eq!(names(&plain), vec!["move"]);
        assert!(!plain.contains_key("tool_choice"));
        let bare = RequestPins::default().at(Rung::Text).messages_fields(&ask);
        assert!(!bare.contains_key("tools"), "no tools, no empty array");
        // Gemini.
        let config = |rung| pins.at(rung).generate_fields(&ask)["generationConfig"].clone();
        assert!(config(Rung::Text).get("responseMimeType").is_none());
        assert_eq!(
            config(Rung::JsonMode)["responseMimeType"],
            "application/json"
        );
        assert!(config(Rung::JsonMode).get("responseJsonSchema").is_none());
        assert_eq!(
            config(Rung::Native)["responseJsonSchema"],
            proposal_schema()
        );
        // Ollama.
        assert!(!pins
            .at(Rung::Text)
            .ollama_fields(&ask)
            .contains_key("format"));
        assert_eq!(
            pins.at(Rung::JsonMode).ollama_fields(&ask)["format"],
            "json"
        );
        assert_eq!(
            pins.at(Rung::Native).ollama_fields(&ask)["format"],
            proposal_schema()
        );
        // The scripted floor asks for nothing special.
        assert_eq!(pins.at(Rung::Scripted), pins.at(Rung::Text));
    }

    #[test]
    fn json_mode_vendors_need_the_word_json_in_every_default_prompt() {
        for kind in [
            TaskKind::Inspect,
            TaskKind::Propose,
            TaskKind::Diagnose,
            TaskKind::Repair,
            TaskKind::Announce,
            TaskKind::Taunt,
        ] {
            assert!(default_system_prompt(kind).contains("JSON"), "{kind:?}");
        }
    }

    const NOOP: &str = r#"{"actions": [{"noop": true}], "diagnostics": ["pins:stub"]}"#;

    /// One real loopback exchange through `make`'s adapter; returns the JSON
    /// body the stub received, i.e. the bytes that crossed the socket.
    fn sent_body<A: crate::ProviderAdapter>(remote: bool, make: impl Fn(&str) -> A) -> Value {
        use crate::policy::{AiSessionPolicy, Fallback, ProviderCapabilities};
        use crate::protocol::{AdapterKind, PrivacyMode, RequestId};
        let (stub, _) = crate::stub::settle(
            |n| {
                let stub =
                    crate::stub::StubServer::spawn_completion(crate::stub::chat_ok(NOOP, None));
                let url = stub.url();
                let mut adapter = make(&url);
                let mut ask = request(300 + u64::from(n), 3);
                let (kind, mode, caps) = if remote {
                    (
                        AdapterKind::Remote,
                        PrivacyMode::RemoteAllowed,
                        ProviderCapabilities::remote_http(),
                    )
                } else {
                    (
                        AdapterKind::Local,
                        PrivacyMode::LocalOnly,
                        ProviderCapabilities::local_http(),
                    )
                };
                ask.id = RequestId(300 + u64::from(n));
                ask.model.adapter = kind;
                ask.privacy = mode;
                let policy = AiSessionPolicy {
                    adapters: vec![kind],
                    mode,
                    destinations: if remote { vec![url] } else { Vec::new() },
                    scopes: Vec::new(),
                    request_limit: 8,
                    max_concurrency: 1,
                    spend_limit: None,
                    fallback: Fallback {
                        order: Vec::new(),
                        scripted_ok: true,
                    },
                };
                let outcome = crate::ProviderAdapter::call(&mut adapter, &ask, &policy, &caps);
                (stub, outcome)
            },
            |(stub, _)| !stub.bodies().is_empty(),
        );
        let bodies = stub.bodies();
        serde_json::from_str(&bodies[0]).expect("sent body is JSON")
    }

    fn custom_pins() -> RequestPins {
        RequestPins::default()
            .with_system("Answer in JSON.")
            .and_then(|p| p.with_temperature_milli(300))
            .and_then(|p| p.with_tools(vec![sample_tool("move")]))
            .expect("pins in bounds")
    }

    /// The pins reach the socket for every adapter, in the vendor's own
    /// field names: defaults when the caller sets none, the caller's when set.
    #[test]
    fn every_adapter_sends_the_pins_over_the_socket() {
        use crate::transport::parse_endpoint;
        use crate::vendor::VendorSecret;
        use crate::MockStep::Succeed;
        let end = |url: &str| parse_endpoint(url).expect("stub url parses");
        let secret = || VendorSecret::Direct(String::from("pins-token"));
        let kind = request(1, 3).kind;
        let sys = default_system_prompt(kind);
        for pins in [RequestPins::default(), custom_pins()] {
            let custom = pins != RequestPins::default();
            let (want_sys, want_t) = if custom {
                ("Answer in JSON.", 0.3)
            } else {
                (sys, 0.0)
            };
            let tool = |value: &Value, at: &str| {
                if custom {
                    assert_eq!(value.pointer(at), Some(&json!("move")), "{at}");
                } else {
                    assert!(value.get("tools").is_none(), "tools only when given");
                }
            };

            let chat_like = [
                (
                    "openai",
                    sent_body(true, |u| {
                        crate::OpenAiAdapter::new(vec![Succeed], end(u), secret())
                            .with_pins(pins.clone())
                    }),
                ),
                (
                    "remote",
                    sent_body(true, |u| {
                        crate::RemoteAdapter::new(vec![Succeed], end(u), secret())
                            .with_pins(pins.clone())
                    }),
                ),
                (
                    "local",
                    sent_body(false, |u| {
                        crate::LocalAdapter::new(vec![Succeed], end(u))
                            .expect("loopback")
                            .with_pins(pins.clone())
                    }),
                ),
            ];
            for (name, body) in &chat_like {
                assert_eq!(body["messages"][0]["role"], "system", "{name}");
                assert_eq!(body["messages"][0]["content"], want_sys, "{name}");
                assert_eq!(body["temperature"].as_f64(), Some(want_t), "{name}");
                assert_eq!(body["seed"], PINNED_SEED, "{name}");
                tool(body, "/tools/0/function/name");
            }

            let anthropic = sent_body(true, |u| {
                crate::AnthropicAdapter::new(vec![Succeed], end(u), secret())
                    .with_pins(pins.clone())
            });
            assert_eq!(anthropic["system"], want_sys, "anthropic");
            assert_eq!(anthropic["temperature"].as_f64(), Some(want_t), "anthropic");
            assert!(anthropic.get("seed").is_none(), "anthropic offers no seed");
            tool(&anthropic, "/tools/0/name");

            let gemini = sent_body(true, |u| {
                crate::GeminiAdapter::new(vec![Succeed], end(u), secret()).with_pins(pins.clone())
            });
            assert_eq!(
                gemini["systemInstruction"]["parts"][0]["text"], want_sys,
                "gemini"
            );
            assert_eq!(
                gemini["generationConfig"]["temperature"].as_f64(),
                Some(want_t)
            );
            assert_eq!(gemini["generationConfig"]["seed"], PINNED_SEED, "gemini");
            tool(&gemini, "/tools/0/functionDeclarations/0/name");

            let ollama = sent_body(false, |u| {
                crate::OllamaAdapter::new(vec![Succeed], end(u))
                    .expect("loopback")
                    .with_pins(pins.clone())
            });
            assert_eq!(ollama["messages"][0]["content"], want_sys, "ollama");
            assert_eq!(ollama["options"]["temperature"].as_f64(), Some(want_t));
            assert_eq!(ollama["options"]["seed"], PINNED_SEED, "ollama");
            tool(&ollama, "/tools/0/function/name");
        }
    }
}
