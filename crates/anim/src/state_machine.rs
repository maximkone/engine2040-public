//! state_machine: the animation state machine as versioned, validated data.
//!
//! Owner: `engine-anim`; consumers: authors through [`Document`] (edit, undo,
//! reload) and the tick through [`Playback`]. Units: integer ticks only, fade
//! weights in `0..=WEIGHT_ONE` fixed point; no float time enters a decision.
//! Determinism: stable `Vec` order, integer math, no clock; [`StateMachineDef::digest`]
//! is FNV over canonical bytes. Errors: [`StateMachineError`] (author time) and
//! [`PlayError`] (run time) name the state, transition or path at fault.
//!
//! PORT(godotengine/godot@4.x:scene/animation/animation_node_state_machine.cpp,
//! MIT): the transition vocabulary (switch mode immediate, sync, at end; advance
//! disabled, enabled, auto; advance condition; priority; reset; cross-fade) and
//! the shortest-path `travel`. Ours is integer ticks with a total tie-break.

use std::fmt;

use engine_core::Fnv64;

use crate::graph::{AnimGraph, ClipBank, GraphError, NodeId};

mod edit;
mod playback;

#[cfg(feature = "json")]
pub mod anim_door;
#[cfg(feature = "json")]
pub mod door;
#[cfg(feature = "json")]
pub mod json;

pub use edit::{Document, Edit};
pub use playback::{
    fighter_crossfade_ticks, FadeFrom, Playback, StateSample, TickReport, FIGHTER_CROSSFADE_TICKS,
};

/// Data format version this build reads and writes.
pub const SM_VERSION: u32 = 1;
/// Most states one machine may hold.
pub const MAX_STATES: usize = 128;
/// Most transitions one machine may hold.
pub const MAX_TRANSITIONS: usize = 1024;
/// Most parameters one machine may hold.
pub const MAX_PARAMS: usize = 64;
/// Longest cross-fade, in ticks.
pub const MAX_FADE_TICKS: u32 = 600;
/// Longest state clip length, in ticks.
pub const MAX_STATE_TICKS: u32 = 1_000_000;
/// Longest state, parameter or machine name, in bytes.
pub const MAX_NAME_BYTES: usize = 48;
/// Deepest condition nesting.
pub const MAX_COND_DEPTH: usize = 8;
/// Most condition nodes per transition.
pub const MAX_COND_NODES: usize = 32;
/// Fixed-point one for fade weights (a power of two: exact in `f32`).
pub const WEIGHT_ONE: u32 = 4096;
/// Reserved playback-rate parameter name: when a machine declares this
/// parameter, [`Playback`](playback::Playback) advances the clip position by
/// `rate / RATE_Q8_ONE` ticks per tick instead of one. Authors declare it as
/// `min 0, max RATE_Q8_MAX, default RATE_Q8_ONE`; the sim drives it through
/// the existing [`Playback::set_param`](playback::Playback::set_param) beside
/// the fight flags, so a speed-scaled run needs no new call. Absent, the rate
/// reads 1x. Fades stay on wall ticks; only the clip age (and the leaving
/// age inside a fade) scales.
pub const RATE_Q8_PARAM: &str = "rate_q8";
/// Q8 one for the playback rate: 256 reads 1x.
pub const RATE_Q8_ONE: i64 = 256;
/// Rate ceiling in Q8 units: 1024 reads 4x; higher values clamp here.
pub const RATE_Q8_MAX: i64 = 1024;

/// What a state plays: a clip key from the bank or a graph node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Clip key in the [`ClipBank`].
    Clip(u32),
    /// Root node id in the [`AnimGraph`] (a blend space, layers, emits).
    Node(u32),
}

/// One state: a named clip or node with a tick length.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StateDef {
    /// Unique authoring name; transitions refer to it.
    pub name: String,
    /// What the state plays.
    pub source: Source,
    /// Length of one pass, in ticks (at least 1).
    pub length_ticks: u32,
    /// Wrap at the end (true) or hold the last frame (false).
    pub looping: bool,
}

/// When a transition may switch once its condition allows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwitchMode {
    /// Switch on the tick it is taken.
    Immediate,
    /// Switch now and start the target at the same relative playback position.
    Sync,
    /// Wait until the source ends; the fade ends with it.
    AtEnd,
}

/// Who may take a transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdvanceMode {
    /// Never used, not even by travel.
    Disabled,
    /// Used only by travel (and explicit requests).
    Enabled,
    /// Used by travel and automatically when its condition holds.
    Auto,
}

/// Fade-in curve of the target, integer only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Curve {
    /// `x`.
    Linear,
    /// `x^2`.
    EaseIn,
    /// `1 - (1 - x)^2`.
    EaseOut,
    /// `3x^2 - 2x^3`.
    EaseInOut,
}

/// Comparison of a parameter against a constant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmpOp {
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl CmpOp {
    pub(crate) fn test(self, left: i64, right: i64) -> bool {
        match self {
            CmpOp::Eq => left == right,
            CmpOp::Ne => left != right,
            CmpOp::Lt => left < right,
            CmpOp::Le => left <= right,
            CmpOp::Gt => left > right,
            CmpOp::Ge => left >= right,
        }
    }
}

/// Advance condition over integer parameters and time in the state.
/// `P` is the parameter reference: a name in data, an index once compiled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CondOf<P> {
    /// Always true.
    Always,
    /// `param op value`.
    Param {
        /// Parameter reference.
        param: P,
        /// Comparison.
        op: CmpOp,
        /// Constant to compare with.
        value: i64,
    },
    /// Ticks spent in the current state are at least this many.
    TimeAtLeast(u32),
    /// Negation.
    Not(Box<CondOf<P>>),
    /// Every child holds (empty is true).
    All(Vec<CondOf<P>>),
    /// Some child holds (empty is false).
    Any(Vec<CondOf<P>>),
}

/// Authored condition: parameters by name.
pub type Cond = CondOf<String>;

impl<P> CondOf<P> {
    /// Node count and depth in one walk.
    fn size(&self, depth: usize) -> (usize, usize) {
        match self {
            CondOf::Not(inner) => {
                let (nodes, deep) = inner.size(depth + 1);
                (nodes + 1, deep)
            }
            CondOf::All(kids) | CondOf::Any(kids) => kids.iter().fold((1, depth), |acc, kid| {
                let (nodes, deep) = kid.size(depth + 1);
                (acc.0 + nodes, acc.1.max(deep))
            }),
            _ => (1, depth),
        }
    }

    /// Map every parameter reference, failing on the first bad one.
    fn try_map<Q, E>(&self, f: &mut impl FnMut(&P) -> Result<Q, E>) -> Result<CondOf<Q>, E> {
        Ok(match self {
            CondOf::Always => CondOf::Always,
            CondOf::Param { param, op, value } => CondOf::Param {
                param: f(param)?,
                op: *op,
                value: *value,
            },
            CondOf::TimeAtLeast(n) => CondOf::TimeAtLeast(*n),
            CondOf::Not(inner) => CondOf::Not(Box::new(inner.try_map(f)?)),
            CondOf::All(kids) => CondOf::All(
                kids.iter()
                    .map(|kid| kid.try_map(f))
                    .collect::<Result<_, E>>()?,
            ),
            CondOf::Any(kids) => CondOf::Any(
                kids.iter()
                    .map(|kid| kid.try_map(f))
                    .collect::<Result<_, E>>()?,
            ),
        })
    }

    fn write_canonical(&self, hash: &mut Fnv64)
    where
        P: AsRef<str>,
    {
        match self {
            CondOf::Always => hash.write_u8(0),
            CondOf::Param { param, op, value } => {
                hash.write_u8(1);
                hash.write_str(param.as_ref());
                hash.write_u8(match op {
                    CmpOp::Eq => 0,
                    CmpOp::Ne => 1,
                    CmpOp::Lt => 2,
                    CmpOp::Le => 3,
                    CmpOp::Gt => 4,
                    CmpOp::Ge => 5,
                });
                hash.write_i64(*value);
            }
            CondOf::TimeAtLeast(n) => {
                hash.write_u8(2);
                hash.write_u32(*n);
            }
            CondOf::Not(inner) => {
                hash.write_u8(3);
                inner.write_canonical(hash);
            }
            CondOf::All(kids) | CondOf::Any(kids) => {
                hash.write_u8(if matches!(self, CondOf::All(_)) { 4 } else { 5 });
                hash.write_u32(u32::try_from(kids.len()).unwrap_or(u32::MAX));
                for kid in kids {
                    kid.write_canonical(hash);
                }
            }
        }
    }
}

impl CondOf<usize> {
    /// Evaluate against parameter values and ticks in the state.
    pub(crate) fn eval(&self, params: &[i64], age: u32) -> bool {
        match self {
            CondOf::Always => true,
            CondOf::Param { param, op, value } => {
                op.test(params.get(*param).copied().unwrap_or(0), *value)
            }
            CondOf::TimeAtLeast(n) => age >= *n,
            CondOf::Not(inner) => !inner.eval(params, age),
            CondOf::All(kids) => kids.iter().all(|kid| kid.eval(params, age)),
            CondOf::Any(kids) => kids.iter().any(|kid| kid.eval(params, age)),
        }
    }
}

/// One integer parameter with its legal range.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParamDef {
    /// Unique name.
    pub name: String,
    /// Starting value.
    pub default: i64,
    /// Smallest legal value.
    pub min: i64,
    /// Largest legal value.
    pub max: i64,
}

/// One transition between two states, by name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TransitionDef {
    /// Source state name.
    pub from: String,
    /// Target state name.
    pub to: String,
    /// When the switch happens.
    pub switch: SwitchMode,
    /// Who may take it.
    pub advance: AdvanceMode,
    /// Automatic-advance condition (travel ignores it).
    pub condition: Cond,
    /// Cross-fade length in ticks; zero is a hard cut.
    pub fade_ticks: u32,
    /// Fade curve.
    pub curve: Curve,
    /// Lower wins ties in auto advance and in travel.
    pub priority: u32,
    /// Restart the target from tick zero (ignored by sync).
    pub reset: bool,
}

impl TransitionDef {
    /// Automatic, immediate, unconditional, instant, linear: edit what differs.
    pub fn new(from: &str, to: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            switch: SwitchMode::Immediate,
            advance: AdvanceMode::Auto,
            condition: CondOf::Always,
            fade_ticks: 0,
            curve: Curve::Linear,
            priority: 0,
            reset: true,
        }
    }
}

/// The whole machine as authored data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StateMachineDef {
    /// Format version; only [`SM_VERSION`] is read.
    pub version: u32,
    /// Machine name.
    pub name: String,
    /// Name of the starting state.
    pub start: String,
    /// Parameters.
    pub params: Vec<ParamDef>,
    /// States in authored order.
    pub states: Vec<StateDef>,
    /// Transitions in authored order (the order breaks the last ties).
    pub transitions: Vec<TransitionDef>,
}

/// Author-time failure; every variant names what to fix.
#[derive(Clone, PartialEq, Debug)]
pub enum StateMachineError {
    /// Version other than [`SM_VERSION`].
    UnsupportedVersion(u32),
    /// No states.
    NoStates,
    /// A list is over its cap.
    TooMany {
        /// Which list.
        what: &'static str,
        /// The cap.
        max: usize,
    },
    /// Empty, over-long or odd name.
    BadName {
        /// Which kind of name.
        what: &'static str,
        /// The name.
        name: String,
    },
    /// Two items share a name.
    Duplicate {
        /// Which kind of name.
        what: &'static str,
        /// The name.
        name: String,
    },
    /// The start state is not in the machine.
    UnknownStart(String),
    /// A state has zero or over-long length.
    BadLength {
        /// State name.
        state: String,
        /// Length in ticks.
        ticks: u32,
    },
    /// A transition names a state that is absent.
    UnknownState {
        /// Transition index.
        transition: usize,
        /// The name.
        name: String,
    },
    /// A condition names a parameter that is absent.
    UnknownParam {
        /// Transition index.
        transition: usize,
        /// The name.
        name: String,
    },
    /// Fade over [`MAX_FADE_TICKS`].
    FadeTooLong {
        /// Transition index.
        transition: usize,
        /// Ticks asked.
        ticks: u32,
    },
    /// Condition too deep or too large.
    CondTooBig {
        /// Transition index.
        transition: usize,
    },
    /// Parameter range empty or default outside it.
    BadParamRange(String),
    /// A loop of unconditional instant automatic transitions: it would flip
    /// states every tick forever.
    InstantCycle(Vec<String>),
    /// A state's graph node does not compile (a cyclic graph lands here).
    BadSource {
        /// State name.
        state: String,
        /// Graph failure.
        error: GraphError,
    },
    /// A state's clip key is not in the bank.
    UnknownClip {
        /// State name.
        state: String,
        /// Clip key.
        clip: u32,
    },
    /// An edit index is out of range.
    IndexOutOfRange {
        /// Which list.
        what: &'static str,
        /// The index.
        index: usize,
    },
    /// Removing a state or parameter that something still uses.
    InUse {
        /// Which kind.
        what: &'static str,
        /// The name.
        name: String,
    },
    /// A replace edit tried to rename; use a rename edit.
    RenameViaReplace(String),
    /// Nothing to undo or redo.
    NothingToUndo,
    /// Text or JSON problem; `path` points into the document.
    Json {
        /// Document path such as `states[2].length_ticks`.
        path: String,
        /// What is wrong.
        reason: &'static str,
    },
}

impl fmt::Display for StateMachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => {
                write!(f, "state machine: version {v} is not {SM_VERSION}")
            }
            Self::NoStates => write!(f, "state machine: no states"),
            Self::TooMany { what, max } => write!(f, "state machine: more than {max} {what}"),
            Self::BadName { what, name } => write!(f, "state machine: bad {what} name '{name}'"),
            Self::Duplicate { what, name } => write!(f, "state machine: duplicate {what} '{name}'"),
            Self::UnknownStart(n) => write!(f, "state machine: start state '{n}' is absent"),
            Self::BadLength { state, ticks } => {
                write!(f, "state machine: state '{state}' length {ticks} ticks is not in 1..={MAX_STATE_TICKS}")
            }
            Self::UnknownState { transition, name } => {
                write!(
                    f,
                    "state machine: transition {transition} names absent state '{name}'"
                )
            }
            Self::UnknownParam { transition, name } => {
                write!(
                    f,
                    "state machine: transition {transition} reads absent parameter '{name}'"
                )
            }
            Self::FadeTooLong { transition, ticks } => {
                write!(f, "state machine: transition {transition} fade {ticks} ticks is over {MAX_FADE_TICKS}")
            }
            Self::CondTooBig { transition } => {
                write!(
                    f,
                    "state machine: transition {transition} condition is too deep or large"
                )
            }
            Self::BadParamRange(n) => write!(
                f,
                "state machine: parameter '{n}' range is empty or default is outside it"
            ),
            Self::InstantCycle(states) => write!(
                f,
                "state machine: instant automatic cycle through {}",
                states.join(", ")
            ),
            Self::BadSource { state, error } => {
                write!(f, "state machine: state '{state}': {error}")
            }
            Self::UnknownClip { state, clip } => {
                write!(
                    f,
                    "state machine: state '{state}' plays missing clip {clip}"
                )
            }
            Self::IndexOutOfRange { what, index } => {
                write!(f, "state machine: {what} index {index} is out of range")
            }
            Self::InUse { what, name } => write!(f, "state machine: {what} '{name}' is still used"),
            Self::RenameViaReplace(n) => write!(
                f,
                "state machine: '{n}' cannot be renamed by a replace edit"
            ),
            Self::NothingToUndo => write!(f, "state machine: nothing to undo or redo"),
            Self::Json { path, reason } => write!(f, "state machine: {path}: {reason}"),
        }
    }
}

impl std::error::Error for StateMachineError {}

/// Run-time failure.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlayError {
    /// State name absent.
    UnknownState(String),
    /// Parameter name absent.
    UnknownParam(String),
    /// Value outside the parameter range.
    ParamOutOfRange {
        /// Parameter name.
        name: String,
        /// Value asked.
        value: i64,
    },
    /// No usable transitions lead from `from` to `to`.
    NoPath {
        /// Start state name.
        from: String,
        /// Target state name.
        to: String,
    },
}

impl fmt::Display for PlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownState(n) => write!(f, "playback: state '{n}' is absent"),
            Self::UnknownParam(n) => write!(f, "playback: parameter '{n}' is absent"),
            Self::ParamOutOfRange { name, value } => {
                write!(f, "playback: parameter '{name}' cannot be {value}")
            }
            Self::NoPath { from, to } => write!(f, "playback: no path from '{from}' to '{to}'"),
        }
    }
}

impl std::error::Error for PlayError {}

/// Non-fatal findings from [`StateMachineDef::lints`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Lint {
    /// The state cannot be reached from the start by any usable transition.
    Unreachable(String),
    /// The state has no usable way out.
    DeadEnd(String),
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

impl StateMachineDef {
    /// Empty machine at the current version; fill `states` then validate.
    pub fn new(name: &str, start: &str) -> Self {
        Self {
            version: SM_VERSION,
            name: name.to_string(),
            start: start.to_string(),
            params: Vec::new(),
            states: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// Index of the state named `name`.
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    /// Index of the parameter named `name`.
    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }

    /// Structural validation: names, references, ranges, caps, instant loops.
    /// Source existence is a separate step ([`StateMachineDef::check_sources`]).
    pub fn validate(&self) -> Result<(), StateMachineError> {
        if self.version != SM_VERSION {
            return Err(StateMachineError::UnsupportedVersion(self.version));
        }
        if !name_ok(&self.name) {
            return Err(StateMachineError::BadName {
                what: "machine",
                name: self.name.clone(),
            });
        }
        if self.states.is_empty() {
            return Err(StateMachineError::NoStates);
        }
        cap("states", self.states.len(), MAX_STATES)?;
        cap("transitions", self.transitions.len(), MAX_TRANSITIONS)?;
        cap("parameters", self.params.len(), MAX_PARAMS)?;
        self.validate_names()?;
        self.validate_transitions()?;
        self.check_instant_cycles()
    }

    fn validate_names(&self) -> Result<(), StateMachineError> {
        for (i, state) in self.states.iter().enumerate() {
            if !name_ok(&state.name) {
                return Err(StateMachineError::BadName {
                    what: "state",
                    name: state.name.clone(),
                });
            }
            if self.states[..i].iter().any(|s| s.name == state.name) {
                return Err(StateMachineError::Duplicate {
                    what: "state",
                    name: state.name.clone(),
                });
            }
            if state.length_ticks == 0 || state.length_ticks > MAX_STATE_TICKS {
                return Err(StateMachineError::BadLength {
                    state: state.name.clone(),
                    ticks: state.length_ticks,
                });
            }
        }
        for (i, param) in self.params.iter().enumerate() {
            if !name_ok(&param.name) {
                return Err(StateMachineError::BadName {
                    what: "parameter",
                    name: param.name.clone(),
                });
            }
            if self.params[..i].iter().any(|p| p.name == param.name) {
                return Err(StateMachineError::Duplicate {
                    what: "parameter",
                    name: param.name.clone(),
                });
            }
            if param.min > param.max || param.default < param.min || param.default > param.max {
                return Err(StateMachineError::BadParamRange(param.name.clone()));
            }
        }
        if self.state_index(&self.start).is_none() {
            return Err(StateMachineError::UnknownStart(self.start.clone()));
        }
        Ok(())
    }

    fn validate_transitions(&self) -> Result<(), StateMachineError> {
        for (i, t) in self.transitions.iter().enumerate() {
            for name in [&t.from, &t.to] {
                if self.state_index(name).is_none() {
                    return Err(StateMachineError::UnknownState {
                        transition: i,
                        name: name.clone(),
                    });
                }
            }
            if t.fade_ticks > MAX_FADE_TICKS {
                return Err(StateMachineError::FadeTooLong {
                    transition: i,
                    ticks: t.fade_ticks,
                });
            }
            let (nodes, depth) = t.condition.size(1);
            if nodes > MAX_COND_NODES || depth > MAX_COND_DEPTH {
                return Err(StateMachineError::CondTooBig { transition: i });
            }
            t.condition.try_map(&mut |name: &String| {
                self.param_index(name)
                    .ok_or_else(|| StateMachineError::UnknownParam {
                        transition: i,
                        name: name.clone(),
                    })
            })?;
        }
        Ok(())
    }

    /// Unconditional instant automatic edges must not close a loop.
    fn check_instant_cycles(&self) -> Result<(), StateMachineError> {
        let n = self.states.len();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
        for t in &self.transitions {
            let instant = t.advance == AdvanceMode::Auto
                && t.condition == CondOf::Always
                && t.fade_ticks == 0
                && t.switch != SwitchMode::AtEnd;
            if instant {
                if let (Some(a), Some(b)) = (self.state_index(&t.from), self.state_index(&t.to)) {
                    next[a].push(b);
                }
            }
        }
        // Kahn: states left with edges after peeling sources sit on or behind a cycle;
        // peel sinks too so only cycle members remain.
        let mut indeg = vec![0usize; n];
        for outs in &next {
            for &b in outs {
                indeg[b] += 1;
            }
        }
        let mut alive = vec![true; n];
        let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        while let Some(i) = queue.pop() {
            alive[i] = false;
            for &b in &next[i] {
                indeg[b] -= 1;
                if indeg[b] == 0 {
                    queue.push(b);
                }
            }
        }
        let stuck: Vec<String> = (0..n)
            .filter(|&i| alive[i])
            .map(|i| self.states[i].name.clone())
            .collect();
        if stuck.is_empty() {
            Ok(())
        } else {
            Err(StateMachineError::InstantCycle(stuck))
        }
    }

    /// Check every state's source: clips in the bank, nodes compile in the
    /// graph (a cyclic graph is refused here, with node names).
    pub fn check_sources(
        &self,
        clips: &ClipBank,
        graph: &AnimGraph,
    ) -> Result<(), StateMachineError> {
        for state in &self.states {
            match state.source {
                Source::Clip(clip) => {
                    if clips.get(clip).is_none() {
                        return Err(StateMachineError::UnknownClip {
                            state: state.name.clone(),
                            clip,
                        });
                    }
                }
                Source::Node(node) => {
                    graph
                        .compile(NodeId(node))
                        .map_err(|error| StateMachineError::BadSource {
                            state: state.name.clone(),
                            error,
                        })?;
                }
            }
        }
        Ok(())
    }

    /// Findings that do not stop a load: unreachable states, dead ends.
    pub fn lints(&self) -> Vec<Lint> {
        let n = self.states.len();
        let mut seen = vec![false; n];
        let mut out_degree = vec![0usize; n];
        let mut stack = Vec::new();
        if let Some(start) = self.state_index(&self.start) {
            seen[start] = true;
            stack.push(start);
        }
        for t in &self.transitions {
            if t.advance != AdvanceMode::Disabled {
                if let Some(a) = self.state_index(&t.from) {
                    out_degree[a] += 1;
                }
            }
        }
        while let Some(a) = stack.pop() {
            for t in &self.transitions {
                if t.advance == AdvanceMode::Disabled || self.states[a].name != t.from {
                    continue;
                }
                if let Some(b) = self.state_index(&t.to) {
                    if !seen[b] {
                        seen[b] = true;
                        stack.push(b);
                    }
                }
            }
        }
        let mut lints = Vec::new();
        for (i, state) in self.states.iter().enumerate() {
            if !seen[i] {
                lints.push(Lint::Unreachable(state.name.clone()));
            }
            if out_degree[i] == 0 && n > 1 {
                lints.push(Lint::DeadEnd(state.name.clone()));
            }
        }
        lints
    }

    /// FNV-1a64 over canonical bytes: field order fixed, lists length-prefixed.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(self.version);
        h.write_str(&self.name);
        h.write_str(&self.start);
        h.write_u32(u32::try_from(self.params.len()).unwrap_or(u32::MAX));
        for p in &self.params {
            h.write_str(&p.name);
            h.write_i64(p.default);
            h.write_i64(p.min);
            h.write_i64(p.max);
        }
        h.write_u32(u32::try_from(self.states.len()).unwrap_or(u32::MAX));
        for s in &self.states {
            h.write_str(&s.name);
            match s.source {
                Source::Clip(c) => {
                    h.write_u8(0);
                    h.write_u32(c);
                }
                Source::Node(n) => {
                    h.write_u8(1);
                    h.write_u32(n);
                }
            }
            h.write_u32(s.length_ticks);
            h.write_bool(s.looping);
        }
        h.write_u32(u32::try_from(self.transitions.len()).unwrap_or(u32::MAX));
        for t in &self.transitions {
            h.write_str(&t.from);
            h.write_str(&t.to);
            h.write_u8(match t.switch {
                SwitchMode::Immediate => 0,
                SwitchMode::Sync => 1,
                SwitchMode::AtEnd => 2,
            });
            h.write_u8(match t.advance {
                AdvanceMode::Disabled => 0,
                AdvanceMode::Enabled => 1,
                AdvanceMode::Auto => 2,
            });
            t.condition.write_canonical(&mut h);
            h.write_u32(t.fade_ticks);
            h.write_u8(match t.curve {
                Curve::Linear => 0,
                Curve::EaseIn => 1,
                Curve::EaseOut => 2,
                Curve::EaseInOut => 3,
            });
            h.write_u32(t.priority);
            h.write_bool(t.reset);
        }
        h.finish()
    }
}

fn cap(what: &'static str, got: usize, max: usize) -> Result<(), StateMachineError> {
    if got > max {
        Err(StateMachineError::TooMany { what, max })
    } else {
        Ok(())
    }
}

/// One compiled transition: indices instead of names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct CTrans {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) switch: SwitchMode,
    pub(crate) advance: AdvanceMode,
    pub(crate) cond: CondOf<usize>,
    pub(crate) fade: u32,
    pub(crate) curve: Curve,
    pub(crate) priority: u32,
    pub(crate) reset: bool,
}

/// Validated machine with indices, ready to share between playbacks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompiledMachine {
    pub(crate) def: StateMachineDef,
    pub(crate) trans: Vec<CTrans>,
    /// Outgoing transition indices per state, sorted by (priority, index).
    pub(crate) out: Vec<Vec<usize>>,
    pub(crate) start: usize,
    pub(crate) digest: u64,
}

impl CompiledMachine {
    /// Validate and compile.
    pub fn new(def: &StateMachineDef) -> Result<Self, StateMachineError> {
        def.validate()?;
        let mut trans = Vec::with_capacity(def.transitions.len());
        for (i, t) in def.transitions.iter().enumerate() {
            let cond = t.condition.try_map(&mut |name: &String| {
                def.param_index(name)
                    .ok_or_else(|| StateMachineError::UnknownParam {
                        transition: i,
                        name: name.clone(),
                    })
            })?;
            let unknown = |name: &String| StateMachineError::UnknownState {
                transition: i,
                name: name.clone(),
            };
            trans.push(CTrans {
                from: def.state_index(&t.from).ok_or_else(|| unknown(&t.from))?,
                to: def.state_index(&t.to).ok_or_else(|| unknown(&t.to))?,
                switch: t.switch,
                advance: t.advance,
                cond,
                fade: t.fade_ticks,
                curve: t.curve,
                priority: t.priority,
                reset: t.reset,
            });
        }
        let mut out = vec![Vec::new(); def.states.len()];
        for (i, t) in trans.iter().enumerate() {
            out[t.from].push(i);
        }
        for list in &mut out {
            list.sort_by_key(|&i| (trans[i].priority, i));
        }
        Ok(Self {
            start: def.state_index(&def.start).unwrap_or(0),
            digest: def.digest(),
            def: def.clone(),
            trans,
            out,
        })
    }

    /// The authored data this was compiled from.
    pub fn def(&self) -> &StateMachineDef {
        &self.def
    }

    /// Digest of the authored data.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// Shortest legal path from state `from` to state `to`, as transition
    /// indices. Cost order: fewest edges, then lowest priority sum, then the
    /// lexicographically smallest index list. Disabled edges are never used;
    /// conditions are ignored. Empty when `from == to`.
    pub fn travel_path(&self, from: usize, to: usize) -> Result<Vec<usize>, PlayError> {
        type Label = (u32, u64, Vec<usize>);
        let n = self.def.states.len();
        let no_path = || PlayError::NoPath {
            from: self
                .def
                .states
                .get(from)
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            to: self
                .def
                .states
                .get(to)
                .map(|s| s.name.clone())
                .unwrap_or_default(),
        };
        if from >= n || to >= n {
            return Err(no_path());
        }
        let mut label: Vec<Option<Label>> = vec![None; n];
        let mut settled = vec![false; n];
        label[from] = Some((0, 0, Vec::new()));
        loop {
            let mut pick: Option<usize> = None;
            for i in 0..n {
                if settled[i] || label[i].is_none() {
                    continue;
                }
                if pick.is_none_or(|p| label[i] < label[p]) {
                    pick = Some(i);
                }
            }
            let Some(u) = pick else { break };
            settled[u] = true;
            if u == to {
                break;
            }
            let (len, prio, path) = label[u].clone().unwrap_or_default();
            for &e in &self.out[u] {
                let t = &self.trans[e];
                if t.advance == AdvanceMode::Disabled || settled[t.to] {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(e);
                let cand: Label = (len + 1, prio + u64::from(t.priority), next_path);
                if label[t.to].as_ref().is_none_or(|old| cand < *old) {
                    label[t.to] = Some(cand);
                }
            }
        }
        label[to].take().map(|l| l.2).ok_or_else(no_path)
    }
}

#[cfg(test)]
mod tests;
#[cfg(all(test, feature = "json"))]
mod tests_json;
