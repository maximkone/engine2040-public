//! soft: the Rapier cloth of PX44c: a 4x4 pinned sheet sags, holds its pins exactly, stays
//! finite, settles, and prints the one stable `physfx: soft:` receipt line.

use super::*;
use crate::soft::{ClothDesc, ClothError, SoftCloth, SoftWorld, MAX_CLOTHS};
use crate::verbs::{fx_cloth, ClothEvent};

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
