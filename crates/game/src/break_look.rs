//! break_look: the picture of a break, painted from authority events and never fed back.
//!
//! The arena's bonded collection decides what breaks and emits [`BreakEvent`]s: a bond snapped,
//! a chunk lost its last path to an anchor. A [`BreakLook`] watches one prop. From the tick of its
//! first event the intact mesh is hidden and the prop is drawn as its fracture chunks instead;
//! every chunk the authority frees becomes a body of the float physical layer (`engine-physfx`)
//! that tumbles, lands and sleeps. [`BreakLook::frame`] hands the renderer one list: a mesh key
//! and a transform per piece, plus the keys of the intact meshes to skip. The draw hook is
//! [`crate::break_window::BreakScene`].
//!
//! A tick whose blow snaps a bond also makes one sound: [`BreakLook::take_cue`] hands the window
//! one [`BreakCue`] (the bank's impact sample, a gain that rises with the blow and the struck
//! chunk's center) for the audio feed to play, and one dust cloud: [`BreakLook::take_dust`] hands
//! the render's particle feed (NS11) one [`BreakDust`] (where the cloud sits, how many puffs and
//! how wide) for every tick that frees pieces. A blast maps the same way: [`blast_cue`] and
//! [`blast_dust`] turn the authority's blast integers into the same two feed shapes (the bank has
//! no explosion sample, so a blast plays the stone crash, loudest at the physical layer's own
//! momentum cap).
//!
//! Caller: the window loop calls [`BreakLook::observe`] once per accepted sim tick with that
//! tick's `TickBreaks` events and [`BreakLook::frame`] once per drawn frame with the sub-tick
//! alpha; the break scene does both, takes the cue for the audio feed and the dust for the
//! particle feed.
//! Data owner: the authority owns what broke; this module owns only the picture. Units: meters,
//! seconds, Y up; rotations are unit quaternions `[x, y, z, w]`. Determinism: the same events on
//! the same ticks give the same transforms on one machine; nothing here is hashed and the sim
//! never reads it. Errors: [`LookError`]; a refused call leaves the look exactly as it was.
//! Budget: at most the tier's body cap of debris bodies (the oldest is recycled first), and no
//! work at all before the first event.

use engine_audio::cue_names;
use engine_core::hash::hash_bytes;
use engine_destruction::bonded::{Bond, BreakEvent, Chunk3, MAX_BONDED_CHUNKS};
use engine_physfx::{
    Blast, Error as FxError, Key, Material, PhysFx, Shard, Stats, Tier, Transform, MAX_BLAST_NS,
};
use engine_render::present::MeshKey;
use std::collections::BTreeMap;

/// Mesh id block of break pieces. Each prop owns [`IDS_PER_PROP`] ids of it (two per chunk), so
/// two props never share a key.
pub const BREAK_MESH_ID_BASE: u64 = 0xB4EA_0000;
/// Ids one prop owns in that block.
const IDS_PER_PROP: u64 = 2048;
const _: () = assert!(
    MAX_BONDED_CHUNKS * 2 <= 2048,
    "a prop's chunks must fit its id block"
);
/// Half-width of the floor the debris lands on, meters.
const FLOOR_HALF_M: f32 = 50.0;
/// Half-thickness of that floor slab, meters.
const FLOOR_THICK_M: f32 = 0.5;
/// How much a blow lifts the pieces: the push leans up by this much of its own length.
const PUSH_LIFT: f32 = 0.5;
/// Fastest speed a blow may give the freed pieces as a whole, m/s.
const MAX_KICK_MS: f32 = 12.0;
/// Largest push handed to the physical layer, newton-seconds. It accepts 1e5 per axis.
const MAX_PUSH_NS: f32 = 90_000.0;
/// No side of a flying piece is thinner than this share of its longest side: rubble is chunky,
/// and a fracture patch cut from a flat face has no thickness at all.
const MIN_SIDE_SHARE: f32 = 0.25;
/// A flying piece's body is this share of its chunk box. The boxes of neighbouring chunks overlap
/// (they are boxes around cut cells), and bodies that start inside each other keep shoving; the
/// margin lets them start apart and sleep sooner.
const BODY_FIT: f32 = 0.8;
/// Thinnest a piece is made, half-extent in meters: the physical layer refuses bodies under 1 cm.
const MIN_BODY_HALF_M: f32 = 0.02;
/// Largest piece the physical layer takes, half-extent in meters. A bigger chunk is a prop to
/// split first.
const MAX_BODY_HALF_M: f32 = 5.0;

/// Why a prop or a call was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum LookError {
    /// The prop has no chunks.
    EmptyProp,
    /// The prop has more chunks than a bonded collection holds.
    TooManyChunks,
    /// A chunk is not finite or has no size; its index is carried.
    BadChunk(usize),
    /// A bond names a chunk the prop does not have; its index is carried.
    BadBond(usize),
    /// An anchor names a chunk the prop does not have; its index is carried.
    BadAnchor(usize),
    /// An event names a chunk the prop does not have.
    UnknownChunk(u32),
    /// An event names a bond the prop does not have.
    UnknownBond(u32),
    /// The tick is not after the last observed one: a replay would spawn debris twice.
    StaleTick {
        /// The last tick observed.
        last: u32,
        /// The tick given.
        got: u32,
    },
    /// The physical layer refused a spawn or a read.
    Fx(FxError),
}

impl std::fmt::Display for LookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyProp => f.write_str("break prop has no chunks"),
            Self::TooManyChunks => f.write_str("break prop has more chunks than a collection"),
            Self::BadChunk(i) => write!(f, "break chunk {i} is not finite or has no size"),
            Self::BadBond(i) => write!(f, "break bond {i} names a missing chunk"),
            Self::BadAnchor(i) => write!(f, "break anchor {i} names a missing chunk"),
            Self::UnknownChunk(i) => write!(f, "break event names missing chunk {i}"),
            Self::UnknownBond(i) => write!(f, "break event names missing bond {i}"),
            Self::StaleTick { last, got } => {
                write!(f, "tick {got} is not after the last observed tick {last}")
            }
            Self::Fx(error) => write!(f, "physical layer refused: {error}"),
        }
    }
}

impl std::error::Error for LookError {}

/// A mesh the draw hook uploads once under its key: a box of half-extents `half`, centered on
/// its own origin, so a [`PieceDraw`] transform places it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceMesh {
    /// Key of the piece, stable for the prop, the chunk and the size.
    pub key: MeshKey,
    /// Half-extents of the box, meters.
    pub half: [f32; 3],
}

/// The two boxes a chunk is drawn as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkMeshes {
    /// The chunk's own box, drawn while it stands in the wall.
    pub standing: PieceMesh,
    /// The box of the body the physical layer simulates, drawn while the chunk flies, so the
    /// picture rests where the physics rests.
    pub flying: PieceMesh,
}

/// One breakable prop as the picture knows it: the rest boxes of its chunks and how they hang
/// together. Take the boxes before the first step of the collection (a freed chunk moves).
#[derive(Clone, Debug)]
pub struct BreakProp {
    intact: MeshKey,
    material: Material,
    chunks: Vec<Chunk3>,
    /// Endpoints of each bond, in descriptor order.
    bonds: Vec<[usize; 2]>,
    /// Chunks that never fall, ascending and unique.
    anchors: Vec<usize>,
    pieces: Vec<ChunkMeshes>,
    /// Bottom of the lowest chunk: the ground the debris lands on.
    floor_y: f32,
}

impl BreakProp {
    /// A prop from its rest chunks, bond list and anchors (a `BondedDesc`'s fields, or a
    /// `BondedCollection`'s `chunks()`, `bonds()` and `anchors()` before its first step).
    /// `prop` numbers the prop so its piece keys are its own; `intact` is the mesh the picture
    /// draws while it stands; `material` says what the debris is made of.
    pub fn new(
        prop: u16,
        intact: MeshKey,
        material: Material,
        chunks: &[Chunk3],
        bonds: &[Bond],
        anchors: &[u32],
    ) -> Result<Self, LookError> {
        if chunks.is_empty() {
            return Err(LookError::EmptyProp);
        }
        if chunks.len() > MAX_BONDED_CHUNKS {
            return Err(LookError::TooManyChunks);
        }
        for (i, c) in chunks.iter().enumerate() {
            let finite = [c.x, c.y, c.z, c.hx, c.hy, c.hz, c.mass]
                .iter()
                .all(|v| v.is_finite());
            let sized = [c.hx, c.hy, c.hz]
                .iter()
                .all(|h| *h > 0.0 && *h <= MAX_BODY_HALF_M);
            if !finite || !sized || c.mass <= 0.0 {
                return Err(LookError::BadChunk(i));
            }
        }
        let count = chunks.len();
        let mut ends = Vec::with_capacity(bonds.len());
        for (i, bond) in bonds.iter().enumerate() {
            match (index(bond.a, count), index(bond.b, count)) {
                (Some(a), Some(b)) => ends.push([a, b]),
                _ => return Err(LookError::BadBond(i)),
            }
        }
        let mut stand = Vec::with_capacity(anchors.len());
        for (i, anchor) in anchors.iter().enumerate() {
            stand.push(index(*anchor, count).ok_or(LookError::BadAnchor(i))?);
        }
        stand.sort_unstable();
        stand.dedup();
        let pieces = chunks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let chunk = [c.hx, c.hy, c.hz];
                let longest = c.hx.max(c.hy).max(c.hz);
                let standing = chunk.map(|h| h.max(MIN_BODY_HALF_M));
                let flying = chunk
                    .map(|h| (h.max(longest * MIN_SIDE_SHARE) * BODY_FIT).max(MIN_BODY_HALF_M));
                ChunkMeshes {
                    standing: PieceMesh {
                        key: piece_key(prop, i, false, standing),
                        half: standing,
                    },
                    flying: PieceMesh {
                        key: piece_key(prop, i, true, flying),
                        half: flying,
                    },
                }
            })
            .collect();
        let floor_y = chunks
            .iter()
            .map(|c| c.y - c.hy)
            .fold(f32::INFINITY, f32::min);
        Ok(Self {
            intact,
            material,
            chunks: chunks.to_vec(),
            bonds: ends,
            anchors: stand,
            pieces,
            floor_y,
        })
    }
}

/// One piece to draw this frame: which mesh, and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceDraw {
    /// Key of the piece mesh ([`BreakLook::meshes`] lists what to upload under each).
    pub mesh: MeshKey,
    /// Where the piece is, blended between the last two ticks.
    pub at: Transform,
}

impl PieceDraw {
    /// The model matrix as 3 rows of 4, row-major: the rotation, then the position in the
    /// last column. The layout of `InstanceRecord::model` in the renderer.
    #[must_use]
    pub fn model_3x4(&self) -> [f32; 12] {
        let [x, y, z, w] = self.at.rot;
        let [px, py, pz] = self.at.pos;
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            px,
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            py,
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            pz,
        ]
    }
}

/// The sound one break or blast makes: which bank cue to play, how loud and where the blow landed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreakCue {
    /// Bank cue name (`engine_audio::cue_names`). The bank ships one impact sample; a break
    /// plays it.
    pub name: &'static str,
    /// Gain in `0.0..=1.0`, rising with the speed the blow gives the freed pieces.
    pub gain: f32,
    /// Center of the chunk the blow struck, meters.
    pub at_m: [f32; 3],
}

/// Quietest a break sounds: rubble is always a crash, a nudge at the gate still lands.
const MIN_BREAK_GAIN: f32 = 0.3;

/// The dust one break or blast kicks up: how many puffs, where the cloud sits and how wide it is.
/// The render's particle feed (NS11) turns this into emitters; this module only says what the blow
/// raised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreakDust {
    /// Center of the cloud, meters: the struck chunk on a blow, the freed pieces' mean on a
    /// collapse.
    pub at_m: [f32; 3],
    /// Puffs this break asks for, at least [`PUFFS_PER_PIECE`] per freed piece and rising with
    /// the blow, never over [`MAX_PUFFS_PER_BREAK`].
    pub puffs: u32,
    /// Radius of the cloud, meters: how far the freed pieces fell from the blow, within
    /// [`MIN_DUST_RADIUS_M`]`..=`[`MAX_DUST_RADIUS_M`].
    pub radius_m: f32,
}

/// Puffs each freed piece raises, before the blow's share.
const PUFFS_PER_PIECE: u32 = 2;
/// Ceiling of one break's dust: the feed's per-event budget, whatever the cloud.
const MAX_PUFFS_PER_BREAK: u32 = 96;
/// Steps the blow's share of the puffs climbs; the top step doubles the base.
const DUST_STEPS: u32 = 3;
/// Smallest cloud radius, meters: a nudge still puffs around the struck chunk.
const MIN_DUST_RADIUS_M: f32 = 0.2;
/// Largest cloud radius, meters: a wall's fall never asks the feed for a fog bank.
const MAX_DUST_RADIUS_M: f32 = 4.0;

/// What to draw for the prop in one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// The pieces, standing ones first (chunk order), then the flying ones (oldest first).
    /// Empty until the prop breaks.
    pub draws: Vec<PieceDraw>,
    /// Intact meshes to skip: the prop's own mesh from the break tick on.
    pub hidden: Vec<MeshKey>,
}

/// What the picture shows of one chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Piece {
    /// Still in place.
    Standing,
    /// Has a body in the physical layer (until it is recycled).
    Flying,
    /// Freed, but the body cap left no room: it is simply gone.
    Gone,
}

/// What one tick's events ask for.
struct Plan {
    /// Chunks the tick frees, in spawn priority order.
    freed: Vec<usize>,
    /// The chunk that took the blow, and the blow in the authority's units.
    hit: Option<(usize, f32)>,
}

/// The look of one breakable prop. See the module docs.
pub struct BreakLook {
    prop: BreakProp,
    fx: PhysFx,
    /// Debris bodies the tier admits at once.
    body_cap: usize,
    pieces: Vec<Piece>,
    /// `(batch, index)` of a debris body to the chunk it is.
    flying: BTreeMap<(u32, u32), usize>,
    broke: bool,
    /// Sound of the last observed tick, until [`BreakLook::take_cue`] takes it.
    cue: Option<BreakCue>,
    /// Dust of the last observed tick, until [`BreakLook::take_dust`] takes it.
    dust: Option<BreakDust>,
    last_tick: Option<u32>,
}

impl BreakLook {
    /// A look over `prop` with the caps of `tier`: a floor under the prop and a solid box for
    /// every anchored chunk, so the debris lands and piles against the stumps.
    pub fn new(tier: Tier, prop: BreakProp) -> Result<Self, LookError> {
        let mut fx = PhysFx::new(tier);
        let count = prop.chunks.len();
        let centre = |axis: fn(&Chunk3) -> f32| {
            prop.chunks.iter().map(axis).sum::<f32>() / f32::from(u16::try_from(count).unwrap_or(1))
        };
        fx.add_static_box(
            [FLOOR_HALF_M, FLOOR_THICK_M, FLOOR_HALF_M],
            [
                centre(|c| c.x),
                prop.floor_y - FLOOR_THICK_M,
                centre(|c| c.z),
            ],
        )
        .map_err(LookError::Fx)?;
        for anchor in &prop.anchors {
            let c = prop.chunks[*anchor];
            fx.add_static_box(prop.pieces[*anchor].standing.half, [c.x, c.y, c.z])
                .map_err(LookError::Fx)?;
        }
        Ok(Self {
            fx,
            body_cap: tier.caps().bodies,
            pieces: vec![Piece::Standing; count],
            flying: BTreeMap::new(),
            broke: false,
            cue: None,
            dust: None,
            last_tick: None,
            prop,
        })
    }

    /// Shoves this prop's flying debris with an authority blast and reports how many bodies the
    /// physical layer moved. Standing chunks are the authority's, not bodies of this layer, so a
    /// blast before the break moves nothing; a refused blast leaves the world unchanged.
    pub fn apply_blast(&mut self, blast: Blast) -> Result<usize, LookError> {
        self.fx.apply_blast(blast).map_err(LookError::Fx)
    }

    /// Applies one authority blast and raises its sound and dust in the same call: the integers
    /// shove the flying debris exactly as [`BreakLook::apply_blast`], and [`BreakLook::take_cue`]
    /// and [`BreakLook::take_dust`] then hand out the blast's cue and cloud. A blast before the
    /// break moves no body but still sounds and puffs; a refused blast raises nothing. Call it
    /// after [`BreakLook::observe`] for the same tick: the blast is the louder event and its cue
    /// replaces that tick's break cue.
    pub fn observe_blast(&mut self, blast: Blast) -> Result<usize, LookError> {
        let pushed = self.apply_blast(blast)?;
        self.cue = Some(blast_cue(blast));
        self.dust = Some(blast_dust(blast));
        Ok(pushed)
    }

    /// Takes one sim tick: spawns debris for the chunks the events free, then steps the physical
    /// layer once. Call it every accepted tick, with an empty slice when nothing broke. Any
    /// event at all hides the intact mesh from this tick on. `tick` is the caller's sim tick;
    /// the ticks inside the events are not read. A tick at or before the last one is refused.
    pub fn observe(&mut self, tick: u32, events: &[BreakEvent]) -> Result<(), LookError> {
        if let Some(last) = self.last_tick {
            if tick <= last {
                return Err(LookError::StaleTick { last, got: tick });
            }
        }
        let plan = self.plan(events)?;
        let take = plan.freed.len().min(self.body_cap);
        let heard: &[usize] = if take == 0 {
            &plan.freed
        } else {
            &plan.freed[..take]
        };
        let cue = self.break_cue(heard, plan.hit);
        let dust = self.break_dust(heard, plan.hit);
        let admitted = if take == 0 {
            None
        } else {
            let (shards, push) = self.debris(&plan.freed[..take], plan.hit);
            let material = self.prop.material;
            let batch = self
                .fx
                .spawn_debris_of(material, &shards, push)
                .map_err(LookError::Fx)?;
            Some(batch)
        };
        // The physical layer took the batch: nothing below can fail.
        if let Some(batch) = admitted {
            for (index, chunk) in plan.freed[..take].iter().enumerate() {
                if index < batch.admitted {
                    self.pieces[*chunk] = Piece::Flying;
                    let slot = u32::try_from(index).unwrap_or(u32::MAX);
                    self.flying.insert((batch.id, slot), *chunk);
                } else {
                    self.pieces[*chunk] = Piece::Gone;
                }
            }
        }
        for chunk in plan.freed.iter().skip(take) {
            self.pieces[*chunk] = Piece::Gone;
        }
        self.broke |= !events.is_empty();
        if self.broke {
            self.fx.step();
        }
        self.cue = cue;
        self.dust = dust;
        self.last_tick = Some(tick);
        Ok(())
    }

    /// What to draw now. `alpha` in `[0, 1]` blends the last two sim ticks (a non-finite one is
    /// refused). Before the first event the frame is empty: the intact mesh draws itself.
    pub fn frame(&self, alpha: f32) -> Result<Frame, LookError> {
        if !self.broke {
            return Ok(Frame::default());
        }
        let placed = self.fx.transforms(alpha).map_err(LookError::Fx)?;
        let mut draws = Vec::with_capacity(self.pieces.len());
        for (chunk, piece) in self.pieces.iter().enumerate() {
            if *piece == Piece::Standing {
                let c = self.prop.chunks[chunk];
                draws.push(PieceDraw {
                    mesh: self.prop.pieces[chunk].standing.key,
                    at: Transform {
                        pos: [c.x, c.y, c.z],
                        ..Transform::IDENTITY
                    },
                });
            }
        }
        for body in placed {
            if let Key::Shard { batch, index } = body.key {
                if let Some(chunk) = self.flying.get(&(batch, index)) {
                    draws.push(PieceDraw {
                        mesh: self.prop.pieces[*chunk].flying.key,
                        at: body.at,
                    });
                }
            }
        }
        Ok(Frame {
            draws,
            hidden: vec![self.prop.intact],
        })
    }

    /// The piece meshes to upload once: two boxes per chunk, in chunk order.
    #[must_use]
    pub fn meshes(&self) -> &[ChunkMeshes] {
        &self.prop.pieces
    }

    /// The sound the last observed tick's blow made, once: the window's audio feed plays it. A
    /// quiet tick, a tick that only frees chunks, and a second read all give `None`.
    pub fn take_cue(&mut self) -> Option<BreakCue> {
        self.cue.take()
    }

    /// The dust the last observed tick raised, once: the render's particle feed (NS11) plays it.
    /// A tick that frees nothing and a second read give `None`.
    pub fn take_dust(&mut self) -> Option<BreakDust> {
        self.dust.take()
    }

    /// Counters of the physical layer, for its receipt line.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.fx.stats()
    }

    /// Reads the events of one tick without changing anything: which standing chunks they free,
    /// nearest the blow first (the order the body cap keeps them in), and the blow itself.
    fn plan(&self, events: &[BreakEvent]) -> Result<Plan, LookError> {
        let count = self.prop.chunks.len();
        let mut freed = Vec::new();
        let mut seen = vec![false; count];
        let mut ends = vec![0u32; count];
        let mut impulse = 0.0f32;
        for event in events {
            match *event {
                BreakEvent::BondBroken { bond, strain, .. } => {
                    let [a, b] = index(bond, self.prop.bonds.len())
                        .map(|i| self.prop.bonds[i])
                        .ok_or(LookError::UnknownBond(bond))?;
                    ends[a] += 1;
                    ends[b] += 1;
                    // The authority adds `impulse / mean mass` to a bond's strain: run it back.
                    let mean = (self.prop.chunks[a].mass + self.prop.chunks[b].mass) * 0.5;
                    impulse = impulse.max(strain * mean);
                }
                BreakEvent::ChunkFreed { chunk, .. } => {
                    let i = index(chunk, count).ok_or(LookError::UnknownChunk(chunk))?;
                    if self.pieces[i] == Piece::Standing && !seen[i] {
                        seen[i] = true;
                        freed.push(i);
                    }
                }
            }
        }
        // The struck chunk sits in the most snapped bonds; the lowest index wins a tie.
        let hit = ends
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .max_by_key(|(i, n)| (**n, std::cmp::Reverse(*i)))
            .map(|(i, _)| (i, impulse));
        if let Some((struck, _)) = hit {
            let at = self.prop.chunks[struck];
            let dist2 = |i: &usize| {
                let c = self.prop.chunks[*i];
                let d = [c.x - at.x, c.y - at.y, c.z - at.z];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            };
            freed.sort_by(|a, b| dist2(a).total_cmp(&dist2(b)).then(a.cmp(b)));
        }
        Ok(Plan { freed, hit })
    }

    /// The bodies for these freed chunks (already cut to what the cap admits) and the push of the
    /// blow in newton-seconds. The blow gives the freed pieces, as one body, the speed the
    /// authority's impulse would give their combined mass, away from the struck chunk and leaning
    /// up. A tick with no snapped bond (a collapse) pushes nothing: the pieces just fall.
    fn debris(&self, freed: &[usize], hit: Option<(usize, f32)>) -> (Vec<Shard>, [f32; 3]) {
        let shards: Vec<Shard> = freed
            .iter()
            .map(|i| {
                let c = self.prop.chunks[*i];
                Shard {
                    at: Transform {
                        pos: [c.x, c.y, c.z],
                        ..Transform::IDENTITY
                    },
                    half: self.prop.pieces[*i].flying.half,
                    velocity: [0.0; 3],
                }
            })
            .collect();
        let Some((struck, kick)) = self.kick(freed, hit) else {
            return (shards, [0.0; 3]);
        };
        let density = self.prop.material.props().density();
        let kilograms: f32 = shards
            .iter()
            .map(|s| density * 8.0 * s.half[0] * s.half[1] * s.half[2])
            .sum();
        let at = self.prop.chunks[struck];
        let count = f32::from(u16::try_from(freed.len()).unwrap_or(1));
        let away = [
            shards.iter().map(|s| s.at.pos[0]).sum::<f32>() / count - at.x,
            shards.iter().map(|s| s.at.pos[1]).sum::<f32>() / count - at.y,
            shards.iter().map(|s| s.at.pos[2]).sum::<f32>() / count - at.z,
        ];
        let lean = unit(away).map_or([0.0, 1.0, 0.0], |u| {
            unit([u[0], u[1] + PUSH_LIFT, u[2]]).unwrap_or([0.0, 1.0, 0.0])
        });
        let push = (kick * kilograms).min(MAX_PUSH_NS);
        (shards, lean.map(|c| c * push))
    }

    /// The chunk a blow struck and the speed it gives the freed pieces as one body, m/s: the
    /// authority's impulse over their combined mass, capped. `None` on a tick with no snapped
    /// bond, so a collapse that only frees chunks carries no blow of its own.
    fn kick(&self, freed: &[usize], hit: Option<(usize, f32)>) -> Option<(usize, f32)> {
        let (struck, impulse) = hit?;
        let mass: f32 = freed.iter().map(|i| self.prop.chunks[*i].mass).sum();
        Some((struck, (impulse / mass).min(MAX_KICK_MS)))
    }

    /// The sound this tick's blow makes: the bank's impact sample at the struck chunk, its gain
    /// rising with [`Self::kick`]. `None` when no bond snapped or the speed is not finite.
    fn break_cue(&self, freed: &[usize], hit: Option<(usize, f32)>) -> Option<BreakCue> {
        let (struck, speed) = self.kick(freed, hit)?;
        if !speed.is_finite() {
            return None;
        }
        let c = self.prop.chunks[struck];
        Some(BreakCue {
            name: cue_names::IMPACT_STONE,
            gain: break_gain(speed),
            at_m: [c.x, c.y, c.z],
        })
    }

    /// The dust this tick raised: a blow puffs at the struck chunk, a collapse (no snapped bond)
    /// over the mean of the freed pieces. `None` on a tick that frees nothing. The cloud's radius
    /// is how far the freed pieces fall from its center, so a whole wall's fall is a wider cloud
    /// than one knocked corner.
    fn break_dust(&self, freed: &[usize], hit: Option<(usize, f32)>) -> Option<BreakDust> {
        if freed.is_empty() {
            return None;
        }
        let count = f32::from(u16::try_from(freed.len()).unwrap_or(u16::MAX));
        let centre = |axis: usize| {
            freed
                .iter()
                .map(|i| {
                    let c = self.prop.chunks[*i];
                    [c.x, c.y, c.z][axis]
                })
                .sum::<f32>()
                / count
        };
        let at_m = match hit {
            Some((struck, _)) => {
                let c = self.prop.chunks[struck];
                [c.x, c.y, c.z]
            }
            None => [centre(0), centre(1), centre(2)],
        };
        let spread = freed
            .iter()
            .map(|i| {
                let c = self.prop.chunks[*i];
                let d = [c.x - at_m[0], c.y - at_m[1], c.z - at_m[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .sum::<f32>()
            / count;
        let speed = self.kick(freed, hit).map_or(0.0, |(_, speed)| speed);
        Some(BreakDust {
            at_m,
            puffs: dust_puffs(freed.len(), speed),
            radius_m: spread.clamp(MIN_DUST_RADIUS_M, MAX_DUST_RADIUS_M),
        })
    }
}

/// Gain of a break at `speed_m`: a crash at the cap, never silent and never over one.
fn break_gain(speed_m: f32) -> f32 {
    (speed_m / MAX_KICK_MS).clamp(MIN_BREAK_GAIN, 1.0)
}

/// Puffs a break of `freed` pieces raises at `kick_ms`: [`PUFFS_PER_PIECE`] each, plus up to that
/// much again as the blow climbs [`MAX_KICK_MS`], capped at [`MAX_PUFFS_PER_BREAK`]. A collapse
/// (speed zero, or not finite) still raises the base cloud.
fn dust_puffs(freed: usize, kick_ms: f32) -> u32 {
    let base = u32::try_from(freed)
        .unwrap_or(u32::MAX)
        .saturating_mul(PUFFS_PER_PIECE);
    let steps = if !kick_ms.is_finite() || kick_ms <= MAX_KICK_MS * 0.25 {
        0
    } else if kick_ms <= MAX_KICK_MS * 0.5 {
        1
    } else if kick_ms <= MAX_KICK_MS * 0.75 {
        DUST_STEPS - 1
    } else {
        DUST_STEPS
    };
    base.saturating_add(base.saturating_mul(steps) / DUST_STEPS)
        .min(MAX_PUFFS_PER_BREAK)
}

/// Quietest a blast sounds: an explosion is never a rustle, however small.
const MIN_BLAST_GAIN: f32 = 0.4;
/// Puffs a blast raises before its momentum's share.
const BASE_BLAST_PUFFS: u32 = 24;
/// Ceiling of one blast's dust, well inside the particle feed's per-event budget.
const MAX_BLAST_PUFFS: u32 = 192;
/// Smallest blast cloud radius, meters: a point blast still puffs.
const MIN_BLAST_DUST_RADIUS_M: f32 = 0.5;
/// Largest blast cloud radius, meters: a 100 m blast never asks the feed for a fog bank.
const MAX_BLAST_DUST_RADIUS_M: f32 = 8.0;

/// The sound an authority blast makes: the bank's stone crash at the blast's center, its gain
/// rising with the momentum it carries and full at the physical layer's [`MAX_BLAST_NS`] cap. A
/// blast is never silent.
#[must_use]
pub fn blast_cue(blast: Blast) -> BreakCue {
    BreakCue {
        name: cue_names::IMPACT_STONE,
        gain: blast_gain(blast.impulse_ns),
        at_m: blast_center_m(blast),
    }
}

/// The dust an authority blast throws: a cloud at its center as wide as the blast reaches, with
/// puffs rising with the momentum up to [`MAX_BLAST_PUFFS`]. The render's particle feed (NS11)
/// turns this into emitters; this module only says what the blast raised.
#[must_use]
pub fn blast_dust(blast: Blast) -> BreakDust {
    let reach_m = millimeters_m(blast.radius_mm);
    BreakDust {
        at_m: blast_center_m(blast),
        puffs: blast_puffs(blast.impulse_ns),
        radius_m: reach_m.clamp(MIN_BLAST_DUST_RADIUS_M, MAX_BLAST_DUST_RADIUS_M),
    }
}

/// The blast's center in meters.
fn blast_center_m(blast: Blast) -> [f32; 3] {
    blast.center_mm.map(|mm| {
        let magnitude = millimeters_m(mm.unsigned_abs());
        if mm < 0 {
            -magnitude
        } else {
            magnitude
        }
    })
}

/// `mm` as meters without a numeric cast. A valid blast is under 100 m, so the whole-meter
/// quotient fits the exact `u16` conversion; a wild value saturates and the caller clamps it.
fn millimeters_m(mm: u32) -> f32 {
    let whole = u16::try_from(mm / 1_000).unwrap_or(u16::MAX);
    let part = u16::try_from(mm % 1_000).unwrap_or_default();
    f32::from(whole) + f32::from(part) / 1_000.0
}

/// Gain of a blast carrying `impulse_ns`: half at a 3500 N s blast, full at [`MAX_BLAST_NS`],
/// never silent.
fn blast_gain(impulse_ns: u32) -> f32 {
    let share = u64::from(impulse_ns).min(u64::from(MAX_BLAST_NS));
    let permille = u32::try_from(share * 1_000 / u64::from(MAX_BLAST_NS)).unwrap_or(1_000);
    let gain = f32::from(u16::try_from(permille).unwrap_or(1_000)) / 1_000.0;
    gain.clamp(MIN_BLAST_GAIN, 1.0)
}

/// Puffs a blast carrying `impulse_ns` raises: [`BASE_BLAST_PUFFS`] plus a share of the rest that
/// rises to [`MAX_BLAST_PUFFS`] at [`MAX_BLAST_NS`].
fn blast_puffs(impulse_ns: u32) -> u32 {
    let share = u64::from(impulse_ns).min(u64::from(MAX_BLAST_NS));
    let extra = u64::from(MAX_BLAST_PUFFS - BASE_BLAST_PUFFS) * share / u64::from(MAX_BLAST_NS);
    u32::try_from(u64::from(BASE_BLAST_PUFFS) + extra).unwrap_or(MAX_BLAST_PUFFS)
}

/// `value` as an index below `len`, when it is one.
fn index(value: u32, len: usize) -> Option<usize> {
    usize::try_from(value).ok().filter(|i| *i < len)
}

/// `v` scaled to length one; `None` for a vector too short to have a direction.
fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (len > 1e-6).then(|| v.map(|c| c / len))
}

/// Key of one chunk's standing or flying box: the prop's id block, then two ids per chunk; the
/// digest follows the box size, so a recooked prop uploads again.
fn piece_key(prop: u16, chunk: usize, flying: bool, half: [f32; 3]) -> MeshKey {
    let slot = u64::try_from(chunk).unwrap_or(0) * 2 + u64::from(flying);
    let id = BREAK_MESH_ID_BASE + u64::from(prop) * IDS_PER_PROP + slot;
    let mut raw = [0u8; 12];
    for (slot, h) in raw.chunks_exact_mut(4).zip(half) {
        slot.copy_from_slice(&h.to_bits().to_le_bytes());
    }
    let mut digest = [0xB4u8; 32];
    digest[..8].copy_from_slice(&hash_bytes(&raw).to_le_bytes());
    digest[8..10].copy_from_slice(&prop.to_le_bytes());
    MeshKey::new(id, digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_kit::{kit_actors, kit_mesh_key, KitActor};
    use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
    use engine_destruction::bake::BREAKING_THRESHOLD;
    use engine_destruction::bonded::{BondedCollection, BondedDesc, DEFAULT_BOND_STRAIN};
    use engine_destruction::{fracture_at, FractureSpec, PropMesh};
    use engine_input::fighter::ATTACK;
    use engine_input::{Buttons, FighterId, FighterInput};

    /// The sim tick the scripted impact lands on.
    const BREAK_TICK: u32 = 4;
    /// Six seconds of sim time at the fixed 60 Hz.
    const SIX_SECONDS: u32 = 360;
    /// Match seed of the hash test.
    const SEED: u64 = 2040;

    /// A closed box shell: 12 triangles, the winding of the destruction fixtures.
    fn box_shell(min: [f32; 3], max: [f32; 3]) -> PropMesh {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        PropMesh {
            positions: vec![
                [x0, y0, z0],
                [x1, y0, z0],
                [x1, y1, z0],
                [x0, y1, z0],
                [x0, y0, z1],
                [x1, y0, z1],
                [x1, y1, z1],
                [x0, y1, z1],
            ],
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
        }
    }

    /// Chunks this near the hit take the blow too, as under a thrown boulder, meters.
    const STRIKE_REACH_M: f32 = 1.6;

    /// The kit's north wall (actor 3), fractured around its middle and struck there on
    /// `BREAK_TICK`.
    struct WallBreak {
        actor: KitActor,
        desc: BondedDesc,
        events: Vec<BreakEvent>,
    }

    /// `blow` is added to the impulse that just clears the authority's fracture gate, per chunk.
    fn wall_break(blow: f32) -> WallBreak {
        wall_break_seeded(blow, 7)
    }

    /// [`wall_break`] with the fracture pattern of `seed`.
    fn wall_break_seeded(blow: f32, seed: u64) -> WallBreak {
        let actor = kit_actors()
            .expect("placement parses")
            .into_iter()
            .find(|a| a.id == 3)
            .expect("wall_north is actor 3");
        let half = actor.size_m.map(|s| s * 0.5);
        let min = [0, 1, 2].map(|a| actor.pos_m[a] - half[a]);
        let max = [0, 1, 2].map(|a| actor.pos_m[a] + half[a]);
        let spec = FractureSpec {
            cells: 24,
            radius_m: 1.2,
            seed,
        };
        let fracture = fracture_at(&box_shell(min, max), actor.pos_m, &spec).expect("fractures");
        let desc = fracture
            .to_desc(DEFAULT_BOND_STRAIN, BREAKING_THRESHOLD)
            .expect("descriptor");
        let mut field = BondedCollection::new(&desc).expect("collection");
        for (chunk, c) in desc.chunks.iter().enumerate() {
            let d = [0, 1, 2].map(|a| [c.x, c.y, c.z][a] - actor.pos_m[a]);
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= STRIKE_REACH_M * STRIKE_REACH_M {
                let gate = c.mass * field.break_threshold();
                field
                    .apply_impact(chunk, gate + blow, BREAK_TICK)
                    .expect("impact");
            }
        }
        field.step(1.0 / 60.0);
        let events = field.drain_events();
        assert!(freed_count(&events) > 0, "a blow of {blow} frees nothing");
        WallBreak {
            actor,
            desc,
            events,
        }
    }

    fn wall_look(tier: Tier, wall: &WallBreak) -> BreakLook {
        let prop = BreakProp::new(
            0,
            kit_mesh_key(&wall.actor),
            Material::Stone,
            &wall.desc.chunks,
            &wall.desc.bonds,
            &wall.desc.anchors,
        )
        .expect("wall prop");
        BreakLook::new(tier, prop).expect("wall look")
    }

    /// Observes ticks `0..=until`, the wall's events on `BREAK_TICK` and nothing else.
    fn play(look: &mut BreakLook, wall: &WallBreak, until: u32) {
        for tick in 0..=until {
            let events: &[BreakEvent] = if tick == BREAK_TICK {
                &wall.events
            } else {
                &[]
            };
            look.observe(tick, events).expect("tick observed");
        }
    }

    fn freed_count(events: &[BreakEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .count()
    }

    /// F2P: the intact mesh is on screen until the break tick and gone on it, with every chunk
    /// drawn in its place.
    #[test]
    fn the_intact_mesh_is_hidden_on_the_break_tick() {
        let wall = wall_break(6.0);
        let intact = kit_mesh_key(&wall.actor);
        let mut look = wall_look(Tier::Comfort, &wall);
        for tick in 0..BREAK_TICK {
            look.observe(tick, &[]).expect("quiet tick");
            assert_eq!(
                look.frame(1.0).expect("frame"),
                Frame::default(),
                "tick {tick}: the wall still draws itself"
            );
        }
        look.observe(BREAK_TICK, &wall.events).expect("break tick");
        let frame = look.frame(0.0).expect("frame");
        assert_eq!(frame.hidden, vec![intact], "gone on the break tick itself");
        assert_eq!(frame.draws.len(), wall.desc.chunks.len());
        let keys: Vec<MeshKey> = look
            .meshes()
            .iter()
            .flat_map(|m| [m.standing.key, m.flying.key])
            .collect();
        assert!(frame.draws.iter().all(|d| keys.contains(&d.mesh)));
        assert!(frame.draws.iter().all(|d| d.mesh != intact));
        look.observe(BREAK_TICK + 1, &[]).expect("next tick");
        assert_eq!(look.frame(1.0).expect("frame").hidden, vec![intact]);
    }

    /// Debris never passes the tier's body table: 200 freed chunks on the weakest tier leave 96
    /// bodies, the first ones in the caller's order, and the picture draws exactly those.
    #[test]
    fn debris_count_stays_under_the_tier_cap() {
        // A 20 x 10 field of cubes with room around each; every row hangs 0.3 m higher than the
        // one before, so all but the first row fall.
        let chunks: Vec<Chunk3> = (0..200)
            .map(|k| Chunk3 {
                x: f32::from(u8::try_from(k % 20).expect("small")) * 0.4,
                y: 0.1 + f32::from(u8::try_from(k / 20).expect("small")) * 0.3,
                z: f32::from(u8::try_from(k / 20).expect("small")) * 0.4,
                hx: 0.1,
                hy: 0.1,
                hz: 0.1,
                mass: 0.008,
            })
            .collect();
        let intact = MeshKey::new(9, [9; 32]);
        let prop = BreakProp::new(2, intact, Material::Stone, &chunks, &[], &[]).expect("prop");
        let mut look = BreakLook::new(Tier::Potato, prop).expect("look");
        let cap = Tier::Potato.caps().bodies;
        let all: Vec<BreakEvent> = (0..200)
            .map(|chunk| BreakEvent::ChunkFreed {
                tick: BREAK_TICK,
                chunk,
            })
            .collect();
        look.observe(BREAK_TICK, &all).expect("break tick");
        assert_eq!(look.stats().bodies, cap, "the table is full, not over");
        let frame = look.frame(1.0).expect("frame");
        let kept: Vec<MeshKey> = look.meshes()[..cap].iter().map(|m| m.flying.key).collect();
        let drawn: Vec<MeshKey> = frame.draws.iter().map(|d| d.mesh).collect();
        assert_eq!(
            drawn, kept,
            "the first {cap} in the caller's order, no more"
        );
        for tick in BREAK_TICK + 1..BREAK_TICK + SIX_SECONDS {
            look.observe(tick, &[]).expect("tick");
            assert!(look.stats().bodies <= cap);
        }
        let settled = look.frame(1.0).expect("frame");
        assert_eq!(settled.draws.len(), look.stats().bodies);
        assert_eq!(look.stats().awake, 0, "the pile sleeps");
    }

    /// Every debris body sleeps within six seconds of sim time, whatever the fracture pattern,
    /// and the pieces land above the ground, moved by the blow.
    #[test]
    fn debris_sleeps_within_six_seconds() {
        for seed in [7, 8, 11, 13] {
            let ticks = settle(&wall_break_seeded(6.0, seed));
            println!("break_look wall seed {seed}: asleep after {ticks} ticks");
        }
    }

    /// Plays the wall's break and returns the ticks after the break tick until nothing is awake.
    fn settle(wall: &WallBreak) -> u32 {
        let mut look = wall_look(Tier::Comfort, wall);
        play(&mut look, wall, BREAK_TICK);
        let bodies = look.stats().bodies;
        assert_eq!(
            bodies,
            freed_count(&wall.events),
            "one body per freed chunk"
        );
        assert!(bodies <= Tier::Comfort.caps().bodies);
        assert!(look.stats().awake > 0, "fresh debris is awake");
        let rest = look.frame(1.0).expect("frame");
        let mut asleep_at = None;
        for tick in BREAK_TICK + 1..=BREAK_TICK + SIX_SECONDS {
            look.observe(tick, &[]).expect("tick");
            if look.stats().awake == 0 {
                asleep_at = Some(tick - BREAK_TICK);
                break;
            }
        }
        let ticks = asleep_at.expect("every body sleeps within 6 s");
        let end = look.frame(1.0).expect("frame");
        assert_eq!(end.draws.len(), rest.draws.len(), "nothing was lost");
        let floor = wall
            .desc
            .chunks
            .iter()
            .map(|c| c.y - c.hy)
            .fold(f32::MAX, f32::min);
        assert!(
            end.draws.iter().all(|d| d.at.pos[1] > floor),
            "above the ground"
        );
        let moved = end
            .draws
            .iter()
            .zip(&rest.draws)
            .filter(|(a, b)| {
                let d = [0, 1, 2].map(|k| a.at.pos[k] - b.at.pos[k]);
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 0.04
            })
            .count();
        assert!(moved > 0, "the pieces fell or flew from where they stood");
        let slept = look.frame(0.3).expect("frame");
        assert_eq!(slept, end, "asleep means the same at every alpha");
        ticks
    }

    /// The break makes one sound: at a chunk, once, louder for a harder blow, and never before
    /// the break tick or on a tick that only frees chunks (a collapse: the snapped bond already
    /// sounded the tick before).
    #[test]
    fn the_break_makes_one_sound_at_the_struck_chunk() {
        let wall = wall_break(6.0);
        let mut look = wall_look(Tier::Comfort, &wall);
        for tick in 0..BREAK_TICK {
            look.observe(tick, &[]).expect("quiet tick");
            assert_eq!(look.take_cue(), None, "tick {tick}: silent until the break");
        }
        look.observe(BREAK_TICK, &wall.events).expect("break tick");
        let cue = look.take_cue().expect("the break sounds");
        assert_eq!(cue.name, cue_names::IMPACT_STONE);
        assert!(cue.gain > 0.0 && cue.gain <= 1.0, "{cue:?}");
        assert!(
            wall.desc.chunks.iter().any(|c| [c.x, c.y, c.z] == cue.at_m),
            "the sound sits on a chunk: {cue:?}"
        );
        assert_eq!(look.take_cue(), None, "one sound per break tick");
        look.observe(BREAK_TICK + 1, &[]).expect("tick");
        assert_eq!(look.take_cue(), None, "a quiet tick is silent");

        // A harder blow never sounds quieter, and the gain never passes a crash.
        let harder = wall_break(12.0);
        let mut loud = wall_look(Tier::Comfort, &harder);
        play(&mut loud, &harder, BREAK_TICK);
        let loud = loud.take_cue().expect("the harder break sounds").gain;
        assert!(loud >= cue.gain, "{loud} vs {}", cue.gain);
        assert!(break_gain(0.0) < break_gain(6.0));
        assert!((break_gain(MAX_KICK_MS * 4.0) - 1.0).abs() < f32::EPSILON);

        // Chunks freed with no snapped bond (a collapse) make no sound of their own.
        let freed: Vec<BreakEvent> = wall
            .events
            .iter()
            .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .copied()
            .collect();
        assert!(!freed.is_empty(), "the wall frees chunks");
        let mut collapse = wall_look(Tier::Comfort, &wall);
        collapse.observe(BREAK_TICK, &freed).expect("collapse tick");
        assert_eq!(collapse.take_cue(), None, "no bond, no blow");
    }

    /// The break kicks up dust: at a chunk, once, more puffs for a harder blow, and never on a
    /// tick that frees nothing. A collapse with no snapped bond still puffs, at the base count.
    #[test]
    fn the_break_kicks_up_dust_at_the_struck_chunk() {
        let wall = wall_break(6.0);
        let mut look = wall_look(Tier::Comfort, &wall);
        for tick in 0..BREAK_TICK {
            look.observe(tick, &[]).expect("quiet tick");
            assert_eq!(
                look.take_dust(),
                None,
                "tick {tick}: no dust before the break"
            );
        }
        look.observe(BREAK_TICK, &wall.events).expect("break tick");
        let dust = look.take_dust().expect("the break raises dust");
        assert!(
            dust.puffs > 0 && dust.puffs <= MAX_PUFFS_PER_BREAK,
            "{dust:?}"
        );
        assert!(
            wall.desc
                .chunks
                .iter()
                .any(|c| [c.x, c.y, c.z] == dust.at_m),
            "the cloud sits on a chunk: {dust:?}"
        );
        assert!(
            (MIN_DUST_RADIUS_M..=MAX_DUST_RADIUS_M).contains(&dust.radius_m),
            "{dust:?}"
        );
        assert_eq!(look.take_dust(), None, "one cloud per break tick");
        look.observe(BREAK_TICK + 1, &[]).expect("tick");
        assert_eq!(look.take_dust(), None, "a quiet tick raises nothing");

        // A harder blow never raises fewer puffs.
        let harder = wall_break(12.0);
        let mut loud = wall_look(Tier::Comfort, &harder);
        play(&mut loud, &harder, BREAK_TICK);
        assert!(loud.take_dust().expect("the harder break puffs").puffs >= dust.puffs);

        // Chunks freed with no snapped bond (a collapse) still raise the base cloud.
        let freed: Vec<BreakEvent> = wall
            .events
            .iter()
            .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .copied()
            .collect();
        let mut collapse = wall_look(Tier::Comfort, &wall);
        collapse.observe(BREAK_TICK, &freed).expect("collapse tick");
        let collapse = collapse.take_dust().expect("a collapse raises dust too");
        assert!(
            collapse.puffs > 0 && collapse.puffs <= dust.puffs,
            "{collapse:?}"
        );

        // The puff count: two per piece, doubled at the cap, never over the budget.
        assert_eq!(dust_puffs(0, 0.0), 0, "no pieces, no dust");
        assert_eq!(dust_puffs(3, 0.0), 6, "a collapse is the base cloud");
        assert_eq!(dust_puffs(3, MAX_KICK_MS), 12, "a full blow doubles it");
        assert_eq!(dust_puffs(3, f32::NAN), 6, "a bad speed is no blow");
        let many = usize::try_from(MAX_PUFFS_PER_BREAK).expect("the budget fits a usize");
        assert_eq!(
            dust_puffs(many, MAX_KICK_MS),
            MAX_PUFFS_PER_BREAK,
            "the per-event budget holds"
        );
    }

    /// The sound an authority blast makes: the stone crash at its center, half volume at half the
    /// physical layer's momentum cap, full at it, and never silent.
    #[test]
    fn a_blast_sounds_at_its_center_and_rises_with_its_momentum() {
        let blast = Blast {
            center_mm: [1_500, 250, -2_000],
            radius_mm: 5_000,
            impulse_ns: MAX_BLAST_NS / 2,
        };
        let cue = blast_cue(blast);
        assert_eq!(cue.name, cue_names::IMPACT_STONE, "the bank's stone crash");
        assert_eq!(cue.at_m, [1.5, 0.25, -2.0], "the blast's center, in meters");
        assert!((cue.gain - 0.5).abs() < f32::EPSILON, "{cue:?}");
        assert_eq!(
            blast_cue(Blast {
                impulse_ns: 0,
                ..blast
            })
            .gain,
            MIN_BLAST_GAIN,
            "a blast is never a rustle"
        );
        assert_eq!(
            blast_cue(Blast {
                impulse_ns: MAX_BLAST_NS,
                ..blast
            })
            .gain,
            1.0
        );
        assert_eq!(
            blast_cue(Blast {
                impulse_ns: u32::MAX,
                ..blast
            })
            .gain,
            1.0,
            "past the cap, full and no louder"
        );
        assert!(
            blast_cue(Blast {
                impulse_ns: MAX_BLAST_NS * 3 / 4,
                ..blast
            })
            .gain
                >= cue.gain,
            "a bigger blast never sounds quieter"
        );
    }

    /// The dust a blast throws: a cloud at the center as wide as its reach, puffs rising with the
    /// momentum and capped, and the same integers giving the same cloud twice.
    #[test]
    fn a_blast_throws_dust_as_wide_as_its_reach_and_thicker_with_its_momentum() {
        let blast = Blast {
            center_mm: [1_500, 250, -2_000],
            radius_mm: 5_000,
            impulse_ns: MAX_BLAST_NS,
        };
        let dust = blast_dust(blast);
        assert_eq!(dust.at_m, [1.5, 0.25, -2.0]);
        assert_eq!(dust.radius_m, 5.0, "the cloud is the blast's reach");
        assert_eq!(dust.puffs, MAX_BLAST_PUFFS);

        let small = blast_dust(Blast {
            radius_mm: 1_000,
            impulse_ns: 0,
            ..blast
        });
        assert_eq!(small.puffs, BASE_BLAST_PUFFS, "a weak blast still puffs");
        assert_eq!(small.radius_m, 1.0);
        assert!(small.puffs < dust.puffs);
        assert!(small.radius_m < dust.radius_m);

        let point = blast_dust(Blast {
            radius_mm: 0,
            ..blast
        });
        assert_eq!(
            point.radius_m, MIN_BLAST_DUST_RADIUS_M,
            "a point blast keeps a cloud"
        );
        let huge = blast_dust(Blast {
            radius_mm: 100_000,
            impulse_ns: u32::MAX,
            ..blast
        });
        assert_eq!(huge.radius_m, MAX_BLAST_DUST_RADIUS_M, "no fog bank");
        assert_eq!(huge.puffs, MAX_BLAST_PUFFS);

        assert_eq!(blast_dust(blast), dust, "same integers, same cloud");
        assert_eq!(
            blast_cue(blast),
            blast_cue(blast),
            "same integers, same sound"
        );
    }

    /// A blast shoves the debris of a broken prop (never more bodies than it has) and moves
    /// nothing before the break, when the chunks are still the authority's standing pieces.
    #[test]
    fn a_blast_shoves_debris_and_not_a_standing_prop() {
        let wall = wall_break(6.0);
        let blast = Blast {
            center_mm: [0, 1_500, 0],
            radius_mm: 50_000,
            impulse_ns: MAX_BLAST_NS,
        };
        let mut whole = wall_look(Tier::Comfort, &wall);
        assert_eq!(
            whole.apply_blast(blast).expect("a blast on a whole prop"),
            0,
            "standing chunks are not bodies of the physical layer"
        );

        let mut look = wall_look(Tier::Comfort, &wall);
        play(&mut look, &wall, BREAK_TICK);
        let bodies = look.stats().bodies;
        assert!(bodies > 0, "the break made debris");
        let pushed = look.apply_blast(blast).expect("the blast shoves");
        assert!(pushed > 0, "some debris is inside 50 m");
        assert!(
            pushed <= bodies,
            "{pushed} pushed of {bodies} bodies, never more"
        );
    }

    /// The same events on the same ticks give the same transforms; another blow does not.
    #[test]
    fn the_same_events_give_the_same_transforms_twice() {
        let run = |blow: f32| {
            let wall = wall_break(blow);
            let mut look = wall_look(Tier::Comfort, &wall);
            let mut seen = Vec::new();
            for tick in 0..=BREAK_TICK + 40 {
                let events: &[BreakEvent] = if tick == BREAK_TICK {
                    &wall.events
                } else {
                    &[]
                };
                look.observe(tick, events).expect("tick");
                for alpha in [0.0, 0.5, 1.0] {
                    seen.push(look.frame(alpha).expect("frame"));
                }
            }
            seen
        };
        let first = run(6.0);
        assert!(first == run(6.0), "the same events replay x2");
        let harder = run(12.0);
        assert!(
            first != harder,
            "a harder blow throws the pieces differently"
        );
    }

    fn input(tick: u32, fighter: u8, buttons: Buttons) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId(fighter),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons,
        }
    }

    /// The crate the arena builds, as the picture's prop (its center, half and piece count).
    fn crate_look() -> BreakLook {
        let seed = engine_arena::seed::expand(SEED, 2 * 2 + 1)
            .last()
            .copied()
            .expect("stream");
        let crate_prop =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, seed).expect("crate bakes");
        let prop = BreakProp::new(
            1,
            MeshKey::new(1, [0xA1; 32]),
            Material::Wood,
            crate_prop.chunks(),
            crate_prop.bonds(),
            &crate_prop.anchors(),
        )
        .expect("crate prop");
        BreakLook::new(Tier::Comfort, prop).expect("crate look")
    }

    /// Fighter 0 swings on tick 0 and lands on tick 4; fighter 1 idles. Returns every tick's
    /// checkpoint hash, feeding each tick's break events to `look` when there is one.
    fn scripted_match(mut look: Option<&mut BreakLook>) -> Vec<u64> {
        let config = ArenaConfig::try_new(SEED, 32, 2, 20.0).expect("config");
        let mut live = LiveMatch::with_rules(&config, ArenaRules::V1).expect("match builds");
        let mut hashes = Vec::new();
        while live.tick() < config.ticks {
            let tick = live.tick();
            let mut inputs = Vec::new();
            if tick == 0 {
                inputs.push(input(tick, 0, Buttons(ATTACK)));
            }
            inputs.push(input(tick, 1, Buttons::NONE));
            let stepped = live.step(&inputs).expect("tick steps");
            if let Some(look) = look.as_mut() {
                look.observe(tick, &stepped.breaks.events)
                    .expect("the match's events fit the crate");
            }
            hashes.push(stepped.checkpoint.hash);
        }
        hashes
    }

    /// The window picture never feeds back: the same match with the look watching it gives the
    /// same hash on every tick, and the look really drew the crate break.
    #[test]
    fn the_sim_hash_is_equal_with_the_look_on_and_off() {
        let off = scripted_match(None);
        assert_eq!(off, scripted_match(None), "the match replays");
        let mut look = crate_look();
        let on = scripted_match(Some(&mut look));
        assert_eq!(on, off, "observing changed the sim");
        let frame = look.frame(1.0).expect("frame");
        assert_eq!(frame.hidden, vec![MeshKey::new(1, [0xA1; 32])]);
        assert!(look.stats().bodies > 0, "the sword hit broke the crate");
        assert!(!frame.draws.is_empty());
    }

    /// A refused call changes nothing, and the same tick can be tried again.
    #[test]
    fn refused_events_and_ticks_leave_the_look_as_it_was() {
        let wall = wall_break(6.0);
        let mut look = wall_look(Tier::Comfort, &wall);
        let ghost_chunk = [BreakEvent::ChunkFreed {
            tick: 1,
            chunk: 9_999,
        }];
        assert_eq!(
            look.observe(1, &ghost_chunk),
            Err(LookError::UnknownChunk(9_999))
        );
        let ghost_bond = [BreakEvent::BondBroken {
            tick: 1,
            bond: 9_999,
            strain: 4.0,
        }];
        assert_eq!(
            look.observe(1, &ghost_bond),
            Err(LookError::UnknownBond(9_999))
        );
        assert_eq!(look.frame(1.0).expect("frame"), Frame::default());
        assert_eq!(look.stats().bodies, 0);
        // The refusals did not use up tick 1.
        look.observe(1, &wall.events).expect("tick 1 still free");
        let broken = look.frame(1.0).expect("frame");
        assert!(!broken.hidden.is_empty());
        let stale = look.observe(1, &wall.events);
        assert_eq!(stale, Err(LookError::StaleTick { last: 1, got: 1 }));
        assert_eq!(
            look.observe(0, &[]),
            Err(LookError::StaleTick { last: 1, got: 0 })
        );
        assert_eq!(
            look.stats().bodies,
            freed_count(&wall.events),
            "no debris twice"
        );
        assert!(stale.expect_err("stale").to_string().contains("tick 1"));
        assert_eq!(
            look.frame(f32::NAN).unwrap_err(),
            LookError::Fx(FxError::NonFinite(engine_physfx::Field::Alpha))
        );
    }

    /// A chunk freed twice (a replayed event on a later tick) is one body.
    #[test]
    fn a_chunk_freed_again_stays_one_body() {
        let wall = wall_break(6.0);
        let mut look = wall_look(Tier::Comfort, &wall);
        play(&mut look, &wall, BREAK_TICK);
        let bodies = look.stats().bodies;
        let again: Vec<BreakEvent> = wall
            .events
            .iter()
            .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .copied()
            .collect();
        look.observe(BREAK_TICK + 1, &again)
            .expect("replayed events");
        assert_eq!(look.stats().bodies, bodies);
    }

    #[test]
    fn a_bad_prop_is_refused() {
        let key = MeshKey::new(3, [3; 32]);
        let chunk = Chunk3 {
            x: 0.0,
            y: 0.5,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        };
        let build = |chunks: &[Chunk3], bonds: &[Bond], anchors: &[u32]| {
            BreakProp::new(0, key, Material::Wood, chunks, bonds, anchors).map(|_| ())
        };
        assert_eq!(build(&[], &[], &[]), Err(LookError::EmptyProp));
        let many = vec![chunk; MAX_BONDED_CHUNKS + 1];
        assert_eq!(build(&many, &[], &[]), Err(LookError::TooManyChunks));
        let nan = Chunk3 {
            x: f32::NAN,
            ..chunk
        };
        assert_eq!(build(&[chunk, nan], &[], &[]), Err(LookError::BadChunk(1)));
        let flat = Chunk3 { hy: 0.0, ..chunk };
        assert_eq!(build(&[flat], &[], &[]), Err(LookError::BadChunk(0)));
        let huge = Chunk3 { hx: 6.0, ..chunk };
        assert_eq!(build(&[huge], &[], &[]), Err(LookError::BadChunk(0)));
        let bond = Bond {
            a: 0,
            b: 7,
            breaking_strain: 1.0,
        };
        assert_eq!(build(&[chunk], &[bond], &[]), Err(LookError::BadBond(0)));
        assert_eq!(build(&[chunk], &[], &[0, 1]), Err(LookError::BadAnchor(1)));
        assert_eq!(
            build(&[chunk], &[], &[0, 0]),
            Ok(()),
            "a repeated anchor is one anchor"
        );
    }

    /// Two props never share a piece key, and a chunk's key follows its size.
    #[test]
    fn piece_keys_are_unique_per_prop_and_chunk() {
        let wall = wall_break(6.0);
        let key = |prop: u16, chunks: &[Chunk3]| {
            BreakProp::new(
                prop,
                MeshKey::new(1, [1; 32]),
                Material::Stone,
                chunks,
                &[],
                &[],
            )
            .expect("prop")
            .pieces
            .iter()
            .flat_map(|p| [p.standing.key, p.flying.key])
            .collect::<Vec<_>>()
        };
        // Standing and flying box of every chunk, in chunk order.
        let a = key(0, &wall.desc.chunks);
        let b = key(1, &wall.desc.chunks);
        let mut all: Vec<MeshKey> = a.iter().chain(&b).copied().collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), a.len() + b.len());
        let mut wider = wall.desc.chunks.clone();
        wider[0].hy = 2.0;
        let c = key(0, &wider);
        for k in 0..2 {
            assert_eq!(c[k].id, a[k].id);
            assert_ne!(c[k].digest, a[k].digest, "a recooked chunk uploads again");
        }
        assert_eq!(c[2..], a[2..], "the other chunks keep their keys");
    }

    #[test]
    fn the_model_matrix_rotates_then_moves() {
        let draw = |rot: [f32; 4]| PieceDraw {
            mesh: MeshKey::new(1, [0; 32]),
            at: Transform {
                pos: [1.0, 2.0, 3.0],
                rot,
            },
        };
        let flat = draw([0.0, 0.0, 0.0, 1.0]).model_3x4();
        assert_eq!(
            flat,
            [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0]
        );
        // A quarter turn about +Y sends +X to -Z.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let turned = draw([0.0, s, 0.0, s]).model_3x4();
        let want = [0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 2.0, -1.0, 0.0, 0.0, 3.0];
        for (got, want) in turned.iter().zip(want) {
            assert!((got - want).abs() < 1e-6, "{turned:?}");
        }
    }
}
