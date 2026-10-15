//! `AnimDoor`: the state behind the verbs `anim.graph.set`, `anim.graph.undo`
//! and `anim.preview`. One undoable state machine per door session, JSON in
//! and JSON out; refusals carry a stable code, the document path and one
//! recovery step, so a pack only maps them onto its own error type.

use serde_json::{json, Value};

use super::door::{parse_script, preview, PreviewError, MAX_PREVIEW_TICKS};
use super::json;
use super::{Document, StateMachineError};

/// A refusal a verb pack can forward.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DoorRefusal {
    /// `bad_document`, `invalid_machine`, `nothing_to_undo`, `no_graph`,
    /// `bad_script` or `preview_failed`.
    pub code: &'static str,
    /// What is wrong, naming the path or the state.
    pub message: String,
    /// One recovery step.
    pub recovery: &'static str,
}

const FIX_AND_RESET: &str =
    "fix the named field and call anim.graph.set again; the last good graph is kept";
const SCRIPT_SHAPE: &str = "send {\"ticks\":N,\"params\":[{\"at\":T,\"name\":..,\"value\":..}],\"travel\":[{\"at\":T,\"to\":..}]}";

fn refusal_of(error: &StateMachineError) -> DoorRefusal {
    let (code, recovery) = match error {
        StateMachineError::Json { .. } => ("bad_document", FIX_AND_RESET),
        StateMachineError::NothingToUndo => ("nothing_to_undo", "call anim.graph.set first"),
        _ => ("invalid_machine", FIX_AND_RESET),
    };
    DoorRefusal {
        code,
        message: error.to_string(),
        recovery,
    }
}

fn preview_refusal(error: &PreviewError) -> DoorRefusal {
    match error {
        PreviewError::Machine(inner) => refusal_of(inner),
        PreviewError::Script(inner) => DoorRefusal {
            code: "bad_script",
            message: inner.to_string(),
            recovery: SCRIPT_SHAPE,
        },
        PreviewError::Play { tick, error } => DoorRefusal {
            code: "preview_failed",
            message: format!("tick {tick}: {error}"),
            recovery: "use a state and parameter name from the graph, inside the parameter range",
        },
        PreviewError::TooLong(n) => DoorRefusal {
            code: "preview_failed",
            message: format!("{n} ticks is over the {MAX_PREVIEW_TICKS} tick preview cap"),
            recovery: "lower ticks",
        },
    }
}

fn no_graph() -> DoorRefusal {
    DoorRefusal {
        code: "no_graph",
        message: "no graph is set".into(),
        recovery: "call anim.graph.set first, or pass graph to anim.preview",
    }
}

/// Digests travel as 16 hex digits: a JSON number would lose bits above 2^53.
fn hex(digest: u64) -> String {
    format!("{digest:016x}")
}

/// The anim verbs' session state.
#[derive(Clone, Debug, Default)]
pub struct AnimDoor {
    doc: Option<Document>,
}

impl AnimDoor {
    /// No graph yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// `anim.graph.set`: validate the JSON machine and make it current. The
    /// first graph starts the document; later ones are undoable replacements.
    pub fn set(&mut self, text: &str) -> Result<Value, DoorRefusal> {
        let def = json::parse(text).map_err(|e| refusal_of(&e))?;
        let before = self.doc.as_ref().map(|d| hex(d.def().digest()));
        let doc = match self.doc.take() {
            Some(mut doc) => {
                let result = doc.replace(def);
                self.doc = Some(doc);
                result.map_err(|e| refusal_of(&e))?;
                self.doc.as_ref()
            }
            None => {
                self.doc = Some(Document::new(def).map_err(|e| refusal_of(&e))?);
                self.doc.as_ref()
            }
        };
        let now = doc.ok_or_else(no_graph)?.def();
        let lints: Vec<String> = now.lints().iter().map(|l| format!("{l:?}")).collect();
        Ok(json!({
            "before": before,
            "after": hex(now.digest()),
            "states": now.states.len(),
            "transitions": now.transitions.len(),
            "lints": lints,
        }))
    }

    /// `anim.graph.undo`: undo the last set; the digest returns exactly.
    pub fn undo(&mut self) -> Result<Value, DoorRefusal> {
        let doc = self
            .doc
            .as_mut()
            .ok_or_else(|| refusal_of(&StateMachineError::NothingToUndo))?;
        doc.undo().map_err(|e| refusal_of(&e))?;
        Ok(json!({"after": hex(doc.def().digest()), "undo_depth": doc.undo_depth()}))
    }

    /// The current graph as canonical JSON.
    pub fn get(&self) -> Option<Value> {
        self.doc.as_ref().map(|d| json::to_value(d.def()))
    }

    /// `anim.preview`: run a scripted tape on `graph` (JSON text) or, when it
    /// is `None`, on the current graph. Nothing is applied.
    pub fn preview(&self, graph: Option<&str>, script: &str) -> Result<Value, DoorRefusal> {
        let candidate;
        let def = match graph {
            Some(text) => {
                candidate = json::parse(text).map_err(|e| refusal_of(&e))?;
                &candidate
            }
            None => self.doc.as_ref().map(Document::def).ok_or_else(no_graph)?,
        };
        let script = parse_script(script).map_err(|e| preview_refusal(&e))?;
        let report = preview(def, &script).map_err(|e| preview_refusal(&e))?;
        let events: Vec<Value> = report
            .events
            .iter()
            .map(
                |e| json!({"tick": e.tick, "from": e.from, "to": e.to, "fade_ticks": e.fade_ticks}),
            )
            .collect();
        Ok(json!({
            "events": events,
            "final_state": report.final_state,
            "digest": hex(report.digest),
        }))
    }
}
