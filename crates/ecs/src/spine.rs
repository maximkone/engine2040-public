//! spine: the five actor components as SoA columns.
//!
//! Owner: engine-ecs. One slot per actor in index order (slot = the match
//! slot id), so iteration, canonical bytes and digests never depend on hash
//! order; zero dependencies beyond `engine-core`. Consumers: the arena spine
//! step writes the columns every tick; the parity harness, net rollback,
//! the editor and the door read them through [`Spine::snapshot`].
//!
//! Numeric scope: [`Transform`] pose and the [`AnimState`] gait phase and
//! facing hold the frozen arena path's `f32` values bit for bit (canonical
//! bytes are `to_bits` LE, the same rule the arena checkpoint uses), so the
//! parity harness compares byte-equal checkpoints against the frozen
//! golden. Every other field is an integer or [`Fixed`]. The pose and gait
//! move to `Fixed` only together with the golden's migration receipt; until
//! then the spine may not add a float field.
//!
//! Budget: columns reserve once at build ([`Spine::with_capacity`]); a step
//! that writes every column through [`Spine::for_each_mut`] and a hash
//! through [`CanonicalBytes::canonical_hash`] allocate nothing (pinned by
//! the counting-allocator tests below).

use std::hash::Hasher;

use engine_core::fixed::Fixed;

use crate::snapshot::{hash_bytes, CanonicalBytes, CanonicalHasher};

/// Spine snapshot format version. Bump only with a hash-law receipt.
pub const SPINE_VERSION: u16 = 1;

/// Most actor slots one spine holds (the input contract's participant cap).
///
/// 2026-10-21 (lead): the cap was 16 while the participant cap moved to 32
/// (owner decision GE12, 2026-10-20), so a 32-fighter match could not build
/// a spine at all: `Spine::with_capacity` clamped the reservation to 16 and
/// the 17th [`Spine::spawn`] returned [`SpineError::TooManySlots`], so the
/// arena's spine step answered `None` for a fight the legacy path could seat.
/// The doc comment already called this the participant cap, so the constant
/// now matches it.
pub const MAX_SPINE_SLOTS: usize = 32;

/// Slot index as the `u32` word canonical bytes carry (slots never exceed
/// [`MAX_SPINE_SLOTS`], so the fallback is unreachable).
fn slot_word(slot: usize) -> u32 {
    u32::try_from(slot).unwrap_or(u32::MAX)
}

/// Pose column: feet position `[x, y, z]` in meters plus facing yaw in
/// radians. `f32` by the parity rule in the module doc.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transform {
    /// Feet position in meters.
    pub pos: [f32; 3],
    /// Facing yaw in radians.
    pub yaw: f32,
}

/// Body column: the planar root delta applied on the last tick (meters per
/// tick, `[x, z]`) plus the capsule radius and height in meters, all
/// [`Fixed`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Body {
    /// Planar root delta of the last tick, meters per tick.
    pub vel: [Fixed; 2],
    /// Capsule radius in meters.
    pub radius: Fixed,
    /// Capsule height in meters.
    pub height: Fixed,
}

/// Vitals column: health in permille, stamina points, poise points and
/// the combat state bits ([`Vitals::ATTACKING`] and friends).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vitals {
    /// Health in permille, `0..=1000`.
    pub hp_permille: u16,
    /// Stamina points.
    pub stamina: u16,
    /// Poise points (unused by the frozen rules: always 0 there).
    pub poise: u16,
    /// Combat state bits.
    pub state: u8,
}

impl Vitals {
    /// An attack is in flight.
    pub const ATTACKING: u8 = 1;
    /// Block is held.
    pub const BLOCKING: u8 = 2;
    /// A dodge is in flight.
    pub const DODGING: u8 = 4;
    /// Knocked down.
    pub const DOWN: u8 = 8;
    /// Out of the fight (health zero).
    pub const OUT: u8 = 16;
}

/// One actor intent: quantized wish, absolute facing in centidegrees and
/// button bits, the input contract's shape without the tick/id envelope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Intent {
    /// Wish x in `-100..=100` percent of top speed.
    pub mx: i8,
    /// Wish z in `-100..=100` percent of top speed.
    pub mz: i8,
    /// Absolute facing in centidegrees, `-18000..=18000`.
    pub facing: i16,
    /// Button bits (attack 1, block 2, dodge 4).
    pub buttons: u8,
}

/// AI column: the intent that drove the last tick, the tick it was decided
/// on and where it came from ([`AiState::FROM_TAPE`] or
/// [`AiState::FROM_BRAIN`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiState {
    /// Intent applied on the last tick.
    pub intent: Intent,
    /// Tick the intent was decided on.
    pub clock: u32,
    /// Intent source tag.
    pub source: u8,
}

impl AiState {
    /// No intent decided yet.
    pub const FROM_NONE: u8 = 0;
    /// The tape supplied the intent (a human slot).
    pub const FROM_TAPE: u8 = 1;
    /// A brain filled the slot.
    pub const FROM_BRAIN: u8 = 2;
}

/// Anim column: locomotion state discriminant, gait phase, gait facing,
/// airborne and interrupt flags plus the presentation pose reference.
/// Phase and facing are `f32` by the parity rule in the module doc.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimState {
    /// Locomotion state discriminant (the arena's `LocoState` byte).
    pub loco: u8,
    /// Gait phase.
    pub phase: f32,
    /// Gait facing unit vector `[x, z]`.
    pub facing: [f32; 2],
    /// True while airborne.
    pub airborne: bool,
    /// True while a combat interrupt freezes the gait.
    pub interrupt: bool,
    /// Presentation pose reference (clip or pose-table row); 0 = none.
    pub pose_ref: u32,
}

/// Normalize signed zero so `0.0` and `-0.0` hash identically (the arena's
/// `canon_zero` rule).
fn canon_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Stream one `f32` as raw bits LE after zero canonicalization.
fn write_f32<H: Hasher>(state: &mut H, value: f32) {
    state.write(&canon_zero(value).to_bits().to_le_bytes());
}

impl CanonicalBytes for Transform {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        for axis in self.pos {
            out.extend_from_slice(&canon_zero(axis).to_bits().to_le_bytes());
        }
        out.extend_from_slice(&canon_zero(self.yaw).to_bits().to_le_bytes());
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        for axis in self.pos {
            write_f32(state, axis);
        }
        write_f32(state, self.yaw);
    }
}

impl CanonicalBytes for Body {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        for lane in self.vel {
            out.extend_from_slice(&lane.to_le_bytes());
        }
        out.extend_from_slice(&self.radius.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        for lane in self.vel {
            state.write(&lane.to_le_bytes());
        }
        state.write(&self.radius.to_le_bytes());
        state.write(&self.height.to_le_bytes());
    }
}

impl CanonicalBytes for Vitals {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.hp_permille.to_le_bytes());
        out.extend_from_slice(&self.stamina.to_le_bytes());
        out.extend_from_slice(&self.poise.to_le_bytes());
        out.push(self.state);
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.hp_permille.to_le_bytes());
        state.write(&self.stamina.to_le_bytes());
        state.write(&self.poise.to_le_bytes());
        state.write(&[self.state]);
    }
}

impl CanonicalBytes for AiState {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.intent.mx.to_le_bytes());
        out.extend_from_slice(&self.intent.mz.to_le_bytes());
        out.extend_from_slice(&self.intent.facing.to_le_bytes());
        out.push(self.intent.buttons);
        out.extend_from_slice(&self.clock.to_le_bytes());
        out.push(self.source);
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.intent.mx.to_le_bytes());
        state.write(&self.intent.mz.to_le_bytes());
        state.write(&self.intent.facing.to_le_bytes());
        state.write(&[self.intent.buttons]);
        state.write(&self.clock.to_le_bytes());
        state.write(&[self.source]);
    }
}

impl CanonicalBytes for AnimState {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        out.push(self.loco);
        out.extend_from_slice(&canon_zero(self.phase).to_bits().to_le_bytes());
        for axis in self.facing {
            out.extend_from_slice(&canon_zero(axis).to_bits().to_le_bytes());
        }
        out.push(u8::from(self.airborne));
        out.push(u8::from(self.interrupt));
        out.extend_from_slice(&self.pose_ref.to_le_bytes());
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        state.write(&[self.loco]);
        write_f32(state, self.phase);
        for axis in self.facing {
            write_f32(state, axis);
        }
        state.write(&[u8::from(self.airborne), u8::from(self.interrupt)]);
        state.write(&self.pose_ref.to_le_bytes());
    }
}

/// The five spine columns of one slot, borrowed mutably together by
/// [`Spine::for_each_mut`] and [`Spine::slot_mut`].
pub struct SlotMut<'a> {
    /// Pose column.
    pub transform: &'a mut Transform,
    /// Body column.
    pub body: &'a mut Body,
    /// Vitals column.
    pub vitals: &'a mut Vitals,
    /// AI column.
    pub ai: &'a mut AiState,
    /// Anim column.
    pub anim: &'a mut AnimState,
}

/// Which column a digest or a parity divergence names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    /// [`Transform`] column.
    Transform,
    /// [`Body`] column.
    Body,
    /// [`Vitals`] column.
    Vitals,
    /// [`AiState`] column.
    Ai,
    /// [`AnimState`] column.
    Anim,
}

impl Column {
    /// Every column in canonical order.
    pub const ORDER: [Column; 5] = [
        Column::Transform,
        Column::Body,
        Column::Vitals,
        Column::Ai,
        Column::Anim,
    ];

    /// Stable lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            Column::Transform => "transform",
            Column::Body => "body",
            Column::Vitals => "vitals",
            Column::Ai => "ai",
            Column::Anim => "anim",
        }
    }
}

/// Spine decode failure. Loud `Err`, never panics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpineError {
    /// Fewer bytes than the header or a field needs.
    TooShort,
    /// Unknown version (holds the found version).
    BadVersion(u16),
    /// Slot count above [`MAX_SPINE_SLOTS`] (holds the found count).
    TooManySlots(u32),
    /// Alive flag was not 0 or 1 (holds the found flag).
    BadFlag(u8),
    /// Bytes left over after a full decode (holds the count).
    Trailing(usize),
    /// A spawn past the reserved capacity.
    Full,
}

impl core::fmt::Display for SpineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "spine bytes too short"),
            Self::BadVersion(v) => write!(f, "spine bad version {v}"),
            Self::TooManySlots(n) => write!(f, "spine holds {n} slots, at most {MAX_SPINE_SLOTS}"),
            Self::BadFlag(b) => write!(f, "spine bad alive flag {b}"),
            Self::Trailing(n) => write!(f, "spine trailing {n} bytes"),
            Self::Full => write!(f, "spine is full: reserve more slots at build"),
        }
    }
}

impl std::error::Error for SpineError {}

/// Versioned bytes plus hash returned by [`Spine::snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpineSnapshot {
    /// Format version (always [`SPINE_VERSION`]).
    pub version: u16,
    /// Canonical versioned bytes (exact [`Spine::restore`] input).
    pub bytes: Vec<u8>,
    /// FNV-1a64 over [`SpineSnapshot::bytes`].
    pub hash: u64,
}

/// The spine store: five SoA columns plus an alive flag per slot, index
/// order only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spine {
    alive: Vec<bool>,
    transform: Vec<Transform>,
    body: Vec<Body>,
    vitals: Vec<Vitals>,
    ai: Vec<AiState>,
    anim: Vec<AnimState>,
}

impl Spine {
    /// Empty spine with room for `slots` actors (capped at
    /// [`MAX_SPINE_SLOTS`]); the one allocation point. Setup path only.
    pub fn with_capacity(slots: usize) -> Self {
        let slots = slots.min(MAX_SPINE_SLOTS);
        Self {
            alive: Vec::with_capacity(slots),
            transform: Vec::with_capacity(slots),
            body: Vec::with_capacity(slots),
            vitals: Vec::with_capacity(slots),
            ai: Vec::with_capacity(slots),
            anim: Vec::with_capacity(slots),
        }
    }

    /// Slots ever spawned (dead slots keep their index).
    pub fn len(&self) -> usize {
        self.alive.len()
    }

    /// True when no slot was ever spawned.
    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }

    /// Live slot count.
    pub fn live(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }

    /// Spawn one actor into the next slot; refuses past the reserved
    /// capacity (so the tick path never grows) and past
    /// [`MAX_SPINE_SLOTS`]. Returns the slot index.
    pub fn spawn(
        &mut self,
        transform: Transform,
        body: Body,
        vitals: Vitals,
    ) -> Result<usize, SpineError> {
        if self.alive.len() >= self.alive.capacity() || self.alive.len() >= MAX_SPINE_SLOTS {
            return Err(SpineError::Full);
        }
        let slot = self.alive.len();
        self.alive.push(true);
        self.transform.push(transform);
        self.body.push(body);
        self.vitals.push(vitals);
        self.ai.push(AiState::default());
        self.anim.push(AnimState::default());
        Ok(slot)
    }

    /// Mark a slot dead; its columns stop entering bytes and hashes. Stale
    /// or dead slots return false.
    pub fn kill(&mut self, slot: usize) -> bool {
        match self.alive.get_mut(slot) {
            Some(alive) if *alive => {
                *alive = false;
                true
            }
            _ => false,
        }
    }

    /// True when the slot exists and is live.
    pub fn is_alive(&self, slot: usize) -> bool {
        self.alive.get(slot).copied().unwrap_or(false)
    }

    /// Pose column, every slot in index order (dead slots included).
    pub fn transforms(&self) -> &[Transform] {
        &self.transform
    }

    /// Body column, every slot in index order.
    pub fn bodies(&self) -> &[Body] {
        &self.body
    }

    /// Vitals column, every slot in index order.
    pub fn vitals(&self) -> &[Vitals] {
        &self.vitals
    }

    /// AI column, every slot in index order.
    pub fn ai(&self) -> &[AiState] {
        &self.ai
    }

    /// Anim column, every slot in index order.
    pub fn anims(&self) -> &[AnimState] {
        &self.anim
    }

    /// Mutable view of one live slot's five columns; `None` for dead or
    /// unknown slots.
    pub fn slot_mut(&mut self, slot: usize) -> Option<SlotMut<'_>> {
        if !self.is_alive(slot) {
            return None;
        }
        Some(SlotMut {
            transform: &mut self.transform[slot],
            body: &mut self.body[slot],
            vitals: &mut self.vitals[slot],
            ai: &mut self.ai[slot],
            anim: &mut self.anim[slot],
        })
    }

    /// Visit every live slot mutably in index order (the step shape: one
    /// pass, no allocation).
    pub fn for_each_mut(&mut self, mut f: impl FnMut(usize, SlotMut<'_>)) {
        for slot in 0..self.alive.len() {
            if !self.alive[slot] {
                continue;
            }
            f(
                slot,
                SlotMut {
                    transform: &mut self.transform[slot],
                    body: &mut self.body[slot],
                    vitals: &mut self.vitals[slot],
                    ai: &mut self.ai[slot],
                    anim: &mut self.anim[slot],
                },
            );
        }
    }

    /// Spawn one RC4 prefab bundle over the spine columns. Setup path only;
    /// delegates to [`crate::views::SpawnBundle::spawn_into`]. Errors: the
    /// views bundle error (full store or unknown component). Determinism:
    /// slot order only.
    pub fn spawn_bundle(
        &mut self,
        bundle: &crate::views::SpawnBundle,
    ) -> Result<usize, crate::views::BundleError> {
        bundle.spawn_into(self)
    }

    /// Spawn every bundle in order, then count live slots on one optional
    /// pass. Setup path only; delegates to
    /// [`crate::views::spawn_prefab_batch`]. Returns the live-slot total.
    /// Errors: the first refused bundle. Determinism: slot order only.
    pub fn spawn_bundles(
        &mut self,
        bundles: &[crate::views::SpawnBundle],
    ) -> Result<usize, crate::views::BundleError> {
        crate::views::spawn_prefab_batch(self, bundles)
    }

    /// Visit every live slot through the optional view (absent prefab
    /// columns read as `None`). Tick path: borrows in place, no heap use.
    /// Delegates to [`crate::views::for_each_optional`].
    pub fn for_each_optional(&mut self, f: impl FnMut(usize, crate::views::OptionalSlot<'_>)) {
        crate::views::for_each_optional(self, f);
    }

    /// Per-column digests over live slots in index order (slot index plus
    /// the column bytes), so a divergence names its column. Streams, no
    /// allocation.
    pub fn column_digests(&self) -> [u64; 5] {
        let mut out = [0u64; 5];
        for (index, column) in Column::ORDER.iter().enumerate() {
            let mut state = CanonicalHasher::new();
            for slot in 0..self.alive.len() {
                if !self.alive[slot] {
                    continue;
                }
                state.write(&slot_word(slot).to_le_bytes());
                match column {
                    Column::Transform => self.transform[slot].append_hash(&mut state),
                    Column::Body => self.body[slot].append_hash(&mut state),
                    Column::Vitals => self.vitals[slot].append_hash(&mut state),
                    Column::Ai => self.ai[slot].append_hash(&mut state),
                    Column::Anim => self.anim[slot].append_hash(&mut state),
                }
            }
            out[index] = state.finish();
        }
        out
    }

    /// Capture the spine as versioned bytes plus hash (slot order). Setup
    /// and boundary path: allocates the byte vector.
    pub fn snapshot(&self) -> SpineSnapshot {
        let mut bytes = Vec::new();
        self.append_canonical(&mut bytes);
        let hash = hash_bytes(&bytes);
        SpineSnapshot {
            version: SPINE_VERSION,
            bytes,
            hash,
        }
    }

    /// Restore the exact spine [`Spine::snapshot`] captured.
    pub fn restore(bytes: &[u8]) -> Result<Self, SpineError> {
        let mut pos = 0usize;
        let version = read_u16(bytes, &mut pos)?;
        if version != SPINE_VERSION {
            return Err(SpineError::BadVersion(version));
        }
        let slot_count = read_u32(bytes, &mut pos)?;
        let slots = usize::try_from(slot_count)
            .ok()
            .filter(|&n| n <= MAX_SPINE_SLOTS)
            .ok_or(SpineError::TooManySlots(slot_count))?;
        let mut spine = Self::with_capacity(slots);
        for _ in 0..slots {
            let flag = read_u8(bytes, &mut pos)?;
            match flag {
                1 => {
                    let transform = Transform {
                        pos: [
                            read_f32(bytes, &mut pos)?,
                            read_f32(bytes, &mut pos)?,
                            read_f32(bytes, &mut pos)?,
                        ],
                        yaw: read_f32(bytes, &mut pos)?,
                    };
                    let body = Body {
                        vel: [read_fixed(bytes, &mut pos)?, read_fixed(bytes, &mut pos)?],
                        radius: read_fixed(bytes, &mut pos)?,
                        height: read_fixed(bytes, &mut pos)?,
                    };
                    let vitals = Vitals {
                        hp_permille: read_u16(bytes, &mut pos)?,
                        stamina: read_u16(bytes, &mut pos)?,
                        poise: read_u16(bytes, &mut pos)?,
                        state: read_u8(bytes, &mut pos)?,
                    };
                    let ai = AiState {
                        intent: Intent {
                            mx: i8::from_le_bytes([read_u8(bytes, &mut pos)?]),
                            mz: i8::from_le_bytes([read_u8(bytes, &mut pos)?]),
                            facing: i16::from_le_bytes(read_u16(bytes, &mut pos)?.to_le_bytes()),
                            buttons: read_u8(bytes, &mut pos)?,
                        },
                        clock: read_u32(bytes, &mut pos)?,
                        source: read_u8(bytes, &mut pos)?,
                    };
                    let anim = AnimState {
                        loco: read_u8(bytes, &mut pos)?,
                        phase: read_f32(bytes, &mut pos)?,
                        facing: [read_f32(bytes, &mut pos)?, read_f32(bytes, &mut pos)?],
                        airborne: read_u8(bytes, &mut pos)? != 0,
                        interrupt: read_u8(bytes, &mut pos)? != 0,
                        pose_ref: read_u32(bytes, &mut pos)?,
                    };
                    let slot = spine.spawn(transform, body, vitals)?;
                    spine.ai[slot] = ai;
                    spine.anim[slot] = anim;
                }
                0 => {
                    let slot =
                        spine.spawn(Transform::default(), Body::default(), Vitals::default())?;
                    spine.alive[slot] = false;
                }
                other => return Err(SpineError::BadFlag(other)),
            }
        }
        if pos != bytes.len() {
            return Err(SpineError::Trailing(bytes.len() - pos));
        }
        Ok(spine)
    }
}

impl CanonicalBytes for Spine {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&SPINE_VERSION.to_le_bytes());
        out.extend_from_slice(&slot_word(self.alive.len()).to_le_bytes());
        for slot in 0..self.alive.len() {
            if self.alive[slot] {
                out.push(1);
                self.transform[slot].append_canonical(out);
                self.body[slot].append_canonical(out);
                self.vitals[slot].append_canonical(out);
                self.ai[slot].append_canonical(out);
                self.anim[slot].append_canonical(out);
            } else {
                out.push(0);
            }
        }
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        state.write(&SPINE_VERSION.to_le_bytes());
        state.write(&slot_word(self.alive.len()).to_le_bytes());
        for slot in 0..self.alive.len() {
            if self.alive[slot] {
                state.write(&[1]);
                self.transform[slot].append_hash(state);
                self.body[slot].append_hash(state);
                self.vitals[slot].append_hash(state);
                self.ai[slot].append_hash(state);
                self.anim[slot].append_hash(state);
            } else {
                state.write(&[0]);
            }
        }
    }
}

/// Read one byte, bounds-checked.
fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, SpineError> {
    let b = bytes.get(*pos).ok_or(SpineError::TooShort)?;
    *pos += 1;
    Ok(*b)
}

/// Read `u16` LE, bounds-checked.
fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, SpineError> {
    let end = pos.checked_add(2).ok_or(SpineError::TooShort)?;
    let s = bytes.get(*pos..end).ok_or(SpineError::TooShort)?;
    *pos = end;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

/// Read `u32` LE, bounds-checked.
fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, SpineError> {
    let end = pos.checked_add(4).ok_or(SpineError::TooShort)?;
    let s = bytes.get(*pos..end).ok_or(SpineError::TooShort)?;
    *pos = end;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read `f32` from raw-bits LE, bounds-checked.
fn read_f32(bytes: &[u8], pos: &mut usize) -> Result<f32, SpineError> {
    Ok(f32::from_bits(read_u32(bytes, pos)?))
}

/// Read [`Fixed`] from raw `i64` LE, bounds-checked.
fn read_fixed(bytes: &[u8], pos: &mut usize) -> Result<Fixed, SpineError> {
    let end = pos.checked_add(8).ok_or(SpineError::TooShort)?;
    let s = bytes.get(*pos..end).ok_or(SpineError::TooShort)?;
    *pos = end;
    let mut raw = [0u8; 8];
    raw.copy_from_slice(s);
    Ok(Fixed::from_bits(i64::from_le_bytes(raw)))
}

/// Float quarantine scanner (ARC14, reused by ARC17): finds float tokens in
/// production source text so a test can fail the gate on a new float.
#[cfg(test)]
pub(crate) mod quarantine {
    /// One float token and the top-level item that holds it.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct FloatSite {
        /// Zero-based line of the token in the scanned text.
        pub(crate) line: usize,
        /// Header line of the enclosing top-level item, trimmed.
        pub(crate) item: String,
    }

    /// True when `code` (comments already cut) holds an `f32`/`f64` type or
    /// suffix token, or a decimal float literal like `0.5`.
    fn has_float(code: &str) -> bool {
        let b = code.as_bytes();
        let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        for ty in ["f32", "f64"] {
            for (at, _) in code.match_indices(ty) {
                let ident_before = at
                    .checked_sub(1)
                    .is_some_and(|i| b[i].is_ascii_alphabetic() || b[i] == b'_');
                let word_after = b.get(at + 3).is_some_and(|c| word(*c));
                if !ident_before && !word_after {
                    return true;
                }
            }
        }
        for (at, _) in code.match_indices('.') {
            let digit_before = at.checked_sub(1).is_some_and(|i| b[i].is_ascii_digit());
            let digit_after = b.get(at + 1).is_some_and(u8::is_ascii_digit);
            if digit_before && digit_after {
                let mut start = at - 1;
                while start > 0 && b[start - 1].is_ascii_digit() {
                    start -= 1;
                }
                // `t.0.1` is tuple access, not a literal.
                let tuple_or_ident = start
                    .checked_sub(1)
                    .is_some_and(|i| word(b[i]) || b[i] == b'.');
                if !tuple_or_ident {
                    return true;
                }
            }
        }
        false
    }

    /// Every float token in the production part of `src` (everything above
    /// the first column-0 `#[cfg(test)]`), with its enclosing item. Comment
    /// text never counts.
    pub(crate) fn float_sites(src: &str) -> Vec<FloatSite> {
        let mut sites = Vec::new();
        let mut item = String::new();
        for (line, raw) in src.lines().enumerate() {
            if raw.starts_with("#[cfg(test)]") {
                break;
            }
            let opens_item = !raw.is_empty()
                && !raw.starts_with(char::is_whitespace)
                && !raw.starts_with("//")
                && !raw.starts_with("#[")
                && !raw.starts_with(['}', ')', ']']);
            if opens_item {
                item = raw.trim().to_string();
            }
            let code = raw.split("//").next().unwrap_or_default();
            if has_float(code) {
                sites.push(FloatSite {
                    line,
                    item: item.clone(),
                });
            }
        }
        sites
    }

    #[test]
    fn scanner_tells_floats_from_lookalikes() {
        assert!(has_float("pub yaw: f32,"));
        assert!(has_float("let x = 1f64;"));
        assert!(has_float("if value == 0.0 {"));
        assert!(has_float("let n = 12.5;"));
        assert!(!has_float("read_f32(bytes, &mut pos)?"));
        assert!(!has_float("let a = t.0.1;"));
        assert!(!has_float("for i in 0..5 {"));
        assert!(!has_float("let v: u32 = 1_000;"));
        assert!(!has_float("type F32x4 = u8;"));
    }
}

#[cfg(test)]
mod tests {
    use super::quarantine::float_sites;
    use super::*;
    use engine_core::alloc_probe::measure;

    /// The only items of the spine module that may hold a float: the frozen
    /// arena path's pose and gait, kept as raw `f32` bits for byte parity
    /// with the golden (see the module doc). Everything else is integer or
    /// `Fixed`. An entry that no longer holds a float must be removed.
    const PARITY_ITEMS: [&str; 5] = [
        "pub struct Transform {",
        "pub struct AnimState {",
        "fn canon_zero(value: f32) -> f32 {",
        "fn write_f32<H: Hasher>(state: &mut H, value: f32) {",
        "fn read_f32(bytes: &[u8], pos: &mut usize) -> Result<f32, SpineError> {",
    ];

    /// Items that hold a float in `src`, sorted and deduplicated.
    fn float_items(src: &str) -> Vec<String> {
        let mut items: Vec<String> = float_sites(src).into_iter().map(|s| s.item).collect();
        items.sort();
        items.dedup();
        items
    }

    #[test]
    fn spine_module_holds_floats_only_in_the_parity_items() {
        let mut want: Vec<String> = PARITY_ITEMS.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(
            float_items(include_str!("spine.rs")),
            want,
            "a float outside the parity items (or a stale allowlist entry)"
        );
    }

    #[test]
    fn spine_schedule_and_views_hold_no_float_at_all() {
        // The integer-ns step (`SpineCtx::dt_ns`) lives in the schedule file.
        assert!(float_sites(include_str!("spine_schedule.rs")).is_empty());
        assert!(float_sites(include_str!("views.rs")).is_empty());
    }

    #[test]
    fn a_seeded_float_fails_the_gate() {
        // The same checks the two tests above run, on sources with one float
        // seeded in: each must now report it.
        // A Windows checkout carries CRLF; the seeds below are LF text.
        let spine = include_str!("spine.rs").replace("\r\n", "\n");
        let seeded_field = spine.replace(
            "    /// Capsule height in meters.
    pub height: Fixed,",
            "    /// Capsule height in meters.
    pub height: Fixed,
    pub mass: f32,",
        );
        assert_ne!(seeded_field, spine, "seed must land");
        assert!(float_items(&seeded_field).contains(&"pub struct Body {".to_string()));
        let seeded_dt = format!(
            "{}
pub struct DtF32 {{
    pub dt: f32,
}}
",
            include_str!("spine_schedule.rs")
                .split("#[cfg(test)]")
                .next()
                .unwrap_or_default()
        );
        assert_eq!(
            float_sites(&seeded_dt).last().map(|s| s.item.as_str()),
            Some("pub struct DtF32 {")
        );
        let seeded_literal = "pub fn step() {
    let half = 0.5;
}
";
        assert_eq!(float_sites(seeded_literal).len(), 1);
    }

    fn body() -> Body {
        Body {
            vel: [Fixed::ZERO, Fixed::ZERO],
            radius: Fixed::from_bits(26_214),
            height: Fixed::from_bits(117_965),
        }
    }

    /// Four live slots plus one dead slot with every column written.
    fn seeded() -> Spine {
        let mut spine = Spine::with_capacity(5);
        for i in 0..5u32 {
            let f = i as f32;
            let slot = spine
                .spawn(
                    Transform {
                        pos: [f * 1.5, 0.0, -f],
                        yaw: f * 0.25,
                    },
                    body(),
                    Vitals {
                        hp_permille: 1000 - i as u16 * 10,
                        stamina: 100,
                        poise: 0,
                        state: 0,
                    },
                )
                .expect("capacity holds five");
            let cell = spine.slot_mut(slot).expect("live slot");
            cell.ai.intent = Intent {
                mx: 10 * i as i8,
                mz: -5,
                facing: 900 * i as i16,
                buttons: 1,
            };
            cell.ai.clock = i;
            cell.ai.source = AiState::FROM_BRAIN;
            cell.anim.loco = 1;
            cell.anim.phase = f * 0.1;
            cell.anim.facing = [0.0, 1.0];
            cell.anim.pose_ref = 7;
        }
        assert!(spine.kill(2));
        spine
    }

    #[test]
    fn roundtrip_bytes_and_hash_equal_twice() {
        let spine = seeded();
        let once = spine.snapshot();
        let twice = seeded().snapshot();
        assert_eq!(once, twice, "same build gives the same bytes and hash");
        assert_eq!(once.version, SPINE_VERSION);
        assert_eq!(once.hash, hash_bytes(&once.bytes));
        assert_eq!(
            once.hash,
            spine.canonical_hash(),
            "streamed hash equals byte hash"
        );
        let back = match Spine::restore(&once.bytes) {
            Ok(spine) => spine,
            Err(error) => panic!("roundtrip decodes: {error}"),
        };
        // Dead slots keep stale columns in memory but never in bytes, so
        // compare the live slots column by column.
        for slot in (0..spine.len()).filter(|s| spine.is_alive(*s)) {
            assert!(back.is_alive(slot));
            assert_eq!(back.transforms()[slot], spine.transforms()[slot]);
            assert_eq!(back.bodies()[slot], spine.bodies()[slot]);
            assert_eq!(back.vitals()[slot], spine.vitals()[slot]);
            assert_eq!(back.ai()[slot], spine.ai()[slot]);
            assert_eq!(back.anims()[slot], spine.anims()[slot]);
        }
        assert_eq!(back.column_digests(), spine.column_digests());
        assert_eq!(back.live(), 4);
        assert_eq!(back.len(), 5);
        assert!(!back.is_alive(2));
        assert_eq!(
            back.snapshot().bytes,
            once.bytes,
            "re-snapshot is byte-identical"
        );
        // Not vacuous: one pose bit moves bytes, hash and the transform digest only.
        let mut moved = seeded();
        moved.slot_mut(1).expect("live").transform.pos[0] += 0.125;
        assert_ne!(moved.snapshot().bytes, once.bytes);
        let (a, b) = (spine.column_digests(), moved.column_digests());
        assert_ne!(a[0], b[0], "transform digest must move");
        assert_eq!(&a[1..], &b[1..], "other columns must not move");
    }

    #[test]
    fn dead_slots_leave_bytes_and_digests() {
        let mut spine = seeded();
        let before = spine.snapshot();
        // Writing a dead slot is refused and moves nothing.
        assert!(spine.slot_mut(2).is_none());
        assert!(!spine.kill(2));
        assert!(!spine.kill(9));
        assert_eq!(spine.snapshot(), before);
        // Killing a live slot moves every column digest (the slot leaves).
        let digests = spine.column_digests();
        assert!(spine.kill(0));
        assert_ne!(spine.column_digests(), digests);
        assert_eq!(spine.live(), 3);
    }

    #[test]
    fn capacity_and_bad_bytes_refuse_loudly() {
        let mut full = Spine::with_capacity(1);
        full.spawn(Transform::default(), body(), Vitals::default())
            .expect("first spawn fits");
        assert_eq!(
            full.spawn(Transform::default(), body(), Vitals::default()),
            Err(SpineError::Full)
        );
        assert_eq!(Spine::restore(&[]), Err(SpineError::TooShort));
        let mut bad = seeded().snapshot().bytes;
        bad[0] = 0xff;
        bad[1] = 0xff;
        assert_eq!(Spine::restore(&bad), Err(SpineError::BadVersion(0xffff)));
        let mut flag = seeded().snapshot().bytes;
        flag[6] = 2;
        assert_eq!(Spine::restore(&flag), Err(SpineError::BadFlag(2)));
        let mut trailing = Spine::with_capacity(0).snapshot().bytes;
        trailing.push(0);
        assert_eq!(Spine::restore(&trailing), Err(SpineError::Trailing(1)));
        let mut many = Spine::with_capacity(0).snapshot().bytes;
        many[2..6].copy_from_slice(&(MAX_SPINE_SLOTS as u32 + 1).to_le_bytes());
        assert_eq!(
            Spine::restore(&many),
            Err(SpineError::TooManySlots(MAX_SPINE_SLOTS as u32 + 1))
        );
        let short = &seeded().snapshot().bytes[..20];
        assert_eq!(Spine::restore(short), Err(SpineError::TooShort));
    }

    #[test]
    fn signed_zero_hashes_like_zero() {
        let mut a = seeded();
        let mut b = seeded();
        a.slot_mut(0).expect("live").transform.pos[1] = 0.0;
        b.slot_mut(0).expect("live").transform.pos[1] = -0.0;
        a.slot_mut(1).expect("live").anim.phase = 0.0;
        b.slot_mut(1).expect("live").anim.phase = -0.0;
        assert_eq!(a.snapshot().bytes, b.snapshot().bytes);
        assert_eq!(a.canonical_hash(), b.canonical_hash());
        assert_eq!(a.column_digests(), b.column_digests());
    }

    #[test]
    fn column_names_and_order_are_stable() {
        let names: Vec<&str> = Column::ORDER.iter().map(|c| c.name()).collect();
        assert_eq!(names, ["transform", "body", "vitals", "ai", "anim"]);
    }

    #[test]
    fn spine_bundles_wire_the_optional_views() {
        use crate::views::SpawnBundle;
        let mut spine = Spine::with_capacity(4);
        let bundles = [
            SpawnBundle {
                transform: Some(Transform {
                    pos: [1.0, 0.0, -1.0],
                    yaw: 0.5,
                }),
                body: Some(body()),
                vitals: Some(Vitals {
                    hp_permille: 900,
                    stamina: 80,
                    poise: 0,
                    state: 0,
                }),
                ai: None,
                anim: None,
            },
            SpawnBundle::default(),
        ];
        let slot = spine.spawn_bundle(&bundles[0]).expect("fits");
        assert_eq!(slot, 0);
        assert_eq!(spine.live(), 1);
        let live = spine.spawn_bundles(&bundles[1..]).expect("fits");
        assert_eq!((live, spine.live()), (2, 2));
        let (mut seen, mut ai_seen) = (0usize, 0usize);
        spine.for_each_optional(|_, view| {
            seen += 1;
            ai_seen += usize::from(view.ai.is_some());
        });
        assert_eq!((seen, ai_seen), (2, 0));
        // Failure path stays loud through the wrapper.
        let mut full = Spine::with_capacity(1);
        full.spawn_bundle(&bundles[0]).expect("fits");
        assert!(full.spawn_bundle(&bundles[1]).is_err());
    }

    #[test]
    fn step_and_hash_allocate_nothing_after_warmup() {
        // Calibrate the probe first: a zero reading must mean zero.
        let (_, seen) = measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls >= 1, "alloc probe is blind");
        let mut spine = seeded();
        // Warmup: one step writing every column of every live slot.
        let step = |spine: &mut Spine, tick: u32| {
            spine.for_each_mut(|slot, cell| {
                cell.transform.pos[0] += 0.01;
                cell.transform.yaw += 0.001;
                cell.body.vel = [Fixed::from_bits(655), Fixed::from_bits(-655)];
                cell.vitals.stamina = cell.vitals.stamina.saturating_sub(1);
                cell.vitals.state = Vitals::ATTACKING;
                cell.ai.intent.mx = slot as i8;
                cell.ai.clock = tick;
                cell.ai.source = AiState::FROM_TAPE;
                cell.anim.phase += 0.05;
                cell.anim.loco = 2;
            });
        };
        step(&mut spine, 1);
        let ((), heap) = measure(|| {
            for tick in 2..66 {
                step(&mut spine, tick);
                std::hint::black_box(spine.canonical_hash());
                std::hint::black_box(spine.column_digests());
            }
        });
        assert_eq!(
            heap.calls, 0,
            "spine step or hash allocated on the tick path"
        );
    }
}
