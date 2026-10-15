//! exposure: continuous cover-exposure scalar per walkable tile.
//! Owner: spatial crate. Consumer: cover-approach ranking in `afford`,
//! wired in-test below against the real `afford` and `sight` APIs.
//! Units: threat eyes meters in, integer cm inside, permille 0..=1000 out.
//! Determinism: tile-sorted output, threat-order-invariant counts, integer
//! math only; same inputs twice give the same rows and digest.
//! Errors: typed `ExposureError`; empty or non-finite threats refuse loudly.
//! Advisory: scalars rank cover only, never gate placement or enter a digest.
//! Rays reuse the sight slab test (parry ideas-only, no new dependency).
//! Budget: room-scale brief work, never the tick; at most 0.2 ms per tile
//! with one threat over up to 500 solids (timing probe in tests below);
//! each extra threat adds one linear fan.

use super::index::CM_PER_M;
use super::navcheck::TILE_CM;
use super::sight::{los_point, SightWorld, STAND_TOP_CM};
use engine_core::Fnv64;
use std::collections::BTreeSet;
use std::fmt;

/// Largest exposure: every threat ray reaches the tile.
pub const EXPOSURE_MAX_PERMILLE: u16 = 1000;
/// Advisory split: exposed at or above, covered below. Ranking uses the raw
/// scalar; the split only names hand labels, never gates.
pub const EXPOSED_AT_PERMILLE: u16 = 500;
/// Threat-ray fan: one ray per offset pair, so 9 rays per threat per tile.
/// Offsets stay inside the 100 cm tile (|33| < 50).
pub const FAN_OFFSETS_CM: [i32; 3] = [-33, 0, 33];
/// Digest tag so exposure bytes never alias another domain hash.
const EXPOSURE_TAG: &[u8] = b"e2exposure:1";

/// What stopped an exposure query. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExposureError {
    /// No threat eye was given; with nothing to hide from every tile would
    /// read zero, so the caller passes at least one eye instead.
    NoThreats,
    /// One threat eye has a non-finite coordinate; the index names it.
    NonFiniteThreat(usize),
}

impl fmt::Display for ExposureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoThreats => write!(f, "exposure: needs at least one threat eye"),
            Self::NonFiniteThreat(i) => write!(f, "exposure: threat {i} is not finite"),
        }
    }
}

impl std::error::Error for ExposureError {}

/// Cover exposure of one walkable tile: integer permille of threat rays that
/// reach it. 0 hides from every ray, 1000 stands in the open to every ray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileExposure {
    /// Bake tile coordinate.
    pub tile: (i32, i32),
    /// Reaching rays per mille, 0..=1000.
    pub exposure_permille: u16,
}

/// Integer cm to meters. Arena coordinates stay within +-1,000,000 cm, where
/// the f64 division is bit-identical everywhere, so the sight quantizer maps
/// the value back to the same centimeter.
fn cm_to_m(cm: i32) -> f32 {
    f64_to_f32(f64::from(cm) / f64::from(CM_PER_M))
}

/// Total f64 to f32 with round-to-nearest-even: bit-identical to the numeric
/// cast over finite inputs, with no cast and no panic. Small finite quotients
/// take the normal path; the zero, subnormal, overflow and non-finite arms
/// keep the function total for every input.
fn f64_to_f32(v: f64) -> f32 {
    let bits = v.to_bits();
    let sign = u32::try_from((bits >> 32) & 0x8000_0000).unwrap_or(0);
    let exp_raw = u32::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    let mant = bits & 0x000F_FFFF_FFFF_FFFF;
    if exp_raw == 0 {
        // Zero, or a subnormal far below the smallest positive f32 value:
        // both read as signed zero.
        return f32::from_bits(sign);
    }
    if exp_raw == 0x7FF {
        if mant == 0 {
            return f32::from_bits(sign | 0x7F80_0000);
        }
        let payload = u32::try_from(mant >> 29).unwrap_or(0) | 0x0040_0000;
        return f32::from_bits(sign | 0x7F80_0000 | payload);
    }
    let exp_unbiased = i64::from(exp_raw) - 1023;
    let exp32 = exp_unbiased + 127;
    if exp32 >= 255 {
        return f32::from_bits(sign | 0x7F80_0000);
    }
    if exp32 <= 0 {
        // f32 subnormal: scale the 53-bit significand down with a sticky-bit
        // round. Unreachable from arena quotients, which read at least 0.01.
        let sig = (1u64 << 52) | mant;
        let shift = u32::try_from(-(exp_unbiased + 97)).unwrap_or(64);
        if shift > 53 {
            return f32::from_bits(sign);
        }
        let kept = u32::try_from(sig >> shift).unwrap_or(0);
        let dropped = sig & ((1u64 << shift) - 1);
        let half = 1u64 << (shift - 1);
        let round_up = dropped > half || (dropped == half && (kept & 1) == 1);
        let rounded = kept + u32::from(round_up);
        if rounded == 0x0080_0000 {
            return f32::from_bits(sign | (1u32 << 23));
        }
        return f32::from_bits(sign | rounded);
    }
    let kept = u32::try_from(mant >> 29).unwrap_or(0);
    let dropped = mant & 0x1FFF_FFFF;
    let round_up = dropped > 0x1000_0000 || (dropped == 0x1000_0000 && (kept & 1) == 1);
    let mut mant32 = kept + u32::from(round_up);
    let mut exp_out = exp32;
    if mant32 == 0x0080_0000 {
        mant32 = 0;
        exp_out += 1;
    }
    if exp_out >= 255 {
        return f32::from_bits(sign | 0x7F80_0000);
    }
    let exp_bits = u32::try_from(exp_out).unwrap_or(0) << 23;
    f32::from_bits(sign | exp_bits | mant32)
}

/// Exposure of one tile: clear rays over the threat fan in permille.
/// Probes stand at standing head height over the tile center fan; bake tiles
/// carry no heights, so single-floor rooms read over y = 0. A probe inside a
/// solid reads clear (shared endpoint-exclusion semantics); callers feed
/// walkable tiles only.
fn tile_permille(world: &SightWorld, threats_m: &[[f32; 3]], tile: (i32, i32)) -> u16 {
    let cx = tile.0.saturating_mul(TILE_CM).saturating_add(TILE_CM / 2);
    let cz = tile.1.saturating_mul(TILE_CM).saturating_add(TILE_CM / 2);
    let mut clear: u64 = 0;
    let mut total: u64 = 0;
    for eye in threats_m {
        for dx in FAN_OFFSETS_CM {
            for dz in FAN_OFFSETS_CM {
                let probe = [
                    cm_to_m(cx.saturating_add(dx)),
                    cm_to_m(STAND_TOP_CM),
                    cm_to_m(cz.saturating_add(dz)),
                ];
                total += 1;
                // Eyes arrive validated and probes are small-int meters, so the
                // point quantizer never refuses; a refusal would read blocked.
                if los_point(world, *eye, probe).unwrap_or(false) {
                    clear += 1;
                }
            }
        }
    }
    // Clear never exceeds total, so the quotient caps at 1000 and always fits;
    // the fallback below never runs.
    u16::try_from(clear * 1000 / total).unwrap_or(EXPOSURE_MAX_PERMILLE)
}

/// Cover exposure per tile from threat-ray fans.
///
/// Every threat fans 9 rays onto every tile; the scalar is the reaching share
/// in permille. Output is tile-sorted with duplicates collapsed, so any input
/// order and any threat order give the same rows. Empty tiles read empty;
/// empty or non-finite threats refuse with [`ExposureError`].
pub fn exposure_per_tile(
    world: &SightWorld,
    threats_m: &[[f32; 3]],
    tiles: &[(i32, i32)],
) -> Result<Vec<TileExposure>, ExposureError> {
    if threats_m.is_empty() {
        return Err(ExposureError::NoThreats);
    }
    for (i, eye) in threats_m.iter().enumerate() {
        if !eye.iter().all(|v| v.is_finite()) {
            return Err(ExposureError::NonFiniteThreat(i));
        }
    }
    let ordered: BTreeSet<(i32, i32)> = tiles.iter().copied().collect();
    let mut out = Vec::with_capacity(ordered.len());
    for tile in ordered {
        out.push(TileExposure {
            tile,
            exposure_permille: tile_permille(world, threats_m, tile),
        });
    }
    Ok(out)
}

/// Order-invariant digest over `(tile, permille)`: sorted by tile, then tag
/// plus LE bytes. Same rows in any order give the same digest; any flipped
/// scalar flips it. No float ever reaches the digest.
pub fn digest_exposures(rows: &[TileExposure]) -> u64 {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|r| r.tile);
    let mut hash = Fnv64::new();
    hash.write(EXPOSURE_TAG);
    for r in &sorted {
        hash.write_i32(r.tile.0);
        hash.write_i32(r.tile.1);
        hash.write(&r.exposure_permille.to_le_bytes());
    }
    hash.finish()
}

/// Brief rows for exposures: `exposure <tx>,<tz> <permille>` in tile order.
/// Advisory text beside the brief, never into its digest.
pub fn exposure_lines(rows: &[TileExposure]) -> Vec<String> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|r| r.tile);
    sorted
        .iter()
        .map(|r| {
            format!(
                "exposure {},{} {}\n",
                r.tile.0, r.tile.1, r.exposure_permille
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sight::SightWorld;
    use crate::Actor;

    fn prop(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: format!("p{id}"),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    /// One tall wall at x = 5 m: x[450,550], y[0,300], z[-500,500] (cm).
    /// Height 0 removes the wall (open field) without moving anything else.
    fn wall_room(height_m: f32) -> SightWorld {
        if height_m <= 0.0 {
            SightWorld::build(&[])
        } else {
            SightWorld::build(&[prop(1, 5.0, height_m / 2.0, 0.0, 1.0, height_m, 10.0)])
        }
    }

    /// Threat eye east of the wall, at standing head height.
    fn threat_east() -> [f32; 3] {
        [30.0, 1.5, 0.0]
    }

    /// Threat eye west of the wall, mirroring the east one.
    fn threat_west() -> [f32; 3] {
        [-20.0, 1.5, 0.0]
    }

    /// Hand-labeled tiles with margin from the shadow wedge: west of the wall
    /// hides from the east threat, east stands open, the far ends see around.
    /// 40 labels: 18 covered + 22 exposed.
    fn hand_labels() -> Vec<((i32, i32), bool)> {
        let mut out = Vec::new();
        for x in [1, 2] {
            for z in -4..=4 {
                out.push(((x, z), false));
            }
        }
        for x in [7, 8] {
            for z in -4..=4 {
                out.push(((x, z), true));
            }
        }
        for tile in [(5, 6), (6, 6), (5, -6), (6, -6)] {
            out.push((tile, true));
        }
        out
    }

    fn labeled_tiles() -> Vec<(i32, i32)> {
        hand_labels().iter().map(|(t, _)| *t).collect()
    }

    #[test]
    fn hand_label_agreement_rate() {
        let world = wall_room(3.0);
        let rows = exposure_per_tile(&world, &[threat_east()], &labeled_tiles())
            .expect("labeled room answers");
        assert_eq!(rows.len(), 40, "fixture holds 40 labels");
        let by_tile: std::collections::BTreeMap<(i32, i32), u16> =
            rows.iter().map(|r| (r.tile, r.exposure_permille)).collect();
        let mut failures: Vec<String> = Vec::new();
        for (tile, want_exposed) in hand_labels() {
            let got = by_tile[&tile] >= EXPOSED_AT_PERMILLE;
            if got != want_exposed {
                failures.push(format!("tile {tile:?}: permille {}", by_tile[&tile]));
            }
        }
        let pass = hand_labels().len() - failures.len();
        assert!(
            pass * 100 >= 90 * hand_labels().len(),
            "exposure labels {pass}/{} < 90%: {failures:?}",
            hand_labels().len()
        );
        assert!(failures.is_empty(), "label failures: {failures:?}");
    }

    #[test]
    fn open_field_reads_fully_exposed() {
        let world = wall_room(0.0);
        let rows = exposure_per_tile(&world, &[threat_east()], &labeled_tiles())
            .expect("open field answers");
        for r in &rows {
            assert_eq!(
                r.exposure_permille, EXPOSURE_MAX_PERMILLE,
                "tile {:?} open to the threat",
                r.tile
            );
        }
    }

    #[test]
    fn two_sided_threats_read_halfway() {
        // Same tile hides from the east threat and stands open to the west
        // one: the scalar lands exactly mid-scale, the continuity proof.
        let world = wall_room(3.0);
        let rows = exposure_per_tile(&world, &[threat_east(), threat_west()], &[(2, 0)])
            .expect("two threats answer");
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].exposure_permille > 0 && rows[0].exposure_permille < EXPOSURE_MAX_PERMILLE,
            "mid-scale, got {}",
            rows[0].exposure_permille
        );
        assert_eq!(rows[0].exposure_permille, 500);
    }

    #[test]
    fn head_height_sees_over_low_walls() {
        // Probes stand at head height: a 1 m wall hides nothing at 1.7 m,
        // a 3 m wall hides the tile fully. Both agree with hand intuition.
        let tile = [(2, 0)];
        let tall =
            exposure_per_tile(&wall_room(3.0), &[threat_east()], &tile).expect("tall wall answers");
        assert_eq!(tall[0].exposure_permille, 0);
        let low =
            exposure_per_tile(&wall_room(1.0), &[threat_east()], &tile).expect("low wall answers");
        assert_eq!(low[0].exposure_permille, EXPOSURE_MAX_PERMILLE);
    }

    #[test]
    fn errors_name_their_cause() {
        let world = wall_room(3.0);
        assert_eq!(
            exposure_per_tile(&world, &[], &[(0, 0)]),
            Err(ExposureError::NoThreats)
        );
        assert_eq!(
            exposure_per_tile(&world, &[[f32::NAN, 1.5, 0.0]], &[(0, 0)]),
            Err(ExposureError::NonFiniteThreat(0))
        );
        assert_eq!(
            exposure_per_tile(
                &world,
                &[threat_east(), [30.0, f32::INFINITY, 0.0]],
                &[(0, 0)]
            ),
            Err(ExposureError::NonFiniteThreat(1))
        );
        assert_eq!(
            format!("{}", ExposureError::NoThreats),
            "exposure: needs at least one threat eye"
        );
        assert_eq!(
            format!("{}", ExposureError::NonFiniteThreat(2)),
            "exposure: threat 2 is not finite"
        );
    }

    #[test]
    fn tiles_sort_and_dedup() {
        let world = wall_room(3.0);
        let rows = exposure_per_tile(&world, &[threat_east()], &[(8, 0), (1, 0), (8, 0), (1, 0)])
            .expect("dup tiles answer");
        assert_eq!(
            rows.iter().map(|r| r.tile).collect::<Vec<_>>(),
            vec![(1, 0), (8, 0)]
        );
        // Threat order never moves the rows: counts only, no sequences.
        let swapped = exposure_per_tile(&world, &[threat_west(), threat_east()], &[(8, 0), (1, 0)])
            .expect("swapped threats answer");
        let straight =
            exposure_per_tile(&world, &[threat_east(), threat_west()], &[(1, 0), (8, 0)])
                .expect("straight threats answer");
        assert_eq!(swapped, straight);
        let empty = exposure_per_tile(&world, &[threat_east()], &[]).expect("no tiles read empty");
        assert!(empty.is_empty());
    }

    #[test]
    fn digest_stable_twice_and_order_invariant() {
        let world = wall_room(3.0);
        let rows = exposure_per_tile(&world, &[threat_east()], &labeled_tiles())
            .expect("digest room answers");
        let d1 = digest_exposures(&rows);
        assert_eq!(d1, digest_exposures(&rows), "digest stable across runs");
        let mut rev = rows.clone();
        rev.reverse();
        assert_eq!(digest_exposures(&rev), d1, "digest order-invariant");
        let mut flipped = rows.clone();
        flipped[0].exposure_permille = (flipped[0].exposure_permille + 1) % 1001;
        assert_ne!(
            digest_exposures(&flipped),
            d1,
            "one flipped scalar flips the digest"
        );
    }

    #[test]
    fn lines_render_sorted_rows() {
        let rows = vec![
            TileExposure {
                tile: (8, 0),
                exposure_permille: 1000,
            },
            TileExposure {
                tile: (1, 0),
                exposure_permille: 0,
            },
        ];
        assert_eq!(
            exposure_lines(&rows).concat(),
            "exposure 1,0 0\nexposure 8,0 1000\n"
        );
        assert_eq!(exposure_lines(&rows).len(), 2);
    }

    #[test]
    fn cover_approach_rank_shape() {
        // The cover-approach feed shape, written against the public APIs only: walkable
        // staging tiles beside cover, ordered safest-first by exposure.
        use crate::afford::cover_approach_tiles;
        use crate::navcheck::NavView;
        use std::collections::BTreeMap;
        let wall = prop(1, 5.0, 1.5, 0.0, 1.0, 3.0, 10.0);
        let actors = [wall];
        let mut tiles = BTreeMap::new();
        for tx in 2..=8 {
            for tz in -6..=6 {
                tiles.insert((tx, tz), crate::Tile { walk: true });
            }
        }
        let nav = NavView {
            tiles,
            links: vec![],
            flow: BTreeMap::new(),
            doors: BTreeMap::new(),
            spawn_id: 0,
        };
        let approaches =
            cover_approach_tiles(&actors, Some(&nav)).expect("wall room has approaches");
        assert!(!approaches.is_empty(), "cover draws staging tiles");
        let mut sorted = approaches.clone();
        sorted.sort_unstable();
        assert_eq!(approaches, sorted, "approaches arrive tile-sorted");
        let world = SightWorld::build(&actors);
        let rows = exposure_per_tile(&world, &[threat_east()], &approaches)
            .expect("approach exposures answer");
        assert_eq!(rows.len(), approaches.len());
        let mut ranked_rows = rows.clone();
        ranked_rows.sort_by_key(|r| (r.exposure_permille, r.tile));
        let ranked: Vec<(i32, i32)> = ranked_rows.iter().map(|r| r.tile).collect();
        // Every staging tile west of the wall outranks every tile east of it.
        let west: Vec<(i32, i32)> = ranked.iter().copied().filter(|t| t.0 <= 3).collect();
        let east: Vec<(i32, i32)> = ranked.iter().copied().filter(|t| t.0 >= 6).collect();
        assert!(
            !west.is_empty() && !east.is_empty(),
            "both sides stage: {ranked:?}"
        );
        let last_west = ranked.iter().rposition(|t| t.0 <= 3).expect("west stages");
        let first_east = ranked.iter().position(|t| t.0 >= 6).expect("east stages");
        assert!(
            last_west < first_east,
            "covered side stages first: {ranked:?}"
        );
        let _ = (west, east);
    }

    #[test]
    #[ignore = "timing: exposure budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn exposure_budget_point_two_ms_per_tile() {
        // 500 scattered crates; one threat; the far target clears every roof
        // so each ray pays the full 500-box scan: the honest worst case.
        let actors: Vec<Actor> = (0..500u32)
            .map(|i| {
                let x = (i % 25) as f32 * 2.0;
                let z = (i / 25) as f32 * 2.0;
                prop(1000 + i, x, 1.0, z, 1.0, 2.0, 1.0)
            })
            .collect();
        let world = SightWorld::build(&actors);
        let tiles: Vec<(i32, i32)> = (0..120).map(|i| (i % 12 - 6, i / 12 - 5)).collect();
        let threats = [[48.0, 10.0, 38.0]];
        let out = exposure_per_tile(&world, &threats, &tiles).expect("budget room answers");
        assert_eq!(out.len(), tiles.len());
        let mut best_nanos: u128 = u128::MAX;
        for _ in 0..11 {
            let start = std::time::Instant::now();
            let rerun = exposure_per_tile(&world, &threats, &tiles).expect("rerun answers");
            assert_eq!(rerun.len(), tiles.len());
            best_nanos = best_nanos.min(start.elapsed().as_nanos());
        }
        let per_tile_nanos = best_nanos / 120;
        assert!(
            per_tile_nanos <= 200_000,
            "exposure {per_tile_nanos} ns/tile over budget 200_000 ns/tile"
        );
    }
}
