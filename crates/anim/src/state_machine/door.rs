//! Door-shaped entry points: set a graph from JSON, preview a scripted run.
//! Plain Rust functions with typed errors; a verb pack calls these and adds
//! the wire format. Neither touches the document unless it succeeds.

use std::sync::Arc;

use serde_json::Value;

use super::json;
use super::{
    CompiledMachine, Document, Lint, PlayError, Playback, StateMachineDef, StateMachineError,
};

/// Most ticks a preview may run.
pub const MAX_PREVIEW_TICKS: u32 = 10_000;

/// What `graph_set` did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SetReceipt {
    /// Digest before.
    pub before: u64,
    /// Digest after.
    pub after: u64,
    /// States in the new machine.
    pub states: usize,
    /// Transitions in the new machine.
    pub transitions: usize,
    /// Non-fatal findings.
    pub lints: Vec<Lint>,
}

/// Replace the document's machine with the JSON one; undoable; on any error
/// the last good machine stays.
pub fn graph_set(doc: &mut Document, text: &str) -> Result<SetReceipt, StateMachineError> {
    let def = json::parse(text)?;
    let before = doc.def().digest();
    doc.replace(def)?;
    let now = doc.def();
    Ok(SetReceipt {
        before,
        after: now.digest(),
        states: now.states.len(),
        transitions: now.transitions.len(),
        lints: now.lints(),
    })
}

/// A scripted run: parameter writes and travel requests at given ticks.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PreviewScript {
    /// Ticks to run (at most [`MAX_PREVIEW_TICKS`]).
    pub ticks: u32,
    /// `(tick, parameter, value)` writes, applied before that tick runs.
    pub params: Vec<(u32, String, i64)>,
    /// `(tick, target state)` travel requests, applied before that tick runs.
    pub travel: Vec<(u32, String)>,
}

/// One transition seen in a preview.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PreviewEvent {
    /// Tick it started on.
    pub tick: u32,
    /// State left.
    pub from: String,
    /// State entered.
    pub to: String,
    /// Fade length in ticks.
    pub fade_ticks: u32,
}

/// Result of a preview.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PreviewReport {
    /// Every transition in order.
    pub events: Vec<PreviewEvent>,
    /// State at the end.
    pub final_state: String,
    /// Playback digest at the end: equal on every replay.
    pub digest: u64,
}

/// Why a preview failed.
#[derive(Clone, PartialEq, Debug)]
pub enum PreviewError {
    /// The machine does not validate.
    Machine(StateMachineError),
    /// A scripted write or travel was refused.
    Play {
        /// Tick of the request.
        tick: u32,
        /// Reason.
        error: PlayError,
    },
    /// Script runs longer than the cap.
    TooLong(u32),
    /// Script JSON is wrong.
    Script(StateMachineError),
}

/// Run `script` on a private copy; the document is not touched.
pub fn preview(
    def: &StateMachineDef,
    script: &PreviewScript,
) -> Result<PreviewReport, PreviewError> {
    if script.ticks > MAX_PREVIEW_TICKS {
        return Err(PreviewError::TooLong(script.ticks));
    }
    let machine = CompiledMachine::new(def).map_err(PreviewError::Machine)?;
    let mut play = Playback::new(Arc::new(machine));
    let mut events = Vec::new();
    for tick in 0..script.ticks {
        let play_err = |error| PreviewError::Play { tick, error };
        for (_, name, value) in script.params.iter().filter(|w| w.0 == tick) {
            play.set_param(name, *value).map_err(play_err)?;
        }
        for (_, target) in script.travel.iter().filter(|w| w.0 == tick) {
            play.travel(target).map_err(play_err)?;
        }
        let before = play.state();
        let report = play.tick();
        if let Some(e) = report.taken {
            let t = &def.transitions[e];
            events.push(PreviewEvent {
                tick: tick + 1,
                from: def.states[before].name.clone(),
                to: t.to.clone(),
                fade_ticks: t.fade_ticks,
            });
        }
    }
    Ok(PreviewReport {
        events,
        final_state: play.state_name().to_string(),
        digest: play.digest(),
    })
}

/// Read a script from JSON: `{"ticks":N,"params":[{"at":T,"name":..,"value":..}],"travel":[{"at":T,"to":..}]}`.
pub fn parse_script(text: &str) -> Result<PreviewScript, PreviewError> {
    let bad = |path: &str, reason: &'static str| {
        PreviewError::Script(StateMachineError::Json {
            path: path.to_string(),
            reason,
        })
    };
    let root: Value = serde_json::from_str(text).map_err(|_| bad("$", "not valid JSON"))?;
    let map = root
        .as_object()
        .ok_or_else(|| bad("$", "expected an object"))?;
    let num = |v: &Value, path: &str| -> Result<u32, PreviewError> {
        v.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| bad(path, "expected a tick count"))
    };
    let mut script = PreviewScript {
        ticks: num(
            map.get("ticks")
                .ok_or_else(|| bad("$.ticks", "missing field"))?,
            "$.ticks",
        )?,
        ..PreviewScript::default()
    };
    for (i, w) in map
        .get("params")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let at = format!("$.params[{i}]");
        let at_tick = num(w.get("at").ok_or_else(|| bad(&at, "missing at"))?, &at)?;
        let name = w
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(&at, "missing name"))?;
        let value = w
            .get("value")
            .and_then(Value::as_i64)
            .ok_or_else(|| bad(&at, "missing integer value"))?;
        script.params.push((at_tick, name.to_string(), value));
    }
    for (i, w) in map
        .get("travel")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let at = format!("$.travel[{i}]");
        let at_tick = num(w.get("at").ok_or_else(|| bad(&at, "missing at"))?, &at)?;
        let to = w
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(&at, "missing to"))?;
        script.travel.push((at_tick, to.to_string()));
    }
    Ok(script)
}
