//! `anim` pack: animation data an AI can edit.
//!
//! The state machine verbs (`anim.graph.set`, `anim.graph.undo`,
//! `anim.graph.get`, `anim.preview`) wrap `engine_anim::state_machine`: a
//! versioned JSON machine (states, transitions with switch mode, condition,
//! fade ticks; integer ticks only) that is validated before it is kept, undoable,
//! and previewable on a scripted tape without being applied. The machine lives
//! per door session in a side table keyed by the session id, so the core door
//! state is untouched; a refused set keeps the last good graph.
//!
//! The clip edit verbs (`anim.trim`, `anim.loop`, `anim.retime`, `anim.mirror`,
//! `anim.cleanup`) and the strip verbs join here later as `SyncTool<McpServer>`
//! types plus one `pack.add(VerbSpec::of::<Tool>(NAME, PACK, card)?)` line each.
//! Errors: [`McpToolError`] with the machine's own code (`bad_document`,
//! `invalid_machine`, `no_graph`, `bad_script`, `preview_failed`,
//! `nothing_to_undo`), the document path and one recovery step.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use engine_anim::state_machine::anim_door::{AnimDoor, DoorRefusal};
use rmcp::schemars::JsonSchema;
use rmcp::serde::de::DeserializeOwned;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "anim";
/// Verb: validate a JSON state machine and make it current (undoable).
pub const TOOL_ANIM_GRAPH_SET: &str = "anim.graph.set";
/// Verb: undo the last `anim.graph.set`.
pub const TOOL_ANIM_GRAPH_UNDO: &str = "anim.graph.undo";
/// Verb: read the current state machine as canonical JSON.
pub const TOOL_ANIM_GRAPH_GET: &str = "anim.graph.get";
/// Verb: run a scripted tape on a graph without applying it.
pub const TOOL_ANIM_PREVIEW: &str = "anim.preview";

/// Args for `anim.graph.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphSetArgs {
    /// The machine: `{"version":1,"name":..,"start":..,"params":[{"name","default","min","max"}],
    /// "states":[{"name","clip"|"node","length_ticks","looping"}],
    /// "transitions":[{"from","to","switch":"immediate|sync|at_end","advance":"disabled|enabled|auto",
    /// "when":"always"|{"param","op","value"}|{"time_ge":n}|{"not":c}|{"all":[..]}|{"any":[..]},
    /// "fade_ticks","curve":"linear|ease_in|ease_out|ease_in_out","priority","reset"}]}`.
    /// Integers only; unknown fields are refused with their path.
    pub graph: Value,
}

/// Result of `anim.graph.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphSetOutput {
    /// Digest (16 hex digits) before, null for the first graph.
    pub before: Option<String>,
    /// Digest (16 hex digits) after.
    pub after: String,
    /// State count.
    pub states: u32,
    /// Transition count.
    pub transitions: u32,
    /// Non-fatal findings (unreachable states, dead ends).
    pub lints: Vec<String>,
}

/// Args for the verbs that take nothing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(clippy::empty_structs_with_brackets, reason = "serde reads the wire shape `{}`; a unit struct reads null")]
pub struct NoArgs {}

/// Result of `anim.graph.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphUndoOutput {
    /// Digest (16 hex digits) after the undo.
    pub after: String,
    /// Edits still undoable.
    pub undo_depth: u32,
}

/// Result of `anim.graph.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphGetOutput {
    /// The current machine, null before the first set.
    pub graph: Option<Value>,
}

/// Args for `anim.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewArgs {
    /// A candidate machine to run instead of the current one (not applied).
    #[serde(default)]
    pub graph: Option<Value>,
    /// The tape: `{"ticks":N,"params":[{"at":T,"name":..,"value":..}],"travel":[{"at":T,"to":..}]}`;
    /// at most 10000 ticks; writes and travel requests apply before tick `at` runs.
    pub script: Value,
}

/// One transition seen in a preview.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewEvent {
    /// Tick the transition started on.
    pub tick: u32,
    /// State left.
    pub from: String,
    /// State entered.
    pub to: String,
    /// Cross-fade length in ticks.
    pub fade_ticks: u32,
}

/// Result of `anim.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewOutput {
    /// Every transition in order.
    pub events: Vec<PreviewEvent>,
    /// State at the end of the tape.
    pub final_state: String,
    /// Playback digest (16 hex digits): equal on every replay.
    pub digest: String,
}

fn refuse(refusal: DoorRefusal) -> McpToolError {
    McpToolError::new(
        refusal.code,
        format!("{} Next: {}.", refusal.message, refusal.recovery),
    )
}

/// Pre-apply cost caps for one `anim.graph.set`: hostile graph sets refuse
/// before the door mutates anything. Caps sit well under the underlying
/// `engine_anim` limits (128 states, 1024 transitions, 600 fade ticks per
/// transition, 1MiB JSON) so legal fighter graphs (2-4 states) pass with
/// headroom while giant sets fail cheap without touching the door.
const MAX_GRAPH_STATES: u32 = 32;
/// Most transitions one pre-apply graph may hold.
const MAX_GRAPH_TRANSITIONS: u32 = 64;
/// Most summed `fade_ticks` over all transitions, in ticks.
const MAX_GRAPH_FADE_SUM_TICKS: u64 = 4_800;
/// Most canonical JSON bytes one pre-apply graph may hold.
const MAX_GRAPH_BYTES: usize = 16_384;

/// Cost of one candidate graph: state/transition counts, summed fade ticks,
/// and estimated bytes from the canonical JSON length. Pure: reads only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GraphCost {
    /// States in `states`.
    states: u32,
    /// Transitions in `transitions`.
    transitions: u32,
    /// Summed `fade_ticks` over all transitions, saturating.
    fade_sum: u64,
    /// Canonical JSON length in bytes.
    bytes: usize,
}

/// Count the cost of one candidate graph. Pure: the same input seals the same
/// cost on every machine; missing lists count zero so shape errors stay with
/// the validator; integer math only, no clock.
fn graph_cost(graph: &Value) -> GraphCost {
    let states = graph
        .get("states")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let transitions_list = graph.get("transitions").and_then(Value::as_array);
    let transitions = transitions_list.map_or(0, Vec::len);
    let mut fade_sum: u64 = 0;
    if let Some(list) = transitions_list {
        for item in list {
            let fade = item.get("fade_ticks").and_then(Value::as_u64).unwrap_or(0);
            fade_sum = fade_sum.saturating_add(fade);
        }
    }
    GraphCost {
        states: u32::try_from(states).unwrap_or(u32::MAX),
        transitions: u32::try_from(transitions).unwrap_or(u32::MAX),
        fade_sum,
        bytes: graph.to_string().len(),
    }
}

/// Caps summary for refusal messages, so the caller sees the legal values
/// without a second round trip.
fn graph_caps_summary() -> String {
    format!(
        "caps states<={MAX_GRAPH_STATES} transitions<={MAX_GRAPH_TRANSITIONS} \
         fade_sum<={MAX_GRAPH_FADE_SUM_TICKS} bytes<={MAX_GRAPH_BYTES}"
    )
}

/// Pre-apply gate over one cost: refuse hostile graphs before `anim.graph.set`
/// touches the door. Errors: `graph_too_large` naming the violated invariant
/// plus the caps and one recovery; the door and the last-good graph are
/// untouched because the caller returns before `with_door`.
fn check_graph_cost(cost: &GraphCost) -> Result<(), McpToolError> {
    let caps = graph_caps_summary();
    let recovery = "shrink the graph to the caps and call anim.graph.set again; \
        the last good graph is kept";
    if cost.states > MAX_GRAPH_STATES {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: states {} exceeds cap {MAX_GRAPH_STATES}; {caps}; \
                 recovery: {recovery}",
                cost.states,
            ),
        ));
    }
    if cost.transitions > MAX_GRAPH_TRANSITIONS {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: transitions {} exceeds cap {MAX_GRAPH_TRANSITIONS}; {caps}; \
                 recovery: {recovery}",
                cost.transitions,
            ),
        ));
    }
    if cost.fade_sum > MAX_GRAPH_FADE_SUM_TICKS {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: fade_sum {} exceeds cap {MAX_GRAPH_FADE_SUM_TICKS}; {caps}; \
                 recovery: {recovery}",
                cost.fade_sum,
            ),
        ));
    }
    if cost.bytes > MAX_GRAPH_BYTES {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: bytes {} exceeds cap {MAX_GRAPH_BYTES}; {caps}; \
                 recovery: {recovery}",
                cost.bytes,
            ),
        ));
    }
    Ok(())
}

fn typed<T: DeserializeOwned>(tool: &str, value: Value) -> Result<T, McpToolError> {
    rmcp::serde_json::from_value(value).map_err(|error| {
        McpToolError::new(
            "bad_output",
            format!(
                "`{tool}` produced output outside its schema ({error}); re-run and report the verb"
            ),
        )
    })
}

/// Run `f` on this session's machine under the audited dispatch seam.
fn with_door<R>(
    server: &McpServer,
    tool: &str,
    f: impl FnOnce(&mut AnimDoor) -> Result<R, DoorRefusal>,
) -> Result<R, McpToolError> {
    static DOORS: OnceLock<Mutex<BTreeMap<u64, AnimDoor>>> = OnceLock::new();
    server.dispatch(tool, |state| {
        let mut doors = DOORS.get_or_init(Mutex::default).lock().map_err(|_| {
            McpToolError::new(
                "lock_poisoned",
                "anim door table is poisoned; restart the door",
            )
        })?;
        f(doors.entry(state.session()).or_default()).map_err(refuse)
    })
}

fn graph_set(server: &McpServer, args: &GraphSetArgs) -> Result<GraphSetOutput, McpToolError> {
    check_graph_cost(&graph_cost(&args.graph))?;
    let text = args.graph.to_string();
    let value = with_door(server, TOOL_ANIM_GRAPH_SET, |door| door.set(&text))?;
    typed(TOOL_ANIM_GRAPH_SET, value)
}

fn graph_undo(server: &McpServer, _args: &NoArgs) -> Result<GraphUndoOutput, McpToolError> {
    let value = with_door(server, TOOL_ANIM_GRAPH_UNDO, AnimDoor::undo)?;
    typed(TOOL_ANIM_GRAPH_UNDO, value)
}

fn graph_get(server: &McpServer, _args: &NoArgs) -> Result<GraphGetOutput, McpToolError> {
    let graph = with_door(server, TOOL_ANIM_GRAPH_GET, |door| Ok(door.get()))?;
    Ok(GraphGetOutput { graph })
}

fn preview(server: &McpServer, args: &PreviewArgs) -> Result<PreviewOutput, McpToolError> {
    let graph = args.graph.as_ref().map(Value::to_string);
    let script = args.script.to_string();
    let value = with_door(server, TOOL_ANIM_PREVIEW, |door| {
        door.preview(graph.as_deref(), &script)
    })?;
    typed(TOOL_ANIM_PREVIEW, value)
}

crate::e2_tool!(
    /// `anim.graph.set`: validate a JSON state machine and make it current.
    GraphSetTool,
    name = TOOL_ANIM_GRAPH_SET,
    description = "Set the animation state machine from JSON (states, transitions with switch mode, condition and fade ticks, integers only). Refused documents name the field path and keep the last good graph; undoable with anim.graph.undo.",
    readonly = false,
    args = GraphSetArgs,
    output = GraphSetOutput,
    invoke = graph_set
);

crate::e2_tool!(
    /// `anim.graph.undo`: undo the last `anim.graph.set` exactly.
    GraphUndoTool,
    name = TOOL_ANIM_GRAPH_UNDO,
    description = "Undo the last anim.graph.set; the graph digest returns to its earlier value exactly.",
    readonly = false,
    args = NoArgs,
    output = GraphUndoOutput,
    invoke = graph_undo
);

crate::e2_tool!(
    /// `anim.graph.get`: the current machine as canonical JSON.
    GraphGetTool,
    name = TOOL_ANIM_GRAPH_GET,
    description = "Read the current animation state machine as canonical JSON (null before the first anim.graph.set). No side effects.",
    readonly = true,
    args = NoArgs,
    output = GraphGetOutput,
    invoke = graph_get
);

crate::e2_tool!(
    /// `anim.preview`: run a scripted tape on a graph without applying it.
    PreviewTool,
    name = TOOL_ANIM_PREVIEW,
    description = "Run a scripted tape (parameter writes and travel requests by tick, at most 10000 ticks) on the current or a candidate state machine; returns every transition, the final state and a replay digest. Nothing is applied.",
    readonly = true,
    args = PreviewArgs,
    output = PreviewOutput,
    invoke = preview
);

/// The anim pack: state machine verbs, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "animation: the state machine as versioned data (set, undo, get, preview on a tape); \
         clip edits (trim, loop, retime, mirror, cleanup) and strips join later",
        false,
    );
    pack.add(VerbSpec::of::<GraphSetTool>(
        TOOL_ANIM_GRAPH_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<GraphUndoTool>(
        TOOL_ANIM_GRAPH_UNDO,
        PACK,
        CapabilityCard::Revert,
    )?);
    pack.add(VerbSpec::of::<GraphGetTool>(
        TOOL_ANIM_GRAPH_GET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<PreviewTool>(
        TOOL_ANIM_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use rmcp::serde_json::json;

    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    fn machine(fade: u64) -> Value {
        json!({
            "version": 1, "name": "fighter", "start": "idle",
            "params": [{"name": "speed", "default": 0, "min": 0, "max": 10}],
            "states": [
                {"name": "idle", "clip": 0, "length_ticks": 30, "looping": true},
                {"name": "walk", "clip": 1, "length_ticks": 24, "looping": true}
            ],
            "transitions": [
                {"from": "idle", "to": "walk", "when": {"param": "speed", "op": ">=", "value": 2}, "fade_ticks": fade},
                {"from": "walk", "to": "idle", "when": {"param": "speed", "op": "<", "value": 2}, "fade_ticks": fade}
            ]
        })
    }

    #[test]
    fn pack_registers_four_annotated_verbs() {
        let pack = pack().expect("registers");
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_ANIM_GRAPH_SET,
                TOOL_ANIM_GRAPH_UNDO,
                TOOL_ANIM_GRAPH_GET,
                TOOL_ANIM_PREVIEW
            ]
        );
        for verb in pack.verbs() {
            assert_ne!(verb.card(), CapabilityCard::Deny);
            assert!(
                verb.model().annotations.is_some(),
                "{} is annotated",
                verb.name()
            );
        }
        assert!(super::super::PackRegistry::shipped().errors().is_empty());
        assert!(crate::mcp::tool_names().contains(&TOOL_ANIM_PREVIEW));
    }

    #[test]
    fn set_undo_preview_through_the_door() {
        let server = server(41);
        let first = GraphSetTool::invoke(&server, GraphSetArgs { graph: machine(6) }).expect("set");
        assert_eq!(
            (first.before.clone(), first.states, first.transitions),
            (None, 2, 2)
        );
        let second =
            GraphSetTool::invoke(&server, GraphSetArgs { graph: machine(9) }).expect("set");
        assert_eq!(second.before, Some(first.after.clone()));
        let refused = GraphSetTool::invoke(
            &server,
            GraphSetArgs {
                graph: json!({"version": 1, "name": "x", "start": "a", "mood": 1}),
            },
        )
        .expect_err("unknown field");
        assert_eq!(refused.code, "bad_document");
        assert!(refused.message.contains("$.mood"), "{}", refused.message);
        let kept = GraphGetTool::invoke(&server, NoArgs {}).expect("get");
        let kept = kept.graph.expect("a graph is set");
        assert_eq!(kept["transitions"][0]["fade_ticks"], 9, "last good kept");
        let tape = json!({"ticks": 60, "params": [{"at": 2, "name": "speed", "value": 5}]});
        let a = PreviewTool::invoke(
            &server,
            PreviewArgs {
                graph: None,
                script: tape.clone(),
            },
        )
        .expect("preview");
        let b = PreviewTool::invoke(
            &server,
            PreviewArgs {
                graph: None,
                script: tape,
            },
        )
        .expect("preview");
        assert_eq!(a, b);
        assert_eq!(
            a.events.first().map(|e| (e.to.as_str(), e.fade_ticks)),
            Some(("walk", 9))
        );
        let back = GraphUndoTool::invoke(&server, NoArgs {}).expect("undo");
        assert_eq!(back.after, first.after, "undo restores the first digest");
        assert_eq!(
            GraphUndoTool::invoke(&server, NoArgs {})
                .expect_err("nothing left")
                .code,
            "nothing_to_undo"
        );
    }

    #[test]
    fn sessions_do_not_share_graphs_and_preview_needs_a_graph() {
        let (a, b) = (server(51), server(52));
        GraphSetTool::invoke(&a, GraphSetArgs { graph: machine(3) }).expect("set");
        assert_eq!(
            GraphGetTool::invoke(&b, NoArgs {}).expect("get").graph,
            None
        );
        let script = json!({"ticks": 5});
        let refused = PreviewTool::invoke(
            &b,
            PreviewArgs {
                graph: None,
                script: script.clone(),
            },
        )
        .expect_err("no graph");
        assert_eq!(refused.code, "no_graph");
        let candidate = PreviewTool::invoke(
            &b,
            PreviewArgs {
                graph: Some(machine(3)),
                script,
            },
        )
        .expect("a candidate runs without a set graph");
        assert_eq!(candidate.final_state, "idle");
    }

    fn giant_states(count: usize) -> Value {
        let mut states = Vec::with_capacity(count);
        for i in 0..count {
            states.push(json!({
                "name": format!("s{i}"),
                "clip": 0,
                "length_ticks": 30,
                "looping": true
            }));
        }
        json!({
            "version": 1, "name": "fighter", "start": "s0",
            "params": [{"name": "speed", "default": 0, "min": 0, "max": 10}],
            "states": states,
            "transitions": [
                {"from": "s0", "to": "s1",
                    "when": {"param": "speed", "op": ">=", "value": 2}, "fade_ticks": 5},
                {"from": "s1", "to": "s0",
                    "when": {"param": "speed", "op": "<", "value": 2}, "fade_ticks": 5}
            ]
        })
    }

    fn fade_sum_graph(count: usize, fade_each: u64) -> Value {
        let mut transitions = Vec::with_capacity(count);
        for i in 0..count {
            let (from, to) = if i % 2 == 0 {
                ("idle", "walk")
            } else {
                ("walk", "idle")
            };
            transitions.push(json!({
                "from": from, "to": to,
                "when": {"param": "speed", "op": ">=", "value": 2},
                "fade_ticks": fade_each
            }));
        }
        json!({
            "version": 1, "name": "fighter", "start": "idle",
            "params": [{"name": "speed", "default": 0, "min": 0, "max": 10}],
            "states": [
                {"name": "idle", "clip": 0, "length_ticks": 30, "looping": true},
                {"name": "walk", "clip": 1, "length_ticks": 24, "looping": true}
            ],
            "transitions": transitions
        })
    }

    fn over_bytes_graph(pad_len: usize) -> Value {
        let mut graph = machine(5);
        if let Some(obj) = graph.as_object_mut() {
            obj.insert("pad".to_string(), Value::String("x".repeat(pad_len)));
        }
        graph
    }

    fn hostile_graphs() -> Vec<Value> {
        vec![
            giant_states(40),
            giant_states(50),
            giant_states(70),
            giant_states(90),
            giant_states(110),
            giant_states(150),
            giant_states(200),
            fade_sum_graph(9, 600),
            fade_sum_graph(10, 600),
            fade_sum_graph(12, 500),
            fade_sum_graph(15, 400),
            fade_sum_graph(20, 300),
            fade_sum_graph(12, 600),
            fade_sum_graph(16, 400),
            over_bytes_graph(20_000),
            over_bytes_graph(22_000),
            over_bytes_graph(25_000),
            over_bytes_graph(30_000),
            over_bytes_graph(40_000),
            over_bytes_graph(50_000),
        ]
    }

    #[test]
    fn hostile_graph_sets_refuse_pre_apply_and_keep_last_good() {
        let hostiles = hostile_graphs();
        assert_eq!(hostiles.len(), 20);
        let legals = vec![machine(3), machine(6), machine(9)];

        let mut pure_refused = 0;
        for graph in &hostiles {
            let first = graph_cost(graph);
            let second = graph_cost(graph);
            assert_eq!(first, second, "same input seals the same cost");
            let verdict = check_graph_cost(&first);
            let replay = check_graph_cost(&second);
            assert_eq!(
                verdict.is_ok(),
                replay.is_ok(),
                "repeated runs give identical results"
            );
            let refused = verdict.expect_err("hostile graph refuses pre-apply");
            assert_eq!(refused.code, "graph_too_large");
            assert!(
                refused.message.contains("caps states<="),
                "{}",
                refused.message
            );
            assert!(refused.message.contains("recovery:"), "{}", refused.message);
            assert!(
                crate::tool_capabilities::message_carries_recovery(&refused.message),
                "{}",
                refused.message
            );
            pure_refused += 1;
        }
        assert!(
            pure_refused >= 16,
            "at least 16 of 20 hostile graphs refuse pre-apply, got {pure_refused}"
        );

        for graph in &legals {
            let cost = graph_cost(graph);
            check_graph_cost(&cost).expect("legal small graph passes the gate");
            let replay = graph_cost(graph);
            assert_eq!(cost, replay, "legal cost replays exactly");
        }

        let door = server(61);
        let first = GraphSetTool::invoke(
            &door,
            GraphSetArgs {
                graph: legals[0].clone(),
            },
        )
        .expect("legal set lands");
        let baseline = GraphGetTool::invoke(&door, NoArgs {})
            .expect("get")
            .graph
            .expect("a graph is set");
        assert_eq!(first.states, 2);

        let mut wired_refused = 0;
        for graph in &hostiles {
            let refused = GraphSetTool::invoke(
                &door,
                GraphSetArgs {
                    graph: graph.clone(),
                },
            )
            .expect_err("hostile set refuses before the door mutates");
            assert_eq!(
                refused.code, "graph_too_large",
                "pre-apply, not the validator"
            );
            wired_refused += 1;
            let kept = GraphGetTool::invoke(&door, NoArgs {})
                .expect("get")
                .graph
                .expect("a graph is set");
            assert_eq!(kept, baseline, "a refused set keeps the last good graph");
        }
        assert!(
            wired_refused >= 16,
            "at least 16 of 20 hostile sets refuse at the door, got {wired_refused}"
        );

        for graph in &legals {
            GraphSetTool::invoke(
                &door,
                GraphSetArgs {
                    graph: graph.clone(),
                },
            )
            .expect("legal small graph passes at the door");
        }
        let kept = GraphGetTool::invoke(&door, NoArgs {})
            .expect("get")
            .graph
            .expect("a graph is set");
        assert!(
            kept["states"].as_array().map_or(0, Vec::len) <= 4,
            "legal graphs stay small"
        );
    }
}
