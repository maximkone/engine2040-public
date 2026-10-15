//! replicate: server-authoritative component replication (2-64 entities).
//!
//! Owner: `engine-net`; consumer: server-authoritative sessions: the
//! [`crate::server::Server`] owns truth and [`publish_server_positions`]
//! mirrors its positions into the [`ReplicationServer`] world each tick.
//! Units: entity ids (`u32`), ticks (`u32`), mm/yaw-ddeg ints. Determinism:
//! integer-only math, `BTreeMap` order, FNV hashes; no float, no wall clock,
//! no `HashMap` in hashed state. Errors: [`ReplicateError`] refuses loudly.
//!
//! Idea-only PORT(cBournhonesque/lightyear@main:crates/replication): the
//! server-plugin/client-plugin split (authority emits spawn/delta/despawn
//! against each client's acked tick; client plugin mirrors) reimplemented
//! clean-room, no vendor bytes. License MIT OR Apache-2.0 (repo API spdx
//! Apache-2.0; LICENSE-MIT + LICENSE-APACHE at root on main, verified
//! 2026-09-26 before code).
//! Budget: one message per client per tick, at most [`MAX_MESSAGE_BYTES`] B;
//! history holds [`MAX_HISTORY`] snapshots; tombstones derive from diffs.

use std::collections::{BTreeMap, VecDeque};

use engine_core::Fnv64;

/// Wire version accepted by [`decode_message`]; anything else refuses.
pub const REPLICATE_VERSION: u8 = 1;
/// Wire magic 0x3252: ASCII `R` then `2`, the first two bytes of every message.
pub const REPLICATE_MAGIC: u16 = 0x3252;
/// Entity capacity: matches the `engine-net` server transport bound.
pub const MAX_ENTITIES: usize = 64;
/// Upper bound on one encoded message in bytes (flood guard).
pub const MAX_MESSAGE_BYTES: usize = 1200;
/// Snapshots retained for ack-based diffs; older baselines resync full.
pub const MAX_HISTORY: usize = 33;
/// Ticks between forced full-state resyncs so lossy clients converge.
pub const RESYNC_EVERY: u32 = 16;
/// Field mask bit: [`Transform`] present in an update.
pub const MASK_TRANSFORM: u8 = 0x01;
/// Field mask bit: [`Vitals`] present in an update.
pub const MASK_VITALS: u8 = 0x02;
/// All field mask bits; any other bit refuses the update.
pub const MASK_ALL: u8 = MASK_TRANSFORM | MASK_VITALS;
/// Message flag bit: full-state resync (mirror clears, then spawns).
pub const RESYNC_FLAG: u8 = 0x01;
/// Encoded header length: magic 2 + version 1 + tick 4 + baseline 4 + flags 1 + count 1.
pub const HEADER_LEN: usize = 13;
/// Encoded full-entry length: id 4 + transform 10 + vitals 4.
pub const SPAWN_ENTRY_LEN: usize = 18;
/// Encoded despawn entry length: id 4.
pub const DESPAWN_ENTRY_LEN: usize = 4;

/// Replicated entity handle. Any `u32` is a valid id; the server owns them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntityId(pub u32);

/// Quantized transform in integer millimeters plus integer decidegrees.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Transform {
    /// Feet x in millimeters.
    pub x_mm: i32,
    /// Feet y in millimeters.
    pub y_mm: i32,
    /// Facing yaw in decidegrees, `[-1800, 1800)`.
    pub yaw_ddeg: i16,
}

/// Quantized combat state: permille hp plus stamina points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Vitals {
    /// Health in permille.
    pub hp: u16,
    /// Stamina points.
    pub stamina: u16,
}

impl Vitals {
    /// Fresh-fighter vitals: full health, full stamina.
    pub const FULL: Self = Self {
        hp: 1000,
        stamina: 100,
    };
}

/// One entity's full component set: the spawn unit and the diff baseline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FullEntry {
    /// Entity this entry describes.
    pub entity: EntityId,
    /// Authoritative transform.
    pub transform: Transform,
    /// Authoritative vitals.
    pub vitals: Vitals,
}

/// One entity's change set: `mask` names the components that differ from the
/// baseline; unnamed components are zeroed and ignored. `mask` is never zero
/// and never carries bits outside [`MASK_ALL`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntityUpdate {
    /// Entity this update applies to.
    pub entity: EntityId,
    /// Component mask over `MASK_*`; nonzero, subset of [`MASK_ALL`].
    pub mask: u8,
    /// New transform when [`MASK_TRANSFORM`] is set, else zeroed.
    pub transform: Transform,
    /// New vitals when [`MASK_VITALS`] is set, else zeroed.
    pub vitals: Vitals,
}

/// Server-to-client replication message for one server tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReplicationMessage {
    /// Server tick this message covers.
    pub tick: u32,
    /// Acked baseline tick the sender diffed against.
    pub baseline: u32,
    /// Full-state resync: the client clears its mirror, then spawns.
    pub resync: bool,
    /// New entities (or every entity on resync), sorted by id.
    pub spawns: Vec<FullEntry>,
    /// Changed entities with field masks, sorted by id.
    pub updates: Vec<EntityUpdate>,
    /// Removed entities, sorted by id.
    pub despawns: Vec<EntityId>,
}

/// Replication failures: bounded diagnostics, never panics, never partial.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplicateError {
    /// Entity id already present in the world.
    DuplicateEntity(u32),
    /// Update or lookup names an entity the world does not hold.
    UnknownEntity(u32),
    /// Ack names a client with no sync state, or a message batch names one.
    UnknownClient(u32),
    /// Ack or apply targets an already-passed tick; carries both ticks.
    StaleTick {
        /// Requested tick.
        tick: u32,
        /// Newest tick held.
        current: u32,
    },
    /// Ack targets a tick past the authored tip; carries both ticks.
    FutureTick {
        /// Requested tick.
        tick: u32,
        /// Newest tick held.
        current: u32,
    },
    /// Wire version byte is not [`REPLICATE_VERSION`]; culprit carried.
    VersionMismatch {
        /// Version byte found on the wire.
        got: u8,
    },
    /// Magic bytes do not match [`REPLICATE_MAGIC`].
    BadMagic,
    /// Message ends mid-field or carries trailing bytes.
    Truncated,
    /// Encoded message exceeds [`MAX_MESSAGE_BYTES`].
    TooLarge,
    /// One section names more than [`MAX_ENTITIES`] entries; count carried.
    TooManyEntities(usize),
    /// Update mask is zero or carries bits outside [`MASK_ALL`].
    BadMask(u8),
    /// Client mirror disagrees with server truth at `tick`.
    Desync {
        /// Server tick compared at.
        tick: u32,
        /// Server world hash.
        server: u64,
        /// Client world hash.
        client: u64,
    },
}

impl std::fmt::Display for ReplicateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::DuplicateEntity(id) => write!(f, "entity {id} already replicated"),
            Self::UnknownEntity(id) => write!(f, "unknown entity {id}"),
            Self::UnknownClient(id) => write!(f, "unknown client {id}"),
            Self::StaleTick { tick, current } => {
                write!(f, "stale tick {tick}, current {current}")
            }
            Self::FutureTick { tick, current } => {
                write!(f, "future tick {tick}, current {current}")
            }
            Self::VersionMismatch { got } => {
                write!(f, "replication version mismatch: got {got}")
            }
            Self::BadMagic => write!(f, "bad replication magic"),
            Self::Truncated => write!(f, "truncated replication message"),
            Self::TooLarge => write!(f, "replication message exceeds size bound"),
            Self::TooManyEntities(n) => write!(f, "too many entities: {n}"),
            Self::BadMask(m) => write!(f, "bad component mask {m:#04x}"),
            Self::Desync {
                tick,
                server,
                client,
            } => write!(
                f,
                "replication desync at tick {tick}: server {server:#016x} != client {client:#016x}"
            ),
        }
    }
}

impl std::error::Error for ReplicateError {}

/// Server replication plugin: owns the authoritative component world, a
/// bounded snapshot history for ack-based diffs, and per-client ack cursors.
/// Data owner for replicated truth; clients never write here.
pub struct ReplicationServer {
    tick: u32,
    world: BTreeMap<EntityId, FullEntry>,
    history: VecDeque<(u32, BTreeMap<EntityId, FullEntry>)>,
    acked: BTreeMap<u32, u32>,
}

impl ReplicationServer {
    /// Open an empty authoritative world at tick zero.
    pub fn new() -> Self {
        Self {
            tick: 0,
            world: BTreeMap::new(),
            history: VecDeque::new(),
            acked: BTreeMap::new(),
        }
    }

    /// Last authored server tick.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Live entity count.
    pub fn len(&self) -> usize {
        self.world.len()
    }

    /// True when no entity is replicated.
    pub fn is_empty(&self) -> bool {
        self.world.is_empty()
    }

    /// Full entry for `entity`, if replicated.
    pub fn entry(&self, entity: EntityId) -> Option<FullEntry> {
        self.world.get(&entity).copied()
    }

    /// Author one fixed tick: stamps the clock and snapshots the world for
    /// future ack-based diffs. Steady state is one bounded snapshot push.
    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.history.push_back((self.tick, self.world.clone()));
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
    }

    /// Replicate one entity with full components. Duplicate ids refuse and
    /// change nothing; worlds past capacity refuse the same way.
    pub fn spawn(
        &mut self,
        entity: EntityId,
        transform: Transform,
        vitals: Vitals,
    ) -> Result<(), ReplicateError> {
        if self.world.contains_key(&entity) {
            return Err(ReplicateError::DuplicateEntity(entity.0));
        }
        if self.world.len() >= MAX_ENTITIES {
            return Err(ReplicateError::TooManyEntities(self.world.len() + 1));
        }
        self.world.insert(
            entity,
            FullEntry {
                entity,
                transform,
                vitals,
            },
        );
        Ok(())
    }

    /// Remove one entity; unknown ids refuse and change nothing. The next
    /// diff against an older baseline names the tombstone automatically.
    pub fn despawn(&mut self, entity: EntityId) -> Result<(), ReplicateError> {
        if self.world.remove(&entity).is_none() {
            return Err(ReplicateError::UnknownEntity(entity.0));
        }
        Ok(())
    }

    /// Authoritatively move one entity; unknown ids refuse loudly.
    pub fn set_transform(
        &mut self,
        entity: EntityId,
        transform: Transform,
    ) -> Result<(), ReplicateError> {
        let Some(entry) = self.world.get_mut(&entity) else {
            return Err(ReplicateError::UnknownEntity(entity.0));
        };
        entry.transform = transform;
        Ok(())
    }

    /// Authoritatively wound or restore one entity; unknown ids refuse loudly.
    pub fn set_vitals(&mut self, entity: EntityId, vitals: Vitals) -> Result<(), ReplicateError> {
        let Some(entry) = self.world.get_mut(&entity) else {
            return Err(ReplicateError::UnknownEntity(entity.0));
        };
        entry.vitals = vitals;
        Ok(())
    }

    /// Forget one client's ack cursor (on disconnect). True when held.
    pub fn remove_client(&mut self, client: u32) -> bool {
        self.acked.remove(&client).is_some()
    }

    /// Last tick acked by `client`, if it ever acked.
    pub fn acked_tick(&self, client: u32) -> Option<u32> {
        self.acked.get(&client).copied()
    }

    /// Record one client's ack: advances its baseline so later diffs shrink.
    /// Duplicate or stale acks answer `Ok(false)` and change nothing; acks
    /// past the authored tip refuse with [`ReplicateError::FutureTick`].
    pub fn on_ack(&mut self, client: u32, tick: u32) -> Result<bool, ReplicateError> {
        if tick > self.tick {
            return Err(ReplicateError::FutureTick {
                tick,
                current: self.tick,
            });
        }
        match self.acked.get(&client).copied() {
            Some(done) if tick <= done => Ok(false),
            _ => {
                self.acked.insert(client, tick);
                Ok(true)
            }
        }
    }

    /// Build one client's message for the current tick: spawns for entities
    /// newer than its acked baseline, field-mask deltas for changed ones,
    /// tombstones for removed ones. Unknown, evicted or older baselines
    /// (`tick - acked >= RESYNC_EVERY`), and every [`RESYNC_EVERY`]th tick,
    /// fall back to a full-state resync, which always fits
    /// [`MAX_MESSAGE_BYTES`]; a diff that would overflow the budget degrades
    /// to the same full resync rather than truncating truth. The baseline
    /// only reads the client's last-acked tick, never redefines it.
    pub fn message_for(&self, client: u32) -> Result<ReplicationMessage, ReplicateError> {
        let acked = self.acked.get(&client).copied();
        let baseline = acked.and_then(|tick| {
            self.history
                .iter()
                .find(|(held, _)| *held == tick)
                .map(|(_, world)| world)
        });
        let stale = acked.is_some_and(|last| self.tick.wrapping_sub(last) >= RESYNC_EVERY);
        let want_resync = baseline.is_none() || stale || self.tick.is_multiple_of(RESYNC_EVERY);
        if want_resync {
            return Ok(ReplicationMessage {
                tick: self.tick,
                baseline: acked.unwrap_or(self.tick),
                resync: true,
                spawns: self.world.values().copied().collect(),
                updates: Vec::new(),
                despawns: Vec::new(),
            });
        }
        let base = baseline.expect("non-resync path holds a baseline");
        let floor = acked.expect("non-resync path holds an ack");
        let mut spawns = Vec::new();
        let mut updates = Vec::new();
        for entry in self.world.values() {
            match base.get(&entry.entity) {
                None => spawns.push(*entry),
                Some(old) if *old != *entry => {
                    let mut mask = 0u8;
                    if old.transform != entry.transform {
                        mask |= MASK_TRANSFORM;
                    }
                    if old.vitals != entry.vitals {
                        mask |= MASK_VITALS;
                    }
                    updates.push(EntityUpdate {
                        entity: entry.entity,
                        mask,
                        transform: if mask & MASK_TRANSFORM != 0 {
                            entry.transform
                        } else {
                            Transform::default()
                        },
                        vitals: if mask & MASK_VITALS != 0 {
                            entry.vitals
                        } else {
                            Vitals { hp: 0, stamina: 0 }
                        },
                    });
                }
                _ => {}
            }
        }
        let mut despawns: Vec<EntityId> = base
            .keys()
            .filter(|entity| !self.world.contains_key(entity))
            .copied()
            .collect();
        despawns.sort();
        despawns.dedup();
        let mut message = ReplicationMessage {
            tick: self.tick,
            baseline: floor,
            resync: false,
            spawns,
            updates,
            despawns,
        };
        if wire_len(&message) > MAX_MESSAGE_BYTES {
            message = ReplicationMessage {
                tick: self.tick,
                baseline: floor,
                resync: true,
                spawns: self.world.values().copied().collect(),
                updates: Vec::new(),
                despawns: Vec::new(),
            };
        }
        if wire_len(&message) > MAX_MESSAGE_BYTES {
            return Err(ReplicateError::TooLarge);
        }
        Ok(message)
    }

    /// Canonical world hash over the sorted entity set (FNV-1a, wrapping
    /// multiply): the same entities twice give the same hash. No allocation.
    pub fn world_hash(&self) -> u64 {
        world_hash(&self.world)
    }

    /// Compare one client mirror hash against server truth. Matches answer
    /// `Ok(())`; mismatches refuse with [`ReplicateError::Desync`].
    pub fn verify_client(&self, client_hash: u64) -> Result<(), ReplicateError> {
        let server = self.world_hash();
        if server == client_hash {
            Ok(())
        } else {
            Err(ReplicateError::Desync {
                tick: self.tick,
                server,
                client: client_hash,
            })
        }
    }
}

impl Default for ReplicationServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Client replication plugin: mirrors one server's component world by
/// applying [`ReplicationMessage`]s in tick order. Never extrapolates: a
/// message the client cannot apply refuses loudly and changes nothing.
pub struct ReplicationClient {
    mirror: BTreeMap<EntityId, FullEntry>,
    last_tick: Option<u32>,
    applied: u64,
    resyncs: u64,
}

/// What one [`ReplicationClient::apply`] call did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ApplyReport {
    /// Server tick applied (or already held, on duplicates).
    pub tick: u32,
    /// Entities spawned, updated or despawned by this call.
    pub applied: usize,
    /// True when this call was a full-state resync.
    pub resync: bool,
    /// True when the message was a duplicate or stale tick.
    pub duplicate: bool,
}

impl ReplicationClient {
    /// Open an empty mirror holding no server tick.
    pub fn new() -> Self {
        Self {
            mirror: BTreeMap::new(),
            last_tick: None,
            applied: 0,
            resyncs: 0,
        }
    }

    /// Last server tick applied, if any message arrived.
    pub fn last_tick(&self) -> Option<u32> {
        self.last_tick
    }

    /// Mirrored entity count.
    pub fn len(&self) -> usize {
        self.mirror.len()
    }

    /// True when nothing is mirrored yet.
    pub fn is_empty(&self) -> bool {
        self.mirror.is_empty()
    }

    /// Mirrored components for `entity`, if present.
    pub fn entry(&self, entity: EntityId) -> Option<FullEntry> {
        self.mirror.get(&entity).copied()
    }

    /// Messages applied so far (duplicates excluded).
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Full-state resyncs applied so far.
    pub fn resyncs(&self) -> u64 {
        self.resyncs
    }

    /// Tick to ack back to the server (the newest applied tick).
    pub fn ack_tick(&self) -> Option<u32> {
        self.last_tick
    }

    /// Apply one server message: validates shapes, order and references
    /// first, then mutates. Duplicates and stale ticks answer `duplicate`
    /// and change nothing; updates for entities the mirror never spawned
    /// refuse with [`ReplicateError::UnknownEntity`] and change nothing.
    pub fn apply(&mut self, message: &ReplicationMessage) -> Result<ApplyReport, ReplicateError> {
        if self.last_tick.is_some_and(|done| message.tick <= done) {
            return Ok(ApplyReport {
                tick: message.tick,
                applied: 0,
                resync: false,
                duplicate: true,
            });
        }
        validate_message(message)?;
        if !message.resync {
            for update in &message.updates {
                let known = self.mirror.contains_key(&update.entity)
                    || message
                        .spawns
                        .iter()
                        .any(|spawn| spawn.entity == update.entity);
                if !known {
                    return Err(ReplicateError::UnknownEntity(update.entity.0));
                }
            }
            for spawn in &message.spawns {
                if self.mirror.contains_key(&spawn.entity) {
                    return Err(ReplicateError::DuplicateEntity(spawn.entity.0));
                }
            }
        }
        let mut applied = 0usize;
        if message.resync {
            self.mirror.clear();
            self.resyncs += 1;
        }
        for spawn in &message.spawns {
            self.mirror.insert(spawn.entity, *spawn);
            applied += 1;
        }
        for update in &message.updates {
            let Some(entry) = self.mirror.get_mut(&update.entity) else {
                return Err(ReplicateError::UnknownEntity(update.entity.0));
            };
            if update.mask & MASK_TRANSFORM != 0 {
                entry.transform = update.transform;
            }
            if update.mask & MASK_VITALS != 0 {
                entry.vitals = update.vitals;
            }
            applied += 1;
        }
        for entity in &message.despawns {
            if self.mirror.remove(entity).is_some() {
                applied += 1;
            }
        }
        self.last_tick = Some(message.tick);
        self.applied += 1;
        Ok(ApplyReport {
            tick: message.tick,
            applied,
            resync: message.resync,
            duplicate: false,
        })
    }

    /// Canonical mirror hash with the same fold as the server: equal worlds
    /// hash equal, so agreement is one comparison.
    pub fn world_hash(&self) -> u64 {
        world_hash(&self.mirror)
    }
}

impl Default for ReplicationClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Mirror one authoritative [`crate::server::Server`] session into the
/// replication world: every linked player spawns or moves its entity (cm to
/// mm, saturating), and entities whose player left despawn. Returns the
/// mirrored entity count. This is the consumer seam: server truth in,
/// component replication out.
pub fn publish_server_positions(
    repl: &mut ReplicationServer,
    session: &crate::server::Server,
    links: &[(u32, EntityId)],
) -> usize {
    for (player, entity) in links {
        match session.pos(*player) {
            Some((x_cm, y_cm)) => {
                let transform = Transform {
                    x_mm: x_cm.saturating_mul(10),
                    y_mm: y_cm.saturating_mul(10),
                    yaw_ddeg: 0,
                };
                match repl.entry(*entity) {
                    None => {
                        let _ = repl.spawn(*entity, transform, Vitals::FULL);
                    }
                    Some(current) if current.transform != transform => {
                        let _ = repl.set_transform(*entity, transform);
                    }
                    _ => {}
                }
            }
            None => {
                if repl.entry(*entity).is_some() {
                    let _ = repl.despawn(*entity);
                }
            }
        }
    }
    repl.len()
}

/// Canonical fold over an entity set: count plus sorted LE field bytes,
/// folded in order through the shared [`Fnv64`] owner.
fn world_hash(world: &BTreeMap<EntityId, FullEntry>) -> u64 {
    let mut fold = Fnv64::new();
    fold.write(&world.len().to_le_bytes());
    for (id, entry) in world {
        fold.write(&id.0.to_le_bytes());
        fold.write(&entry.transform.x_mm.to_le_bytes());
        fold.write(&entry.transform.y_mm.to_le_bytes());
        fold.write(&entry.transform.yaw_ddeg.to_le_bytes());
        fold.write(&entry.vitals.hp.to_le_bytes());
        fold.write(&entry.vitals.stamina.to_le_bytes());
    }
    fold.finish()
}

/// Canonical encoded length of one message (header plus sections).
pub fn wire_len(message: &ReplicationMessage) -> usize {
    HEADER_LEN
        + message.spawns.len() * SPAWN_ENTRY_LEN
        + 1
        + message
            .updates
            .iter()
            .map(|update| {
                DESPAWN_ENTRY_LEN
                    + 1
                    + (if update.mask & MASK_TRANSFORM != 0 {
                        10
                    } else {
                        0
                    })
                    + (if update.mask & MASK_VITALS != 0 { 4 } else { 0 })
            })
            .sum::<usize>()
        + 1
        + message.despawns.len() * DESPAWN_ENTRY_LEN
}

/// Shape check shared by encoders, decoders and appliers: section bounds,
/// sorted unique ids, nonzero in-mask updates with zeroed unnamed fields.
fn validate_message(message: &ReplicationMessage) -> Result<(), ReplicateError> {
    for section in [
        message.spawns.len(),
        message.updates.len(),
        message.despawns.len(),
    ] {
        if section > MAX_ENTITIES {
            return Err(ReplicateError::TooManyEntities(section));
        }
    }
    let mut prev: Option<u32> = None;
    for spawn in &message.spawns {
        if prev.is_some_and(|id| spawn.entity.0 <= id) {
            return Err(ReplicateError::DuplicateEntity(spawn.entity.0));
        }
        prev = Some(spawn.entity.0);
    }
    prev = None;
    for update in &message.updates {
        if prev.is_some_and(|id| update.entity.0 <= id) {
            return Err(ReplicateError::DuplicateEntity(update.entity.0));
        }
        prev = Some(update.entity.0);
        if update.mask == 0 || update.mask & !MASK_ALL != 0 {
            return Err(ReplicateError::BadMask(update.mask));
        }
        if update.mask & MASK_TRANSFORM == 0 && update.transform != Transform::default()
            || update.mask & MASK_VITALS == 0 && update.vitals != (Vitals { hp: 0, stamina: 0 })
        {
            return Err(ReplicateError::BadMask(update.mask));
        }
    }
    prev = None;
    for entity in &message.despawns {
        if prev.is_some_and(|id| entity.0 <= id) {
            return Err(ReplicateError::DuplicateEntity(entity.0));
        }
        prev = Some(entity.0);
    }
    Ok(())
}

fn encode_transform(out: &mut Vec<u8>, transform: &Transform) {
    out.extend_from_slice(&transform.x_mm.to_le_bytes());
    out.extend_from_slice(&transform.y_mm.to_le_bytes());
    out.extend_from_slice(&transform.yaw_ddeg.to_le_bytes());
}

fn decode_transform(bytes: &[u8]) -> Result<(Transform, usize), ReplicateError> {
    if bytes.len() < 10 {
        return Err(ReplicateError::Truncated);
    }
    Ok((
        Transform {
            x_mm: i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            y_mm: i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            yaw_ddeg: i16::from_le_bytes([bytes[8], bytes[9]]),
        },
        10,
    ))
}

/// Encode one message: magic, version, tick, baseline, flags, then spawns,
/// updates and despawns with counts. No padding, no trailing bytes.
pub fn encode_message(message: &ReplicationMessage) -> Result<Vec<u8>, ReplicateError> {
    validate_message(message)?;
    if wire_len(message) > MAX_MESSAGE_BYTES {
        return Err(ReplicateError::TooLarge);
    }
    let mut out = Vec::with_capacity(wire_len(message));
    out.extend_from_slice(&REPLICATE_MAGIC.to_le_bytes());
    out.push(REPLICATE_VERSION);
    out.extend_from_slice(&message.tick.to_le_bytes());
    out.extend_from_slice(&message.baseline.to_le_bytes());
    out.push(if message.resync { RESYNC_FLAG } else { 0 });
    out.push(message.spawns.len() as u8);
    for spawn in &message.spawns {
        out.extend_from_slice(&spawn.entity.0.to_le_bytes());
        encode_transform(&mut out, &spawn.transform);
        out.extend_from_slice(&spawn.vitals.hp.to_le_bytes());
        out.extend_from_slice(&spawn.vitals.stamina.to_le_bytes());
    }
    out.push(message.updates.len() as u8);
    for update in &message.updates {
        out.extend_from_slice(&update.entity.0.to_le_bytes());
        out.push(update.mask);
        if update.mask & MASK_TRANSFORM != 0 {
            encode_transform(&mut out, &update.transform);
        }
        if update.mask & MASK_VITALS != 0 {
            out.extend_from_slice(&update.vitals.hp.to_le_bytes());
            out.extend_from_slice(&update.vitals.stamina.to_le_bytes());
        }
    }
    out.push(message.despawns.len() as u8);
    for entity in &message.despawns {
        out.extend_from_slice(&entity.0.to_le_bytes());
    }
    debug_assert_eq!(out.len(), wire_len(message));
    Ok(out)
}

/// Decode and fully validate one message. Future versions refuse before any
/// other parsing; length must match exactly.
pub fn decode_message(bytes: &[u8]) -> Result<ReplicationMessage, ReplicateError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(ReplicateError::TooLarge);
    }
    if bytes.len() < HEADER_LEN {
        return Err(ReplicateError::Truncated);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != REPLICATE_MAGIC {
        return Err(ReplicateError::BadMagic);
    }
    if bytes[2] != REPLICATE_VERSION {
        return Err(ReplicateError::VersionMismatch { got: bytes[2] });
    }
    let tick = u32::from_le_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]);
    let baseline = u32::from_le_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]);
    let flags = bytes[11];
    if flags & !RESYNC_FLAG != 0 {
        return Err(ReplicateError::Truncated);
    }
    let resync = flags & RESYNC_FLAG != 0;
    let mut cursor = HEADER_LEN;
    let take = |cursor: &mut usize, n: usize| -> Result<&[u8], ReplicateError> {
        if bytes.len() < *cursor + n {
            return Err(ReplicateError::Truncated);
        }
        let slice = &bytes[*cursor..*cursor + n];
        *cursor += n;
        Ok(slice)
    };
    let nspawn = usize::from(bytes[cursor - 1]);
    if nspawn > MAX_ENTITIES {
        return Err(ReplicateError::TooManyEntities(nspawn));
    }
    let mut spawns = Vec::with_capacity(nspawn);
    for _ in 0..nspawn {
        let id = u32::from_le_bytes(take(&mut cursor, 4)?.try_into().expect("sized take"));
        let (transform, used) = decode_transform(take(&mut cursor, 10)?)?;
        let _ = used;
        let vitals_bytes = take(&mut cursor, 4)?;
        spawns.push(FullEntry {
            entity: EntityId(id),
            transform,
            vitals: Vitals {
                hp: u16::from_le_bytes([vitals_bytes[0], vitals_bytes[1]]),
                stamina: u16::from_le_bytes([vitals_bytes[2], vitals_bytes[3]]),
            },
        });
    }
    let nupdate = usize::from(take(&mut cursor, 1)?[0]);
    if nupdate > MAX_ENTITIES {
        return Err(ReplicateError::TooManyEntities(nupdate));
    }
    let mut updates = Vec::with_capacity(nupdate);
    for _ in 0..nupdate {
        let id = u32::from_le_bytes(take(&mut cursor, 4)?.try_into().expect("sized take"));
        let mask = take(&mut cursor, 1)?[0];
        if mask == 0 || mask & !MASK_ALL != 0 {
            return Err(ReplicateError::BadMask(mask));
        }
        let transform = if mask & MASK_TRANSFORM != 0 {
            decode_transform(take(&mut cursor, 10)?)?.0
        } else {
            Transform::default()
        };
        let vitals = if mask & MASK_VITALS != 0 {
            let raw = take(&mut cursor, 4)?;
            Vitals {
                hp: u16::from_le_bytes([raw[0], raw[1]]),
                stamina: u16::from_le_bytes([raw[2], raw[3]]),
            }
        } else {
            Vitals { hp: 0, stamina: 0 }
        };
        updates.push(EntityUpdate {
            entity: EntityId(id),
            mask,
            transform,
            vitals,
        });
    }
    let ndespawn = usize::from(take(&mut cursor, 1)?[0]);
    if ndespawn > MAX_ENTITIES {
        return Err(ReplicateError::TooManyEntities(ndespawn));
    }
    let mut despawns = Vec::with_capacity(ndespawn);
    for _ in 0..ndespawn {
        let id = u32::from_le_bytes(take(&mut cursor, 4)?.try_into().expect("sized take"));
        despawns.push(EntityId(id));
    }
    if cursor != bytes.len() {
        return Err(ReplicateError::Truncated);
    }
    let message = ReplicationMessage {
        tick,
        baseline,
        resync,
        spawns,
        updates,
        despawns,
    };
    validate_message(&message)?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::Server;

    const EA: EntityId = EntityId(0);
    const EB: EntityId = EntityId(1);

    fn transform(x_mm: i32, y_mm: i32) -> Transform {
        Transform {
            x_mm,
            y_mm,
            yaw_ddeg: 0,
        }
    }

    fn seeded_world() -> ReplicationServer {
        let mut server = ReplicationServer::new();
        server
            .spawn(EA, transform(1000, 0), Vitals::FULL)
            .expect("spawn A");
        server
            .spawn(EB, transform(-3000, 4000), Vitals::FULL)
            .expect("spawn B");
        server.advance();
        server
    }

    /// Drive one authoritative session: scripted inputs tick the server sim,
    /// positions publish into replication, and each client gets every message
    /// (dropped when `loss_every` hits) over the real wire codec. Returns the
    /// server hash, both client hashes, the message log and resync counts.
    fn run_session(seed: u64, ticks: u32, loss_every: u32) -> (u64, u64, u64, Vec<Vec<u8>>, u64) {
        let mut session = Server::with_seed(seed);
        let players: Vec<u32> = (0..2).map(|_| session.join().expect("join")).collect();
        let links: Vec<(u32, EntityId)> = vec![(players[0], EA), (players[1], EB)];
        let mut server = ReplicationServer::new();
        let mut first = ReplicationClient::new();
        let mut second = ReplicationClient::new();
        let mut log: Vec<Vec<u8>> = Vec::new();
        let mut step: u32 = 0;
        for _ in 0..ticks {
            for (index, player) in players.iter().enumerate() {
                let bits = (step * 7 + index as u32 * 11 + 1) & 0xF;
                assert!(session.submit(*player, bits));
            }
            session.tick();
            publish_server_positions(&mut server, &session, &links);
            server.advance();
            for client_id in [1u32, 2u32] {
                let message = server.message_for(client_id).expect("message fits");
                let bytes = encode_message(&message).expect("encodes");
                log.push(bytes.clone());
                step += 1;
                let drop_it = loss_every != 0 && step.is_multiple_of(loss_every);
                if drop_it {
                    continue;
                }
                let decoded = decode_message(&bytes).expect("decodes");
                let client = if client_id == 1 {
                    &mut first
                } else {
                    &mut second
                };
                client.apply(&decoded).expect("applies");
                server
                    .on_ack(client_id, decoded.tick)
                    .expect("ack inside tip");
            }
        }
        // Final resync sweep: every RESYNC_EVERY ticks the server already
        // emits full state, so one last pair of messages converges loss.
        for client_id in [1u32, 2u32] {
            let message = server.message_for(client_id).expect("final message");
            let decoded =
                decode_message(&encode_message(&message).expect("encodes")).expect("decodes");
            let client = if client_id == 1 {
                &mut first
            } else {
                &mut second
            };
            let _ = client.apply(&decoded);
        }
        (
            server.world_hash(),
            first.world_hash(),
            second.world_hash(),
            log,
            first.resyncs() + second.resyncs(),
        )
    }

    #[test]
    fn deltas_replicate_and_agree_x2() {
        let (server, first, second, log, _) = run_session(7, 48, 0);
        assert_eq!(first, server, "client one must mirror server truth");
        assert_eq!(second, server, "client two must mirror server truth");
        // Same complete session twice gives the identical agreement.
        let (server2, first2, second2, log2, _) = run_session(7, 48, 0);
        assert_eq!((server, first, second), (server2, first2, second2));
        assert_eq!(log, log2, "wire bytes must replay exactly");
        assert!(!log.is_empty());
        // A different seed must diverge: guards a vacuous equality.
        let (other, _, _, _, _) = run_session(8, 48, 0);
        assert_ne!(server, other);
    }

    #[test]
    fn replay_log_exact() {
        let (server, first, _, log, _) = run_session(19, 32, 0);
        // Fresh clients replaying the recorded wire log land on the same hash.
        for _ in 0..2 {
            let mut replay = ReplicationClient::new();
            for bytes in &log {
                let message = decode_message(bytes).expect("log decodes");
                let _ = replay.apply(&message).expect("log applies");
            }
            assert_eq!(replay.world_hash(), server);
            assert_eq!(replay.world_hash(), first);
            assert_eq!(replay.last_tick(), Some(32));
        }
    }

    #[test]
    fn unchanged_components_omitted() {
        let mut server = seeded_world();
        assert!(server.on_ack(1, 1).expect("ack"));
        // Touch only B's vitals: the delta names one entity, one component.
        server
            .set_vitals(
                EB,
                Vitals {
                    hp: 760,
                    stamina: 35,
                },
            )
            .expect("wound B");
        server.advance();
        let message = server.message_for(1).expect("delta");
        assert!(!message.resync);
        assert!(message.spawns.is_empty());
        assert_eq!(message.updates.len(), 1);
        assert_eq!(message.updates[0].entity, EB);
        assert_eq!(message.updates[0].mask, MASK_VITALS);
        assert!(wire_len(&message) < HEADER_LEN + 2 + 2 * SPAWN_ENTRY_LEN);
        let mut client = ReplicationClient::new();
        // A delta alone cannot bootstrap a client that missed the spawn:
        // loud refusal, mirror unchanged.
        assert_eq!(
            client.apply(&message),
            Err(ReplicateError::UnknownEntity(1))
        );
        assert!(client.is_empty());
        let full = server.message_for(9).expect("late joiner gets full");
        assert!(full.resync);
        let report = client.apply(&full).expect("join");
        assert_eq!(report.applied, 2);
        assert!(report.resync);
        assert_eq!(client.world_hash(), server.world_hash());
        // The join converged the mirror; ack it and step a quiet tick: the
        // next message is an empty non-resync delta, not a resend.
        assert!(server.on_ack(1, full.tick).expect("ack join"));
        server.advance();
        let quiet = server.message_for(1).expect("quiet");
        assert!(!quiet.resync);
        assert!(quiet.spawns.is_empty() && quiet.updates.is_empty() && quiet.despawns.is_empty());
    }

    #[test]
    fn loss_converges_via_resync() {
        let (server, first, second, _, resyncs) = run_session(7, 64, 5);
        assert!(resyncs > 0, "loss must force at least one full resync");
        assert_eq!(first, server, "client one converges after loss");
        assert_eq!(second, server, "client two converges after loss");
    }

    #[test]
    fn session_bridge_tracks_authority() {
        let mut session = Server::with_seed(3);
        let players: Vec<u32> = (0..2).map(|_| session.join().expect("join")).collect();
        let links: Vec<(u32, EntityId)> = vec![(players[0], EA), (players[1], EB)];
        let mut server = ReplicationServer::new();
        let mut client = ReplicationClient::new();
        for step in 0..8u32 {
            for (index, player) in players.iter().enumerate() {
                assert!(session.submit(*player, (step + index as u32) & 0xF));
            }
            session.tick();
            publish_server_positions(&mut server, &session, &links);
            server.advance();
            let message = server.message_for(1).expect("message");
            let decoded =
                decode_message(&encode_message(&message).expect("encodes")).expect("decodes");
            client.apply(&decoded).expect("applies");
            server.on_ack(1, decoded.tick).expect("ack");
        }
        assert_eq!(client.world_hash(), server.world_hash());
        for (player, entity) in &links {
            let (x_cm, y_cm) = session.pos(*player).expect("player placed");
            let mirrored = client.entry(*entity).expect("entity mirrored");
            assert_eq!(mirrored.transform.x_mm, x_cm.saturating_mul(10));
            assert_eq!(mirrored.transform.y_mm, y_cm.saturating_mul(10));
        }
        // A leaver's entity despawns on the next publish and the tombstone
        // replicates: the mirror drops the entity and still agrees.
        assert!(session.leave(players[1]));
        publish_server_positions(&mut server, &session, &links);
        server.advance();
        let message = server.message_for(1).expect("tombstone");
        assert_eq!(message.despawns, vec![EB]);
        client.apply(&message).expect("despawn applies");
        assert_eq!(client.entry(EB), None);
        assert_eq!(client.world_hash(), server.world_hash());
    }

    #[test]
    fn disagree_detected_and_loud() {
        let authority = seeded_world();
        let mut client = ReplicationClient::new();
        let full = authority.message_for(1).expect("full");
        client.apply(&full).expect("join");
        let server = authority.world_hash();
        let first = client.world_hash();
        assert_eq!(first, server);
        assert!(authority.verify_client(first).is_ok());
        let tampered = first.wrapping_add(1);
        let err = authority.verify_client(tampered).expect_err("must refuse");
        assert_eq!(
            err,
            ReplicateError::Desync {
                tick: authority.tick(),
                server,
                client: tampered,
            }
        );
        assert_eq!(
            err.to_string(),
            format!(
                "replication desync at tick {}: server {server:#016x} != client {tampered:#016x}",
                authority.tick()
            )
        );
    }

    #[test]
    fn stale_duplicate_changes_nothing() {
        let mut server = seeded_world();
        let mut client = ReplicationClient::new();
        let full = server.message_for(1).expect("full");
        let report = client.apply(&full).expect("join");
        assert!(!report.duplicate);
        assert!(server.on_ack(1, full.tick).expect("ack"));
        let before = client.world_hash();
        // Duplicate delivery of the same tick: acknowledged, changes nothing.
        let again = client.apply(&full).expect("duplicate");
        assert!(again.duplicate);
        assert_eq!(again.applied, 0);
        assert_eq!(client.world_hash(), before);
        assert!(!server.on_ack(1, full.tick).expect("dup ack"));
        // Direct stale apply against a moved tip refuses loudly instead.
        server.set_transform(EA, transform(2000, 0)).expect("move");
        server.advance();
        let newer = server.message_for(1).expect("newer");
        client.apply(&newer).expect("advance");
        let stale = client.apply(&full).expect("stale answers duplicate");
        assert!(stale.duplicate);
        assert_eq!(client.last_tick(), Some(newer.tick));
    }

    #[test]
    fn corrupt_wire_and_shape_refuse() {
        let server = seeded_world();
        let message = server.message_for(1).expect("full");
        let mut bytes = encode_message(&message).expect("encodes");
        // Future version refuses before any other parsing.
        bytes[2] = REPLICATE_VERSION + 1;
        assert_eq!(
            decode_message(&bytes),
            Err(ReplicateError::VersionMismatch {
                got: REPLICATE_VERSION + 1
            })
        );
        bytes[2] = REPLICATE_VERSION;
        // Bad magic refuses.
        bytes[0] ^= 0xFF;
        assert_eq!(decode_message(&bytes), Err(ReplicateError::BadMagic));
        bytes[0] ^= 0xFF;
        // Truncation and trailing bytes refuse.
        assert_eq!(
            decode_message(&bytes[..bytes.len() - 1]),
            Err(ReplicateError::Truncated)
        );
        let mut padded = bytes.clone();
        padded.push(0);
        assert_eq!(decode_message(&padded), Err(ReplicateError::Truncated));
        // Unknown-flag bits refuse.
        let mut flagged = bytes.clone();
        flagged[11] |= 0x02;
        assert_eq!(decode_message(&flagged), Err(ReplicateError::Truncated));
        // An update for an entity the mirror never spawned refuses and the
        // mirror is unchanged.
        let mut client = ReplicationClient::new();
        client.apply(&message).expect("join");
        let mut evil = message.clone();
        evil.tick += 1;
        evil.resync = false;
        evil.spawns.clear();
        evil.updates.push(EntityUpdate {
            entity: EntityId(777),
            mask: MASK_VITALS,
            transform: Transform::default(),
            vitals: Vitals::FULL,
        });
        let before = client.world_hash();
        assert_eq!(client.apply(&evil), Err(ReplicateError::UnknownEntity(777)));
        assert_eq!(client.world_hash(), before);
        // A zero mask is not an update: the encoder refuses it outright ...
        let mut bad_mask = message.clone();
        bad_mask.tick += 2;
        bad_mask.resync = false;
        bad_mask.spawns.clear();
        bad_mask.updates.push(EntityUpdate {
            entity: EA,
            mask: 0,
            transform: Transform::default(),
            vitals: Vitals { hp: 0, stamina: 0 },
        });
        assert_eq!(encode_message(&bad_mask), Err(ReplicateError::BadMask(0)));
        // ... and the wire decoder refuses a flipped mask byte the same way.
        let mut wounded = seeded_world();
        assert!(wounded.on_ack(1, 1).expect("ack"));
        wounded
            .set_vitals(EB, Vitals { hp: 1, stamina: 1 })
            .expect("wound");
        wounded.advance();
        let delta = wounded.message_for(1).expect("delta");
        assert_eq!(delta.updates.len(), 1);
        let mut raw = encode_message(&delta).expect("delta encodes");
        let mask_at = HEADER_LEN + delta.spawns.len() * SPAWN_ENTRY_LEN + 1 + 4;
        assert_eq!(raw[mask_at], MASK_VITALS);
        raw[mask_at] = 0;
        assert_eq!(decode_message(&raw), Err(ReplicateError::BadMask(0)));
        raw[mask_at] = MASK_ALL + 1;
        assert_eq!(
            decode_message(&raw),
            Err(ReplicateError::BadMask(MASK_ALL + 1))
        );
    }

    #[test]
    fn server_guards_and_ack_rules() {
        let mut server = ReplicationServer::new();
        assert!(server.is_empty());
        server
            .spawn(EA, transform(0, 0), Vitals::FULL)
            .expect("spawn");
        assert_eq!(
            server.spawn(EA, transform(0, 0), Vitals::FULL),
            Err(ReplicateError::DuplicateEntity(0))
        );
        assert_eq!(
            server.set_transform(EntityId(63), transform(1, 1)),
            Err(ReplicateError::UnknownEntity(63))
        );
        assert_eq!(
            server.set_vitals(EntityId(63), Vitals::FULL),
            Err(ReplicateError::UnknownEntity(63))
        );
        assert_eq!(
            server.despawn(EntityId(63)),
            Err(ReplicateError::UnknownEntity(63))
        );
        // Capacity is bounded at MAX_ENTITIES.
        for id in 1..MAX_ENTITIES as u32 {
            server
                .spawn(EntityId(id), transform(id as i32, 0), Vitals::FULL)
                .expect("fill");
        }
        assert_eq!(server.len(), MAX_ENTITIES);
        assert_eq!(
            server.spawn(EntityId(1000), transform(0, 0), Vitals::FULL),
            Err(ReplicateError::TooManyEntities(MAX_ENTITIES + 1))
        );
        // Acks past the tip refuse; stale acks answer false.
        assert_eq!(
            server.on_ack(1, server.tick() + 1),
            Err(ReplicateError::FutureTick {
                tick: server.tick() + 1,
                current: server.tick()
            })
        );
        assert!(server.on_ack(1, server.tick()).expect("ack"));
        assert!(!server.on_ack(1, server.tick()).expect("dup ack"));
        assert_eq!(server.acked_tick(1), Some(server.tick()));
        assert!(server.remove_client(1));
        assert!(!server.remove_client(1));
        assert_eq!(server.acked_tick(1), None);
        // Evicted baselines resync full instead of diffing against air.
        for _ in 0..MAX_HISTORY as u32 + 2 {
            server.advance();
        }
        assert!(server.on_ack(7, 0).expect("ancient ack"));
        let message = server.message_for(7).expect("evicted baseline");
        assert!(message.resync, "evicted history must resync full");
        assert_eq!(message.spawns.len(), MAX_ENTITIES);
    }

    #[test]
    fn drop_rejoin_blind_30_converges_full() {
        // ON30 leg: 600-tick authority-vs-mirror bout with seeded loss plus
        // a scripted 30-tick blind (ticks 300..330, client hears nothing).
        // Metric: 0 divergent checkpoints post-rejoin, 0 messages over
        // MAX_MESSAGE_BYTES. Lightyear idea: diffs ride the last-acked tick;
        // unknown or older baselines resync full, never a partial diff.
        let mut server = seeded_world();
        let mut mirror = ReplicationClient::new();
        let mut load = ReplicationClient::new();
        let join = server.message_for(7).expect("join fits");
        assert!(join.resync, "late joiner gets full");
        mirror.apply(&join).expect("join applies");
        assert!(server.on_ack(7, join.tick).expect("ack join"));
        let load_join = server.message_for(8).expect("load join fits");
        assert!(load_join.resync, "load joiner gets full");
        load.apply(&load_join).expect("load join applies");
        assert!(server.on_ack(8, load_join.tick).expect("ack load join"));
        let mut oversize = 0usize;
        let mut rejoin_was_resync = false;
        let mut rejoin_at: Option<u32> = None;
        let mut rejoin_hash: Option<u64> = None;
        for _ in 0..600u32 {
            let tick = server.tick().wrapping_add(1);
            server
                .set_transform(EA, transform(1000 + (tick as i32).wrapping_mul(3), 0))
                .expect("move A");
            server
                .set_transform(EB, transform(-3000 - (tick as i32).wrapping_mul(5), 4000))
                .expect("move B");
            server.advance();
            let now = server.tick();
            let message = server.message_for(7).expect("message fits");
            if wire_len(&message) > MAX_MESSAGE_BYTES {
                oversize += 1;
            }
            let bytes = encode_message(&message).expect("encodes within budget");
            assert!(bytes.len() <= MAX_MESSAGE_BYTES, "wire stays in budget");
            // Load peer stays connected through the blind: same wire budget,
            // same seeded loss, no blind. The blinded rejoin must converge
            // under this load, not just in isolation.
            let load_message = server.message_for(8).expect("load message fits");
            if wire_len(&load_message) > MAX_MESSAGE_BYTES {
                oversize += 1;
            }
            let load_bytes = encode_message(&load_message).expect("load encodes within budget");
            assert!(
                load_bytes.len() <= MAX_MESSAGE_BYTES,
                "load wire stays in budget"
            );
            let blind = (300..330u32).contains(&now);
            // Seeded loss outside the blind: every 11th tick drops.
            let lost = now % 11 == 0;
            if !lost {
                let load_decoded = decode_message(&load_bytes).expect("load decodes");
                load.apply(&load_decoded).expect("load applies");
                server
                    .on_ack(8, load_decoded.tick)
                    .expect("ack load inside tip");
            }
            if blind {
                continue;
            }
            if lost {
                continue;
            }
            let decoded = decode_message(&bytes).expect("decodes");
            mirror.apply(&decoded).expect("applies");
            server.on_ack(7, decoded.tick).expect("ack inside tip");
            // First message the mirror actually receives after the blind is
            // the rejoin: tick 330 itself is seeded loss (330 % 11 == 0), so
            // an exact-330 check can never observe it. Record the first
            // applied tick >= 330 instead; stale (>= RESYNC_EVERY) forces full.
            if now >= 330 && rejoin_at.is_none() {
                rejoin_at = Some(now);
                rejoin_was_resync = decoded.resync;
                rejoin_hash = Some(mirror.world_hash());
                assert_eq!(
                    mirror.world_hash(),
                    server.world_hash(),
                    "rejoin converges at first received tick under load"
                );
                assert_eq!(
                    load.world_hash(),
                    server.world_hash(),
                    "loaded peer agrees at blinded rejoin tick"
                );
            }
        }
        assert_eq!(oversize, 0, "no message may exceed the budget");
        assert_eq!(
            rejoin_at,
            Some(331),
            "rejoin is first received tick after blind (330 is seeded loss)"
        );
        assert!(
            rejoin_was_resync,
            "30-tick stale baseline must resync full, never a partial diff"
        );
        assert!(
            rejoin_hash.is_some(),
            "rejoin hash observed at first received tick under load"
        );
        // Final resync sweep converges any loss after the rejoin.
        for _ in 0..2 {
            let message = server.message_for(7).expect("final fits");
            let decoded =
                decode_message(&encode_message(&message).expect("encodes")).expect("decodes");
            let _ = mirror.apply(&decoded);
            let _ = server.on_ack(7, decoded.tick);
        }
        assert_eq!(
            mirror.world_hash(),
            server.world_hash(),
            "mirror must hold full checkpoint equality after rejoin"
        );
        assert!(server.verify_client(mirror.world_hash()).is_ok());
    }
}
