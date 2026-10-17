//! bridge: byte-level adapters over arena/scene/ecs snapshots for extraction.
//!
//! Owner: this module; consumer: render/audio/UI extraction. Units:
//! snapshots (LE bytes), stable ids (`StableId`), meters/radians/ticks.
//! Determinism: rows are always in `StableId` order, so the same inputs give
//! the same digest in any ingest order. Errors: typed `BridgeError`; unknown
//! ids/versions refuse loudly.

use crate::snapshot::{hash_bytes, restore, SnapshotError, SNAPSHOT_VERSION};
use crate::spine::{Spine, SpineError, SpineSnapshot, MAX_SPINE_SLOTS};

/// Wire version of the caller-encoded arena layout (see [`encode_arena`]).
pub const ARENA_WIRE_VERSION: u16 = 1;
/// Wire version of the caller-encoded scene layout (see [`encode_scene`]).
pub const SCENE_WIRE_VERSION: u16 = 1;
/// Largest arena fighter list one ingest accepts (bounded buffers).
pub const ARENA_MAX_ROWS: usize = 256;
/// Largest scene node list one ingest accepts (bounded buffers).
pub const SCENE_MAX_ROWS: usize = 65_536;
/// Largest ECS slot count one ingest accepts (bounded buffers).
pub const ECS_MAX_SLOTS: usize = 65_536;

/// Tag bits for arena ids; the low 16 bits carry the fighter slot.
const ARENA_TAG: u64 = 0x4152_454E_4100_0000;
/// Tag bits for ECS ids; the low 16 bits carry the entity slot.
const ECS_TAG: u64 = 0x4543_535F_4500_0000;

/// Stable id in the extraction view. Arena and ECS ids fold a namespaced tag
/// around the small integer slot; scene ids hash the 16 raw `ObjectId` bytes
/// with a domain tag. Namespaces are disjoint by tag bits, and any residual
/// collision across them surfaces as [`BridgeError::DuplicateId`], never as
/// a silent alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableId(u64);

impl StableId {
    /// Stable id for an arena fighter slot.
    pub fn arena(slot: u8) -> Self {
        Self(ARENA_TAG | u64::from(slot))
    }
    /// Stable id for a live ECS slot. The caller bounds slots (ingest caps
    /// them at [`ECS_MAX_SLOTS`], so the index always fits the low 16 bits).
    pub fn ecs_index(slot: usize) -> Self {
        debug_assert!(slot <= 0xFFFF, "ecs slot must fit the id low bits");
        Self(ECS_TAG | (slot as u64))
    }
    /// Stable id for a scene node from its 16 raw `ObjectId` bytes. The
    /// caller copies the bytes out of the owning crate; this function never
    /// imports it.
    pub fn scene(id16: &[u8; 16]) -> Self {
        let mut tagged = [0u8; 22];
        tagged[..6].copy_from_slice(b"scene:");
        tagged[6..].copy_from_slice(id16);
        Self(hash_bytes(&tagged))
    }
    /// Raw id value, for maps and receipts outside this crate.
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for StableId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "stable:{:016x}", self.0)
    }
}

/// Which store a row was extracted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Headless match state (arena owns it).
    Arena,
    /// Authored hierarchy node (scene owns it).
    Scene,
    /// Simulated entity (ecs owns it).
    Ecs,
}

impl Domain {
    /// Digest tag byte for the extraction digest.
    fn tag(self) -> u8 {
        match self {
            Self::Arena => 0,
            Self::Scene => 1,
            Self::Ecs => 2,
        }
    }
}

/// One extracted row: where to find the object plus its planar pose. Full
/// data (rotations, scales, combat, brains) stays in the owning store; the
/// bridge carries lookup plus position so extraction never reads sim state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// Stable id for cross-store lookup.
    pub id: StableId,
    /// Store this row came from.
    pub domain: Domain,
    /// Position in meters. ECS 2D `(x, y)` maps to `[x, 0.0, y]` (planar
    /// convention: the 2D ordinate is depth, height is the floor).
    pub pos: [f32; 3],
    /// Facing yaw in radians; zero when the domain carries none.
    pub yaw: f32,
    /// Scene hierarchy link; `None` outside scene rows.
    pub parent: Option<StableId>,
}

/// One extraction frame: every ingested store read once, rows in stable
/// order. Its first consumer is the editor spine inspector
/// (`engine-editor::spine_inspect`): it reads rows from spine bytes and never
/// touches sim state.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractionView {
    /// Arena or spine tick, or zero when none was ingested.
    pub tick: u32,
    /// Checkpoint hash (arena) or spine snapshot hash (spine) when present.
    pub arena_hash: Option<u64>,
    /// Rows in [`StableId`] order, whatever order ingestion happened in.
    pub rows: Vec<Row>,
}

impl ExtractionView {
    /// Row count.
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    /// True when no store was ingested.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// Row by stable id, `None` for unknown ids.
    pub fn get(&self, id: StableId) -> Option<&Row> {
        self.rows
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|i| &self.rows[i])
    }
    /// FNV-1a64 over the canonical view (tick, arena hash, row count, then
    /// each row's id, domain tag, position bits, yaw bits and parent). Same
    /// inputs give the same digest in any ingest order.
    pub fn digest(&self) -> u64 {
        let mut out = Vec::with_capacity(16 + self.rows.len() * 38);
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&(self.rows.len() as u64).to_le_bytes());
        match self.arena_hash {
            Some(h) => out.extend_from_slice(&h.to_le_bytes()),
            None => out.extend_from_slice(&[0xFF; 8]),
        }
        for row in &self.rows {
            out.extend_from_slice(&row.id.0.to_le_bytes());
            out.push(row.domain.tag());
            for v in row.pos {
                out.extend_from_slice(&v.to_bits().to_le_bytes());
            }
            out.extend_from_slice(&row.yaw.to_bits().to_le_bytes());
            match row.parent {
                Some(p) => out.extend_from_slice(&p.0.to_le_bytes()),
                None => out.extend_from_slice(&[0; 8]),
            }
        }
        hash_bytes(&out)
    }
}

/// Typed bridge failure. Every variant names the culprit; corrupt, stale or
/// unknown input is always an `Err`, never a guess or a partial view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// Fewer bytes than the header or a field needs.
    TooShort {
        /// Store layout being read: `"arena"`, `"scene"` or `"ecs"`.
        domain: &'static str,
    },
    /// Unknown wire version (holds what was found).
    BadVersion {
        /// Store layout being read.
        domain: &'static str,
        /// Version stamped on the bytes.
        got: u16,
    },
    /// Bytes left over after a full decode (holds the count).
    Trailing {
        /// Store layout being read.
        domain: &'static str,
        /// Leftover byte count.
        count: usize,
    },
    /// Row count above the ingest bound (holds the count).
    TooMany {
        /// Store layout being read.
        domain: &'static str,
        /// Claimed row count.
        count: usize,
    },
    /// ECS or spine alive flag was not 0 or 1 (holds the found flag).
    BadFlag {
        /// `"ecs"` or `"spine"`.
        domain: &'static str,
        /// Flag byte found.
        flag: u8,
    },
    /// Snapshot bytes do not hash to the hash the snapshot carries.
    BadHash {
        /// Store layout being read.
        domain: &'static str,
    },
    /// Scene node id was nil; nil is the "no parent" slot, not a node.
    NilId,
    /// A pose held NaN or infinity.
    NonFinite {
        /// Store layout being read.
        domain: &'static str,
    },
    /// Stable id already present (holds the repeated id).
    DuplicateId(StableId),
    /// Lookup for a stable id the view does not hold.
    UnknownId(StableId),
    /// Arena tick differs from the tick already ingested; the consumer is
    /// stale and must refresh instead of mixing frames.
    TickSkew {
        /// Tick on the newly offered bytes.
        got: u32,
        /// Tick already ingested.
        expected: u32,
    },
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { domain } => write!(f, "bridge {domain} snapshot too short"),
            Self::BadVersion { domain, got } => {
                write!(f, "bridge {domain} snapshot bad version {got}")
            }
            Self::Trailing { domain, count } => {
                write!(f, "bridge {domain} snapshot trailing {count} bytes")
            }
            Self::TooMany { domain, count } => {
                write!(f, "bridge {domain} snapshot too many rows ({count})")
            }
            Self::BadFlag { domain, flag } => {
                write!(f, "bridge {domain} snapshot bad alive flag {flag}")
            }
            Self::BadHash { domain } => {
                write!(f, "bridge {domain} snapshot bytes do not match their hash")
            }
            Self::NilId => write!(f, "bridge scene node id must not be nil"),
            Self::NonFinite { domain } => {
                write!(f, "bridge {domain} snapshot holds a non-finite pose")
            }
            Self::DuplicateId(id) => write!(f, "bridge duplicate stable id {id}"),
            Self::UnknownId(id) => write!(f, "bridge unknown stable id {id}"),
            Self::TickSkew { got, expected } => write!(
                f,
                "bridge arena tick skew: offered tick {got} against ingested {expected}"
            ),
        }
    }
}

impl std::error::Error for BridgeError {}

/// One arena fighter for [`encode_arena`]: slot plus planar pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArenaFighter {
    /// Fighter slot (`FighterId.0` at the call site).
    pub slot: u8,
    /// Feet position `[x, y, z]` in meters.
    pub pos: [f32; 3],
    /// Facing yaw in radians.
    pub yaw: f32,
}

/// One scene node for [`encode_scene`]: raw identity plus local pose. The
/// caller copies the 16 `ObjectId` bytes (nil, all zero, means "no parent"
/// in the parent slot only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneNode {
    /// Raw `ObjectId` bytes; never nil.
    pub id: [u8; 16],
    /// Raw parent `ObjectId` bytes; all zero means a root.
    pub parent: [u8; 16],
    /// Translation in meters, parent frame.
    pub pos: [f32; 3],
    /// Unit quaternion `[x, y, z, w]`, parent frame.
    pub rot: [f32; 4],
    /// Scale factors per axis.
    pub scale: [f32; 3],
}

impl SceneNode {
    /// True for the nil id (legal only as a parent, never as a node).
    pub fn is_nil_id(&self) -> bool {
        self.id == [0; 16]
    }
    /// True when parented to nil (a root).
    pub fn is_root(&self) -> bool {
        self.parent == [0; 16]
    }
}

/// Encode arena state into the bridge wire layout the caller passes to
/// [`Bridge::ingest_arena`]: `u16` version, `u32` tick, `u64` match hash,
/// `u32` count, then per fighter `u8` slot, three `f32` meters, one `f32`
/// yaw. Non-finite poses and over-bound lists fail instead of encoding.
pub fn encode_arena(
    tick: u32,
    hash: u64,
    fighters: &[ArenaFighter],
) -> Result<Vec<u8>, BridgeError> {
    const DOMAIN: &str = "arena";
    if fighters.len() > ARENA_MAX_ROWS {
        return Err(BridgeError::TooMany {
            domain: DOMAIN,
            count: fighters.len(),
        });
    }
    for f in fighters {
        if !f.pos.iter().all(|v| v.is_finite()) || !f.yaw.is_finite() {
            return Err(BridgeError::NonFinite { domain: DOMAIN });
        }
    }
    let mut out = Vec::with_capacity(18 + fighters.len() * 17);
    push_u16(&mut out, ARENA_WIRE_VERSION);
    push_u32(&mut out, tick);
    push_u64(&mut out, hash);
    push_u32(&mut out, fighters.len() as u32);
    for f in fighters {
        out.push(f.slot);
        for v in f.pos {
            push_f32(&mut out, v);
        }
        push_f32(&mut out, f.yaw);
    }
    Ok(out)
}

/// Encode scene state into the bridge wire layout the caller passes to
/// [`Bridge::ingest_scene`]: `u16` version, `u32` count, then per node 16 id
/// bytes, 16 parent bytes, three `f32` position, four `f32` rotation, three
/// `f32` scale. Nil ids and non-finite poses fail instead of encoding.
pub fn encode_scene(nodes: &[SceneNode]) -> Result<Vec<u8>, BridgeError> {
    const DOMAIN: &str = "scene";
    if nodes.len() > SCENE_MAX_ROWS {
        return Err(BridgeError::TooMany {
            domain: DOMAIN,
            count: nodes.len(),
        });
    }
    for n in nodes {
        if n.is_nil_id() {
            return Err(BridgeError::NilId);
        }
        if !n.pos.iter().all(|v| v.is_finite())
            || !n.rot.iter().all(|v| v.is_finite())
            || !n.scale.iter().all(|v| v.is_finite())
        {
            return Err(BridgeError::NonFinite { domain: DOMAIN });
        }
    }
    let mut out = Vec::with_capacity(6 + nodes.len() * 72);
    push_u16(&mut out, SCENE_WIRE_VERSION);
    push_u32(&mut out, nodes.len() as u32);
    for n in nodes {
        out.extend_from_slice(&n.id);
        out.extend_from_slice(&n.parent);
        for v in n.pos {
            push_f32(&mut out, v);
        }
        for v in n.rot {
            push_f32(&mut out, v);
        }
        for v in n.scale {
            push_f32(&mut out, v);
        }
    }
    Ok(out)
}

/// Byte-level bridge over the three separate stores. The bridge owns the two
/// foreign wire layouts above plus the ECS canonical bytes; callers encode
/// their own types at the call site, so no crate imports another in either
/// direction. Rows stay sorted by [`StableId`]; a failed ingest changes
/// nothing.
#[derive(Debug, Default)]
pub struct Bridge {
    rows: Vec<Row>,
    tick: Option<u32>,
    arena_hash: Option<u64>,
}

impl Bridge {
    /// Empty bridge.
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            tick: None,
            arena_hash: None,
        }
    }
    /// Row count ingested so far.
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    /// True when nothing was ingested yet.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// Ingest caller-encoded arena bytes (see [`encode_arena`]). A second
    /// arena ingest on a different tick fails with [`BridgeError::TickSkew`];
    /// the same tick twice fails on the repeated ids.
    pub fn ingest_arena(&mut self, bytes: &[u8]) -> Result<(), BridgeError> {
        let (tick, hash, fresh) = parse_arena(bytes)?;
        if let Some(expected) = self.tick {
            if tick != expected {
                return Err(BridgeError::TickSkew {
                    got: tick,
                    expected,
                });
            }
        }
        self.commit(fresh)?;
        self.tick = Some(tick);
        self.arena_hash = Some(hash);
        Ok(())
    }
    /// Ingest a spine snapshot at `tick`: one arena-domain row per live slot
    /// ([`StableId::arena`] of the slot), pose and yaw from the transform
    /// column, and the snapshot hash as the view's state hash. Reads the
    /// versioned bytes only, so the extraction never borrows sim state. Tick
    /// skew against an earlier arena or spine ingest fails like
    /// [`Bridge::ingest_arena`]; the same slot ids cannot ingest twice.
    pub fn ingest_spine(&mut self, tick: u32, snapshot: &SpineSnapshot) -> Result<(), BridgeError> {
        const DOMAIN: &str = "spine";
        if hash_bytes(&snapshot.bytes) != snapshot.hash {
            return Err(BridgeError::BadHash { domain: DOMAIN });
        }
        let spine = Spine::restore(&snapshot.bytes).map_err(|error| match error {
            SpineError::TooShort => BridgeError::TooShort { domain: DOMAIN },
            SpineError::BadVersion(got) => BridgeError::BadVersion {
                domain: DOMAIN,
                got,
            },
            SpineError::TooManySlots(count) => BridgeError::TooMany {
                domain: DOMAIN,
                count: usize::try_from(count).unwrap_or(usize::MAX),
            },
            SpineError::BadFlag(flag) => BridgeError::BadFlag {
                domain: DOMAIN,
                flag,
            },
            SpineError::Trailing(count) => BridgeError::Trailing {
                domain: DOMAIN,
                count,
            },
            SpineError::Full => BridgeError::TooMany {
                domain: DOMAIN,
                count: MAX_SPINE_SLOTS,
            },
        })?;
        if let Some(expected) = self.tick {
            if tick != expected {
                return Err(BridgeError::TickSkew {
                    got: tick,
                    expected,
                });
            }
        }
        let mut fresh = Vec::with_capacity(spine.live());
        for (slot, transform) in spine.transforms().iter().enumerate() {
            if !spine.is_alive(slot) {
                continue;
            }
            let [x, y, z] = transform.pos;
            if !(x.is_finite() && y.is_finite() && z.is_finite() && transform.yaw.is_finite()) {
                return Err(BridgeError::NonFinite { domain: DOMAIN });
            }
            let slot = u8::try_from(slot).map_err(|_| BridgeError::TooMany {
                domain: DOMAIN,
                count: slot,
            })?;
            fresh.push(Row {
                id: StableId::arena(slot),
                domain: Domain::Arena,
                pos: [canon_zero(x), canon_zero(y), canon_zero(z)],
                yaw: canon_zero(transform.yaw),
                parent: None,
            });
        }
        self.commit(fresh)?;
        self.tick = Some(tick);
        self.arena_hash = Some(snapshot.hash);
        Ok(())
    }
    /// Ingest caller-encoded scene bytes (see [`encode_scene`]).
    pub fn ingest_scene(&mut self, bytes: &[u8]) -> Result<(), BridgeError> {
        let fresh = parse_scene(bytes)?;
        self.commit(fresh)
    }
    /// Ingest ECS canonical bytes (see `crate::snapshot`). Only live slots
    /// extract; killed slots are simply absent (`get` refuses them loudly).
    pub fn ingest_ecs(&mut self, bytes: &[u8]) -> Result<(), BridgeError> {
        const DOMAIN: &str = "ecs";
        if bytes.len() < 6 {
            return Err(BridgeError::TooShort { domain: DOMAIN });
        }
        let version = u16::from_le_bytes([bytes[0], bytes[1]]);
        if version != SNAPSHOT_VERSION {
            return Err(BridgeError::BadVersion {
                domain: DOMAIN,
                got: version,
            });
        }
        let slots = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
        if slots > ECS_MAX_SLOTS {
            return Err(BridgeError::TooMany {
                domain: DOMAIN,
                count: slots,
            });
        }
        if slots > bytes.len() {
            return Err(BridgeError::TooShort { domain: DOMAIN });
        }
        let world = restore(bytes).map_err(|error| match error {
            SnapshotError::TooShort => BridgeError::TooShort { domain: DOMAIN },
            SnapshotError::BadVersion(got) => BridgeError::BadVersion {
                domain: DOMAIN,
                got,
            },
            SnapshotError::BadFlag(flag) => BridgeError::BadFlag {
                domain: DOMAIN,
                flag,
            },
            SnapshotError::Trailing(count) => BridgeError::Trailing {
                domain: DOMAIN,
                count,
            },
        })?;
        let mut fresh = Vec::new();
        for (i, alive) in world.alive.iter().enumerate() {
            if !alive {
                continue;
            }
            let p = world.pos[i];
            if !p.x.is_finite() || !p.y.is_finite() {
                return Err(BridgeError::NonFinite { domain: DOMAIN });
            }
            fresh.push(Row {
                id: StableId::ecs_index(i),
                domain: Domain::Ecs,
                pos: [canon_zero(p.x), 0.0, canon_zero(p.y)],
                yaw: 0.0,
                parent: None,
            });
        }
        self.commit(fresh)
    }
    /// Row by stable id; unknown ids fail with [`BridgeError::UnknownId`].
    pub fn get(&self, id: StableId) -> Result<&Row, BridgeError> {
        self.rows
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|i| &self.rows[i])
            .ok_or(BridgeError::UnknownId(id))
    }
    /// Snapshot-consistent extraction view: rows in [`StableId`] order.
    pub fn view(&self) -> ExtractionView {
        ExtractionView {
            tick: self.tick.unwrap_or(0),
            arena_hash: self.arena_hash,
            rows: self.rows.clone(),
        }
    }
    /// Merge parsed rows after checking them against the sorted store, so a
    /// duplicate changes nothing and the row order stays stable.
    fn commit(&mut self, mut fresh: Vec<Row>) -> Result<(), BridgeError> {
        fresh.sort_by_key(|row| row.id);
        for pair in fresh.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(BridgeError::DuplicateId(pair[0].id));
            }
        }
        for row in &fresh {
            if self
                .rows
                .binary_search_by_key(&row.id, |known| known.id)
                .is_ok()
            {
                return Err(BridgeError::DuplicateId(row.id));
            }
        }
        self.rows.append(&mut fresh);
        self.rows.sort_by_key(|row| row.id);
        Ok(())
    }
}

/// Recorded unify-vs-bridge verdict, quoted verbatim in the evidence log.
/// Recommendation: keep the bridge; do not unify yet.
pub const UNIFY_VS_BRIDGE_VERDICT: &str = "Bridge verdict (kept, unification deferred). \
Adapter coverage 3/3: arena ingest carries tick, checkpoint hash and fighter poses; \
scene ingest carries node ids, parent links and local poses (rotation/scale validate on \
the wire but do not enter rows: full transforms stay in the owning store); ecs ingest \
reuses the canonical snapshot bytes and extracts live slots with the 2D ordinate mapped \
to depth. Refusal semantics are identical across stores: unknown wire versions, short \
reads, trailing bytes, over-bound counts, non-finite poses, nil scene ids, duplicate \
stable ids, unknown lookup ids and arena tick skew all fail loudly with a typed error, \
and a failed ingest changes nothing. Unification would change four things: (1) a common \
snapshot envelope stamping tick, build hash and schema version on every store, so stale \
consumers detect skew without per-domain tick rules; (2) one id space with per-owner \
write rules enforced at the handle, replacing tag-folded stable ids; (3) referential \
checks the bridge skips, such as dangling scene parents; (4) typed component reads \
instead of position-only rows. None of that is free: it rewrites state paths across \
arena, scene and ecs against 130+ ticked tasks of evidence, so it waits until bridge \
pain (refusal gaps, id collisions, envelope skew) shows up in a consumer receipt. \
First consumer: ARC6 spine extraction reads ExtractionView, never sim state; \
ARC6 landed that consumer: the view is public and the editor spine inspector reads it.";

/// Normalize signed zero so `0.0` and `-0.0` digest identically.
fn canon_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Push `u16` LE.
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Push `u32` LE.
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Push `u64` LE.
fn push_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Push `f32` via raw bits LE (strict-IEEE, no fast-math).
fn push_f32(out: &mut Vec<u8>, v: f32) {
    push_u32(out, v.to_bits());
}

/// Bounded cursor over foreign bytes; every read names its domain on failure.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    domain: &'static str,
}

impl<'a> Cursor<'a> {
    /// Cursor at the start of `bytes`.
    fn new(bytes: &'a [u8], domain: &'static str) -> Self {
        Self {
            bytes,
            pos: 0,
            domain,
        }
    }
    /// Bytes left unread.
    fn rest(&self) -> usize {
        self.bytes.len() - self.pos
    }
    /// Take `N` bytes, bounds-checked.
    fn take<const N: usize>(&mut self) -> Result<[u8; N], BridgeError> {
        let end = self
            .pos
            .checked_add(N)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(BridgeError::TooShort {
                domain: self.domain,
            })?;
        let mut buf = [0u8; N];
        buf.copy_from_slice(&self.bytes[self.pos..end]);
        self.pos = end;
        Ok(buf)
    }
    /// Read `u8`.
    fn read_u8(&mut self) -> Result<u8, BridgeError> {
        Ok(self.take::<1>()?[0])
    }
    /// Read `u16` LE.
    fn read_u16(&mut self) -> Result<u16, BridgeError> {
        Ok(u16::from_le_bytes(self.take::<2>()?))
    }
    /// Read `u32` LE.
    fn read_u32(&mut self) -> Result<u32, BridgeError> {
        Ok(u32::from_le_bytes(self.take::<4>()?))
    }
    /// Read `u64` LE.
    fn read_u64(&mut self) -> Result<u64, BridgeError> {
        Ok(u64::from_le_bytes(self.take::<8>()?))
    }
    /// Read `f32` from raw-bits LE, finite-checked.
    fn read_f32(&mut self) -> Result<f32, BridgeError> {
        let v = f32::from_bits(self.read_u32()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(BridgeError::NonFinite {
                domain: self.domain,
            })
        }
    }
    /// Read 16 raw id bytes.
    fn read_id16(&mut self) -> Result<[u8; 16], BridgeError> {
        self.take::<16>()
    }
}

/// Decode arena bytes into its tick, match hash and rows.
fn parse_arena(bytes: &[u8]) -> Result<(u32, u64, Vec<Row>), BridgeError> {
    const DOMAIN: &str = "arena";
    let mut cur = Cursor::new(bytes, DOMAIN);
    let version = cur.read_u16()?;
    if version != ARENA_WIRE_VERSION {
        return Err(BridgeError::BadVersion {
            domain: DOMAIN,
            got: version,
        });
    }
    let tick = cur.read_u32()?;
    let hash = cur.read_u64()?;
    let count = cur.read_u32()? as usize;
    if count > ARENA_MAX_ROWS {
        return Err(BridgeError::TooMany {
            domain: DOMAIN,
            count,
        });
    }
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = cur.read_u8()?;
        let pos = [cur.read_f32()?, cur.read_f32()?, cur.read_f32()?];
        let yaw = cur.read_f32()?;
        rows.push(Row {
            id: StableId::arena(slot),
            domain: Domain::Arena,
            pos: [canon_zero(pos[0]), canon_zero(pos[1]), canon_zero(pos[2])],
            yaw: canon_zero(yaw),
            parent: None,
        });
    }
    if cur.rest() > 0 {
        return Err(BridgeError::Trailing {
            domain: DOMAIN,
            count: cur.rest(),
        });
    }
    Ok((tick, hash, rows))
}

/// Decode scene bytes into rows.
fn parse_scene(bytes: &[u8]) -> Result<Vec<Row>, BridgeError> {
    const DOMAIN: &str = "scene";
    let mut cur = Cursor::new(bytes, DOMAIN);
    let version = cur.read_u16()?;
    if version != SCENE_WIRE_VERSION {
        return Err(BridgeError::BadVersion {
            domain: DOMAIN,
            got: version,
        });
    }
    let count = cur.read_u32()? as usize;
    if count > SCENE_MAX_ROWS {
        return Err(BridgeError::TooMany {
            domain: DOMAIN,
            count,
        });
    }
    let mut rows = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let id = cur.read_id16()?;
        let parent = cur.read_id16()?;
        if id == [0; 16] {
            return Err(BridgeError::NilId);
        }
        let pos = [cur.read_f32()?, cur.read_f32()?, cur.read_f32()?];
        for _ in 0..4 {
            cur.read_f32()?;
        }
        for _ in 0..3 {
            cur.read_f32()?;
        }
        rows.push(Row {
            id: StableId::scene(&id),
            domain: Domain::Scene,
            pos: [canon_zero(pos[0]), canon_zero(pos[1]), canon_zero(pos[2])],
            yaw: 0.0,
            parent: if parent == [0; 16] {
                None
            } else {
                Some(StableId::scene(&parent))
            },
        });
    }
    if cur.rest() > 0 {
        return Err(BridgeError::Trailing {
            domain: DOMAIN,
            count: cur.rest(),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Position, Velocity, World};

    /// Two fighters plus a root/child scene pair plus a 2-alive ECS world.
    fn sample_inputs() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let arena = encode_arena(
            41,
            0x9E37_79B9_7F4A_7C15,
            &[
                ArenaFighter {
                    slot: 0,
                    pos: [1.0, 0.0, -2.0],
                    yaw: 0.5,
                },
                ArenaFighter {
                    slot: 1,
                    pos: [-3.0, 0.0, 4.0],
                    yaw: -1.0,
                },
            ],
        )
        .expect("sample arena encodes");
        let scene = encode_scene(&[
            SceneNode {
                id: [1; 16],
                parent: [0; 16],
                pos: [10.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
            SceneNode {
                id: [2; 16],
                parent: [1; 16],
                pos: [0.0, 2.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 2.0, 1.0],
            },
        ])
        .expect("sample scene encodes");
        let mut world = World::new();
        world.spawn(Position { x: 3.0, y: 4.0 }, Velocity { x: 1.0, y: -0.5 });
        world.spawn(Position { x: -1.0, y: 2.0 }, Velocity::default());
        world.kill(0);
        world.spawn(Position { x: 7.0, y: 8.0 }, Velocity { x: 0.25, y: 0.5 });
        let ecs = crate::snapshot::snapshot(&world).bytes;
        (arena, scene, ecs)
    }

    /// Acceptance: one extraction test reading all three states.
    #[test]
    fn extraction_reads_all_three_states() {
        let (arena, scene, ecs) = sample_inputs();
        let mut bridge = Bridge::new();
        assert!(bridge.is_empty());
        bridge.ingest_arena(&arena).expect("arena ingests");
        bridge.ingest_scene(&scene).expect("scene ingests");
        bridge.ingest_ecs(&ecs).expect("ecs ingests");
        assert_eq!(bridge.len(), 6);

        let view = bridge.view();
        assert_eq!(view.tick, 41);
        assert_eq!(view.arena_hash, Some(0x9E37_79B9_7F4A_7C15));
        assert_eq!(view.len(), 6);
        // Rows come out in stable-id order, not ingest order.
        let mut ids: Vec<StableId> = view.rows.iter().map(|row| row.id).collect();
        ids.sort();
        assert_eq!(ids, view.rows.iter().map(|row| row.id).collect::<Vec<_>>());

        // Each domain landed with its pose: arena slot 0, the scene child
        // with its parent link, and the ECS 2D ordinate mapped to depth.
        let arena_row = view.get(StableId::arena(0)).expect("arena slot 0");
        assert_eq!(arena_row.domain, Domain::Arena);
        assert_eq!(arena_row.pos, [1.0, 0.0, -2.0]);
        assert_eq!(arena_row.yaw, 0.5);
        let child = view.get(StableId::scene(&[2; 16])).expect("scene child");
        assert_eq!(child.domain, Domain::Scene);
        assert_eq!(child.parent, Some(StableId::scene(&[1; 16])));
        assert_eq!(child.pos, [0.0, 2.0, 0.0]);
        let ecs_row = view.get(StableId::ecs_index(2)).expect("ecs slot 2");
        assert_eq!(ecs_row.domain, Domain::Ecs);
        assert_eq!(ecs_row.pos, [7.0, 0.0, 8.0]);
        // The killed ECS slot extracts nothing; lookup refuses it loudly.
        assert_eq!(
            bridge.get(StableId::ecs_index(0)).err(),
            Some(BridgeError::UnknownId(StableId::ecs_index(0)))
        );

        // Namespaces never alias the same small index across domains.
        assert_ne!(StableId::arena(0), StableId::ecs_index(0));
        assert_ne!(StableId::arena(0), StableId::scene(&[1; 16]));
        assert_ne!(StableId::ecs_index(0), StableId::scene(&[1; 16]));
    }

    /// A three-fighter spine at tick 41: slot 1 is killed, so it extracts
    /// nothing; the snapshot is the only thing the bridge ever sees.
    fn sample_spine() -> SpineSnapshot {
        use crate::spine::{Body, Transform, Vitals};
        let mut spine = Spine::with_capacity(3);
        for (x, yaw) in [(1.0, 0.5), (-3.0, -1.0), (6.0, 2.0)] {
            spine
                .spawn(
                    Transform {
                        pos: [x, 0.0, -x],
                        yaw,
                    },
                    Body::default(),
                    Vitals {
                        hp_permille: 1000,
                        ..Vitals::default()
                    },
                )
                .expect("slot is free");
        }
        assert!(spine.kill(1));
        spine.snapshot()
    }

    /// ARC6 acceptance: the three-state extraction read, now over spine bytes.
    #[test]
    fn extraction_reads_all_three_states_from_spine_bytes() {
        let (_, scene, ecs) = sample_inputs();
        let spine = sample_spine();
        let mut bridge = Bridge::new();
        bridge.ingest_spine(41, &spine).expect("spine ingests");
        bridge.ingest_scene(&scene).expect("scene ingests");
        bridge.ingest_ecs(&ecs).expect("ecs ingests");
        // Two live fighters, two scene nodes, two live ECS slots.
        assert_eq!(bridge.len(), 6);

        let view = bridge.view();
        assert_eq!(view.tick, 41);
        assert_eq!(view.arena_hash, Some(spine.hash));
        let first = view.get(StableId::arena(0)).expect("spine slot 0");
        assert_eq!(first.domain, Domain::Arena);
        assert_eq!(first.pos, [1.0, 0.0, -1.0]);
        assert_eq!(first.yaw, 0.5);
        let third = view.get(StableId::arena(2)).expect("spine slot 2");
        assert_eq!(third.pos, [6.0, 0.0, -6.0]);
        // The killed slot extracts nothing; lookup refuses it loudly.
        assert_eq!(
            bridge.get(StableId::arena(1)).err(),
            Some(BridgeError::UnknownId(StableId::arena(1)))
        );
        // The scene and ECS rows ride the same frame beside the spine.
        assert!(view.get(StableId::scene(&[2; 16])).is_some());
        assert!(view.get(StableId::ecs_index(2)).is_some());
        // The same bytes always give the same frame, in any ingest order.
        let mut reversed = Bridge::new();
        reversed.ingest_ecs(&ecs).expect("ecs ingests");
        reversed.ingest_scene(&scene).expect("scene ingests");
        reversed.ingest_spine(41, &spine).expect("spine ingests");
        assert_eq!(reversed.view().digest(), view.digest());
    }

    #[test]
    fn spine_ingest_refuses_bad_bytes_and_skew_and_changes_nothing() {
        let spine = sample_spine();
        let mut bridge = Bridge::new();
        bridge.ingest_spine(41, &spine).expect("spine ingests");
        let before = bridge.view().digest();

        let mut forged = sample_spine();
        forged.hash ^= 1;
        assert_eq!(
            Bridge::new().ingest_spine(41, &forged).err(),
            Some(BridgeError::BadHash { domain: "spine" })
        );
        let mut short = sample_spine();
        short.bytes.truncate(3);
        short.hash = hash_bytes(&short.bytes);
        assert_eq!(
            Bridge::new().ingest_spine(41, &short).err(),
            Some(BridgeError::TooShort { domain: "spine" })
        );
        let mut stale = sample_spine();
        stale.bytes[0] = 9;
        stale.hash = hash_bytes(&stale.bytes);
        assert_eq!(
            Bridge::new().ingest_spine(41, &stale).err(),
            Some(BridgeError::BadVersion {
                domain: "spine",
                got: 9
            })
        );
        // A second frame on another tick is skew; the same tick repeats ids.
        assert_eq!(
            bridge.ingest_spine(42, &spine).err(),
            Some(BridgeError::TickSkew {
                got: 42,
                expected: 41
            })
        );
        assert_eq!(
            bridge.ingest_spine(41, &spine).err(),
            Some(BridgeError::DuplicateId(StableId::arena(0)))
        );
        assert_eq!(
            bridge.view().digest(),
            before,
            "failed ingests change nothing"
        );
    }

    #[test]
    fn bridge_wire_roundtrip_preserves_bytes() {
        // The encoded wire bytes survive ingest with an identical
        // digest, and re-encoding the same inputs gives identical bytes.
        let fighters = [
            ArenaFighter {
                slot: 0,
                pos: [1.0, 0.0, -2.0],
                yaw: 0.5,
            },
            ArenaFighter {
                slot: 1,
                pos: [-3.0, 0.0, 4.0],
                yaw: -1.0,
            },
        ];
        let bytes = encode_arena(41, 0x9E37_79B9_7F4A_7C15, &fighters).expect("encodes");
        let mut first = Bridge::new();
        first.ingest_arena(&bytes).expect("ingests");
        let digest = first.view().digest();
        let bytes_again = encode_arena(41, 0x9E37_79B9_7F4A_7C15, &fighters).expect("re-encodes");
        assert_eq!(bytes_again, bytes, "encode is deterministic");
        let mut second = Bridge::new();
        second.ingest_arena(&bytes_again).expect("re-ingests");
        assert_eq!(second.view().digest(), digest, "roundtrip keeps digest");
    }

    #[test]
    fn ingest_order_does_not_change_the_digest() {
        let (arena, scene, ecs) = sample_inputs();
        let mut first = Bridge::new();
        first.ingest_arena(&arena).expect("arena");
        first.ingest_scene(&scene).expect("scene");
        first.ingest_ecs(&ecs).expect("ecs");
        let mut second = Bridge::new();
        second.ingest_ecs(&ecs).expect("ecs");
        second.ingest_arena(&arena).expect("arena");
        second.ingest_scene(&scene).expect("scene");
        assert_eq!(first.view(), second.view());
        assert_eq!(first.view().digest(), second.view().digest());
        // Same inputs rebuilt give the same digest; an empty view differs.
        let mut third = Bridge::new();
        let (a2, s2, e2) = sample_inputs();
        third.ingest_scene(&s2).expect("scene");
        third.ingest_ecs(&e2).expect("ecs");
        third.ingest_arena(&a2).expect("arena");
        assert_eq!(first.view().digest(), third.view().digest());
        assert_ne!(first.view().digest(), Bridge::new().view().digest());
    }

    #[test]
    fn seeded_row_and_ingest_permutation_moves_zero_digests() {
        // Rows stay in StableId order (sorted iteration), so seeded
        // permutations of the fighter list, the scene list and the ingest
        // order never move the digest.
        fn xorshift(mut x: u64) -> u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
        let fighters = [
            ArenaFighter {
                slot: 0,
                pos: [1.0, 0.0, -2.0],
                yaw: 0.5,
            },
            ArenaFighter {
                slot: 1,
                pos: [-3.0, 0.0, 4.0],
                yaw: -1.0,
            },
            ArenaFighter {
                slot: 2,
                pos: [0.5, 1.0, 0.5],
                yaw: 0.0,
            },
        ];
        let nodes = [
            SceneNode {
                id: [1; 16],
                parent: [0; 16],
                pos: [10.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
            SceneNode {
                id: [2; 16],
                parent: [1; 16],
                pos: [0.0, 2.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 2.0, 1.0],
            },
            SceneNode {
                id: [3; 16],
                parent: [0; 16],
                pos: [-4.0, 0.0, 1.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [2.0, 1.0, 1.0],
            },
        ];
        let mut rev_fighters = fighters;
        rev_fighters.reverse();
        let mut rev_nodes = nodes;
        rev_nodes.reverse();
        // Seeded shuffle of three rows with a fixed key.
        let mut shuf_fighters = fighters;
        let mut rng = 0x5EED_00D1_5EED_00D1u64 | 1;
        for i in (1..shuf_fighters.len()).rev() {
            rng = xorshift(rng);
            shuf_fighters.swap(i, (rng as usize) % (i + 1));
        }
        let arena_fwd = encode_arena(41, 0x9E37_79B9_7F4A_7C15, &fighters).expect("arena");
        let arena_rev = encode_arena(41, 0x9E37_79B9_7F4A_7C15, &rev_fighters).expect("arena");
        let arena_shuf = encode_arena(41, 0x9E37_79B9_7F4A_7C15, &shuf_fighters).expect("arena");
        let scene_fwd = encode_scene(&nodes).expect("scene");
        let scene_rev = encode_scene(&rev_nodes).expect("scene");
        let (_, _, ecs) = sample_inputs();
        for arena in [&arena_fwd, &arena_rev, &arena_shuf] {
            for scene in [&scene_fwd, &scene_rev] {
                for order in 0..6 {
                    let mut bridge = Bridge::new();
                    // All six store ingest orders.
                    let seq: [&str; 3] = match order {
                        0 => ["arena", "scene", "ecs"],
                        1 => ["arena", "ecs", "scene"],
                        2 => ["scene", "arena", "ecs"],
                        3 => ["scene", "ecs", "arena"],
                        4 => ["ecs", "arena", "scene"],
                        _ => ["ecs", "scene", "arena"],
                    };
                    for store in seq {
                        match store {
                            "arena" => bridge.ingest_arena(arena).expect("arena"),
                            "scene" => bridge.ingest_scene(scene).expect("scene"),
                            _ => bridge.ingest_ecs(&ecs).expect("ecs"),
                        }
                    }
                    let mut baseline = Bridge::new();
                    baseline.ingest_arena(&arena_fwd).expect("arena");
                    baseline.ingest_scene(&scene_fwd).expect("scene");
                    baseline.ingest_ecs(&ecs).expect("ecs");
                    assert_eq!(
                        bridge.view().digest(),
                        baseline.view().digest(),
                        "permutation moved the digest (order {order})"
                    );
                }
            }
        }
        // A real change still moves the digest, so the test is not vacuous.
        let other = encode_arena(
            41,
            0x9E37_79B9_7F4A_7C15,
            &[ArenaFighter {
                slot: 0,
                pos: [9.0, 0.0, -2.0],
                yaw: 0.5,
            }],
        )
        .expect("arena");
        let mut a = Bridge::new();
        a.ingest_arena(&arena_fwd).expect("arena");
        let mut b = Bridge::new();
        b.ingest_arena(&other).expect("arena");
        assert_ne!(a.view().digest(), b.view().digest());
    }

    #[test]
    fn unknown_versions_refuse_loudly() {
        let (arena, scene, ecs) = sample_inputs();
        let mut bad_arena = arena.clone();
        bad_arena[0] = 0xFF;
        bad_arena[1] = 0xFF;
        assert_eq!(
            Bridge::new().ingest_arena(&bad_arena).err(),
            Some(BridgeError::BadVersion {
                domain: "arena",
                got: 0xFFFF
            })
        );
        let mut bad_scene = scene.clone();
        bad_scene[0] = 0x02;
        assert_eq!(
            Bridge::new().ingest_scene(&bad_scene).err(),
            Some(BridgeError::BadVersion {
                domain: "scene",
                got: 0x0002
            })
        );
        let mut bad_ecs = ecs.clone();
        bad_ecs[0] = 0x7B;
        bad_ecs[1] = 0x00;
        assert_eq!(
            Bridge::new().ingest_ecs(&bad_ecs).err(),
            Some(BridgeError::BadVersion {
                domain: "ecs",
                got: 0x007B
            })
        );
    }

    #[test]
    fn short_trailing_and_bounded_reads_refuse() {
        let (arena, scene, _) = sample_inputs();
        // Truncated headers and bodies.
        assert_eq!(
            Bridge::new().ingest_arena(&arena[..7]).err(),
            Some(BridgeError::TooShort { domain: "arena" })
        );
        assert_eq!(
            Bridge::new().ingest_arena(&arena[..arena.len() - 3]).err(),
            Some(BridgeError::TooShort { domain: "arena" })
        );
        assert_eq!(
            Bridge::new().ingest_scene(&[]).err(),
            Some(BridgeError::TooShort { domain: "scene" })
        );
        assert_eq!(
            Bridge::new().ingest_ecs(&[1, 0, 0]).err(),
            Some(BridgeError::TooShort { domain: "ecs" })
        );
        // Trailing bytes after a full decode.
        let mut trailing = arena.clone();
        trailing.push(0);
        assert_eq!(
            Bridge::new().ingest_arena(&trailing).err(),
            Some(BridgeError::Trailing {
                domain: "arena",
                count: 1
            })
        );
        let mut trailing_scene = scene.clone();
        trailing_scene.extend_from_slice(&[9, 9]);
        assert_eq!(
            Bridge::new().ingest_scene(&trailing_scene).err(),
            Some(BridgeError::Trailing {
                domain: "scene",
                count: 2
            })
        );
        // Absurd counts refuse before any big allocation.
        let mut huge_arena = vec![1, 0];
        huge_arena.extend_from_slice(&1u32.to_le_bytes());
        huge_arena.extend_from_slice(&0u64.to_le_bytes());
        huge_arena.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Bridge::new().ingest_arena(&huge_arena).err(),
            Some(BridgeError::TooMany {
                domain: "arena",
                count: u32::MAX as usize
            })
        );
        let mut huge_scene = vec![1, 0];
        huge_scene.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Bridge::new().ingest_scene(&huge_scene).err(),
            Some(BridgeError::TooMany {
                domain: "scene",
                count: u32::MAX as usize
            })
        );
        let mut huge_ecs = vec![1, 0];
        huge_ecs.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Bridge::new().ingest_ecs(&huge_ecs).err(),
            Some(BridgeError::TooMany {
                domain: "ecs",
                count: u32::MAX as usize
            })
        );
        // Encoder bounds match the ingest bounds.
        let many: Vec<ArenaFighter> = (0..=255)
            .map(|i| ArenaFighter {
                slot: i as u8,
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
            })
            .collect();
        assert_eq!(many.len(), 256);
        assert!(encode_arena(0, 0, &many).is_ok());
        assert_eq!(
            encode_arena(0, 0, &[many.clone(), many.clone()].concat()).err(),
            Some(BridgeError::TooMany {
                domain: "arena",
                count: 512
            })
        );
    }

    #[test]
    fn identity_and_frame_refusals() {
        let (arena, scene, _) = sample_inputs();
        // Nil scene ids never encode and never decode past the gate.
        let nilled = SceneNode {
            id: [0; 16],
            parent: [0; 16],
            pos: [0.0, 0.0, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        };
        assert_eq!(encode_scene(&[nilled]).err(), Some(BridgeError::NilId));
        // Non-finite poses fail at encode time on both foreign layouts.
        let nan_fighter = ArenaFighter {
            slot: 0,
            pos: [f32::NAN, 0.0, 0.0],
            yaw: 0.0,
        };
        assert_eq!(
            encode_arena(0, 0, &[nan_fighter]).err(),
            Some(BridgeError::NonFinite { domain: "arena" })
        );
        let nan_node = SceneNode {
            id: [7; 16],
            parent: [0; 16],
            pos: [0.0, f32::INFINITY, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        };
        assert_eq!(
            encode_scene(&[nan_node]).err(),
            Some(BridgeError::NonFinite { domain: "scene" })
        );
        // Non-finite bytes smuggled past the encoders still refuse on ingest.
        let mut nan_bytes = encode_arena(
            2,
            0,
            &[ArenaFighter {
                slot: 0,
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
            }],
        )
        .expect("finite encodes");
        nan_bytes[19] = 0x00;
        nan_bytes[20] = 0x00;
        nan_bytes[21] = 0xC0;
        nan_bytes[22] = 0x7F;
        assert_eq!(
            Bridge::new().ingest_arena(&nan_bytes).err(),
            Some(BridgeError::NonFinite { domain: "arena" })
        );
        // Ingesting the same bytes twice repeats the ids, loudly.
        let mut bridge = Bridge::new();
        bridge.ingest_arena(&arena).expect("first arena");
        assert_eq!(
            bridge.ingest_arena(&arena).err(),
            Some(BridgeError::DuplicateId(StableId::arena(0)))
        );
        bridge.ingest_scene(&scene).expect("first scene");
        assert_eq!(
            bridge.ingest_scene(&scene).err(),
            Some(BridgeError::DuplicateId(StableId::scene(&[1; 16])))
        );
        // A failed ingest changes nothing: the view still holds 4 rows.
        assert_eq!(bridge.len(), 4);
        // A second arena frame on a new tick is skew, not a merge.
        let next = encode_arena(
            42,
            0,
            &[ArenaFighter {
                slot: 0,
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
            }],
        )
        .expect("next frame encodes");
        assert_eq!(
            bridge.ingest_arena(&next).err(),
            Some(BridgeError::TickSkew {
                got: 42,
                expected: 41
            })
        );
        assert_eq!(bridge.len(), 4);
        // Unknown lookups name the id.
        let missing = StableId::arena(9);
        assert_eq!(
            bridge.get(missing).err(),
            Some(BridgeError::UnknownId(missing))
        );
        // Error text names the culprit for operator diagnostics.
        assert!(BridgeError::UnknownId(missing)
            .to_string()
            .contains("stable:"));
        assert!(BridgeError::TickSkew {
            got: 42,
            expected: 41
        }
        .to_string()
        .contains("42"));
    }

    #[test]
    fn verdict_names_the_tradeoff_for_evidence() {
        assert!(!UNIFY_VS_BRIDGE_VERDICT.is_empty());
        for term in ["bridge", "unification", "refus", "extraction", "tick"] {
            assert!(
                UNIFY_VS_BRIDGE_VERDICT.contains(term),
                "verdict must discuss {term}"
            );
        }
    }
}
