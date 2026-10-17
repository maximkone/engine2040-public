//! sever_cook: watertight sever templates plus stump caps per body segment.
//!
//! Planned consumer (BX7 cut presentation, not wired yet): the GPU cut
//! presentation will read the cooked interior and cap rings
//! from `SeverCookOutput` directly, and the
//! anatomy joints name the segments. Units: millimetres for positions, counts
//! for verts/tris. Determinism: segments sort by name, edges sort by key, the
//! digest is the crate-canonical content hash over a little-endian encoding,
//! so the same rig cooks byte-identical output on every PC. Errors: typed
//! [`SeverCookError`]; an open edge, a doubled edge or a flipped normal
//! refuses the cook loudly.

use std::collections::BTreeMap;
use std::fmt;

/// Most body segments one character cook accepts; bounds the cooked payload.
pub const MAX_SEGMENTS: usize = 64;
/// Most vertices one segment mesh carries; bounds edge-table work.
pub const MAX_VERTS: usize = 4096;
/// Most triangles one segment mesh carries; bounds edge-table work.
pub const MAX_TRIS: usize = 8192;
/// Longest accepted segment name in chars; longer names are refused.
pub const MAX_NAME_CHARS: usize = 64;

/// One closed body-segment shell in integer millimetres with one outward
/// sign per triangle (`1` means outward, anything else is a flipped normal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentMesh {
    /// Segment name (`torso`, `forearm_l`); sorted for determinism.
    pub name: String,
    /// Vertex positions in integer millimetres, parent-rig frame.
    pub verts: Vec<[i32; 3]>,
    /// Closed triangle shell, one index triple per face.
    pub tris: Vec<[u32; 3]>,
    /// Per-triangle outward sign; every entry must be `1`.
    pub outward: Vec<i8>,
}

/// Cooked sever data for one segment: the validated interior wall plus the
/// stump cap that closes a cut. Both shells are watertight by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeveredSegment {
    /// Segment name, carried from the input mesh.
    pub name: String,
    /// Interior wall vertices (integer millimetres, rig frame).
    pub interior_verts: Vec<[i32; 3]>,
    /// Interior wall triangles (validated closed shell).
    pub interior_tris: Vec<[u32; 3]>,
    /// Stump cap vertices (integer millimetres, rig frame).
    pub cap_verts: Vec<[i32; 3]>,
    /// Stump cap triangles (mirrored winding, validated closed shell).
    pub cap_tris: Vec<[u32; 3]>,
}

/// Whole-character sever cook: name-sorted segments plus the content digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeverCookOutput {
    /// Cooked segments in ascending name order.
    pub segments: Vec<SeveredSegment>,
    /// Crate-canonical hash over the sorted segments (names, verts, tris).
    pub digest: u64,
}

/// Why a sever cook failed; every variant names the offending segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeverCookError {
    /// No segments were offered at all.
    Empty,
    /// More segments than [`MAX_SEGMENTS`]; count carried.
    TooManySegments {
        /// Offered segment count.
        count: usize,
    },
    /// Two segments share a name; the dupe name carried (bounded).
    DuplicateName {
        /// Repeated segment name.
        name: String,
    },
    /// A name exceeds [`MAX_NAME_CHARS`]; char count carried.
    NameTooLong {
        /// Truncated offending name.
        name: String,
        /// Full name length in chars.
        chars: usize,
    },
    /// A segment carries more than [`MAX_VERTS`] verts; count carried.
    TooManyVerts {
        /// Offending segment name.
        segment: String,
        /// Offered vertex count.
        count: usize,
    },
    /// A segment carries more than [`MAX_TRIS`] tris; count carried.
    TooManyTris {
        /// Offending segment name.
        segment: String,
        /// Offered triangle count.
        count: usize,
    },
    /// `outward` length differs from `tris` length; both carried.
    FaceSignCount {
        /// Offending segment name.
        segment: String,
        /// Triangle count.
        tris: usize,
        /// Sign count.
        signs: usize,
    },
    /// An index addresses no vertex; culprit index carried.
    IndexOutOfRange {
        /// Offending segment name.
        segment: String,
        /// Out-of-range index.
        index: u32,
    },
    /// An edge has one incident triangle (open boundary); edge carried.
    NonWatertight {
        /// Offending segment name.
        segment: String,
        /// Boundary edge `(lo, hi)`.
        edge: [u32; 2],
    },
    /// An edge has three or more incident triangles; edge carried.
    NonManifold {
        /// Offending segment name.
        segment: String,
        /// Doubled edge `(lo, hi)`.
        edge: [u32; 2],
    },
    /// A triangle carries a non-outward sign; triangle ordinal carried.
    FlippedNormal {
        /// Offending segment name.
        segment: String,
        /// Triangle ordinal with the bad sign.
        tri: u32,
    },
    /// Two triangles cross an edge in the same direction; edge carried.
    InconsistentWinding {
        /// Offending segment name.
        segment: String,
        /// Same-direction edge `(lo, hi)`.
        edge: [u32; 2],
    },
}

impl fmt::Display for SeverCookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "sever cook refused: no segments offered"),
            Self::TooManySegments { count } => {
                write!(
                    f,
                    "sever cook refused: {count} segments over {MAX_SEGMENTS}"
                )
            }
            Self::DuplicateName { name } => {
                write!(f, "sever cook refused: duplicate segment '{name}'")
            }
            Self::NameTooLong { name, chars } => write!(
                f,
                "sever cook refused '{name}': {chars} chars over {MAX_NAME_CHARS}"
            ),
            Self::TooManyVerts { segment, count } => write!(
                f,
                "sever cook refused '{segment}': {count} verts over {MAX_VERTS}"
            ),
            Self::TooManyTris { segment, count } => write!(
                f,
                "sever cook refused '{segment}': {count} tris over {MAX_TRIS}"
            ),
            Self::FaceSignCount {
                segment,
                tris,
                signs,
            } => write!(
                f,
                "sever cook refused '{segment}': {signs} normal signs for {tris} tris"
            ),
            Self::IndexOutOfRange { segment, index } => write!(
                f,
                "sever cook refused '{segment}': index {index} out of range"
            ),
            Self::NonWatertight { segment, edge } => write!(
                f,
                "sever cook refused '{segment}': open boundary on edge {}-{}",
                edge[0], edge[1]
            ),
            Self::NonManifold { segment, edge } => write!(
                f,
                "sever cook refused '{segment}': non-manifold edge {}-{}",
                edge[0], edge[1]
            ),
            Self::FlippedNormal { segment, tri } => write!(
                f,
                "sever cook refused '{segment}': flipped normal on tri {tri}"
            ),
            Self::InconsistentWinding { segment, edge } => write!(
                f,
                "sever cook refused '{segment}': inconsistent winding on edge {}-{}",
                edge[0], edge[1]
            ),
        }
    }
}

impl std::error::Error for SeverCookError {}

/// Append a `u64` as eight little-endian bytes (platform-independent).
fn push_u64(buf: &mut Vec<u8>, value: u64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// Append an `i32` as four little-endian bytes (platform-independent).
fn push_i32(buf: &mut Vec<u8>, value: i32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// Append a name as its byte length plus its UTF-8 bytes, so adjacent names
/// cannot collide across a split point.
fn push_name(buf: &mut Vec<u8>, name: &str) {
    push_u64(buf, u64::try_from(name.len()).unwrap_or(u64::MAX));
    buf.extend_from_slice(name.as_bytes());
}

/// Shorten a name for diagnostics without echoing unbounded input.
fn short_name(name: &str) -> String {
    let mut kept = String::new();
    for (slot, ch) in name.chars().enumerate() {
        if slot >= MAX_NAME_CHARS {
            break;
        }
        kept.push(ch);
    }
    kept
}

/// Bound plus index checks for one segment (no float, no tolerance).
fn check_limits(segment: &SegmentMesh) -> Result<(), SeverCookError> {
    let chars = segment.name.chars().count();
    if chars > MAX_NAME_CHARS {
        return Err(SeverCookError::NameTooLong {
            name: short_name(&segment.name),
            chars,
        });
    }
    if segment.verts.len() > MAX_VERTS {
        return Err(SeverCookError::TooManyVerts {
            segment: short_name(&segment.name),
            count: segment.verts.len(),
        });
    }
    if segment.tris.len() > MAX_TRIS {
        return Err(SeverCookError::TooManyTris {
            segment: short_name(&segment.name),
            count: segment.tris.len(),
        });
    }
    if segment.outward.len() != segment.tris.len() {
        return Err(SeverCookError::FaceSignCount {
            segment: short_name(&segment.name),
            tris: segment.tris.len(),
            signs: segment.outward.len(),
        });
    }
    let verts = u32::try_from(segment.verts.len()).unwrap_or(u32::MAX);
    for tri in &segment.tris {
        for index in tri {
            if *index >= verts {
                return Err(SeverCookError::IndexOutOfRange {
                    segment: short_name(&segment.name),
                    index: *index,
                });
            }
        }
    }
    Ok(())
}

/// Integer watertight proof: every undirected edge appears exactly twice, in
/// opposite directions, and every normal sign reads outward. Pure
/// integer/structural work over sorted edges; no float tolerance anywhere.
fn check_watertight(segment: &SegmentMesh) -> Result<(), SeverCookError> {
    let name = short_name(&segment.name);
    for (slot, sign) in segment.outward.iter().enumerate() {
        if *sign != 1 {
            let tri = u32::try_from(slot).unwrap_or(u32::MAX);
            return Err(SeverCookError::FlippedNormal { segment: name, tri });
        }
    }
    let mut edges: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
    for tri in &segment.tris {
        let corners = [tri[0], tri[1], tri[2]];
        for (from, to) in [
            (corners[0], corners[1]),
            (corners[1], corners[2]),
            (corners[2], corners[0]),
        ] {
            let key = if from <= to { (from, to) } else { (to, from) };
            edges.entry(key).or_default().push((from, to));
        }
    }
    for (key, dirs) in &edges {
        let edge = [key.0, key.1];
        if dirs.len() == 1 {
            return Err(SeverCookError::NonWatertight {
                segment: name,
                edge,
            });
        }
        if dirs.len() > 2 {
            return Err(SeverCookError::NonManifold {
                segment: name,
                edge,
            });
        }
        if dirs[0] == dirs[1] {
            return Err(SeverCookError::InconsistentWinding {
                segment: name,
                edge,
            });
        }
    }
    Ok(())
}

/// Validate one segment: bounds, indices, then the watertight proof.
pub fn validate_segment(segment: &SegmentMesh) -> Result<(), SeverCookError> {
    check_limits(segment)?;
    check_watertight(segment)?;
    Ok(())
}

/// Canonical byte encoding of name-sorted segments: counts, names and
/// integer geometry in little-endian order. The digest hashes exactly this.
fn encode_sorted(segments: &[&SeveredSegment]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    push_u64(&mut buf, u64::try_from(segments.len()).unwrap_or(u64::MAX));
    for segment in segments {
        push_name(&mut buf, &segment.name);
        push_u64(
            &mut buf,
            u64::try_from(segment.interior_verts.len()).unwrap_or(u64::MAX),
        );
        for vert in &segment.interior_verts {
            push_i32(&mut buf, vert[0]);
            push_i32(&mut buf, vert[1]);
            push_i32(&mut buf, vert[2]);
        }
        push_u64(
            &mut buf,
            u64::try_from(segment.interior_tris.len()).unwrap_or(u64::MAX),
        );
        for tri in &segment.interior_tris {
            push_u64(&mut buf, u64::from(tri[0]));
            push_u64(&mut buf, u64::from(tri[1]));
            push_u64(&mut buf, u64::from(tri[2]));
        }
    }
    buf
}

/// Digest over name-sorted segments through the crate-canonical content hash,
/// so sever digests share the one hash owner with the rest of the crate.
fn digest_sorted(segments: &[&SeveredSegment]) -> u64 {
    crate::registry::content_hash(&encode_sorted(segments))
}

/// Cook every segment: validate, emit the interior wall plus a mirrored stump
/// cap, and digest the sorted set. Duplicate names are refused.
///
/// The cap mirrors each triangle's winding, so it is closed exactly when the
/// input is closed; both shells pass [`validate_segment`] before the output
/// lands, which is what the render cut path reads.
pub fn cook_character(segments: &[SegmentMesh]) -> Result<SeverCookOutput, SeverCookError> {
    if segments.is_empty() {
        return Err(SeverCookError::Empty);
    }
    if segments.len() > MAX_SEGMENTS {
        return Err(SeverCookError::TooManySegments {
            count: segments.len(),
        });
    }
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| segments[a].name.cmp(&segments[b].name));
    for pair in order.windows(2) {
        if segments[pair[0]].name == segments[pair[1]].name {
            return Err(SeverCookError::DuplicateName {
                name: short_name(&segments[pair[0]].name),
            });
        }
    }
    let mut cooked: Vec<SeveredSegment> = Vec::with_capacity(order.len());
    for &slot in &order {
        let input = &segments[slot];
        validate_segment(input)?;
        let cap_tris: Vec<[u32; 3]> = input
            .tris
            .iter()
            .map(|tri| [tri[2], tri[1], tri[0]])
            .collect();
        let cap_probe = SegmentMesh {
            name: input.name.clone(),
            verts: input.verts.clone(),
            tris: cap_tris.clone(),
            outward: input.outward.clone(),
        };
        check_watertight(&cap_probe)?;
        cooked.push(SeveredSegment {
            name: input.name.clone(),
            interior_verts: input.verts.clone(),
            interior_tris: input.tris.clone(),
            cap_verts: input.verts.clone(),
            cap_tris,
        });
    }
    let refs: Vec<&SeveredSegment> = cooked.iter().collect();
    let digest = digest_sorted(&refs);
    Ok(SeverCookOutput {
        segments: cooked,
        digest,
    })
}

/// Cube shell corners in integer millimetres (unit cube, origin at min).
fn cube_verts(height_mm: i32) -> Vec<[i32; 3]> {
    vec![
        [0, 0, 0],
        [1000, 0, 0],
        [1000, height_mm, 0],
        [0, height_mm, 0],
        [0, 0, 1000],
        [1000, 0, 1000],
        [1000, height_mm, 1000],
        [0, height_mm, 1000],
    ]
}

/// Closed cube shell: twelve triangles, every edge paired in opposition.
fn cube_tris() -> Vec<[u32; 3]> {
    vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [3, 7, 6],
        [3, 6, 2],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
    ]
}

/// Tetrahedron shell corners in integer millimetres.
fn tetra_verts() -> Vec<[i32; 3]> {
    vec![[0, 0, 0], [1000, 0, 0], [0, 1000, 0], [0, 0, 1000]]
}

/// Closed tetrahedron shell: four triangles, every edge paired in opposition.
fn tetra_tris() -> Vec<[u32; 3]> {
    vec![[0, 2, 1], [0, 3, 2], [0, 1, 3], [1, 2, 3]]
}

/// Square-pyramid shell corners in integer millimetres.
fn pyramid_verts() -> Vec<[i32; 3]> {
    vec![
        [0, 0, 0],
        [1000, 0, 0],
        [1000, 1000, 0],
        [0, 1000, 0],
        [500, 500, 1000],
    ]
}

/// Closed square-pyramid shell: six triangles, edges paired in opposition.
fn pyramid_tris() -> Vec<[u32; 3]> {
    vec![
        [0, 2, 1],
        [0, 3, 2],
        [0, 1, 4],
        [1, 2, 4],
        [2, 3, 4],
        [3, 0, 4],
    ]
}

/// Outward signs for a shell with the given triangle count (all outward).
fn outward_all(tris: usize) -> Vec<i8> {
    vec![1; tris]
}

/// Build one named segment from raw parts with all-outward signs.
fn segment(name: &str, verts: Vec<[i32; 3]>, tris: Vec<[u32; 3]>) -> SegmentMesh {
    let outward = outward_all(tris.len());
    SegmentMesh {
        name: name.to_string(),
        verts,
        tris,
        outward,
    }
}

/// Rome arena rig: torso block, forearm tetra, thigh pyramid.
pub fn rome_arena_segments() -> Vec<SegmentMesh> {
    vec![
        segment("forearm_l", tetra_verts(), tetra_tris()),
        segment("thigh_r", pyramid_verts(), pyramid_tris()),
        segment("torso", cube_verts(1000), cube_tris()),
    ]
}

/// Urban officer rig: taller torso block, forearm tetra, shin pyramid.
pub fn urban_officer_segments() -> Vec<SegmentMesh> {
    vec![
        segment("forearm_l", tetra_verts(), tetra_tris()),
        segment("shin_r", pyramid_verts(), pyramid_tris()),
        segment("torso", cube_verts(2000), cube_tris()),
    ]
}

/// Cook the Rome arena rig into interior templates plus stump caps.
pub fn cook_rome_arena() -> Result<SeverCookOutput, SeverCookError> {
    cook_character(&rome_arena_segments())
}

/// Cook the urban officer rig into interior templates plus stump caps.
pub fn cook_urban_officer() -> Result<SeverCookOutput, SeverCookError> {
    cook_character(&urban_officer_segments())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_tetra() -> SegmentMesh {
        let mut shell = segment("forearm_l", tetra_verts(), tetra_tris());
        shell.tris.pop();
        shell.outward.pop();
        shell
    }

    fn flipped_sign_cube() -> SegmentMesh {
        let mut shell = segment("torso", cube_verts(1000), cube_tris());
        shell.outward[0] = -1;
        shell
    }

    fn flipped_winding_cube() -> SegmentMesh {
        let mut shell = segment("torso", cube_verts(1000), cube_tris());
        shell.tris[0] = [1, 2, 0];
        shell
    }

    #[test]
    fn both_rigs_cook_with_sorted_segments() {
        let rome = cook_rome_arena().expect("arena rig is closed");
        let officer = cook_urban_officer().expect("officer rig is closed");
        assert_eq!(rome.segments.len(), 3);
        assert_eq!(officer.segments.len(), 3);
        for output in [&rome, &officer] {
            let names: Vec<&str> = output.segments.iter().map(|s| s.name.as_str()).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted, "segments cook in name order");
            assert_ne!(output.digest, 0, "digest carries content");
        }
        assert_ne!(
            rome.digest, officer.digest,
            "distinct rigs digest distinctly"
        );
    }

    #[test]
    fn digest_identical_twice() {
        let first = cook_rome_arena().expect("first cook lands");
        let second = cook_rome_arena().expect("second cook lands");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first, second, "whole output repeats bit-identical");
    }

    #[test]
    fn interior_and_cap_templates_are_watertight() {
        for output in [
            cook_rome_arena().expect("arena lands"),
            cook_urban_officer().expect("officer lands"),
        ] {
            assert!(!output.segments.is_empty());
            for segment in &output.segments {
                let interior = SegmentMesh {
                    name: segment.name.clone(),
                    verts: segment.interior_verts.clone(),
                    tris: segment.interior_tris.clone(),
                    outward: outward_all(segment.interior_tris.len()),
                };
                validate_segment(&interior).expect("interior template stays watertight");
                let cap = SegmentMesh {
                    name: segment.name.clone(),
                    verts: segment.cap_verts.clone(),
                    tris: segment.cap_tris.clone(),
                    outward: outward_all(segment.cap_tris.len()),
                };
                validate_segment(&cap).expect("stump cap stays watertight");
                assert_eq!(
                    segment.cap_tris.len(),
                    segment.interior_tris.len(),
                    "one cap face per wall face"
                );
            }
        }
    }

    #[test]
    fn open_segment_fails_loud() {
        let err = cook_character(&[open_tetra()]).expect_err("open shell must fail");
        assert!(
            matches!(err, SeverCookError::NonWatertight { .. }),
            "open edge is NonWatertight, got {err}"
        );
        assert!(
            err.to_string().contains("forearm_l"),
            "refusal names the segment: {err}"
        );
    }

    #[test]
    fn flipped_sign_fails_loud() {
        let err = cook_character(&[flipped_sign_cube()]).expect_err("flipped sign must fail");
        assert!(
            matches!(err, SeverCookError::FlippedNormal { tri: 0, .. }),
            "bad sign is FlippedNormal tri 0, got {err}"
        );
        assert!(err.to_string().contains("torso"), "refusal names it: {err}");
    }

    #[test]
    fn flipped_winding_fails_loud() {
        let err = cook_character(&[flipped_winding_cube()]).expect_err("winding must fail");
        assert!(
            matches!(err, SeverCookError::InconsistentWinding { .. }),
            "same-direction edge is InconsistentWinding, got {err}"
        );
    }

    #[test]
    fn canonical_encoding_repeats_bit_identical() {
        let first = cook_rome_arena().expect("first cook lands");
        let second = cook_rome_arena().expect("second cook lands");
        let a: Vec<&SeveredSegment> = first.segments.iter().collect();
        let b: Vec<&SeveredSegment> = second.segments.iter().collect();
        assert_eq!(encode_sorted(&a), encode_sorted(&b));
        assert!(!encode_sorted(&a).is_empty(), "encoding carries content");
    }
}
