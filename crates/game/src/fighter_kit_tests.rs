//! Proofs for `fighter_kit` against the real hero bytes: the suit comes off, the kit is sound,
//! and no skin, arm, hip or scalp reaches a plate, a lame, the tunic or the galea.

use super::*;
use engine_assets::body_import::import_skinned_body;
use std::path::PathBuf;

fn body(variant: &str) -> SkinnedBody {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../content/fighters/glb")
        .join(format!("{variant}_body.glb"));
    import_skinned_body(&std::fs::read(path).expect("body file")).expect("body imports")
}

fn tris(body: &SkinnedBody, tile: Tile) -> usize {
    tile_triangles(body, tile)
}

fn part<'a>(kit: &'a [KitPart], name: &str) -> &'a KitPart {
    kit.iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no {name} part"))
}

/// Angle of a point around the vertical axis through `(0, cz)`, 0 = +Z toward +X.
fn angle(p: V3, cz: f32) -> f32 {
    p[0].atan2(p[2] - cz)
}

fn radius(p: V3, cz: f32) -> f32 {
    p[0].hypot(p[2] - cz)
}

/// The smallest radius any of `kit` has near `body_point` (within `band` in height and `arc`
/// radians around the axis), or `None` when the kit has no point there.
fn kit_radius_near(kit: &[V3], body_point: V3, cz: f32, band: f32, arc: f32) -> Option<f32> {
    let a = angle(body_point, cz);
    kit.iter()
        .filter(|k| {
            (k[1] - body_point[1]).abs() <= band && wrap_pi(angle(**k, cz) - a).abs() <= arc
        })
        .map(|k| radius(*k, cz))
        .min_by(f32::total_cmp)
}

/// The radius of the kit's surface at the height of `body_point`, around `arc` radians of it:
/// the nearest kit point below and above it (the rings that straddle it), blended by height.
/// `None` when the kit has nothing above or below there.
fn kit_radius_at(kit: &[V3], body_point: V3, cz: f32, arc: f32) -> Option<f32> {
    let a = angle(body_point, cz);
    let near: Vec<V3> = kit
        .iter()
        .copied()
        .filter(|k| wrap_pi(angle(*k, cz) - a).abs() <= arc)
        .collect();
    let below = near
        .iter()
        .filter(|k| k[1] <= body_point[1])
        .max_by(|x, y| x[1].total_cmp(&y[1]))?;
    let above = near
        .iter()
        .filter(|k| k[1] >= body_point[1])
        .min_by(|x, y| x[1].total_cmp(&y[1]))?;
    let same = |h: f32| {
        near.iter()
            .filter(|k| (k[1] - h).abs() < 1.0e-4)
            .map(|k| radius(*k, cz))
            .min_by(f32::total_cmp)
    };
    let (rb, ra) = (same(below[1])?, same(above[1])?);
    let span = above[1] - below[1];
    let t = if span > 1.0e-6 {
        (body_point[1] - below[1]) / span
    } else {
        0.0
    };
    Some(rb + (ra - rb) * t)
}

#[test]
fn the_hero_atlas_tiles_hold_the_pinned_triangle_counts() {
    let hero = body("hero");
    for (tile, want) in [
        (Tile::Armor, 1448),
        (Tile::Body, 884),
        (Tile::Head, 488),
        (Tile::Hair, 120),
        (Tile::Hands, 740),
        (Tile::Gloves, 804),
        (Tile::Helmet, 408),
        (Tile::Other, 222),
        (Tile::Feet, 404),
        (Tile::Boots, 332),
    ] {
        assert_eq!(tris(&hero, tile), want, "{tile:?} triangles");
    }
}

#[test]
fn the_plate_suit_comes_off_and_the_roman_kit_goes_on() {
    let mut hero = body("hero");
    let before: usize = hero.parts.iter().map(|p| p.mesh.indices.len()).sum();
    let kit = dress_hero(&mut hero, HERO_VARIANT);
    let names: Vec<_> = kit.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        ["lorica", "tunic", "galea", "plume", "greaves", "caligae"]
    );
    assert_eq!(names.len(), KIT_PARTS);
    for tile in STRIPPED {
        assert_eq!(tris(&hero, tile), 0, "{tile:?} left the body");
    }
    for tile in [
        Tile::Body,
        Tile::Head,
        Tile::Hair,
        Tile::Hands,
        Tile::Feet,
        Tile::Other,
    ] {
        assert!(tris(&hero, tile) > 0, "{tile:?} stays");
    }
    let after: usize = hero.parts.iter().map(|p| p.mesh.indices.len()).sum();
    assert!(after < before, "the suit's triangles are gone");
    let kit_tris: usize = kit.iter().map(|p| p.indices.len() / 3).sum();
    assert!(
        (2500..=5200).contains(&kit_tris),
        "the kit is about as heavy as the suit it replaces: {kit_tris} triangles"
    );
    let joints = hero.skeleton.joints.len();
    for p in &kit {
        let n = p.positions.len();
        assert!(n > 0 && !p.indices.is_empty(), "{} has geometry", p.name);
        assert_eq!(p.normals.len(), n);
        assert_eq!(p.uv0.len(), n);
        assert_eq!(p.joints.len(), n);
        assert_eq!(p.weights.len(), n);
        assert!(p.indices.len() % 3 == 0, "{}", p.name);
        assert!(
            p.indices
                .iter()
                .all(|i| usize::try_from(*i).is_ok_and(|i| i < n)),
            "{}",
            p.name
        );
        for tri in p.indices.chunks_exact(3) {
            let at = |i: u32| p.positions[usize::try_from(i).unwrap_or(0)];
            let area = cross(sub(at(tri[1]), at(tri[0])), sub(at(tri[2]), at(tri[0])));
            assert!(dot(area, area) > 0.0, "{} has a sliver", p.name);
        }
        assert!(
            p.positions.iter().flatten().all(|c| c.is_finite()),
            "{}",
            p.name
        );
        for nrm in &p.normals {
            assert!(
                (dot(*nrm, *nrm).sqrt() - 1.0).abs() < 1.0e-3,
                "{} normal {nrm:?}",
                p.name
            );
        }
        assert!(
            p.uv0.iter().flatten().all(|c| (0.0..=1.0).contains(c)),
            "{}",
            p.name
        );
        for (j, w) in p.joints.iter().zip(&p.weights) {
            assert!(
                (w.iter().sum::<f32>() - 1.0).abs() < 1.0e-4,
                "{} weights",
                p.name
            );
            assert!(
                j.iter()
                    .zip(w)
                    .all(|(j, w)| *w == 0.0 || usize::from(*j) < joints),
                "{} joint",
                p.name
            );
        }
        assert_eq!(
            usize::try_from(p.texels.width * p.texels.height * 4).ok(),
            Some(p.texels.rgba.len()),
            "{} texels",
            p.name
        );
        assert!((0.0..=1.0).contains(&p.metallic) && (0.0..=1.0).contains(&p.roughness));
    }
}

#[test]
fn no_skin_shows_through_the_lorica_or_the_lames() {
    let mut hero = body("hero");
    let fit = Fit::of(&hero).expect("the hero has the joints");
    let kit = dress_hero(&mut hero, HERO_VARIANT);
    let lorica = part(&kit, "lorica");
    let girdle = &lorica.positions[..GIRDLE_VERTS];
    let y_waist = fit.at(fit.joints.pelvis)[1] + WAIST_ABOVE_PELVIS_M;
    let y_collar = fit.at(fit.joints.shoulder[0])[1] + COLLAR_ABOVE_SHOULDER_M;
    let mut checked = 0;
    for p in fit.trunk_points() {
        if p[1] < y_waist + 0.02 || p[1] > y_collar - 0.02 {
            continue;
        }
        let near = kit_radius_near(girdle, p, 0.0, 0.035, 0.15)
            .unwrap_or_else(|| panic!("no plate near the trunk point {p:?}"));
        assert!(
            near >= radius(p, 0.0) + 0.008,
            "skin at {p:?} reaches {} of a plate at {near}",
            radius(p, 0.0)
        );
        checked += 1;
    }
    assert!(checked > 100, "the test looked at the trunk: {checked}");
    // The lames: every arm point under a lame, in the arm's own frame.
    let j = fit.joints;
    let lames = &lorica.positions[GIRDLE_VERTS..];
    let mut under = 0;
    for side in 0..2 {
        let frame = ArmFrame::new(&fit, side);
        let arm = fit.body_points(|v| v.dom == j.shoulder[side] || v.dom == j.arm_upper[side]);
        let kit_coords: Vec<(f32, f32, f32)> = lames.iter().map(|p| frame.coords(*p)).collect();
        for p in arm {
            let (t, r, theta) = frame.coords(p);
            if !(-0.03..=0.11).contains(&t) || theta.abs() > 1.5 {
                continue;
            }
            let near = kit_coords
                .iter()
                .filter(|(kt, _, kth)| (kt - t).abs() <= 0.04 && (kth - theta).abs() <= 0.2)
                .map(|(_, kr, _)| *kr)
                .min_by(f32::total_cmp)
                .unwrap_or_else(|| panic!("no lame over the arm point {p:?}"));
            assert!(
                near >= r + 0.008,
                "arm at {p:?} (r {r}) reaches a lame at {near}"
            );
            under += 1;
        }
    }
    assert!(under > 30, "the test looked at the arms: {under}");
}

#[test]
fn no_skin_or_scalp_shows_through_the_tunic_or_the_galea() {
    let mut hero = body("hero");
    let fit = Fit::of(&hero).expect("the hero has the joints");
    let kit = dress_hero(&mut hero, HERO_VARIANT);
    let tunic = part(&kit, "tunic");
    let top = fit.at(fit.joints.pelvis)[1] + 0.10;
    let mut hips = 0;
    for p in fit.hip_points() {
        if p[1] < TUNIC_HEM_Y_M + 0.02 || p[1] > top - 0.02 {
            continue;
        }
        let near = kit_radius_near(&tunic.positions, p, 0.0, 0.05, 0.2)
            .unwrap_or_else(|| panic!("no tunic near the hip point {p:?}"));
        assert!(
            near >= radius(p, 0.0) + 0.010,
            "skin at {p:?} reaches {} of a tunic at {near}",
            radius(p, 0.0)
        );
        hips += 1;
    }
    assert!(hips > 40, "the test looked at the hips: {hips}");
    // The galea covers everything of the head above its brow line.
    let galea = part(&kit, "galea");
    let mut scalp = 0;
    for v in fit
        .verts
        .iter()
        .filter(|v| matches!(v.tile, Tile::Head | Tile::Hair))
    {
        if v.p[1] < 1.74 {
            continue;
        }
        let near = kit_radius_at(&galea.positions, v.p, HEAD_CZ_M, 0.2)
            .unwrap_or_else(|| panic!("no galea over the head point {:?}", v.p));
        assert!(
            near >= radius(v.p, HEAD_CZ_M) + 0.006,
            "head at {:?} reaches a galea at {near}",
            v.p
        );
        scalp += 1;
    }
    assert!(scalp > 20, "the test looked at the scalp: {scalp}");
}

#[test]
fn the_helm_has_a_back_and_the_crest_stands_on_the_dome() {
    let mut hero = body("hero");
    let kit = dress_hero(&mut hero, HERO_VARIANT);
    let galea = part(&kit, "galea");
    // A neck guard: kit points behind the head that hang below the nape rim.
    let guard = galea
        .positions
        .iter()
        .filter(|p| p[2] < HEAD_CZ_M - 0.05 && p[1] < rim_y(PI) - 0.04)
        .count();
    assert!(
        guard > 20,
        "the galea has a neck guard behind the head: {guard} points"
    );
    let cheeks = galea
        .positions
        .iter()
        .filter(|p| p[0].abs() > 0.07 && p[1] < 1.66)
        .count();
    assert!(cheeks > 20, "and cheek plates at the sides: {cheeks}");
    // The face stays open: nothing of the galea in front of the head below the brow.
    let in_face = galea
        .positions
        .iter()
        .filter(|p| p[2] > HEAD_CZ_M + 0.05 && p[0].abs() < 0.05 && p[1] < 1.715)
        .count();
    assert_eq!(in_face, 0, "the galea leaves the face open");
    let plume = part(&kit, "plume");
    let top = plume
        .positions
        .iter()
        .map(|p| p[1])
        .fold(f32::MIN, f32::max);
    assert!(
        top > DOME_TOP_Y_M + 0.03,
        "the crest stands above the dome: {top}"
    );
    assert!(plume
        .positions
        .iter()
        .all(|p| p[0].abs() <= CREST_HALF_THICKNESS_M + 1.0e-4));
}

#[test]
fn the_plates_face_outward_and_ride_the_trunk() {
    let mut hero = body("hero");
    let fit = Fit::of(&hero).expect("the hero has the joints");
    let kit = dress_hero(&mut hero, HERO_VARIANT);
    let lorica = part(&kit, "lorica");
    let outward = lorica.positions[..GIRDLE_VERTS]
        .iter()
        .zip(&lorica.normals)
        .filter(|(p, n)| dot(**n, norm([p[0], 0.0, p[2]])) > 0.0)
        .count();
    assert!(
        outward * 100 >= GIRDLE_VERTS * 95,
        "the girdle faces outward: {outward} of {GIRDLE_VERTS}"
    );
    let j = fit.joints;
    for (jr, w) in lorica.joints[..GIRDLE_VERTS].iter().zip(&lorica.weights) {
        for (joint, weight) in jr.iter().zip(w) {
            let joint = usize::from(*joint);
            assert!(
                *weight == 0.0 || joint == j.torso || joint == j.pelvis,
                "girdle joint {joint}"
            );
        }
    }
    for name in ["galea", "plume"] {
        let head = part(&kit, name);
        assert!(
            head.joints.iter().all(|r| usize::from(r[0]) == j.head)
                && head.weights.iter().all(|w| w[0] == 1.0),
            "{name} rides the head alone"
        );
    }
}

#[test]
fn an_opponent_is_not_dressed_and_the_same_hero_dresses_the_same() {
    let mut opponent = body("opp_male_light");
    let untouched = opponent.clone();
    assert!(dress_hero(&mut opponent, "opp_male_light").is_empty());
    assert_eq!(opponent, untouched, "an opponent keeps its body");
    let mut also = body("opp_male_light");
    assert!(
        dress_hero(&mut also, HERO_VARIANT).is_empty(),
        "a body that is not the hero's is left alone even under the hero's name"
    );
    let (mut a, mut b) = (body("hero"), body("hero"));
    assert_eq!(
        dress_hero(&mut a, HERO_VARIANT),
        dress_hero(&mut b, HERO_VARIANT)
    );
    assert_eq!(a, b);
}
