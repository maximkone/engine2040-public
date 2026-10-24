//! things: props and spears as scene things.
//!
//! The fighters come from the look (`fighter_look_scene`); the rest of the
//! scene is staged elsewhere and read here: the kit actors the match collides
//! with, the ring floor and its wall (`fight_ring`), and the spears the spear
//! scene draws (`spear_window`). Each becomes a [`Thing`] with bounds, a
//! shape for rays and sample points. View only; nothing here reaches the sim.
//! Units: meters. Determinism: fixed row order, no clock.

use engine_arena::spear::{Phase, SpearSlot};

use super::geom::{add, scale, sub, unit, P3};
use super::{Kind, Pixels, Sample, Shape, SpearFacts, SpearState, Thing};
use crate::arena_kit::kit_prop_rows;
use crate::fight_ring::{ring_prop_row, ring_wall_row};
use crate::spear_look::SPEAR_LEN_M;
use crate::spear_window::SpearScene;
use crate::view_truth::STEP_M;

/// Thickness of a spear shaft as a radius, meters.
const SHAFT_RADIUS_M: f32 = 0.03;

/// The eight corners, the six face centers and the center of a box.
#[must_use]
pub fn box_samples(lo: P3, hi: P3) -> Vec<Sample> {
    let mid = scale(add(lo, hi), 0.5);
    let mut at: Vec<P3> = super::geom::corners(lo, hi).to_vec();
    for axis in 0..3 {
        for edge in [lo, hi] {
            let mut face = mid;
            face[axis] = edge[axis];
            at.push(face);
        }
    }
    at.push(mid);
    at.into_iter()
        .map(|at| Sample { at, normal: None })
        .collect()
}

/// One prop row as a thing. A prop no taller than [`STEP_M`] is ground a foot
/// stands on, so it blocks nothing; `solid` false forces that for the ring's
/// floor and its wall, whose boxes enclose the whole pit.
fn prop(index: usize, kind: &str, bounds: [P3; 2], solid: bool, drawn: bool) -> Thing {
    let [lo, hi] = bounds;
    let flat = hi[1] - lo[1] <= STEP_M;
    Thing {
        id: format!("prop_{index}_{kind}"),
        kind: Kind::Prop,
        label: kind.to_owned(),
        bounds: Some((lo, hi)),
        shape: if solid && !flat {
            Shape::Box
        } else {
            Shape::Hollow
        },
        drawn,
        samples: box_samples(lo, hi),
        joints: Vec::new(),
        fighter: None,
        spear: None,
    }
}

/// The staged props: the ring floor and its wall (always drawn, never solid for
/// a ray) and, when `with_kit`, the kit actors the match collides with.
/// `kit_drawn` says whether the frame draws the kit: the skinned look clears
/// the kit draws before it stages its ring, so a window with the look bound
/// collides with props it never draws.
#[must_use]
pub fn prop_things(with_kit: bool, kit_drawn: bool) -> Vec<Thing> {
    let mut out = Vec::new();
    if with_kit {
        for (kind, bounds) in kit_prop_rows() {
            out.push(prop(out.len(), kind, bounds, true, kit_drawn));
        }
    }
    for (kind, bounds) in [ring_prop_row(), ring_wall_row()] {
        out.push(prop(out.len(), kind, bounds, false, true));
    }
    out
}

/// Every spear the spear scene draws as a thing. The state comes from the
/// sim's phase for the slot (flying, tethered, or none), the pose from the
/// picture. A slot idle with a spear still drawn means it fell and lies on the
/// sand.
#[must_use]
pub fn spear_things(scene: &SpearScene, phases: &[SpearSlot], alpha: f32) -> Vec<Thing> {
    scene
        .look()
        .draws(alpha)
        .into_iter()
        .map(|draw| {
            let phase = phases
                .get(usize::from(draw.slot))
                .map_or(Phase::Idle, |s| s.phase);
            let (state, stuck_in) = match phase {
                Phase::Flight(_) => (SpearState::Flying, None),
                Phase::Tether(t) => (SpearState::Stuck, t.victim),
                Phase::Idle => (SpearState::Rest, None),
            };
            let dir = unit(draw.dir).unwrap_or([0.0, 0.0, 1.0]);
            spear_thing(draw.slot, state, draw.tip, dir, stuck_in, None)
        })
        .collect()
}

/// One spear as a thing: a rod from tail to tip.
#[must_use]
pub fn spear_thing(
    owner: u8,
    state: SpearState,
    tip: P3,
    dir: P3,
    stuck_in: Option<u8>,
    held_by: Option<u8>,
) -> Thing {
    let tail = sub(tip, scale(dir, SPEAR_LEN_M));
    let pad = [SHAFT_RADIUS_M; 3];
    let lo: P3 = std::array::from_fn(|k| tip[k].min(tail[k]) - pad[k]);
    let hi: P3 = std::array::from_fn(|k| tip[k].max(tail[k]) + pad[k]);
    let mid = scale(add(tip, tail), 0.5);
    Thing {
        id: format!("spear_{owner}"),
        kind: Kind::Spear,
        label: format!("spear of slot {owner}"),
        bounds: Some((lo, hi)),
        shape: Shape::Rod {
            a: tail,
            b: tip,
            radius: SHAFT_RADIUS_M,
        },
        drawn: true,
        samples: [tip, mid, tail]
            .into_iter()
            .map(|at| Sample { at, normal: None })
            .collect(),
        joints: Vec::new(),
        fighter: None,
        spear: Some(SpearFacts {
            owner,
            state,
            tip,
            dir,
            len_m: SPEAR_LEN_M,
            held_by,
            stuck_in,
        }),
    }
}

/// Tally a draw-id buffer into pixels per owner. `owners[i]` names the thing
/// that draw `i` of the submitted list belongs to; a texel holding the clear
/// word counts as clear, and one naming a draw outside the list counts under
/// `other`. Pure: one pass over the ids.
#[must_use]
pub fn tally_pixels(ids: &[u32], owners: &[String]) -> Pixels {
    let mut out = Pixels {
        total: u64::try_from(ids.len()).unwrap_or(u64::MAX),
        ..Pixels::default()
    };
    for id in ids {
        if *id == engine_render::draw_id::CLEAR {
            out.clear = out.clear.saturating_add(1);
            continue;
        }
        let owner = usize::try_from(*id)
            .ok()
            .and_then(|at| owners.get(at))
            .map_or("other", String::as_str);
        let count = out.by_id.entry(owner.to_owned()).or_insert(0);
        *count = count.saturating_add(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_buffer_tallies_per_owner_clear_and_other() {
        let owners = vec![
            "fighter_0".to_owned(),
            "fighter_1".to_owned(),
            "fighter_0".to_owned(),
        ];
        let clear = engine_render::draw_id::CLEAR;
        let px = tally_pixels(&[0, 0, 1, 2, clear, clear, clear, 9], &owners);
        assert_eq!((px.total, px.clear), (8, 3));
        assert_eq!(
            px.by_id.get("fighter_0"),
            Some(&3),
            "draws 0 and 2 are both fighter_0"
        );
        assert_eq!(px.by_id.get("fighter_1"), Some(&1));
        assert_eq!(px.by_id.get("other"), Some(&1), "a draw outside the list");
        assert_eq!(tally_pixels(&[], &owners), Pixels::default());
    }

    #[test]
    fn the_ring_rows_are_hollow_and_a_kit_row_is_solid_only_when_taller_than_a_step() {
        let ring = prop_things(false, true);
        assert_eq!(ring.len(), 2, "floor and tiers");
        assert!(ring
            .iter()
            .all(|t| matches!(t.shape, Shape::Hollow) && t.drawn));
        assert!(
            ring[0].id.starts_with("prop_0_ring_floor"),
            "{}",
            ring[0].id
        );
        let kit = prop_things(true, false);
        assert!(kit.len() >= ring.len());
        let solid = kit.iter().filter(|t| matches!(t.shape, Shape::Box)).count();
        assert!(kit
            .iter()
            .filter(|t| matches!(t.shape, Shape::Box))
            .all(|t| !t.drawn));
        assert!(
            solid == 0 || kit.len() > ring.len(),
            "solid kit rows come from the kit"
        );
        // Same call, same rows.
        let ids: Vec<&str> = kit.iter().map(|t| t.id.as_str()).collect();
        let again = prop_things(true, false);
        assert_eq!(ids, again.iter().map(|t| t.id.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn a_spear_is_a_rod_from_tail_to_tip_with_a_padded_box() {
        let spear = spear_thing(
            1,
            SpearState::Flying,
            [3.0, 1.0, 2.0],
            [0.0, 0.0, 1.0],
            None,
            None,
        );
        let facts = spear.spear.expect("spear facts");
        assert_eq!(facts.owner, 1);
        let (lo, hi) = spear.bounds.expect("bounds");
        assert!(lo[2] < 0.2 && hi[2] > 1.99, "{lo:?} {hi:?}");
        assert_eq!(spear.samples.len(), 3);
        assert!(matches!(spear.shape, Shape::Rod { .. }));
    }

    #[test]
    fn box_samples_cover_corners_faces_and_the_center() {
        let samples = box_samples([0.0; 3], [2.0; 3]);
        assert_eq!(samples.len(), 15);
        assert!(samples.iter().any(|s| s.at == [1.0, 1.0, 1.0]));
        assert!(samples.iter().any(|s| s.at == [0.0, 1.0, 1.0]));
        assert!(samples.iter().any(|s| s.at == [2.0, 2.0, 2.0]));
    }
}
