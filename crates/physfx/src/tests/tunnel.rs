//! tunnel: a fast limb dropped onto another limb must stop at it on the tier with soft CCD, and
//! still pass through on the tier without it (QR6).
//!
//! Both limbs are free bodies in the air, so they fall together and the distance between them
//! only changes by the fast limb's own speed. Nothing else is in the way: the floor is far below
//! and the pair never reaches it. Rapier's ordinary CCD sweeps a fast body against fixed
//! colliders only, so it never stops this pair of moving limbs; the tier with soft CCD looks
//! ahead of the fast limb and stops it.

use super::*;

/// The fast limb's speed, meters per second. At 60 Hz it travels a meter per tick, far past the
/// other limb, so without a look-ahead it passes straight through it.
const FAST_MPS: f32 = 60.0;
/// How far above the slow limb's center the fast limb starts, meters. The two capsules are 42 cm
/// long, so this leaves a 10 cm gap: inside the tier's look-ahead, so the tier with soft CCD sees
/// the pair on the first tick, and far enough that they do not touch at the start.
const START_ABOVE: f32 = 0.52;
/// The height the pair starts at, meters.
const START_Y: f32 = 5.0;
/// The part id of the slow limb.
const SLOW: u32 = 1;
/// The part id of the fast limb.
const FAST: u32 = 2;
/// The ticks the pair is watched for.
const TICKS: usize = 6;

/// Two limbs in the air on the given caps: the slow one at rest, the fast one above it.
fn drop_pair(caps: Caps) -> PhysFx {
    let mut fx = PhysFx::with_caps(caps).expect("caps");
    fx.spawn_limb(SLOW, limb_shape(), at([0.0, START_Y, 0.0]), [0.0; 3])
        .expect("slow limb");
    fx.spawn_limb(
        FAST,
        limb_shape(),
        at([0.0, START_Y + START_ABOVE, 0.0]),
        [0.0, -FAST_MPS, 0.0],
    )
    .expect("fast limb");
    fx
}

/// The height of one limb's center, meters.
fn limb_y(fx: &PhysFx, part: u32) -> f32 {
    fx.transforms(1.0)
        .expect("transforms")
        .into_iter()
        .find(|p| matches!(p.key, Key::Limb { part: id } if id == part))
        .expect("limb")
        .at
        .pos[1]
}

/// The height of the fast limb above the slow one, meters: negative means it passed it.
fn gap(fx: &PhysFx) -> f32 {
    limb_y(fx, FAST) - limb_y(fx, SLOW)
}

/// The tick the fast limb got past the slow one, or `None` when it never did.
fn passed_through(caps: Caps) -> Option<usize> {
    let mut fx = drop_pair(caps);
    for tick in 0..TICKS {
        fx.step();
        let above = gap(&fx);
        println!("physfx tunnel tick {tick}: gap {above:.3} m");
        if above < 0.0 {
            return Some(tick);
        }
    }
    None
}

#[test]
fn a_fast_limb_does_not_tunnel_through_a_limb_at_high() {
    // The tier with soft CCD looks ahead of the fast limb, so the pair meets before the fast limb
    // reaches the slow one and the fast one never gets past it.
    assert!(
        passed_through(Tier::High.caps()).is_none(),
        "the fast limb passed the slow one at High"
    );
}

#[test]
fn the_soft_ccd_flag_is_what_stops_the_fast_limb() {
    // The same world with the same tier and the same two limbs, with the flag off: the fast limb
    // goes straight through. Nothing else changes, so the flag is the cause.
    let without = Caps {
        soft_ccd_mm: 0,
        ..Tier::High.caps()
    };
    assert_eq!(
        passed_through(without),
        Some(0),
        "without the flag the fast limb must still pass through"
    );
}

#[test]
fn a_fast_limb_still_tunnels_through_a_limb_at_comfort() {
    // The comfortable tier has no soft CCD: this is what it did before the flag and what it still
    // does. The tier table is the only thing that changed.
    assert_eq!(
        passed_through(Tier::Comfort.caps()),
        Some(0),
        "the comfortable tier must be unchanged: the fast limb still passes through"
    );
}

#[test]
fn the_soft_ccd_flag_is_on_the_high_tier_only() {
    for tier in [Tier::Potato, Tier::Web, Tier::Comfort] {
        assert_eq!(
            tier.caps().soft_ccd_mm,
            0,
            "{tier:?} must not pay for soft CCD"
        );
    }
    assert!(
        Tier::High.caps().soft_ccd_mm > 0,
        "the high tier must look ahead"
    );
}
