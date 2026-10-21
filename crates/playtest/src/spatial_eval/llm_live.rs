//! SPR4 live recorder: one real model over the eval-room door tools.
//!
//! Consumer: the operator who runs the ignored
//! [`spatial_llm_record_minimax_m3_1_flash_free`] test with `--features live`
//! and `OPENROUTER_API_KEY` set. It implements [`Model`] and hands the result
//! to [`llm::record`], so every row is an answer a provider actually sent.
//!
//! Two arms. `raw` is one chat completion with [`llm::prompt`] (today's
//! `scene.brief`). `verb` stages the door tools `scene.query` and
//! `scene.place` on the matching eval room, executes every call against the
//! room's integer-cm oracles in `world.rs` (the same rules the door's
//! `scene.query`/`scene.place` promise), feeds the JSON back and loops to a
//! final answer. Gold never enters a request or a tool result: a tool result
//! is computed from the room alone.
//!
//! The eval room has no staged revision, so `scene.place` answers a dry-run
//! pose check (`at` plus `footprint_cm`) and writes nothing; `scene.query`
//! serves the kinds the four question families need. Both shapes keep the
//! door tool names and field names. `free_space` and `cone` are refused with
//! the supported kind list rather than guessed.
//!
//! Determinism: recordings are real provider bytes, so only the harness
//! replay is deterministic. Failure mode: a provider refusal or quota error
//! is kept, later questions are skipped and the caller writes no cassette,
//! never a fabricated row.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engine_provider::transport::{parse_endpoint, post_json, Endpoint, TransportError};
use serde_json::{json, Value};

use super::llm::{self, Arm, Model, Question, Reply};
use super::world::{
    between_ids, cover_rows, largest_open, near_ids, place_verdict, reachable_rows, scene_of,
    seg_blocked, visible_rows, Actor, Rect, Scene,
};

/// Model slug recorded into the cassette header.
pub const MODEL: &str = "minimax/minimax-m3.1-flash:free";
/// File stem for [`MODEL`] under `fixtures/spatial-gold/llm/`.
pub const FILE_STEM: &str = "minimax-m3.1-flash-free";
/// Questions are answered by an agent loop; this bounds tool rounds.
const MAX_TOOL_ROUNDS: usize = 6;
/// Completion bound for a tool-call round (the model reasons before it calls).
const TOOL_TOKENS: u32 = 4096;
/// Completion bound for an answer. A reasoning model spends this same budget
/// on its private reasoning, so it needs room for the thoughts and the answer.
const ANSWER_TOKENS: u32 = 16384;
/// Cap on the model's private reasoning, in tokens. The cap is the same for
/// both arms, so the A/B stays fair.
const REASONING_TOKENS: u32 = 1024;
/// Provider attempts per request before the recording aborts. The network
/// here drops connections for a minute now and then, so the budget is wide.
const ATTEMPTS: u32 = 12;

/// One provider answer: the message, why the model stopped, the token
/// counts and how much reasoning text it spent.
struct Completion {
    message: Value,
    finish: String,
    reasoning_chars: usize,
    tokens_in: u32,
    tokens_out: u32,
}

/// Live-recording failure. Messages never carry the bearer token.
#[derive(Debug)]
pub enum LiveError {
    /// `OPENROUTER_API_KEY` is unset.
    NoKey(&'static str),
    /// Endpoint text did not parse.
    Endpoint(String),
    /// Socket, TLS or timeout failure after every retry.
    Transport(String),
    /// Provider answered with an error status or an error envelope.
    Api(String),
    /// A tool call was not valid JSON or named no staged object.
    Tool(String),
    /// Cassette or replay failure after the recording.
    Harness(String),
    /// File write failure.
    Io(String),
}

impl std::fmt::Display for LiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoKey(name) => write!(f, "{name} is unset; export the provider key first"),
            Self::Endpoint(why) => write!(f, "bad provider endpoint: {why}"),
            Self::Transport(why) => write!(f, "provider transport failed: {why}"),
            Self::Api(why) => write!(f, "provider refused the call: {why}"),
            Self::Tool(why) => write!(f, "tool call failed: {why}"),
            Self::Harness(why) => write!(f, "cassette harness failed: {why}"),
            Self::Io(why) => write!(f, "cassette write failed: {why}"),
        }
    }
}

impl std::error::Error for LiveError {}

impl From<TransportError> for LiveError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error.to_string())
    }
}

/// A refused tool call. Typed so the library never reports `Result<_, String>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolError(String);

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ToolError {
    fn from(why: String) -> Self {
        Self(why)
    }
}

fn tool_err<T>(why: impl Into<String>) -> Result<T, ToolError> {
    Err(ToolError(why.into()))
}

/// OpenRouter chat client that answers as a [`Model`] under both arms.
pub struct OpenRouter {
    endpoint: Endpoint,
    key: String,
    model: String,
    /// First live failure, kept so the caller refuses to write a cassette.
    failure: RefCell<Option<String>>,
    /// Optional operator trace (requests, tool calls, answers); never written
    /// to the cassette.
    trace: RefCell<Vec<String>>,
    /// Optional file the trace is appended to, so a long run can be watched.
    trace_path: RefCell<Option<PathBuf>>,
}

impl OpenRouter {
    /// Read `OPENROUTER_API_KEY`, the OpenRouter base URL and one model.
    pub fn from_env(model: &str) -> Result<Self, LiveError> {
        let key = std::env::var("OPENROUTER_API_KEY")
            .map_err(|_| LiveError::NoKey("OPENROUTER_API_KEY"))?;
        if key.trim().is_empty() {
            return Err(LiveError::NoKey("OPENROUTER_API_KEY"));
        }
        let endpoint = parse_endpoint("https://openrouter.ai/api/v1/chat/completions")
            .map_err(|error| LiveError::Endpoint(error.to_string()))?;
        Ok(Self {
            endpoint,
            key,
            model: model.to_owned(),
            failure: RefCell::new(None),
            trace: RefCell::new(Vec::new()),
            trace_path: RefCell::new(None),
        })
    }

    /// Append the trace to this file as well, so a long recording can be
    /// watched while it runs. The file holds provider traffic only; the
    /// bearer token never enters it.
    pub fn set_trace_path(&self, path: PathBuf) {
        *self.trace_path.borrow_mut() = Some(path);
    }

    /// The first failure the model hit, if any. A failed run writes nothing.
    pub fn failure(&self) -> Option<String> {
        self.failure.borrow().clone()
    }

    /// Drain the operator trace for this run.
    pub fn take_trace(&self) -> Vec<String> {
        std::mem::take(&mut *self.trace.borrow_mut())
    }

    fn note(&self, line: String) {
        self.trace.borrow_mut().push(line.clone());
        let path = self.trace_path.borrow().clone();
        let Some(path) = path else {
            return;
        };
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
    }

    /// One chat completion plus the provider's usage counts.
    fn chat(&self, body: &Value) -> Result<Completion, LiveError> {
        let text =
            serde_json::to_string(body).map_err(|error| LiveError::Api(error.to_string()))?;
        let mut base_wait = 2_000_u64;
        let mut last = String::from("no attempt ran");
        for _ in 0..ATTEMPTS {
            match post_json(&self.endpoint, &text, Some(&self.key), 30_000) {
                Ok(response) if response.status == 200 => {
                    let body = dechunk(&response.body);
                    let Ok(value) = serde_json::from_str::<Value>(&body) else {
                        // A truncated body (the peer closed mid-answer) is a
                        // transport fault, so it is retried, not a refusal.
                        last = format!("body is not JSON ({} bytes)", body.len());
                        std::thread::sleep(Duration::from_millis(base_wait));
                        base_wait = (base_wait * 2).min(60_000);
                        continue;
                    };
                    if let Some(error) = value.get("error") {
                        return Err(LiveError::Api(error_text(error)));
                    }
                    let choice = value
                        .get("choices")
                        .and_then(Value::as_array)
                        .and_then(|choices| choices.first())
                        .cloned()
                        .ok_or_else(|| LiveError::Api("answer has no choices[0]".to_owned()))?;
                    let message = choice.get("message").cloned().ok_or_else(|| {
                        LiveError::Api("answer has no choices[0].message".to_owned())
                    })?;
                    let finish = choice
                        .get("finish_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let reasoning_chars = message
                        .get("reasoning")
                        .and_then(Value::as_str)
                        .map(str::len)
                        .unwrap_or(0);
                    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
                    let tokens_in = usage
                        .get("prompt_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let tokens_out = usage
                        .get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let tokens_in = u32::try_from(tokens_in).unwrap_or(u32::MAX);
                    let tokens_out = u32::try_from(tokens_out).unwrap_or(u32::MAX);
                    return Ok(Completion {
                        message,
                        finish,
                        reasoning_chars,
                        tokens_in,
                        tokens_out,
                    });
                }
                Ok(response) if response.status == 429 || response.status >= 500 => {
                    let wait = response.retry_after_ms.unwrap_or(base_wait).max(500);
                    last = format!(
                        "status {}: {}",
                        response.status,
                        error_text_of(&response.body)
                    );
                    self.note(format!("retry after {wait} ms: {last}"));
                    std::thread::sleep(Duration::from_millis(wait.min(60_000)));
                    base_wait = (base_wait * 2).min(60_000);
                }
                Ok(response) => {
                    return Err(LiveError::Api(format!(
                        "status {}: {}",
                        response.status,
                        error_text_of(&response.body)
                    )));
                }
                Err(error) => {
                    last = error.to_string();
                    self.note(format!("retry after {base_wait} ms: {last}"));
                    std::thread::sleep(Duration::from_millis(base_wait));
                    base_wait = (base_wait * 2).min(60_000);
                }
            }
        }
        Err(LiveError::Transport(last))
    }

    /// Request body for one completion. `tools` adds the two door tools.
    fn body(&self, messages: &[Value], tools: bool, max_tokens: u32) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": 0,
            "reasoning": { "max_tokens": REASONING_TOKENS },
        });
        if tools {
            body["tools"] = Value::Array(tool_defs());
            body["tool_choice"] = Value::String("auto".to_owned());
        }
        body
    }

    /// One completion: its usage joins the arm totals and its answer text is
    /// clipped into the operator trace.
    fn round(
        &self,
        label: &str,
        round: usize,
        messages: &[Value],
        tools: bool,
        max_tokens: u32,
        tokens: &mut (u32, u32),
    ) -> Result<(Completion, String), LiveError> {
        let completion = self.chat(&self.body(messages, tools, max_tokens))?;
        tokens.0 = tokens.0.saturating_add(completion.tokens_in);
        tokens.1 = tokens.1.saturating_add(completion.tokens_out);
        let content = completion
            .message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        self.note(format!(
            "round {round} {label} in={} out={} finish={} reasoning={} content={:?}",
            completion.tokens_in,
            completion.tokens_out,
            completion.finish,
            completion.reasoning_chars,
            clip(&content, 200)
        ));
        Ok((completion, content))
    }

    fn run(&self, arm: Arm, q: &Question) -> Result<Reply, LiveError> {
        let scene = scene_of(q.scene)
            .ok_or_else(|| LiveError::Tool(format!("question {} names no eval room", q.id)))?;
        let mut messages = vec![json!({
            "role": "user",
            "content": llm::prompt(arm, q),
        })];
        let mut tokens = (0_u32, 0_u32);
        // Raw arm: exactly one completion on today's scene.brief text.
        if arm == Arm::Raw {
            let (_, answer) =
                self.round("answer", 0, &messages, false, ANSWER_TOKENS, &mut tokens)?;
            return Ok(Reply {
                calls: 0,
                tokens_in: tokens.0,
                tokens_out: tokens.1,
                answer,
            });
        }
        // Verb arm: tool rounds against the eval room, then one final answer.
        let mut calls = 0_u32;
        for round in 0..MAX_TOOL_ROUNDS {
            let (completion, content) =
                self.round("tools", round, &messages, true, TOOL_TOKENS, &mut tokens)?;
            let round_calls = parse_calls(&completion.message)?;
            if round_calls.is_empty() {
                if !content.is_empty() {
                    return Ok(reply(calls, tokens, content));
                }
                break;
            }
            messages.push(assistant_turn(&completion.message));
            for call in round_calls {
                calls = calls.saturating_add(1);
                let result = dispatch(scene, &call.name, &call.arguments);
                self.note(format!(
                    "{} {} -> {}",
                    call.name,
                    clip(&call.arguments, 200),
                    clip(&result, 300)
                ));
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": result,
                }));
            }
        }
        messages.push(json!({
            "role": "user",
            "content": "Tool rounds are spent. Reply with the final answer only.",
        }));
        let (_, answer) = self.round(
            "answer",
            MAX_TOOL_ROUNDS,
            &messages,
            false,
            ANSWER_TOKENS,
            &mut tokens,
        )?;
        Ok(reply(calls, tokens, answer))
    }
}

fn reply(calls: u32, tokens: (u32, u32), answer: String) -> Reply {
    Reply {
        calls,
        tokens_in: tokens.0,
        tokens_out: tokens.1,
        answer,
    }
}

impl Model for OpenRouter {
    fn source(&self) -> &'static str {
        "model"
    }

    fn ask(&self, arm: Arm, q: &Question) -> Reply {
        if self.failure.borrow().is_some() {
            return Reply {
                calls: 0,
                tokens_in: 0,
                tokens_out: 0,
                answer: String::new(),
            };
        }
        match self.run(arm, q) {
            Ok(reply) => reply,
            Err(error) => {
                *self.failure.borrow_mut() = Some(format!(
                    "live recording stopped at {} {}: {error}",
                    arm.as_str(),
                    q.id
                ));
                Reply {
                    calls: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                    answer: String::new(),
                }
            }
        }
    }
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn error_text(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string())
}

fn error_text_of(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").map(error_text))
        .unwrap_or_else(|| body.chars().take(300).collect())
}

/// Undo HTTP/1.1 chunked framing. The provider client returns the raw body
/// bytes when the server sends `Transfer-Encoding: chunked` (OpenRouter does);
/// a plain JSON body is returned unchanged.
fn dechunk(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut at = 0_usize;
    let mut out: Vec<u8> = Vec::new();
    loop {
        let Some(line_end) = bytes[at..].windows(2).position(|pair| pair == b"\r\n") else {
            return body.to_owned();
        };
        let line_end = at + line_end;
        let Ok(size_text) = std::str::from_utf8(&bytes[at..line_end]) else {
            return body.to_owned();
        };
        let Ok(size) = usize::from_str_radix(size_text.trim(), 16) else {
            return body.to_owned();
        };
        at = line_end + 2;
        if size == 0 {
            break;
        }
        let Some(end) = at.checked_add(size) else {
            return body.to_owned();
        };
        if end > bytes.len() {
            return body.to_owned();
        }
        out.extend_from_slice(&bytes[at..end]);
        at = end;
        if bytes.get(at..at + 2) == Some(b"\r\n") {
            at += 2;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| body.to_owned())
}

struct Call {
    id: String,
    name: String,
    arguments: String,
}

fn parse_calls(message: &Value) -> Result<Vec<Call>, LiveError> {
    let Some(list) = message.get("tool_calls").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for call in list {
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| LiveError::Tool("tool call has no id".to_owned()))?;
        let function = call
            .get("function")
            .ok_or_else(|| LiveError::Tool("tool call has no function".to_owned()))?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| LiveError::Tool("tool call has no name".to_owned()))?;
        let arguments = match function.get("arguments") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => "{}".to_owned(),
        };
        out.push(Call {
            id: id.to_owned(),
            name: name.to_owned(),
            arguments,
        });
    }
    Ok(out)
}

fn assistant_turn(message: &Value) -> Value {
    // A reasoning model can answer with reasoning only and no content; an
    // assistant turn with null content is refused by the provider, so the
    // empty string stands in for it.
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut turn = json!({
        "role": "assistant",
        "content": content,
    });
    if let Some(calls) = message.get("tool_calls") {
        turn["tool_calls"] = calls.clone();
    }
    turn
}

fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "scene.query",
                "description": "Spatial questions with reasons over the staged eval room. Read-only. Objects are labeled prop_NNNN (prop_0000..prop_0003 are the four boundary walls; every other label is a crate).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["relation", "sees", "near", "between", "visible", "reachable", "cover_near", "largest_open"],
                        },
                        "anchor": {"type": "string", "description": "Subject object label, e.g. prop_0021"},
                        "other": {"type": "string", "description": "Second object label (sees, between)"},
                        "relation": {
                            "type": "string",
                            "enum": ["left_of", "right_of", "in_front_of", "behind", "visible_from", "reachable_from", "near"],
                        },
                        "facing": {
                            "type": "string",
                            "enum": ["east", "north-east", "north", "north-west", "west", "south-west", "south", "south-east"],
                            "description": "Direction the anchor object stands facing (relation left_of/right_of/in_front_of/behind)",
                        },
                        "radius_cm": {"type": "integer", "description": "Search radius in cm (near, cover_near)"},
                        "eye_cm": {
                            "type": "array",
                            "items": {"type": "integer"},
                            "minItems": 2,
                            "maxItems": 2,
                            "description": "Plan point [x,z] in cm (visible eye, cover_near threat)",
                        },
                        "crouch": {"type": "boolean"},
                        "capsule_radius_cm": {"type": "integer"},
                    },
                    "required": ["kind"],
                    "additionalProperties": false,
                },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "scene.place",
                "description": "Dry-run one placement pose in the staged eval room. Never writes. Returns clean=true when the footprint clears every object and wall, or the labels it collides with.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "object": {"type": "string", "description": "Optional label of the planned object"},
                        "at": {
                            "type": "array",
                            "items": {"type": "integer"},
                            "minItems": 2,
                            "maxItems": 2,
                            "description": "Centre [x,z] in cm",
                        },
                        "footprint_cm": {
                            "type": "array",
                            "items": {"type": "integer"},
                            "minItems": 2,
                            "maxItems": 2,
                            "description": "Full x and z size in cm (default couch 200 x 90)",
                        },
                        "dry_run": {"type": "boolean", "description": "Must stay true; the eval room is never edited"},
                    },
                    "required": ["at"],
                    "additionalProperties": false,
                },
            },
        }),
    ]
}

fn dispatch(scene: &'static Scene, name: &str, arguments: &str) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
    let result = match name {
        "scene.query" => query_json(scene, &args),
        "scene.place" => place_json(scene, &args),
        other => tool_err(format!(
            "unknown tool `{other}`; tools are scene.query and scene.place"
        )),
    };
    match result {
        Ok(text) => text,
        Err(why) => json!({ "error": why.to_string() }).to_string(),
    }
}

fn text_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError(format!("missing `{name}` (string)")))
}

fn int_arg(args: &Value, name: &str) -> Option<i32> {
    args.get(name)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn point_arg(args: &Value, name: &str) -> Result<(i32, i32), ToolError> {
    let pair = args
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError(format!("missing `{name}` (two integers)")))?;
    if pair.len() != 2 {
        return tool_err(format!("`{name}` needs exactly two integers"));
    }
    let one = |index: usize| {
        pair[index]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| ToolError(format!("`{name}` needs two integers")))
    };
    Ok((one(0)?, one(1)?))
}

fn labels_of(scene: &Scene, ids: &[u32]) -> Vec<String> {
    ids.iter().map(|id| llm::label(scene, *id)).collect()
}

fn actor_arg(scene: &Scene, args: &Value, name: &str) -> Result<u32, ToolError> {
    let text = text_arg(args, name)?;
    parse_actor(scene, text).ok_or_else(|| {
        ToolError(format!(
            "unknown object `{text}` for `{name}`; staged labels are prop_0000..prop_{:04}",
            scene.actors.len().saturating_sub(1)
        ))
    })
}

fn parse_actor(scene: &Scene, text: &str) -> Option<u32> {
    if let Some(digits) = text.strip_prefix("prop_") {
        if let Ok(index) = digits.parse::<usize>() {
            return scene.actors.get(index).map(|actor| actor.id);
        }
    }
    let id = text.parse::<u32>().ok()?;
    scene.actors.iter().any(|a| a.id == id).then_some(id)
}

fn actor_by_id(scene: &Scene, id: u32) -> Option<&Actor> {
    scene.actors.iter().find(|a| a.id == id)
}

const COMPASS: [(&str, (i32, i32), u8); 8] = [
    ("east", (1, 0), 0),
    ("north-east", (1, 1), 1),
    ("north", (0, 1), 2),
    ("north-west", (-1, 1), 3),
    ("west", (-1, 0), 4),
    ("south-west", (-1, -1), 5),
    ("south", (0, -1), 6),
    ("south-east", (1, -1), 7),
];

fn compass(word: &str) -> Option<(i32, i32, u8)> {
    let flat = word.trim().to_lowercase().replace('_', "-");
    let flat = match flat.as_str() {
        "n" => "north".to_owned(),
        "ne" => "north-east".to_owned(),
        "e" => "east".to_owned(),
        "se" => "south-east".to_owned(),
        "s" => "south".to_owned(),
        "sw" => "south-west".to_owned(),
        "w" => "west".to_owned(),
        "nw" => "north-west".to_owned(),
        other => other.to_owned(),
    };
    COMPASS
        .iter()
        .find(|(name, _, _)| *name == flat.as_str())
        .map(|(_, dir, oct)| (dir.0, dir.1, *oct))
}

fn facing_arg(args: &Value) -> Result<(i32, i32), ToolError> {
    let word = text_arg(args, "facing")?;
    compass(word)
        .map(|(fx, fz, _)| (fx, fz))
        .ok_or_else(|| ToolError(format!("unknown facing `{word}`; pass a compass word")))
}

/// Strict side/dot test with the same cross convention as the eval gold
/// (`+x` east, `+z` north; facing north puts west on the left).
fn directional(scene: &Scene, anchor: u32, relation: &str, fx: i32, fz: i32) -> Vec<u32> {
    let Some(me) = actor_by_id(scene, anchor) else {
        return Vec::new();
    };
    llm::crates_of(scene)
        .into_iter()
        .filter(|id| *id != anchor)
        .filter(|id| {
            let Some(other) = actor_by_id(scene, *id) else {
                return false;
            };
            let dx = other.x_cm - me.x_cm;
            let dz = other.z_cm - me.z_cm;
            match relation {
                "left_of" => fx * dz - fz * dx > 0,
                "right_of" => fx * dz - fz * dx < 0,
                "in_front_of" => dx * fx + dz * fz > 0,
                "behind" => dx * fx + dz * fz < 0,
                _ => false,
            }
        })
        .collect()
}

/// Same 2D line test as the gold: any other actor rect blocks the segment.
fn sees(scene: &Scene, a: u32, b: u32) -> bool {
    let (Some(pa), Some(pb)) = (actor_by_id(scene, a), actor_by_id(scene, b)) else {
        return false;
    };
    let blockers: Vec<Rect> = scene
        .actors
        .iter()
        .filter(|actor| actor.id != a && actor.id != b)
        .map(Actor::rect)
        .collect();
    !seg_blocked((pa.x_cm, pa.z_cm), (pb.x_cm, pb.z_cm), &blockers)
}

fn blocker_of(scene: &Scene, a: u32, b: u32) -> Option<u32> {
    let (pa, pb) = (actor_by_id(scene, a)?, actor_by_id(scene, b)?);
    scene
        .actors
        .iter()
        .filter(|actor| actor.id != a && actor.id != b)
        .find(|actor| seg_blocked((pa.x_cm, pa.z_cm), (pb.x_cm, pb.z_cm), &[actor.rect()]))
        .map(|actor| actor.id)
}

fn objects_json(scene: &Scene, ids: &[u32]) -> Value {
    Value::Array(
        ids.iter()
            .filter_map(|id| actor_by_id(scene, *id))
            .map(|actor| {
                json!({
                    "object": llm::label(scene, actor.id),
                    "x_cm": actor.x_cm,
                    "z_cm": actor.z_cm,
                })
            })
            .collect(),
    )
}

fn query_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    match text_arg(args, "kind")? {
        "relation" => relation_json(scene, args),
        "sees" => sees_json(scene, args),
        "near" => near_json(scene, args),
        "between" => between_json(scene, args),
        "visible" => visible_json(scene, args),
        "reachable" => reachable_json(scene, args),
        "cover_near" => cover_json(scene, args),
        "largest_open" => largest_json(scene, args),
        other => tool_err(format!(
            "kind `{other}` is not staged in the eval room; pass relation, sees, near, between, visible, reachable, cover_near or largest_open"
        )),
    }
}

fn relation_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let anchor = actor_arg(scene, args, "anchor")?;
    let relation = text_arg(args, "relation")?;
    let ids = match relation {
        "left_of" | "right_of" | "in_front_of" | "behind" => {
            let (fx, fz) = facing_arg(args)?;
            directional(scene, anchor, relation, fx, fz)
        }
        "visible_from" => llm::crates_of(scene)
            .into_iter()
            .filter(|id| *id != anchor && sees(scene, anchor, *id))
            .collect(),
        "reachable_from" => reachable_rows(scene, anchor)
            .into_iter()
            .filter(|row| row.reason == "reachable")
            .map(|row| row.id)
            .collect(),
        "near" => near_ids(scene, anchor, int_arg(args, "radius_cm").unwrap_or(500)),
        other => {
            return tool_err(format!(
                "relation `{other}` is not staged; pass left_of, right_of, in_front_of, behind, visible_from, reachable_from or near"
            ))
        }
    };
    Ok(json!({
        "kind": "relation",
        "relation": relation,
        "anchor": llm::label(scene, anchor),
        "ids": labels_of(scene, &ids),
        "count": ids.len(),
        "objects": objects_json(scene, &ids),
    })
    .to_string())
}

fn sees_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let anchor = actor_arg(scene, args, "anchor")?;
    let other = actor_arg(scene, args, "other")?;
    let visible = sees(scene, anchor, other);
    Ok(json!({
        "kind": "sees",
        "anchor": llm::label(scene, anchor),
        "other": llm::label(scene, other),
        "visible": visible,
        "blocker": if visible { Value::Null } else { json!(blocker_of(scene, anchor, other).map(|id| llm::label(scene, id))) },
    })
    .to_string())
}

fn near_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let anchor = actor_arg(scene, args, "anchor")?;
    let ids = near_ids(scene, anchor, int_arg(args, "radius_cm").unwrap_or(10_000));
    Ok(json!({
        "kind": "near",
        "anchor": llm::label(scene, anchor),
        "ids": labels_of(scene, &ids),
        "count": ids.len(),
        "objects": objects_json(scene, &ids),
    })
    .to_string())
}

fn between_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let anchor = actor_arg(scene, args, "anchor")?;
    let other = actor_arg(scene, args, "other")?;
    let ids = between_ids(scene, anchor, other);
    Ok(json!({
        "kind": "between",
        "anchor": llm::label(scene, anchor),
        "other": llm::label(scene, other),
        "ids": labels_of(scene, &ids),
        "count": ids.len(),
    })
    .to_string())
}

fn visible_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let eye = point_arg(args, "eye_cm")?;
    let word = text_arg(args, "facing")?;
    let (_, _, oct) = compass(word)
        .ok_or_else(|| ToolError(format!("unknown facing `{word}`; pass a compass word")))?;
    let rows = visible_rows(scene, eye, oct);
    let visible: Vec<String> = rows
        .iter()
        .filter(|row| row.reason == "visible")
        .map(|row| llm::label(scene, row.id))
        .collect();
    Ok(json!({
        "kind": "visible",
        "eye_cm": [eye.0, eye.1],
        "visible": visible,
        "rows": rows.iter().map(|row| json!({
            "object": llm::label(scene, row.id),
            "reason": row.reason,
        })).collect::<Vec<_>>(),
    })
    .to_string())
}

fn reachable_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let anchor = actor_arg(scene, args, "anchor")?;
    let rows = reachable_rows(scene, anchor);
    Ok(json!({
        "kind": "reachable",
        "anchor": llm::label(scene, anchor),
        "reachable": rows.iter().filter(|row| row.reason == "reachable").map(|row| llm::label(scene, row.id)).collect::<Vec<_>>(),
        "rows": rows.iter().map(|row| json!({
            "object": llm::label(scene, row.id),
            "reason": row.reason,
        })).collect::<Vec<_>>(),
    })
    .to_string())
}

fn cover_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let anchor = actor_arg(scene, args, "anchor")?;
    let threat = point_arg(args, "eye_cm")?;
    let crouch = args.get("crouch").and_then(Value::as_bool).unwrap_or(false);
    let radius = int_arg(args, "radius_cm").unwrap_or(500);
    let rows = cover_rows(scene, anchor, threat, crouch, radius);
    Ok(json!({
        "kind": "cover_near",
        "anchor": llm::label(scene, anchor),
        "crouch": crouch,
        "rows": rows.iter().map(|row| json!({
            "object": llm::label(scene, row.id),
            "reason": row.reason,
        })).collect::<Vec<_>>(),
    })
    .to_string())
}

fn largest_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let capsule = int_arg(args, "capsule_radius_cm").unwrap_or(30);
    Ok(match largest_open(scene, 100, 45, capsule) {
        Some((x, z, gap)) => json!({
            "kind": "largest_open",
            "clean": true,
            "at": [x, z],
            "clearance_cm": gap,
        })
        .to_string(),
        None => json!({
            "kind": "largest_open",
            "clean": false,
            "reason": "no spot fits the couch footprint plus capsule",
        })
        .to_string(),
    })
}

fn place_json(scene: &'static Scene, args: &Value) -> Result<String, ToolError> {
    let (x, z) = point_arg(args, "at")?;
    let (w, d) = match args.get("footprint_cm") {
        Some(_) => point_arg(args, "footprint_cm")?,
        None => (200, 90),
    };
    if w <= 0 || d <= 0 {
        return tool_err("footprint_cm needs two positive sizes");
    }
    let (hx, hz) = (w / 2, d / 2);
    let (clean, hits) = place_verdict(scene, x, z, hx, hz);
    let outside = x - hx < scene.min_cm
        || x + hx > scene.max_cm
        || z - hz < scene.min_cm
        || z + hz > scene.max_cm;
    Ok(json!({
        "kind": "place",
        "clean": clean,
        "at": [x, z],
        "footprint_cm": [w, d],
        "conflicts": labels_of(scene, &hits),
        "outside_bounds": outside,
        "dry_run": true,
        "object": args.get("object").and_then(Value::as_str).map(str::to_owned),
    })
    .to_string())
}

/// Record one live model over every seeded question and both arms and write
/// the cassette under `fixtures/spatial-gold/llm/`. A failed model call
/// writes nothing and returns the first failure.
pub fn record_and_write(model: &str, file_stem: &str) -> Result<String, LiveError> {
    record_and_write_with(model, file_stem, None, &|_| {})
}

/// Record one live model and report every answered question to `on_progress`
/// (the arm, the question id and the provider trace so far). The trace is
/// appended to `trace_path` when given, so a long run can be watched. A
/// failed model call writes nothing and returns the first failure.
pub fn record_and_write_with(
    model: &str,
    file_stem: &str,
    trace_path: Option<&Path>,
    on_progress: &dyn Fn(&str),
) -> Result<String, LiveError> {
    let agent = OpenRouter::from_env(model)?;
    if let Some(path) = trace_path {
        agent.set_trace_path(path.to_path_buf());
    }
    let questions = llm::questions();
    let cassette = llm::record_with_progress(model, &agent, &questions, &|arm, id| {
        let mut line = format!("{arm} {id} answered");
        for note in agent.take_trace() {
            line.push('\n');
            line.push_str(&note);
        }
        on_progress(&line);
    });
    if let Some(why) = agent.failure() {
        return Err(LiveError::Api(why));
    }
    if cassette.source != "model" {
        return Err(LiveError::Harness(format!(
            "recorded source is `{}`, not `model`",
            cassette.source
        )));
    }
    let path = fixture_path(file_stem);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| LiveError::Io(error.to_string()))?;
    }
    std::fs::write(&path, cassette.to_text()).map_err(|error| LiveError::Io(error.to_string()))?;
    let replay = llm::replay(&cassette, &questions)
        .map_err(|error| LiveError::Harness(error.to_string()))?;
    Ok(replay.to_table())
}

/// Cassette path for one file stem.
pub fn fixture_path(file_stem: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/spatial-gold/llm")
        .join(format!("{file_stem}.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dechunk_reads_a_framed_body_and_leaves_plain_json_alone() {
        let framed = "7\r\n{\"a\":1}\r\n0\r\n\r\n";
        assert_eq!(dechunk(framed), "{\"a\":1}");
        assert_eq!(dechunk("{\"a\":1}"), "{\"a\":1}");
    }

    #[test]
    fn tools_are_the_two_door_names_and_parse() {
        let defs = tool_defs();
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|def| def["function"]["name"].as_str())
            .collect();
        assert_eq!(names, ["scene.query", "scene.place"]);
    }

    #[test]
    fn query_answers_the_pillar_room_without_gold() {
        let scene = scene_of("room_pillar").expect("room");
        let args = json!({
            "kind": "relation",
            "relation": "left_of",
            "anchor": "prop_0006",
            "facing": "north",
        });
        let out: Value =
            serde_json::from_str(&query_json(scene, &args).expect("query")).expect("json");
        let ids: Vec<&str> = out["ids"]
            .as_array()
            .expect("ids")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            ids.contains(&"prop_0005"),
            "west is left when facing north: {ids:?}"
        );
        assert!(!ids.contains(&"prop_0004"), "east is right: {ids:?}");
        let place: Value =
            serde_json::from_str(&place_json(scene, &json!({ "at": [0, 0] })).expect("place"))
                .expect("json");
        assert_eq!(place["clean"], false);
    }

    #[test]
    fn unknown_labels_refuse_with_the_legal_range() {
        let scene = scene_of("room_open").expect("room");
        let error = actor_arg(scene, &json!({ "anchor": "prop_9999" }), "anchor").unwrap_err();
        assert!(error.0.contains("prop_0000..prop_0007"), "{error}");
    }

    #[test]
    fn place_checks_the_couch_footprint() {
        let scene = scene_of("room_open").expect("room");
        let out: Value = serde_json::from_str(
            &place_json(scene, &json!({ "at": [-300, -300] })).expect("place"),
        )
        .expect("json");
        assert_eq!(out["clean"], false);
        assert!(out["conflicts"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty()));
    }
}

#[test]
#[ignore = "live network smoke: one raw and one verb question, writes no fixture"]
fn spatial_llm_live_smoke() {
    let agent = OpenRouter::from_env(MODEL).expect("key and endpoint");
    let questions = llm::questions();
    for (arm, index) in [(Arm::Raw, 0_usize), (Arm::Verb, 4_usize)] {
        let q = &questions[index];
        let reply = agent.ask(arm, q);
        println!(
            "smoke {} {} calls={} in={} out={} answer={:?}",
            arm.as_str(),
            q.id,
            reply.calls,
            reply.tokens_in,
            reply.tokens_out,
            reply.answer
        );
    }
}

#[test]
#[ignore = "live network + OPENROUTER_API_KEY: records fixtures/spatial-gold/llm/minimax-m3.1-flash-free.tsv"]
fn spatial_llm_record_minimax_m3_1_flash_free() {
    use std::cell::Cell;

    struct Tee<'a> {
        inner: &'a OpenRouter,
        seen: Cell<u32>,
    }

    impl Model for Tee<'_> {
        fn source(&self) -> &'static str {
            "model"
        }

        fn ask(&self, arm: Arm, q: &Question) -> Reply {
            let reply = self.inner.ask(arm, q);
            let n = self.seen.get() + 1;
            self.seen.set(n);
            println!(
                "[{n}/100] {} {} calls={} in={} out={} answer={:?}",
                arm.as_str(),
                q.id,
                reply.calls,
                reply.tokens_in,
                reply.tokens_out,
                reply.answer
            );
            reply
        }
    }

    let agent = OpenRouter::from_env(MODEL).expect("key and endpoint");
    let questions = llm::questions();
    let tee = Tee {
        inner: &agent,
        seen: Cell::new(0),
    };
    let cassette = llm::record(MODEL, &tee, &questions);
    if let Some(why) = agent.failure() {
        panic!("{why}");
    }
    let path = fixture_path(FILE_STEM);
    std::fs::write(&path, cassette.to_text()).expect("write cassette");
    println!("wrote {}", path.display());
    let table = llm::replay(&cassette, &questions)
        .expect("replay the fresh cassette")
        .to_table();
    println!("{table}");
}
