//! settle: a knocked-down ragdoll lies still and its world falls asleep, which is what the
//! look's "settles in 3 s" reads (`stats().awake == 0`).

use super::sever::{ct7, ct7_bone, ct7_name};
use super::*;

/// The look's blow on a knocked-down fighter: 300 permille of damage at 0.1 N s each, lifted.
fn look_kick() -> [f32; 3] {
    [30.0, 9.0, 0.0]
}

/// One name of the CT7 bones as the key the world reports it under.
fn key_of(name: &str) -> Key {
    Key::Bone {
        slot: 0,
        bone: u16::try_from(ct7_bone(name)).expect("bone fits"),
    }
}

/// The world transform of one body this tick.
fn at(fx: &PhysFx, name: &str) -> Transform {
    fx.transforms(1.0)
        .expect("transforms")
        .into_iter()
        .find(|one| one.key == key_of(name))
        .expect("body")
        .at
}

/// The look's fall: the standing body is spawned, hit once and (QR12) loses one upper arm.
/// Prints the awake count every 40 ticks and names every body still awake at the end.
fn fall(sever: Option<&str>, ticks: u32) -> u32 {
    let (rig, pose) = ct7();
    let mut fx = on_floor(PhysFx::new(Tier::Comfort));
    fx.spawn_ragdoll(0, &rig, &pose, [0.0; 3], look_kick())
        .expect("ragdoll");
    fx.step();
    if let Some(name) = sever {
        let bone = u16::try_from(ct7_bone(name)).expect("bone fits");
        fx.sever(0, bone, 1).expect("sever").expect("ragdoll");
    }
    let mut settle = 0;
    for tick in 1..=ticks {
        fx.step();
        let awake = fx.stats().awake;
        if awake == 0 {
            settle = tick;
            break;
        }
        if tick % 40 == 0 {
            println!(
                "settle:   groups sever={sever:?} tick={tick} {:?}",
                fx.rest_state()
            );
        }
        if tick % 40 == 0 {
            let low = at(&fx, "arm_lower_r").pos;
            let mut moving = Vec::new();
            for (key, v, w, _) in fx.awake_state() {
                if w > 0.25 || v > 0.1 {
                    let Key::Bone { bone, .. } = key else {
                        moving.push(format!("{key:?} v={v:.2} w={w:.2}"));
                        continue;
                    };
                    let name = ct7_name(bone);
                    moving.push(format!(
                        "{name} v={v:.3} w={w:.3} c={}",
                        fx.contact_count(key)
                    ));
                }
            }
            moving.truncate(8);
            println!(
                "settle: sever={sever:?} tick={tick} awake={awake} contacts={} arm_lower_r y={:.2} moving=[{}]",
                fx.stats().contacts,
                low[1],
                moving.join(", ")
            );
        }
    }
    let stats = fx.stats();
    println!(
        "settle: sever={sever:?} settle={settle} {} awake={}",
        stats.receipt_line(),
        stats.awake
    );
    let placed = fx.transforms(1.0).expect("transforms");
    for (key, v, w, sleeping) in fx.awake_state() {
        let name = placed
            .iter()
            .find(|one| one.key == key)
            .map(|one| (one.at.pos[1], fx.contact_count(key)));
        println!(
            "settle:   awake {key:?} y={:?} v={v:.3} w={w:.3} sleeping={sleeping}",
            name
        );
    }
    settle
}

#[test]
fn a_knocked_down_ragdoll_falls_asleep_within_three_seconds() {
    let whole = fall(None, 600);
    let left = fall(Some("arm_upper_l"), 600);
    let right = fall(Some("arm_upper_r"), 600);
    let hand = fall(Some("palm_l"), 600);
    println!("settle: whole={whole} left={left} right={right} hand={hand}");
    assert!(whole > 0, "the whole ragdoll never fell asleep");
    assert!(
        left > 0,
        "the ragdoll that lost its left arm never fell asleep"
    );
    assert!(
        right > 0,
        "the ragdoll that lost its right arm never fell asleep"
    );
    assert!(
        hand > 0,
        "the ragdoll that lost its left hand never fell asleep"
    );
    assert!(whole <= 180, "the whole ragdoll slept after {whole} ticks");
    assert!(left <= 180, "the left-arm ragdoll slept after {left} ticks");
    assert!(
        right <= 180,
        "the right-arm ragdoll slept after {right} ticks"
    );
    assert!(
        hand <= 180,
        "the left-hand ragdoll slept after {hand} ticks"
    );
}
