//! soft: the Rapier cloth of PX44c: a 4x4 pinned sheet sags, holds its pins exactly, stays
//! finite, settles, and prints the one stable `physfx: soft:` receipt line.

use super::*;
use crate::soft::{ClothDesc, ClothError, SoftCloth, SoftSolve, SoftWorld, MAX_CLOTHS};
use crate::verbs::{fx_cloth, ClothEvent};
use std::time::Instant;

/// A world holding the 4x4 banner of the receipt.
fn banner() -> SoftWorld {
    let mut world = SoftWorld::new();
    let slot = world.spawn(ClothDesc::BANNER).expect("the banner builds");
    assert_eq!(slot, 0);
    world
}

/// The banner in the world.
fn cloth(world: &SoftWorld) -> &SoftCloth {
    world.cloth(0).expect("the banner is in the world")
}

/// The free bottom row of a sheet.
fn last_row(cloth: &SoftCloth) -> Vec<[f32; 3]> {
    let start = (cloth.desc().rows - 1) * cloth.desc().cols;
    cloth.particles()[start..].to_vec()
}

/// Steps the cloth world `steps` fixed ticks.
fn step_soft(world: &mut SoftWorld, steps: usize) {
    for _ in 0..steps {
        world.step();
    }
}

/// Steps the whole fx world `steps` fixed ticks.
fn step_world(fx: &mut PhysFx, steps: usize) {
    for _ in 0..steps {
        fx.step();
    }
}

#[test]
fn soft_cloth_sags_below_its_pins_and_keeps_them_exact() {
    let mut world = banner();
    step_soft(&mut world, 120);
    let cloth = cloth(&world);
    assert_eq!(cloth.nodes(), 16);
    assert_eq!(cloth.pins(), 4);

    let last = last_row(cloth);
    let mean = last.iter().map(|p| p[1]).sum::<f32>() / last.len() as f32;
    let pin_y = cloth.pin_positions()[0][1];
    assert!(
        mean < pin_y - 0.001,
        "the free row hangs {:.4} m below the pins",
        pin_y - mean
    );
    for (i, pin) in cloth.pin_positions().iter().enumerate() {
        assert_eq!(cloth.particles()[i], *pin, "pin {i} moved");
    }
    assert!(cloth.sag_cm() > 0.1, "sag {:.2} cm", cloth.sag_cm());
}

#[test]
fn soft_cloth_stays_finite_and_its_free_row_settles() {
    let mut world = banner();
    let mut prev = last_row(cloth(&world));
    let mut quiet = 0u32;
    let mut best = 0u32;
    for _ in 0..120 {
        world.step();
        let now = last_row(cloth(&world));
        let moved = now
            .iter()
            .zip(&prev)
            .map(|(a, b)| {
                let dx = a[0] - b[0];
                let dy = a[1] - b[1];
                let dz = a[2] - b[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .fold(0.0_f32, f32::max);
        if moved < 0.001 {
            quiet += 1;
            best = best.max(quiet);
        } else {
            quiet = 0;
        }
        prev = now;
    }
    assert!(
        best >= 10,
        "the free row never held 10 quiet steps (best {best})"
    );
    assert!(
        quiet >= 10,
        "the free row still moves after 120 steps (quiet {quiet})"
    );
    let cloth = cloth(&world);
    for p in cloth.particles() {
        assert!(p.iter().all(|v| v.is_finite()), "non-finite particle {p:?}");
    }
    assert!(cloth.sag_cm() > 0.0, "sag {:.2} cm", cloth.sag_cm());
}

#[test]
fn the_soft_receipt_line_is_stable_and_reads_16_nodes_and_4_pins() {
    let event = ClothEvent {
        desc: ClothDesc::BANNER,
    };
    let mut a = PhysFx::new(Tier::Comfort);
    let mut b = PhysFx::new(Tier::Comfort);
    let first = fx_cloth(&mut a, &event).expect("cloth a");
    let second = fx_cloth(&mut b, &event).expect("cloth b");
    assert_eq!(first.slot, 0);
    assert_eq!(second.slot, 0);
    assert_eq!(first.receipt.nodes, 16);
    assert_eq!(first.receipt.pins, 4);
    assert_eq!(first.receipt_line(), second.receipt_line());

    step_world(&mut a, 120);
    step_world(&mut b, 120);
    let ra = a.cloth_receipt(0).expect("a receipt");
    let rb = b.cloth_receipt(0).expect("b receipt");
    let line = ra.receipt_line();
    assert_eq!(line, rb.receipt_line());
    assert!(
        line.starts_with("physfx: soft: nodes=16 pins=4 sag_cm="),
        "{line}"
    );
    assert!(ra.sag_cm > 0.0, "{line}");
    println!("{line}");
}

#[test]
fn a_soft_desc_out_of_range_is_refused_by_name() {
    let bad_grid = ClothDesc {
        rows: 1,
        ..ClothDesc::BANNER
    };
    assert_eq!(
        SoftWorld::new().spawn(bad_grid).err(),
        Some(ClothError::Grid)
    );
    let bad_spacing = ClothDesc {
        spacing_m: f32::NAN,
        ..ClothDesc::BANNER
    };
    assert_eq!(
        SoftWorld::new().spawn(bad_spacing).err(),
        Some(ClothError::Spacing)
    );
    let bad_pin = ClothDesc {
        pin_row: 3,
        ..ClothDesc::BANNER
    };
    assert_eq!(
        SoftWorld::new().spawn(bad_pin).err(),
        Some(ClothError::PinRow)
    );
    let bad_gravity = ClothDesc {
        gravity_scale: 5.0,
        ..ClothDesc::BANNER
    };
    assert_eq!(
        SoftWorld::new().spawn(bad_gravity).err(),
        Some(ClothError::Gravity)
    );
    let bad_stiffness = ClothDesc {
        stiffness_hz: 0.0,
        ..ClothDesc::BANNER
    };
    assert_eq!(
        SoftWorld::new().spawn(bad_stiffness).err(),
        Some(ClothError::Stiffness)
    );
}

#[test]
fn a_soft_world_holds_at_most_max_cloths() {
    let mut world = SoftWorld::new();
    for _ in 0..MAX_CLOTHS {
        world.spawn(ClothDesc::BANNER).expect("a cloth");
    }
    let err = world.spawn(ClothDesc::BANNER).expect_err("the cap refuses");
    assert_eq!(
        err,
        ClothError::OverCap {
            need: MAX_CLOTHS + 1,
            cap: MAX_CLOTHS
        }
    );
}

#[test]
fn a_soft_cape_and_banner_both_hang_and_step() {
    let mut world = SoftWorld::new();
    let cape = world.spawn(ClothDesc::CAPE).expect("cape");
    let banner = world.spawn(ClothDesc::BANNER).expect("banner");
    assert_eq!(world.cloth(cape).expect("cape").nodes(), 60);
    assert_eq!(world.cloth(banner).expect("banner").nodes(), 16);
    step_soft(&mut world, 120);
    assert!(world.cloth(cape).expect("cape").sag_cm() > 0.0);
    assert!(world.cloth(banner).expect("banner").sag_cm() > 0.0);
}

/// The structural edges of a sheet with their rest length, in the order `Sheet::new` builds
/// them: the neighbours and the two diagonals of every grid cell. Both solve forms touch this
/// same edge set, so the constraint total per tick is fixed by the sheet, not by the form.
fn rest_edges(desc: ClothDesc) -> Vec<(usize, usize, f32)> {
    let at = |row: usize, col: usize| row * desc.cols + col;
    let mut out = Vec::new();
    for row in 0..desc.rows {
        for col in 0..desc.cols {
            if col + 1 < desc.cols {
                out.push((at(row, col), at(row, col + 1), desc.spacing_m));
            }
            if row + 1 < desc.rows {
                out.push((at(row, col), at(row + 1, col), desc.spacing_m));
            }
            if row + 1 < desc.rows && col + 1 < desc.cols {
                let diag = desc.spacing_m * std::f32::consts::SQRT_2;
                out.push((at(row, col), at(row + 1, col + 1), diag));
                out.push((at(row + 1, col), at(row, col + 1), diag));
            }
        }
    }
    out
}

/// The worst structural edge in the sheet: longest current length over its rest length.
fn longest_stretch(cloth: &SoftCloth) -> f32 {
    let p = cloth.particles();
    rest_edges(cloth.desc())
        .into_iter()
        .map(|(a, b, rest)| {
            let (dx, dy, dz) = (p[a][0] - p[b][0], p[a][1] - p[b][1], p[a][2] - p[b][2]);
            (dx * dx + dy * dy + dz * dz).sqrt() / rest
        })
        .fold(1.0_f32, f32::max)
}

/// A 64-bit digest of every particle position: the determinism check of the soft layer.
fn digest(cloth: &SoftCloth) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for p in cloth.particles() {
        for v in p {
            hash ^= u64::from(v.to_bits());
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// What one pinned cloth reads after `ticks`: the worst edge stretch at rest, the worst stretch
/// seen on the way there, the tick its free row went quiet for ten ticks in a row, and the state
/// digest.
struct Trip {
    stretch: f32,
    peak: f32,
    settle: usize,
    digest: u64,
}

/// Steps a 4x4 banner on `solve` until it settles (or `ticks`) and reports the four numbers.
fn trip(solve: SoftSolve, ticks: usize) -> Trip {
    let mut world = SoftWorld::new().with_solve(solve);
    world.spawn(ClothDesc::BANNER).expect("the banner builds");
    let mut peak = 1.0_f32;
    let mut prev = last_row(cloth(&world));
    let mut settle = ticks;
    let mut quiet = 0;
    for tick in 1..=ticks {
        world.step();
        let now = last_row(cloth(&world));
        let moved = now
            .iter()
            .zip(&prev)
            .map(|(a, b)| {
                let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .fold(0.0_f32, f32::max);
        quiet = if moved < 0.001 { quiet + 1 } else { 0 };
        if quiet >= 10 && settle == ticks {
            settle = tick;
        }
        prev = now;
        peak = peak.max(longest_stretch(cloth(&world)));
    }
    let cloth = cloth(&world);
    Trip {
        stretch: longest_stretch(cloth),
        peak,
        settle,
        digest: digest(cloth),
    }
}

/// PX47, measured: one step of six solver iterations against six substeps of one iteration
/// each, on the same pinned banner (the same spring set, six solves per tick). The small-step
/// form is not cheaper, not stiffer at rest and not quicker to settle here, so the shipped
/// single-step path stays the default. The numbers are printed, never asserted as a win.
#[test]
fn soft_substeps_measure_against_one_step_of_six_iterations() {
    let six = trip(SoftSolve::Iterations(6), 240);
    let small = trip(SoftSolve::Substeps(6), 240);
    println!(
        "PX47 1x6: rest={:.4} peak={:.4} settle={} | 6x1: rest={:.4} peak={:.4} settle={}",
        six.stretch, six.peak, six.settle, small.stretch, small.peak, small.settle
    );
    let rest_gap = (six.stretch - small.stretch).abs() / six.stretch;
    assert!(
        rest_gap < 0.01,
        "the two forms rest {:.4} and {:.4} apart: {rest_gap:.4}",
        six.stretch,
        small.stretch
    );
    for t in [&six, &small] {
        assert!(t.peak.is_finite() && t.peak < 2.0, "peak {:.4}", t.peak);
    }
}

/// PX47 F2P: the resting stretch of a spring cloth is set by the spring's own compliance, not
/// by how often the solver visits it. Both forms read here at 2 and at 6: the spread stays
/// under half a percent either way, so there is no stiffness the tier knob could move.
#[test]
fn soft_rest_stretch_ignores_the_iteration_count_in_both_forms() {
    let lo_step = trip(SoftSolve::Iterations(2), 240);
    let hi_step = trip(SoftSolve::Iterations(6), 240);
    let lo_small = trip(SoftSolve::Substeps(2), 240);
    let hi_small = trip(SoftSolve::Substeps(6), 240);
    let step_spread = (lo_step.stretch - hi_step.stretch).abs() / hi_step.stretch;
    let small_spread = (lo_small.stretch - hi_small.stretch).abs() / hi_small.stretch;
    println!(
        "PX47 F2P: iter2 rest={:.4} iter6 rest={:.4} spread={:.5} | substep2 rest={:.4} substep6 rest={:.4} spread={:.5}",
        lo_step.stretch, hi_step.stretch, step_spread, lo_small.stretch, hi_small.stretch, small_spread
    );
    assert!(
        step_spread < 0.005,
        "one step moved {step_spread:.5} with the iteration count"
    );
    assert!(
        small_spread < 0.005,
        "substeps moved {small_spread:.5} with the substep count"
    );
}

#[test]
fn soft_digest_is_identical_across_two_worlds() {
    let a = trip(SoftSolve::Substeps(6), 120);
    let b = trip(SoftSolve::Substeps(6), 120);
    println!("PX47 digest a={:016x} b={:016x}", a.digest, b.digest);
    assert_eq!(a.digest, b.digest, "two cloth worlds diverged");
}

/// PX47 cost, a measurement and not a speed claim: the two forms over the same ticks on a cape.
#[test]
fn soft_substeps_cost_measurement() {
    let cape = ClothDesc::CAPE;
    let ticks = 400;
    let mut ratios = Vec::new();
    for _ in 0..3 {
        let mut wide = SoftWorld::new().with_solve(SoftSolve::Iterations(6));
        wide.spawn(cape).expect("cape");
        let mut narrow = SoftWorld::new().with_solve(SoftSolve::Substeps(6));
        narrow.spawn(cape).expect("cape");
        let t0 = Instant::now();
        for _ in 0..ticks {
            wide.step();
        }
        let one = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        for _ in 0..ticks {
            narrow.step();
        }
        let small = t1.elapsed().as_secs_f64();
        ratios.push(small / one);
    }
    let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
    let spread = ratios.iter().fold(0.0_f64, |m, r| m.max((r - mean).abs()));
    println!("PX47 cost: ratios {ratios:?} mean={mean:.3} spread=+-{spread:.3}");
    assert!(mean.is_finite() && mean > 0.0, "cost ratio {mean:.3}");
}
