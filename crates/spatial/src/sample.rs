//! Free-space sampler: Poisson-disc placements with capsule clearance.
//! Owner: spatial. Consumers: the placement solver's fallback, the level
//! builder and the `scene.query` tool's free-space and largest-open answers.
//! Units: meters in, integer cm inside. Determinism: SplitMix64 seed
//! stream, id-sorted boxes, fixed scan order; same seed, actors and
//! config give byte-identical samples. Errors: typed `SampleError`.

use super::index::{boxes_of, pos_known, size_known, to_cm, Box3};
use super::Actor;
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::splitmix64;

/// First synthetic actor id the caller may assign to samples. Sample ids
/// never collide with small scene ids; the caller owns the mapping.
pub const SAMPLE_ACTOR_BASE: u32 = 800_000;
/// Bridson attempts per active point before it retires.
const TRIES_PER_ACTIVE: u32 = 30;
/// Initial-point draws before refusing with `NoSpace`.
const INITIAL_DRAWS: u32 = 100;
/// Largest-open scan pitch in cm.
const OPEN_SCAN_CM: i32 = 25;
/// World half-extent in cm, mirroring the `bounds` red line.
const WORLD_BOUND_CM: i32 = 10_000;

/// Typed sampler failures. Invalid input returns `Err`, never a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleError {
    /// Object footprint is not a positive finite size.
    BadObjectSize,
    /// Capsule radius is negative or past the world bound.
    BadCapsule,
    /// Gap, count or seed bound violated; carries the legal range.
    BadConfig(&'static str),
    /// No valid actor anchors the room, or nothing fits the clearance.
    NoSpace(&'static str),
}

impl std::fmt::Display for SampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadObjectSize => write!(f, "sample: object size must be finite and > 0"),
            Self::BadCapsule => write!(f, "sample: capsule radius 0..=10000 cm"),
            Self::BadConfig(what) => write!(f, "sample: bad config ({what})"),
            Self::NoSpace(what) => write!(f, "sample: no space ({what})"),
        }
    }
}

impl std::error::Error for SampleError {}

/// Sampler configuration. Sizes are meters, gaps/radii integer cm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerConfig {
    /// Object footprint full extents in meters (XZ) and height (Y).
    pub sx_m: f32,
    pub sy_m: f32,
    pub sz_m: f32,
    /// Capsule clearance around the footprint in cm (0 allowed).
    pub capsule_radius_cm: i32,
    /// Minimum center-to-center gap between samples in cm.
    pub min_gap_cm: i32,
    /// Maximum samples to emit (1..=256).
    pub max_samples: usize,
    /// Seed for the candidate stream; same seed gives same samples.
    pub seed: u64,
}

impl SamplerConfig {
    /// Checked constructor: sizes finite and positive, capsule in range,
    /// gap at least 1 cm, count in 1..=256.
    pub fn new(
        sx_m: f32,
        sy_m: f32,
        sz_m: f32,
        capsule_radius_cm: i32,
        min_gap_cm: i32,
        max_samples: usize,
        seed: u64,
    ) -> Result<Self, SampleError> {
        let cfg = Self {
            sx_m,
            sy_m,
            sz_m,
            capsule_radius_cm,
            min_gap_cm,
            max_samples,
            seed,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate ranges without building anything.
    pub fn validate(&self) -> Result<(), SampleError> {
        if !(self.sx_m.is_finite() && self.sy_m.is_finite() && self.sz_m.is_finite())
            || self.sx_m <= 0.0
            || self.sy_m <= 0.0
            || self.sz_m <= 0.0
        {
            return Err(SampleError::BadObjectSize);
        }
        if !(0..=WORLD_BOUND_CM).contains(&self.capsule_radius_cm) {
            return Err(SampleError::BadCapsule);
        }
        if !(1..=WORLD_BOUND_CM).contains(&self.min_gap_cm) {
            return Err(SampleError::BadConfig("min_gap 1..=10000 cm"));
        }
        if !(1..=256).contains(&self.max_samples) {
            return Err(SampleError::BadConfig("max_samples 1..=256"));
        }
        Ok(())
    }
}

/// One placement: grounded center in meters, yaw radians, XZ clearance in
/// cm to the nearest actor rect (unexpanded footprint gap).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub clearance_cm: i32,
}

/// SplitMix64 stream: integer ops only, identical on every target.
struct SplitMix {
    state: u64,
}

impl SplitMix {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        let out = splitmix64(self.state);
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        out
    }

    /// Uniform in [0, 1): exact f64 division of integer bits, no libm.
    fn unit(&mut self) -> f64 {
        const DIV: f64 = 9_007_199_254_740_992.0; // 2^53
        ((self.next() >> 11) as f64) / DIV
    }

    /// Uniform in [lo, hi): integer cm, lo inclusive.
    fn below(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (i64::from(hi) - i64::from(lo)).max(1);
        lo + ((self.unit() * span as f64) as i64 as i32).clamp(0, (span - 1) as i32)
    }
}

fn valid(a: &Actor) -> bool {
    pos_known(a) && size_known(a) && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0
}

/// XZ rect in integer cm.
#[derive(Debug, Clone, Copy)]
struct Rect {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

fn actor_rects(actors: &[Actor]) -> Vec<Rect> {
    let mut boxes = boxes_of(
        &actors
            .iter()
            .filter(|a| valid(a))
            .cloned()
            .collect::<Vec<_>>(),
    );
    boxes.sort_by_key(|b| b.id);
    boxes
        .iter()
        .map(|b| Rect {
            min_x: b.min_x,
            max_x: b.max_x,
            min_z: b.min_z,
            max_z: b.max_z,
        })
        .collect()
}

/// Strict XZ overlap: touching edges are clear, matching the `overlap` check.
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min_x < b.max_x && b.min_x < a.max_x && a.min_z < b.max_z && b.min_z < a.max_z
}

/// Expand a rect by `pad` cm on every side.
fn expand(r: &Rect, pad: i32) -> Rect {
    Rect {
        min_x: r.min_x - pad,
        max_x: r.max_x + pad,
        min_z: r.min_z - pad,
        max_z: r.max_z + pad,
    }
}

/// Footprint rect of the object centered at (cx, cz) in cm.
fn footprint(cx: i32, cz: i32, hx: i32, hz: i32) -> Rect {
    Rect {
        min_x: cx - hx,
        max_x: cx + hx,
        min_z: cz - hz,
        max_z: cz + hz,
    }
}

/// Smallest edge gap from `r` to any rect; negative when overlapping.
/// Touching edges read 0, never negative.
fn clearance_to(r: &Rect, rects: &[Rect]) -> i32 {
    let mut best = i32::MAX;
    for b in rects {
        let gx = if r.min_x >= b.max_x {
            r.min_x - b.max_x
        } else if b.min_x >= r.max_x {
            b.min_x - r.max_x
        } else {
            -1
        };
        let gz = if r.min_z >= b.max_z {
            r.min_z - b.max_z
        } else if b.min_z >= r.max_z {
            b.min_z - r.max_z
        } else {
            -1
        };
        // Separated on one axis reads that axis gap; separated on both
        // reads the smaller; overlapping on both reads -1. Touching is 0.
        let gap = if gx < 0 && gz < 0 {
            -1
        } else if gx < 0 {
            gz
        } else if gz < 0 {
            gx
        } else {
            gx.min(gz)
        };
        best = best.min(gap);
    }
    if best == i32::MAX {
        WORLD_BOUND_CM
    } else {
        best
    }
}

struct Room {
    rects: Vec<Rect>,
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

fn room_of(actors: &[Actor]) -> Option<Room> {
    let boxes: Vec<Box3> = boxes_of(
        &actors
            .iter()
            .filter(|a| valid(a))
            .cloned()
            .collect::<Vec<_>>(),
    );
    if boxes.is_empty() {
        return None;
    }
    let mut room = Room {
        rects: actor_rects(actors),
        min_x: i32::MAX,
        max_x: i32::MIN,
        min_z: i32::MAX,
        max_z: i32::MIN,
    };
    for b in &boxes {
        room.min_x = room.min_x.min(b.min_x);
        room.max_x = room.max_x.max(b.max_x);
        room.min_z = room.min_z.min(b.min_z);
        room.max_z = room.max_z.max(b.max_z);
    }
    Some(room)
}

/// Candidate footprint clears every actor rect once expanded by the
/// capsule radius, and sits inside the room bounds.
fn fits(room: &Room, cx: i32, cz: i32, hx: i32, hz: i32, capsule: i32) -> bool {
    if cx - hx - capsule < room.min_x
        || cx + hx + capsule > room.max_x
        || cz - hz - capsule < room.min_z
        || cz + hz + capsule > room.max_z
    {
        return false;
    }
    let padded = expand(&footprint(cx, cz, hx, hz), capsule);
    !room.rects.iter().any(|b| overlaps(&padded, b))
}

/// Poisson-disc free space: Bridson square-annulus sampling over the room
/// bounds with capsule clearance, grounded outputs, no interpenetration.
/// Deterministic in seed, actors and config; input order never matters.
pub fn sample_free_space(
    actors: &[Actor],
    cfg: &SamplerConfig,
) -> Result<Vec<Sample>, SampleError> {
    cfg.validate()?;
    let room = room_of(actors).ok_or(SampleError::NoSpace("no valid actor anchors the room"))?;
    let hx = to_cm(cfg.sx_m).abs() / 2;
    let hz = to_cm(cfg.sz_m).abs() / 2;
    let gap = cfg.min_gap_cm;
    let lo_x = room.min_x + hx + cfg.capsule_radius_cm;
    let hi_x = room.max_x - hx - cfg.capsule_radius_cm;
    let lo_z = room.min_z + hz + cfg.capsule_radius_cm;
    let hi_z = room.max_z - hz - cfg.capsule_radius_cm;
    if lo_x > hi_x || lo_z > hi_z {
        return Err(SampleError::NoSpace(
            "room too small for footprint + capsule",
        ));
    }
    let mut rng = SplitMix::new(cfg.seed);
    // Grid accel: cell = gap / sqrt(2) so neighbors within gap are found
    // in the 5x5 neighborhood; Vec indexed, never hashed.
    let cell = ((i64::from(gap) * 7071 / 10000).max(1)) as i32;
    let nx = ((hi_x - lo_x) / cell + 1).max(1);
    let nz = ((hi_z - lo_z) / cell + 1).max(1);
    let at = |cx: i32, cz: i32| -> Option<usize> {
        if cx < lo_x || cx > hi_x || cz < lo_z || cz > hi_z {
            return None;
        }
        Some(((cz - lo_z) / cell * nx + (cx - lo_x) / cell) as usize)
    };
    let mut grid: Vec<Option<usize>> = vec![None; (nx as usize) * (nz as usize)];
    let mut pts: Vec<(i32, i32)> = Vec::new();

    let far_enough = |cx: i32, cz: i32, grid: &[Option<usize>], pts: &[(i32, i32)]| -> bool {
        let gx = (cx - lo_x) / cell;
        let gz = (cz - lo_z) / cell;
        for dz in -2..=2 {
            for dx in -2..=2 {
                let (nx2, nz2) = (gx + dx, gz + dz);
                if nx2 < 0 || nz2 < 0 || nx2 >= nx || nz2 >= nz {
                    continue;
                }
                if let Some(idx) = grid[(nz2 * nx + nx2) as usize] {
                    let (px, pz) = pts[idx];
                    let ddx = i64::from(px) - i64::from(cx);
                    let ddz = i64::from(pz) - i64::from(cz);
                    if ddx * ddx + ddz * ddz < i64::from(gap) * i64::from(gap) {
                        return false;
                    }
                }
            }
        }
        true
    };

    let push = |cx: i32, cz: i32, grid: &mut Vec<Option<usize>>, pts: &mut Vec<(i32, i32)>| {
        let idx = pts.len();
        pts.push((cx, cz));
        if let Some(cell_idx) = at(cx, cz) {
            grid[cell_idx] = Some(idx);
        }
    };

    // Seeded first point: uniform draw until one fits, else NoSpace.
    let mut first = None;
    for _ in 0..INITIAL_DRAWS {
        let cx = rng.below(lo_x, hi_x + 1);
        let cz = rng.below(lo_z, hi_z + 1);
        if fits(&room, cx, cz, hx, hz, cfg.capsule_radius_cm) {
            first = Some((cx, cz));
            break;
        }
    }
    let Some((fx, fz)) = first else {
        return Err(SampleError::NoSpace("no fitting point in 100 draws"));
    };
    push(fx, fz, &mut grid, &mut pts);
    let mut active: Vec<usize> = vec![0];

    // Bridson loop: stack order is fixed; only offsets draw from the seed.
    // Bounded: every active retires after TRIES_PER_ACTIVE misses.
    let mut guard = 0usize;
    while !active.is_empty() && pts.len() < cfg.max_samples {
        guard += 1;
        if guard > 1_000_000 {
            break;
        }
        let top = *active.last().expect("active never empty here");
        let (ax, az) = pts[top];
        let mut placed = false;
        for _ in 0..TRIES_PER_ACTIVE {
            // Uniform square annulus [gap, 2*gap]: no trig, no libm.
            let dx = rng.below(-2 * gap, 2 * gap + 1);
            let dz = rng.below(-2 * gap, 2 * gap + 1);
            let adx = i64::from(dx.abs());
            let adz = i64::from(dz.abs());
            if adx * adx + adz * adz < i64::from(gap) * i64::from(gap) {
                continue;
            }
            let cx = ax + dx;
            let cz = az + dz;
            if cx < lo_x || cx > hi_x || cz < lo_z || cz > hi_z {
                continue;
            }
            if !fits(&room, cx, cz, hx, hz, cfg.capsule_radius_cm) {
                continue;
            }
            if !far_enough(cx, cz, &grid, &pts) {
                continue;
            }
            push(cx, cz, &mut grid, &mut pts);
            if pts.len() < cfg.max_samples {
                active.push(pts.len() - 1);
            }
            placed = true;
            break;
        }
        if !placed {
            active.pop();
        }
    }

    // Yaw from the same stream in fixed order: k * PI/4, 8 steps.
    let y = cfg.sy_m / 2.0;
    let mut out = Vec::with_capacity(pts.len());
    for (cx, cz) in pts.into_iter().take(cfg.max_samples) {
        let step = (rng.next() % 8) as f32;
        let clearance = clearance_to(&footprint(cx, cz, hx, hz), &room.rects);
        out.push(Sample {
            x: cx as f32 / 100.0,
            y,
            z: cz as f32 / 100.0,
            yaw: step * std::f32::consts::FRAC_PI_4,
            clearance_cm: clearance,
        });
    }
    Ok(out)
}

/// Largest open spot: grid scan for the footprint center with the maximum
/// clearance to any actor rect. Ties break by (z, x) ascending, so the
/// answer is stable. `None` when nothing fits the footprint + capsule.
pub fn largest_open_spot(
    actors: &[Actor],
    sx_m: f32,
    sy_m: f32,
    sz_m: f32,
    capsule_radius_cm: i32,
) -> Result<Option<Sample>, SampleError> {
    if !(sx_m.is_finite() && sy_m.is_finite() && sz_m.is_finite())
        || sx_m <= 0.0
        || sy_m <= 0.0
        || sz_m <= 0.0
    {
        return Err(SampleError::BadObjectSize);
    }
    if !(0..=WORLD_BOUND_CM).contains(&capsule_radius_cm) {
        return Err(SampleError::BadCapsule);
    }
    let room = room_of(actors).ok_or(SampleError::NoSpace("no valid actor anchors the room"))?;
    let hx = to_cm(sx_m).abs() / 2;
    let hz = to_cm(sz_m).abs() / 2;
    let lo_x = room.min_x + hx + capsule_radius_cm;
    let hi_x = room.max_x - hx - capsule_radius_cm;
    let lo_z = room.min_z + hz + capsule_radius_cm;
    let hi_z = room.max_z - hz - capsule_radius_cm;
    if lo_x > hi_x || lo_z > hi_z {
        return Ok(None);
    }
    let mut best: Option<(i32, i32, i32)> = None; // (clearance, x, z)
    let mut cz = lo_z;
    while cz <= hi_z {
        let mut cx = lo_x;
        while cx <= hi_x {
            if fits(&room, cx, cz, hx, hz, capsule_radius_cm) {
                let gap = clearance_to(&footprint(cx, cz, hx, hz), &room.rects);
                let wins = match best {
                    None => true,
                    Some((g, bx, bz)) => gap > g || (gap == g && (cz, cx) < (bz, bx)),
                };
                if wins {
                    best = Some((gap, cx, cz));
                }
            }
            cx += OPEN_SCAN_CM;
        }
        cz += OPEN_SCAN_CM;
    }
    Ok(best.map(|(gap, cx, cz)| Sample {
        x: cx as f32 / 100.0,
        y: sy_m / 2.0,
        z: cz as f32 / 100.0,
        yaw: 0.0,
        clearance_cm: gap,
    }))
}

/// Render one sample as a grounded actor for checks and relation graphs.
/// Ids come from `SAMPLE_ACTOR_BASE + n`; the caller owns uniqueness.
pub fn sample_actor(n: u32, s: &Sample, cfg: &SamplerConfig) -> Actor {
    Actor {
        id: SAMPLE_ACTOR_BASE + n,
        name: format!("sample_{n:04}"),
        x: s.x,
        y: s.y,
        z: s.z,
        sx: cfg.sx_m,
        sy: cfg.sy_m,
        sz: cfg.sz_m,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: s.yaw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sight::{reachable, Reach, ReachView};
    use std::collections::BTreeMap;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn mk(id: u32, name: &str, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
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

    /// Walled room, 10 m square: thin walls (9.8 m spans so corners only
    /// touch) plus two crates. XZ-disjoint, grounded, `run()` clean.
    fn room() -> Vec<Actor> {
        vec![
            mk(0, "wall_north", 0.0, 1.0, -5.0, 9.8, 2.0, 0.2),
            mk(1, "wall_south", 0.0, 1.0, 5.0, 9.8, 2.0, 0.2),
            mk(2, "wall_west", -5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(3, "wall_east", 5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(4, "crate_a", -2.0, 0.4, -2.0, 0.8, 0.8, 0.8),
            mk(5, "crate_b", 2.5, 0.4, 1.5, 0.8, 0.8, 0.8),
        ]
    }

    fn cfg(seed: u64) -> SamplerConfig {
        SamplerConfig::new(0.8, 0.8, 0.8, 30, 100, 12, seed).expect("valid config")
    }

    /// Every sample id rides polygon 0 with the anchor: reachability is
    /// structural, proven per sample below.
    fn reach_view(anchor: u32, n: usize) -> ReachView {
        let mut adj = BTreeMap::new();
        adj.insert(0u32, vec![0u32]);
        let mut at = BTreeMap::new();
        at.insert(anchor, 0u32);
        for i in 0..n {
            at.insert(SAMPLE_ACTOR_BASE + i as u32, 0u32);
        }
        ReachView { adj, at }
    }

    #[test]
    fn samples_hold_zero_overlap_zero_floating_all_reachable() {
        let actors = room();
        let c = cfg(7);
        let samples = sample_free_space(&actors, &c).expect("room has space");
        assert!(!samples.is_empty(), "open room yields samples");
        assert!(samples.len() <= 12);
        // Grounded by construction: y is half height.
        for s in &samples {
            assert_eq!(s.y, 0.4, "samples rest on the ground plane");
            assert!(s.clearance_cm >= 30, "capsule clearance holds");
        }
        // No interpenetration between samples: pairwise center gap.
        for (i, a) in samples.iter().enumerate() {
            for b in &samples[i + 1..] {
                let dx = (a.x - b.x) * 100.0;
                let dz = (a.z - b.z) * 100.0;
                assert!(
                    dx * dx + dz * dz >= 100.0 * 100.0,
                    "sample gap holds: ({},{}) vs ({},{})",
                    a.x,
                    a.z,
                    b.x,
                    b.z
                );
            }
        }
        // Staged scene: samples added as actors stay clean.
        let mut staged = actors.clone();
        for (i, s) in samples.iter().enumerate() {
            staged.push(sample_actor(i as u32, s, &c));
        }
        let report = crate::run(&staged);
        assert_eq!(
            report.count("overlap"),
            0,
            "0 overlaps: {:?}",
            report.findings
        );
        let new_floats = report
            .findings
            .iter()
            .filter(|f| f.check == "floating" && f.a >= SAMPLE_ACTOR_BASE)
            .count();
        assert_eq!(new_floats, 0, "0 floating samples");
        // Reachable: every sample shares the anchor polygon.
        let view = reach_view(4, samples.len());
        for i in 0..samples.len() {
            assert_eq!(
                reachable(&view, 4, SAMPLE_ACTOR_BASE + i as u32),
                Reach::Reachable,
                "sample {i} reachable"
            );
        }
    }

    #[test]
    fn same_seed_twice_identical_reordered_input_identical() {
        let actors = room();
        let c = cfg(42);
        let a = sample_free_space(&actors, &c).expect("samples");
        let b = sample_free_space(&actors, &c).expect("samples again");
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (x.x.to_bits(), x.z.to_bits()),
                (y.x.to_bits(), y.z.to_bits())
            );
            assert_eq!(x.clearance_cm, y.clearance_cm);
        }
        let mut rev = actors.clone();
        rev.reverse();
        let c2 = sample_free_space(&rev, &c).expect("reordered");
        assert_eq!(a.len(), c2.len());
        for (x, y) in a.iter().zip(c2.iter()) {
            assert_eq!(
                (x.x.to_bits(), x.z.to_bits()),
                (y.x.to_bits(), y.z.to_bits())
            );
        }
    }

    #[test]
    fn seeds_diverge_and_sp5_consumes_a_sample() {
        let actors = room();
        let a = sample_free_space(&actors, &cfg(1)).expect("seed 1");
        let b = sample_free_space(&actors, &cfg(2)).expect("seed 2");
        assert!(!a.is_empty() && !b.is_empty());
        let same = a.len() == b.len()
            && a.iter()
                .zip(b.iter())
                .all(|(x, y)| x.x.to_bits() == y.x.to_bits() && x.z.to_bits() == y.z.to_bits());
        assert!(!same, "different seeds place differently");
        // The solver consumes the sampler: a crate intending Near the first
        // sample solves.
        let mut scene = actors.clone();
        scene.push(mk(60, "mover", -4.0, 0.4, 4.0, 0.8, 0.8, 0.8));
        let s = &a[0];
        let intent = crate::Intent {
            object: crate::ObjectRef::Id(60),
            wants: vec![crate::Want {
                relation: crate::relation::Relation::NextTo,
                target: crate::Target::Point {
                    x_cm: (s.x * 100.0).round() as i32,
                    z_cm: (s.z * 100.0).round() as i32,
                },
                distance_cm: Some((0, 120)),
            }],
        };
        let out = crate::solve(&intent, &scene).expect("sampled point is placeable");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, 60);
    }

    #[test]
    fn largest_open_is_the_room_center_clearance() {
        let actors = room();
        let spot = largest_open_spot(&actors, 0.8, 0.8, 0.8, 30)
            .expect("scan runs")
            .expect("open room has a spot");
        assert!(
            spot.clearance_cm >= 30,
            "clearance holds: {}",
            spot.clearance_cm
        );
        assert_eq!(spot.y, 0.4, "grounded");
        // Symmetric room with two off-center crates: best cell sits near
        // the middle of the largest empty quadrant, far from every crate.
        assert!(spot.x.abs() < 4.0 && spot.z.abs() < 4.0);
        let staged = {
            let mut v = actors.clone();
            v.push(Actor {
                id: SAMPLE_ACTOR_BASE,
                name: "sample_0000".into(),
                x: spot.x,
                y: spot.y,
                z: spot.z,
                sx: 0.8,
                sy: 0.8,
                sz: 0.8,
                parent: None,
                physical: true,
                vx: 0.0,
                vz: 0.0,
                yaw: 0.0,
            });
            v
        };
        let report = crate::run(&staged);
        assert_eq!(report.count("overlap"), 0);
    }

    #[test]
    fn bad_input_refuses_loudly() {
        assert_eq!(
            SamplerConfig::new(0.0, 0.8, 0.8, 30, 100, 4, 1).unwrap_err(),
            SampleError::BadObjectSize
        );
        assert_eq!(
            SamplerConfig::new(0.8, 0.8, 0.8, -1, 100, 4, 1).unwrap_err(),
            SampleError::BadCapsule
        );
        assert_eq!(
            SamplerConfig::new(0.8, 0.8, 0.8, 30, 0, 4, 1).unwrap_err(),
            SampleError::BadConfig("min_gap 1..=10000 cm")
        );
        assert_eq!(
            SamplerConfig::new(0.8, 0.8, 0.8, 30, 100, 0, 1).unwrap_err(),
            SampleError::BadConfig("max_samples 1..=256")
        );
        let nan = SamplerConfig {
            sx_m: f32::NAN,
            sy_m: 0.8,
            sz_m: 0.8,
            capsule_radius_cm: 30,
            min_gap_cm: 100,
            max_samples: 4,
            seed: 1,
        };
        assert_eq!(nan.validate().unwrap_err(), SampleError::BadObjectSize);
        // Empty scene anchors nothing.
        assert!(sample_free_space(&[], &cfg(1)).is_err());
        // A sealed room (one solid plate filling the bounds) holds nothing.
        let plate = vec![mk(0, "plate", 0.0, 0.5, 0.0, 10.0, 1.0, 10.0)];
        assert!(sample_free_space(&plate, &cfg(1)).is_err());
        assert_eq!(
            format!("{}", SampleError::BadCapsule),
            "sample: capsule radius 0..=10000 cm"
        );
    }
}
