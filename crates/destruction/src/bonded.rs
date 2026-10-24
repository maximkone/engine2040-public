//! bonded: pre-fractured collections with bonds, strain and anchors.
//!
//! Consumer: the arena match owns one crate prop, steps it each fixed tick
//! and maps sword hits to impacts; break events drain as typed [`BreakEvent`]
//! rows for the server log. Destruction never imports arena.
//! Units: meters, ticks; impulses are shard units. Determinism: index-order
//! loops with `+ - * /` only, floats hashed as bits. Errors: [`BreakError`].

use crate::bake::BREAKING_THRESHOLD;
use crate::bake3d::fracture3;
use crate::shards3d::in_contact;
use engine_core::Fnv64;

pub use crate::bake3d::Chunk3;
pub use blast::MAX_BLAST_RADIUS_MM;
pub use sag::{SagRules, MAX_OVERLOAD_RATE, MAX_SAG_TICKS, SAG_Q16_ONE};

/// An authority explosion applied as impacts through the same bond rule.
mod blast;
/// Staged collapse: links sag under overload and unsupported clusters hang
/// a few ticks before they fall. Opt-in; off by default.
mod sag;
/// Canonical snapshot bytes and restore of the moving state (rollback resim).
mod snapshot;

/// Largest chunk count one collection holds; bounds memory up front.
pub const MAX_BONDED_CHUNKS: usize = 1024;
/// Largest bond count; bounds the support search up front.
pub const MAX_BONDS: usize = 4096;
/// Largest piece count the crate helper accepts.
pub const MAX_CRATE_PIECES: usize = 64;
/// Default per-bond strain limit; a sword strike lands well above it.
pub const DEFAULT_BOND_STRAIN: f32 = 1.0;
/// Elastic band as a fraction of the fatal gate: a blow at or below
/// `mass * threshold * ELASTIC_BAND` is fully elastic and adds no damage.
pub const ELASTIC_BAND: f32 = 0.5;
/// Most of a bond's limit a single blow below the fatal gate may chip, at the
/// gate itself; scaled by the excess over the elastic limit. A sub-fatal blow
/// never snaps a bond alone, so only a repeat wears it through.
pub const ELASTIC_CHIP: f32 = 0.5;
/// Accepted per-bond strain limit range; (0, `MAX_BOND_STRAIN`].
pub const MAX_BOND_STRAIN: f32 = 1e6;
/// Accepted fracture-threshold range; (0, `MAX_BREAK_THRESHOLD`].
pub const MAX_BREAK_THRESHOLD: f32 = 1e6;
/// Upper bound for `step` dt; larger or non-positive steps are ignored.
pub const MAX_STEP_DT: f32 = 1.0;
/// Gravity for freed chunks; m/s^2.
const GRAVITY_MPS2: f32 = 9.8;
/// Largest accepted chunk center coordinate; meters.
const MAX_CENTER_M: f32 = 10_000.0;
/// Accepted half-extent range; meters.
const MIN_HALF_EXTENT_M: f32 = 1e-4;
const MAX_HALF_EXTENT_M: f32 = 10_000.0;
/// Accepted chunk mass range; mass units.
pub(crate) const MIN_MASS: f32 = 1e-6;
const MAX_MASS: f32 = 1e12;
/// Bottom-layer tolerance when the crate helper picks anchors; meters.
const ANCHOR_EPS_M: f32 = 1e-3;
/// Parent slot of an anchor or an unsupported chunk in the load-leg.
pub const NO_PARENT: u32 = u32::MAX;
/// Layer of a chunk the support flood never reached.
pub const NO_LAYER: u32 = u32::MAX;
/// One whole mass unit in the Q16 load-leg (`1.0 == 65536`).
pub const LOAD_Q16_ONE: u64 = 65_536;

/// One bond between two chunk indices with its own strain limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bond {
    /// First chunk index.
    pub a: u32,
    /// Second chunk index, never equal to `a`.
    pub b: u32,
    /// Strain above which the bond snaps; non-negative and finite.
    pub breaking_strain: f32,
}

/// Construction descriptor: shipped chunks plus their bond graph, anchors
/// and the impulse-per-mass fracture gate. Fracture happens only through
/// impacts and strain; construction leaves every chunk supported and asleep.
#[derive(Clone, Debug)]
pub struct BondedDesc {
    /// Shipped chunk boxes; 1..=`MAX_BONDED_CHUNKS`, finite, well-placed.
    pub chunks: Vec<Chunk3>,
    /// Bonds between chunk indices; at most `MAX_BONDS`, no self-bonds.
    pub bonds: Vec<Bond>,
    /// Anchored chunk indices; fixed in place, never freed, never integrated.
    pub anchors: Vec<u32>,
    /// Impulse per unit mass above which a hit strains bonds; positive.
    pub break_threshold: f32,
}

impl BondedDesc {
    /// Checks chunks, bonds, anchors and the threshold without building.
    pub fn validate(&self) -> Result<(), BreakError> {
        check_chunks(&self.chunks)?;
        if self.bonds.len() > MAX_BONDS {
            return Err(BreakError::TooManyBonds);
        }
        let n = self.chunks.len();
        for (i, bond) in self.bonds.iter().enumerate() {
            let (a, b) = (bond.a as usize, bond.b as usize);
            let ends_ok = u64::from(bond.a) < n as u64 && u64::from(bond.b) < n as u64;
            if !ends_ok || a == b {
                return Err(BreakError::BadBond(i));
            }
            if !bond.breaking_strain.is_finite()
                || bond.breaking_strain < 0.0
                || bond.breaking_strain > MAX_BOND_STRAIN
            {
                return Err(BreakError::BadBond(i));
            }
        }
        let mut pairs: Vec<(u32, u32, usize)> = self
            .bonds
            .iter()
            .enumerate()
            .map(|(i, bond)| (bond.a.min(bond.b), bond.a.max(bond.b), i))
            .collect();
        pairs.sort_unstable();
        for window in pairs.windows(2) {
            if window[0].0 == window[1].0 && window[0].1 == window[1].1 {
                return Err(BreakError::BadBond(window[1].2));
            }
        }
        for anchor in &self.anchors {
            if u64::from(*anchor) >= n as u64 {
                return Err(BreakError::BadChunk(*anchor as usize));
            }
        }
        if !self.break_threshold.is_finite()
            || self.break_threshold <= 0.0
            || self.break_threshold > MAX_BREAK_THRESHOLD
        {
            return Err(BreakError::BadThreshold);
        }
        Ok(())
    }
}

/// Typed break event: bond snaps and chunk frees are separate variants so
/// the server log reads without re-deriving causes. Emitted in index order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BreakEvent {
    /// Bond `bond` snapped at `tick` once strain passed its limit.
    BondBroken {
        /// Match tick of the snap.
        tick: u32,
        /// Bond index in the descriptor bond list.
        bond: u32,
        /// Strain that snapped it.
        strain: f32,
    },
    /// Chunk `chunk` lost its last path to an anchor at `tick` and falls.
    ChunkFreed {
        /// Match tick of the free.
        tick: u32,
        /// Chunk index in the chunk list.
        chunk: u32,
    },
}

/// Canonical 13-byte form: tag plus tick plus id plus payload (strain bits
/// for bonds, zero for freed chunks). The arena folds these into its hash.
pub fn event_bytes(event: &BreakEvent) -> [u8; 13] {
    let mut out = [0u8; 13];
    match *event {
        BreakEvent::BondBroken { tick, bond, strain } => {
            out[0] = 0;
            out[1..5].copy_from_slice(&tick.to_le_bytes());
            out[5..9].copy_from_slice(&bond.to_le_bytes());
            out[9..13].copy_from_slice(&strain.to_bits().to_le_bytes());
        }
        BreakEvent::ChunkFreed { tick, chunk } => {
            out[0] = 1;
            out[1..5].copy_from_slice(&tick.to_le_bytes());
            out[5..9].copy_from_slice(&chunk.to_le_bytes());
        }
    }
    out
}

/// Typed validation failures; external input maps here, never to a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakError {
    /// No chunks supplied.
    EmptyChunks,
    /// More than `MAX_BONDED_CHUNKS` chunks supplied.
    TooManyChunks,
    /// More than `MAX_BONDS` bonds supplied.
    TooManyBonds,
    /// Chunk or anchor index out of range, culprit carried.
    BadChunk(usize),
    /// Bond endpoint, limit or duplicate at the given bond index.
    BadBond(usize),
    /// Threshold non-finite, non-positive or absurd.
    BadThreshold,
    /// Strain amount non-finite or negative.
    BadStrain,
    /// Impact impulse non-finite or negative.
    BadImpulse,
    /// Blast radius zero or past `MAX_BLAST_RADIUS_MM`, or a center outside the world.
    BadBlast,
    /// Sag rules with a stage past `MAX_SAG_TICKS`, or the link rule on with no stress ticks.
    BadSag,
    /// Snapshot bytes are truncated, versioned wrong, of another shape,
    /// non-finite or trailing.
    BadSnapshot,
}

impl core::fmt::Display for BreakError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BreakError::EmptyChunks => write!(f, "bonded collection needs chunks"),
            BreakError::TooManyChunks => write!(f, "bonded collection exceeds MAX_BONDED_CHUNKS"),
            BreakError::TooManyBonds => write!(f, "bonded collection exceeds MAX_BONDS"),
            BreakError::BadChunk(i) => write!(f, "bonded chunk {i} is out of bounds"),
            BreakError::BadBond(i) => write!(f, "bonded bond {i} is out of bounds"),
            BreakError::BadThreshold => write!(f, "fracture threshold is out of bounds"),
            BreakError::BadStrain => write!(f, "strain amount is out of bounds"),
            BreakError::BadImpulse => write!(f, "impact impulse is out of bounds"),
            BreakError::BadBlast => write!(f, "blast radius or center is out of bounds"),
            BreakError::BadSag => write!(f, "sag rules are out of bounds"),
            BreakError::BadSnapshot => write!(f, "bonded snapshot bytes are not valid"),
        }
    }
}

impl std::error::Error for BreakError {}

/// Runtime bonded collection: chunk poses plus velocities, the bond graph
/// with per-bond strain, anchor flags and the pending break log. Chunks
/// start supported and asleep; islands without an anchor path free on the
/// next step and fall under gravity with a ground clamp. Budget: `step`
/// performs no heap allocation in steady state after warmup; the support
/// map and work stack are pre-sized scratch reused every refresh, and
/// break events drain into a caller-owned buffer.
///
/// The support refresh is also the load-leg: every supported chunk gets a
/// parent in the layer above it and a Q16 load (its own mass plus all that
/// hangs below it in that tree), so [`Self::weakest_link`] and the
/// structure analysis read right after any step or break.
#[derive(Clone, Debug)]
pub struct BondedCollection {
    chunks: Vec<Chunk3>,
    vel: Vec<[f32; 3]>,
    bonds: Vec<Bond>,
    strain: Vec<f32>,
    bond_broken: Vec<bool>,
    dynamic: Vec<bool>,
    freed: Vec<bool>,
    anchored: Vec<bool>,
    threshold: f32,
    tick: u32,
    log: Vec<BreakEvent>,
    support_scratch: Vec<bool>,
    work_scratch: Vec<usize>,
    /// Adjacency offsets: chunk `i` owns `adj[adj_start[i]..adj_start[i + 1]]`.
    adj_start: Vec<u32>,
    /// `(neighbor, bond)` pairs sorted ascending per chunk; built once.
    adj: Vec<(u32, u32)>,
    /// Q16 mass per chunk, rounded once at construction; the load unit.
    mass_q16: Vec<u64>,
    /// Load-leg output: own mass plus every chunk hanging below in the
    /// support tree, Q16; refreshed with support.
    load_q16: Vec<u64>,
    /// Support-tree parent per chunk, `NO_PARENT` for anchors and the
    /// unsupported.
    parent: Vec<u32>,
    /// Support-tree depth per chunk, `NO_LAYER` for the unsupported.
    layer: Vec<u32>,
    /// Capacity factor in Q16: a supported, unanchored chunk whose load
    /// exceeds `mass * capacity` fails on the next step. Zero turns the
    /// rule off (the default), so plain props never fail under load.
    capacity_q16: u64,
    /// Chunks failing in the current wave; pre-sized scratch.
    wave_scratch: Vec<usize>,
    /// Staged-collapse state; `None` (the default) keeps the plain step.
    sag: Option<Box<sag::SagState>>,
}

impl BondedCollection {
    /// Builds a supported, asleep collection from a validated descriptor.
    /// Nothing is freed yet: unsupported islands report on the first step.
    /// Loads and parents are already computed, so analysis reads at once.
    pub fn new(desc: &BondedDesc) -> Result<Self, BreakError> {
        desc.validate()?;
        let n = desc.chunks.len();
        let mut anchored = vec![false; n];
        for anchor in &desc.anchors {
            anchored[slot(*anchor)] = true;
        }
        let (adj_start, adj) = build_adjacency(n, &desc.bonds);
        let mass_q16: Vec<u64> = desc.chunks.iter().map(|c| mass_to_q16(c.mass)).collect();
        let mut field = Self {
            chunks: desc.chunks.clone(),
            vel: vec![[0.0f32; 3]; n],
            bonds: desc.bonds.clone(),
            strain: vec![0.0; desc.bonds.len()],
            bond_broken: vec![false; desc.bonds.len()],
            dynamic: vec![false; n],
            freed: vec![false; n],
            anchored,
            threshold: desc.break_threshold,
            tick: 0,
            log: Vec::new(),
            // Pre-sized once: chunk count never changes mid-run, so every
            // refresh below reuses this capacity with no steady-state alloc.
            support_scratch: vec![false; n],
            work_scratch: Vec::with_capacity(n),
            adj_start,
            adj,
            load_q16: mass_q16.clone(),
            mass_q16,
            parent: vec![NO_PARENT; n],
            layer: vec![NO_LAYER; n],
            capacity_q16: 0,
            wave_scratch: Vec::with_capacity(n),
            sag: None,
        };
        field.flood_loads();
        Ok(field)
    }

    /// Crate prop for the arena: `pieces` (1..=`MAX_CRATE_PIECES`) chunks
    /// baked from the `(x, y, z, half)` box with `seed`, contact bonds at
    /// [`DEFAULT_BOND_STRAIN`] and the bottom layer anchored so the crate
    /// sits until a sword hit detaches its top.
    pub fn crate_prop(
        x: f32,
        y: f32,
        z: f32,
        half: f32,
        pieces: usize,
        seed: u64,
    ) -> Result<Self, BreakError> {
        if pieces == 0 {
            return Err(BreakError::EmptyChunks);
        }
        if pieces > MAX_CRATE_PIECES {
            return Err(BreakError::TooManyChunks);
        }
        if !x.is_finite() || !y.is_finite() || !z.is_finite() || !half.is_finite() || half <= 0.0 {
            return Err(BreakError::BadChunk(0));
        }
        let chunks = fracture3(x, y, z, half, half, half, pieces, seed);
        let mut bonds = Vec::new();
        for a in 0..chunks.len() {
            for b in (a + 1)..chunks.len() {
                if in_contact(&chunks[a], &chunks[b]) {
                    bonds.push(Bond {
                        a: a as u32,
                        b: b as u32,
                        breaking_strain: DEFAULT_BOND_STRAIN,
                    });
                }
            }
        }
        let bottom = chunks
            .iter()
            .map(|c| c.y - c.hy)
            .reduce(f32::min)
            .unwrap_or(y - half);
        let mut anchors = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            if (c.y - c.hy) - bottom <= ANCHOR_EPS_M {
                anchors.push(i as u32);
            }
        }
        Self::new(&BondedDesc {
            chunks,
            bonds,
            anchors,
            break_threshold: BREAKING_THRESHOLD,
        })
    }

    /// Applies a hit to one chunk, in two bands like Blast's stress solver:
    /// below `mass * threshold * ELASTIC_BAND` the blow is fully elastic and
    /// changes nothing; between that elastic limit and `mass * threshold`
    /// every intact incident bond gains a chip scaled by the excess over the
    /// elastic limit, so a run of light blows cracks a prop the way one heavy
    /// blow breaks it; above the gate the full `impulse / mean-mass` strain
    /// lands and snaps past the limit. Support refreshes and detached islands
    /// free when any bond snaps. Events queue in the log.
    pub fn apply_impact(
        &mut self,
        chunk: usize,
        impulse: f32,
        tick: u32,
    ) -> Result<(), BreakError> {
        if chunk >= self.chunks.len() {
            return Err(BreakError::BadChunk(chunk));
        }
        if !impulse.is_finite() || impulse < 0.0 {
            return Err(BreakError::BadImpulse);
        }
        // Impulse is validated finite above, so `<=` also rejects NaN-free
        // elastic hits without a fallible comparison.
        let gate = self.chunks[chunk].mass * self.threshold;
        let elastic = gate * ELASTIC_BAND;
        if impulse <= elastic {
            return Ok(());
        }
        let fatal = impulse > gate;
        let mut broke_any = false;
        for bi in 0..self.bonds.len() {
            if self.bond_broken[bi] {
                continue;
            }
            let (a, b) = (self.bonds[bi].a as usize, self.bonds[bi].b as usize);
            if a != chunk && b != chunk {
                continue;
            }
            // Validated masses are positive, so the mean mass divides safely.
            let mean = (self.chunks[a].mass + self.chunks[b].mass) * 0.5;
            let gain = if fatal {
                impulse / mean
            } else {
                // The elastic limit is below the gate, so this span is positive.
                // Bit-exact `+ - * /` only: same bytes on every PC.
                let excess = (impulse - elastic) / (gate - elastic);
                self.bonds[bi].breaking_strain * ELASTIC_CHIP * excess
            };
            self.strain[bi] += gain;
            if self.strain[bi] > self.bonds[bi].breaking_strain {
                self.bond_broken[bi] = true;
                broke_any = true;
                self.log.push(BreakEvent::BondBroken {
                    tick,
                    bond: bi as u32,
                    strain: self.strain[bi],
                });
            }
        }
        if broke_any {
            self.refresh_support(tick);
        }
        Ok(())
    }

    /// Adds strain to one bond directly; snaps past its limit, then support
    /// refreshes. Strain on an already-snapped bond is a no-op.
    pub fn add_strain(&mut self, bond: usize, amount: f32, tick: u32) -> Result<(), BreakError> {
        if bond >= self.bonds.len() {
            return Err(BreakError::BadBond(bond));
        }
        if !amount.is_finite() || amount < 0.0 {
            return Err(BreakError::BadStrain);
        }
        if self.bond_broken[bond] {
            return Ok(());
        }
        self.strain[bond] += amount;
        if self.strain[bond] > self.bonds[bond].breaking_strain {
            self.bond_broken[bond] = true;
            self.log.push(BreakEvent::BondBroken {
                tick,
                bond: bond as u32,
                strain: self.strain[bond],
            });
            self.refresh_support(tick);
        }
        Ok(())
    }

    /// Advances one step: frees unsupported islands, then integrates freed
    /// chunks under gravity with a ground clamp. Anchored and supported
    /// chunks never move. Bad steps are ignored without panicking.
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 || dt > MAX_STEP_DT {
            return;
        }
        self.tick += 1;
        self.refresh_support(self.tick);
        if self.capacity_q16 != 0 {
            self.fail_overloaded(self.tick);
        }
        if self.sag.is_some() {
            self.advance_sag(self.tick);
        }
        let (chunks, vel) = (&mut self.chunks, &mut self.vel);
        for i in 0..chunks.len() {
            if !self.dynamic[i] || self.anchored[i] {
                continue;
            }
            let (c, v) = (&mut chunks[i], &mut vel[i]);
            v[1] -= GRAVITY_MPS2 * dt;
            c.x += v[0] * dt;
            c.y += v[1] * dt;
            c.z += v[2] * dt;
            if c.y < c.hy {
                c.y = c.hy;
                v[1] = 0.0;
            }
        }
    }

    /// Drains the pending break log in emission order; draining never moves
    /// the state hash, so consume-then-hash and hash-then-consume agree.
    pub fn drain_events(&mut self) -> Vec<BreakEvent> {
        std::mem::take(&mut self.log)
    }

    /// Appends pending break events into caller-owned `out` in emission
    /// order, keeping the collection's log capacity so repeated
    /// break/drain cycles allocate nothing in steady state. Existing `out`
    /// content is kept; draining never moves the state hash.
    pub fn drain_into(&mut self, out: &mut Vec<BreakEvent>) {
        out.append(&mut self.log);
    }

    /// FNV-1a64 over tick, threshold, chunk poses plus velocities, bonds
    /// plus strain plus snap flags, and support flags. Index order and
    /// bit-exact floats keep reruns identical.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_len32(self.chunks.len());
        h.write_u32(self.tick);
        h.write_u32(self.threshold.to_bits());
        for (c, v) in self.chunks.iter().zip(self.vel.iter()) {
            for w in [
                c.x.to_bits(),
                c.y.to_bits(),
                c.z.to_bits(),
                c.hx.to_bits(),
                c.hy.to_bits(),
                c.hz.to_bits(),
                c.mass.to_bits(),
                v[0].to_bits(),
                v[1].to_bits(),
                v[2].to_bits(),
            ] {
                h.write_u32(w);
            }
        }
        h.write_len32(self.bonds.len());
        for ((bond, strain), broken) in self
            .bonds
            .iter()
            .zip(self.strain.iter())
            .zip(self.bond_broken.iter())
        {
            h.write_u32(bond.a);
            h.write_u32(bond.b);
            h.write_u32(bond.breaking_strain.to_bits());
            h.write_u32(strain.to_bits());
            h.write_u32(u32::from(*broken));
        }
        for ((dynamic, freed), anchored) in self
            .dynamic
            .iter()
            .zip(self.freed.iter())
            .zip(self.anchored.iter())
        {
            h.write_u32(u32::from(*dynamic));
            h.write_u32(u32::from(*freed));
            h.write_u32(u32::from(*anchored));
        }
        h.finish()
    }

    /// Chunk count.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }
    /// True when the collection holds no chunks.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
    /// Collection tick; counts accepted steps only.
    pub fn tick(&self) -> u32 {
        self.tick
    }
    /// Impulse-per-mass fracture gate of this collection.
    pub fn break_threshold(&self) -> f32 {
        self.threshold
    }
    /// Mass of one chunk; `None` when out of range.
    pub fn chunk_mass(&self, chunk: usize) -> Option<f32> {
        self.chunks.get(chunk).map(|c| c.mass)
    }
    /// Chunk boxes in index order; meters.
    pub fn chunks(&self) -> &[Chunk3] {
        &self.chunks
    }
    /// Bond list in descriptor order: the `bond` of a [`BreakEvent::BondBroken`] indexes it.
    pub fn bonds(&self) -> &[Bond] {
        &self.bonds
    }
    /// Per-chunk velocities in index order; m/s.
    pub fn velocities(&self) -> &[[f32; 3]] {
        &self.vel
    }
    /// Anchor indices in index order.
    pub fn anchors(&self) -> Vec<u32> {
        flagged_indices(&self.anchored)
    }
    /// Freed chunk indices in index order.
    pub fn freed_chunks(&self) -> Vec<u32> {
        flagged_indices(&self.freed)
    }
    /// Snapped bond indices in index order.
    pub fn broken_bonds(&self) -> Vec<u32> {
        flagged_indices(&self.bond_broken)
    }
    /// True when the chunk is anchored; out-of-range ids read false.
    pub fn is_anchored(&self, chunk: usize) -> bool {
        self.anchored.get(chunk).copied().unwrap_or(false)
    }
    /// True when the bond has snapped; out-of-range ids read false.
    pub fn is_bond_broken(&self, bond: usize) -> bool {
        self.bond_broken.get(bond).copied().unwrap_or(false)
    }
    /// Current strain of one bond; `None` when out of range.
    pub fn strain_of(&self, bond: usize) -> Option<f32> {
        self.strain.get(bond).copied()
    }
    /// Bond count.
    pub fn bond_count(&self) -> usize {
        self.bonds.len()
    }
    /// True when the chunk reached an anchor over intact bonds at the last
    /// refresh; out-of-range ids read false.
    pub fn is_supported(&self, chunk: usize) -> bool {
        self.support_scratch.get(chunk).copied().unwrap_or(false)
    }
    /// Supported chunk count at the last refresh.
    pub fn supported_count(&self) -> usize {
        self.support_scratch.iter().filter(|on| **on).count()
    }
    /// Q16 chunk masses in index order; the load unit.
    pub fn masses_q16(&self) -> &[u64] {
        &self.mass_q16
    }
    /// Q16 loads in index order: own mass plus everything hanging below in
    /// the support tree; an unsupported chunk carries only itself.
    pub fn loads_q16(&self) -> &[u64] {
        &self.load_q16
    }
    /// Support-tree parent of one chunk; `None` for anchors, unsupported
    /// chunks and out-of-range ids.
    pub fn load_parent(&self, chunk: usize) -> Option<u32> {
        self.parent.get(chunk).copied().filter(|p| *p != NO_PARENT)
    }
    /// Support-tree layer of one chunk (anchors are layer 0); `None` for
    /// unsupported chunks and out-of-range ids.
    pub fn load_layer(&self, chunk: usize) -> Option<u32> {
        self.layer.get(chunk).copied().filter(|l| *l != NO_LAYER)
    }

    /// Load over own mass of one chunk in Q16 (`65536` carries only
    /// itself); `None` out of range.
    pub fn load_ratio_q16(&self, chunk: usize) -> Option<u64> {
        let load = *self.load_q16.get(chunk)?;
        let mass = *self.mass_q16.get(chunk)?;
        Some(ratio_q16(load, mass))
    }

    /// Capacity factor in Q16; zero means the load rule is off.
    pub fn capacity_q16(&self) -> u64 {
        self.capacity_q16
    }

    /// Turns the load rule on (or off with zero): from the next step, any
    /// supported, unanchored chunk whose load exceeds `mass * capacity`
    /// snaps its bonds, one wave per tick. Returns the previous factor.
    pub fn set_capacity_q16(&mut self, capacity_q16: u64) -> u64 {
        std::mem::replace(&mut self.capacity_q16, capacity_q16)
    }

    /// True when the chunk stands, is not anchored and carries more than
    /// its capacity allows; always false while the rule is off.
    pub fn is_overloaded(&self, chunk: usize) -> bool {
        if self.capacity_q16 == 0 || chunk >= self.chunks.len() {
            return false;
        }
        !self.anchored[chunk]
            && self.support_scratch[chunk]
            && self.load_q16[chunk] > capacity_limit(self.mass_q16[chunk], self.capacity_q16)
    }

    /// The weakest link: the supported, unanchored chunk nearest its
    /// capacity, so the one with the highest load over its own mass; ties
    /// go to the lowest index. `None` when nothing but anchors stands.
    pub fn weakest_link(&self) -> Option<u32> {
        let mut best: Option<(u64, usize)> = None;
        for i in 0..self.chunks.len() {
            if self.anchored[i] || !self.support_scratch[i] {
                continue;
            }
            let ratio = ratio_q16(self.load_q16[i], self.mass_q16[i]);
            if best.is_none_or(|(top, _)| ratio > top) {
                best = Some((ratio, i));
            }
        }
        best.map(|(_, i)| wire(i))
    }

    /// Highest load-over-mass ratio in Q16 among standing, unanchored
    /// chunks; zero when only anchors stand. The capacity a builder sets
    /// above this keeps the structure up at rest.
    pub fn max_load_ratio_q16(&self) -> u64 {
        let mut top = 0u64;
        for i in 0..self.chunks.len() {
            if self.anchored[i] || !self.support_scratch[i] {
                continue;
            }
            top = top.max(ratio_q16(self.load_q16[i], self.mass_q16[i]));
        }
        top
    }

    /// What severing `chunk` would bring down under the load rule: on a
    /// copy, sever it and run cascade waves to the fixpoint; returns the
    /// chunks newly freed, ascending, `chunk` excluded. With the rule off
    /// this equals [`Self::predict_removal`]. Allocates: analysis, never
    /// the tick.
    pub fn predict_collapse(&self, chunk: usize) -> Result<Vec<u32>, BreakError> {
        if chunk >= self.chunks.len() {
            return Err(BreakError::BadChunk(chunk));
        }
        let mut copy = self.clone();
        // The forecast is the eventual outcome: no hang delay, no link stress.
        copy.sag = None;
        copy.sever_chunk(chunk, self.tick)?;
        // Each wave fails at least one chunk, so the waves end within n.
        for _ in 0..copy.chunks.len() {
            if copy.capacity_q16 == 0 || copy.fail_overloaded(self.tick) == 0 {
                break;
            }
        }
        let mut out = Vec::new();
        for i in 0..self.chunks.len() {
            if i != chunk && copy.freed[i] && !self.freed[i] {
                out.push(wire(i));
            }
        }
        Ok(out)
    }

    /// FNV-1a64 over the chunk count, then per chunk its Q16 load, parent
    /// and layer: the load-leg receipt, equal on every PC for one structure.
    pub fn load_digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(wire(self.chunks.len()));
        for i in 0..self.chunks.len() {
            h.write_u64(self.load_q16[i]);
            h.write_u32(self.parent[i]);
            h.write_u32(self.layer[i]);
        }
        h.finish()
    }

    /// Chunks that would lose their last anchor path if `chunk` vanished:
    /// the supported set today minus the supported set with `chunk`
    /// skipped, ascending, `chunk` itself excluded. Anchors may be asked
    /// too, since neighbors can hang on one alone. Allocates: analysis,
    /// never the tick.
    pub fn predict_removal(&self, chunk: usize) -> Result<Vec<u32>, BreakError> {
        if chunk >= self.chunks.len() {
            return Err(BreakError::BadChunk(chunk));
        }
        let without = self.support_flood_oracle(Some(chunk));
        let mut out = Vec::new();
        for (i, (supported, missing)) in self.support_scratch.iter().zip(without.iter()).enumerate()
        {
            if i != chunk && *supported && !*missing {
                out.push(wire(i));
            }
        }
        Ok(out)
    }

    /// Snaps every intact bond of one chunk at `tick` (strain pinned one
    /// unit past each limit, events in ascending neighbor order), then
    /// refreshes support so the chunk and whatever hung on it alone free.
    /// Returns the bonds snapped. Anchored chunks keep their place but lose
    /// their bonds like any other.
    pub fn sever_chunk(&mut self, chunk: usize, tick: u32) -> Result<u32, BreakError> {
        if chunk >= self.chunks.len() {
            return Err(BreakError::BadChunk(chunk));
        }
        let snapped = self.snap_bonds_of(chunk, tick);
        if snapped > 0 {
            self.refresh_support(tick);
        }
        Ok(snapped)
    }

    /// Snaps the intact bonds of one in-range chunk without refreshing;
    /// returns how many snapped.
    fn snap_bonds_of(&mut self, chunk: usize, tick: u32) -> u32 {
        let mut snapped = 0u32;
        for k in slot(self.adj_start[chunk])..slot(self.adj_start[chunk + 1]) {
            let bi = slot(self.adj[k].1);
            if self.bond_broken[bi] {
                continue;
            }
            self.strain[bi] = self.bonds[bi].breaking_strain + 1.0;
            self.bond_broken[bi] = true;
            snapped += 1;
            self.log.push(BreakEvent::BondBroken {
                tick,
                bond: wire(bi),
                strain: self.strain[bi],
            });
        }
        snapped
    }

    /// One cascade wave of the load rule: every overloaded chunk (judged on
    /// the loads standing at the start of the wave, ascending index) snaps
    /// its bonds, then support refreshes once. Returns how many failed; the
    /// next wave waits for the next step, so a collapse is time-sliced and
    /// replays exactly. The wave list is pre-sized scratch.
    fn fail_overloaded(&mut self, tick: u32) -> usize {
        self.wave_scratch.clear();
        for i in 0..self.chunks.len() {
            if self.is_overloaded(i) {
                self.wave_scratch.push(i);
            }
        }
        let failed = self.wave_scratch.len();
        if failed == 0 {
            return 0;
        }
        for k in 0..failed {
            let chunk = self.wave_scratch[k];
            self.snap_bonds_of(chunk, tick);
        }
        self.refresh_support(tick);
        failed
    }

    /// Marks every chunk without an unbroken-bond path to an anchor as
    /// freed and falling, emitting one [`BreakEvent::ChunkFreed`] each in
    /// index order. Anchored chunks seed the search, so they always stay.
    /// Reuses the pre-sized [`Self::support_scratch`] map plus the
    /// [`Self::work_scratch`] list, so steady refreshes allocate nothing.
    fn refresh_support(&mut self, tick: u32) {
        self.flood_loads();
        let defer = self.defers_free();
        for i in 0..self.chunks.len() {
            if !self.support_scratch[i] && !self.freed[i] && !defer {
                self.freed[i] = true;
                self.dynamic[i] = true;
                self.log.push(BreakEvent::ChunkFreed {
                    tick,
                    chunk: wire(i),
                });
            }
        }
    }

    /// Support flood with the load-leg: breadth-first layers out of the
    /// anchors over intact bonds, each chunk parented to the smallest-index
    /// chunk of the layer above it, then Q16 loads summed child into parent
    /// in reverse visit order. Fills `support_scratch`, `parent`, `layer`
    /// and `load_q16`; allocates nothing after construction (the visit
    /// list is the pre-sized `work_scratch`, layer sorts run in place).
    /// Shape only from PORT(geoelements/gns@main:gns/graph_network.py), MIT:
    /// message, aggregate-add and update over the edge list, reimplemented
    /// integer-only with no learned weights and no bytes taken.
    fn flood_loads(&mut self) {
        let n = self.chunks.len();
        // `clear` + `resize` keeps the construction-time capacity: the only
        // growth is a chunk-count increase, which never happens mid-run.
        self.support_scratch.clear();
        self.support_scratch.resize(n, false);
        self.work_scratch.clear();
        for i in 0..n {
            self.load_q16[i] = self.mass_q16[i];
            self.parent[i] = NO_PARENT;
            self.layer[i] = NO_LAYER;
            if self.anchored[i] {
                self.support_scratch[i] = true;
                self.layer[i] = 0;
                self.work_scratch.push(i);
            }
        }
        let mut layer_start = 0;
        let mut depth: u32 = 0;
        while layer_start < self.work_scratch.len() {
            let layer_end = self.work_scratch.len();
            // Ascending order inside a layer makes the first chunk to reach
            // a child its smallest-index parent.
            self.work_scratch[layer_start..layer_end].sort_unstable();
            for k in layer_start..layer_end {
                let current = self.work_scratch[k];
                let span = slot(self.adj_start[current])..slot(self.adj_start[current + 1]);
                for &(neighbor, bond) in &self.adj[span] {
                    if self.bond_broken[slot(bond)] {
                        continue;
                    }
                    let next = slot(neighbor);
                    if !self.support_scratch[next] {
                        self.support_scratch[next] = true;
                        self.parent[next] = wire(current);
                        self.layer[next] = depth + 1;
                        self.work_scratch.push(next);
                    }
                }
            }
            layer_start = layer_end;
            depth += 1;
        }
        // Every chunk of a layer sits after its parent in the visit list,
        // so one reverse pass sums children into parents exactly.
        for k in (0..self.work_scratch.len()).rev() {
            let node = self.work_scratch[k];
            let parent = self.parent[node];
            if parent != NO_PARENT {
                let load = self.load_q16[node];
                let at = slot(parent);
                self.load_q16[at] = self.load_q16[at].saturating_add(load);
            }
        }
    }

    /// The base binary flood kept as the oracle: pops a stack and scans
    /// every bond per popped chunk, marking what an anchor reaches over
    /// intact bonds. `skip` names one chunk treated as removed. Allocates;
    /// analysis and tests only, never the tick.
    fn support_flood_oracle(&self, skip: Option<usize>) -> Vec<bool> {
        let n = self.chunks.len();
        let mut supported = vec![false; n];
        let mut work: Vec<usize> = Vec::with_capacity(n);
        for (i, anchored) in self.anchored.iter().enumerate() {
            if *anchored && Some(i) != skip {
                supported[i] = true;
                work.push(i);
            }
        }
        while let Some(current) = work.pop() {
            for (bi, bond) in self.bonds.iter().enumerate() {
                if self.bond_broken[bi] {
                    continue;
                }
                let (a, b) = (slot(bond.a), slot(bond.b));
                let next = if a == current {
                    b
                } else if b == current {
                    a
                } else {
                    continue;
                };
                if Some(next) == skip || supported[next] {
                    continue;
                }
                supported[next] = true;
                work.push(next);
            }
        }
        supported
    }
}

/// Chunk or bond index as its wire `u32`; counts are bounded far below
/// `u32::MAX` by the `MAX_*` limits, so saturation never triggers.
pub(crate) fn wire(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Wire `u32` back to a slot; the reverse of [`wire`].
pub(crate) fn slot(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// Load over mass in Q16, saturated; a zero mass reads as the maximum.
fn ratio_q16(load_q16: u64, mass_q16: u64) -> u64 {
    if mass_q16 == 0 {
        return u64::MAX;
    }
    let ratio = u128::from(load_q16) * u128::from(LOAD_Q16_ONE) / u128::from(mass_q16);
    u64::try_from(ratio).unwrap_or(u64::MAX)
}

/// Load a chunk may carry under a Q16 capacity factor, saturated.
fn capacity_limit(mass_q16: u64, capacity_q16: u64) -> u64 {
    let limit = u128::from(mass_q16) * u128::from(capacity_q16) / u128::from(LOAD_Q16_ONE);
    u64::try_from(limit).unwrap_or(u64::MAX)
}

/// Chunk mass as Q16 load units, at least one unit so every chunk counts.
fn mass_to_q16(mass: f32) -> u64 {
    let scaled = (f64::from(mass) * 65_536.0).round();
    if scaled >= 1.8e19 {
        return u64::MAX;
    }
    // Validated masses are finite and positive, so the rounded value is a
    // non-negative integer in range; the floor keeps light chunks counted.
    (scaled as u64).max(1)
}

/// Sorted adjacency in compressed form: offsets per chunk plus
/// `(neighbor, bond)` pairs ascending within each chunk's span, so every
/// traversal visits neighbors in one canonical order.
fn build_adjacency(n: usize, bonds: &[Bond]) -> (Vec<u32>, Vec<(u32, u32)>) {
    let mut offsets = vec![0u32; n + 1];
    for bond in bonds {
        offsets[slot(bond.a) + 1] += 1;
        offsets[slot(bond.b) + 1] += 1;
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let mut fill = offsets.clone();
    let mut adj = vec![(0u32, 0u32); slot(offsets[n])];
    for (bi, bond) in bonds.iter().enumerate() {
        let id = wire(bi);
        let a = slot(bond.a);
        adj[slot(fill[a])] = (bond.b, id);
        fill[a] += 1;
        let b = slot(bond.b);
        adj[slot(fill[b])] = (bond.a, id);
        fill[b] += 1;
    }
    for i in 0..n {
        adj[slot(offsets[i])..slot(offsets[i + 1])].sort_unstable();
    }
    (offsets, adj)
}

/// Indices of the set flags in ascending order. Chunk and bond counts are
/// bounded far below `u32::MAX` by the `MAX_*` limits, so no index truncates.
pub(crate) fn flagged_indices(flags: &[bool]) -> Vec<u32> {
    flags
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(i, _)| wire(i))
        .collect()
}

fn check_chunks(chunks: &[Chunk3]) -> Result<(), BreakError> {
    if chunks.is_empty() {
        return Err(BreakError::EmptyChunks);
    }
    if chunks.len() > MAX_BONDED_CHUNKS {
        return Err(BreakError::TooManyChunks);
    }
    for (i, c) in chunks.iter().enumerate() {
        let finite = c.x.is_finite()
            && c.y.is_finite()
            && c.z.is_finite()
            && c.hx.is_finite()
            && c.hy.is_finite()
            && c.hz.is_finite()
            && c.mass.is_finite();
        let placed =
            c.x.abs() <= MAX_CENTER_M && c.y.abs() <= MAX_CENTER_M && c.z.abs() <= MAX_CENTER_M;
        let shaped = c.hx >= MIN_HALF_EXTENT_M
            && c.hx <= MAX_HALF_EXTENT_M
            && c.hy >= MIN_HALF_EXTENT_M
            && c.hy <= MAX_HALF_EXTENT_M
            && c.hz >= MIN_HALF_EXTENT_M
            && c.hz <= MAX_HALF_EXTENT_M;
        let massed = c.mass >= MIN_MASS && c.mass <= MAX_MASS;
        if !finite || !placed || !shaped || !massed {
            return Err(BreakError::BadChunk(i));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake3d::fracture3;
    use engine_core::alloc_probe::measure;
    use engine_core::laws;

    fn baked_chunks() -> Vec<Chunk3> {
        fracture3(0.0, 2.0, 0.0, 1.0, 1.0, 1.0, 6, 11)
    }

    fn chain_desc() -> BondedDesc {
        // Three chunks floating in a row: 0 anchored, 1 middle, 2 tip.
        BondedDesc {
            chunks: vec![
                Chunk3 {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                },
                Chunk3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                },
                Chunk3 {
                    x: 2.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 2.0,
                },
            ],
            bonds: vec![
                Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 1,
                    b: 2,
                    breaking_strain: 1.0,
                },
            ],
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        }
    }

    fn sword_script() -> (u64, Vec<BreakEvent>, Vec<u32>, Vec<u32>) {
        let mut field =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7).expect("valid crate prop");
        // Highest chunk wins; strict `>` keeps the lowest index on ties.
        let mut top = 0;
        for (i, c) in field.chunks().iter().enumerate() {
            if c.y > field.chunks()[top].y {
                top = i;
            }
        }
        let impulse =
            field.chunk_mass(top).expect("top chunk mass") * field.break_threshold() + 1.0;
        field.apply_impact(top, impulse, 3).expect("valid impact");
        for _ in 0..30 {
            field.step(1.0 / 60.0);
        }
        let events = field.drain_events();
        let pos_bits: Vec<u32> = field
            .chunks()
            .iter()
            .flat_map(|c| [c.x.to_bits(), c.y.to_bits(), c.z.to_bits()])
            .collect();
        let vel_bits: Vec<u32> = field
            .velocities()
            .iter()
            .flat_map(|v| [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()])
            .collect();
        (field.state_hash(), events, pos_bits, vel_bits)
    }

    #[test]
    fn impact_script_replays_identical_break_sets() {
        let (h1, e1, p1, v1) = sword_script();
        let (h2, e2, p2, v2) = sword_script();
        assert_eq!(h1, h2);
        assert_eq!(e1, e2);
        assert_eq!(p1, p2);
        assert_eq!(v1, v2);
        assert!(!e1.is_empty(), "a sword strike must break the crate");
        assert!(
            e1.iter()
                .any(|e| matches!(e, BreakEvent::ChunkFreed { .. })),
            "detached islands must free: {e1:?}"
        );
    }

    #[test]
    fn anchored_chunks_stay_while_islands_fall() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        assert_eq!(field.anchors(), vec![0]);
        assert!(field.freed_chunks().is_empty());
        field.step(1.0 / 60.0);
        // Fully supported: nothing frees, nothing moves.
        assert!(field.freed_chunks().is_empty());
        assert!(field.drain_events().is_empty());
        // Snap the tip bond with strain: only the tip frees and falls.
        field.add_strain(1, 1.5, 9).expect("valid strain");
        assert_eq!(field.freed_chunks(), vec![2]);
        assert!(field.is_bond_broken(1) && !field.is_bond_broken(0));
        let tip_y = field.chunks()[2].y;
        for _ in 0..600 {
            field.step(1.0 / 60.0);
        }
        assert!(field.chunks()[2].y < tip_y, "freed tip must fall");
        // Anchored head and supported middle never move nor free.
        assert!(!field.freed_chunks().contains(&0) && !field.freed_chunks().contains(&1));
        assert_eq!(field.chunks()[0].x.to_bits(), 0.0f32.to_bits());
        assert_eq!(field.chunks()[0].y.to_bits(), 2.0f32.to_bits());
        assert_eq!(field.chunks()[1].x.to_bits(), 1.0f32.to_bits());
        // The one strain event plus the one free event drained exactly once.
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        field.add_strain(1, 1.5, 9).expect("valid strain");
        assert_eq!(
            field.drain_events(),
            vec![
                BreakEvent::BondBroken {
                    tick: 9,
                    bond: 1,
                    strain: 1.5
                },
                BreakEvent::ChunkFreed { tick: 9, chunk: 2 },
            ]
        );
        assert!(field.drain_events().is_empty());
    }

    #[test]
    fn anchorless_collection_frees_everything_on_first_step() {
        let desc = BondedDesc {
            anchors: Vec::new(),
            ..chain_desc()
        };
        let mut field = BondedCollection::new(&desc).expect("anchorless builds");
        field.step(1.0 / 60.0);
        assert_eq!(field.freed_chunks(), vec![0, 1, 2]);
        assert_eq!(field.drain_events().len(), 3);
    }

    #[test]
    fn sub_threshold_impact_changes_nothing() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        let before = field.state_hash();
        let gate = field.chunk_mass(2).expect("mass") * field.break_threshold();
        field
            .apply_impact(2, gate * 0.5, 4)
            .expect("sub-threshold valid");
        assert!(field.freed_chunks().is_empty());
        assert!(field.drain_events().is_empty());
        assert_eq!(field.tick(), 0);
        assert_eq!(field.state_hash(), before);
    }

    /// Blast's elastic and fatal bands: a blow at half the gate is elastic and
    /// changes nothing, a blow at four fifths of it chips the bond by the
    /// excess over the elastic limit, and a run of those light blows breaks
    /// what one heavy blow would.
    #[test]
    fn light_blows_below_the_gate_crack_a_bond_then_break_it() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        let gate = field.chunk_mass(2).expect("mass") * field.break_threshold();
        // Four fifths of the fatal gate: past the elastic half, below the gate.
        let light = gate * 0.8;
        // Each blow chips `limit * ELASTIC_CHIP * 0.6` = 0.3 of the one-unit limit.
        for tick in 0..3 {
            field.apply_impact(2, light, tick).expect("light blow");
        }
        let strain = field.strain_of(1).expect("bond 1 exists");
        assert!(
            strain > 0.8 && strain < 1.0,
            "three light blows crack but hold: {strain}"
        );
        assert!(!field.is_bond_broken(1), "three light blows hold the bond");
        field.apply_impact(2, light, 3).expect("fourth light blow");
        assert!(
            field.is_bond_broken(1),
            "the fourth light blow wears the bond through"
        );
        assert_eq!(field.freed_chunks(), vec![2], "the tip frees and falls");
        assert!(
            field
                .drain_events()
                .iter()
                .any(|e| matches!(e, BreakEvent::BondBroken { bond: 1, .. })),
            "a worn-through bond logs one break event"
        );
    }

    #[test]
    fn strain_gate_breaks_only_past_the_limit() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        field.add_strain(0, 0.5, 1).expect("valid strain");
        assert!(!field.is_bond_broken(0));
        assert_eq!(field.strain_of(0), Some(0.5));
        assert!(field.drain_events().is_empty());
        field.add_strain(0, 0.5, 2).expect("valid strain");
        assert!(!field.is_bond_broken(0), "strain at the limit holds");
        field.add_strain(0, 0.25, 3).expect("valid strain");
        assert!(field.is_bond_broken(0));
        // Straining a snapped bond is a silent no-op.
        let before = field.state_hash();
        field.add_strain(0, 100.0, 4).expect("restrain valid");
        assert_eq!(field.state_hash(), before);
    }

    #[test]
    fn invalid_construction_rejected() {
        let good = chain_desc();
        assert_eq!(
            BondedCollection::new(&BondedDesc {
                chunks: Vec::new(),
                ..good.clone()
            })
            .unwrap_err(),
            BreakError::EmptyChunks
        );
        assert_eq!(
            BondedDesc {
                chunks: vec![baked_chunks()[0]; MAX_BONDED_CHUNKS + 1],
                ..good.clone()
            }
            .validate()
            .unwrap_err(),
            BreakError::TooManyChunks
        );
        assert_eq!(
            BondedDesc {
                bonds: vec![
                    Bond {
                        a: 0,
                        b: 1,
                        breaking_strain: 1.0
                    };
                    MAX_BONDS + 1
                ],
                ..good.clone()
            }
            .validate()
            .unwrap_err(),
            BreakError::TooManyBonds
        );
        for chunks in [
            vec![Chunk3 {
                x: f32::NAN,
                y: 0.5,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            }],
            vec![Chunk3 {
                x: 0.0,
                y: 0.5,
                z: 0.0,
                hx: 0.0,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            }],
        ] {
            assert_eq!(
                BondedDesc {
                    chunks,
                    ..good.clone()
                }
                .validate()
                .unwrap_err(),
                BreakError::BadChunk(0)
            );
        }
        // Bond endpoint past the end, self-bond, duplicate pair, bad limit.
        let bad_bonds = [
            vec![Bond {
                a: 0,
                b: 9,
                breaking_strain: 1.0,
            }],
            vec![Bond {
                a: 1,
                b: 1,
                breaking_strain: 1.0,
            }],
            vec![
                Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 1,
                    b: 0,
                    breaking_strain: 2.0,
                },
            ],
            vec![Bond {
                a: 0,
                b: 1,
                breaking_strain: f32::INFINITY,
            }],
        ];
        for bonds in bad_bonds {
            assert!(BondedDesc {
                bonds,
                ..good.clone()
            }
            .validate()
            .is_err());
        }
        assert_eq!(
            BondedDesc {
                anchors: vec![7],
                ..good.clone()
            }
            .validate()
            .unwrap_err(),
            BreakError::BadChunk(7)
        );
        assert_eq!(
            BondedDesc {
                break_threshold: -1.0,
                ..good.clone()
            }
            .validate()
            .unwrap_err(),
            BreakError::BadThreshold
        );
        assert_eq!(
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 0, 7).unwrap_err(),
            BreakError::EmptyChunks
        );
        assert_eq!(
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, MAX_CRATE_PIECES + 1, 7).unwrap_err(),
            BreakError::TooManyChunks
        );
        assert_eq!(
            BondedCollection::crate_prop(0.0, 0.5, 0.0, f32::NAN, 8, 7).unwrap_err(),
            BreakError::BadChunk(0)
        );
    }

    #[test]
    fn bad_calls_rejected_without_mutation() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        let before = field.state_hash();
        assert_eq!(
            field.apply_impact(99, 1e9, 1).unwrap_err(),
            BreakError::BadChunk(99)
        );
        for impulse in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(
                field.apply_impact(1, impulse, 1).unwrap_err(),
                BreakError::BadImpulse,
                "impulse {impulse}"
            );
        }
        assert_eq!(
            field.add_strain(99, 1.0, 1).unwrap_err(),
            BreakError::BadBond(99)
        );
        for amount in [f32::NAN, f32::INFINITY, -0.5] {
            assert_eq!(
                field.add_strain(0, amount, 1).unwrap_err(),
                BreakError::BadStrain,
                "amount {amount}"
            );
        }
        assert_eq!(field.strain_of(99), None);
        assert_eq!(field.chunk_mass(99), None);
        assert!(!field.is_anchored(99) && !field.is_bond_broken(99));
        assert_eq!(field.state_hash(), before);
        assert!(field.drain_events().is_empty());
    }

    #[test]
    fn bad_steps_ignored_without_panic() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        field.apply_impact(2, 1e9, 1).expect("valid impact");
        field.step(1.0 / 60.0);
        let (tick, hash) = (field.tick(), field.state_hash());
        for dt in [0.0, -0.5, f32::NAN, f32::INFINITY, MAX_STEP_DT + 1.0] {
            field.step(dt);
        }
        assert_eq!((field.tick(), field.state_hash()), (tick, hash));
    }

    #[test]
    fn drain_into_matches_drain_events_and_appends() {
        // Two identical chains snapped the same way must emit the same
        // rows whether drained by take or by append, with identical hashes.
        let mut take_field = BondedCollection::new(&chain_desc()).expect("valid chain");
        let mut into_field = BondedCollection::new(&chain_desc()).expect("valid chain");
        take_field.add_strain(1, 1.5, 9).expect("valid strain");
        into_field.add_strain(1, 1.5, 9).expect("valid strain");
        let taken = take_field.drain_events();
        let mut appended = vec![BreakEvent::ChunkFreed { tick: 1, chunk: 0 }];
        into_field.drain_into(&mut appended);
        assert_eq!(&appended[1..], &*taken);
        assert_eq!(appended[0], BreakEvent::ChunkFreed { tick: 1, chunk: 0 });
        assert_eq!(take_field.state_hash(), into_field.state_hash());
        assert!(take_field.drain_events().is_empty());
        let mut again = Vec::new();
        into_field.drain_into(&mut again);
        assert!(again.is_empty(), "second drain appends nothing");
        // Draining an empty log leaves prior caller content and state alone.
        let before = into_field.state_hash();
        let mut out = vec![taken[0]];
        into_field.drain_into(&mut out);
        assert_eq!(out, vec![taken[0]]);
        assert_eq!(into_field.state_hash(), before);
        // The collection keeps its log capacity across `drain_into`, so a
        // repeat break/drain cycle reuses it instead of regrowing.
        let cap = into_field.log.capacity();
        into_field.add_strain(0, 1.5, 10).expect("valid strain");
        into_field.drain_into(&mut Vec::with_capacity(8));
        assert_eq!(into_field.log.capacity(), cap);
    }

    #[test]
    fn support_scratch_is_presized_and_reused() {
        let field = BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7).expect("valid prop");
        assert!(field.support_scratch.capacity() >= field.len());
        assert!(field.work_scratch.capacity() >= field.len());
        let mut field = field;
        let (support_cap, work_cap) = (
            field.support_scratch.capacity(),
            field.work_scratch.capacity(),
        );
        for _ in 0..30 {
            field.step(1.0 / 60.0);
            field.drain_events();
        }
        assert_eq!(field.support_scratch.capacity(), support_cap);
        assert_eq!(field.work_scratch.capacity(), work_cap);
        assert_eq!(field.support_scratch.len(), field.len());
    }

    #[test]
    fn alloc_probe_sees_a_real_allocation() {
        // Calibration: a blind counter must not fake the readings below.
        let (_, seen) = measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn steady_support_refresh_allocates_nothing() {
        // Steady floor for SL19: a supported prop steps with no breaks, so
        // the refresh (supported map plus work stack) plus a `drain_into`
        // into a reused caller buffer must read zero heap allocations.
        let mut field =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7).expect("valid crate prop");
        let mut out = Vec::with_capacity(16);
        for _ in 0..10 {
            field.step(1.0 / 60.0);
            field.drain_into(&mut out);
            out.clear();
        }
        let mut hash = 0u64;
        let ((), heap) = measure(|| {
            for _ in 0..600 {
                field.step(1.0 / 60.0);
                field.drain_into(&mut out);
                out.clear();
            }
            hash = field.state_hash();
            std::hint::black_box(hash);
        });
        assert_eq!(heap.calls, 0, "steady support refresh allocated");
        // Repeat slices allocate and hash identically: no hidden growth.
        let mut second =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7).expect("valid crate prop");
        let mut out2 = Vec::with_capacity(16);
        for _ in 0..10 {
            second.step(1.0 / 60.0);
            second.drain_into(&mut out2);
            out2.clear();
        }
        let ((), again) = measure(|| {
            for _ in 0..600 {
                second.step(1.0 / 60.0);
                second.drain_into(&mut out2);
                out2.clear();
            }
            std::hint::black_box(second.state_hash());
        });
        assert_eq!(again.calls, 0, "second steady slice allocated");
        assert_eq!(second.state_hash(), hash);
    }

    #[test]
    fn event_bytes_are_canonical_and_hash_stable() {
        let a = BreakEvent::BondBroken {
            tick: 9,
            bond: 1,
            strain: 1.5,
        };
        let b = BreakEvent::ChunkFreed { tick: 9, chunk: 2 };
        assert_eq!(event_bytes(&a), event_bytes(&a));
        assert_ne!(event_bytes(&a), event_bytes(&b));
        assert_eq!(event_bytes(&a).len(), 13);
        assert_eq!(event_bytes(&a)[0], 0);
        assert_eq!(event_bytes(&b)[0], 1);
    }

    #[test]
    fn loads_and_parents_read_before_the_first_step() {
        let field = BondedCollection::new(&chain_desc()).expect("valid chain");
        // Masses 1, 1, 2: the anchor carries all four units, the middle
        // carries itself plus the tip, the tip carries only itself.
        assert_eq!(
            field.loads_q16(),
            &[4 * LOAD_Q16_ONE, 3 * LOAD_Q16_ONE, 2 * LOAD_Q16_ONE]
        );
        assert_eq!(
            field.masses_q16(),
            &[LOAD_Q16_ONE, LOAD_Q16_ONE, 2 * LOAD_Q16_ONE]
        );
        assert_eq!(field.load_parent(0), None);
        assert_eq!(field.load_parent(1), Some(0));
        assert_eq!(field.load_parent(2), Some(1));
        assert_eq!(field.load_layer(0), Some(0));
        assert_eq!(field.load_layer(1), Some(1));
        assert_eq!(field.load_layer(2), Some(2));
        assert_eq!(field.load_layer(9), None);
        assert_eq!(field.load_parent(9), None);
        assert_eq!(field.supported_count(), 3);
        assert!(field.is_supported(2) && !field.is_supported(3));
        // The middle carries three units on one: the highest ratio.
        assert_eq!(field.load_ratio_q16(1), Some(3 * LOAD_Q16_ONE));
        assert_eq!(field.load_ratio_q16(2), Some(LOAD_Q16_ONE));
        assert_eq!(field.load_ratio_q16(9), None);
        assert_eq!(field.weakest_link(), Some(1));
        assert_eq!(field.max_load_ratio_q16(), 3 * LOAD_Q16_ONE);
        assert_eq!(field.capacity_q16(), 0);
        assert!(!field.is_overloaded(1), "rule off: nothing is overloaded");
        let again = BondedCollection::new(&chain_desc()).expect("valid chain");
        assert_eq!(field.load_digest(), again.load_digest());
        assert_eq!(field.bond_count(), 2);
    }

    #[test]
    fn load_leg_support_set_matches_the_oracle_flood_through_breaks() {
        let mut field =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 24, 5).expect("valid crate prop");
        let mut seen = Vec::new();
        for hit in 0..field.len() {
            let oracle = field.support_flood_oracle(None);
            assert_eq!(field.support_scratch, oracle, "hit {hit}");
            let mut anchor_load = 0u64;
            let mut standing_mass = 0u64;
            for i in 0..field.len() {
                let load = field.loads_q16()[i];
                let mass = field.masses_q16()[i];
                assert!(load >= mass, "hit {hit} chunk {i}");
                if field.is_anchored(i) {
                    anchor_load += load;
                }
                if field.is_supported(i) {
                    standing_mass += mass;
                } else {
                    assert_eq!(load, mass, "unsupported chunks carry only themselves");
                    assert_eq!(field.load_parent(i), None);
                }
                if let Some(parent) = field.load_parent(i) {
                    let up = field.load_layer(slot(parent)).expect("parent stands");
                    assert_eq!(field.load_layer(i), Some(up + 1), "hit {hit} chunk {i}");
                }
            }
            // The support forest is exact: the anchors carry every standing unit.
            assert_eq!(anchor_load, standing_mass, "hit {hit}");
            seen.push(field.load_digest());
            let impulse = field.chunk_mass(hit).expect("mass") * field.break_threshold() + 1.0;
            field
                .apply_impact(hit, impulse, wire(hit))
                .expect("valid impact");
            field.step(1.0 / 60.0);
        }
        // Reruns give the same digests at every hit.
        let mut again =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 24, 5).expect("valid crate prop");
        for (hit, digest) in seen.iter().enumerate() {
            assert_eq!(again.load_digest(), *digest, "hit {hit}");
            let impulse = again.chunk_mass(hit).expect("mass") * again.break_threshold() + 1.0;
            again
                .apply_impact(hit, impulse, wire(hit))
                .expect("valid impact");
            again.step(1.0 / 60.0);
        }
    }

    #[test]
    fn adjacency_lists_every_bond_twice_in_sorted_spans() {
        let field = BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 16, 3).expect("valid prop");
        assert_eq!(field.adj.len(), 2 * field.bond_count());
        assert_eq!(field.adj_start.len(), field.len() + 1);
        let mut per_bond = vec![0u32; field.bond_count()];
        for i in 0..field.len() {
            let span = &field.adj[slot(field.adj_start[i])..slot(field.adj_start[i + 1])];
            assert!(
                span.windows(2).all(|w| w[0] < w[1]),
                "chunk {i} span sorted"
            );
            for &(neighbor, bond) in span {
                let b = &field.bonds[slot(bond)];
                assert!((b.a == wire(i) && b.b == neighbor) || (b.b == wire(i) && b.a == neighbor));
                per_bond[slot(bond)] += 1;
            }
        }
        assert!(per_bond.iter().all(|n| *n == 2));
    }

    #[test]
    fn sever_chunk_frees_exactly_the_predicted_set() {
        let mut field = BondedCollection::new(&chain_desc()).expect("valid chain");
        assert_eq!(field.predict_removal(1).expect("in range"), vec![2]);
        assert_eq!(
            field.predict_removal(2).expect("in range"),
            Vec::<u32>::new()
        );
        assert_eq!(field.predict_removal(0).expect("anchor asked"), vec![1, 2]);
        assert_eq!(
            field.predict_removal(7).unwrap_err(),
            BreakError::BadChunk(7)
        );
        // Rule off: the collapse prediction is the removal prediction.
        assert_eq!(field.predict_collapse(1).expect("in range"), vec![2]);
        assert_eq!(
            field.predict_collapse(7).unwrap_err(),
            BreakError::BadChunk(7)
        );
        assert_eq!(field.sever_chunk(1, 4).expect("in range"), 2);
        assert_eq!(field.freed_chunks(), vec![1, 2]);
        assert_eq!(field.weakest_link(), None, "only the anchor stands");
        assert_eq!(
            field.drain_events(),
            vec![
                BreakEvent::BondBroken {
                    tick: 4,
                    bond: 0,
                    strain: 2.0
                },
                BreakEvent::BondBroken {
                    tick: 4,
                    bond: 1,
                    strain: 2.0
                },
                BreakEvent::ChunkFreed { tick: 4, chunk: 1 },
                BreakEvent::ChunkFreed { tick: 4, chunk: 2 },
            ]
        );
        // Severing again snaps nothing and moves nothing; bad ids refuse.
        let before = field.state_hash();
        assert_eq!(field.sever_chunk(1, 5).expect("in range"), 0);
        assert_eq!(field.state_hash(), before);
        assert_eq!(
            field.sever_chunk(9, 5).unwrap_err(),
            BreakError::BadChunk(9)
        );
    }

    #[test]
    fn capacity_rule_fails_one_wave_per_tick_and_matches_the_prediction() {
        // A rope of five unit chunks hanging from one anchor: each link
        // carries the links below it. Capacity 2.5 keeps the rope up only
        // while the top link (load 5) is absent, so cutting the anchor bond
        // is not needed: setting the capacity below the top link's ratio
        // starts a cascade that must run one link per tick.
        let mut chunks = Vec::new();
        let mut bonds = Vec::new();
        for i in 0..6u32 {
            chunks.push(Chunk3 {
                x: 0.0,
                y: 10.0 - f32::from(u16::try_from(i).expect("small")),
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            });
            if i > 0 {
                bonds.push(Bond {
                    a: i - 1,
                    b: i,
                    breaking_strain: 1.0,
                });
            }
        }
        let desc = BondedDesc {
            chunks,
            bonds,
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        };
        let mut field = BondedCollection::new(&desc).expect("valid rope");
        assert_eq!(field.loads_q16()[1], 5 * LOAD_Q16_ONE);
        assert_eq!(field.weakest_link(), Some(1));
        assert_eq!(field.max_load_ratio_q16(), 5 * LOAD_Q16_ONE);
        // Capacity above the top ratio: the rope stands through 10 ticks.
        assert_eq!(field.set_capacity_q16(6 * LOAD_Q16_ONE), 0);
        for _ in 0..10 {
            field.step(1.0 / 60.0);
        }
        assert!(field.freed_chunks().is_empty());
        assert!(field.drain_events().is_empty());
        // Predictions on a fresh rope at capacity 2.5: cutting link 1 drops
        // every link below it; cutting link 4 leaves link 1 carrying 3 over
        // its 2.5, so the prediction cascades to the fixpoint on its copy.
        let mut probe = BondedCollection::new(&desc).expect("valid rope");
        probe.set_capacity_q16(5 * LOAD_Q16_ONE / 2);
        assert_eq!(
            probe.predict_collapse(1).expect("in range"),
            vec![2, 3, 4, 5]
        );
        assert_eq!(
            probe.predict_collapse(4).expect("in range"),
            vec![1, 2, 3, 5]
        );
        assert!(
            probe.freed_chunks().is_empty(),
            "prediction never touches the probe"
        );
        // Capacity 2.5 live: links 1 (load 5), 2 (4) and 3 (3) are overloaded
        // at once, link 4 (load 2) holds; the first wave snaps their bonds,
        // which frees everything below link 1 in the same tick.
        field.set_capacity_q16(5 * LOAD_Q16_ONE / 2);
        assert!(field.is_overloaded(1) && field.is_overloaded(2) && field.is_overloaded(3));
        assert!(!field.is_overloaded(4) && !field.is_overloaded(0));
        field.step(1.0 / 60.0);
        assert_eq!(field.freed_chunks(), vec![1, 2, 3, 4, 5]);
        let events = field.drain_events();
        assert!(events.iter().all(|e| matches!(
            e,
            BreakEvent::BondBroken { tick: 11, .. } | BreakEvent::ChunkFreed { tick: 11, .. }
        )));
        // Replays byte-equal.
        let mut again = BondedCollection::new(&desc).expect("valid rope");
        again.set_capacity_q16(6 * LOAD_Q16_ONE);
        for _ in 0..10 {
            again.step(1.0 / 60.0);
        }
        again.set_capacity_q16(5 * LOAD_Q16_ONE / 2);
        again.step(1.0 / 60.0);
        assert_eq!(again.drain_events(), events);
        assert_eq!(again.state_hash(), field.state_hash());
    }

    #[test]
    fn capacity_waves_are_time_sliced() {
        // Three branches of different depth hang from one anchor; a tight
        // capacity fails the deepest branch root first, then the next, so
        // the frees spread over ticks and every wave is one tick.
        let mut chunks = vec![Chunk3 {
            x: 0.0,
            y: 20.0,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        }];
        let mut bonds = Vec::new();
        // Branch roots 1, 2, 3 each hang from the anchor with masses 1;
        // branch k carries k extra unit chunks below its root.
        let mut next = 1u32;
        for depth in 1..=3u32 {
            let root = next;
            chunks.push(Chunk3 {
                x: f32::from(u16::try_from(depth).expect("small")) * 2.0,
                y: 19.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            });
            bonds.push(Bond {
                a: 0,
                b: root,
                breaking_strain: 1.0,
            });
            next += 1;
            let mut above = root;
            for d in 0..depth {
                chunks.push(Chunk3 {
                    x: f32::from(u16::try_from(depth).expect("small")) * 2.0,
                    y: 18.0 - f32::from(u16::try_from(d).expect("small")),
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                });
                bonds.push(Bond {
                    a: above,
                    b: next,
                    breaking_strain: 1.0,
                });
                above = next;
                next += 1;
            }
        }
        let desc = BondedDesc {
            chunks,
            bonds,
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        };
        let mut field = BondedCollection::new(&desc).expect("valid tree");
        // Roots 1, 3 and 6 carry 2, 3 and 4 units: the deepest root (6) is
        // the weakest link.
        assert_eq!(field.weakest_link(), Some(6));
        let predicted = field.predict_collapse(6).expect("in range");
        // Capacity 2.5 with the rule on: root 3 (load 3) fails in the next
        // wave, root 1 (load 2) holds.
        field.set_capacity_q16(5 * LOAD_Q16_ONE / 2);
        let predicted_rule = field.predict_collapse(6).expect("in range");
        assert_eq!(predicted, vec![7, 8, 9], "removal alone drops the branch");
        assert_eq!(
            predicted_rule,
            vec![3, 4, 5, 7, 8, 9],
            "the rule adds root 3's branch"
        );
        field.sever_chunk(6, 1).expect("in range");
        assert_eq!(field.freed_chunks(), vec![6, 7, 8, 9]);
        field.step(1.0 / 60.0);
        assert_eq!(field.freed_chunks(), vec![3, 4, 5, 6, 7, 8, 9]);
        field.step(1.0 / 60.0);
        assert_eq!(
            field.freed_chunks(),
            vec![3, 4, 5, 6, 7, 8, 9],
            "root 1 holds"
        );
        let mut actual: Vec<u32> = field
            .freed_chunks()
            .into_iter()
            .filter(|c| *c != 6)
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, predicted_rule);
    }

    /// The canonical byte stream `BondedCollection::state_hash` walks, in the
    /// exact order it writes it: length, tick, threshold, each chunk's box,
    /// mass and velocity bits, the bond count, each bond's ends, limit, strain
    /// and snap flag, then each chunk's three flags. This is the independent
    /// oracle for the equality test and the baseline in the receipt below.
    fn canonical_state_bytes(field: &BondedCollection) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u32::try_from(field.chunks.len()).unwrap_or(0).to_le_bytes());
        bytes.extend_from_slice(&field.tick.to_le_bytes());
        bytes.extend_from_slice(&field.threshold.to_bits().to_le_bytes());
        for (c, v) in field.chunks.iter().zip(field.vel.iter()) {
            for w in [
                c.x.to_bits(),
                c.y.to_bits(),
                c.z.to_bits(),
                c.hx.to_bits(),
                c.hy.to_bits(),
                c.hz.to_bits(),
                c.mass.to_bits(),
                v[0].to_bits(),
                v[1].to_bits(),
                v[2].to_bits(),
            ] {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&u32::try_from(field.bonds.len()).unwrap_or(0).to_le_bytes());
        for ((bond, strain), broken) in field
            .bonds
            .iter()
            .zip(field.strain.iter())
            .zip(field.bond_broken.iter())
        {
            bytes.extend_from_slice(&bond.a.to_le_bytes());
            bytes.extend_from_slice(&bond.b.to_le_bytes());
            bytes.extend_from_slice(&bond.breaking_strain.to_bits().to_le_bytes());
            bytes.extend_from_slice(&strain.to_bits().to_le_bytes());
            bytes.extend_from_slice(&u32::from(*broken).to_le_bytes());
        }
        for ((dynamic, freed), anchored) in field
            .dynamic
            .iter()
            .zip(field.freed.iter())
            .zip(field.anchored.iter())
        {
            bytes.extend_from_slice(&u32::from(*dynamic).to_le_bytes());
            bytes.extend_from_slice(&u32::from(*freed).to_le_bytes());
            bytes.extend_from_slice(&u32::from(*anchored).to_le_bytes());
        }
        bytes
    }

    /// One box chunk with the shared half extents.
    fn box_chunk(x: f32, y: f32, z: f32, mass: f32) -> Chunk3 {
        Chunk3 {
            x,
            y,
            z,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass,
        }
    }

    /// A collection built from an explicit shape.
    fn collection(chunks: Vec<Chunk3>, bonds: Vec<Bond>, anchors: Vec<u32>) -> BondedCollection {
        BondedCollection::new(&BondedDesc {
            chunks,
            bonds,
            anchors,
            break_threshold: BREAKING_THRESHOLD,
        })
        .expect("shape builds")
    }

    /// The live-duel crate prop: 8 pieces, bottom anchored.
    fn crate_prop(pieces: usize, seed: u64) -> BondedCollection {
        BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, pieces, seed).expect("crate prop builds")
    }

    /// Strike the highest chunk past the gate, then let the freed island fall:
    /// broken bonds, broken strain and moving velocities ride the hash.
    fn snap_top_and_step(field: &mut BondedCollection) {
        let mut top = 0;
        for (i, c) in field.chunks().iter().enumerate() {
            if c.y > field.chunks()[top].y {
                top = i;
            }
        }
        let mass = field.chunk_mass(top).expect("top mass");
        let impulse = mass * field.break_threshold() + 1.0;
        field.apply_impact(top, impulse, 3).expect("valid impact");
        for _ in 0..30 {
            field.step(1.0 / 60.0);
        }
    }

    /// Every reachable shape: a lone chunk, one bond, the chain, the live-duel
    /// crate, a crate with a broken island and a full 64-piece crate.
    fn hash_shapes() -> Vec<BondedCollection> {
        let mut cracked = crate_prop(8, 7);
        snap_top_and_step(&mut cracked);
        vec![
            collection(vec![box_chunk(0.0, 2.0, 0.0, 1.0)], Vec::new(), vec![0]),
            collection(
                vec![box_chunk(0.0, 2.0, 0.0, 1.0), box_chunk(1.0, 2.0, 0.0, 3.0)],
                vec![Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                }],
                vec![0],
            ),
            BondedCollection::new(&chain_desc()).expect("valid chain"),
            crate_prop(8, 7),
            cracked,
            crate_prop(MAX_CRATE_PIECES, 5),
        ]
    }

    /// The tick hash is FNV-1a over the canonical word stream on every shape:
    /// the digest is a desync contract, so the independent byte oracle must
    /// agree bit for bit. A chunk mutation moves the digest (non-vacuity).
    #[test]
    fn state_hash_is_fnv1a_over_the_canonical_word_stream() {
        for field in hash_shapes() {
            assert_eq!(
                field.state_hash(),
                engine_core::hash_bytes(&canonical_state_bytes(&field)),
                "shape with {} chunks, {} bonds left the hash law",
                field.len(),
                field.bonds().len()
            );
        }
        let mut field = crate_prop(8, 7);
        let before = field.state_hash();
        assert_eq!(
            before,
            engine_core::hash_bytes(&canonical_state_bytes(&field)),
            "the receipt fixture starts on the hash law"
        );
        field.chunks[0].x += 0.25;
        assert_ne!(
            field.state_hash(),
            before,
            "a moved chunk must move the digest"
        );
    }

    /// Timing receipt: the shipped per-tick hash body against the same
    /// canonical bytes folded by `engine_core::hash_bytes`, interleaved on one
    /// broken-crate fixture. The shipped body must not be the slower side; the
    /// ratio is the receipt. Run: cargo test -p engine-destruction --lib
    /// --release state_hash_bench_receipt -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn state_hash_bench_receipt() {
        let mut field = crate_prop(8, 7);
        snap_top_and_step(&mut field);
        let bytes = canonical_state_bytes(&field);
        let iters = 200usize;
        let batch = 2000usize;
        let mut body = Vec::with_capacity(iters);
        let mut stream = Vec::with_capacity(iters);
        for _ in 0..iters {
            let t = std::time::Instant::now();
            for _ in 0..batch {
                std::hint::black_box(field.state_hash());
            }
            body.push(t.elapsed().as_nanos());
            let t = std::time::Instant::now();
            for _ in 0..batch {
                std::hint::black_box(engine_core::hash_bytes(&bytes));
            }
            stream.push(t.elapsed().as_nanos());
        }
        body.sort_unstable();
        stream.sort_unstable();
        let per = |ns: u128| {
            engine_core::checked::f64_from_u64(u64::try_from(ns).unwrap_or(0))
                / engine_core::checked::f64_from_u64(u64::try_from(batch).unwrap_or(1))
        };
        let (body_p50, stream_p50) = (per(body[iters / 2]), per(stream[iters / 2]));
        eprintln!(
            "state_hash_p50_us={:.4} canonical_stream_p50_us={:.4} ratio={:.2} bytes={} chunks={} bonds={}",
            body_p50 / 1000.0,
            stream_p50 / 1000.0,
            body_p50 / stream_p50,
            bytes.len(),
            field.len(),
            field.bonds().len()
        );
        assert_eq!(field.state_hash(), engine_core::hash_bytes(&bytes));
        assert!(
            body_p50 < stream_p50,
            "the shipped hash must not be the slower side"
        );
    }

    /// One 60 Hz tick: the step every law below runs.
    const LAW_DT: f32 = 1.0 / 60.0;
    /// Energy slack in joules. Rounding of a crate piece's height and speed
    /// is about 1e-6 J in total; a crate piece (about 0.1 kg) sheds about
    /// 1e-3 J per tick of fall (m g^2 dt^2 / 2), so 1e-4 J sits between.
    const ENERGY_SLACK_J: f64 = 1e-4;

    /// The piece a sword strike lands on: the highest chunk, lowest index on ties.
    fn highest_chunk(field: &BondedCollection) -> usize {
        let mut top = 0;
        for (i, c) in field.chunks().iter().enumerate() {
            if c.y > field.chunks()[top].y {
                top = i;
            }
        }
        top
    }

    /// Canonical state bytes with the tick counter zeroed. The tick moves on
    /// every step even at rest, so the laws compare everything else.
    fn law_bytes(field: &BondedCollection) -> Vec<u8> {
        let mut bytes = canonical_state_bytes(field);
        bytes[4..8].copy_from_slice(&[0; 4]);
        bytes
    }

    /// A collection with the break events it has emitted, drained after every
    /// step. Equal only when the physical state and the event log both match.
    #[derive(Clone, Debug)]
    struct Run {
        field: BondedCollection,
        seen: Vec<BreakEvent>,
    }

    impl PartialEq for Run {
        fn eq(&self, other: &Self) -> bool {
            law_bytes(&self.field) == law_bytes(&other.field) && self.seen == other.seen
        }
    }

    impl Run {
        fn new(field: BondedCollection) -> Self {
            Self {
                field,
                seen: Vec::new(),
            }
        }

        /// One fixed tick; the events it emitted join the log.
        fn advance(&mut self) {
            self.field.step(LAW_DT);
            self.seen.extend(self.field.drain_events());
        }
    }

    /// Mass held by the structure: each piece counts once, as standing or as
    /// falling. A piece counted twice or dropped would move this sum.
    fn held_mass(field: &BondedCollection) -> f64 {
        let freed = field.freed_chunks();
        (0..field.len())
            .map(|i| {
                let standing = u8::from(field.is_supported(i));
                let falling = u8::from(freed.contains(&wire(i)));
                f64::from(field.chunks()[i].mass) * f64::from(standing + falling)
            })
            .sum()
    }

    /// Momentum of the whole collection along one axis: the sum of m v, f64.
    fn momentum(field: &BondedCollection, axis: usize) -> f64 {
        field
            .chunks()
            .iter()
            .zip(field.velocities())
            .map(|(c, v)| f64::from(c.mass) * f64::from(v[axis]))
            .sum()
    }

    /// Mechanical energy: kinetic plus m g y over every piece, in joules (f64).
    fn energy(field: &BondedCollection) -> f64 {
        field
            .chunks()
            .iter()
            .zip(field.velocities())
            .map(|(c, v)| {
                let speed_sq: f64 = v.iter().map(|k| f64::from(*k) * f64::from(*k)).sum();
                let m = f64::from(c.mass);
                0.5 * m * speed_sq + m * f64::from(GRAVITY_MPS2) * f64::from(c.y)
            })
            .sum()
    }

    /// The x-mirror of a collection: each piece's center x negated, bonds,
    /// anchors and gate kept, so piece i of the copy mirrors piece i.
    fn x_mirror(field: &BondedCollection) -> BondedCollection {
        let chunks = field
            .chunks()
            .iter()
            .map(|c| Chunk3 { x: -c.x, ..*c })
            .collect();
        BondedCollection::new(&BondedDesc {
            chunks,
            bonds: field.bonds().to_vec(),
            anchors: field.anchors(),
            break_threshold: field.break_threshold(),
        })
        .expect("the mirror of a valid collection is valid")
    }

    /// `b` is the x-mirror of `a`, piece by piece: mirrored x, equal heights,
    /// mirrored sideways speed, same supports, bond strain and snaps, freed set.
    fn x_mirrored(a: &BondedCollection, b: &BondedCollection) -> bool {
        let pieces = a
            .chunks()
            .iter()
            .zip(b.chunks())
            .zip(a.velocities().iter().zip(b.velocities()))
            .enumerate()
            .all(|(i, ((ca, cb), (va, vb)))| {
                ca.x == -cb.x
                    && ca.y == cb.y
                    && ca.z == cb.z
                    && va[0] == -vb[0]
                    && va[1] == vb[1]
                    && va[2] == vb[2]
                    && a.is_supported(i) == b.is_supported(i)
            });
        let bonds = (0..a.bond_count()).all(|j| {
            a.is_bond_broken(j) == b.is_bond_broken(j) && a.strain_of(j) == b.strain_of(j)
        });
        a.len() == b.len() && pieces && bonds && a.freed_chunks() == b.freed_chunks()
    }

    /// An intact crate on the ground, with no strike, stays bit for bit at
    /// rest for ten seconds: no creep, no bounce, no spontaneous bond snap.
    #[test]
    fn an_intact_crate_at_rest_never_creeps_or_breaks() {
        let field = crate_prop(8, 7);
        assert_eq!(
            field.supported_count(),
            field.len(),
            "every piece stands at construction"
        );
        laws::rest_stays_rest(Run::new(field), 600, Run::advance)
            .expect("a crate with no strike stays at rest");
    }

    /// Two copies of one strike, stepped side by side, emit the same breaks
    /// and fall to the same poses: no hidden input, no order dependence.
    #[test]
    fn a_struck_crate_replays_to_the_same_breaks_and_fall() {
        let field = crate_prop(8, 7);
        let top = highest_chunk(&field);
        let impulse = field.chunk_mass(top).expect("top mass") * field.break_threshold() + 1.0;
        let end = laws::replay(Run::new(field), 600, |run, tick| {
            if tick == 3 {
                run.field
                    .apply_impact(top, impulse, 3)
                    .expect("valid impact");
            }
            run.advance();
        })
        .expect("two copies of one strike stay equal");
        assert!(end
            .seen
            .iter()
            .any(|e| matches!(e, BreakEvent::BondBroken { .. })));
        assert!(end
            .seen
            .iter()
            .any(|e| matches!(e, BreakEvent::ChunkFreed { .. })));
    }

    /// A crate struck at piece i and its x-mirror struck at the same piece
    /// index break mirrored: same snapped bonds, same freed set, mirrored falls.
    #[test]
    fn mirrored_strikes_break_mirrored_crates() {
        let a = crate_prop(8, 7);
        let b = x_mirror(&a);
        let top = highest_chunk(&a);
        let impulse = a.chunk_mass(top).expect("top mass") * a.break_threshold() + 1.0;
        let mut probe = crate_prop(8, 7);
        probe.apply_impact(top, impulse, 3).expect("valid impact");
        assert!(
            !probe.freed_chunks().is_empty(),
            "the strike must free pieces"
        );
        laws::mirror(
            a,
            b,
            600,
            |s, tick| {
                if tick == 3 {
                    s.apply_impact(top, impulse, 3).expect("valid impact");
                }
                s.step(LAW_DT);
            },
            x_mirrored,
        )
        .expect("a struck crate and its mirror break and fall mirrored");
    }

    /// A strike adds no energy and gravity only sheds it: after the impact,
    /// kinetic plus potential over all pieces never rises, including the
    /// ground stop of each falling piece.
    #[test]
    fn a_struck_crate_never_gains_energy_as_its_pieces_fall() {
        let mut field = crate_prop(8, 7);
        let top = highest_chunk(&field);
        let impulse = field.chunk_mass(top).expect("top mass") * field.break_threshold() + 1.0;
        field.apply_impact(top, impulse, 3).expect("valid impact");
        assert!(
            !field.freed_chunks().is_empty(),
            "the strike must free pieces"
        );
        let struck = energy(&field);
        let mut fallen = field.clone();
        for _ in 0..600 {
            fallen.step(LAW_DT);
        }
        assert!(energy(&fallen) < struck, "the pieces really fell");
        laws::energy_never_rises(field, 600, |s| s.step(LAW_DT), energy, ENERGY_SLACK_J)
            .expect("a strike adds no energy and the fall only sheds it");
    }

    /// Piece mass is held through a snap and the fall: the pieces sum to the
    /// original total, each counted once, standing or falling. The masses are
    /// fixed at construction, so the sum recomputes bit for bit (slack 0).
    #[test]
    fn piece_mass_is_held_through_every_snap() {
        let field = crate_prop(8, 7);
        let top = highest_chunk(&field);
        let total: f64 = field.chunks().iter().map(|c| f64::from(c.mass)).sum();
        assert_eq!(held_mass(&field), total, "the pieces sum to the original");
        let mut probe = crate_prop(8, 7);
        probe.sever_chunk(top, 3).expect("in range");
        assert!(
            !probe.freed_chunks().is_empty(),
            "the snap must free pieces"
        );
        let mut step_no = 0u32;
        laws::conserved(
            field,
            600,
            |s| {
                step_no += 1;
                if step_no == 3 {
                    s.sever_chunk(top, 3).expect("in range");
                }
                s.step(LAW_DT);
            },
            held_mass,
            0.0,
        )
        .expect("every piece is standing or falling, counted once");
    }

    /// A snap moves nothing, and a freed piece with sideways speed keeps its
    /// x and z momentum through the fall: gravity and the ground act on y only.
    #[test]
    fn sideways_momentum_survives_a_snap_and_the_fall() {
        let mut field = crate_prop(8, 7);
        let top = highest_chunk(&field);
        let before: Vec<f64> = (0..3).map(|axis| momentum(&field, axis)).collect();
        field.sever_chunk(top, 3).expect("in range");
        let after: Vec<f64> = (0..3).map(|axis| momentum(&field, axis)).collect();
        assert_eq!(
            before, after,
            "a snap moves nothing: the freed piece carries no velocity"
        );
        // Sideways speed for the freed piece; no force acts along x or z.
        field.vel[top] = [0.8, 0.0, -0.3];
        laws::conserved(
            field.clone(),
            600,
            |s| s.step(LAW_DT),
            |s| momentum(s, 0),
            0.0,
        )
        .expect("x momentum holds through the fall");
        laws::conserved(field, 600, |s| s.step(LAW_DT), |s| momentum(s, 2), 0.0)
            .expect("z momentum holds through the fall");
    }
}
