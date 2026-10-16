//! wall_slice: offline siege-wall slice bake for FPS/crowd maps.
//!
//! Consumer: siege walls in arena and game matches: bake once offline with
//! [`bake_slice`], ship [`slice_bytes`], and at load time run
//! [`decode_slice`] then [`BondedCollection::new`](crate::bonded::BondedCollection::new).
//! The match then serves table reads (`cut`-style index picks) plus
//! [`apply_impact`](crate::bonded::BondedCollection::apply_impact): no RNG
//! and no clip run at runtime, so the shipped chunk data is the whole wall.
//! Data owner: `engine-destruction`. Units: meters for boxes, shard impulse
//! units for the threshold, ticks for events. Determinism: the bake is pure
//! over the spec (seeded bisection plus index-order contact bonds and bottom
//! anchors), and the bytes are little-endian `to_bits` in index order, so
//! the same spec gives the same bytes and digest on every PC. Errors:
//! [`WallSliceError`] at every boundary; hostile bytes refuse, never panic.
//! Budget: the bake runs offline (`pieces^2` contact checks, capped by
//! [`MAX_SLICE_PIECES`]); decode is `O(chunks + bonds)` once per load and
//! the tick path allocates nothing steady-state (see `bonded`).

use crate::bake3d::fracture3;
use crate::bonded::{slot, wire, Bond, BondedDesc, BreakError, MAX_BONDED_CHUNKS, MAX_BONDS};
use crate::cursor::Cursor;
use crate::shards3d::in_contact;

/// Wire version of the wall-slice bytes.
pub const WALL_SLICE_V1: u32 = 1;
/// Most pieces one slice bake emits; bounds the offline contact checks.
pub const MAX_SLICE_PIECES: usize = 64;
/// Bottom-layer tolerance when the bake picks anchors, meters (the
/// bonded crate-helper convention).
const WALL_ANCHOR_EPS_M: f32 = 1e-3;

/// Offline bake inputs: one wall course box plus seed and bond limits.
/// Determinism: the same spec bakes the same descriptor on every PC.
#[derive(Clone, Copy, Debug)]
pub struct SiegeSliceSpec {
    /// Course center x, meters.
    pub cx: f32,
    /// Course center y, meters.
    pub cy: f32,
    /// Course center z, meters.
    pub cz: f32,
    /// Course half-extent, meters; the course is a `half_m` cube.
    pub half_m: f32,
    /// Chunk count, `1..=MAX_SLICE_PIECES`.
    pub pieces: usize,
    /// Bake seed; expanded only through the seeded bisection.
    pub seed: u64,
    /// Per-bond strain limit; carried onto every contact bond.
    pub breaking_strain: f32,
    /// Impulse per unit mass above which a hit strains bonds.
    pub break_threshold: f32,
}

impl SiegeSliceSpec {
    /// Range-checks the spec without baking; geometry the box cannot hold
    /// (absurd centers, slivers) refuses at [`bake_slice`] with the
    /// bonded shape error instead.
    pub fn validate(&self) -> Result<(), WallSliceError> {
        if !self.cx.is_finite() || !self.cy.is_finite() || !self.cz.is_finite() {
            return Err(WallSliceError::BadSpec("center"));
        }
        if !self.half_m.is_finite() || self.half_m <= 0.0 {
            return Err(WallSliceError::BadSpec("half_m"));
        }
        if self.pieces == 0 || self.pieces > MAX_SLICE_PIECES {
            return Err(WallSliceError::BadSpec("pieces"));
        }
        if !self.breaking_strain.is_finite() || self.breaking_strain < 0.0 {
            return Err(WallSliceError::BadSpec("breaking_strain"));
        }
        if !self.break_threshold.is_finite() || self.break_threshold <= 0.0 {
            return Err(WallSliceError::BadSpec("break_threshold"));
        }
        Ok(())
    }
}

/// Why a wall-slice bake or decode was refused; user input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallSliceError {
    /// Spec row outside its range, row name carried.
    BadSpec(&'static str),
    /// The baked or decoded descriptor failed the bonded shape check.
    Bonded(BreakError),
    /// Slice bytes are truncated, versioned wrong, over the chunk/bond
    /// caps, non-finite or trailing.
    BadSlice,
}

impl core::fmt::Display for WallSliceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSpec(row) => write!(f, "siege slice spec is out of bounds ({row})"),
            Self::Bonded(err) => write!(f, "siege slice shape refused ({err})"),
            Self::BadSlice => write!(f, "siege slice bytes are not valid"),
        }
    }
}

impl std::error::Error for WallSliceError {}

impl From<BreakError> for WallSliceError {
    fn from(err: BreakError) -> Self {
        Self::Bonded(err)
    }
}

/// Bakes one wall course offline: seeded bisection chunks, contact bonds at
/// `breaking_strain`, the bottom layer anchored. Ship [`slice_bytes`] of
/// the returned descriptor; the runtime never calls this.
pub fn bake_slice(spec: &SiegeSliceSpec) -> Result<BondedDesc, WallSliceError> {
    spec.validate()?;
    let chunks = fracture3(
        spec.cx,
        spec.cy,
        spec.cz,
        spec.half_m,
        spec.half_m,
        spec.half_m,
        spec.pieces,
        spec.seed,
    );
    let mut bonds = Vec::new();
    for a in 0..chunks.len() {
        for b in (a + 1)..chunks.len() {
            if in_contact(&chunks[a], &chunks[b]) {
                bonds.push(Bond {
                    a: wire(a),
                    b: wire(b),
                    breaking_strain: spec.breaking_strain,
                });
            }
        }
    }
    let bottom = chunks
        .iter()
        .map(|c| c.y - c.hy)
        .reduce(f32::min)
        .unwrap_or(spec.cy - spec.half_m);
    let mut anchors = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        if (c.y - c.hy) - bottom <= WALL_ANCHOR_EPS_M {
            anchors.push(wire(i));
        }
    }
    let desc = BondedDesc {
        chunks,
        bonds,
        anchors,
        break_threshold: spec.break_threshold,
    };
    desc.validate()?;
    Ok(desc)
}

/// Canonical slice bytes: version, chunk count plus bit-exact boxes, bond
/// count plus endpoints and limits, anchors, then the threshold bits.
/// Pure over the descriptor: the same bake ships the same bytes twice.
#[must_use]
pub fn slice_bytes(desc: &BondedDesc) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        16 + desc.chunks.len() * 28 + desc.bonds.len() * 12 + desc.anchors.len() * 4,
    );
    out.extend_from_slice(&WALL_SLICE_V1.to_le_bytes());
    out.extend_from_slice(&wire(desc.chunks.len()).to_le_bytes());
    for c in &desc.chunks {
        for w in [
            c.x.to_bits(),
            c.y.to_bits(),
            c.z.to_bits(),
            c.hx.to_bits(),
            c.hy.to_bits(),
            c.hz.to_bits(),
            c.mass.to_bits(),
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    out.extend_from_slice(&wire(desc.bonds.len()).to_le_bytes());
    for b in &desc.bonds {
        out.extend_from_slice(&b.a.to_le_bytes());
        out.extend_from_slice(&b.b.to_le_bytes());
        out.extend_from_slice(&b.breaking_strain.to_bits().to_le_bytes());
    }
    out.extend_from_slice(&wire(desc.anchors.len()).to_le_bytes());
    for a in &desc.anchors {
        out.extend_from_slice(&a.to_le_bytes());
    }
    out.extend_from_slice(&desc.break_threshold.to_bits().to_le_bytes());
    out
}

/// Digest over [`slice_bytes`]: the bake receipt the match log folds.
#[must_use]
pub fn slice_digest(desc: &BondedDesc) -> u64 {
    engine_core::hash_bytes(&slice_bytes(desc))
}

/// Runtime load of shipped slice bytes: parses and validates without RNG
/// and without any clip, so the bytes are the only thing the match needs.
/// On any error nothing is returned and nothing half-applies.
pub fn decode_slice(bytes: &[u8]) -> Result<BondedDesc, WallSliceError> {
    let mut r = Cursor::new(bytes, WallSliceError::BadSlice);
    if r.read_u32()? != WALL_SLICE_V1 {
        return Err(WallSliceError::BadSlice);
    }
    let chunks_len = slot(r.read_u32()?);
    if chunks_len == 0 || chunks_len > MAX_BONDED_CHUNKS {
        return Err(WallSliceError::BadSlice);
    }
    let mut chunks = Vec::with_capacity(chunks_len);
    for _ in 0..chunks_len {
        chunks.push(crate::bonded::Chunk3 {
            x: r.read_finite()?,
            y: r.read_finite()?,
            z: r.read_finite()?,
            hx: r.read_finite()?,
            hy: r.read_finite()?,
            hz: r.read_finite()?,
            mass: r.read_finite()?,
        });
    }
    let bonds_len = slot(r.read_u32()?);
    if bonds_len > MAX_BONDS {
        return Err(WallSliceError::BadSlice);
    }
    let mut bonds = Vec::with_capacity(bonds_len);
    for _ in 0..bonds_len {
        bonds.push(Bond {
            a: r.read_u32()?,
            b: r.read_u32()?,
            breaking_strain: r.read_finite()?,
        });
    }
    let anchors_len = slot(r.read_u32()?);
    if anchors_len > chunks_len {
        return Err(WallSliceError::BadSlice);
    }
    let mut anchors = Vec::with_capacity(anchors_len);
    for _ in 0..anchors_len {
        anchors.push(r.read_u32()?);
    }
    let break_threshold = r.read_finite()?;
    if !r.is_done() {
        return Err(WallSliceError::BadSlice);
    }
    let desc = BondedDesc {
        chunks,
        bonds,
        anchors,
        break_threshold,
    };
    desc.validate()?;
    Ok(desc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bonded::BondedCollection;

    fn spec(seed: u64) -> SiegeSliceSpec {
        SiegeSliceSpec {
            cx: 60.0,
            cy: 2.0,
            cz: 0.0,
            half_m: 2.0,
            pieces: 8,
            seed,
            breaking_strain: 700.0,
            break_threshold: crate::bake::BREAKING_THRESHOLD,
        }
    }

    #[test]
    fn bytes_x2_and_seed_moves_the_digest() {
        let first = bake_slice(&spec(99)).expect("wall slice bakes");
        let second = bake_slice(&spec(99)).expect("wall slice bakes");
        assert_eq!(first.chunks.len(), 8);
        assert_eq!(slice_bytes(&first), slice_bytes(&second), "bytes x2");
        assert_eq!(slice_digest(&first), slice_digest(&second));
        let other = bake_slice(&spec(100)).expect("wall slice bakes");
        assert_ne!(
            slice_bytes(&first),
            slice_bytes(&other),
            "another seed, another slice"
        );
        assert_ne!(slice_digest(&first), slice_digest(&other));
        // The bake holds the course box: mass conserved, bottom anchored.
        let total: f32 = first.chunks.iter().map(|c| c.mass).sum();
        let expect = 8.0 * 2.0f32.powi(3);
        assert!(
            (total - expect).abs() < 0.5,
            "mass conserved: {total} against {expect}"
        );
        assert!(!first.anchors.is_empty(), "the course sits on anchors");
    }

    #[test]
    fn decoded_bytes_serve_the_consumer_path_byte_equal() {
        // The shipped-bytes path (decode, build, hit, step) matches the
        // baked path exactly, so arena/game matches that load the bytes
        // replay the bake with no clip at runtime.
        let baked = bake_slice(&spec(7)).expect("wall slice bakes");
        let bytes = slice_bytes(&baked);
        assert_eq!(bytes, slice_bytes(&baked), "encode is stable");
        let shipped = decode_slice(&bytes).expect("shipped bytes load");
        assert_eq!(slice_bytes(&shipped), bytes, "round trip is byte-equal");
        let mut live = BondedCollection::new(&baked).expect("valid slice");
        let mut loaded = BondedCollection::new(&shipped).expect("valid slice");
        assert_eq!(loaded.state_hash(), live.state_hash());
        live.apply_impact(0, 1e9, 3).expect("hit lands");
        loaded.apply_impact(0, 1e9, 3).expect("hit lands");
        assert_eq!(loaded.drain_events(), live.drain_events());
        live.step(1.0 / 60.0);
        loaded.step(1.0 / 60.0);
        assert_eq!(loaded.state_hash(), live.state_hash());
        assert_eq!(loaded.drain_events(), live.drain_events());
    }

    #[test]
    fn bad_specs_refuse_before_any_bake() {
        // `BondedDesc` has no `PartialEq`, so failing bakes assert on the
        // typed error they carry, never on the descriptor.
        let mut bad = spec(1);
        bad.pieces = 0;
        assert_eq!(
            bake_slice(&bad).unwrap_err(),
            WallSliceError::BadSpec("pieces")
        );
        bad = spec(1);
        bad.pieces = MAX_SLICE_PIECES + 1;
        assert_eq!(
            bake_slice(&bad).unwrap_err(),
            WallSliceError::BadSpec("pieces")
        );
        bad = spec(1);
        bad.half_m = 0.0;
        assert_eq!(
            bake_slice(&bad).unwrap_err(),
            WallSliceError::BadSpec("half_m")
        );
        bad = spec(1);
        bad.cx = f32::NAN;
        assert_eq!(
            bake_slice(&bad).unwrap_err(),
            WallSliceError::BadSpec("center")
        );
        bad = spec(1);
        bad.breaking_strain = f32::NAN;
        assert_eq!(
            bake_slice(&bad).unwrap_err(),
            WallSliceError::BadSpec("breaking_strain")
        );
        bad = spec(1);
        bad.break_threshold = 0.0;
        assert_eq!(
            bake_slice(&bad).unwrap_err(),
            WallSliceError::BadSpec("break_threshold")
        );
        assert_eq!(spec(1).validate(), Ok(()));
    }

    #[test]
    fn hostile_bytes_refuse() {
        let bytes = slice_bytes(&bake_slice(&spec(7)).expect("wall slice bakes"));
        for cut in 0..bytes.len() {
            assert!(
                decode_slice(&bytes[..cut]).is_err(),
                "truncated at {cut} must refuse"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            decode_slice(&trailing).unwrap_err(),
            WallSliceError::BadSlice
        );
        let mut bad_version = bytes.clone();
        bad_version[0] = 9;
        assert_eq!(
            decode_slice(&bad_version).unwrap_err(),
            WallSliceError::BadSlice
        );
        let mut absurd = bytes.clone();
        let many = u32::try_from(MAX_BONDED_CHUNKS + 1).expect("slice caps fit u32");
        absurd[4..8].copy_from_slice(&many.to_le_bytes());
        assert_eq!(decode_slice(&absurd).unwrap_err(), WallSliceError::BadSlice);
        // A NaN box center is refused.
        let mut nan = bytes.clone();
        nan[8..12].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert!(decode_slice(&nan).is_err());
        assert!(decode_slice(&bytes).is_ok());
    }
}
