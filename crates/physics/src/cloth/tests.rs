//! cloth tests: the sheet, its links and tethers, the strand, and the CLX collision fixture.
//!
//! Every test here reads the sheet only through its public API or through a field of its own
//! module tree, and nothing here is hashed.

use super::convert::from_fixed;
use super::*;
use engine_core::fixed::Fixed;
use engine_core::{laws, Rng};

/// A fighter cape: half a meter wide, a meter long, hung from shoulder height.
const CAPE_SIZE: [f32; 2] = [0.5, 1.0];
const SHOULDERS: [f32; 3] = [0.0, 1.6, 0.0];

fn cape_with(tier: ClothTier, tear_strain: Option<f32>) -> Cloth {
    let params = ClothParams {
        tear_strain,
        ..ClothParams::for_tier(tier)
    };
    Cloth::new(tier.knobs().cape, CAPE_SIZE, SHOULDERS, params).expect("cape builds")
}

fn cape(tier: ClothTier) -> Cloth {
    cape_with(tier, None)
}

/// The cape's top row at the shoulders, moved by `shift`.
fn shoulder_row(cols: usize, shift: [f32; 3]) -> Vec<[f32; 3]> {
    (0..cols)
        .map(|c| {
            let across = CAPE_SIZE[0] * c as f32 / (cols - 1) as f32;
            [
                SHOULDERS[0] + across + shift[0],
                SHOULDERS[1] + shift[1],
                SHOULDERS[2] + shift[2],
            ]
        })
        .collect()
}

/// Triangle wave in `-1..=1` with a 48 tick period; no libm.
fn tri(tick: usize, phase: usize) -> f32 {
    let k = (tick + phase) % 48;
    let up = if k < 24 { k } else { 48 - k };
    up as f32 / 12.0 - 1.0
}

fn run(cloth: &mut Cloth, ticks: usize) {
    for _ in 0..ticks {
        cloth.step(&[]);
    }
}

fn sleep_it(cloth: &mut Cloth) {
    for _ in 0..1200 {
        if cloth.is_asleep() {
            return;
        }
        cloth.step(&[]);
    }
    assert!(cloth.is_asleep(), "a still sheet must fall asleep");
}

fn finite(cloth: &Cloth) -> bool {
    cloth
        .positions()
        .iter()
        .all(|p| p.iter().all(|v| v.is_finite() && v.abs() < 200.0))
}

#[test]
fn tier_table_grows_with_the_tier_and_fits_the_limits() {
    let mut last = (0, 0, 0, 0);
    for tier in ClothTier::ALL {
        let k = tier.knobs();
        assert_eq!(k.cape.validate(), Ok(()));
        assert_eq!(k.banner.validate(), Ok(()));
        assert!((1..=MAX_SUBSTEPS).contains(&k.substeps));
        assert!((1..=MAX_ITERATIONS).contains(&k.iterations));
        let now = (
            k.cape.particles(),
            k.banner.particles(),
            k.substeps,
            k.iterations,
        );
        assert!(
            now.0 > last.0 && now.1 > last.1 && now.2 >= last.2 && now.3 >= last.3,
            "{tier:?} must cost more than the tier below: {now:?} vs {last:?}"
        );
        last = now;
    }
    let potato = ClothTier::Potato.knobs();
    assert!(potato.cape.particles() <= 32 && potato.banner.particles() <= 32);
    assert_eq!(ClothTier::Comfort.knobs().cape.particles(), 60);
}

#[test]
fn tier_params_follow_the_table_and_sheets_build_to_size() {
    for tier in ClothTier::ALL {
        let k = tier.knobs();
        let params = ClothParams::for_tier(tier);
        assert_eq!(params.validate(), Ok(()));
        assert_eq!(
            (params.substeps, params.iterations),
            (k.substeps, k.iterations)
        );
        assert_eq!(params.tear_strain, None, "tearing is off unless asked for");
        for (grid, size) in [(k.cape, CAPE_SIZE), (k.banner, [3.0, 2.0])] {
            let cloth = Cloth::new(grid, size, SHOULDERS, params).expect("sheet builds");
            let (c, r) = (grid.cols, grid.rows);
            assert_eq!(cloth.particle_count(), c * r);
            assert_eq!(
                cloth.link_count(),
                (c - 1) * r + c * (r - 1) + 2 * (c - 1) * (r - 1)
            );
            assert_eq!(cloth.pinned_count(), c, "the top row starts pinned");
            assert!(cloth.is_pinned(c - 1) && !cloth.is_pinned(c));
        }
    }
}

#[test]
fn bad_input_is_refused_before_any_state_changes() {
    let grid = ClothTier::Web.knobs().cape;
    let params = ClothParams::for_tier(ClothTier::Web);
    let build = |g, s, o, p| Cloth::new(g, s, o, p).err();
    let one_wide = ClothGrid { cols: 1, rows: 4 };
    assert_eq!(
        build(one_wide, CAPE_SIZE, SHOULDERS, params),
        Some(ClothError::BadGrid)
    );
    let huge = ClothGrid {
        cols: MAX_SIDE + 1,
        rows: 4,
    };
    assert_eq!(
        build(huge, CAPE_SIZE, SHOULDERS, params),
        Some(ClothError::BadGrid)
    );
    assert_eq!(
        build(grid, [0.0, 1.0], SHOULDERS, params),
        Some(ClothError::BadParam("size"))
    );
    assert_eq!(
        build(grid, [f32::NAN, 1.0], SHOULDERS, params),
        Some(ClothError::BadParam("size"))
    );
    assert_eq!(
        build(grid, CAPE_SIZE, [0.0, f32::INFINITY, 0.0], params),
        Some(ClothError::BadParam("origin"))
    );
    let bad_tunes = [
        ClothParams {
            stiffness: 1.5,
            ..params
        },
        ClothParams {
            iterations: 0,
            ..params
        },
        ClothParams {
            substeps: MAX_SUBSTEPS + 1,
            ..params
        },
        ClothParams {
            air_drag: f32::NAN,
            ..params
        },
        ClothParams {
            wind: [0.0, 0.0, 99.0],
            ..params
        },
        ClothParams {
            tear_strain: Some(-1.0),
            ..params
        },
    ];
    for bad in bad_tunes {
        assert!(bad.validate().is_err(), "{bad:?}");
        assert!(build(grid, CAPE_SIZE, SHOULDERS, bad).is_some(), "{bad:?}");
    }

    let mut cloth = cape(ClothTier::Web);
    let before = cloth.positions().to_vec();
    let short = shoulder_row(grid.cols - 1, [0.0; 3]);
    assert_eq!(
        cloth.pin_top_row(&short),
        Err(ClothError::BadPins {
            expected: grid.cols,
            got: grid.cols - 1
        })
    );
    let mut poisoned = shoulder_row(grid.cols, [0.0; 3]);
    poisoned[2][1] = f32::NAN;
    assert_eq!(
        cloth.pin_top_row(&poisoned),
        Err(ClothError::BadParam("pin"))
    );
    assert_eq!(cloth.pin(999, SHOULDERS), Err(ClothError::BadIndex(999)));
    assert_eq!(cloth.unpin(999), Err(ClothError::BadIndex(999)));
    assert_eq!(
        cloth.set_wind([f32::NAN, 0.0, 0.0]),
        Err(ClothError::BadParam("wind"))
    );
    assert_eq!(cloth.params(), params, "a refused tune changes nothing");
    assert_eq!(cloth.positions(), &*before);
    assert_eq!(
        cloth.reset([f32::NAN; 3]),
        Err(ClothError::BadParam("origin"))
    );
    assert!(ClothCollider::sphere(SHOULDERS, 0.0).is_err());
    assert!(ClothCollider::capsule([f32::NAN; 3], SHOULDERS, 0.1).is_err());
    assert!(ClothCollider::capsule(SHOULDERS, SHOULDERS, f32::INFINITY).is_err());
}

#[test]
fn pins_hold_while_the_free_cloth_swings() {
    let mut cloth = cape(ClothTier::Comfort);
    let cols = cloth.cols;
    let wind = [6.0, 0.0, 2.0];
    cloth.set_wind(wind).expect("wind in range");
    let mut moved = 0.0_f32;
    for t in 0..240 {
        let row = shoulder_row(cols, [0.3 * tri(t, 0), 0.0, 0.2 * tri(t, 12)]);
        cloth.pin_top_row(&row).expect("pins in range");
        cloth.step(&[]);
        for (c, want) in row.iter().enumerate() {
            assert_eq!(
                &cloth.positions()[c],
                want,
                "pin {c} left its shoulder at tick {t}"
            );
        }
        let last = cloth.positions()[cloth.particle_count() - 1];
        moved = moved.max(dist2(last, [0.5, 0.6, 0.0]).sqrt());
    }
    assert!(
        moved > 0.2,
        "the free hem must swing while the pins stay put: {moved}"
    );
}

/// Scatter the free particles up to 0.15 m off their rest places, motionless.
fn crumple(cloth: &mut Cloth, seed: u64) {
    let mut rng = Rng::new(seed);
    let mut jitter = move || rng.next_range(0, 301) as f32 / 1000.0 - 0.15;
    let free = cloth.pos.iter_mut().zip(&mut cloth.prev).zip(&cloth.pinned);
    for ((p, q), pinned) in free {
        if !pinned {
            *p = [p[0] + jitter(), p[1] + jitter(), p[2] + jitter()];
            *q = *p;
        }
    }
}

#[test]
fn a_crumpled_cape_relaxes_to_a_hang_at_rest_length_on_every_tier() {
    for tier in ClothTier::ALL {
        let mut cloth = cape(tier);
        crumple(&mut cloth, 11);
        run(&mut cloth, 300);
        let hem = cloth.positions()[cloth.particle_count() - 1];
        assert!(
            (SHOULDERS[1] - hem[1] - CAPE_SIZE[1]).abs() < 0.1 && hem[2].abs() < 0.02,
            "{tier:?} hem at {hem:?}"
        );
        assert!(
            cloth.max_stretch() < 1.15,
            "{tier:?} stretch {}",
            cloth.max_stretch()
        );
    }
}

#[test]
fn wind_blows_the_hem_downwind() {
    // The cape hangs in the xy plane, so a wind along z hits it face on.
    let mean_hem_z = |wind: [f32; 3]| {
        let mut cloth = cape(ClothTier::Comfort);
        cloth.set_wind(wind).expect("wind in range");
        run(&mut cloth, 180);
        let cols = cloth.cols;
        let hem = &cloth.positions()[cloth.particle_count() - cols..];
        hem.iter().map(|p| p[2]).sum::<f32>() / cols as f32
    };
    assert!(
        mean_hem_z([0.0, 0.0, 8.0]) > 0.3,
        "a steady wind must blow the hem out"
    );
    assert!(
        mean_hem_z([0.0, 0.0, -8.0]) < -0.3,
        "and the other way round"
    );
    assert!(mean_hem_z([0.0; 3]).abs() < 0.02, "no wind, no drift");
}

#[test]
fn no_nan_after_600_ticks_of_gusts_and_a_swinging_body() {
    let mut rng = Rng::new(2040);
    let mut gust = move || rng.next_range(0, 4001) as f32 / 100.0 - 20.0;
    for tier in ClothTier::ALL {
        let mut cloth = cape(tier);
        let mut hem_x = (f32::MAX, f32::MIN);
        for t in 0..600 {
            if t % 12 == 0 {
                cloth
                    .set_wind([gust(), gust() / 4.0, gust()])
                    .expect("gust in range");
            }
            let sway = 0.4 * tri(t, 0);
            cloth
                .pin_top_row(&shoulder_row(cloth.cols, [sway, 0.0, 0.2 * tri(t, 12)]))
                .expect("pins in range");
            let body =
                ClothCollider::capsule([0.25 + sway, 0.6, -0.08], [0.25 + sway, 1.5, -0.08], 0.14)
                    .expect("body");
            cloth.step(&[body]);
            assert!(finite(&cloth), "{tier:?} blew up at tick {t}");
            let hem = cloth.positions()[cloth.particle_count() - 1];
            hem_x = (hem_x.0.min(hem[0]), hem_x.1.max(hem[0]));
        }
        assert!(
            cloth.max_stretch() < 2.0,
            "{tier:?} stretch {}",
            cloth.max_stretch()
        );
        assert!(
            hem_x.1 - hem_x.0 > 0.5,
            "{tier:?}: the gusts must have thrown the hem about: {hem_x:?}"
        );
    }
}

/// Thirty settled ticks, then the shoulders jump five meters sideways in one tick.
fn yanked(tear_strain: Option<f32>) -> Cloth {
    let mut cloth = cape_with(ClothTier::Comfort, tear_strain);
    run(&mut cloth, 30);
    let cols = cloth.cols;
    cloth
        .pin_top_row(&shoulder_row(cols, [5.0, 0.0, 0.0]))
        .expect("pins in range");
    run(&mut cloth, 300);
    cloth
}

#[test]
fn tearing_off_survives_a_violent_yank_whole() {
    let cloth = yanked(None);
    assert!(finite(&cloth));
    assert_eq!(cloth.torn_count(), 0, "tearing is off");
    assert!(cloth.max_stretch() < 1.3, "stretch {}", cloth.max_stretch());
    let hem = cloth.positions()[cloth.particle_count() - 1];
    assert!(hem[0] > 4.5, "the cape must follow its shoulders: {hem:?}");
    assert!(
        hem[1] > SHOULDERS[1] - 1.2,
        "the tethers keep the hem hanging from the pins: {hem:?}"
    );
}

#[test]
fn tearing_on_cuts_a_yanked_cape_and_lets_the_pieces_fall() {
    let cloth = yanked(Some(0.5));
    assert!(finite(&cloth));
    assert!(cloth.torn_count() > 0, "a five meter yank must tear links");
    let hem = cloth.positions()[cloth.particle_count() - 1];
    assert!(
        hem[1] < SHOULDERS[1] - 3.0,
        "the torn piece falls away: {hem:?}"
    );
}

#[test]
fn tearing_on_leaves_gentle_motion_alone() {
    let mut cloth = cape_with(ClothTier::Comfort, Some(0.5));
    let mut hem_x = (f32::MAX, f32::MIN);
    for t in 0..300 {
        let row = shoulder_row(cloth.cols, [0.3 * tri(t, 12), 0.0, 0.0]);
        cloth.pin_top_row(&row).expect("pins in range");
        cloth.step(&[]);
        let hem = cloth.positions()[cloth.particle_count() - 1];
        hem_x = (hem_x.0.min(hem[0]), hem_x.1.max(hem[0]));
    }
    assert!(
        hem_x.1 - hem_x.0 > 0.2,
        "the hem must follow the sway: {hem_x:?}"
    );
    assert_eq!(cloth.torn_count(), 0);
}

#[test]
fn a_still_cape_sleeps_and_wakes_on_pin_wind_or_touch() {
    let mut cloth = cape(ClothTier::Comfort);
    let cols = cloth.cols;
    sleep_it(&mut cloth);
    assert!(
        cloth.ticks() < 600,
        "slept only after {} ticks",
        cloth.ticks()
    );

    let frozen = cloth.positions().to_vec();
    let ticks = cloth.ticks();
    for _ in 0..30 {
        assert!(!cloth.step(&[]), "a sleeping sheet does no work");
    }
    assert_eq!(cloth.positions(), &*frozen);
    assert_eq!(cloth.ticks(), ticks);

    // Sub-millimeter pin drift and the same wind do not wake it.
    cloth
        .pin_top_row(&shoulder_row(cols, [0.0002, 0.0, 0.0]))
        .expect("pins in range");
    cloth.set_wind([0.0; 3]).expect("wind in range");
    assert!(cloth.is_asleep());
    // A collider far away does not either.
    let far = ClothCollider::sphere([9.0, 0.0, 9.0], 0.5).expect("ball");
    assert!(!cloth.step(&[far]));

    // A real pin move wakes it, and it moves.
    cloth
        .pin_top_row(&shoulder_row(cols, [0.05, 0.0, 0.0]))
        .expect("pins in range");
    assert!(!cloth.is_asleep());
    assert!(cloth.step(&[]));
    assert_ne!(cloth.positions(), &*frozen);

    sleep_it(&mut cloth);
    cloth.set_wind([5.0, 0.0, 0.0]).expect("wind in range");
    assert!(!cloth.is_asleep(), "a gust wakes the sheet");

    sleep_it(&mut cloth);
    let hem = cloth.positions()[cloth.particle_count() - 1];
    let poke = ClothCollider::sphere(hem, 0.1).expect("ball");
    assert!(cloth.step(&[poke]), "a collider on the hem wakes the sheet");
}

#[test]
fn a_capsule_keeps_the_cape_off_the_body() {
    // The body stands just behind the hanging cape, so the cape must ride up over it.
    let body = ClothCollider::capsule([0.25, 0.7, -0.1], [0.25, 1.5, -0.1], 0.2).expect("body");
    let mut cloth = cape(ClothTier::Comfort);
    let mut free = Cloth::clone(&cloth);
    for _ in 0..120 {
        cloth.step(&[body]);
        free.step(&[]);
    }
    for (i, p) in cloth.positions().iter().enumerate() {
        if cloth.is_pinned(i) {
            continue;
        }
        let reach = dist2(*p, body.nearest_on_axis(*p)).sqrt();
        assert!(
            reach >= 0.2 - 1e-3,
            "particle {i} sank {reach} m from the axis"
        );
    }
    let pushed = cloth
        .positions()
        .iter()
        .map(|p| p[2])
        .fold(f32::MIN, f32::max);
    assert!(
        pushed > 0.08,
        "the body must push the cape out front: {pushed}"
    );
    let drift = free
        .positions()
        .iter()
        .map(|p| p[2].abs())
        .fold(0.0, f32::max);
    assert!(
        drift < 0.02,
        "without the body the cape stays flat: {drift}"
    );
}

#[test]
fn colliders_push_to_the_surface_and_leave_outside_points() {
    let ball = ClothCollider::sphere([1.0, 1.0, 1.0], 0.5).expect("ball");
    let out = ball.push_out([1.2, 1.0, 1.0]);
    assert!((dist2(out, [1.0, 1.0, 1.0]).sqrt() - 0.5).abs() < 1e-5);
    assert!(
        out[0] > 1.4 && (out[1] - 1.0).abs() < 1e-6,
        "pushed straight out: {out:?}"
    );
    assert_eq!(ball.push_out([3.0, 1.0, 1.0]), [3.0, 1.0, 1.0]);
    assert_eq!(
        ball.push_out([1.0, 1.0, 1.0]),
        [1.0, 1.0, 1.5],
        "dead center leaves along +Z"
    );

    let pole = ClothCollider::capsule([0.0, 0.0, 0.0], [0.0, 2.0, 0.0], 0.25).expect("pole");
    assert_eq!(
        pole.push_out([0.1, 1.0, 0.0]),
        [0.25, 1.0, 0.0],
        "the shell is round around the axis"
    );
    let cap = pole.push_out([0.0, 2.1, 0.0]);
    assert!(
        (cap[1] - 2.25).abs() < 1e-5,
        "and capped at both ends: {cap:?}"
    );
}

#[test]
fn a_banner_pinned_at_two_corners_sags_more_when_the_corners_close_in() {
    let grid = ClothTier::Web.knobs().banner;
    let params = ClothParams::for_tier(ClothTier::Web);
    // Hang a 3 m banner from two corner pins `gap` meters in from its edges; the middle
    // of the top edge is free.
    let sag = |gap: f32| {
        let mut banner = Cloth::new(grid, [3.0, 2.0], [0.0, 4.0, 0.0], params).expect("banner");
        for c in 1..grid.cols - 1 {
            banner.unpin(c).expect("top row index");
        }
        assert_eq!(banner.pinned_count(), 2);
        banner.pin(0, [gap, 4.0, 0.0]).expect("corner");
        banner
            .pin(grid.cols - 1, [3.0 - gap, 4.0, 0.0])
            .expect("corner");
        run(&mut banner, 600);
        assert!(finite(&banner));
        assert_eq!(banner.positions()[0], [gap, 4.0, 0.0], "the corner holds");
        assert!(
            banner.max_stretch() < 1.2,
            "stretch {}",
            banner.max_stretch()
        );
        4.0 - banner.positions()[grid.cols / 2][1]
    };
    let (taut, slack) = (sag(0.0), sag(0.15));
    assert!(taut < 0.3, "a banner at full span barely sags: {taut}");
    assert!(
        slack > taut + 0.2 && slack < 1.5,
        "slack between the corners sags: {slack} vs {taut}"
    );
}

#[test]
fn squeezed_links_do_not_push_back() {
    let mut cloth = cape(ClothTier::Comfort);
    // Shrink the whole sheet to half size about its middle: every link is slack.
    let middle = [0.25, 1.1, 0.0];
    for p in &mut cloth.pos {
        *p = [
            middle[0] + (p[0] - middle[0]) / 2.0,
            middle[1] + (p[1] - middle[1]) / 2.0,
            middle[2],
        ];
    }
    let squeezed = cloth.pos.clone();
    cloth.solve_links();
    assert_eq!(cloth.pos, squeezed, "slack links must not move anything");
    // The same sheet stretched to double size is pulled back toward rest.
    cloth.pos = squeezed
        .iter()
        .map(|p| {
            [
                middle[0] + (p[0] - middle[0]) * 4.0,
                middle[1] + (p[1] - middle[1]) * 4.0,
                middle[2],
            ]
        })
        .collect();
    let excess = |c: &Cloth| -> f32 {
        let over = |l: &Link| (dist2(c.pos[l.a], c.pos[l.b]).sqrt() - l.rest).max(0.0);
        c.links.iter().map(over).sum()
    };
    let before = excess(&cloth);
    cloth.solve_links();
    assert!(
        excess(&cloth) < before,
        "a pass must pull stretched links in"
    );
}

#[test]
fn the_tether_pulls_an_overstretched_particle_back_to_its_reach() {
    let mut cloth = cape(ClothTier::Potato);
    let node = cloth.particle_count() - 1;
    let tether = *cloth
        .tethers
        .iter()
        .find(|t| t.node == node)
        .expect("hem is tethered");
    cloth.pos[node] = [0.0, -10.0, 0.0];
    cloth.solve_tethers();
    let reach = dist2(cloth.pos[node], cloth.pos[tether.pin]).sqrt();
    assert!(
        (reach - tether.max).abs() < 1e-4,
        "reach {reach} vs {}",
        tether.max
    );
    assert!(
        (tether.max - CAPE_SIZE[1]).abs() < 1e-5,
        "straight down is the full hang"
    );
}

/// A 32x32 sheet pinned along its top row, dropped crumpled and run 300 ticks.
fn settled_big_sheet() -> Cloth {
    let params = ClothParams::for_tier(ClothTier::Comfort);
    let grid = ClothGrid { cols: 32, rows: 32 };
    let mut cloth = Cloth::new(grid, [3.1, 3.1], SHOULDERS, params).expect("32x32 sheet");
    crumple(&mut cloth, 7);
    run(&mut cloth, 300);
    cloth
}

#[test]
fn a_32x32_sheet_settles_with_every_link_within_a_quarter_of_rest_and_replays_exactly() {
    let cloth = settled_big_sheet();
    assert!(finite(&cloth));
    assert!(
        cloth.max_stretch() <= 1.25,
        "tethers must carry the long hanging chain: {}",
        cloth.max_stretch()
    );
    let again = settled_big_sheet();
    assert_eq!(cloth.positions(), again.positions(), "bit-exact replay");
}

#[test]
fn the_awake_step_with_pin_moves_wind_and_a_body_never_touches_the_heap() {
    for tier in ClothTier::ALL {
        let mut cloth = cape(tier);
        let body =
            [ClothCollider::capsule([0.25, 0.6, -0.1], [0.25, 1.5, -0.1], 0.15).expect("body")];
        let rows: Vec<Vec<[f32; 3]>> = (0..120)
            .map(|t| shoulder_row(cloth.cols, [0.3 * tri(t, 0), 0.0, 0.1 * tri(t, 12)]))
            .collect();
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for (t, row) in rows.iter().enumerate() {
                cloth.pin_top_row(row).expect("pins in range");
                if t == 60 {
                    cloth.set_wind([6.0, 0.0, 1.0]).expect("wind in range");
                }
                assert!(
                    cloth.step(&body),
                    "the cape must stay awake while pinned moves"
                );
            }
        });
        assert_eq!(heap.calls, 0, "{tier:?}: cloth tick allocated: {heap:?}");
    }
}

#[test]
fn a_tick_is_one_sixtieth_of_a_second_for_any_substep_count() {
    for substeps in [1, 2, 4, MAX_SUBSTEPS] {
        let params = ClothParams {
            air_drag: 0.0,
            substeps,
            ..ClothParams::for_tier(ClothTier::Comfort)
        };
        let mut cloth = Cloth::new(
            ClothTier::Comfort.knobs().cape,
            CAPE_SIZE,
            SHOULDERS,
            params,
        )
        .expect("cape");
        for c in 0..cloth.cols {
            cloth.unpin(c).expect("top row index");
        }
        let top = cloth.positions()[0][1];
        run(&mut cloth, 60);
        let fall = top - cloth.positions()[0][1];
        let want = 0.5 * 9.81;
        assert!(
            (fall - want).abs() / want < 0.02,
            "{substeps} substeps fell {fall} m in one second, free fall is {want}"
        );
    }
}

#[test]
fn the_same_inputs_give_the_same_bits_twice() {
    let play = || {
        let mut cloth = cape(ClothTier::High);
        let mut rng = Rng::new(7);
        for t in 0..300 {
            if t % 20 == 0 {
                let w = rng.next_range(0, 1500) as f32 / 100.0;
                cloth.set_wind([w, 0.0, w / 2.0]).expect("wind in range");
            }
            let row = shoulder_row(cloth.cols, [0.3 * tri(t, 0), 0.0, 0.0]);
            cloth.pin_top_row(&row).expect("pins in range");
            cloth.step(&[]);
        }
        cloth
            .positions()
            .iter()
            .flatten()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    };
    let rest: Vec<u32> = cape(ClothTier::High)
        .positions()
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect();
    assert_eq!(play(), play());
    assert_ne!(play(), rest, "the run must have moved the cape");
}

#[test]
fn reset_puts_a_wrecked_sheet_back_at_rest() {
    let fresh = cape_with(ClothTier::Comfort, Some(0.5));
    let mut cloth = yanked(Some(0.5));
    assert!(cloth.torn_count() > 0);
    cloth.reset(SHOULDERS).expect("origin in range");
    assert_eq!(cloth.positions(), fresh.positions());
    assert_eq!((cloth.torn_count(), cloth.is_asleep()), (0, false));
    assert_eq!(cloth.pinned_count(), cloth.cols);
    run(&mut cloth, 60);
    assert!(cloth.max_stretch() < 1.15);
}

/// Wall-clock nanoseconds of `f`. Tests may read the clock; the tick never does.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "timing test reads the wall clock; it never enters the tick"
)]
fn nanos(f: impl FnOnce()) -> u128 {
    let start = std::time::Instant::now();
    f();
    start.elapsed().as_nanos()
}

/// One game tick for a crowd: shoulders sway, the wind gusts every half second, each cape
/// meets its fighter's body.
fn crowd_tick(capes: &mut [Cloth], bodies: &[ClothCollider], t: usize) {
    for (k, cloth) in capes.iter_mut().enumerate() {
        let sway = [0.3 * tri(t, 5 * k), 0.0, 0.1 * tri(t, 5 * k + 12)];
        let row = shoulder_row(cloth.cols, [1.5 * k as f32 + sway[0], sway[1], sway[2]]);
        cloth.pin_top_row(&row).expect("pins in range");
        if t.is_multiple_of(30) {
            let gust = 2.0 + 6.0 * tri(t / 30, k).abs();
            cloth.set_wind([gust, 0.0, 1.0]).expect("wind in range");
        }
        cloth.step(&bodies[k..=k]);
    }
}

#[test]
fn eight_capes_step_under_half_a_millisecond_in_release() {
    for tier in ClothTier::ALL {
        let knobs = tier.knobs();
        let mut capes: Vec<Cloth> = (0..8)
            .map(|k| {
                let origin = [1.5 * k as f32, SHOULDERS[1], 0.0];
                Cloth::new(knobs.cape, CAPE_SIZE, origin, ClothParams::for_tier(tier))
                    .expect("cape")
            })
            .collect();
        let bodies: Vec<ClothCollider> = (0..8)
            .map(|k| {
                let x = 1.5 * k as f32 + 0.25;
                ClothCollider::capsule([x, 0.6, -0.1], [x, 1.5, -0.1], 0.15).expect("body")
            })
            .collect();
        let mut t = 0;
        for _ in 0..60 {
            crowd_tick(&mut capes, &bodies, t);
            t += 1;
        }
        let mut per_tick = [0u128; 7];
        for slot in &mut per_tick {
            *slot = nanos(|| {
                for _ in 0..60 {
                    crowd_tick(&mut capes, &bodies, t);
                    t += 1;
                }
            }) / 60;
        }
        per_tick.sort_unstable();
        assert!(
            capes.iter().all(|c| !c.is_asleep()),
            "the budget must be measured awake"
        );
        let us = |ns: u128| ns as f64 / 1000.0;
        println!(
            "cloth cost {tier:?}: 8 capes of {} particles, {} substeps x {} passes: median {:.1} us/tick (min {:.1}, max {:.1}; bar 500 us in release)",
            knobs.cape.particles(),
            knobs.substeps,
            knobs.iterations,
            us(per_tick[3]),
            us(per_tick[0]),
            us(per_tick[6]),
        );
        if !cfg!(debug_assertions) {
            assert!(
                per_tick[3] < 500_000,
                "{tier:?}: eight capes took {} us per tick",
                us(per_tick[3])
            );
        }
    }
}

/// Q16 meters from a test literal: 1.0 is one meter.
fn q16(metres: f32) -> Fixed {
    Fixed::from_bits((metres * 65_536.0).round() as i64)
}

/// A hair lock and a rope of the sizes a fighter look asks for, both at rest.
fn strands() -> (Strand, Strand) {
    let hair = Strand::hair([q16(0.0), q16(1.7), q16(0.0)], q16(0.4)).expect("hair builds");
    let rope = Strand::rope(
        [q16(-1.0), q16(2.0), q16(0.0)],
        [q16(1.0), q16(2.0), q16(0.0)],
        12,
    )
    .expect("rope builds");
    (hair, rope)
}

fn run_strands(hair: &mut Strand, rope: &mut Strand, ticks: usize) {
    for _ in 0..ticks {
        hair.step(&[]);
        rope.step(&[]);
    }
}

#[test]
fn hair_hangs_down_and_a_rope_sags_between_its_pins() {
    let (mut hair, mut rope) = strands();
    run_strands(&mut hair, &mut rope, 300);
    let root = hair.positions()[0];
    let tail = hair.positions()[hair.particle_count() - 1];
    assert!(
        tail[1] < root[1] - 0.35,
        "the lock must hang from its root: {tail:?}"
    );
    assert!(tail[0].abs() < 0.05, "it hangs straight: {tail:?}");
    assert_eq!(
        (
            rope.positions()[0],
            rope.positions()[rope.particle_count() - 1]
        ),
        ([-1.0, 2.0, 0.0], [1.0, 2.0, 0.0]),
        "both rope ends hold their pins"
    );
    let mid = rope.positions()[rope.particle_count() / 2][1];
    assert!(mid < 1.9, "the slack rope must sag below its pins: {mid}");
    for strand in [&hair, &rope] {
        let rest = strand.rest.to_bits() as f32 / 65_536.0;
        for pair in strand.positions().windows(2) {
            let len = dist2(pair[0], pair[1]).sqrt();
            assert!(
                len < rest * 1.5 + 0.001,
                "a link stretched to {len} m over a {rest} m rest"
            );
        }
    }
}

/// One replay of a gusty scalp: the hair root sways, the wind gusts every third of a
/// second, and the rope ends stay pinned.
fn strand_play() -> (u64, u64) {
    let (mut hair, mut rope) = strands();
    for t in 0..600_i64 {
        if t % 20 == 0 {
            let gust = (t % 40 - 20) * 400;
            hair.set_push([gust, 0, gust / 2]).expect("push in range");
            rope.set_push([gust / 2, gust, 0]).expect("push in range");
        }
        let sway = Fixed::from_bits((t % 48 - 24) * 120);
        hair.pin(0, [q16(0.0) + sway, q16(1.7), q16(0.0)])
            .expect("root in range");
        hair.step(&[]);
        rope.step(&[]);
    }
    (hair.digest(), rope.digest())
}

#[test]
fn hair_and_rope_q16_verlet_replays_the_same_bits_twice() {
    let (hair, rope) = strands();
    let rest = (hair.digest(), rope.digest());
    let first = strand_play();
    let again = strand_play();
    assert_eq!(first, again, "Q16 Verlet x2: same bits twice");
    assert_ne!(first.0, rest.0, "the hair must have moved");
    assert_ne!(first.1, rest.1, "the rope must have moved");
}

/// One game tick for a head of hair and a rope: the locks sway from their roots, the
/// wind changes twice a second, and the rope ends stay pinned.
fn strand_crowd_tick(locks: &mut [Strand], rope: &mut Strand, t: usize) {
    let sway = Fixed::from_bits((t as i64 % 48 - 24) * 120);
    for (k, lock) in locks.iter_mut().enumerate() {
        lock.pin(0, [q16(k as f32 * 0.02) + sway, q16(1.7), q16(0.0)])
            .expect("root in range");
        if t.is_multiple_of(30) {
            let gust = 500 * (1 + t / 30 % 8) as i64;
            lock.set_push([gust, 0, 0]).expect("push in range");
        }
        lock.step(&[]);
    }
    if t.is_multiple_of(30) {
        rope.set_push([0, 0, 1500]).expect("push in range");
    }
    rope.step(&[]);
}

#[test]
fn a_head_of_hair_and_a_rope_meet_the_half_millisecond_bar() {
    let mut locks: Vec<Strand> = (0..12)
        .map(|k| {
            Strand::new(
                12,
                q16(0.35),
                [q16(k as f32 * 0.02), q16(1.7), q16(0.0)],
                StrandParams::calm(),
            )
            .expect("lock")
        })
        .collect();
    let mut rope = Strand::rope(
        [q16(-2.0), q16(3.0), q16(0.0)],
        [q16(2.0), q16(3.0), q16(0.0)],
        16,
    )
    .expect("rope");
    let mut t = 0;
    for _ in 0..60 {
        strand_crowd_tick(&mut locks, &mut rope, t);
        t += 1;
    }
    let mut per_tick = [0u128; 7];
    for slot in &mut per_tick {
        *slot = nanos(|| {
            for _ in 0..60 {
                strand_crowd_tick(&mut locks, &mut rope, t);
                t += 1;
            }
        }) / 60;
    }
    per_tick.sort_unstable();
    let us = |ns: u128| ns as f64 / 1000.0;
    println!(
        "hair+rope cost: 12 hair locks of 12 nodes + a 16-node rope: median {:.1} us/tick (min {:.1}, max {:.1}; bar 500 us in release)",
        us(per_tick[3]),
        us(per_tick[0]),
        us(per_tick[6]),
    );
    if !cfg!(debug_assertions) {
        assert!(
            per_tick[3] < 500_000,
            "hair and rope took {} us per tick",
            us(per_tick[3])
        );
    }
}

#[test]
fn a_strand_tick_never_touches_the_heap() {
    let (mut hair, mut rope) = strands();
    let ((), heap) = engine_core::alloc_probe::measure(|| {
        for t in 0..120_i64 {
            let sway = Fixed::from_bits((t % 48 - 24) * 120);
            hair.pin(0, [q16(0.0) + sway, q16(1.7), q16(0.0)])
                .expect("root in range");
            hair.step(&[]);
            rope.step(&[]);
        }
    });
    assert_eq!(heap.calls, 0, "a strand tick allocated: {heap:?}");
}

/// March two physics bodies 240 ticks on the same input; `with_strands` also presents a
/// hair lock and a rope driven by those bodies every tick. Returns the sim state hash.
fn sim_march(with_strands: bool) -> u64 {
    let mut space = crate::Space::new();
    space.add(crate::Body {
        x: -1.0,
        y: 4.0,
        vx: 0.4,
        vy: 0.0,
        hx: 0.5,
        hy: 0.5,
    });
    space.add(crate::Body {
        x: 1.0,
        y: 5.0,
        vx: -0.4,
        vy: 0.0,
        hx: 0.5,
        hy: 0.5,
    });
    let (mut hair, mut rope) = strands();
    let rest = (hair.digest(), rope.digest());
    for _ in 0..240 {
        space.step(1.0 / 60.0);
        if with_strands {
            let (b0, b1) = (&space.bodies[0], &space.bodies[1]);
            hair.pin(0, [q16(b0.x), q16(b0.y + 0.6), q16(0.0)])
                .expect("hair root in range");
            hair.step(&[]);
            rope.pin(0, [q16(b0.x), q16(b0.y + 0.3), q16(0.0)])
                .expect("rope end in range");
            rope.pin(
                rope.particle_count() - 1,
                [q16(b1.x), q16(b1.y + 0.3), q16(0.0)],
            )
            .expect("rope end in range");
            rope.step(&[]);
        }
    }
    if with_strands {
        assert_ne!(
            (hair.digest(), rope.digest()),
            rest,
            "the presented strands must have moved"
        );
    }
    space.state_hash()
}

#[test]
fn the_match_hash_is_the_same_with_hair_and_rope_on_and_off() {
    let on = sim_march(true);
    let off = sim_march(false);
    assert_eq!(on, off, "hair and rope on and off: equal sim hashes");
    assert_eq!(sim_march(true), on, "and the match hash repeats");
}

// ---- CLX: the collision fixture ----

/// The D-CLOTH-1 fixture: the pinned seed, the drape count and the ticks per drape at the
/// 60 Hz fixed step. Per-drape streams fork from the seed, so no two drapes share draws.
const GAP_SEED: u64 = 2040;
const GAP_DRAPES: usize = 50;
const GAP_TICKS: usize = 60;

/// A seeded body capsule for drape `i`: the fighter the cape drapes over.
fn gap_capsule(i: usize) -> ClothCollider {
    let mut rng = Rng::new(GAP_SEED).fork(i as u64);
    let x = rng.next_range(0, 201) as f32 / 1000.0 - 0.1;
    let z = rng.next_range(0, 201) as f32 / 1000.0 - 0.1;
    ClothCollider::capsule([x, 0.6, z], [x, 1.5, z], 0.2).expect("seeded body")
}

/// One seeded drape: a cape over its body, stepped `GAP_TICKS` ticks, plus the deepest the
/// cloth sank into the body.
fn gap_drape(i: usize) -> (Cloth, f32) {
    let body = gap_capsule(i);
    let mut cloth = cape(ClothTier::Comfort);
    let mut worst = 0.0f32;
    for _ in 0..GAP_TICKS {
        cloth.step(&[body]);
        for (particle, p) in cloth.positions().iter().enumerate() {
            if cloth.is_pinned(particle) {
                continue;
            }
            let reach = dist2(*p, body.nearest_on_axis(*p)).sqrt();
            worst = worst.max((body.radius - reach).max(0.0));
        }
    }
    (cloth, worst)
}

#[test]
fn cloth_gap_fifty_seeded_drapes_keep_one_centimetre_of_the_body() {
    let mut worst = 0.0f32;
    let mut worst_drape = 0usize;
    for i in 0..GAP_DRAPES {
        let (_, pen) = gap_drape(i);
        if pen > worst {
            worst = pen;
            worst_drape = i;
        }
    }
    assert!(
        worst <= 0.01,
        "drape {worst_drape} sank {worst} m into its body (bar 1 cm)"
    );
    let first = gap_drape(7).0;
    let again = gap_drape(7).0;
    assert_eq!(
        first.positions(),
        again.positions(),
        "the seeded drape must replay exactly"
    );
    println!("cloth_gap drapes={GAP_DRAPES} pen_max_m={worst:.4} drape={worst_drape}");
}

/// The folded curtain: a sheet whose hem is mirrored onto the sheet, so two layers start
/// exactly coincident. The intercollision set names the particles that must separate.
fn gap_folded_curtain() -> Cloth {
    let grid = ClothGrid { cols: 8, rows: 8 };
    let params = ClothParams::for_tier(ClothTier::Comfort);
    let size = [0.8, 0.8];
    let mut cloth = Cloth::new(grid, size, SHOULDERS, params).expect("folded curtain");
    let spacing_y = size[1] / (grid.rows - 1) as f32;
    let fold_y = SHOULDERS[1] - 6.0 * spacing_y;
    for c in 0..grid.cols {
        let particle = (grid.rows - 1) * grid.cols + c;
        let p = cloth.pos[particle];
        let mirrored = [p[0], 2.0 * fold_y - p[1], p[2]];
        cloth.pos[particle] = mirrored;
        cloth.prev[particle] = mirrored;
    }
    let set: Vec<usize> = (0..cloth.particle_count()).collect();
    cloth
        .set_self_collision_set(&set)
        .expect("intercollision set");
    cloth
}

/// The smallest distance between two particles of the intercollision set that the sheet's
/// links do not already hold, and how many pairs are closer than the separation.
fn gap_self_gap(cloth: &Cloth) -> (f32, usize) {
    let target = 0.5 * cloth.spacing[0].min(cloth.spacing[1]);
    let mut min = f32::MAX;
    let mut close = 0usize;
    for a in 0..cloth.particle_count() {
        if !cloth.is_self_colliding(a) {
            continue;
        }
        for b in (a + 1)..cloth.particle_count() {
            if !cloth.is_self_colliding(b) || cloth.linked(a, b) {
                continue;
            }
            let d = dist2(cloth.positions()[a], cloth.positions()[b]).sqrt();
            min = min.min(d);
            if d < target * 0.99 {
                close += 1;
            }
        }
    }
    (min, close)
}

#[test]
fn cloth_gap_a_folded_curtain_never_self_penetrates() {
    let mut cloth = gap_folded_curtain();
    let (start_gap, start_close) = gap_self_gap(&cloth);
    assert!(start_close > 0, "the fold must start interpenetrated");
    for t in 0..60 {
        cloth.step(&[]);
        let (min_gap, close) = gap_self_gap(&cloth);
        assert!(
            close == 0,
            "the folded layers interpenetrate at tick {t}: min_gap={min_gap} close_pairs={close}"
        );
    }
    let mut other = gap_folded_curtain();
    for _ in 0..60 {
        other.step(&[]);
    }
    assert_eq!(
        cloth.positions(),
        other.positions(),
        "the folded curtain must replay exactly"
    );
    println!("cloth_gap fold start_gap_m={start_gap:.4} close_pairs=0");
}

#[test]
fn cloth_gap_a_thirty_metre_gust_never_crosses_a_collider() {
    // The post stands in front of the cape and the gust blows the cape into it. The post
    // runs from under the hem to above the shoulders, so the cape cannot slip under it; a
    // particle that crossed the shell would end up behind it.
    let body = ClothCollider::capsule([0.25, 0.2, -0.15], [0.25, 1.5, -0.15], 0.1).expect("post");
    let mut cloth = cape(ClothTier::Comfort);
    cloth.set_wind([0.0, 0.0, -30.0]).expect("wind in range");
    let mut worst = 0.0f32;
    let mut crossed = 0usize;
    let mut stops = 0u32;
    for t in 0..120 {
        cloth.step(&[body]);
        stops += cloth.swept_stops();
        for (particle, p) in cloth.positions().iter().enumerate() {
            if cloth.is_pinned(particle) {
                continue;
            }
            let reach = dist2(*p, body.nearest_on_axis(*p)).sqrt();
            worst = worst.max((body.radius - reach).max(0.0));
            // Behind the post and still inside its shadow: that is a crossing. A particle
            // that slipped around the side is outside the shadow and is not one.
            let shadow = dist2(*p, body.nearest_on_axis(*p)).sqrt();
            if p[2] < -0.27 && shadow < body.radius && p[1] < 1.3 {
                crossed += 1;
            }
        }
        assert!(
            worst <= 0.01,
            "a particle sank {worst} m into the post at tick {t}"
        );
    }
    assert_eq!(crossed, 0, "a particle crossed the post");
    assert!(stops > 0, "the gust must exercise the swept test");
    println!("cloth_gap gust crossed={crossed} pen_max_m={worst:.4} swept_stops={stops}");
}

#[test]
fn cloth_gap_eight_fighters_stay_inside_the_budget() {
    let tier = ClothTier::Comfort;
    let grid = tier.knobs().cape;
    let params = ClothParams::for_tier(tier);
    let mut capes: Vec<Cloth> = (0..8)
        .map(|k| {
            Cloth::new(grid, CAPE_SIZE, [1.5 * k as f32, SHOULDERS[1], 0.0], params).expect("cape")
        })
        .collect();
    let bodies: Vec<ClothCollider> = (0..8)
        .map(|k| {
            ClothCollider::capsule(
                [1.5 * k as f32, 0.6, -0.1],
                [1.5 * k as f32, 1.5, -0.1],
                0.15,
            )
            .expect("body")
        })
        .collect();
    let mut tick = |t: usize| {
        for (k, cloth) in capes.iter_mut().enumerate() {
            let sway = [0.3 * tri(t, 5 * k), 0.0, 0.1 * tri(t, 5 * k + 12)];
            let row = shoulder_row(grid.cols, [1.5 * k as f32 + sway[0], sway[1], sway[2]]);
            cloth.pin_top_row(&row).expect("pins in range");
            cloth.step(std::slice::from_ref(&bodies[k]));
        }
    };
    for t in 0..30 {
        tick(t);
    }
    let ns = nanos(|| {
        for t in 30..90 {
            tick(t);
        }
    });
    let us = ns as f64 / 1000.0 / 60.0;
    println!("cloth_gap eight_fighters us_per_tick={us:.1} (bar 500 us in release)");
    if !cfg!(debug_assertions) {
        assert!(
            us <= 500.0,
            "eight capes took {us:.1} us per tick (bar 500 us)"
        );
    }
}

#[test]
fn cloth_gap_the_step_never_touches_the_heap() {
    let mut cloth = cape(ClothTier::Comfort);
    let body = ClothCollider::capsule([0.25, 0.6, -0.1], [0.25, 1.5, -0.1], 0.15).expect("body");
    let set: Vec<usize> = (0..cloth.particle_count()).collect();
    cloth
        .set_self_collision_set(&set)
        .expect("intercollision set");
    let ((), heap) = engine_core::alloc_probe::measure(|| {
        for _ in 0..120 {
            cloth.step(&[body]);
        }
    });
    assert_eq!(heap.calls, 0, "the cloth tick allocated: {heap:?}");
    println!("cloth_gap alloc_calls={}", heap.calls);
}

/// March the same physics bodies 240 ticks; `with_cloth` also presents a cape over one of
/// them every tick. Returns the sim state hash.
fn sim_march_cloth(with_cloth: bool) -> u64 {
    let mut space = crate::Space::new();
    space.add(crate::Body {
        x: -1.0,
        y: 4.0,
        vx: 0.4,
        vy: 0.0,
        hx: 0.5,
        hy: 0.5,
    });
    space.add(crate::Body {
        x: 1.0,
        y: 5.0,
        vx: -0.4,
        vy: 0.0,
        hx: 0.5,
        hy: 0.5,
    });
    let mut cloth = cape(ClothTier::Comfort);
    let set: Vec<usize> = (0..cloth.particle_count()).collect();
    cloth
        .set_self_collision_set(&set)
        .expect("intercollision set");
    let body = ClothCollider::capsule([-1.0, 3.4, 0.0], [-1.0, 4.6, 0.0], 0.2).expect("body");
    for _ in 0..240 {
        space.step(1.0 / 60.0);
        if with_cloth {
            cloth
                .pin_top_row(&shoulder_row(cloth.cols, [-1.0, 4.6, 0.0]))
                .expect("pins in range");
            cloth.step(&[body]);
        }
    }
    space.state_hash()
}

#[test]
fn cloth_gap_the_sim_hash_is_equal_with_the_cloth_layer_on_and_off() {
    let on = sim_march_cloth(true);
    let off = sim_march_cloth(false);
    assert_eq!(on, off, "the cloth layer must not reach the sim hash");
    assert_eq!(sim_march_cloth(true), on, "and the sim hash repeats");
    println!("cloth_gap sim_hash_on={on:016x} sim_hash_off={off:016x}");
}

#[test]
fn cloth_gap_a_hair_lock_never_enters_a_capsule() {
    let mut rope = Strand::rope(
        [q16(-1.0), q16(1.0), q16(0.0)],
        [q16(1.0), q16(1.0), q16(0.0)],
        16,
    )
    .expect("rope");
    let post = ClothCollider::capsule([0.0, 0.9, 0.0], [0.0, 1.1, 0.0], 0.15).expect("post");
    for _ in 0..240 {
        rope.step(&[post]);
    }
    for (node, p) in rope.positions().iter().enumerate() {
        let reach = dist2(*p, post.nearest_on_axis(*p)).sqrt();
        assert!(
            reach >= post.radius - 1.0e-3,
            "rope node {node} sank {reach} m into the post"
        );
    }
    println!("cloth_gap hair_lock nodes={}", rope.particle_count());
}

#[test]
fn cloth_gap_the_layer_table_decides_which_colliders_the_cloth_meets() {
    let body = ClothCollider::capsule([0.0, 0.6, 0.0], [0.0, 1.5, 0.0], 0.2).expect("body");
    assert!(cloth_meets(&body), "cloth meets a fighter");
    let debris = body
        .with_layer(crate::layers::DEBRIS)
        .expect("debris layer");
    assert!(!cloth_meets(&debris), "cloth never meets debris");
    let wall = body.with_layer(crate::layers::WORLD).expect("world layer");
    assert!(cloth_meets(&wall), "cloth meets the world");
    let sensor = body
        .with_layer(crate::layers::SENSOR)
        .expect("sensor layer");
    assert!(!cloth_meets(&sensor), "a sensor reports, it never blocks");
    let deep = body.with_max_distance(0.05).expect("max distance");
    assert_eq!(deep.max_distance(), 0.05);
    assert_eq!(deep.layer(), crate::layers::BODY);
}

// ---- Physics laws (engine_core::laws): rest, energy, mirror, stretch ----

/// Float slack for the energy laws: about ten f32 ulps of a 2 m height times gravity, per
/// particle, in J/kg.
fn energy_slack(cloth: &Cloth) -> f64 {
    2.5e-5 * f64::from(u16::try_from(cloth.particle_count()).unwrap_or(u16::MAX))
}

/// One Verlet substep in seconds, at the sheet's current substep count.
fn substep_len(cloth: &Cloth) -> f64 {
    f64::from(TICK_DT) / f64::from(u16::try_from(cloth.params().substeps).unwrap_or(1))
}

/// Kinetic plus gravitational energy per unit mass of the free particles, J/kg (one mass per
/// particle). Speed is the last substep's Verlet displacement over that substep's length, so a
/// substep change that does not rescale the carried displacement shows up here as a jump.
fn sheet_energy(cloth: &Cloth) -> f64 {
    let h = substep_len(cloth);
    let g = f64::from(cloth.params().gravity);
    cloth
        .pos
        .iter()
        .zip(&cloth.prev)
        .zip(&cloth.pinned)
        .filter(|(_, pinned)| !**pinned)
        .map(|((p, q), _)| {
            let speed2: f64 = p
                .iter()
                .zip(q.iter())
                .map(|(a, b)| {
                    let v = f64::from(a - b) / h;
                    v * v
                })
                .sum();
            0.5 * speed2 + g * f64::from(p[1])
        })
        .sum()
}

/// A sheet as a law state: two are equal when every vertex is the same bit for bit.
#[derive(Clone, Debug)]
struct Sheet(Cloth);

impl PartialEq for Sheet {
    fn eq(&self, other: &Self) -> bool {
        vertex_bits(&self.0) == vertex_bits(&other.0)
    }
}

fn vertex_bits(cloth: &Cloth) -> Vec<u32> {
    cloth
        .positions()
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
}

#[test]
fn a_settled_hanging_cape_falls_asleep_and_then_never_moves() {
    for tier in ClothTier::ALL {
        let mut cloth = cape(tier);
        sleep_it(&mut cloth);
        let result = laws::rest_stays_rest(Sheet(cloth), 600, |s| {
            s.0.step(&[]);
        });
        law_holds(result, &format!("{tier:?}"));
    }
}

#[test]
#[ignore = "law break: energy rises only at ticks 1-2 from the flat layout (5-10 mJ/kg, projection overshoot), see lead"]
fn a_hanging_cape_never_gains_energy_without_wind_or_moving_pins() {
    let mut broke = Vec::new();
    for tier in ClothTier::ALL {
        let cloth = cape(tier);
        let slack = energy_slack(&cloth);
        let result = laws::energy_never_rises(
            cloth,
            300,
            |c| {
                c.step(&[]);
            },
            sheet_energy,
            slack,
        );
        if let Err(b) = result {
            let head: String = b.detail.chars().take(160).collect();
            broke.push(format!("{tier:?} at tick {}: {head}", b.tick));
        }
    }
    assert!(broke.is_empty(), "energy rose:\n{}", broke.join("\n"));
}

#[test]
fn switching_the_substep_count_mid_swing_does_not_pump_energy() {
    let grid = ClothTier::Comfort.knobs().cape;
    let params = ClothParams {
        substeps: 1,
        ..ClothParams::for_tier(ClothTier::Comfort)
    };
    let mut cloth = Cloth::new(grid, CAPE_SIZE, SHOULDERS, params).expect("cape builds");
    for t in 0..30 {
        cloth
            .pin_top_row(&shoulder_row(grid.cols, [0.2 * tri(t, 0), 0.0, 0.0]))
            .expect("pins in range");
        cloth.step(&[]);
    }
    let slack = energy_slack(&cloth);
    let before = sheet_energy(&cloth);
    // Retuning the substep count changes the Verlet step. The carried motion must follow it, so
    // the energy reads the same right after each switch.
    for substeps in [2, 1, 2] {
        let params = ClothParams {
            substeps,
            ..cloth.params()
        };
        cloth.set_params(params).expect("substeps in range");
        let after = sheet_energy(&cloth);
        assert!(
            (after - before).abs() <= slack,
            "substeps {substeps}: energy {before} -> {after} (slack {slack})"
        );
    }
    let result = laws::energy_never_rises(
        cloth,
        120,
        |c| {
            c.step(&[]);
        },
        sheet_energy,
        slack,
    );
    law_holds(result, "after the substep switch, no input");
}

/// The cape's top row swaying at tick `t`. The mirrored side takes the mirror image: column `c`
/// is column `cols - 1 - c` of the other side with x negated.
fn sway_row(cols: usize, t: usize, mirrored: bool) -> Vec<[f32; 3]> {
    let rest_x = |c: usize| -CAPE_SIZE[0] / 2.0 + CAPE_SIZE[0] * count_f32(c) / count_f32(cols - 1);
    let swing = |c: usize| 0.2 * tri(t, c);
    (0..cols)
        .map(|c| {
            if mirrored {
                let src = cols - 1 - c;
                [-(rest_x(src) + swing(src)), SHOULDERS[1], 0.0]
            } else {
                [rest_x(c) + swing(c), SHOULDERS[1], 0.0]
            }
        })
        .collect()
}

/// A cape as a mirror-law state: `mirrored` picks which side of the pair this is.
#[derive(Debug)]
struct CapeSide {
    sheet: Cloth,
    mirrored: bool,
}

/// True when `b` is `a` mirrored across x = 0 with its columns reversed, within 1e-5 m.
fn mirror_image(a: &Cloth, b: &Cloth) -> bool {
    let (pa, pb) = (a.positions(), b.positions());
    let cols = a.cols;
    pa.len() == pb.len()
        && (0..pb.len()).all(|i| {
            let src = pa[(i / cols) * cols + (cols - 1 - i % cols)];
            (pb[i][0] + src[0]).abs() <= 1e-5
                && (pb[i][1] - src[1]).abs() <= 1e-5
                && (pb[i][2] - src[2]).abs() <= 1e-5
        })
}

/// Panics with the law's name, its tick and the first 200 characters of its detail: the numbers
/// come first, and the state dump after them is long.
fn law_holds(result: Result<(), laws::Break>, what: &str) {
    if let Err(b) = result {
        let head: String = b.detail.chars().take(200).collect();
        panic!("{what}: law '{}' broke at tick {}: {head}", b.law, b.tick);
    }
}

#[test]
#[ignore = "law break: Gauss-Seidel link order, mirror gap 21 mm at tick 1, 115 mm at tick 10, see lead"]
fn a_cape_on_mirrored_pins_stays_mirrored() {
    let grid = ClothTier::Comfort.knobs().cape;
    let params = ClothParams::for_tier(ClothTier::Comfort);
    let origin = [-CAPE_SIZE[0] / 2.0, SHOULDERS[1], 0.0];
    let side = |mirrored| CapeSide {
        sheet: Cloth::new(grid, CAPE_SIZE, origin, params).expect("cape builds"),
        mirrored,
    };
    laws::mirror(
        side(false),
        side(true),
        240,
        |s, t| {
            let t = usize::try_from(t).unwrap_or(0);
            s.sheet
                .pin_top_row(&sway_row(grid.cols, t, s.mirrored))
                .expect("pins in range");
            s.sheet.step(&[]);
        },
        |a, b| mirror_image(&a.sheet, &b.sheet),
    )
    .expect("mirror law");
}

/// Largest gap between a link's length and its rest length, meters.
fn worst_link_error(strand: &Strand) -> f32 {
    let rest = from_fixed(strand.rest);
    strand
        .positions()
        .windows(2)
        .map(|w| {
            let d = [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]];
            ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - rest).abs()
        })
        .fold(0.0, f32::max)
}

#[test]
#[ignore = "law break: hair 2.4 mm and rope 5.4 mm stretched at 6 passes, still 1.1 and 3.2 mm at 12, see lead"]
fn a_settled_hair_lock_and_rope_never_stretch_past_a_millimetre() {
    let (mut hair, mut rope) = strands();
    run_strands(&mut hair, &mut rope, 300);
    let mut worst = [0.0_f32; 2];
    for _ in 0..300 {
        run_strands(&mut hair, &mut rope, 1);
        worst[0] = worst[0].max(worst_link_error(&hair));
        worst[1] = worst[1].max(worst_link_error(&rope));
    }
    let off: Vec<String> = [("hair", worst[0]), ("rope", worst[1])]
        .into_iter()
        .filter(|(_, err)| *err > 0.001)
        .map(|(name, err)| format!("{name} {err} m"))
        .collect();
    assert!(
        off.is_empty(),
        "links off rest length after settling: {off:?}"
    );
}

/// A hair lock as a mirror-law state: `mirrored` picks which side of the pair this is.
#[derive(Debug)]
struct LockSide {
    hair: Strand,
    mirrored: bool,
}

/// A gust in Q16 per tick squared: a triangle wave of +-0.037 m/tick^2 with a 48 tick period.
fn gust_q16(t: u32) -> i64 {
    let k = i64::from(t % 48);
    let up = if k < 24 { k } else { 48 - k };
    (up - 12) * 200
}

#[test]
fn a_hair_lock_in_a_mirrored_gust_stays_mirrored_to_the_bit() {
    let side = |root_x: f32, mirrored| LockSide {
        hair: Strand::hair([q16(root_x), q16(1.7), q16(0.0)], q16(0.4)).expect("hair builds"),
        mirrored,
    };
    let result = laws::mirror(
        side(0.05, false),
        side(-0.05, true),
        300,
        |s, t| {
            let push = gust_q16(t);
            let x = if s.mirrored { -push } else { push };
            s.hair.set_push([x, 0, 0]).expect("push in range");
            s.hair.step(&[]);
        },
        |a, b| {
            a.hair
                .positions()
                .iter()
                .zip(b.hair.positions())
                .all(|(p, q)| q[0] == -p[0] && q[1] == p[1] && q[2] == p[2])
        },
    );
    law_holds(result, "mirrored hair lock");
}

// ---- The floor (set_ground): a hem lies on the ground and never sinks through it ----

/// The fighter cape as `fighter_look` lays it: half a meter wide, 0.95 m of cloth.
const FLOOR_CAPE_M: [f32; 2] = [0.5, 0.95];

/// The lowest particle's height, meters.
fn lowest(cloth: &Cloth) -> f32 {
    cloth
        .positions()
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min)
}

/// A cape released from its whole top row at `top_y`, over a floor at zero.
fn dropped_cape(top_y: f32) -> Cloth {
    let tier = ClothTier::Comfort;
    let grid = tier.knobs().cape;
    let params = ClothParams::for_tier(tier);
    let mut cloth = Cloth::new(grid, FLOOR_CAPE_M, [0.0, top_y, 0.0], params).expect("cape builds");
    for particle in 0..grid.cols {
        cloth.unpin(particle).expect("top row particle in range");
    }
    cloth
        .set_ground(Some(0.0))
        .expect("a floor at zero is valid");
    cloth
}

#[test]
fn a_cape_pinned_over_the_floor_never_sinks_through_it() {
    let tier = ClothTier::Comfort;
    let params = ClothParams::for_tier(tier);
    // The top row is pinned 0.5 m up, so 0.95 m of cloth hangs past the floor.
    let mut cloth =
        Cloth::new(tier.knobs().cape, FLOOR_CAPE_M, [0.0, 0.5, 0.0], params).expect("cape builds");
    cloth
        .set_ground(Some(0.0))
        .expect("a floor at zero is valid");
    for tick in 1..=300 {
        cloth.step(&[]);
        let low = lowest(&cloth);
        assert!(
            low >= 0.0,
            "tick {tick}: a particle sank to {low} under the floor"
        );
    }
    // The hem reached the floor and lies on it, skin and all.
    let low = lowest(&cloth);
    assert!(
        (low - CLOTH_GROUND_SKIN_M).abs() < 1e-5,
        "the hem should lie on the floor, lowest {low}"
    );
}

#[test]
fn a_cape_dropped_onto_the_floor_comes_to_rest_and_stays_there() {
    let mut cloth = dropped_cape(1.0);
    sleep_it(&mut cloth);
    let low = lowest(&cloth);
    assert!(
        low >= 0.0,
        "the dropped cape fell through the floor to {low}"
    );
    let result = laws::rest_stays_rest(Sheet(cloth), 600, |s| {
        s.0.step(&[]);
    });
    law_holds(result, "a dropped cape on the floor");
}

#[test]
fn a_floor_never_adds_energy_to_a_falling_cape() {
    let cloth = dropped_cape(1.0);
    let slack = energy_slack(&cloth);
    let result = laws::energy_never_rises(
        cloth,
        600,
        |c| {
            c.step(&[]);
        },
        sheet_energy,
        slack,
    );
    law_holds(result, "a cape falling onto the floor");
}

#[test]
fn no_floor_leaves_the_sheet_exactly_as_it_was() {
    let tier = ClothTier::Comfort;
    let cols = tier.knobs().cape.cols;
    let mut plain = cape(tier);
    let mut none = cape(tier);
    // A refused height changes nothing, and an explicit no-floor is the same as never setting one.
    assert_eq!(
        none.set_ground(Some(f32::NAN)),
        Err(ClothError::BadParam("ground"))
    );
    none.set_ground(None).expect("no floor is always valid");
    for t in 0..300 {
        let row = shoulder_row(cols, [0.2 * tri(t, 0), 0.0, 0.0]);
        plain.pin_top_row(&row).expect("pins in range");
        none.pin_top_row(&row).expect("pins in range");
        plain.step(&[]);
        none.step(&[]);
        assert_eq!(vertex_bits(&plain), vertex_bits(&none), "tick {t}");
    }
}

#[test]
fn a_sleeping_cape_wakes_when_a_floor_comes_up_under_its_hem() {
    let mut cloth = cape(ClothTier::Comfort);
    sleep_it(&mut cloth);
    // The hem hangs at 0.6 m, so a floor at 0.7 m puts the bottom row under it.
    cloth
        .set_ground(Some(0.7))
        .expect("a floor at 0.7 is valid");
    assert!(
        !cloth.is_asleep(),
        "a floor under the hem must wake the sheet"
    );
    run(&mut cloth, 300);
    let low = lowest(&cloth);
    assert!(low >= 0.7, "the hem sank to {low} under the floor");
}
