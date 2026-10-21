//! layers: every collider of this crate carries the group and the mask of the layer table.
//!
//! The table in `engine_physics::layers` owns the bits. This module checks the wiring from
//! the outside: the pairs the table allows are the pairs the world makes, a fighter rests on
//! a crate, and a piece of debris never touches cloth.

use super::*;
use crate::groups;
use engine_physics::layers as table;
use rapier3d::geometry::InteractionTestMode;

/// Every layer of the table, with its bit.
fn every_layer() -> Vec<u32> {
    table::names()
        .map(|word| table::layer(word).expect("a known layer"))
        .collect()
}

/// The layer one group carries, read back from the world.
fn layer_of_body(fx: &PhysFx, key: Key) -> u32 {
    let own = fx.groups_of(key).expect("the body is in the world");
    own.memberships.bits()
}

#[test]
fn the_groups_of_two_layers_agree_with_the_table() {
    // The pair rule of the float world must be the pair rule of the table, or a body
    // would meet what the table says it may not meet.
    for a in every_layer() {
        for b in every_layer() {
            let pair = groups::of(a).test(groups::of(b));
            assert_eq!(
                pair,
                table::collides_layers(a, b),
                "{} vs {}",
                table::name(a).unwrap_or("?"),
                table::name(b).unwrap_or("?")
            );
        }
    }
}

#[test]
fn every_body_carries_the_group_and_mask_of_the_table_layers() {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    fx.add_static_box_on(table::PROP, [2.0, 0.5, 2.0], [0.0, 0.5, 0.0])
        .expect("crate");
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 1.1, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    fx.spawn_limb(7, limb_shape(), at([3.0, 2.0, 0.0]), [0.0; 3])
        .expect("limb");
    fx.spawn_debris(&debris_field(1, 1.0), [0.0; 3])
        .expect("debris");
    fx.spawn_thrown(
        5,
        limb_shape(),
        Material::Wood,
        Flight {
            from: [0.0, 3.0, 0.0],
            to: [1.0, 3.0, 0.0],
            ticks: 10,
        },
    )
    .expect("thrown");

    // The bones of a fighter and the limbs that came off are the body, a shard is debris,
    // a thrown object is a prop.
    assert_eq!(
        layer_of_body(&fx, Key::Bone { slot: 0, bone: 0 }),
        table::BODY
    );
    assert_eq!(layer_of_body(&fx, Key::Limb { part: 7 }), table::BODY);
    assert_eq!(
        layer_of_body(&fx, Key::Shard { batch: 0, index: 0 }),
        table::DEBRIS
    );
    assert_eq!(layer_of_body(&fx, Key::Thrown { id: 5 }), table::PROP);

    // Every collider of the world, the static boxes included, carries one table layer and
    // the mask that layer accepts.
    let mut counts = std::collections::BTreeMap::new();
    for own in fx.collider_groups() {
        let layer = own.memberships.bits();
        assert!(table::is_single(layer), "{layer:#x} is not one layer");
        assert_eq!(
            own.filter.bits(),
            table::default_mask(layer),
            "{} accepts the wrong layers",
            table::name(layer).unwrap_or("?")
        );
        assert_eq!(own.test_mode, InteractionTestMode::And);
        *counts.entry(layer).or_insert(0usize) += 1;
    }
    // The floor is the world, the crate and the thrown object are props, the fourteen
    // bones and the limb are the body, the shard is debris.
    assert_eq!(counts.get(&table::WORLD), Some(&1));
    assert_eq!(counts.get(&table::PROP), Some(&2));
    assert_eq!(counts.get(&table::BODY), Some(&(rig.bones() + 1)));
    assert_eq!(counts.get(&table::DEBRIS), Some(&1));
    assert_eq!(counts.len(), 4, "no layer outside the table");
}

/// A crate whose top is one meter above the floor.
fn crate_world() -> PhysFx {
    let mut fx = arena(Tier::Comfort);
    fx.add_static_box_on(table::PROP, [2.0, 0.5, 2.0], [0.0, 0.5, 0.0])
        .expect("crate");
    fx
}

#[test]
fn a_ragdoll_dropped_on_a_crate_rests_on_it() {
    let (rig, pose) = humanoid();
    let mut fx = crate_world();
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 1.1, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    step_n(&mut fx, 300);
    let bones = bones_of(&fx, 0);
    assert_eq!(bones.len(), rig.bones());
    let lowest = bones
        .iter()
        .map(|bone| bone.at.pos[1])
        .fold(f32::INFINITY, f32::min);
    let pelvis = bones[0].at.pos[1];
    assert!(
        lowest > 0.98,
        "a bone sank into the crate: the lowest is at {lowest}"
    );
    assert!(
        pelvis > 1.0,
        "the fighter is not on the crate: the pelvis is at {pelvis}"
    );
}

#[test]
fn debris_hits_a_body_and_never_meets_a_cloth_probe() {
    // One shard on one probe: the probe's layer is the only thing that differs.
    let probe = |layer: u32| {
        let mut fx = PhysFx::new(Tier::Comfort);
        fx.add_static_box_on(layer, [1.0, 0.05, 1.0], [0.0, 0.0, 0.0])
            .expect("probe");
        fx.spawn_debris(
            &[Shard {
                at: at([0.0, 0.1, 0.0]),
                half: [0.1; 3],
                velocity: [0.0; 3],
            }],
            [0.0; 3],
        )
        .expect("debris");
        step_n(&mut fx, 120);
        fx.contact_count(Key::Shard { batch: 0, index: 0 })
    };
    assert_eq!(
        probe(table::CLOTH),
        0,
        "a piece of debris must never touch cloth"
    );
    assert!(
        probe(table::WORLD) > 0,
        "the same piece of debris rests on a body of the world layer"
    );
    assert!(
        probe(table::PROP) > 0,
        "the same piece of debris rests on a prop"
    );
}
