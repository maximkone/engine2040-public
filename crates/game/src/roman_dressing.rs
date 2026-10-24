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
