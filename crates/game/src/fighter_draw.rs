//! VS4: the fighter views and the bodies drawn from them.
//!
//! [`App::fighter_views`] serves one view per fighter after every tick: slot,
//! feet position and yaw, equal to the dataset rows. The capture path draws a
//! proxy body per view on the floor; the window path loads the bound CT7
//! bodies, poses them at the same views and pushes their part draws.
//!
//! The tests here pin the row's draw leg: every drawn fighter stands at its
//! view, at least two drawn bodies clear 5,000 triangles, and every frame's
//! drawn positions equal the views over a 2-fighter 120-tick run.
//!
//! Test-only: this module is declared under `#[cfg(test)]` and needs `app`
//! for the window draw leg.

use engine_arena::{run_dataset, ArenaConfig};
use engine_input::{Buttons, FighterId, FighterInput};

use crate::{App, GameConfig};

/// The 2-fighter, 120-tick session the row names.
fn row_config() -> GameConfig {
    GameConfig {
        seed: 7,
        ticks: 120,
        fighters: 2,
        half_extent_m: 12.0,
    }
}

/// An accepted do-nothing input on the human slot.
fn idle(tick: u32) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId::new(crate::HUMAN_SLOT).expect("human slot is valid"),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: Buttons::NONE,
    }
}

#[test]
fn fighter_views_match_sim_positions_every_tick() {
    // VS4 F2P half: a 2-fighter 120-tick run serves one view per fighter
    // after every tick, byte-equal to the dataset rows (slot, feet
    // position, yaw). Empty before tick 0.
    let config = row_config();
    let mut app = App::new(config).expect("app starts");
    assert!(app.fighter_views().is_empty());
    for tick in 0..config.ticks {
        assert!(app.tick(idle(tick)).input_accepted);
        let views = app.fighter_views();
        assert_eq!(views.len(), 2, "one view per fighter at tick {tick}");
        let short = ArenaConfig {
            ticks: tick + 1,
            ..app.arena
        };
        let dataset = run_dataset(&short, &app.tape).expect("dataset replays");
        let row = &dataset.rows[usize::try_from(tick).unwrap() * 2];
        for (view, entry) in views.iter().zip(row.observation.fighters.as_slice()) {
            assert_eq!(view.slot, entry.id, "slot order at tick {tick}");
            assert_eq!(view.pos_m, entry.pos, "position at tick {tick}");
            assert_eq!(view.yaw, entry.yaw, "yaw at tick {tick}");
        }
    }
}

#[cfg(feature = "app")]
#[test]
fn vs4_capture_draws_two_bodies_over_5000_tris_every_tick() {
    // VS4 F2P mesh half: over a 2-fighter 120-tick run every frame draws
    // one body per fighter at that view's feet on the floor, and each
    // drawn body mesh carries CT7-scale triangles (over 5,000). The CT7
    // manifest itself binds 2+ bodies over the same floor.
    let config = row_config();
    let dir = crate::fighter_look::fighters_content_dir();
    let bindings = crate::fighter_bind::bind_all(&dir).expect("CT7 manifest binds");
    let big = bindings
        .iter()
        .filter(|binding| binding.tris_lod0 > 5_000)
        .count();
    assert!(
        big >= 2,
        "CT7 binds {big} of {} bodies over 5,000 tris",
        bindings.len()
    );

    let mut app = App::new(config).expect("app starts");
    for tick in 0..config.ticks {
        let outcome = app.tick(idle(tick));
        assert!(outcome.input_accepted, "tick {tick}");
        let views = app.fighter_views();
        let dots = crate::windowed::live_fight_dots(&app, &outcome.cues);
        assert_eq!(
            dots.len(),
            views.len(),
            "one drawn body per view at tick {tick}"
        );
        let resident: Vec<_> = dots
            .iter()
            .map(|dot| crate::windowed::fight_mesh_key(dot.slot))
            .collect();
        let list = crate::windowed::staged_fight_frame_list(128, 96, None, tick, &resident, &dots)
            .expect("fight list builds");
        list.validate().expect("fight list validates");
        for (dot, view) in dots.iter().zip(views) {
            assert_eq!(dot.slot, view.slot, "slot at tick {tick}");
            // The capture body stands on the floor at the view's x/z.
            assert_eq!(
                dot.pos_m,
                [view.pos_m[0], 0.0, view.pos_m[2]],
                "drawn feet at tick {tick}"
            );
            let mesh = crate::windowed::fight_body_mesh(dot.pos_m).expect("body mesh builds");
            let tris = mesh.indices.len() / 3;
            assert!(
                tris > 5_000,
                "drawn body carries CT7-scale tris at tick {tick}: {tris}"
            );
            assert!(
                list.items
                    .iter()
                    .any(|item| item.mesh == crate::windowed::fight_mesh_key(dot.slot)),
                "body {} is drawn at tick {tick}",
                dot.slot
            );
        }
    }
}

#[cfg(feature = "app")]
#[test]
fn vs4_window_look_draws_the_ct7_bodies_at_the_views() {
    // VS4 window leg: the bound CT7 bodies load, pose at every tick's
    // views and push their part draws. Slots 0 and 1 draw bodies whose
    // LOD0 triangles clear 5,000, so two drawn meshes carry the bar.
    use crate::fighter_look::{look_mesh_key, FighterLook};

    let config = row_config();
    let dir = crate::fighter_look::fighters_content_dir();
    let bindings = crate::fighter_bind::bind_all(&dir).expect("CT7 manifest binds");
    let mut look = FighterLook::load(&dir).expect("CT7 bodies load");
    assert_eq!(look.rig_count(), bindings.len(), "one rig per bound body");

    let mut resident = Vec::new();
    for slot in 0..2usize {
        let variant = look.variant_for_slot(slot);
        let binding = bindings
            .iter()
            .find(|binding| binding.variant == variant)
            .expect("slot variant binds");
        assert!(
            binding.tris_lod0 > 5_000,
            "slot {slot} body {} carries {} tris",
            binding.variant,
            binding.tris_lod0
        );
        for part in 0..64usize {
            resident.push(look_mesh_key(slot, part));
        }
    }

    let mut app = App::new(config).expect("app starts");
    for tick in 0..config.ticks {
        let outcome = app.tick(idle(tick));
        assert!(outcome.input_accepted, "tick {tick}");
        let views = app.fighter_views();
        look.observe_tick(tick, views, app.hp(), &outcome.cues);
        let truths = look.pose_frame_joints(2, 1.0).expect("CT7 bodies pose");
        assert_eq!(
            truths.len(),
            views.len(),
            "one posed body per view at tick {tick}"
        );
        for (truth, view) in truths.iter().zip(views) {
            assert_eq!(truth.slot, view.slot, "slot at tick {tick}");
            for axis in 0..3 {
                let delta = (truth.root[axis] - view.pos_m[axis]).abs();
                assert!(
                    delta < 1e-5,
                    "drawn root axis {axis} at tick {tick}: {} vs {}",
                    truth.root[axis],
                    view.pos_m[axis]
                );
            }
        }
        let mut list = crate::windowed::hud_draw_list(128, 96).expect("hud builds");
        let pushed = look.push_draws(&mut list, &resident, 2);
        assert!(pushed >= 2, "part draws at tick {tick}: {pushed}");
        for slot in 0..2usize {
            assert!(
                list.items
                    .iter()
                    .any(|item| item.mesh == look_mesh_key(slot, 0)),
                "slot {slot} first part is drawn at tick {tick}"
            );
        }
    }
}
