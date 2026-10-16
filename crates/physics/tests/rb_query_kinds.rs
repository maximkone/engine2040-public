//! Query-kind dispatcher spike over the integer segment index.
//!
//! Owner: physics lane. Units: whole millimetres, millisecond timings are
//! advisory only. Determinism: seeded swings hash identically twice via
//! integer math only. Errors: typed [`QueryError`], never strings.
//! Consumer: blade swings; the swept shape-cast kind
//! ([`QueryKind::SweptToi`]) rides [`QueryBvh::sweep_box_into`] with a 250 mm
//! blade while ray and overlap stay on the thin [`QueryBvh::cast_into`]
//! path, and the moving-blade test below proves the swept kind hits where
//! the static kinds miss. Budget: build once per world change, casts
//! allocate nothing.

use engine_core::alloc_probe;
use engine_core::{Fnv64, Rng};
use engine_physics::query::{
    brute_cast_into, brute_sweep_box_into, BoxMm, HitMm, QueryBvh, QueryError, SegmentMm,
};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

/// Dispatcher taxonomy, reimplemented integer-only over the existing index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryKind {
    /// Thin sight ray, first hit decides.
    Ray,
    /// Point or small-box overlap probe.
    ShapeOverlap,
    /// Swept segment with sorted time of impact.
    SweptToi,
}

/// Moving-blade half extents in whole millimetres: the swept kind grows
/// every static box by this before casting the centre segment, so a blade
/// clips targets the thin ray flies past.
const BLADE_HALF_MM: [u32; 3] = [250, 250, 250];

/// Thin dispatcher over one build-once index and its box slice.
struct Dispatcher<'a> {
    index: &'a QueryBvh,
    boxes: &'a [BoxMm],
}

impl<'a> Dispatcher<'a> {
    fn over(index: &'a QueryBvh, boxes: &'a [BoxMm]) -> Self {
        Self { index, boxes }
    }

    /// Dispatch one cast by kind: ray and overlap share the thin static
    /// path (a zero-thickness probe), while the swept kind rides the
    /// Minkowski-grown swept path with the blade half extents above.
    fn dispatch(
        &self,
        kind: QueryKind,
        seg: SegmentMm,
        out: &mut [HitMm],
    ) -> Result<usize, QueryError> {
        match kind {
            QueryKind::Ray | QueryKind::ShapeOverlap => self.index.cast_into(self.boxes, seg, out),
            QueryKind::SweptToi => self
                .index
                .sweep_box_into(self.boxes, seg, BLADE_HALF_MM, out),
        }
    }

    /// First-hit ray specialization: the sorted path makes it the minimum.
    fn ray_first(&self, seg: SegmentMm) -> Result<Option<HitMm>, QueryError> {
        let mut one = [HitMm::EMPTY; 1];
        let mut spill = [HitMm::EMPTY; 256];
        let total = self.index.cast_into(self.boxes, seg, &mut spill)?;
        if total == 0 {
            return Ok(None);
        }
        let first = spill[0];
        let kept = self.index.cast_into(self.boxes, seg, &mut one);
        match kept {
            Ok(1) => Ok(Some(one[0])),
            Ok(_) => Ok(Some(first)),
            Err(QueryError::BufferTooSmall { .. }) => Ok(Some(first)),
            Err(other) => Err(other),
        }
    }

    /// Point probe: zero-length segment hits the boxes containing the point.
    fn point_probe(&self, at_mm: [i32; 3], out: &mut [HitMm]) -> Result<usize, QueryError> {
        let seg = SegmentMm {
            from_mm: at_mm,
            to_mm: at_mm,
        };
        self.dispatch(QueryKind::ShapeOverlap, seg, out)
    }

    /// Advisory miss distance in whole millimetres, integer only, never a gate.
    fn miss_distance_mm(&self, at_mm: [i32; 3]) -> u64 {
        let mut best = u64::MAX;
        for candidate in self.boxes {
            let mut sum = 0u128;
            for axis in 0..3 {
                let gap = if at_mm[axis] < candidate.min_mm[axis] {
                    (i64::from(candidate.min_mm[axis]) - i64::from(at_mm[axis])) as u64
                } else if at_mm[axis] > candidate.max_mm[axis] {
                    (i64::from(at_mm[axis]) - i64::from(candidate.max_mm[axis])) as u64
                } else {
                    0
                };
                sum += u128::from(gap) * u128::from(gap);
            }
            let dist = u64::try_from(sum.isqrt()).unwrap_or(u64::MAX);
            if dist < best {
                best = dist;
            }
        }
        if self.boxes.is_empty() {
            u64::MAX
        } else {
            best
        }
    }
}

fn draw_mm(rng: &mut Rng, low: i32, high: i32) -> i32 {
    let span = i64::from(high) - i64::from(low);
    assert!(span > 0, "test range ordered");
    let draw = rng.next_u64() % u64::try_from(span).expect("span fits u64");
    i32::try_from(i64::from(low) + i64::try_from(draw).expect("draw fits i64"))
        .expect("drawn value fits i32")
}

/// District boxes: 20 buildings on a jittered 90 m grid, 12 parts each.
fn swing_boxes(rng: &mut Rng) -> Vec<BoxMm> {
    let mut out = Vec::new();
    let mut id = 0u32;
    for serial in 0..20u32 {
        let grid_x = i32::try_from(serial % 5).expect("grid fits i32") - 2;
        let grid_z = i32::try_from(serial / 5).expect("grid fits i32") - 1;
        let base_x = grid_x * 90_000 + draw_mm(rng, -20_000, 20_000);
        let base_z = grid_z * 90_000 + draw_mm(rng, -20_000, 20_000);
        for _ in 0..12 {
            let size_x = draw_mm(rng, 2_000, 12_000);
            let size_y = draw_mm(rng, 2_000, 30_000);
            let size_z = draw_mm(rng, 2_000, 12_000);
            let corner_x = base_x + draw_mm(rng, -15_000, 15_000);
            let corner_y = draw_mm(rng, 0, 40_000);
            let corner_z = base_z + draw_mm(rng, -15_000, 15_000);
            let material = u16::try_from(id % 8).expect("slot fits u16");
            out.push(
                BoxMm::from_mm(
                    [corner_x, corner_y, corner_z],
                    [corner_x + size_x, corner_y + size_y, corner_z + size_z],
                    id,
                    material,
                )
                .expect("grown corners stay ordered"),
            );
            id += 1;
        }
    }
    out
}

/// Blade swings: 1,000 seeded segments, 2..8 m travel at blade speed.
fn blade_swings(rng: &mut Rng) -> Vec<SegmentMm> {
    let mut out = Vec::with_capacity(1_000);
    while out.len() < 1_000 {
        let from = [
            draw_mm(rng, -250_000, 250_000),
            draw_mm(rng, 0, 40_000),
            draw_mm(rng, -200_000, 200_000),
        ];
        let step = [
            draw_mm(rng, -8_000, 8_000),
            draw_mm(rng, -8_000, 8_000),
            draw_mm(rng, -8_000, 8_000),
        ];
        let mut len2 = 0i64;
        for limb in step {
            let edge = i64::from(limb);
            len2 += edge * edge;
        }
        if !(2_000i64 * 2_000..=8_000i64 * 8_000).contains(&len2) {
            continue;
        }
        out.push(SegmentMm {
            from_mm: from,
            to_mm: [from[0] + step[0], from[1] + step[1], from[2] + step[2]],
        });
    }
    out
}

fn sweep_hash(dispatch: &Dispatcher<'_>, segs: &[SegmentMm]) -> (u64, usize) {
    let mut buf = [HitMm::EMPTY; 256];
    let mut oracle = [HitMm::EMPTY; 256];
    let mut hasher = Fnv64::new();
    let mut total = 0usize;
    for seg in segs {
        let found = dispatch
            .dispatch(QueryKind::SweptToi, *seg, &mut buf)
            .expect("fixture fits the caller buffer");
        let wanted = brute_sweep_box_into(dispatch.boxes, *seg, BLADE_HALF_MM, &mut oracle)
            .expect("fixture fits the caller buffer");
        assert_eq!(found, wanted, "dispatcher and oracle agree on count");
        assert_eq!(&buf[..found], &oracle[..wanted], "hits match hit for hit");
        for hit in &buf[..found] {
            hasher.write_u32(hit.collider);
            hasher.write_u16(hit.material);
            hasher.write_u64(hit.dist_mm);
            for axis in 0..3 {
                hasher.write_i32(hit.entry_mm[axis]);
                hasher.write_i32(hit.exit_mm[axis]);
            }
        }
        total += found;
    }
    (hasher.finish(), total)
}

#[test]
fn kinds_agree_with_oracle_on_1000_swings() {
    let seed = Rng::new(2040);
    let boxes = swing_boxes(&mut seed.fork(71));
    assert_eq!(boxes.len(), 240);
    let segs = blade_swings(&mut seed.fork(72));
    assert_eq!(segs.len(), 1_000);
    let index = QueryBvh::build(&boxes);
    let dispatch = Dispatcher::over(&index, &boxes);
    let mut buf = [HitMm::EMPTY; 256];
    let mut oracle = [HitMm::EMPTY; 256];
    let mut swept = [HitMm::EMPTY; 256];
    for seg in &segs {
        for kind in [QueryKind::Ray, QueryKind::ShapeOverlap] {
            let found = dispatch
                .dispatch(kind, *seg, &mut buf)
                .expect("fixture fits the caller buffer");
            let wanted =
                brute_cast_into(&boxes, *seg, &mut oracle).expect("fixture fits the caller buffer");
            assert_eq!(found, wanted, "kind {kind:?} matches the thin oracle");
            assert_eq!(&buf[..found], &oracle[..wanted], "kind {kind:?} hits match");
        }
        let swept_found = dispatch
            .dispatch(QueryKind::SweptToi, *seg, &mut buf)
            .expect("fixture fits the caller buffer");
        let swept_wanted = brute_sweep_box_into(&boxes, *seg, BLADE_HALF_MM, &mut oracle)
            .expect("fixture fits the caller buffer");
        assert_eq!(
            swept_found, swept_wanted,
            "swept kind matches the swept oracle"
        );
        assert_eq!(
            &buf[..swept_found],
            &oracle[..swept_wanted],
            "swept hits match"
        );
        let thin_found =
            brute_cast_into(&boxes, *seg, &mut swept).expect("fixture fits the caller buffer");
        assert!(
            swept_found >= thin_found,
            "grown boxes keep every thin hit ({swept_found} vs {thin_found})"
        );
    }
    for seg in segs.iter().take(32) {
        let thin = index
            .cast_into(&boxes, *seg, &mut buf)
            .expect("fixture fits the caller buffer");
        let thin_hits = buf[..thin].to_vec();
        let grown = index
            .sweep_box_into(&boxes, *seg, [0, 0, 0], &mut swept)
            .expect("fixture fits the caller buffer");
        assert_eq!(grown, thin, "zero half extent is the thin cast");
        assert_eq!(
            &swept[..grown],
            &thin_hits[..],
            "zero-half hits match hit for hit"
        );
    }
    let first = dispatch
        .ray_first(segs[0])
        .expect("first-hit probe refuses nothing valid");
    let many = dispatch
        .dispatch(QueryKind::Ray, segs[0], &mut buf)
        .expect("fixture fits the caller buffer");
    assert_eq!(first.is_some(), many > 0, "first hit tracks the full cast");
    if let Some(hit) = first {
        assert_eq!(hit, buf[0], "first hit is the sorted minimum");
    }
    let inside: [i32; 3] = boxes[0].min_mm;
    let probed = dispatch
        .point_probe(inside, &mut buf)
        .expect("point probe fits");
    assert!(probed >= 1, "corner point sits inside its own box");
    assert_eq!(dispatch.miss_distance_mm(inside), 0);
    let far = dispatch.miss_distance_mm([900_000, 200_000, 900_000]);
    assert!(far > 0, "far point reports a positive advisory gap");
}

#[test]
fn kinds_refuse_with_typed_errors() {
    let seed = Rng::new(2040);
    let boxes = swing_boxes(&mut seed.fork(73));
    let segs = blade_swings(&mut seed.fork(74));
    let index = QueryBvh::build(&boxes);
    let dispatch = Dispatcher::over(&index, &boxes);
    let seg = segs[0];
    let mut tight = [HitMm::EMPTY; 1];
    let mut roomy = [HitMm::EMPTY; 256];
    let total = dispatch
        .dispatch(QueryKind::SweptToi, seg, &mut roomy)
        .expect("roomy buffer fits");
    if total > 1 {
        let err = dispatch
            .dispatch(QueryKind::Ray, seg, &mut tight)
            .expect_err("one slot cannot hold every hit");
        assert_eq!(
            err,
            QueryError::BufferTooSmall {
                needed: total,
                capacity: 1
            }
        );
    }
    let stale = dispatch
        .index
        .cast_into(&boxes[..boxes.len() - 1], seg, &mut roomy)
        .expect_err("length mismatch refused");
    assert_eq!(
        stale,
        QueryError::StaleBuild {
            built: boxes.len(),
            given: boxes.len() - 1
        }
    );
    let swept_total = dispatch
        .dispatch(QueryKind::SweptToi, seg, &mut roomy)
        .expect("roomy buffer fits the swept cast");
    if swept_total > 1 {
        let err = dispatch
            .index
            .sweep_box_into(&boxes, seg, BLADE_HALF_MM, &mut tight)
            .expect_err("one slot cannot hold every swept hit");
        assert_eq!(
            err,
            QueryError::BufferTooSmall {
                needed: swept_total,
                capacity: 1
            }
        );
    }
    let swept_stale = dispatch
        .index
        .sweep_box_into(&boxes[..boxes.len() - 1], seg, BLADE_HALF_MM, &mut roomy)
        .expect_err("swept length mismatch refused");
    assert_eq!(
        swept_stale,
        QueryError::StaleBuild {
            built: boxes.len(),
            given: boxes.len() - 1
        }
    );
}

#[test]
fn swept_blade_hits_where_static_misses() {
    let boxes = [BoxMm::from_mm([0, 0, 0], [1000, 1000, 1000], 7, 3).expect("ordered")];
    let index = QueryBvh::build(&boxes);
    let dispatch = Dispatcher::over(&index, &boxes);
    // The centre line flies 100 mm above the box: every static probe misses.
    let skim = SegmentMm {
        from_mm: [-500, 1100, 500],
        to_mm: [1500, 1100, 500],
    };
    let mut out = [HitMm::EMPTY; 4];
    let mut oracle = [HitMm::EMPTY; 4];
    assert_eq!(
        dispatch
            .dispatch(QueryKind::Ray, skim, &mut out)
            .expect("fits"),
        0,
        "thin ray flies past the box"
    );
    assert_eq!(
        dispatch
            .point_probe(skim.from_mm, &mut out)
            .expect("point probe fits"),
        0,
        "static start point sits outside the box"
    );
    assert_eq!(
        dispatch
            .point_probe(skim.to_mm, &mut out)
            .expect("point probe fits"),
        0,
        "static end point sits outside the box"
    );
    // The 250 mm blade around the same line clips the grown box.
    let n = dispatch
        .dispatch(QueryKind::SweptToi, skim, &mut out)
        .expect("swept fits");
    assert_eq!(n, 1, "moving blade clips the box: {out:?}");
    assert_eq!(out[0].collider, 7);
    assert_eq!(out[0].material, 3);
    assert_eq!(out[0].entry_mm, [-250, 1100, 500]);
    assert_eq!(out[0].exit_mm, [1250, 1100, 500]);
    assert_eq!(out[0].dist_mm, 250);
    let wanted =
        brute_sweep_box_into(&boxes, skim, BLADE_HALF_MM, &mut oracle).expect("swept oracle fits");
    assert_eq!(wanted, n, "swept oracle agrees on the count");
    assert_eq!(
        &oracle[..wanted],
        &out[..n],
        "swept oracle agrees hit for hit"
    );
    // The zero-half sweep is the thin cast: still a miss here.
    let mut thin = [HitMm::EMPTY; 4];
    assert_eq!(
        index
            .sweep_box_into(&boxes, skim, [0, 0, 0], &mut thin)
            .expect("fits"),
        0,
        "zero half extent keeps the miss"
    );
}

#[test]
fn kinds_hash_twice_identical_bytes() {
    let seed = Rng::new(2040);
    let boxes = swing_boxes(&mut seed.fork(75));
    let segs = blade_swings(&mut seed.fork(76));
    let index = QueryBvh::build(&boxes);
    let dispatch = Dispatcher::over(&index, &boxes);
    let (first, total) = sweep_hash(&dispatch, &segs);
    assert!(total > 20, "swings must land hits, got {total}");
    let rebuilt = QueryBvh::build(&boxes);
    let again = Dispatcher::over(&rebuilt, &boxes);
    let (second, total_again) = sweep_hash(&again, &segs);
    assert_eq!(total, total_again, "rebuild repeats the sweep");
    assert_eq!(first, second, "same swings hash twice");
    println!("query-kinds parity: 1000 swings over 240 boxes, {total} hits, hash {first:016x}");
}

#[expect(
    clippy::disallowed_methods,
    reason = "spike timing probe reads wall time; the reading never enters simulation state or hashes"
)]
#[expect(
    clippy::disallowed_types,
    reason = "spike timing probe reads wall time; the reading never enters simulation state or hashes"
)]
#[test]
fn kinds_spike_ms_baseline_vs_dispatcher() {
    let seed = Rng::new(2040);
    let boxes = swing_boxes(&mut seed.fork(77));
    let segs = blade_swings(&mut seed.fork(78));
    let index = QueryBvh::build(&boxes);
    let dispatch = Dispatcher::over(&index, &boxes);
    let mut buf = [HitMm::EMPTY; 256];
    for seg in segs.iter().take(32) {
        dispatch
            .dispatch(QueryKind::SweptToi, *seg, &mut buf)
            .expect("warmup fits");
    }
    let start_base = std::time::Instant::now();
    let mut base_hits = 0usize;
    for seg in &segs {
        base_hits +=
            brute_sweep_box_into(&boxes, *seg, BLADE_HALF_MM, &mut buf).expect("fixture fits");
    }
    let base_ms = start_base.elapsed().as_secs_f64() * 1000.0;
    let ((cand_hits, direct_ms), allocs) = alloc_probe::measure(|| {
        let start = std::time::Instant::now();
        let mut hits = 0usize;
        for seg in &segs {
            hits += dispatch
                .dispatch(QueryKind::SweptToi, *seg, &mut buf)
                .expect("fixture fits");
        }
        (hits, start.elapsed().as_secs_f64() * 1000.0)
    });
    assert_eq!(base_hits, cand_hits, "dispatcher keeps every oracle hit");
    assert_eq!(
        allocs.calls, 0,
        "dispatcher casts allocate nothing, got {allocs:?}"
    );
    let start_direct = std::time::Instant::now();
    let mut direct_hits = 0usize;
    for seg in &segs {
        direct_hits += index
            .sweep_box_into(&boxes, *seg, BLADE_HALF_MM, &mut buf)
            .expect("fixture fits");
    }
    let direct_wall_ms = start_direct.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(direct_hits, cand_hits, "dispatcher adds no verdict change");
    let _ = (direct_ms, direct_wall_ms);
    println!(
        "query-kinds spike: 1000 swept swings over 240 boxes, brute {base_ms:.3} ms vs dispatcher {direct_wall_ms:.3} ms, {cand_hits} hits"
    );
}
