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
const MIN_MASS: f32 = 1e-6;
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

    /// Applies a hit to one chunk: sub-threshold impulses change nothing;
    /// above `mass * threshold` every intact incident bond gains
    /// `impulse / mean-mass` strain and snaps past its limit, then support
    /// refreshes and detached islands free. Events queue in the log.
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
        // sub-threshold hits without a fallible comparison.
        if impulse <= self.chunks[chunk].mass * self.threshold {
            return Ok(());
        }
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
            self.strain[bi] += impulse / mean;
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
        h.write_u32(self.chunks.len() as u32);
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
        h.write_u32(self.bonds.len() as u32);
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
        for i in 0..self.chunks.len() {
            if !self.support_scratch[i] && !self.freed[i] {
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
}
