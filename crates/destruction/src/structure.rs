//! structure: load-path analysis over a bonded collection plus the seeded
//! wall and building fixtures the collapse proofs stand on.
//!
//! Consumer: the break verbs (`structure.analyze` through the door's break
//! pack) and the collapse, snapshot and budget proofs; the arena may raise
//! a siege wall through [`building`]. Destruction never imports arena.
//! Units: meters for geometry, Q16 mass units for loads (`1.0 == 65536`),
//! chunk indices as `u32` slots. Determinism: every number derives from the
//! spec and its seed through integer `Rng` draws and `+ - * /` only, bonds
//! are contact bonds in index order, and the report digest is FNV-1a over
//! the fields in index order. Errors: [`StructureError`] for a bad spec,
//! [`BreakError`] from the collection. Budget: `analyze` runs one removal
//! cascade on a copy on top of the per-tick load-leg, so it is an off-tick
//! query; the tick cost lives in `bonded.rs`.

use crate::bake::BREAKING_THRESHOLD;
use crate::bonded::{
    event_bytes, slot, wire, Bond, BondedCollection, BondedDesc, BreakError, Chunk3,
    DEFAULT_BOND_STRAIN, MAX_BONDED_CHUNKS, MAX_BONDS,
};
use crate::shards3d::in_contact;
use engine_core::{Fnv64, Rng};

/// Most bays along one axis a building spec accepts.
pub const MAX_BAYS: u32 = 16;
/// Most stories a building spec accepts.
pub const MAX_STORIES: u32 = 8;
/// Most seeded slab knockouts a building spec accepts.
pub const MAX_KNOCKOUTS: u32 = 8;
/// Largest accepted length in a spec; meters.
pub const MAX_SPEC_LEN_M: f32 = 100.0;
/// Largest accepted density; mass units per cubic meter.
pub const MAX_DENSITY: f32 = 1.0e4;
/// Chunks in the oracle wall.
pub const ORACLE_WALL_CHUNKS: usize = 8;
/// Bottom-layer tolerance when picking anchors; meters.
const GROUND_EPS_M: f32 = 1e-3;
/// Mass jitter in 1/1024 units: masses scale by `[0.9, 1.1)`.
const JITTER_BASE: u64 = 922;
const JITTER_SPAN: u64 = 205;
/// Bond strain jitter in 1/4096 units: limits scale by `[1.0, 1.25)`.
const STRAIN_JITTER_SPAN: u64 = 1024;
/// Oracle-wall mass jitter in 1/100 units: `[-0.24, +0.24]`.
const WALL_JITTER_SPAN: u64 = 49;

/// What one building element is; parallel to the chunk list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
    /// Vertical post carrying the floors above it.
    Column,
    /// Horizontal beam along x on top of a row of columns.
    BeamX,
    /// Horizontal beam along z on top of a row of columns.
    BeamZ,
    /// Floor slab resting on the beams of one bay.
    Slab,
}

impl ElementKind {
    /// Wire value of the kind.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Column => 0,
            Self::BeamX => 1,
            Self::BeamZ => 2,
            Self::Slab => 3,
        }
    }

    /// Stable lowercase name for reports and the door.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Column => "column",
            Self::BeamX => "beam_x",
            Self::BeamZ => "beam_z",
            Self::Slab => "slab",
        }
    }
}

/// Why a building spec was refused; user input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureError {
    /// Bays, stories or knockouts are zero or past their cap.
    BadCount,
    /// A length is non-finite, non-positive or past `MAX_SPEC_LEN_M`.
    BadLength,
    /// Density is non-finite, non-positive or past `MAX_DENSITY`.
    BadDensity,
    /// The spec builds more than `MAX_BONDED_CHUNKS` elements.
    TooManyElements,
    /// The collection refused the generated descriptor.
    Bonded(BreakError),
}

impl core::fmt::Display for StructureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StructureError::BadCount => write!(f, "building counts are out of bounds"),
            StructureError::BadLength => write!(f, "building length is out of bounds"),
            StructureError::BadDensity => write!(f, "building density is out of bounds"),
            StructureError::TooManyElements => {
                write!(f, "building exceeds MAX_BONDED_CHUNKS elements")
            }
            StructureError::Bonded(e) => write!(f, "building descriptor refused: {e}"),
        }
    }
}

impl std::error::Error for StructureError {}

impl From<BreakError> for StructureError {
    fn from(e: BreakError) -> Self {
        StructureError::Bonded(e)
    }
}

/// A seeded frame building: one column per grid point and story, beams on
/// top of every column row, one slab per bay, ground columns anchored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingSpec {
    /// Bays along x; `1..=MAX_BAYS`.
    pub bays_x: u32,
    /// Bays along z; `1..=MAX_BAYS`.
    pub bays_z: u32,
    /// Stories; `1..=MAX_STORIES`.
    pub stories: u32,
    /// Bay width, column center to column center; meters.
    pub bay_m: f32,
    /// Clear story height, column foot to beam underside; meters.
    pub story_m: f32,
    /// Column half width; meters.
    pub column_half_m: f32,
    /// Beam half depth and half width; meters.
    pub beam_half_m: f32,
    /// Slab half thickness; meters.
    pub slab_half_m: f32,
    /// Mass per cubic meter before the seeded jitter.
    pub density: f32,
    /// Seeded slabs left out; `0..=MAX_KNOCKOUTS`.
    pub knockouts: u32,
    /// Seed for mass jitter, strain jitter and knockouts.
    pub seed: u64,
}

impl BuildingSpec {
    /// The proof building: three stories over 6 x 5 bays of 4 m with 3 m
    /// clear stories (42 columns, 71 beams and 30 slabs per story, 429
    /// elements), two seeded slabs knocked out, concrete density 2.4.
    #[must_use]
    pub fn three_story(seed: u64) -> Self {
        Self {
            bays_x: 6,
            bays_z: 5,
            stories: 3,
            bay_m: 4.0,
            story_m: 3.0,
            column_half_m: 0.2,
            beam_half_m: 0.15,
            slab_half_m: 0.08,
            density: 2.4,
            knockouts: 2,
            seed,
        }
    }

    /// Range-checks every knob without building.
    pub fn validate(&self) -> Result<(), StructureError> {
        let counts_ok = (1..=MAX_BAYS).contains(&self.bays_x)
            && (1..=MAX_BAYS).contains(&self.bays_z)
            && (1..=MAX_STORIES).contains(&self.stories)
            && self.knockouts <= MAX_KNOCKOUTS;
        if !counts_ok {
            return Err(StructureError::BadCount);
        }
        for len in [
            self.bay_m,
            self.story_m,
            self.column_half_m,
            self.beam_half_m,
            self.slab_half_m,
        ] {
            if !len.is_finite() || len <= 0.0 || len > MAX_SPEC_LEN_M {
                return Err(StructureError::BadLength);
            }
        }
        // Members must fit inside their bay and story.
        if self.column_half_m * 4.0 >= self.bay_m
            || self.beam_half_m * 4.0 >= self.bay_m
            || (self.beam_half_m + self.slab_half_m) * 2.0 >= self.story_m
        {
            return Err(StructureError::BadLength);
        }
        if !self.density.is_finite() || self.density <= 0.0 || self.density > MAX_DENSITY {
            return Err(StructureError::BadDensity);
        }
        if self.element_count() > MAX_BONDED_CHUNKS {
            return Err(StructureError::TooManyElements);
        }
        Ok(())
    }

    /// Elements the spec emits before knockouts.
    #[must_use]
    pub fn element_count(&self) -> usize {
        let (bx, bz) = (big(self.bays_x), big(self.bays_z));
        let columns = (bx + 1) * (bz + 1);
        let beams = bx * (bz + 1) + (bx + 1) * bz;
        let slabs = bx * bz;
        big(self.stories) * (columns + beams + slabs)
    }
}

/// A generated building: the bonded descriptor plus one kind per chunk.
#[derive(Clone, Debug)]
pub struct Building {
    /// Chunks, contact bonds and ground anchors, ready for the collection.
    pub desc: BondedDesc,
    /// Element kind per chunk, same order as `desc.chunks`.
    pub kinds: Vec<ElementKind>,
}

/// Builds the seeded frame building: story by story, columns then x beams
/// then z beams then slabs, each row in ascending z then x, so indices are
/// canonical. Masses are box volume times density times a seeded jitter;
/// bonds are contact bonds at a seeded strain; the ground layer anchors.
pub fn building(spec: &BuildingSpec) -> Result<Building, StructureError> {
    spec.validate()?;
    let mut rng = Rng::new(spec.seed).fork_named("building");
    let skipped = knockout_slabs(spec, &mut rng);
    let mut frame = Frame {
        rng: &mut rng,
        density: spec.density,
        chunks: Vec::with_capacity(spec.element_count()),
        kinds: Vec::with_capacity(spec.element_count()),
    };
    let mut slab_index = 0u64;
    for story in 0..spec.stories {
        emit_story(&mut frame, spec, story, &skipped, &mut slab_index);
    }
    let Frame { chunks, kinds, .. } = frame;
    let bonds = contact_bonds(&mut rng, &chunks)?;
    let anchors = ground_anchors(&chunks);
    let desc = BondedDesc {
        chunks,
        bonds,
        anchors,
        break_threshold: BREAKING_THRESHOLD,
    };
    desc.validate()?;
    Ok(Building { desc, kinds })
}

/// Global slab ordinals (story-major, then z, then x) the seed knocks out.
/// Only interior bays qualify: a column then keeps at least two slabs under
/// its foot, so no knockout can float an element at rest.
fn knockout_slabs(spec: &BuildingSpec, rng: &mut Rng) -> Vec<u64> {
    let mut interior: Vec<u64> = Vec::new();
    let mut ordinal = 0u64;
    for _ in 0..spec.stories {
        for iz in 0..spec.bays_z {
            for ix in 0..spec.bays_x {
                if ix > 0 && ix + 1 < spec.bays_x && iz > 0 && iz + 1 < spec.bays_z {
                    interior.push(ordinal);
                }
                ordinal += 1;
            }
        }
    }
    let mut skipped = Vec::new();
    while skipped.len() < big(spec.knockouts).min(interior.len()) {
        let at = rng.next_range(0, u64::try_from(interior.len()).unwrap_or(u64::MAX));
        let pick = interior[usize::try_from(at).unwrap_or(0)];
        if !skipped.contains(&pick) {
            skipped.push(pick);
        }
    }
    skipped
}

/// The building under construction: members so far plus the shared stream.
struct Frame<'a> {
    rng: &'a mut Rng,
    density: f32,
    chunks: Vec<Chunk3>,
    kinds: Vec<ElementKind>,
}

impl Frame<'_> {
    /// Appends `nz` rows of `nx` members of one kind in ascending z then x.
    /// `at(ix, iz)` gives `[x, y, z, hx, hy, hz]`; `keep()` is asked once
    /// per member in the same order and may refuse one (a knockout), the
    /// jitter draw happening either way so later members never shift.
    fn row(
        &mut self,
        kind: ElementKind,
        nx: u32,
        nz: u32,
        at: impl Fn(f32, f32) -> [f32; 6],
        keep: &mut impl FnMut() -> bool,
    ) {
        for iz in 0..nz {
            for ix in 0..nx {
                let member = jittered(
                    self.rng,
                    self.density,
                    at(f32::from(small(ix)), f32::from(small(iz))),
                );
                if keep() {
                    self.chunks.push(member);
                    self.kinds.push(kind);
                }
            }
        }
    }
}

/// One story: columns, then x beams and z beams on their tops, then one
/// slab per bay on the beams, skipping knocked-out slab ordinals.
fn emit_story(
    frame: &mut Frame<'_>,
    spec: &BuildingSpec,
    story: u32,
    skipped: &[u64],
    slab_index: &mut u64,
) {
    let pitch = spec.story_m + 2.0 * spec.beam_half_m + 2.0 * spec.slab_half_m;
    let base_y = f32::from(small(story)) * pitch;
    let beam_y = base_y + spec.story_m + spec.beam_half_m;
    let slab_y = beam_y + spec.beam_half_m + spec.slab_half_m;
    let (bay, ch, bh, sh, sm) = (
        spec.bay_m,
        spec.column_half_m,
        spec.beam_half_m,
        spec.slab_half_m,
        spec.story_m,
    );
    let mut all = || true;
    frame.row(
        ElementKind::Column,
        spec.bays_x + 1,
        spec.bays_z + 1,
        |ix, iz| [ix * bay, base_y + sm * 0.5, iz * bay, ch, sm * 0.5, ch],
        &mut all,
    );
    frame.row(
        ElementKind::BeamX,
        spec.bays_x,
        spec.bays_z + 1,
        |ix, iz| [(ix + 0.5) * bay, beam_y, iz * bay, bay * 0.5, bh, bh],
        &mut all,
    );
    frame.row(
        ElementKind::BeamZ,
        spec.bays_x + 1,
        spec.bays_z,
        |ix, iz| [ix * bay, beam_y, (iz + 0.5) * bay, bh, bh, bay * 0.5],
        &mut all,
    );
    let mut keep_slab = || {
        let keep = !skipped.contains(&*slab_index);
        *slab_index += 1;
        keep
    };
    frame.row(
        ElementKind::Slab,
        spec.bays_x,
        spec.bays_z,
        |ix, iz| {
            [
                (ix + 0.5) * bay,
                slab_y,
                (iz + 0.5) * bay,
                bay * 0.5,
                sh,
                bay * 0.5,
            ]
        },
        &mut keep_slab,
    );
}

/// The eight-chunk oracle wall of the load-leg spike: two columns of
/// masses 5/3/2 and 5/3 on anchors 0 and 3, three infill chunks of mass 2
/// (5 between the columns, 6 under column 1's tip, 7 under column 4).
/// Seed 0 is the hand case with loads 14/9/4/10/5/2/2/2; other seeds
/// jitter every mass by up to a quarter unit, which never moves the
/// weakest link off the columns.
#[must_use]
pub fn oracle_wall(seed: u64) -> BondedDesc {
    let base = [5.0f32, 3.0, 2.0, 5.0, 3.0, 2.0, 2.0, 2.0];
    let at = [
        [0.0f32, 0.5, 0.0],
        [0.0, 1.5, 0.0],
        [0.0, 2.5, 0.0],
        [4.0, 0.5, 0.0],
        [4.0, 1.5, 0.0],
        [2.0, 1.5, 0.0],
        [0.0, 3.5, 0.0],
        [4.0, 2.5, 0.0],
    ];
    let mut rng = Rng::new(seed).fork_named("oracle-wall");
    let mut chunks = Vec::with_capacity(ORACLE_WALL_CHUNKS);
    for (mass, pos) in base.iter().zip(at.iter()) {
        let jitter = if seed == 0 {
            0.0
        } else {
            (f32::from(small_u64(rng.next_range(0, WALL_JITTER_SPAN))) - 24.0) / 100.0
        };
        chunks.push(Chunk3 {
            x: pos[0],
            y: pos[1],
            z: pos[2],
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: mass + jitter,
        });
    }
    let pairs = [(0u32, 1u32), (1, 2), (3, 4), (1, 5), (2, 6), (4, 7), (4, 5)];
    let bonds = pairs
        .iter()
        .map(|(a, b)| Bond {
            a: *a,
            b: *b,
            breaking_strain: DEFAULT_BOND_STRAIN,
        })
        .collect();
    BondedDesc {
        chunks,
        bonds,
        anchors: vec![0, 3],
        break_threshold: BREAKING_THRESHOLD,
    }
}

/// Column chunk indices of the oracle wall; the weakest link must be one.
pub const ORACLE_WALL_COLUMNS: [u32; 3] = [1, 2, 4];
/// Infill chunk indices of the oracle wall; cutting one holds the rest.
pub const ORACLE_WALL_INFILL: [u32; 3] = [5, 6, 7];

/// What the structure analysis says about one collection right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructureReport {
    /// Chunk count.
    pub chunk_count: u32,
    /// Bond count.
    pub bond_count: u32,
    /// Anchored chunk count.
    pub anchor_count: u32,
    /// Chunks standing on an anchor path.
    pub supported_count: u32,
    /// Highest load-over-mass ratio in Q16 among standing members.
    pub max_load_ratio_q16: u64,
    /// The weakest link, if any member stands.
    pub weakest: Option<u32>,
    /// Q16 load the weakest link carries.
    pub weakest_load_q16: u64,
    /// Chunks that would come down with the weakest link, ascending.
    pub predicted_collapse: Vec<u32>,
    /// Q16 mass of `predicted_collapse`.
    pub predicted_mass_q16: u64,
    /// The load-leg digest of the collection.
    pub load_digest: u64,
}

/// Analyze one collection: counts, the weakest link and what its loss
/// brings down under the collection's load rule.
#[must_use]
pub fn analyze(field: &BondedCollection) -> StructureReport {
    let weakest = field.weakest_link();
    let (predicted_collapse, weakest_load_q16) = match weakest {
        Some(w) => (
            field.predict_collapse(slot(w)).unwrap_or_default(),
            field.loads_q16().get(slot(w)).copied().unwrap_or(0),
        ),
        None => (Vec::new(), 0),
    };
    let predicted_mass_q16 = predicted_collapse
        .iter()
        .map(|c| field.masses_q16().get(slot(*c)).copied().unwrap_or(0))
        .fold(0u64, u64::saturating_add);
    StructureReport {
        chunk_count: wire(field.len()),
        bond_count: wire(field.bond_count()),
        anchor_count: wire(field.anchors().len()),
        supported_count: wire(field.supported_count()),
        max_load_ratio_q16: field.max_load_ratio_q16(),
        weakest,
        weakest_load_q16,
        predicted_collapse,
        predicted_mass_q16,
        load_digest: field.load_digest(),
    }
}

/// FNV-1a64 over every report field in order; the predicted set is
/// length-prefixed.
#[must_use]
pub fn report_digest(report: &StructureReport) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(report.chunk_count);
    h.write_u32(report.bond_count);
    h.write_u32(report.anchor_count);
    h.write_u32(report.supported_count);
    h.write_u64(report.max_load_ratio_q16);
    h.write_u32(report.weakest.unwrap_or(u32::MAX));
    h.write_u64(report.weakest_load_q16);
    h.write_u32(wire(report.predicted_collapse.len()));
    for c in &report.predicted_collapse {
        h.write_u32(*c);
    }
    h.write_u64(report.predicted_mass_q16);
    h.write_u64(report.load_digest);
    h.finish()
}

/// Runs `ticks` fixed steps, folding every drained break event's canonical
/// bytes and then the final state hash into one digest: the collapse
/// receipt two runs must agree on.
pub fn collapse_digest(field: &mut BondedCollection, ticks: u32, dt: f32) -> u64 {
    let mut h = Fnv64::new();
    let mut out = Vec::new();
    for _ in 0..ticks {
        field.step(dt);
        field.drain_into(&mut out);
        h.write_u32(wire(out.len()));
        for event in &out {
            h.write(&event_bytes(event));
        }
        out.clear();
    }
    h.write_u64(field.state_hash());
    h.finish()
}

/// One seeded cut for the break corpus: the oracle wall of `index * 2`,
/// one chunk severed at a seeded tick, then 40 fixed steps folded by
/// [`collapse_digest`]. Same seed always gives the same digest; doubling
/// the index keeps every engine seed pair on its own stream.
#[must_use]
pub fn seeded_wall_cut_digest(index: u64) -> u64 {
    // Engine seeds 2k and 2k+1 draw one stream; double so all differ.
    let seed = index * 2;
    let mut rng = Rng::new(seed).fork_named("cut-corpus");
    let mut field = match BondedCollection::new(&oracle_wall(seed)) {
        Ok(field) => field,
        Err(_) => return 0,
    };
    let chunk = usize::try_from(rng.next_range(0, 8)).unwrap_or(0);
    let tick = u32::try_from(rng.next_range(1, 5)).unwrap_or(1);
    if field.sever_chunk(chunk, tick).is_err() {
        return 0;
    }
    collapse_digest(&mut field, 40, 1.0 / 60.0)
}

/// Box chunk from `[x, y, z, hx, hy, hz]` with mass `volume * density *
/// jitter`, the jitter drawn once per member.
fn jittered(rng: &mut Rng, density: f32, b: [f32; 6]) -> Chunk3 {
    let jitter = f32::from(small_u64(JITTER_BASE + rng.next_range(0, JITTER_SPAN))) / 1024.0;
    Chunk3 {
        x: b[0],
        y: b[1],
        z: b[2],
        hx: b[3],
        hy: b[4],
        hz: b[5],
        mass: b[3] * b[4] * b[5] * 8.0 * density * jitter,
    }
}

/// Contact bonds in `(a, b)` index order at a seeded strain in `[1, 1.25)`
/// times the default; refuses past `MAX_BONDS`.
fn contact_bonds(rng: &mut Rng, chunks: &[Chunk3]) -> Result<Vec<Bond>, StructureError> {
    let mut bonds = Vec::new();
    for a in 0..chunks.len() {
        for b in (a + 1)..chunks.len() {
            if !in_contact(&chunks[a], &chunks[b]) {
                continue;
            }
            if bonds.len() >= MAX_BONDS {
                return Err(StructureError::Bonded(BreakError::TooManyBonds));
            }
            let jitter = f32::from(small_u64(rng.next_range(0, STRAIN_JITTER_SPAN))) / 4096.0;
            bonds.push(Bond {
                a: wire(a),
                b: wire(b),
                breaking_strain: DEFAULT_BOND_STRAIN * (1.0 + jitter),
            });
        }
    }
    Ok(bonds)
}

/// Indices of every chunk whose underside sits on the lowest underside.
fn ground_anchors(chunks: &[Chunk3]) -> Vec<u32> {
    let bottom = chunks
        .iter()
        .map(|c| c.y - c.hy)
        .reduce(f32::min)
        .unwrap_or(0.0);
    chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| (c.y - c.hy) - bottom <= GROUND_EPS_M)
        .map(|(i, _)| wire(i))
        .collect()
}

/// Spec count as a slot count.
fn big(count: u32) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

/// Spec count as a `u16` for exact `f32` conversion; counts are capped far
/// below the `u16` range.
fn small(count: u32) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

/// Jitter draw as a `u16` for exact `f32` conversion; spans are tiny.
fn small_u64(draw: u64) -> u16 {
    u16::try_from(draw).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bonded::LOAD_Q16_ONE;

    const DT: f32 = 1.0 / 60.0;

    fn q16(units: u64) -> u64 {
        units * LOAD_Q16_ONE
    }

    #[test]
    fn load_oracle_wall_matches_the_hand_loads() {
        let field = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        assert_eq!(field.len(), ORACLE_WALL_CHUNKS);
        assert_eq!(
            field.loads_q16(),
            &[
                q16(14),
                q16(9),
                q16(4),
                q16(10),
                q16(5),
                q16(2),
                q16(2),
                q16(2)
            ]
        );
        // Smallest-index parents: infill 5 hangs on column 1, not 4.
        let parents: Vec<Option<u32>> = (0..field.len()).map(|i| field.load_parent(i)).collect();
        assert_eq!(
            parents,
            vec![
                None,
                Some(0),
                Some(1),
                None,
                Some(3),
                Some(1),
                Some(2),
                Some(4)
            ]
        );
        let layers: Vec<Option<u32>> = (0..field.len()).map(|i| field.load_layer(i)).collect();
        assert_eq!(
            layers,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(2)
            ]
        );
        assert_eq!(field.weakest_link(), Some(1));
        assert_eq!(field.load_ratio_q16(1), Some(q16(3)));
        let digest = field.load_digest();
        let again = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        assert_eq!(again.load_digest(), digest);
        let report = analyze(&field);
        assert_eq!(report.weakest, Some(1));
        assert_eq!(report.predicted_collapse, vec![2, 6]);
        assert_eq!(report.predicted_mass_q16, q16(4));
        assert_eq!(report.supported_count, 8);
        assert_eq!(report.anchor_count, 2);
        assert_eq!(report.bond_count, 7);
        assert_eq!(report_digest(&report), report_digest(&analyze(&again)));
        println!("load_oracle loads=14/9/4/10/5/2/2/2 weakest=1 digest={digest:016x}");
    }

    #[test]
    fn weakest_link_is_a_column_on_eight_seeded_walls() {
        let mut column_hits = 0;
        for seed in 0..8u64 {
            let mut field = BondedCollection::new(&oracle_wall(seed)).expect("valid wall");
            let weakest = field.weakest_link().expect("the wall stands");
            if ORACLE_WALL_COLUMNS.contains(&weakest) {
                column_hits += 1;
            }
            // Column cut collapses what hangs on it; an infill cut holds.
            let predicted = field.predict_collapse(slot(weakest)).expect("in range");
            assert!(
                !predicted.is_empty(),
                "seed {seed}: a column carries something"
            );
            let infill_cut = field.predict_collapse(5).expect("in range");
            assert!(
                infill_cut.is_empty(),
                "seed {seed}: infill 5 carries nothing"
            );
            field.sever_chunk(slot(weakest), 1).expect("in range");
            let mut freed = field.freed_chunks();
            freed.retain(|c| *c != weakest);
            assert_eq!(freed, predicted, "seed {seed}");
            // Replay equality at the same seed.
            let mut again = BondedCollection::new(&oracle_wall(seed)).expect("valid wall");
            again.sever_chunk(slot(weakest), 1).expect("in range");
            assert_eq!(again.drain_events(), field.drain_events(), "seed {seed}");
            assert_eq!(again.state_hash(), field.state_hash(), "seed {seed}");
            assert_eq!(again.load_digest(), field.load_digest(), "seed {seed}");
        }
        assert_eq!(column_hits, 8, "weakest==column 8/8");
        let mut field = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        field.sever_chunk(5, 2).expect("in range");
        assert_eq!(field.freed_chunks(), vec![5], "infill cut holds the wall");
        for &infill in &ORACLE_WALL_INFILL {
            assert_ne!(field.weakest_link(), Some(infill));
        }
    }

    #[test]
    fn three_story_building_has_the_proof_size_and_stands() {
        let spec = BuildingSpec::three_story(0);
        assert_eq!(spec.element_count(), 429);
        let built = building(&spec).expect("valid building");
        assert_eq!(built.desc.chunks.len(), 427, "two slabs knocked out");
        assert_eq!(built.kinds.len(), built.desc.chunks.len());
        assert_eq!(built.desc.anchors.len(), 42, "every ground column anchors");
        assert!(built.desc.bonds.len() <= MAX_BONDS);
        let mut field = BondedCollection::new(&built.desc).expect("collection builds");
        assert_eq!(field.supported_count(), field.len(), "every element stands");
        for _ in 0..30 {
            field.step(DT);
        }
        assert!(field.freed_chunks().is_empty());
        assert!(field.drain_events().is_empty());
        let report = analyze(&field);
        assert_eq!(report.chunk_count, 427);
        assert!(report.weakest.is_some());
        println!(
            "building chunks={} bonds={} anchors={} max_ratio_q16={} weakest={:?}",
            report.chunk_count,
            report.bond_count,
            report.anchor_count,
            report.max_load_ratio_q16,
            report.weakest
        );
    }

    #[test]
    fn building_spec_refuses_bad_knobs() {
        let good = BuildingSpec::three_story(1);
        assert!(good.validate().is_ok());
        for bad in [
            BuildingSpec { bays_x: 0, ..good },
            BuildingSpec {
                bays_z: MAX_BAYS + 1,
                ..good
            },
            BuildingSpec { stories: 0, ..good },
            BuildingSpec {
                knockouts: MAX_KNOCKOUTS + 1,
                ..good
            },
        ] {
            assert_eq!(bad.validate(), Err(StructureError::BadCount));
        }
        for bad in [
            BuildingSpec {
                bay_m: f32::NAN,
                ..good
            },
            BuildingSpec {
                story_m: 0.0,
                ..good
            },
            BuildingSpec {
                column_half_m: 2.0,
                ..good
            },
            BuildingSpec {
                slab_half_m: 2.0,
                ..good
            },
        ] {
            assert_eq!(bad.validate(), Err(StructureError::BadLength));
        }
        assert_eq!(
            BuildingSpec {
                density: -1.0,
                ..good
            }
            .validate(),
            Err(StructureError::BadDensity)
        );
        assert_eq!(
            BuildingSpec {
                bays_x: 16,
                bays_z: 16,
                stories: 8,
                ..good
            }
            .validate(),
            Err(StructureError::TooManyElements)
        );
        assert!(building(&BuildingSpec { bays_x: 0, ..good }).is_err());
    }

    /// Builds the seeded proof building with its capacity set `num/den`
    /// above the standing load of its most loaded member. The seed is
    /// doubled: engine seeds `2k` and `2k+1` draw one stream, so only even
    /// seeds are distinct buildings.
    fn building_with_margin(seed: u64, num: u64, den: u64) -> (BondedCollection, Vec<ElementKind>) {
        let built = building(&BuildingSpec::three_story(seed * 2)).expect("valid building");
        let mut field = BondedCollection::new(&built.desc).expect("collection builds");
        let capacity = field.max_load_ratio_q16() * num / den;
        field.set_capacity_q16(capacity);
        (field, built.kinds)
    }

    /// The quarter-margin proof building.
    fn designed_building(seed: u64) -> (BondedCollection, Vec<ElementKind>) {
        building_with_margin(seed, 5, 4)
    }

    /// Sliced collapse: one wave per tick until two quiet ticks.
    fn settle(field: &mut BondedCollection) {
        let mut quiet = 0;
        let mut last = field.freed_chunks().len();
        for _ in 0..400 {
            field.step(DT);
            let now = field.freed_chunks().len();
            quiet = if now == last { quiet + 1 } else { 0 };
            last = now;
            if quiet >= 2 {
                break;
            }
        }
    }

    #[test]
    fn progressive_weakest_cuts_match_the_prediction_until_the_first_collapse() {
        // Cut the weakest link again and again; before every cut the
        // analysis names what will fall, and after it exactly that falls.
        // Three safety margins: the tighter the margin, the sooner a cut
        // starts a collapse.
        for (num, den) in [(21u64, 20u64), (11, 10), (5, 4)] {
            let mut agreed = 0;
            let mut cuts_total = 0;
            let mut collapsed = 0;
            let mut sizes = Vec::new();
            for seed in 0..20u64 {
                let (mut field, _) = building_with_margin(seed, num, den);
                field.step(DT);
                assert!(
                    field.freed_chunks().is_empty(),
                    "seed {seed} stands at rest"
                );
                for cut in 0..60u32 {
                    let report = analyze(&field);
                    let Some(weakest) = report.weakest else {
                        break;
                    };
                    let before = field.freed_chunks();
                    field.sever_chunk(slot(weakest), 2 + cut).expect("in range");
                    settle(&mut field);
                    let mut fell: Vec<u32> = field
                        .freed_chunks()
                        .into_iter()
                        .filter(|c| !before.contains(c) && *c != weakest)
                        .collect();
                    fell.sort_unstable();
                    cuts_total += 1;
                    if fell == report.predicted_collapse {
                        agreed += 1;
                    }
                    if !report.predicted_collapse.is_empty() {
                        collapsed += 1;
                        sizes.push(report.predicted_collapse.len());
                        break;
                    }
                }
            }
            println!(
                "margin {num}/{den}: predicted==actual on {agreed}/{cuts_total} cuts, \
                 {collapsed}/20 buildings reached a collapse, sizes {sizes:?}"
            );
            assert_eq!(agreed, cuts_total, "margin {num}/{den}: every cut matches");
        }
    }

    #[test]
    fn weakest_link_removal_collapses_the_predicted_set_on_twenty_buildings() {
        let mut agree = 0;
        let mut nonempty = 0;
        let mut digests = Vec::new();
        for seed in 0..20u64 {
            let (mut field, kinds) = designed_building(seed);
            // Designed: nothing fails at rest.
            field.step(DT);
            assert!(
                field.freed_chunks().is_empty(),
                "seed {seed} stands at rest"
            );
            let report = analyze(&field);
            let weakest = report.weakest.expect("the building stands");
            let predicted = report.predicted_collapse.clone();
            if !predicted.is_empty() {
                nonempty += 1;
            }
            field.sever_chunk(slot(weakest), 2).expect("in range");
            // Sliced: one wave per tick until two quiet ticks.
            let mut quiet = 0;
            let mut last = field.freed_chunks().len();
            for _ in 0..200 {
                field.step(DT);
                let now = field.freed_chunks().len();
                quiet = if now == last { quiet + 1 } else { 0 };
                last = now;
                if quiet >= 2 {
                    break;
                }
            }
            let mut actual = field.freed_chunks();
            actual.retain(|c| *c != weakest);
            if actual == predicted {
                agree += 1;
            }
            println!(
                "seed {seed}: weakest={weakest} kind={:?} predicted={} actual={} max_ratio_q16={}",
                kinds[slot(weakest)],
                predicted.len(),
                actual.len(),
                report.max_load_ratio_q16
            );
            digests.push((report_digest(&report), collapse_digest(&mut field, 120, DT)));
        }
        assert!(agree >= 19, "predicted set agreement {agree}/20");
        // Second run: every report and collapse digest repeats.
        for seed in 0..20u64 {
            let (mut field, _) = designed_building(seed);
            field.step(DT);
            let report = analyze(&field);
            let weakest = report.weakest.expect("stands");
            field.sever_chunk(slot(weakest), 2).expect("in range");
            let mut quiet = 0;
            let mut last = field.freed_chunks().len();
            for _ in 0..200 {
                field.step(DT);
                let now = field.freed_chunks().len();
                quiet = if now == last { quiet + 1 } else { 0 };
                last = now;
                if quiet >= 2 {
                    break;
                }
            }
            let digest = (report_digest(&report), collapse_digest(&mut field, 120, DT));
            assert_eq!(
                digest,
                digests[usize::try_from(seed).expect("small")],
                "seed {seed}"
            );
        }
        println!("collapse agreement {agree}/20, non-empty collapses {nonempty}/20");
    }

    #[test]
    fn ten_thousand_seeded_cuts_are_byte_equal_twice() {
        let fold = || {
            let mut h = Fnv64::new();
            for seed in 0..10_000u64 {
                h.write_u64(seeded_wall_cut_digest(seed));
            }
            h.finish()
        };
        let first = fold();
        assert_eq!(first, fold(), "10,000 cuts x2");
        assert_eq!(first, 0xced4ea909061defd, "corpus digest is pinned");
        assert_eq!(
            crate::seeded_wall_cut_digest(7),
            seeded_wall_cut_digest(7),
            "lib re-export matches the module path"
        );
        println!("10,000 seeded cuts corpus digest={first:016x}");
    }

    #[test]
    fn collapse_digest_folds_events_and_state() {
        let mut a = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        let mut b = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        a.sever_chunk(1, 1).expect("in range");
        b.sever_chunk(1, 1).expect("in range");
        assert_eq!(
            collapse_digest(&mut a, 60, DT),
            collapse_digest(&mut b, 60, DT)
        );
        let mut c = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        c.sever_chunk(5, 1).expect("in range");
        let mut d = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        d.sever_chunk(1, 1).expect("in range");
        assert_ne!(
            collapse_digest(&mut c, 60, DT),
            collapse_digest(&mut d, 60, DT)
        );
    }

    #[test]
    #[ignore = "wall-clock budget: run alone with --ignored on a quiet box"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn step_budget_building_600_ticks() {
        let (mut field, _) = designed_building(0);
        let weakest = field.weakest_link().expect("stands");
        field.sever_chunk(slot(weakest), 1).expect("in range");
        let mut samples = Vec::with_capacity(600);
        let mut out = Vec::new();
        for _ in 0..600 {
            let start = std::time::Instant::now();
            field.step(DT);
            samples.push(start.elapsed());
            field.drain_into(&mut out);
            out.clear();
        }
        samples.sort_unstable();
        let p50 = samples[299];
        let p99 = samples[593];
        let wall = BondedCollection::new(&oracle_wall(0)).expect("valid wall");
        let mut wall_samples = Vec::with_capacity(1000);
        let mut wall = wall;
        for _ in 0..1000 {
            let start = std::time::Instant::now();
            wall.step(DT);
            wall_samples.push(start.elapsed());
        }
        wall_samples.sort_unstable();
        println!(
            "building step p50={:?} p99={:?}; oracle wall step p50={:?}",
            p50, p99, wall_samples[499]
        );
        assert!(
            p99.as_micros() <= 1000,
            "building step p99 {p99:?} over 1 ms"
        );
        assert!(
            wall_samples[499].as_micros() <= 50,
            "wall step p50 over 50 us"
        );
    }
}
