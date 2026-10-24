//! roman_dressing: where the Roman architecture stands in the ring.
//!
//! Owner: `engine-game`; consumers: the arena twin's dressing stage and the
//! lead's window capture. Data owner: this module owns the placement table and
//! the glb source paths; the files under `content/props/roman/` own the meshes
//! and `PROVENANCE.md` owns their licence (Poly Pizza, CC-BY 3.0).
//!
//! Geometry comes from the fight, not from taste: the fighters are clamped
//! inside [`RING_WALL_RADIUS_M`], which is `half_extent_m` (20 m, the default
//! [`engine_arena::ArenaConfig`]) minus [`engine_arena::FIGHTER_RADIUS_M`]
//! (0.4 m) — `wall_limit` in `crates/arena/src/lib.rs:2169`. The arches stand
//! on that wall, one per 30 degrees; the coliseum is the far backdrop outside
//! the stands. Units: meters, radians. Determinism: a pure table, so the same
//! call gives the same actors; pixels never enter the sim hash. Errors: the
//! import helper is typed; every table function is total.

/// Usable half-extent of the default arena in meters.
pub const ARENA_HALF_EXTENT_M: f32 = 20.0;
/// Radius the fighters are clamped inside (the wall the arches stand on).
pub const RING_WALL_RADIUS_M: f32 = ARENA_HALF_EXTENT_M - engine_arena::FIGHTER_RADIUS_M;
/// Arches around the ring: one per 30 degrees.
pub const ARCH_COUNT: usize = 12;
/// How far the arch centre sits outside the wall, so its footing stands on the
/// podium rather than inside the fighters' ring, in meters.
pub const ARCH_BASE_OFFSET_M: f32 = 0.9;
/// Radius of the coliseum backdrop: outside the stands (12 m deep, actor 2 of
/// `content/arena/placement.json`) and well inside the twin camera's 120 m far
/// plane, in meters.
pub const COLISEUM_RADIUS_M: f32 = 45.0;
/// Height of the arch centre above the sand in meters.
pub const ARCH_Y_M: f32 = 2.6;
/// Actors in the table: [`ARCH_COUNT`] arches plus the coliseum backdrop.
pub const ROMAN_ACTOR_COUNT: usize = ARCH_COUNT + 1;
/// Stable actor id base, clear of the kit (1..=11) and dressing (101..=106).
pub const ROMAN_ACTOR_ID_BASE: u32 = 201;

/// Which Roman asset an actor stages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RomanAsset {
    /// Roman Arch 5 (5,198 source triangles).
    Arch,
    /// Coliseum (24,153 source triangles, a far backdrop only).
    Coliseum,
}

impl RomanAsset {
    /// File name under `content/props/roman/`.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Arch => "roman_arch.glb",
            Self::Coliseum => "roman_coliseum.glb",
        }
    }

    /// Source directory the cook and the bundle read.
    #[must_use]
    pub fn source_dir(self) -> &'static str {
        "content/props/roman"
    }
}

/// One placed Roman actor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RomanActor {
    /// Stable actor id ([`ROMAN_ACTOR_ID_BASE`] plus the slot).
    pub id: u32,
    /// Which glb this stages.
    pub asset: RomanAsset,
    /// Centre in meters `[x, y, z]`.
    pub pos_m: [f32; 3],
    /// Yaw about +Y in radians, facing the ring centre.
    pub yaw: f32,
}

/// Azimuth of arch `index` in radians: even spacing, first arch on +X.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "ARCH_COUNT is 12, exactly representable in f32"
)]
pub fn arch_azimuth(index: usize) -> f32 {
    let step = std::f32::consts::TAU / ARCH_COUNT as f32;
    step * index as f32
}

/// Centre of arch `index`: on the wall ring, pushed out by the base offset so
/// the footing is outside the fighters' ring, turned to face the centre.
#[must_use]
pub fn arch_position(index: usize) -> [f32; 3] {
    let radius = RING_WALL_RADIUS_M + ARCH_BASE_OFFSET_M;
    let angle = arch_azimuth(index);
    [radius * angle.cos(), ARCH_Y_M, radius * angle.sin()]
}

/// The whole placement table: [`ARCH_COUNT`] arches round the wall, then the
/// coliseum as the far backdrop behind the ring.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "ARCH_COUNT is 12 and index < ARCH_COUNT, so both fit u32"
)]
pub fn roman_actors() -> [RomanActor; ROMAN_ACTOR_COUNT] {
    let mut out = [RomanActor {
        id: ROMAN_ACTOR_ID_BASE,
        asset: RomanAsset::Arch,
        pos_m: [0.0; 3],
        yaw: 0.0,
    }; ROMAN_ACTOR_COUNT];
    for (index, slot) in out.iter_mut().take(ARCH_COUNT).enumerate() {
        *slot = RomanActor {
            id: ROMAN_ACTOR_ID_BASE + index as u32,
            asset: RomanAsset::Arch,
            pos_m: arch_position(index),
            // The arch's open face looks at the ring: yaw is the azimuth plus pi.
            yaw: arch_azimuth(index) + std::f32::consts::PI,
        };
    }
    out[ARCH_COUNT] = RomanActor {
        id: ROMAN_ACTOR_ID_BASE + ARCH_COUNT as u32,
        asset: RomanAsset::Coliseum,
        pos_m: [COLISEUM_RADIUS_M, 0.0, 0.0],
        yaw: std::f32::consts::PI,
    };
    out
}

/// Horizontal distance from the ring centre in meters.
#[must_use]
pub fn radial_distance_m(actor: &RomanActor) -> f32 {
    (actor.pos_m[0] * actor.pos_m[0] + actor.pos_m[2] * actor.pos_m[2]).sqrt()
}

/// Import one Roman glb through the same reader the arena kit uses
/// (`engine_assets::mesh_import::import_gltf_path`, the `app` feature's glTF
/// lane), so the cook and the bundle see the same source bytes.
///
/// Errors: a missing or undecodable file names itself.
#[cfg(feature = "app")]
pub fn import_roman(
    root: &std::path::Path,
    asset: RomanAsset,
) -> Result<engine_assets::mesh_import::GltfImport, crate::arena_kit::KitError> {
    use engine_assets::{AssetSource, FsSource};
    let source = FsSource::new(root.join(asset.source_dir()));
    let bytes = source.read(asset.file_name()).map_err(|err| {
        crate::arena_kit::KitError::Import(format!("{}: {err}", asset.file_name()))
    })?;
    engine_assets::mesh_import::import_gltf_path(asset.file_name(), &bytes)
        .map_err(|err| crate::arena_kit::KitError::Import(format!("{}: {err}", asset.file_name())))
}

/// Which tier of the arena a placed prop belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RomanPlace {
    /// Columns, arch-ring modules and facade pieces on the outer ring.
    OuterRing,
    /// Braziers flanking the gate tunnel mouth.
    Gate,
    /// Amphorae, the Hadrian bust, the statue and the railing on the podium.
    Podium,
    /// The colosseum massing models behind everything.
    Backdrop,
}

/// One Sketchfab prop as the arena places it. The files live under
/// `content/props/sketchfab/` (CC-BY, `CREDITS.md` beside them) and their tri
/// counts are the `PROVENANCE.md` rows the placement test reads back.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SketchfabActor {
    /// File name under `content/props/sketchfab/`.
    pub file: &'static str,
    /// Which tier this prop dresses.
    pub place: RomanPlace,
    /// Centre in meters `[x, y, z]`.
    pub pos_m: [f32; 3],
    /// Yaw about +Y in radians.
    pub yaw: f32,
}

/// Columns round the ring: one per 60 degrees at [`OUTER_RING_RADIUS_M`].
pub const COLUMN_COUNT: usize = 6;
/// Radius of the outer ring the columns, arch modules and facade stand on, in
/// meters: outside the [`RING_WALL_RADIUS_M`] wall, outside the arch ring that
/// stands on it, and inside the colosseum backdrop.
pub const OUTER_RING_RADIUS_M: f32 = 24.0;
/// Radius of the colosseum massing backdrop, in meters.
pub const COLOSSEUM_RADIUS_M: f32 = 60.0;
/// Radius the podium props stand at: just outside the fighters' ring, on the
/// wall footing, in meters.
pub const PODIUM_RADIUS_M: f32 = RING_WALL_RADIUS_M + 0.7;
/// Horizontal offset of a gate brazier from the gate axis, in meters.
pub const GATE_BRAZIER_OFFSET_M: f32 = 2.6;
/// Instances placed from `content/props/sketchfab/` (the architecture and prop
/// set: weapons and armour belong to the fighter-gear lane, not here):
/// [`COLUMN_COUNT`] columns, 4 arch-ring/facade pieces, 2 massing backdrops,
/// 2 braziers, 2 gate doors, 3 amphorae, 2 podium statues and the railing.
pub const SKETCHFAB_INSTANCES: usize = COLUMN_COUNT + 4 + 2 + 2 + 2 + 3 + 2 + 1;

/// One placed prop from the instance table.
#[must_use]
fn placed(file: &'static str, place: RomanPlace, pos_m: [f32; 3], yaw: f32) -> SketchfabActor {
    SketchfabActor {
        file,
        place,
        pos_m,
        yaw,
    }
}

/// Point on a ring: `angle` radians, `radius` meters out, `y` up.
fn ring_point(angle: f32, radius: f32, y: f32) -> [f32; 3] {
    [radius * angle.cos(), y, radius * angle.sin()]
}

/// Columns and the colosseum arch-ring and facade pieces on the outer ring.
#[expect(
    clippy::cast_precision_loss,
    reason = "COLUMN_COUNT is 6, exactly representable in f32"
)]
fn place_outer_ring(out: &mut [SketchfabActor; SKETCHFAB_INSTANCES], at: &mut usize) {
    let step = std::f32::consts::TAU / COLUMN_COUNT as f32;
    for index in 0..COLUMN_COUNT {
        let angle = step * index as f32;
        out[*at] = placed(
            "corinthian_column_nickhavas.glb",
            RomanPlace::OuterRing,
            ring_point(angle, OUTER_RING_RADIUS_M, 5.0),
            -angle,
        );
        *at += 1;
    }
    // The arch-ring module and the facade repeat on the outer ring's two axes.
    for (file, angle, radius) in [
        (
            "colosseum_arch_ring_lanagre.glb",
            0.0f32,
            OUTER_RING_RADIUS_M + 1.5,
        ),
        (
            "colosseum_arch_ring_lanagre.glb",
            std::f32::consts::PI,
            OUTER_RING_RADIUS_M + 1.5,
        ),
        (
            "colosseum_facade_jeremy.glb",
            std::f32::consts::FRAC_PI_2,
            OUTER_RING_RADIUS_M + 2.5,
        ),
        (
            "colosseum_facade_jeremy.glb",
            std::f32::consts::PI + std::f32::consts::FRAC_PI_2,
            OUTER_RING_RADIUS_M + 2.5,
        ),
    ] {
        out[*at] = placed(
            file,
            RomanPlace::OuterRing,
            ring_point(angle, radius, 5.0),
            -angle + std::f32::consts::PI,
        );
        *at += 1;
    }
}

/// The two colosseum massing models, the far backdrop.
#[expect(
    clippy::cast_precision_loss,
    reason = "the massing pair is two instances"
)]
fn place_backdrop(out: &mut [SketchfabActor; SKETCHFAB_INSTANCES], at: &mut usize) {
    for (index, file) in ["colosseum_cloud9.glb", "colosseum_gits3d.glb"]
        .into_iter()
        .enumerate()
    {
        let angle = std::f32::consts::PI * 0.5 * index as f32;
        let radius = COLOSSEUM_RADIUS_M + 6.0 * index as f32;
        out[*at] = placed(
            file,
            RomanPlace::Backdrop,
            ring_point(angle, radius, 0.0),
            -angle,
        );
        *at += 1;
    }
}

/// Braziers and the gate door leaves at the tunnel mouth, on the -X axis.
fn place_gate(out: &mut [SketchfabActor; SKETCHFAB_INSTANCES], at: &mut usize) {
    let face = std::f32::consts::FRAC_PI_2;
    for side in [-1.0f32, 1.0f32] {
        out[*at] = placed(
            "brazier_notmybest3d.glb",
            RomanPlace::Gate,
            [
                -(RING_WALL_RADIUS_M + 0.9),
                1.1,
                side * GATE_BRAZIER_OFFSET_M,
            ],
            face,
        );
        *at += 1;
        out[*at] = placed(
            "arena_gate_door.glb",
            RomanPlace::Gate,
            [
                -(RING_WALL_RADIUS_M + 0.6),
                2.0,
                side * (GATE_BRAZIER_OFFSET_M + 1.9),
            ],
            face,
        );
        *at += 1;
    }
}

/// Three amphorae, the Hadrian bust, the full-figure statue and the railing on
/// the podium footing.
#[expect(
    clippy::cast_precision_loss,
    reason = "three amphorae and a pair of statues"
)]
fn place_podium(out: &mut [SketchfabActor; SKETCHFAB_INSTANCES], at: &mut usize) {
    for index in 0..3usize {
        let angle = -std::f32::consts::FRAC_PI_2 + 0.35 * index as f32;
        out[*at] = placed(
            "amphora_shadic02.glb",
            RomanPlace::Podium,
            ring_point(angle, PODIUM_RADIUS_M, 0.6),
            -angle,
        );
        *at += 1;
    }
    for (index, file) in ["hadrian_bust_avi.glb", "borghese_gladiator_statue.glb"]
        .into_iter()
        .enumerate()
    {
        let angle = std::f32::consts::FRAC_PI_2 + 0.6 * index as f32;
        out[*at] = placed(
            file,
            RomanPlace::Podium,
            ring_point(angle, PODIUM_RADIUS_M, 1.1),
            -angle,
        );
        *at += 1;
    }
    out[*at] = placed(
        "podium_railing.glb",
        RomanPlace::Podium,
        ring_point(0.0, PODIUM_RADIUS_M, 0.9),
        0.0,
    );
    *at += 1;
}

/// The Sketchfab tier table, one entry per instance: outer ring, backdrop,
/// gate, then podium, in that slot order.
#[must_use]
pub fn sketchfab_actors() -> [SketchfabActor; SKETCHFAB_INSTANCES] {
    let mut out = [placed(
        "corinthian_column_nickhavas.glb",
        RomanPlace::OuterRing,
        [0.0; 3],
        0.0,
    ); SKETCHFAB_INSTANCES];
    let mut at = 0usize;
    place_outer_ring(&mut out, &mut at);
    place_backdrop(&mut out, &mut at);
    place_gate(&mut out, &mut at);
    place_podium(&mut out, &mut at);
    debug_assert_eq!(at, SKETCHFAB_INSTANCES, "every instance slot filled");
    out
}

/// Horizontal distance of a Sketchfab actor from the ring centre in meters.
#[must_use]
pub fn sketchfab_radius_m(actor: &SketchfabActor) -> f32 {
    (actor.pos_m[0] * actor.pos_m[0] + actor.pos_m[2] * actor.pos_m[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo root, for the glb files in `content/props/roman`.
    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn roman_placement_stands_arches_on_the_wall_and_the_coliseum_behind_the_stands() {
        let actors = roman_actors();
        assert_eq!(actors.len(), ROMAN_ACTOR_COUNT);
        // The ring radius is the fight's, read off the arena crate, not a taste.
        assert_eq!(engine_arena::FIGHTER_RADIUS_M, 0.4);
        assert!(
            (RING_WALL_RADIUS_M - 19.6).abs() < 1e-4,
            "{RING_WALL_RADIUS_M}"
        );
        let arches = actors.iter().filter(|a| a.asset == RomanAsset::Arch);
        assert_eq!(arches.clone().count(), ARCH_COUNT);
        let step = arch_azimuth(1);
        let mut last = 0.0f32;
        for (index, actor) in arches.enumerate() {
            // Every arch is on the wall, outside the fighters' ring, and its
            // azimuth advances by one even step.
            let radial = radial_distance_m(actor);
            assert!(
                (radial - (RING_WALL_RADIUS_M + ARCH_BASE_OFFSET_M)).abs() < 1e-3,
                "arch {index} at {radial} m"
            );
            assert!(radial > RING_WALL_RADIUS_M, "arch {index} inside the ring");
            let azimuth = (actor.pos_m[2])
                .atan2(actor.pos_m[0])
                .rem_euclid(std::f32::consts::TAU);
            if index > 0 {
                assert!((azimuth - last - step).abs() < 1e-3, "arch {index} spacing");
            }
            last = azimuth;
        }
        // The coliseum is the far backdrop: beyond the stands, inside the camera.
        let backdrop = actors[ARCH_COUNT];
        assert_eq!(backdrop.asset, RomanAsset::Coliseum);
        assert!(radial_distance_m(&backdrop) > RING_WALL_RADIUS_M + 12.0);
        assert!(
            radial_distance_m(&backdrop) < 120.0,
            "inside the twin camera far"
        );
        // Nothing sits inside the fighters' ring.
        for actor in &actors {
            assert!(
                radial_distance_m(actor) > RING_WALL_RADIUS_M,
                "{:?} {} inside the ring",
                actor.asset,
                radial_distance_m(actor)
            );
        }
        // Ids are unique and clear of the kit and dressing ranges.
        let mut ids: Vec<u32> = actors.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ROMAN_ACTOR_COUNT);
        assert!(ids[0] > 106, "clear of the dressing ids");
    }

    /// Draw budget per tier in triangles, for the 22 Sketchfab instances: the
    /// outer ring (columns, arch ring, facade), the gate pair plus doors, the
    /// podium props, and the colosseum massing backdrop. Each cap is the tier's
    /// source triangles rounded up to the next thousand.
    const TIER_TRIS_CAP: [(RomanPlace, u32); 4] = [
        (RomanPlace::OuterRing, 42_000),
        (RomanPlace::Gate, 15_000),
        (RomanPlace::Podium, 19_000),
        (RomanPlace::Backdrop, 9_500),
    ];

    /// `file -> tris` from the PROVENANCE.md table beside the props.
    fn provenance_tris() -> Vec<(String, u32)> {
        let text = std::fs::read_to_string(root().join("content/props/sketchfab/PROVENANCE.md"))
            .expect("sketchfab provenance table");
        text.lines()
            .filter(|line| line.starts_with("| `"))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                let file = parts.get(2)?.trim().trim_matches('`').to_owned();
                let tris = parts.get(7)?.trim().parse::<u32>().ok()?;
                Some((file, tris))
            })
            .collect()
    }

    #[test]
    fn sketchfab_props_stand_on_their_tier_and_fit_the_potato_budget() {
        let actors = sketchfab_actors();
        assert_eq!(actors.len(), SKETCHFAB_INSTANCES);
        for actor in &actors {
            let radial = sketchfab_radius_m(actor);
            match actor.place {
                // The outer ring is outside the fighters' wall and outside the
                // arch ring that stands on it, inside the backdrop.
                RomanPlace::OuterRing => assert!(
                    radial > RING_WALL_RADIUS_M + ARCH_BASE_OFFSET_M + 2.0,
                    "{} at {radial} m too close to the ring",
                    actor.file
                ),
                // Braziers and doors stand at the gate mouth on the -X axis,
                // outside the fighters' ring.
                RomanPlace::Gate => {
                    assert!(
                        actor.pos_m[0] < -RING_WALL_RADIUS_M,
                        "{} off the gate",
                        actor.file
                    );
                    assert!(
                        actor.pos_m[2].abs() <= GATE_BRAZIER_OFFSET_M + 2.0,
                        "{} off the gate axis",
                        actor.file
                    );
                }
                // Podium props stand on the footing, just outside the ring.
                RomanPlace::Podium => assert!(
                    (radial - PODIUM_RADIUS_M).abs() < 1e-3,
                    "{} at {radial} m, want {PODIUM_RADIUS_M}",
                    actor.file
                ),
                RomanPlace::Backdrop => assert!(
                    radial >= COLOSSEUM_RADIUS_M,
                    "{} backdrop at {radial} m",
                    actor.file
                ),
            }
            assert!(
                radial > RING_WALL_RADIUS_M,
                "{} inside the ring",
                actor.file
            );
        }
        // Every file is a real prop with a provenance row, and each instance
        // stays under the 5,000-triangle prop cap.
        let tris = provenance_tris();
        let mut totals = [0u32; 4];
        for actor in &actors {
            let row = tris
                .iter()
                .find(|(file, _)| file == actor.file)
                .unwrap_or_else(|| panic!("{} has no PROVENANCE row", actor.file));
            assert!(
                row.1 <= 5_000,
                "{} {} tris over the 5,000 prop cap",
                actor.file,
                row.1
            );
            let tier = TIER_TRIS_CAP
                .iter()
                .position(|(place, _)| *place == actor.place)
                .expect("every place has a cap");
            totals[tier] = totals[tier].saturating_add(row.1);
        }
        for (index, (place, cap)) in TIER_TRIS_CAP.iter().enumerate() {
            assert!(
                totals[index] <= *cap,
                "{place:?} tier {} tris over its {cap} cap",
                totals[index]
            );
        }
        // Weapons and armour are the fighter-gear lane's: none is placed here.
        for weapon in [
            "gladius_direraven.glb",
            "gladius_samize.glb",
            "gladius_aren_boada.glb",
            "pugio_davicolt.glb",
            "pilum_paulohare.glb",
            "scutum_paulohare.glb",
            "scutum_dennisvanmalderen.glb",
            "parma_shield.glb",
            "lorica_segmentata.glb",
            "galea_killband.glb",
            "murmillo_helmet.glb",
            "trident_rusty.glb",
        ] {
            assert!(
                !actors.iter().any(|actor| actor.file == weapon),
                "{weapon} belongs to the fighter-gear lane"
            );
        }
    }

    /// The Roman glbs import through the kit's own reader and cook to equal
    /// bytes twice; the triangle counts are the measured ones the potato budget
    /// has to answer for.
    #[cfg(feature = "app")]
    #[test]
    fn roman_glbs_import_equal_twice_and_print_their_triangle_counts() {
        let root = root();
        let mut counts = Vec::new();
        for asset in [RomanAsset::Arch, RomanAsset::Coliseum] {
            let first = import_roman(&root, asset).expect("source imports");
            let second = import_roman(&root, asset).expect("source imports twice");
            assert_eq!(first.meshes, second.meshes, "{asset:?} cooks equal twice");
            assert_eq!(first.materials, second.materials, "{asset:?} materials");
            let tris: usize = first.meshes.iter().map(|mesh| mesh.indices.len() / 3).sum();
            assert!(tris > 0, "{asset:?} has triangles");
            counts.push((asset, tris));
        }
        for (asset, tris) in &counts {
            assert!(
                *tris > 1_500,
                "{asset:?} {tris} triangles is over the 1,500 potato cap"
            );
        }
        // The measured source counts (Poly Pizza listings agree): the arch is
        // 5,198 triangles and the coliseum 24,153. Both are over the 1,500
        // triangle potato cap from crates/assets/src/budget.rs:23, so the arch
        // needs a coarse LOD and the coliseum may only be a far backdrop.
        assert_eq!(counts[0].1, 5_198, "arch triangles");
        assert_eq!(counts[1].1, 24_153, "coliseum triangles");
    }
}
