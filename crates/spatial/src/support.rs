//! SPX2: seated joints and the support chain (opt-in checks `seatedJoint`,
//! `supportChain`).
//!
//! Consumer: `run_spatial` with either name in `checks` (they sit in
//! `OPT_IN_CHECKS`, so the frozen 18-check catalog and its hashes do not
//! move). Units: meters in, integer cm inside. Determinism: solids ordered by
//! id, ties to the lower id, integer ratios only.
//!
//! Rules (Forge `SpatialMath` ratios, integers here):
//! - A solid is grounded when its bottom is within 25% of its own height of
//!   y = 0 (and at least 1 cm).
//! - Another solid supports it by resting (its top 0..=5 cm below the bottom)
//!   or by a seat: the bottom sits inset into the lower top by at most 20% of
//!   the LOWER height while at least half of the upper footprint rests on it.
//!   A seat is a joint: yellow, never red.
//! - An inset past that, but not past half of the upper height, is a real
//!   embed: red.
//! - A candidate that spans the upper solid's whole height, or that holds
//!   over half of it, is a wall, not a supporter. Such a wall mounts the
//!   upper solid, so a wall pipe is neither floating nor sunken.
//! - A solid with no support, no mount and no ground is a floating base. One
//!   `supportChain` finding names the base, never the crates stacked on it.
//! - A solid held by no other solid whose bottom is buried deeper than 15% of
//!   its own height is sunken: a red `supportChain` row with `b` unset and
//!   `pct` the depth as a percent of its height. Ground plates are the floor
//!   itself and never sunken.

use super::checks::{PLATE_H_CM, PLATE_SIDE_CM};
use super::extent::robust_extent;
use super::index::{pos_known, size_known, to_cm};
use super::{Actor, Finding};
use std::collections::{BTreeMap, BTreeSet};

/// Resting contact band in cm: supporter top within this below the bottom.
const REST_CM: i32 = 5;
/// Grounded floor in cm: a bottom at or under this is on the ground.
const GROUND_CM: i32 = 1;
/// Burial past this percent of a solid's own height reads sunken.
const SUNKEN_PCT: i32 = 15;

#[derive(Clone, Copy)]
struct Solid {
    id: u32,
    lo: [i32; 3],
    hi: [i32; 3],
}

impl Solid {
    fn h(&self) -> i32 {
        self.hi[1] - self.lo[1]
    }
    /// Plan-view overlap area with `o`, cm^2 (0 when the interiors miss).
    fn overlap_area(&self, o: &Solid) -> i64 {
        let ix = (self.hi[0].min(o.hi[0]) - self.lo[0].max(o.lo[0])).max(0);
        let iz = (self.hi[2].min(o.hi[2]) - self.lo[2].max(o.lo[2])).max(0);
        i64::from(ix) * i64::from(iz)
    }
    fn area(&self) -> i64 {
        i64::from(self.hi[0] - self.lo[0]) * i64::from(self.hi[2] - self.lo[2])
    }
    /// A flat slab that is the floor of the layout.
    fn is_plate(&self) -> bool {
        self.h() <= PLATE_H_CM
            && self.hi[0] - self.lo[0] <= PLATE_SIDE_CM
            && self.hi[2] - self.lo[2] <= PLATE_SIDE_CM
    }
}

fn solids(actors: &[Actor]) -> Vec<Solid> {
    let mut out: Vec<Solid> = actors
        .iter()
        .filter(|a| a.physical && pos_known(a) && size_known(a))
        .filter(|a| a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0)
        .map(|a| {
            let half = [
                to_cm(a.sx).abs() / 2,
                to_cm(a.sy).abs() / 2,
                to_cm(a.sz).abs() / 2,
            ];
            let c = [to_cm(a.x), to_cm(a.y), to_cm(a.z)];
            Solid {
                id: a.id,
                lo: [c[0] - half[0], c[1] - half[1], c[2] - half[2]],
                hi: [c[0] + half[0], c[1] + half[1], c[2] + half[2]],
            }
        })
        .collect();
    out.sort_by_key(|s| s.id);
    out
}

/// How a lower solid relates to an upper one.
enum Hold {
    /// Not a supporter and not a mount (apart, or beside).
    None,
    /// A wall: spans the upper solid or holds over half of it.
    Mount,
    /// Top within the resting band.
    Rest,
    /// Shallow seat: a joint (inset cm, percent of the lower height).
    Seat(i32),
    /// Deep embed: red (inset cm).
    Embed(i32),
}

fn hold(low: &Solid, up: &Solid) -> Hold {
    if low.id == up.id || low.overlap_area(up) == 0 || low.lo[1] > up.lo[1] {
        return Hold::None;
    }
    if low.hi[1] >= up.hi[1] {
        return Hold::Mount;
    }
    let gap = up.lo[1] - low.hi[1];
    if gap > REST_CM {
        return Hold::None;
    }
    if gap >= 0 {
        return Hold::Rest;
    }
    let inset = -gap;
    if inset * 2 > up.h() {
        return Hold::Mount;
    }
    if inset * 5 <= low.h() && low.overlap_area(up) * 2 >= up.area() {
        Hold::Seat(inset)
    } else {
        Hold::Embed(inset)
    }
}

fn pct(part: i32, whole: i32) -> f32 {
    f32::from(i16::try_from(part * 100 / whole.max(1)).unwrap_or(i16::MAX))
}

/// Seated joints (yellow) and support-chain findings over `actors`, sorted by
/// `(check, a, b)`. Embeds are red `supportChain` rows with `b` the lower
/// solid and `pct` the inset as a percent of the lower height; a floating base
/// is a yellow `supportChain` row with `b == u32::MAX` and `pct` the number of
/// solids stacked on it.
pub fn support_findings(actors: &[Actor]) -> Vec<Finding> {
    // SPX8: a stray or a 50x backdrop is not a layout piece; it neither floats nor holds.
    let apart: BTreeSet<u32> = robust_extent(actors)
        .map(|e| e.excluded.iter().map(|x| x.id).collect())
        .unwrap_or_default();
    let all: Vec<Solid> = solids(actors)
        .into_iter()
        .filter(|s| !apart.contains(&s.id))
        .collect();
    let mut out = Vec::new();
    let mut carried_by: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut bases = Vec::new();
    for up in &all {
        let grounded = up.lo[1] <= GROUND_CM.max(up.h() / 4);
        let (mut supported, mut mounted) = (grounded, false);
        let (mut seated, mut held) = (false, false);
        for low in &all {
            match hold(low, up) {
                Hold::None => {}
                Hold::Mount => mounted = true,
                Hold::Rest => {
                    (supported, held) = (true, true);
                    carried_by.entry(low.id).or_default().insert(up.id);
                }
                Hold::Seat(inset) => {
                    (supported, held) = (true, true);
                    carried_by.entry(low.id).or_default().insert(up.id);
                    if !seated {
                        seated = true;
                        out.push(finding(
                            "seatedJoint",
                            up.id,
                            low.id,
                            pct(inset, low.h()),
                            "yellow",
                        ));
                    }
                }
                Hold::Embed(inset) => {
                    (supported, held) = (true, true);
                    carried_by.entry(low.id).or_default().insert(up.id);
                    out.push(finding(
                        "supportChain",
                        up.id,
                        low.id,
                        pct(inset, low.h()),
                        "red",
                    ));
                }
            }
        }
        if !supported && !mounted {
            bases.push(up.id);
        }
        if !held && !mounted && !up.is_plate() && -up.lo[1] * 100 > SUNKEN_PCT * up.h() {
            out.push(finding(
                "supportChain",
                up.id,
                u32::MAX,
                pct(-up.lo[1], up.h()),
                "red",
            ));
        }
    }
    for base in bases {
        let mut seen = BTreeSet::new();
        let mut stack = vec![base];
        while let Some(id) = stack.pop() {
            for next in carried_by.get(&id).into_iter().flatten() {
                if seen.insert(*next) {
                    stack.push(*next);
                }
            }
        }
        out.push(finding(
            "supportChain",
            base,
            u32::MAX,
            f32::from(u16::try_from(seen.len()).unwrap_or(u16::MAX)),
            "yellow",
        ));
    }
    out.sort_by(|x, y| (x.check, x.a, x.b).cmp(&(y.check, y.a, y.b)));
    out
}

fn finding(check: &'static str, a: u32, b: u32, pct: f32, severity: &'static str) -> Finding {
    Finding {
        check,
        a,
        b,
        pct,
        severity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_spatial, SPATIAL_CHECKS};
    use engine_core::Rng;

    /// Box by cm: `lo` corner and `size`, as an actor in meters.
    fn boxed(id: u32, lo: [i32; 3], size: [i32; 3]) -> Actor {
        let c = |k: usize| (lo[k] * 2 + size[k]) as f32 / 200.0;
        Actor {
            id,
            name: format!("s{id}"),
            x: c(0),
            y: c(1),
            z: c(2),
            sx: size[0] as f32 / 100.0,
            sy: size[1] as f32 / 100.0,
            sz: size[2] as f32 / 100.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn run(actors: &[Actor]) -> Vec<(&'static str, u32, u32, &'static str)> {
        support_findings(actors)
            .iter()
            .map(|f| (f.check, f.a, f.b, f.severity))
            .collect()
    }

    #[test]
    fn support_chain_reads_the_four_forge_cases() {
        // Desk: frame 90 tall on the floor, top plate seated 5 cm into it.
        let desk = [
            boxed(1, [0, 0, 0], [100, 90, 60]),
            boxed(2, [0, 85, 0], [100, 30, 60]),
        ];
        assert_eq!(run(&desk), vec![("seatedJoint", 2, 1, "yellow")]);
        // Two crates, the upper 50% inside the lower: red embed.
        let crates = [
            boxed(1, [0, 0, 0], [100, 100, 100]),
            boxed(2, [0, 50, 0], [100, 100, 100]),
        ];
        assert_eq!(run(&crates), vec![("supportChain", 2, 1, "red")]);
        // A pipe through a wall mounts on it: no floating, nothing sunken.
        let pipe = [
            boxed(1, [0, 0, 0], [20, 300, 400]),
            boxed(2, [-100, 150, 100], [300, 10, 10]),
        ];
        assert_eq!(run(&pipe), vec![]);
        // A crate on a raised table names the table, not the crate.
        let raised = [
            boxed(1, [0, 100, 0], [100, 80, 100]),
            boxed(2, [20, 180, 20], [40, 40, 40]),
        ];
        assert_eq!(run(&raised), vec![("supportChain", 1, u32::MAX, "yellow")]);
        assert_eq!(support_findings(&raised)[0].pct, 1.0);
        // Resting stacks on the ground are quiet.
        let stack = [
            boxed(1, [0, 0, 0], [100, 100, 100]),
            boxed(2, [0, 100, 0], [100, 100, 100]),
        ];
        assert_eq!(run(&stack), vec![]);
    }

    #[test]
    fn support_chain_reads_a_buried_base_as_sunken_past_15_percent() {
        let sunk = |depth: i32| run(&[boxed(1, [0, -depth, 0], [100, 100, 100])]);
        assert_eq!(sunk(30), vec![("supportChain", 1, u32::MAX, "red")]);
        assert_eq!(sunk(16), vec![("supportChain", 1, u32::MAX, "red")]);
        assert_eq!(sunk(15), vec![], "15% exactly is still on the ground");
        assert_eq!(sunk(0), vec![]);
        assert_eq!(
            support_findings(&[boxed(1, [0, -30, 0], [100, 100, 100])])[0].pct,
            30.0
        );
        // A ground plate is the floor: half buried is its job.
        assert_eq!(run(&[boxed(1, [0, -5, 0], [4000, 10, 4000])]), vec![]);
        // A crate held by a solid is judged by that solid, not by the ground.
        let held = [
            boxed(1, [0, -200, 0], [100, 100, 100]),
            boxed(2, [0, -100, 0], [100, 100, 100]),
        ];
        assert_eq!(run(&held), vec![("supportChain", 1, u32::MAX, "red")]);
    }

    #[test]
    fn support_chain_skips_a_stray_and_a_backdrop() {
        // 24 crates resting on the ground in a 6 m square.
        let mut scene: Vec<Actor> = (0..24)
            .map(|i| {
                boxed(
                    i + 1,
                    [(i as i32 % 6) * 100, 0, (i as i32 / 6) * 100],
                    [50, 50, 50],
                )
            })
            .collect();
        assert_eq!(run(&scene), vec![]);
        // A crate hanging 500 m up is a floating base... unless it is a stray.
        let mut hanging = boxed(100, [0, 60_000, 0], [50, 50, 50]);
        scene.push(hanging.clone());
        assert_eq!(run(&scene), vec![], "a stray is not a floating base");
        // A 600 m wide plate under the crates is a backdrop: it holds nothing and sinks nothing.
        scene.pop();
        scene.push(boxed(101, [-30_000, -300, -30_000], [60_000, 600, 60_000]));
        assert_eq!(run(&scene), vec![], "a backdrop is not a sunken base");
        // With fewer than four boxes nothing is excluded, so the same crate floats.
        hanging.id = 1;
        assert_eq!(
            run(&[hanging]),
            vec![("supportChain", 1, u32::MAX, "yellow")]
        );
    }

    #[test]
    fn opt_in_checks_ride_run_spatial_and_the_catalog_stays_18() {
        assert_eq!(SPATIAL_CHECKS.len(), 18);
        let crates = vec![
            boxed(1, [0, 0, 0], [100, 100, 100]),
            boxed(2, [0, 50, 0], [100, 100, 100]),
        ];
        let report = run_spatial(&crates, &["supportChain"], None).expect("opt-in is legal");
        assert_eq!(report.count("supportChain"), 1);
        assert_eq!(report.findings[0].severity, "red");
        assert_eq!(
            run_spatial(&crates, &["seatedJoint"], None)
                .unwrap()
                .count("seatedJoint"),
            0
        );
        // The frozen full run never asks for them.
        assert_eq!(crate::run(&crates).count("supportChain"), 0);
    }

    #[test]
    fn twenty_permutations_give_one_digest() {
        let mut actors = vec![
            boxed(1, [0, 0, 0], [100, 90, 60]),
            boxed(2, [0, 85, 0], [100, 30, 60]),
            boxed(3, [300, 100, 0], [100, 80, 100]),
            boxed(4, [320, 180, 20], [40, 40, 40]),
            boxed(5, [600, 0, 0], [100, 100, 100]),
            boxed(6, [600, 50, 0], [100, 100, 100]),
        ];
        let want = support_findings(&actors);
        assert_eq!(want.len(), 3);
        let mut rng = Rng::new(0x5009);
        for _ in 0..20 {
            for i in (1..actors.len()).rev() {
                actors.swap(i, (rng.next_u32() as usize) % (i + 1));
            }
            assert_eq!(support_findings(&actors), want);
        }
    }

    /// Truth by construction: seeded towers whose verdict is known from the
    /// recipe, scattered 10 m apart, compared to the findings.
    #[test]
    fn seeded_towers_match_their_recipe() {
        let mut rng = Rng::new(0x70e7);
        for case in 0..50 {
            let mut actors = Vec::new();
            let mut want: Vec<(&'static str, u32, u32, &'static str)> = Vec::new();
            for tower in 0..4u32 {
                let x = tower as i32 * 1000;
                let base_id = tower * 10 + 1;
                let kind = rng.next_u32() % 4;
                let raise = if kind == 3 { 120 } else { 0 };
                actors.push(boxed(base_id, [x, raise, 0], [100, 100, 100]));
                if kind == 3 {
                    want.push(("supportChain", base_id, u32::MAX, "yellow"));
                }
                // The top: rests (kind 0), seats 10 cm (kind 1), embeds 50 (kind 2).
                let inset = [0, 10, 50, 0][kind as usize];
                actors.push(boxed(
                    base_id + 1,
                    [x, raise + 100 - inset, 0],
                    [100, 100, 100],
                ));
                match kind {
                    1 => want.push(("seatedJoint", base_id + 1, base_id, "yellow")),
                    2 => want.push(("supportChain", base_id + 1, base_id, "red")),
                    _ => {}
                }
            }
            want.sort_by(|p, q| (p.0, p.1, p.2).cmp(&(q.0, q.1, q.2)));
            assert_eq!(run(&actors), want, "case {case}");
            assert_eq!(run(&actors), want, "case {case} x2");
        }
    }
}
