//! bt_trace: the AI decision debugger's tape (the LimboAI idea).
//!
//! One row per node the tree *visited* on a tick: which node, its status, the
//! state or blackboard key it read and the value it saw, and (for a utility
//! pick) the five lane scores and the chosen lane. A person reads the indented
//! tree ([`BtTrace::tree_text`]); an agent reads the one-line-per-node form
//! ([`BtTrace::lines_text`]) or the canonical bytes ([`BtTrace::to_bytes`])
//! the `ai.decisions` door verb takes in `trace` mode.
//!
//! Utility lanes come from the existing decision tape, not a second source:
//! [`BtTrace::note_decisions`] turns each [`Decision`] row of a
//! [`DecisionLog`] into a lane row, so one trace answers both "which node ran"
//! and "why did the selector pick that lane".
//!
//! Off by default and read-only. The tree walker is the same generic function
//! with or without a sink (`Off` compiles to nothing), so the trace cannot
//! change a status, an action or a blackboard value. The ring is one fixed
//! buffer sized at [`BtTrace::new`]; a push overwrites the oldest row and
//! allocates nothing, so a long fight costs one buffer.
//!
//! Bytes are canonical: row count (`u32` LE), then per row tick (`u32`), slot,
//! node index, depth, kind, status, key, then `a`, `b`, `val` (`i32` LE), and
//! five `f32` score words after lane rows only. [`BtTrace::from_bytes`]
//! refuses a short, long, oversized or unknown-tag tape instead of guessing.

use core::fmt;

use crate::bt::{BtBlackboard, BtCond, BtNode, BtState, BtStatus};
use crate::decision_log::{Decision, DecisionLog, ACTION_COUNT as LANES, ACTION_NAMES};

/// Largest ring a trace will hold, and the largest tape it will decode.
pub const MAX_ROWS: usize = 4_096;
/// Bytes per encoded row before the lane scores.
pub const ROW_BYTES: usize = 4 + 6 + 12;
/// Extra bytes a lane row carries: five `f32` score words.
pub const LANE_EXTRA_BYTES: usize = LANES * 4;
/// Largest canonical tape in bytes (every row a lane row).
pub const MAX_TAPE_BYTES: usize = 4 + MAX_ROWS * (ROW_BYTES + LANE_EXTRA_BYTES);
/// Node index a lane row carries, so it sorts after the tree rows of its tick.
pub const LANE_IDX: u8 = 255;

/// Read key: none.
pub const KEY_NONE: u8 = 0;
/// Read key: separation in millimetres.
pub const KEY_DIST: u8 = 1;
/// Read key: the foe-attacking flag.
pub const KEY_FOE: u8 = 2;
/// Read key: the tick.
pub const KEY_TICK: u8 = 3;
/// Read key of blackboard slot 0; slot `n` is `KEY_BB0 + n`.
pub const KEY_BB0: u8 = 16;

/// What a traced node is. The tags are the wire bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// Action leaf; `a` is the action id.
    Action = 1,
    /// `dist <= a`.
    DistLe = 2,
    /// `dist > a`.
    DistGt = 3,
    /// The foe attacks.
    FoeAttacking = 4,
    /// The foe is idle.
    FoeIdle = 5,
    /// `bb[slot] >= a`.
    BbGe = 6,
    /// `bb[slot] < a`.
    BbLt = 7,
    /// `(tick & a) == b`.
    TickMod = 8,
    /// `bb[slot] = a`.
    SetBb = 9,
    /// `bb[slot] += a`.
    AddBb = 10,
    /// Tick gate with period `a`.
    TickGate = 11,
    /// Sequence composite.
    Sequence = 12,
    /// Selector composite.
    Selector = 13,
    /// A utility pick: `a` is the chosen lane, `scores` the five lane scores.
    Lane = 14,
    /// Reactive sequence composite.
    ReactiveSequence = 15,
    /// Reactive fallback composite.
    ReactiveFallback = 16,
}

impl NodeKind {
    /// Decode a wire tag.
    pub const fn from_tag(tag: u8) -> Option<Self> {
        Some(match tag {
            1 => Self::Action,
            2 => Self::DistLe,
            3 => Self::DistGt,
            4 => Self::FoeAttacking,
            5 => Self::FoeIdle,
            6 => Self::BbGe,
            7 => Self::BbLt,
            8 => Self::TickMod,
            9 => Self::SetBb,
            10 => Self::AddBb,
            11 => Self::TickGate,
            12 => Self::Sequence,
            13 => Self::Selector,
            14 => Self::Lane,
            15 => Self::ReactiveSequence,
            16 => Self::ReactiveFallback,
            _ => return None,
        })
    }

    /// Wire tag.
    pub const fn tag(self) -> u8 {
        match self {
            Self::Action => 1,
            Self::DistLe => 2,
            Self::DistGt => 3,
            Self::FoeAttacking => 4,
            Self::FoeIdle => 5,
            Self::BbGe => 6,
            Self::BbLt => 7,
            Self::TickMod => 8,
            Self::SetBb => 9,
            Self::AddBb => 10,
            Self::TickGate => 11,
            Self::Sequence => 12,
            Self::Selector => 13,
            Self::Lane => 14,
            Self::ReactiveSequence => 15,
            Self::ReactiveFallback => 16,
        }
    }
}

/// One visited node on one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceRow {
    /// Tick the tree ran on.
    pub tick: u32,
    /// Fighter slot the tree drove.
    pub slot: u8,
    /// Pre-order index of the node in its tree ([`LANE_IDX`] for a lane row).
    pub idx: u8,
    /// Nesting depth, root at 0.
    pub depth: u8,
    /// What the node is.
    pub kind: NodeKind,
    /// How it ended.
    pub status: BtStatus,
    /// What it read: a `KEY_*` constant, or `KEY_BB0 + slot`.
    pub key: u8,
    /// First payload: threshold, bound, action id, value, delta, mask or period.
    pub a: i32,
    /// Second payload (`TickMod` expect).
    pub b: i32,
    /// The value read at `key` (the value after a write for `SetBb`/`AddBb`).
    pub val: i32,
    /// Utility scores, lane rows only.
    pub scores: [f32; LANES],
}

impl TraceRow {
    /// A blank row to fill a fresh ring.
    pub const EMPTY: Self = Self {
        tick: 0,
        slot: 0,
        idx: 0,
        depth: 0,
        kind: NodeKind::Sequence,
        status: BtStatus::Success,
        key: KEY_NONE,
        a: 0,
        b: 0,
        val: 0,
        scores: [0.0; LANES],
    };

    /// Same row, comparing score bits so a NaN never compares unequal by accident.
    fn same(&self, other: &Self) -> bool {
        self.tick == other.tick
            && self.slot == other.slot
            && self.idx == other.idx
            && self.depth == other.depth
            && self.kind == other.kind
            && self.status == other.status
            && self.key == other.key
            && self.a == other.a
            && self.b == other.b
            && self.val == other.val
            && self
                .scores
                .iter()
                .zip(other.scores.iter())
                .all(|(left, right)| left.to_bits() == right.to_bits())
    }

    /// Display value: the tick key reads as `u32`, everything else as `i32`.
    fn val_text(&self) -> String {
        if self.key == KEY_TICK {
            bits_u32(self.val).to_string()
        } else {
            self.val.to_string()
        }
    }

    /// Key name: `dist_mm`, `foe_attacking`, `tick`, `bb3`, or empty.
    fn key_text(&self) -> String {
        match self.key {
            KEY_NONE => String::new(),
            KEY_DIST => "dist_mm".to_owned(),
            KEY_FOE => "foe_attacking".to_owned(),
            KEY_TICK => "tick".to_owned(),
            other => format!("bb{}", other.saturating_sub(KEY_BB0)),
        }
    }

    /// Node label with its parameters, e.g. `bb0>=24` or `action attack`.
    fn label(&self) -> String {
        let slot = self.key.saturating_sub(KEY_BB0);
        match self.kind {
            NodeKind::Action => format!(
                "action {}",
                usize::try_from(self.a)
                    .ok()
                    .and_then(|id| ACTION_NAMES.get(id))
                    .copied()
                    .unwrap_or("unknown")
            ),
            NodeKind::DistLe => format!("dist<={}", self.a),
            NodeKind::DistGt => format!("dist>{}", self.a),
            NodeKind::FoeAttacking => "foe_attacking".to_owned(),
            NodeKind::FoeIdle => "foe_idle".to_owned(),
            NodeKind::BbGe => format!("bb{slot}>={}", self.a),
            NodeKind::BbLt => format!("bb{slot}<{}", self.a),
            NodeKind::TickMod => format!("tick&{}=={}", bits_u32(self.a), bits_u32(self.b)),
            NodeKind::SetBb => format!("set bb{slot}={}", self.a),
            NodeKind::AddBb => format!("add bb{slot}+={}", self.a),
            NodeKind::TickGate => format!("tick_gate {}", bits_u32(self.a)),
            NodeKind::Sequence => "sequence".to_owned(),
            NodeKind::Selector => "selector".to_owned(),
            NodeKind::ReactiveSequence => "reactive_sequence".to_owned(),
            NodeKind::ReactiveFallback => "reactive_fallback".to_owned(),
            NodeKind::Lane => format!(
                "lane chose {}",
                usize::try_from(self.a)
                    .ok()
                    .and_then(|id| ACTION_NAMES.get(id))
                    .copied()
                    .unwrap_or("unknown")
            ),
        }
    }

    /// `[key=value]` for a read node, `scores=[..]` for a lane row, else empty.
    fn detail(&self) -> String {
        if self.kind == NodeKind::Lane {
            return format!(
                "scores=[{:.1}, {:.1}, {:.1}, {:.1}, {:.1}]",
                self.scores[0], self.scores[1], self.scores[2], self.scores[3], self.scores[4]
            );
        }
        if self.key == KEY_NONE {
            return String::new();
        }
        format!("[{}={}]", self.key_text(), self.val_text())
    }
}

/// `u32` stored in an `i32` field bit for bit (masks, periods, ticks).
pub const fn bits_i32(value: u32) -> i32 {
    i32::from_le_bytes(value.to_le_bytes())
}

/// The `u32` an `i32` field holds bit for bit.
pub const fn bits_u32(value: i32) -> u32 {
    u32::from_le_bytes(value.to_le_bytes())
}

fn status_tag(status: BtStatus) -> u8 {
    match status {
        BtStatus::Success => 0,
        BtStatus::Failure => 1,
        BtStatus::Running => 2,
    }
}

fn status_from_tag(tag: u8) -> Option<BtStatus> {
    match tag {
        0 => Some(BtStatus::Success),
        1 => Some(BtStatus::Failure),
        2 => Some(BtStatus::Running),
        _ => None,
    }
}

fn status_word(status: BtStatus) -> &'static str {
    match status {
        BtStatus::Success => "success",
        BtStatus::Failure => "failure",
        BtStatus::Running => "running",
    }
}

fn status_letter(status: BtStatus) -> char {
    match status {
        BtStatus::Success => 'S',
        BtStatus::Failure => 'F',
        BtStatus::Running => 'R',
    }
}

/// Where the tree walker reports each visited node. `ON` is a constant, so a
/// walker built with [`Off`] carries none of the trace code.
pub(crate) trait Sink {
    /// True when the sink records.
    const ON: bool;
    /// One node finished with `status`. Read-only over state and blackboard.
    fn visit(
        &mut self,
        node: &BtNode,
        idx: u8,
        depth: u8,
        status: BtStatus,
        state: &BtState,
        bb: &BtBlackboard,
    );
}

/// The no-trace sink: the default tick path.
pub(crate) struct Off;

impl Sink for Off {
    const ON: bool = false;
    fn visit(&mut self, _: &BtNode, _: u8, _: u8, _: BtStatus, _: &BtState, _: &BtBlackboard) {}
}

/// A trace plus the slot its rows are filed under.
pub(crate) struct Tap<'a> {
    /// Ring the rows land in.
    pub(crate) trace: &'a mut BtTrace,
    /// Fighter slot of the tree being ticked.
    pub(crate) slot: u8,
}

impl Sink for Tap<'_> {
    const ON: bool = true;
    fn visit(
        &mut self,
        node: &BtNode,
        idx: u8,
        depth: u8,
        status: BtStatus,
        state: &BtState,
        bb: &BtBlackboard,
    ) {
        let mut row = TraceRow {
            tick: state.tick,
            slot: self.slot,
            idx,
            depth,
            status,
            ..TraceRow::EMPTY
        };
        let slot_key = |slot: u8| KEY_BB0.saturating_add(slot);
        let slot_val = |slot: u8| bb.get(usize::from(slot)).unwrap_or(0);
        match node {
            BtNode::Action(id) => {
                row.kind = NodeKind::Action;
                row.a = i32::from(*id);
            }
            BtNode::Cond(cond) => match *cond {
                BtCond::DistLe(bound) => {
                    row.kind = NodeKind::DistLe;
                    row.a = bound;
                    row.key = KEY_DIST;
                    row.val = state.dist_mm;
                }
                BtCond::DistGt(bound) => {
                    row.kind = NodeKind::DistGt;
                    row.a = bound;
                    row.key = KEY_DIST;
                    row.val = state.dist_mm;
                }
                BtCond::FoeAttacking => {
                    row.kind = NodeKind::FoeAttacking;
                    row.key = KEY_FOE;
                    row.val = i32::from(state.foe_attacking);
                }
                BtCond::FoeIdle => {
                    row.kind = NodeKind::FoeIdle;
                    row.key = KEY_FOE;
                    row.val = i32::from(state.foe_attacking);
                }
                BtCond::BbGe { slot, threshold } => {
                    row.kind = NodeKind::BbGe;
                    row.a = threshold;
                    row.key = slot_key(slot);
                    row.val = slot_val(slot);
                }
                BtCond::BbLt { slot, threshold } => {
                    row.kind = NodeKind::BbLt;
                    row.a = threshold;
                    row.key = slot_key(slot);
                    row.val = slot_val(slot);
                }
                BtCond::TickMod { mask, expect } => {
                    row.kind = NodeKind::TickMod;
                    row.a = bits_i32(mask);
                    row.b = bits_i32(expect);
                    row.key = KEY_TICK;
                    row.val = bits_i32(state.tick);
                }
            },
            BtNode::SetBb { slot, value } => {
                row.kind = NodeKind::SetBb;
                row.a = *value;
                row.key = slot_key(*slot);
                row.val = slot_val(*slot);
            }
            BtNode::AddBb { slot, delta } => {
                row.kind = NodeKind::AddBb;
                row.a = *delta;
                row.key = slot_key(*slot);
                row.val = slot_val(*slot);
            }
            BtNode::TickGate { period } => {
                row.kind = NodeKind::TickGate;
                row.a = bits_i32(*period);
                row.key = KEY_TICK;
                row.val = bits_i32(state.tick);
            }
            BtNode::Sequence(_) => row.kind = NodeKind::Sequence,
            BtNode::Selector(_) => row.kind = NodeKind::Selector,
            BtNode::ReactiveSequence(_) => row.kind = NodeKind::ReactiveSequence,
            BtNode::ReactiveFallback(_) => row.kind = NodeKind::ReactiveFallback,
        }
        self.trace.push(row);
    }
}

/// Fixed-capacity ring of [`TraceRow`]s; the newest rows survive.
///
/// Equality compares the rows only: capacity and the dropped count are buffer
/// choices, not tape content.
#[derive(Clone, Debug)]
pub struct BtTrace {
    rows: Box<[TraceRow]>,
    start: usize,
    len: usize,
    dropped: u64,
}

impl PartialEq for BtTrace {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self
                .iter()
                .zip(other.iter())
                .all(|(left, right)| left.same(right))
    }
}

impl BtTrace {
    /// New ring holding at most `cap` rows, clamped to `1..=MAX_ROWS`. This is
    /// the only allocation; every later push reuses the buffer.
    pub fn new(cap: usize) -> Self {
        Self {
            rows: vec![TraceRow::EMPTY; cap.clamp(1, MAX_ROWS)].into_boxed_slice(),
            start: 0,
            len: 0,
            dropped: 0,
        }
    }

    /// Capacity in rows.
    pub fn cap(&self) -> usize {
        self.rows.len()
    }

    /// Rows held.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when no row was recorded.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Rows pushed out of the ring by newer ones.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Forget every row; the buffer stays.
    pub fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
        self.dropped = 0;
    }

    /// Record one row, overwriting the oldest when full. Allocates nothing.
    pub fn push(&mut self, row: TraceRow) {
        let cap = self.rows.len();
        if self.len < cap {
            let at = (self.start + self.len) % cap;
            self.rows[at] = row;
            self.len += 1;
        } else {
            self.rows[self.start] = row;
            self.start = (self.start + 1) % cap;
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Rows held, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &TraceRow> {
        let cap = self.rows.len();
        (0..self.len).map(move |offset| &self.rows[(self.start + offset) % cap])
    }

    /// Rows for one slot (or all) with `from <= tick <= to`, oldest first.
    pub fn window(&self, slot: Option<u8>, from: u32, to: u32) -> impl Iterator<Item = &TraceRow> {
        self.iter().filter(move |row| {
            slot.is_none_or(|wanted| row.slot == wanted) && row.tick >= from && row.tick <= to
        })
    }

    /// File one utility pick as a lane row (read-only over the decision).
    pub fn note_decision(&mut self, decision: &Decision) {
        self.push(TraceRow {
            tick: decision.tick,
            slot: decision.fighter,
            idx: LANE_IDX,
            depth: 0,
            kind: NodeKind::Lane,
            status: BtStatus::Success,
            a: i32::from(decision.chosen),
            scores: decision.scores,
            ..TraceRow::EMPTY
        });
    }

    /// File every row of the existing decision tape as a lane row.
    pub fn note_decisions(&mut self, log: &DecisionLog) {
        for decision in log.rows() {
            self.note_decision(decision);
        }
    }

    /// Canonical bytes, see the module docs.
    pub fn to_bytes(&self) -> Vec<u8> {
        let count = u32::try_from(self.len).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(4 + self.len * ROW_BYTES);
        out.extend_from_slice(&count.to_le_bytes());
        for row in self.iter() {
            out.extend_from_slice(&row.tick.to_le_bytes());
            out.push(row.slot);
            out.push(row.idx);
            out.push(row.depth);
            out.push(row.kind.tag());
            out.push(status_tag(row.status));
            out.push(row.key);
            out.extend_from_slice(&row.a.to_le_bytes());
            out.extend_from_slice(&row.b.to_le_bytes());
            out.extend_from_slice(&row.val.to_le_bytes());
            if row.kind == NodeKind::Lane {
                for score in row.scores {
                    out.extend_from_slice(&score.to_bits().to_le_bytes());
                }
            }
        }
        out
    }

    /// Decode canonical bytes. Refuses a short tape, trailing bytes, a count
    /// above [`MAX_ROWS`] and unknown kind or status tags.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TraceError> {
        let head = bytes.get(..4).ok_or(TraceError::Short {
            needed: 4,
            got: bytes.len(),
        })?;
        let count = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let count_rows = usize::try_from(count).unwrap_or(usize::MAX);
        if count_rows > MAX_ROWS {
            return Err(TraceError::TooManyRows(count));
        }
        let mut trace = Self::new(count_rows.max(1));
        let mut at = 4usize;
        for _ in 0..count_rows {
            let word = |from: usize| -> Result<[u8; 4], TraceError> {
                bytes
                    .get(from..from + 4)
                    .map(|lane| [lane[0], lane[1], lane[2], lane[3]])
                    .ok_or(TraceError::Short {
                        needed: from + 4,
                        got: bytes.len(),
                    })
            };
            let tick = u32::from_le_bytes(word(at)?);
            let tags = bytes.get(at + 4..at + 10).ok_or(TraceError::Short {
                needed: at + 10,
                got: bytes.len(),
            })?;
            let kind = NodeKind::from_tag(tags[3]).ok_or(TraceError::BadKind(tags[3]))?;
            let status = status_from_tag(tags[4]).ok_or(TraceError::BadStatus(tags[4]))?;
            let mut row = TraceRow {
                tick,
                slot: tags[0],
                idx: tags[1],
                depth: tags[2],
                kind,
                status,
                key: tags[5],
                a: i32::from_le_bytes(word(at + 10)?),
                b: i32::from_le_bytes(word(at + 14)?),
                val: i32::from_le_bytes(word(at + 18)?),
                scores: [0.0; LANES],
            };
            at += ROW_BYTES;
            if kind == NodeKind::Lane {
                for (lane, score) in row.scores.iter_mut().enumerate() {
                    *score = f32::from_bits(u32::from_le_bytes(word(at + lane * 4)?));
                }
                at += LANE_EXTRA_BYTES;
            }
            trace.push(row);
        }
        if at < bytes.len() {
            return Err(TraceError::Long {
                expected: at,
                got: bytes.len(),
            });
        }
        Ok(trace)
    }

    /// Selected rows in reading order: by tick, then slot, then pre-order
    /// index (lane rows last), capped at `limit` rows. Second value counts the
    /// rows the cap cut.
    fn ordered(
        &self,
        slot: Option<u8>,
        from: u32,
        to: u32,
        limit: usize,
    ) -> (Vec<&TraceRow>, usize) {
        let mut rows: Vec<&TraceRow> = self.window(slot, from, to).collect();
        rows.sort_by_key(|row| (row.tick, row.slot, row.idx));
        let cut = rows.len().saturating_sub(limit);
        rows.truncate(limit);
        (rows, cut)
    }

    /// Indented tree per tick and slot, for a person. `limit` caps the rows.
    pub fn tree_text(&self, slot: Option<u8>, from: u32, to: u32, limit: usize) -> String {
        let (rows, cut) = self.ordered(slot, from, to, limit);
        let mut out = String::new();
        let mut group = None;
        for row in rows {
            if group != Some((row.tick, row.slot)) {
                out.push_str(&format!("tick {} slot {}\n", row.tick, row.slot));
                group = Some((row.tick, row.slot));
            }
            let indent = "  ".repeat(usize::from(row.depth) + 1);
            if row.kind == NodeKind::Lane {
                out.push_str(&format!("{indent}{} {}\n", row.label(), row.detail()));
            } else {
                let detail = row.detail();
                let gap = if detail.is_empty() { "" } else { " " };
                out.push_str(&format!(
                    "{indent}{} -> {}{gap}{detail}\n",
                    row.label(),
                    status_word(row.status)
                ));
            }
        }
        if cut > 0 {
            out.push_str(&format!("... {cut} more row(s) cut by the limit\n"));
        }
        out
    }

    /// One line per row (`tick slot #idx depth label status reads`), for an
    /// agent. `limit` caps the rows.
    pub fn lines_text(&self, slot: Option<u8>, from: u32, to: u32, limit: usize) -> String {
        let (rows, cut) = self.ordered(slot, from, to, limit);
        let mut out = String::new();
        for row in rows {
            let detail = row.detail();
            let gap = if detail.is_empty() { "" } else { " " };
            if row.kind == NodeKind::Lane {
                out.push_str(&format!(
                    "t{} s{} lane {}{gap}{detail}\n",
                    row.tick,
                    row.slot,
                    row.label().trim_start_matches("lane "),
                ));
            } else {
                out.push_str(&format!(
                    "t{} s{} #{} d{} {} {}{gap}{detail}\n",
                    row.tick,
                    row.slot,
                    row.idx,
                    row.depth,
                    row.label(),
                    status_letter(row.status),
                ));
            }
        }
        if cut > 0 {
            out.push_str(&format!("... {cut} more row(s) cut by the limit\n"));
        }
        out
    }
}

/// Why a trace tape was refused. The door maps each to a static code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceError {
    /// Fewer bytes than the header and rows promise.
    Short {
        /// Bytes the tape needs at least.
        needed: usize,
        /// Bytes it carries.
        got: usize,
    },
    /// Bytes after the last promised row.
    Long {
        /// Bytes the tape promises.
        expected: usize,
        /// Bytes it carries.
        got: usize,
    },
    /// Row count above [`MAX_ROWS`].
    TooManyRows(u32),
    /// Node kind tag unknown.
    BadKind(u8),
    /// Status tag unknown.
    BadStatus(u8),
}

impl fmt::Display for TraceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { needed, got } => {
                write!(formatter, "trace is short: needs {needed} bytes, has {got}")
            }
            Self::Long { expected, got } => write!(
                formatter,
                "trace has trailing bytes: expected {expected}, has {got}"
            ),
            Self::TooManyRows(count) => {
                write!(formatter, "trace holds {count} rows, cap is {MAX_ROWS}")
            }
            Self::BadKind(tag) => write!(formatter, "trace node kind tag {tag} is unknown"),
            Self::BadStatus(tag) => write!(formatter, "trace status tag {tag} is unknown"),
        }
    }
}

impl std::error::Error for TraceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bt::{guard_tree, pressure_tree, BtTree, ACTION_ATTACK};
    use crate::gladiator::{Brain, Percept};
    use engine_core::alloc_probe::measure;
    use engine_input::FighterId;

    fn state(tick: u32, dist_mm: i32, foe: bool) -> BtState {
        BtState::new(dist_mm, foe, 500, tick)
    }

    /// Tick one tree for `ticks` ticks over a fixed state stream, tracing or not.
    /// Returns (statuses, actions, final blackboard hash).
    fn run(tree: &BtTree, ticks: u32, mut trace: Option<&mut BtTrace>) -> (Vec<u8>, Vec<u8>, u64) {
        let mut bb = BtBlackboard::new();
        let mut out = 3u8;
        let mut statuses = Vec::new();
        let mut actions = Vec::new();
        for tick in 0..ticks {
            let dist = i32::try_from((tick * 733) % 11_000).unwrap_or(0);
            let st = state(tick, dist, tick % 3 == 0);
            let status = tree.tick_with(&st, &mut bb, &mut out, 0, trace.as_deref_mut());
            statuses.push(status_tag(status));
            actions.push(out);
        }
        (statuses, actions, bb.state_hash())
    }

    #[test]
    fn a_traced_tick_names_the_nodes_statuses_and_reads() {
        let tree = pressure_tree();
        let mut bb = BtBlackboard::new();
        assert!(bb.set(0, 3));
        let mut out = 0u8;
        let mut trace = BtTrace::new(64);
        let status = tree.tick_with(
            &state(7, 1_800, false),
            &mut bb,
            &mut out,
            1,
            Some(&mut trace),
        );
        assert_eq!(status, BtStatus::Success);
        assert_eq!(out, ACTION_ATTACK);
        let rows: Vec<&TraceRow> = trace.iter().collect();
        // Post-order: bb0>=24 fails first, with the value it saw.
        let first = rows[0];
        assert_eq!(
            (first.kind, first.status),
            (NodeKind::BbGe, BtStatus::Failure)
        );
        assert_eq!((first.key, first.val, first.a), (KEY_BB0, 3, 24));
        assert_eq!(
            (first.idx, first.depth, first.slot, first.tick),
            (2, 2, 1, 7)
        );
        // The attack lane ran its add and its action; the add shows the new value.
        let add = rows.iter().find(|row| row.kind == NodeKind::AddBb).unwrap();
        assert_eq!(
            (add.key, add.val, add.status),
            (KEY_BB0, 4, BtStatus::Success)
        );
        let dist = rows
            .iter()
            .find(|row| row.kind == NodeKind::DistLe && row.val == 1_800);
        assert!(dist.is_some(), "the distance read carries the value seen");
        let root = rows.last().unwrap();
        assert_eq!(
            (root.kind, root.idx, root.depth),
            (NodeKind::Selector, 0, 0)
        );
        assert_eq!(root.status, BtStatus::Success);
        // The foe-idle branch never ran the dodge fallback.
        assert!(rows
            .iter()
            .all(|row| !(row.kind == NodeKind::Action && row.a == 2)));
    }

    #[test]
    fn tracing_never_changes_a_decision_or_a_hash() {
        for tree in [pressure_tree(), guard_tree()] {
            let off = run(&tree, 600, None);
            let mut trace = BtTrace::new(256);
            let on = run(&tree, 600, Some(&mut trace));
            assert_eq!(off, on, "statuses, actions and blackboard match");
            assert!(
                !trace.is_empty() && trace.dropped() > 0,
                "the trace did record"
            );
        }
    }

    #[test]
    fn a_traced_bout_keeps_the_same_tape_digest() {
        let (a, b) = (pressure_tree(), guard_tree());
        for seed in [1u64, 7, 0xd57b_5c8a] {
            let plain = crate::bout::run_bout(&a, &b, 3, 400, seed);
            let mut trace = BtTrace::new(512);
            let traced = crate::bout::run_bout_traced(&a, &b, 3, 400, seed, &mut trace);
            assert_eq!(plain, traced, "same tape with the trace on and off");
            assert_eq!(plain.digest, traced.digest);
            assert!(trace.window(Some(0), 0, 400).count() > 0);
            assert!(trace.window(Some(1), 0, 400).count() > 0);
        }
    }

    #[test]
    fn the_ring_is_bounded_and_ticking_allocates_nothing() {
        let tree = pressure_tree();
        let mut trace = BtTrace::new(32);
        let before = trace.rows.as_ptr();
        let mut bb = BtBlackboard::new();
        let mut out = 0u8;
        let ((), counted) = measure(|| {
            for tick in 0..500u32 {
                let _ = tree.tick_with(
                    &state(tick, 1_500, tick % 2 == 0),
                    &mut bb,
                    &mut out,
                    0,
                    Some(&mut trace),
                );
            }
        });
        assert_eq!(counted.calls, 0, "a traced tick allocates nothing");
        assert_eq!(trace.len(), 32);
        assert_eq!(trace.cap(), 32);
        assert!(trace.dropped() > 0);
        assert_eq!(trace.rows.as_ptr(), before, "the ring never moved");
        let newest = trace.iter().last().unwrap();
        assert_eq!(newest.tick, 499, "the newest rows survive");
        let oldest_tick = trace.iter().next().unwrap().tick;
        assert!(
            oldest_tick > 480,
            "the oldest rows were dropped: {oldest_tick}"
        );
    }

    #[test]
    fn the_default_tick_records_nothing() {
        let tree = pressure_tree();
        let mut trace = BtTrace::new(8);
        let mut bb = BtBlackboard::new();
        let mut out = 0u8;
        let _ = tree.tick(&state(1, 1_000, false), &mut bb, &mut out);
        let _ = tree.tick_with(&state(2, 1_000, false), &mut bb, &mut out, 0, None);
        assert!(trace.is_empty(), "off by default: nothing reaches a trace");
        trace.clear();
    }

    #[test]
    fn bytes_round_trip_and_bad_tapes_are_refused() {
        let tree = guard_tree();
        let mut trace = BtTrace::new(128);
        let _ = run(&tree, 20, Some(&mut trace));
        let mut log = DecisionLog::new(4);
        log.push(Decision {
            tick: 5,
            fighter: 0,
            scores: [9.0, 1.0, 0.5, 0.0, 0.0],
            chosen: 0,
        });
        trace.note_decisions(&log);
        let bytes = trace.to_bytes();
        let back = BtTrace::from_bytes(&bytes).expect("round trip");
        assert_eq!(back, trace);
        assert_eq!(back.to_bytes(), bytes, "canonical: same rows, same bytes");

        assert!(matches!(
            BtTrace::from_bytes(&bytes[..bytes.len() - 1]),
            Err(TraceError::Short { .. })
        ));
        let mut long = bytes.clone();
        long.push(0);
        assert!(matches!(
            BtTrace::from_bytes(&long),
            Err(TraceError::Long { .. })
        ));
        assert!(matches!(
            BtTrace::from_bytes(&[0xff, 0xff, 0xff, 0xff]),
            Err(TraceError::TooManyRows(_))
        ));
        assert!(matches!(
            BtTrace::from_bytes(&[1]),
            Err(TraceError::Short { .. })
        ));
        let mut bad_kind = bytes.clone();
        bad_kind[4 + 7] = 99;
        assert_eq!(BtTrace::from_bytes(&bad_kind), Err(TraceError::BadKind(99)));
        let mut bad_status = bytes;
        bad_status[4 + 8] = 9;
        assert_eq!(
            BtTrace::from_bytes(&bad_status),
            Err(TraceError::BadStatus(9))
        );
    }

    #[test]
    fn the_tree_text_indents_by_depth_and_shows_reads() {
        let tree = pressure_tree();
        let mut bb = BtBlackboard::new();
        let mut out = 0u8;
        let mut trace = BtTrace::new(64);
        let _ = tree.tick_with(
            &state(7, 1_800, false),
            &mut bb,
            &mut out,
            0,
            Some(&mut trace),
        );
        let text = trace.tree_text(Some(0), 0, 100, 100);
        assert!(text.starts_with("tick 7 slot 0\n"), "{text}");
        assert!(text.contains("\n  selector -> success\n"), "{text}");
        assert!(text.contains("\n    sequence -> failure\n"), "{text}");
        assert!(
            text.contains("\n      bb0>=24 -> failure [bb0=0]\n"),
            "{text}"
        );
        assert!(
            text.contains("\n      dist<=2500 -> success [dist_mm=1800]\n"),
            "{text}"
        );
        assert!(text.contains("action attack -> success"), "{text}");
        // Pre-order: the selector line comes before its children.
        assert!(text.find("selector").unwrap() < text.find("bb0>=24").unwrap());

        let lines = trace.lines_text(Some(0), 7, 7, 100);
        assert!(lines.contains("t7 s0 #2 d2 bb0>=24 F [bb0=0]\n"), "{lines}");
        let none = trace.lines_text(Some(1), 0, 100, 100);
        assert!(none.is_empty(), "another slot has no rows");
        let capped = trace.lines_text(Some(0), 0, 100, 2);
        assert!(capped.contains("more row(s) cut by the limit"), "{capped}");
    }

    #[test]
    fn lane_rows_come_from_the_existing_decision_tape() {
        let mut brain = Brain::new(FighterId(2), 1);
        brain.record_decisions(8);
        for tick in 0..4u32 {
            brain.decide(&Percept::new(
                [0.0, 0.0, 0.0],
                0.0,
                [1.4, 0.0, 0.0],
                true,
                1.4,
                tick,
            ));
        }
        let log = brain.decisions().expect("recording is on");
        let mut trace = BtTrace::new(32);
        trace.note_decisions(log);
        assert_eq!(trace.len(), log.len());
        let first = log.rows().next().unwrap();
        let lane = trace.iter().next().unwrap();
        assert_eq!(lane.kind, NodeKind::Lane);
        assert_eq!(lane.scores, first.scores);
        assert_eq!(lane.a, i32::from(first.chosen));
        let text = trace.tree_text(Some(2), 0, 10, 100);
        assert!(text.contains("lane chose "), "{text}");
        assert!(text.contains("scores=["), "{text}");
    }
}
