//! bt: serializable integer behavior trees for bout bots.
//! Owner: this module owns versioned trees over integer state.
//! Units: millimetres, permille, ticks; actions are small ids.
//! Determinism: same tree plus same states give same actions;
//! tick order is visit order, no clock, no float.
//! Errors: typed decode and build failures, never panics.
//! Budget: one decision inside 100 micros; tick allocates nothing.

use engine_core::Fnv64;

/// Tree encoding version. One byte, checked on decode.
pub const BT_VERSION: u8 = 1;
/// Largest tree in nodes. Bounds decode and build time.
pub const MAX_NODES: usize = 64;
/// Deepest nesting in nodes. Bounds decode recursion.
pub const MAX_DEPTH: usize = 8;
/// Largest child list on one composite. Empty lists stay legal.
pub const MAX_CHILDREN: usize = 16;
/// Integer blackboard slots per brain.
pub const BB_SLOTS: usize = 8;
/// Legal action ids are `0..ACTION_COUNT`.
pub const ACTION_COUNT: u8 = 6;
/// Close strike.
pub const ACTION_ATTACK: u8 = 0;
/// Hold guard.
pub const ACTION_BLOCK: u8 = 1;
/// Sidestep.
pub const ACTION_DODGE: u8 = 2;
/// Close distance.
pub const ACTION_ADVANCE: u8 = 3;
/// Give ground.
pub const ACTION_RETREAT: u8 = 4;
/// Wander strafe (anti-stillness).
pub const ACTION_REPOSITION: u8 = 5;
/// Decision budget in micros (0.1 ms).
pub const DECISION_BUDGET_MICROS: u64 = 100;

/// Integer percept for one decision. All fields are integers; no float
/// ever enters the tick. Units: millimetres, flag, permille, ticks.
/// Determinism: pure data; same bytes give same decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtState {
    /// Separation in millimetres, clamped to `0..=60_000`.
    pub dist_mm: i32,
    /// `1` while the foe attacks, else `0`.
    pub foe_attacking: u8,
    /// Stamina in permille, clamped to `0..=1000`.
    pub stamina_permille: u16,
    /// Fixed tick driving gated nodes.
    pub tick: u32,
}

impl BtState {
    /// Build a clamped state. Never panics; out-of-range inputs clamp.
    /// Units: millimetres, flag, permille, ticks.
    pub const fn new(dist_mm: i32, foe_attacking: bool, stamina_permille: u16, tick: u32) -> Self {
        let dist = if dist_mm < 0 {
            0
        } else if dist_mm > 60_000 {
            60_000
        } else {
            dist_mm
        };
        let foe = if foe_attacking { 1 } else { 0 };
        let stamina = if stamina_permille > 1000 {
            1000
        } else {
            stamina_permille
        };
        Self {
            dist_mm: dist,
            foe_attacking: foe,
            stamina_permille: stamina,
            tick,
        }
    }

    /// FNV-1a64 over the four fields, little-endian. Same state gives
    /// the same digest on every platform. Units: hash over integers.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_i32(self.dist_mm);
        hasher.write_u8(self.foe_attacking);
        hasher.write_u16(self.stamina_permille);
        hasher.write_u32(self.tick);
        hasher.finish()
    }
}

/// Integer blackboard: fixed slots, no keys, no heap. The tick reads
/// and writes slots through checked accessors; out-of-range slots
/// fail the node instead of panicking. Units: plain integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtBlackboard {
    slots: [i32; BB_SLOTS],
}

impl BtBlackboard {
    /// Empty blackboard (all slots zero). Determinism: fixed start.
    pub const fn new() -> Self {
        Self {
            slots: [0; BB_SLOTS],
        }
    }

    /// Read a slot. `None` when out of range. Never panics.
    /// Units: slot index plus integer value.
    pub fn get(&self, slot: usize) -> Option<i32> {
        if slot < BB_SLOTS {
            Some(self.slots[slot])
        } else {
            None
        }
    }

    /// Write a slot with saturation to `+-1_000_000`. `false` when out
    /// of range. Never panics. Units: slot index plus integer value.
    pub fn set(&mut self, slot: usize, value: i32) -> bool {
        if slot < BB_SLOTS {
            self.slots[slot] = value.clamp(-1_000_000, 1_000_000);
            true
        } else {
            false
        }
    }

    /// Saturating add on a slot. `false` when out of range. Never
    /// panics. Units: slot index plus integer delta.
    pub fn add(&mut self, slot: usize, delta: i32) -> bool {
        if slot < BB_SLOTS {
            self.slots[slot] = self.slots[slot]
                .saturating_add(delta)
                .clamp(-1_000_000, 1_000_000);
            true
        } else {
            false
        }
    }

    /// FNV-1a64 over slots in order. Same slots give the same digest.
    /// Units: hash over integers.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        for slot in self.slots {
            hasher.write_i32(slot);
        }
        hasher.finish()
    }
}

impl Default for BtBlackboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Tick status. `Running` resumes next tick and never blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtStatus {
    /// Node succeeded.
    Success,
    /// Node failed.
    Failure,
    /// Node needs more ticks.
    Running,
}

/// Integer condition over state and blackboard. Pure, total and
/// allocation-free. Units: millimetres, slots, ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtCond {
    /// True while separation is at or under the bound.
    DistLe(i32),
    /// True while separation is above the bound.
    DistGt(i32),
    /// True while the foe attacks.
    FoeAttacking,
    /// True while the foe is idle.
    FoeIdle,
    /// True while blackboard slot holds at least the threshold.
    BbGe { slot: u8, threshold: i32 },
    /// True while blackboard slot holds below the threshold.
    BbLt { slot: u8, threshold: i32 },
    /// True while `(tick & mask) == expect`.
    TickMod { mask: u32, expect: u32 },
}

impl BtCond {
    /// Evaluate over integer state. Out-of-range slots read as false.
    /// Never panics. Units: millimetres, slots, ticks.
    pub fn eval(&self, state: &BtState, bb: &BtBlackboard) -> bool {
        match *self {
            BtCond::DistLe(bound) => state.dist_mm <= bound,
            BtCond::DistGt(bound) => state.dist_mm > bound,
            BtCond::FoeAttacking => state.foe_attacking == 1,
            BtCond::FoeIdle => state.foe_attacking == 0,
            BtCond::BbGe { slot, threshold } => {
                let index = usize::from(slot);
                match bb.get(index) {
                    Some(value) => value >= threshold,
                    None => false,
                }
            }
            BtCond::BbLt { slot, threshold } => {
                let index = usize::from(slot);
                match bb.get(index) {
                    Some(value) => value < threshold,
                    None => false,
                }
            }
            BtCond::TickMod { mask, expect } => (state.tick & mask) == expect,
        }
    }
}

/// One tree node. Leaf payloads are integers; composites own ordered
/// children. Trees are built cold (allocation at build is fine); the
/// tick only borrows. Units: action ids, slots, ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BtNode {
    /// Decide an action id (`0..ACTION_COUNT`); writes the output slot.
    Action(u8),
    /// Branch on an integer condition; touches no output.
    Cond(BtCond),
    /// Write a blackboard slot; fails when the slot is out of range.
    SetBb { slot: u8, value: i32 },
    /// Saturating add on a blackboard slot; fails when out of range.
    AddBb { slot: u8, delta: i32 },
    /// Succeed when the tick lands on the period, else `Running`.
    /// A zero period always fails (decode and build refuse it).
    TickGate { period: u32 },
    /// Run children in order; fail fast, propagate `Running`.
    Sequence(Vec<BtNode>),
    /// Try children in order; succeed fast, propagate `Running`.
    Selector(Vec<BtNode>),
}

impl BtNode {
    /// Count nodes in this subtree. Pure integer walk, no heap use.
    /// Units: node count.
    pub fn node_count(&self) -> usize {
        match self {
            BtNode::Action(_)
            | BtNode::Cond(_)
            | BtNode::SetBb { .. }
            | BtNode::AddBb { .. }
            | BtNode::TickGate { .. } => 1,
            BtNode::Sequence(kids) | BtNode::Selector(kids) => {
                let mut total = 1usize;
                for kid in kids {
                    total = total.saturating_add(kid.node_count());
                }
                total
            }
        }
    }

    /// Nesting depth with this node at 1. Pure walk, no heap use.
    /// Units: depth in nodes.
    pub fn depth(&self) -> usize {
        match self {
            BtNode::Action(_)
            | BtNode::Cond(_)
            | BtNode::SetBb { .. }
            | BtNode::AddBb { .. }
            | BtNode::TickGate { .. } => 1,
            BtNode::Sequence(kids) | BtNode::Selector(kids) => {
                let mut best = 0usize;
                for kid in kids {
                    let child = kid.depth();
                    if child > best {
                        best = child;
                    }
                }
                best.saturating_add(1)
            }
        }
    }
}

/// Versioned behavior tree. The root owns the whole brain; encode and
/// decode move it as bytes with a version prefix. Units: nodes.
/// Determinism: same tree plus same state stream gives byte-identical
/// decisions. Errors: [`BtError`] on bad builds and bad bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtTree {
    root: BtNode,
}

/// Build and decode failures. Typed, never strings. Units: tags and
/// counts carried as integers. Errors: returned, never panicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtError {
    /// No bytes to read.
    Empty,
    /// Version byte differs from the shipped one; culprit carried.
    BadVersion {
        /// Seen version byte.
        got: u8,
    },
    /// Node tag is unknown; culprit carried.
    BadTag {
        /// Seen tag byte.
        tag: u8,
    },
    /// Condition tag is unknown; culprit carried.
    BadCond {
        /// Seen condition tag.
        tag: u8,
    },
    /// Bytes end mid-node.
    Truncated,
    /// Bytes remain after one tree; remainder length carried.
    Trailing {
        /// Leftover byte count.
        left: usize,
    },
    /// Tree holds more than the node cap; count carried.
    TooManyNodes {
        /// Counted nodes.
        got: usize,
    },
    /// Tree nests deeper than the depth cap; depth carried.
    TooDeep {
        /// Measured depth.
        got: usize,
    },
    /// Action id is outside `0..ACTION_COUNT`; culprit carried.
    BadAction {
        /// Seen action id.
        got: u8,
    },
    /// Slot is outside the blackboard; culprit carried.
    BadSlot {
        /// Seen slot.
        got: u8,
    },
    /// Child list is longer than the cap; count carried.
    BadChildCount {
        /// Seen child count.
        got: usize,
    },
    /// Gate period is zero.
    BadPeriod,
}

impl core::fmt::Display for BtError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            BtError::Empty => write!(f, "bt: no bytes to decode"),
            BtError::BadVersion { got } => write!(f, "bt: version {got} is not {BT_VERSION}"),
            BtError::BadTag { tag } => write!(f, "bt: node tag {tag} is unknown"),
            BtError::BadCond { tag } => write!(f, "bt: condition tag {tag} is unknown"),
            BtError::Truncated => write!(f, "bt: bytes end mid-node"),
            BtError::Trailing { left } => write!(f, "bt: {left} trailing bytes after the tree"),
            BtError::TooManyNodes { got } => write!(f, "bt: {got} nodes exceed {MAX_NODES}"),
            BtError::TooDeep { got } => write!(f, "bt: depth {got} exceeds {MAX_DEPTH}"),
            BtError::BadAction { got } => write!(f, "bt: action {got} is outside the table"),
            BtError::BadSlot { got } => write!(f, "bt: slot {got} is outside the blackboard"),
            BtError::BadChildCount { got } => write!(f, "bt: {got} children exceed {MAX_CHILDREN}"),
            BtError::BadPeriod => write!(f, "bt: gate period must be nonzero"),
        }
    }
}

impl std::error::Error for BtError {}

/// Byte tags for the versioned encoding. Integer tags only.
const TAG_ACTION: u8 = 1;
/// Byte tag for condition nodes.
const TAG_COND: u8 = 2;
/// Byte tag for blackboard writes.
const TAG_SET_BB: u8 = 3;
/// Byte tag for blackboard adds.
const TAG_ADD_BB: u8 = 4;
/// Byte tag for tick gates.
const TAG_GATE: u8 = 5;
/// Byte tag for sequence composites.
const TAG_SEQUENCE: u8 = 16;
/// Byte tag for selector composites.
const TAG_SELECTOR: u8 = 17;
/// Condition tag for distance at-or-under.
const COND_DIST_LE: u8 = 0;
/// Condition tag for distance above.
const COND_DIST_GT: u8 = 1;
/// Condition tag for foe attacking.
const COND_FOE_ATTACKING: u8 = 2;
/// Condition tag for foe idle.
const COND_FOE_IDLE: u8 = 3;
/// Condition tag for slot at-or-above.
const COND_BB_GE: u8 = 4;
/// Condition tag for slot below.
const COND_BB_LT: u8 = 5;
/// Condition tag for tick mask match.
const COND_TICK_MOD: u8 = 6;

impl BtTree {
    /// Build a tree after checking size, depth, actions, slots, child
    /// counts and gate periods. Units: nodes. Errors: [`BtError`].
    pub fn new(root: BtNode) -> Result<Self, BtError> {
        check_node(&root)?;
        let count = root.node_count();
        if count > MAX_NODES {
            return Err(BtError::TooManyNodes { got: count });
        }
        let depth = root.depth();
        if depth > MAX_DEPTH {
            return Err(BtError::TooDeep { got: depth });
        }
        Ok(Self { root })
    }

    /// Borrow the root node. Units: nodes.
    pub fn root(&self) -> &BtNode {
        &self.root
    }

    /// Count nodes in the tree. Units: node count.
    pub fn node_count(&self) -> usize {
        self.root.node_count()
    }

    /// Nesting depth with the root at 1. Units: depth in nodes.
    pub fn depth(&self) -> usize {
        self.root.depth()
    }

    /// FNV-1a64 over the versioned encoding. Same tree gives the same
    /// digest on every platform. Units: hash over bytes.
    pub fn tree_hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u8(BT_VERSION);
        hash_node(&mut hasher, &self.root);
        hasher.finish()
    }

    /// Encode as versioned bytes (cold build path; allocation is fine
    /// here, never on the tick). Units: bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(BT_VERSION);
        encode_node(&self.root, &mut out);
        out
    }

    /// Decode versioned bytes. Validates every tag, count and range;
    /// rejects trailing bytes. Never panics. Units: bytes.
    /// Errors: [`BtError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, BtError> {
        if bytes.is_empty() {
            return Err(BtError::Empty);
        }
        if bytes[0] != BT_VERSION {
            return Err(BtError::BadVersion { got: bytes[0] });
        }
        let mut cursor = 1usize;
        let root = decode_node(bytes, &mut cursor, 1)?;
        if cursor != bytes.len() {
            return Err(BtError::Trailing {
                left: bytes.len().saturating_sub(cursor),
            });
        }
        Self::new(root)
    }

    /// Tick the tree over integer state. Writes the latest action id
    /// into `out_action` whenever an action node runs; conditions and
    /// blackboard writes leave it alone. Empty sequences succeed and
    /// empty selectors fail. Integer math only, no heap use: the
    /// caller owns `bb` and `out_action` across ticks. Units: ticks.
    pub fn tick(&self, state: &BtState, bb: &mut BtBlackboard, out_action: &mut u8) -> BtStatus {
        tick_node(&self.root, state, bb, out_action)
    }
}

/// Check one subtree for bad payloads without counting size. Cold
/// build path. Units: nodes. Errors: [`BtError`].
fn check_node(node: &BtNode) -> Result<(), BtError> {
    match node {
        BtNode::Action(id) => {
            if *id < ACTION_COUNT {
                Ok(())
            } else {
                Err(BtError::BadAction { got: *id })
            }
        }
        BtNode::Cond(cond) => check_cond(cond),
        BtNode::SetBb { slot, .. } | BtNode::AddBb { slot, .. } => {
            if usize::from(*slot) < BB_SLOTS {
                Ok(())
            } else {
                Err(BtError::BadSlot { got: *slot })
            }
        }
        BtNode::TickGate { period } => {
            if *period == 0 {
                Err(BtError::BadPeriod)
            } else {
                Ok(())
            }
        }
        BtNode::Sequence(kids) | BtNode::Selector(kids) => {
            if kids.len() > MAX_CHILDREN {
                return Err(BtError::BadChildCount { got: kids.len() });
            }
            for kid in kids {
                check_node(kid)?;
            }
            Ok(())
        }
    }
}

/// Check one condition for bad slots. Cold build path. Units: slots.
/// Errors: [`BtError`].
fn check_cond(cond: &BtCond) -> Result<(), BtError> {
    match *cond {
        BtCond::DistLe(_)
        | BtCond::DistGt(_)
        | BtCond::FoeAttacking
        | BtCond::FoeIdle
        | BtCond::TickMod { .. } => Ok(()),
        BtCond::BbGe { slot, .. } | BtCond::BbLt { slot, .. } => {
            if usize::from(slot) < BB_SLOTS {
                Ok(())
            } else {
                Err(BtError::BadSlot { got: slot })
            }
        }
    }
}

/// Hash one node in visit order. No heap use. Units: hash over bytes.
fn hash_node(hasher: &mut Fnv64, node: &BtNode) {
    match node {
        BtNode::Action(id) => {
            hasher.write_u8(TAG_ACTION);
            hasher.write_u8(*id);
        }
        BtNode::Cond(cond) => {
            hasher.write_u8(TAG_COND);
            hash_cond(hasher, cond);
        }
        BtNode::SetBb { slot, value } => {
            hasher.write_u8(TAG_SET_BB);
            hasher.write_u8(*slot);
            hasher.write_i32(*value);
        }
        BtNode::AddBb { slot, delta } => {
            hasher.write_u8(TAG_ADD_BB);
            hasher.write_u8(*slot);
            hasher.write_i32(*delta);
        }
        BtNode::TickGate { period } => {
            hasher.write_u8(TAG_GATE);
            hasher.write_u32(*period);
        }
        BtNode::Sequence(kids) => {
            hasher.write_u8(TAG_SEQUENCE);
            write_len(hasher, kids.len());
            for kid in kids {
                hash_node(hasher, kid);
            }
        }
        BtNode::Selector(kids) => {
            hasher.write_u8(TAG_SELECTOR);
            write_len(hasher, kids.len());
            for kid in kids {
                hash_node(hasher, kid);
            }
        }
    }
}

/// Hash one condition. No heap use. Units: hash over integers.
fn hash_cond(hasher: &mut Fnv64, cond: &BtCond) {
    match *cond {
        BtCond::DistLe(bound) => {
            hasher.write_u8(COND_DIST_LE);
            hasher.write_i32(bound);
        }
        BtCond::DistGt(bound) => {
            hasher.write_u8(COND_DIST_GT);
            hasher.write_i32(bound);
        }
        BtCond::FoeAttacking => hasher.write_u8(COND_FOE_ATTACKING),
        BtCond::FoeIdle => hasher.write_u8(COND_FOE_IDLE),
        BtCond::BbGe { slot, threshold } => {
            hasher.write_u8(COND_BB_GE);
            hasher.write_u8(slot);
            hasher.write_i32(threshold);
        }
        BtCond::BbLt { slot, threshold } => {
            hasher.write_u8(COND_BB_LT);
            hasher.write_u8(slot);
            hasher.write_i32(threshold);
        }
        BtCond::TickMod { mask, expect } => {
            hasher.write_u8(COND_TICK_MOD);
            hasher.write_u32(mask);
            hasher.write_u32(expect);
        }
    }
}

/// Hash a child count as two bytes without narrowing casts. The count
/// never exceeds the small cap, so the high byte stays zero. No heap
/// use. Units: counts.
fn write_len(hasher: &mut Fnv64, len: usize) {
    let capped = len.min(255);
    let low = u8::try_from(capped).unwrap_or(255);
    hasher.write_u8(low);
    hasher.write_u8(0);
}

/// Append one node to the cold byte buffer. Build path only, never the
/// tick. Units: bytes.
fn encode_node(node: &BtNode, out: &mut Vec<u8>) {
    match node {
        BtNode::Action(id) => {
            out.push(TAG_ACTION);
            out.push(*id);
        }
        BtNode::Cond(cond) => {
            out.push(TAG_COND);
            encode_cond(cond, out);
        }
        BtNode::SetBb { slot, value } => {
            out.push(TAG_SET_BB);
            out.push(*slot);
            out.extend_from_slice(&value.to_le_bytes());
        }
        BtNode::AddBb { slot, delta } => {
            out.push(TAG_ADD_BB);
            out.push(*slot);
            out.extend_from_slice(&delta.to_le_bytes());
        }
        BtNode::TickGate { period } => {
            out.push(TAG_GATE);
            out.extend_from_slice(&period.to_le_bytes());
        }
        BtNode::Sequence(kids) => {
            out.push(TAG_SEQUENCE);
            push_len(out, kids.len());
            for kid in kids {
                encode_node(kid, out);
            }
        }
        BtNode::Selector(kids) => {
            out.push(TAG_SELECTOR);
            push_len(out, kids.len());
            for kid in kids {
                encode_node(kid, out);
            }
        }
    }
}

/// Append one condition to the cold byte buffer. Build path only.
/// Units: bytes.
fn encode_cond(cond: &BtCond, out: &mut Vec<u8>) {
    match *cond {
        BtCond::DistLe(bound) => {
            out.push(COND_DIST_LE);
            out.extend_from_slice(&bound.to_le_bytes());
        }
        BtCond::DistGt(bound) => {
            out.push(COND_DIST_GT);
            out.extend_from_slice(&bound.to_le_bytes());
        }
        BtCond::FoeAttacking => out.push(COND_FOE_ATTACKING),
        BtCond::FoeIdle => out.push(COND_FOE_IDLE),
        BtCond::BbGe { slot, threshold } => {
            out.push(COND_BB_GE);
            out.push(slot);
            out.extend_from_slice(&threshold.to_le_bytes());
        }
        BtCond::BbLt { slot, threshold } => {
            out.push(COND_BB_LT);
            out.push(slot);
            out.extend_from_slice(&threshold.to_le_bytes());
        }
        BtCond::TickMod { mask, expect } => {
            out.push(COND_TICK_MOD);
            out.extend_from_slice(&mask.to_le_bytes());
            out.extend_from_slice(&expect.to_le_bytes());
        }
    }
}

/// Push a child count as one byte with a range check at decode. The
/// builder already caps the list. Units: counts.
fn push_len(out: &mut Vec<u8>, len: usize) {
    let capped = len.min(255);
    out.push(u8::try_from(capped).unwrap_or(255));
}

/// Read one little-endian `i32` or fail with truncation. Never panics.
/// Units: bytes. Errors: [`BtError`].
fn read_i32(bytes: &[u8], cursor: &mut usize) -> Result<i32, BtError> {
    if bytes.len().saturating_sub(*cursor) < 4 {
        return Err(BtError::Truncated);
    }
    let mut lane = [0u8; 4];
    lane[0] = bytes[*cursor];
    lane[1] = bytes[*cursor + 1];
    lane[2] = bytes[*cursor + 2];
    lane[3] = bytes[*cursor + 3];
    *cursor += 4;
    Ok(i32::from_le_bytes(lane))
}

/// Read one little-endian `u32` or fail with truncation. Never panics.
/// Units: bytes. Errors: [`BtError`].
fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, BtError> {
    if bytes.len().saturating_sub(*cursor) < 4 {
        return Err(BtError::Truncated);
    }
    let mut lane = [0u8; 4];
    lane[0] = bytes[*cursor];
    lane[1] = bytes[*cursor + 1];
    lane[2] = bytes[*cursor + 2];
    lane[3] = bytes[*cursor + 3];
    *cursor += 4;
    Ok(u32::from_le_bytes(lane))
}

/// Read one byte or fail with truncation. Never panics. Units: bytes.
/// Errors: [`BtError`].
fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, BtError> {
    if *cursor >= bytes.len() {
        return Err(BtError::Truncated);
    }
    let value = bytes[*cursor];
    *cursor += 1;
    Ok(value)
}

/// Decode one node at `cursor` with a depth guard. Cold path; builds
/// heap nodes for composites only. Never panics. Units: bytes.
/// Errors: [`BtError`].
fn decode_node(bytes: &[u8], cursor: &mut usize, depth: usize) -> Result<BtNode, BtError> {
    if depth > MAX_DEPTH {
        return Err(BtError::TooDeep { got: depth });
    }
    let tag = read_u8(bytes, cursor)?;
    match tag {
        TAG_ACTION => {
            let id = read_u8(bytes, cursor)?;
            if id < ACTION_COUNT {
                Ok(BtNode::Action(id))
            } else {
                Err(BtError::BadAction { got: id })
            }
        }
        TAG_COND => Ok(BtNode::Cond(decode_cond(bytes, cursor)?)),
        TAG_SET_BB => {
            let slot = read_u8(bytes, cursor)?;
            let value = read_i32(bytes, cursor)?;
            if usize::from(slot) < BB_SLOTS {
                Ok(BtNode::SetBb { slot, value })
            } else {
                Err(BtError::BadSlot { got: slot })
            }
        }
        TAG_ADD_BB => {
            let slot = read_u8(bytes, cursor)?;
            let delta = read_i32(bytes, cursor)?;
            if usize::from(slot) < BB_SLOTS {
                Ok(BtNode::AddBb { slot, delta })
            } else {
                Err(BtError::BadSlot { got: slot })
            }
        }
        TAG_GATE => {
            let period = read_u32(bytes, cursor)?;
            if period == 0 {
                Err(BtError::BadPeriod)
            } else {
                Ok(BtNode::TickGate { period })
            }
        }
        TAG_SEQUENCE | TAG_SELECTOR => {
            let count_byte = read_u8(bytes, cursor)?;
            let count = usize::from(count_byte);
            if count > MAX_CHILDREN {
                return Err(BtError::BadChildCount { got: count });
            }
            let mut kids = Vec::new();
            for _ in 0..count {
                kids.push(decode_node(bytes, cursor, depth + 1)?);
            }
            if tag == TAG_SEQUENCE {
                Ok(BtNode::Sequence(kids))
            } else {
                Ok(BtNode::Selector(kids))
            }
        }
        other => Err(BtError::BadTag { tag: other }),
    }
}

/// Decode one condition at `cursor`. Never panics. Units: bytes.
/// Errors: [`BtError`].
fn decode_cond(bytes: &[u8], cursor: &mut usize) -> Result<BtCond, BtError> {
    let tag = read_u8(bytes, cursor)?;
    match tag {
        COND_DIST_LE => Ok(BtCond::DistLe(read_i32(bytes, cursor)?)),
        COND_DIST_GT => Ok(BtCond::DistGt(read_i32(bytes, cursor)?)),
        COND_FOE_ATTACKING => Ok(BtCond::FoeAttacking),
        COND_FOE_IDLE => Ok(BtCond::FoeIdle),
        COND_BB_GE => {
            let slot = read_u8(bytes, cursor)?;
            let threshold = read_i32(bytes, cursor)?;
            if usize::from(slot) < BB_SLOTS {
                Ok(BtCond::BbGe { slot, threshold })
            } else {
                Err(BtError::BadSlot { got: slot })
            }
        }
        COND_BB_LT => {
            let slot = read_u8(bytes, cursor)?;
            let threshold = read_i32(bytes, cursor)?;
            if usize::from(slot) < BB_SLOTS {
                Ok(BtCond::BbLt { slot, threshold })
            } else {
                Err(BtError::BadSlot { got: slot })
            }
        }
        COND_TICK_MOD => {
            let mask = read_u32(bytes, cursor)?;
            let expect = read_u32(bytes, cursor)?;
            Ok(BtCond::TickMod { mask, expect })
        }
        other => Err(BtError::BadCond { tag: other }),
    }
}

/// Tick one node. Integer math only, no heap use; the caller owns all
/// buffers. Sequence fails fast and selector succeeds fast; both pass
/// `Running` through at once. Units: ticks.
fn tick_node(
    node: &BtNode,
    state: &BtState,
    bb: &mut BtBlackboard,
    out_action: &mut u8,
) -> BtStatus {
    match node {
        BtNode::Action(id) => {
            *out_action = *id;
            BtStatus::Success
        }
        BtNode::Cond(cond) => {
            if cond.eval(state, bb) {
                BtStatus::Success
            } else {
                BtStatus::Failure
            }
        }
        BtNode::SetBb { slot, value } => {
            let index = usize::from(*slot);
            if bb.set(index, *value) {
                BtStatus::Success
            } else {
                BtStatus::Failure
            }
        }
        BtNode::AddBb { slot, delta } => {
            let index = usize::from(*slot);
            if bb.add(index, *delta) {
                BtStatus::Success
            } else {
                BtStatus::Failure
            }
        }
        BtNode::TickGate { period } => {
            if *period == 0 {
                BtStatus::Failure
            } else if state.tick.is_multiple_of(*period) {
                BtStatus::Success
            } else {
                BtStatus::Running
            }
        }
        BtNode::Sequence(kids) => {
            for kid in kids {
                match tick_node(kid, state, bb, out_action) {
                    BtStatus::Failure => return BtStatus::Failure,
                    BtStatus::Running => return BtStatus::Running,
                    BtStatus::Success => {}
                }
            }
            BtStatus::Success
        }
        BtNode::Selector(kids) => {
            for kid in kids {
                match tick_node(kid, state, bb, out_action) {
                    BtStatus::Success => return BtStatus::Success,
                    BtStatus::Running => return BtStatus::Running,
                    BtStatus::Failure => {}
                }
            }
            BtStatus::Failure
        }
    }
}

/// Digest one decision for tape chaining. Same inputs give the same
/// digest on every platform. Units: hash over integers.
pub fn tick_digest(state: &BtState, bb: &BtBlackboard, action: u8, status: BtStatus) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u64(state.state_hash());
    hasher.write_u64(bb.state_hash());
    hasher.write_u8(action);
    let lane = match status {
        BtStatus::Success => 0u8,
        BtStatus::Failure => 1u8,
        BtStatus::Running => 2u8,
    };
    hasher.write_u8(lane);
    hasher.finish()
}

/// Pressure brain: strike in range, guard the incoming swing, else
/// close distance. Counts engagements on slot 0 and wanders one tick
/// after a long exchange (then resets the counter) so the bout never
/// stills. Units: nodes.
pub fn pressure_tree() -> BtTree {
    let root = BtNode::Selector(vec![
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::BbGe {
                slot: 0,
                threshold: 24,
            }),
            BtNode::SetBb { slot: 0, value: 0 },
            BtNode::Action(ACTION_REPOSITION),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::FoeAttacking),
            BtNode::Cond(BtCond::DistLe(4_000)),
            BtNode::AddBb { slot: 0, delta: 1 },
            BtNode::Action(ACTION_BLOCK),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::DistLe(2_500)),
            BtNode::AddBb { slot: 0, delta: 1 },
            BtNode::Action(ACTION_ATTACK),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::DistGt(8_000)),
            BtNode::Action(ACTION_ADVANCE),
        ]),
        BtNode::Action(ACTION_DODGE),
    ]);
    match BtTree::new(root) {
        Ok(tree) => tree,
        Err(_) => BtTree {
            root: BtNode::Action(ACTION_ADVANCE),
        },
    }
}

/// Guard brain: answer far ground with approach, mid ground with
/// sidesteps on alternate ticks, and strikes only when close and the
/// foe is idle. Uses slot 1 as its own exchange counter, wandered one
/// tick per long exchange (then reset) so the bout never stills.
/// Units: nodes.
pub fn guard_tree() -> BtTree {
    let root = BtNode::Selector(vec![
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::BbGe {
                slot: 1,
                threshold: 24,
            }),
            BtNode::SetBb { slot: 1, value: 0 },
            BtNode::Action(ACTION_REPOSITION),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::DistGt(8_000)),
            BtNode::Action(ACTION_ADVANCE),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::FoeAttacking),
            BtNode::Cond(BtCond::TickMod { mask: 1, expect: 0 }),
            BtNode::AddBb { slot: 1, delta: 1 },
            BtNode::Action(ACTION_DODGE),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::FoeAttacking),
            BtNode::AddBb { slot: 1, delta: 1 },
            BtNode::Action(ACTION_BLOCK),
        ]),
        BtNode::Sequence(vec![
            BtNode::Cond(BtCond::FoeIdle),
            BtNode::Cond(BtCond::DistLe(2_500)),
            BtNode::AddBb { slot: 1, delta: 1 },
            BtNode::Action(ACTION_ATTACK),
        ]),
        BtNode::Action(ACTION_RETREAT),
    ]);
    match BtTree::new(root) {
        Ok(tree) => tree,
        Err(_) => BtTree {
            root: BtNode::Action(ACTION_RETREAT),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn state_at(tick: u32, dist_mm: i32, foe_attacking: bool) -> BtState {
        BtState::new(dist_mm, foe_attacking, 500, tick)
    }

    fn build(nodes: BtNode) -> BtTree {
        BtTree::new(nodes).unwrap()
    }

    #[test]
    fn sequence_fails_fast_and_selector_picks_first_success() {
        let seq = build(BtNode::Sequence(vec![
            BtNode::Action(3),
            BtNode::Cond(BtCond::FoeIdle),
            BtNode::Action(0),
        ]));
        let mut bb = BtBlackboard::new();
        let mut out = 9u8;
        let attacking = state_at(0, 1_000, true);
        assert_eq!(seq.tick(&attacking, &mut bb, &mut out), BtStatus::Failure);
        assert_eq!(out, 3, "first action ran before the failing guard");
        let sel = build(BtNode::Selector(vec![
            BtNode::Cond(BtCond::FoeAttacking),
            BtNode::Action(1),
            BtNode::Action(2),
        ]));
        let mut out = 9u8;
        assert_eq!(sel.tick(&attacking, &mut bb, &mut out), BtStatus::Success);
        assert_eq!(out, 9, "guard success leaves the output alone");
        let idle = state_at(0, 1_000, false);
        let mut out = 9u8;
        assert_eq!(sel.tick(&idle, &mut bb, &mut out), BtStatus::Success);
        assert_eq!(out, 1, "selector skips the failed guard and runs Action(1)");
        let empty_seq = build(BtNode::Sequence(vec![]));
        assert_eq!(
            empty_seq.tick(&attacking, &mut bb, &mut out),
            BtStatus::Success
        );
        let empty_sel = build(BtNode::Selector(vec![]));
        assert_eq!(
            empty_sel.tick(&attacking, &mut bb, &mut out),
            BtStatus::Failure
        );
    }

    #[test]
    fn blackboard_add_gates_later_decisions() {
        let tree = build(BtNode::Sequence(vec![
            BtNode::AddBb { slot: 0, delta: 1 },
            BtNode::Cond(BtCond::BbGe {
                slot: 0,
                threshold: 3,
            }),
            BtNode::Action(0),
        ]));
        let mut bb = BtBlackboard::new();
        let idle = state_at(7, 1_000, false);
        let mut out = 9u8;
        assert_eq!(tree.tick(&idle, &mut bb, &mut out), BtStatus::Failure);
        assert_eq!(tree.tick(&idle, &mut bb, &mut out), BtStatus::Failure);
        assert_eq!(tree.tick(&idle, &mut bb, &mut out), BtStatus::Success);
        assert_eq!(out, 0);
        assert_eq!(bb.get(0), Some(3));
    }

    #[test]
    fn out_of_range_slots_fail_without_panic() {
        let mut bb = BtBlackboard::new();
        assert_eq!(bb.get(77), None);
        assert!(!bb.set(77, 5));
        assert!(!bb.add(77, 5));
        let write = BtNode::SetBb { slot: 40, value: 1 };
        let idle = state_at(0, 1_000, false);
        let mut out = 9u8;
        assert_eq!(
            tick_node(&write, &idle, &mut bb, &mut out),
            BtStatus::Failure
        );
        assert_eq!(out, 9, "failed writes leave the output alone");
        assert!(BtTree::new(BtNode::Action(9)).is_err());
        assert!(BtTree::new(BtNode::SetBb { slot: 40, value: 1 }).is_err());
        assert!(BtTree::new(BtNode::TickGate { period: 0 }).is_err());
    }

    #[test]
    fn gate_returns_running_between_periods() {
        let tree = build(BtNode::Sequence(vec![
            BtNode::TickGate { period: 4 },
            BtNode::Action(2),
        ]));
        let mut bb = BtBlackboard::new();
        let mut out = 9u8;
        assert_eq!(
            tree.tick(&state_at(8, 1_000, false), &mut bb, &mut out),
            BtStatus::Success
        );
        assert_eq!(out, 2);
        assert_eq!(
            tree.tick(&state_at(9, 1_000, false), &mut bb, &mut out),
            BtStatus::Running
        );
    }

    #[test]
    fn serial_round_trip_is_byte_identical() {
        let tree = pressure_tree();
        let bytes = tree.encode();
        assert_eq!(bytes[0], BT_VERSION);
        let back = BtTree::decode(&bytes).unwrap();
        assert_eq!(back.encode(), bytes);
        assert_eq!(back.tree_hash(), tree.tree_hash());
        assert_eq!(back, tree);
    }

    #[test]
    fn decode_rejects_bad_version_tag_truncation_and_trailing() {
        let good = guard_tree().encode();
        let mut bad_version = good.clone();
        bad_version[0] = BT_VERSION.wrapping_add(1);
        assert!(matches!(
            BtTree::decode(&bad_version),
            Err(BtError::BadVersion { .. })
        ));
        assert!(matches!(BtTree::decode(&[]), Err(BtError::Empty)));
        assert!(matches!(
            BtTree::decode(&[BT_VERSION, 99]),
            Err(BtError::BadTag { .. })
        ));
        assert!(matches!(
            BtTree::decode(&[BT_VERSION, TAG_ACTION]),
            Err(BtError::Truncated)
        ));
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(matches!(
            BtTree::decode(&trailing),
            Err(BtError::Trailing { .. })
        ));
        assert!(matches!(
            BtTree::decode(&[BT_VERSION, TAG_ACTION, 77]),
            Err(BtError::BadAction { .. })
        ));
        assert!(matches!(
            BtTree::decode(&[BT_VERSION, TAG_GATE, 0, 0, 0, 0]),
            Err(BtError::BadPeriod)
        ));
    }

    #[test]
    fn rejects_oversize_deep_and_wide_trees() {
        let mut kids = Vec::new();
        for _ in 0..(MAX_CHILDREN + 1) {
            kids.push(BtNode::Action(0));
        }
        assert!(matches!(
            BtTree::new(BtNode::Sequence(kids)),
            Err(BtError::BadChildCount { .. })
        ));
        let mut deep = BtNode::Action(0);
        for _ in 0..(MAX_DEPTH + 2) {
            deep = BtNode::Sequence(vec![deep]);
        }
        assert!(matches!(BtTree::new(deep), Err(BtError::TooDeep { .. })));
        let mut wide = Vec::new();
        for _ in 0..(MAX_NODES + 4) {
            wide.push(BtNode::Action(1));
        }
        let chunked = wide
            .chunks(MAX_CHILDREN)
            .map(|lane| BtNode::Sequence(lane.to_vec()))
            .collect::<Vec<_>>();
        assert!(matches!(
            BtTree::new(BtNode::Sequence(chunked)),
            Err(BtError::TooManyNodes { .. })
        ));
    }

    #[test]
    fn shipped_trees_stay_hashed_and_decide_deterministically() {
        for tree in [pressure_tree(), guard_tree()] {
            let bytes = tree.encode();
            assert_eq!(
                BtTree::decode(&bytes).unwrap().tree_hash(),
                tree.tree_hash()
            );
            let mut first = Vec::new();
            let mut second = Vec::new();
            for tick in 0..64u32 {
                let state = BtState::new(1_000 + tick as i32 * 37, tick % 3 == 0, 500, tick);
                let mut bb_a = BtBlackboard::new();
                let mut bb_b = BtBlackboard::new();
                let mut out_a = 9u8;
                let mut out_b = 9u8;
                let a = tree.tick(&state, &mut bb_a, &mut out_a);
                let b = tree.tick(&state, &mut bb_b, &mut out_b);
                first.push((out_a, a));
                second.push((out_b, b));
            }
            assert_eq!(first, second);
        }
    }

    #[test]
    fn tick_into_reused_buffers_allocates_nothing() {
        let tree = pressure_tree();
        let state = state_at(11, 2_000, true);
        let mut bb = BtBlackboard::new();
        let mut out = ACTION_ADVANCE;
        let ((), heap) = measure(|| {
            for _ in 0..64 {
                std::hint::black_box(tree.tick(&state, &mut bb, &mut out));
            }
        });
        assert!(out < ACTION_COUNT);
        assert_eq!(heap.calls, 0, "integer tick must not allocate");
    }

    #[test]
    fn error_display_names_each_cause() {
        assert!(format!("{}", BtError::Empty).contains("no bytes"));
        assert!(format!("{}", BtError::Truncated).contains("mid-node"));
        assert!(format!("{}", BtError::BadPeriod).contains("nonzero"));
        assert!(format!("{}", BtError::BadAction { got: 9 }).contains('9'));
    }

    #[ignore = "timing: measures wall time for the decision budget"]
    #[test]
    fn decision_stays_inside_budget() {
        let tree = pressure_tree();
        let state = state_at(3, 2_000, true);
        let mut bb = BtBlackboard::new();
        let mut out = ACTION_ADVANCE;
        let start = std::time::Instant::now();
        let rounds = 20_000u32;
        for _ in 0..rounds {
            std::hint::black_box(tree.tick(&state, &mut bb, &mut out));
        }
        let micros = start.elapsed().as_micros();
        let per = micros / u128::from(rounds);
        assert!(
            per < u128::from(DECISION_BUDGET_MICROS),
            "decision {per} micros exceeds the budget"
        );
    }
}
