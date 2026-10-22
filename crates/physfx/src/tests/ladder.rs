//! ladder: the tiers are ordered on every knob, each tier holds its caps under a soak and stays
//! under its cost bar, and the receipt line says so.

use super::*;

/// Every knob of a cap table by name. The struct is taken apart with no `..`: a new knob does
/// not compile until it is listed here, so it cannot escape the ordering test.
fn knobs(caps: Caps) -> [(&'static str, usize); 11] {
    let Caps {
        bodies,
        ragdolls,
        contacts,
        statics,
        solver_iterations,
        ccd_substeps,
        soft_ccd_mm,
        blast_bodies,
        impact_bodies,
        tethers,
        tether_nodes,
    } = caps;
    [
        ("bodies", bodies),
        ("ragdolls", ragdolls),
        ("contacts", contacts),
        ("statics", statics),
        ("solver_iterations", solver_iterations),
        ("ccd_substeps", ccd_substeps),
        (
            "soft_ccd_mm",
            usize::try_from(soft_ccd_mm).expect("soft ccd fits usize"),
        ),
        ("blast_bodies", blast_bodies),
        ("impact_bodies", impact_bodies),
        ("tethers", tethers),
        ("tether_nodes", tether_nodes),
    ]
}

/// The first knob where `weaker` has more than `stronger`, or `None` when the pair is ordered.
fn first_unordered(weaker: Caps, stronger: Caps) -> Option<&'static str> {
    knobs(weaker)
        .iter()
        .zip(knobs(stronger))
        .find(|((_, low), (_, high))| *low > *high)
        .map(|((name, _), _)| *name)
}

#[test]
fn ladder_orders_every_knob_and_the_bar_potato_to_high() {
    for pair in Tier::ALL.windows(2) {
        let (weaker, stronger) = (pair[0], pair[1]);
        assert_eq!(
            first_unordered(weaker.caps(), stronger.caps()),
            None,
            "{weaker:?} has more of a knob than {stronger:?}"
        );
        assert!(
            weaker.tick_bar_us() <= stronger.tick_bar_us(),
            "{weaker:?} is allowed more per tick than {stronger:?}"
        );
    }
    for tier in Tier::ALL {
        assert!(tier.tick_bar_us() > 0, "{tier:?} has no bar");
        // A frame is 16.7 ms: physics may take a share of it, never the whole frame.
        assert!(tier.tick_bar_us() <= 8_000, "{tier:?} bar is half a frame");
    }
}

#[test]
fn ladder_ordering_check_names_the_knob_that_breaks_it() {
    let (potato, high) = (Tier::Potato.caps(), Tier::High.caps());
    assert_eq!(first_unordered(potato, high), None);
    assert_eq!(first_unordered(high, potato), Some("bodies"));
    let swapped_contacts = Caps {
        contacts: potato.contacts - 1,
        ..high
    };
    assert_eq!(first_unordered(potato, swapped_contacts), Some("contacts"));
    let more_substeps = Caps {
        ccd_substeps: high.ccd_substeps + 1,
        ..potato
    };
    assert_eq!(first_unordered(more_substeps, high), Some("ccd_substeps"));
}

#[test]
fn ladder_receipt_line_names_bodies_contacts_and_micros_in_order() {
    let stats = Stats {
        bodies: 318,
        awake: 7,
        contacts: 41,
        recycled: 5,
        steps: 600,
        micros: 1_234,
    };
    assert_eq!(
        stats.receipt_line(),
        "physfx: bodies=318 contacts=41 us=1234"
    );
    assert_eq!(
        Stats::default().receipt_line(),
        "physfx: bodies=0 contacts=0 us=0"
    );
    // Only those three counters are in the line.
    let quiet = Stats {
        awake: 0,
        recycled: 0,
        steps: 1,
        ..stats
    };
    assert_eq!(quiet.receipt_line(), stats.receipt_line());
}

/// How many ragdolls, debris bodies and limbs a soak holds.
struct Scene {
    ragdolls: usize,
    debris: usize,
    limbs: usize,
}

/// The plan's soak (8 ragdolls, 200 debris, 6 limbs).
const PLAN: Scene = Scene {
    ragdolls: 8,
    debris: 200,
    limbs: 6,
};

/// The plan's scene cut to what `tier` admits without recycling: the ragdolls up to its ragdoll
/// cap, the limbs, and as much debris as still fits the body cap.
fn fitted(tier: Tier, bones: usize) -> Scene {
    let caps = tier.caps();
    let ragdolls = PLAN.ragdolls.min(caps.ragdolls);
    let limbs = PLAN.limbs;
    let debris = PLAN.debris.min(caps.bodies - ragdolls * bones - limbs);
    Scene {
        ragdolls,
        debris,
        limbs,
    }
}

/// Spawns `scene` over a floor the way the plan's budget test does.
fn fill(fx: &mut PhysFx, rig: &Rig, pose: &LocalPose, scene: &Scene) {
    for slot in 0..scene.ragdolls {
        let slot = u8::try_from(slot).expect("slot");
        let start = moved(pose, [2.5 * f32::from(slot) - 8.75, 0.5, 0.0]);
        let knock = [0.0, 0.0, 40.0 + 10.0 * f32::from(slot)];
        fx.spawn_ragdoll(slot, rig, &start, [0.0, 0.0, 1.0], knock)
            .expect("ragdoll");
    }
    if scene.debris > 0 {
        fx.spawn_debris(&debris_field(scene.debris, 2.0), [0.0, 150.0, 0.0])
            .expect("debris");
    }
    for part in 0..scene.limbs {
        let pos = [part as f32 - 2.5, 2.5, -3.0];
        let part = u32::try_from(part).expect("part");
        fx.spawn_limb(part, limb_shape(), at(pos), [0.0, 1.0, -1.0])
            .expect("limb");
    }
}

/// Ragdoll slots that have at least one bone alive.
fn ragdoll_slots(fx: &PhysFx) -> usize {
    let mut slots: Vec<u8> = fx
        .transforms(1.0)
        .expect("transforms")
        .iter()
        .filter_map(|p| match p.key {
            Key::Bone { slot, .. } => Some(slot),
            _ => None,
        })
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots.len()
}

#[test]
fn ladder_soak_each_tier_holds_its_scene_for_600_ticks_under_its_bar() {
    const TICKS: usize = 600;
    let (rig, pose) = humanoid();
    for tier in Tier::ALL {
        let caps = tier.caps();
        let scene = fitted(tier, rig.bones());
        let mut fx = arena(tier).with_clock(wall_micros);
        fill(&mut fx, &rig, &pose, &scene);
        let bodies = scene.ragdolls * rig.bones() + scene.debris + scene.limbs;
        assert_eq!(fx.stats().bodies, bodies, "{tier:?}: the scene fits whole");
        assert!(bodies <= caps.bodies);

        let mut micros = Vec::with_capacity(TICKS);
        let mut peak_contacts = 0;
        for tick in 0..TICKS {
            fx.step();
            let stats = fx.stats();
            assert!(
                stats.bodies <= caps.bodies,
                "{tier:?} tick {tick}: {} bodies",
                stats.bodies
            );
            micros.push(stats.micros);
            peak_contacts = peak_contacts.max(stats.contacts);
        }
        let last = fx.stats();
        assert_eq!(
            last.recycled, 0,
            "{tier:?}: a scene that fits the tier is never shed"
        );
        assert_eq!(ragdoll_slots(&fx), scene.ragdolls, "{tier:?}");
        assert!(
            last.contacts <= caps.contacts,
            "{tier:?}: {} contacts at the end",
            last.contacts
        );

        micros.sort_unstable();
        let mean = micros.iter().sum::<u64>() / micros.len() as u64;
        let p95 = micros[micros.len() * 95 / 100];
        let bar = tier.tick_bar_us();
        // The receipt: the tier's counters, the worst contact count and the mean step cost.
        let receipt = Stats {
            contacts: peak_contacts,
            micros: mean,
            ..last
        };
        println!(
            "{}  tier={tier:?} ticks={TICKS} p95={p95} bar={bar} awake={}",
            receipt.receipt_line(),
            last.awake
        );
        if !cfg!(debug_assertions) {
            assert!(
                mean < bar,
                "{tier:?}: mean {mean} us is over the {bar} us bar"
            );
            assert!(
                p95 < 3 * bar,
                "{tier:?}: p95 {p95} us is over {} us",
                3 * bar
            );
        }
    }
}

#[test]
fn ladder_overload_never_passes_a_cap_and_recycles_what_does_not_fit() {
    let (rig, pose) = humanoid();
    let demand = PLAN.ragdolls * rig.bones() + PLAN.debris + PLAN.limbs;
    for tier in Tier::ALL {
        let caps = tier.caps();
        let mut fx = arena(tier);
        fill(&mut fx, &rig, &pose, &PLAN);
        for tick in 0..120 {
            fx.step();
            assert!(fx.stats().bodies <= caps.bodies, "{tier:?} tick {tick}");
            assert!(ragdoll_slots(&fx) <= caps.ragdolls, "{tier:?} tick {tick}");
        }
        assert_eq!(
            fx.stats().recycled > 0,
            demand > caps.bodies,
            "{tier:?}: {demand} bodies asked of a cap of {}",
            caps.bodies
        );
    }
}
