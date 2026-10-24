//! arena_lod: the arena's one armed LOD policy — a pixel-error table with a
//! switch back to distance bands.
//!
//! Until now no production [`LodTable`] existed: every table in the tree was a
//! fixture with `pixel_thresholds_px: None`, so the live pick was bands only
//! and the field of view and the frame height could not move a level. This
//! module owns the arena's real table and the pick that reads the live camera.
//!
//! The table is the distance bands the arena already used, projected once to
//! pixels at the reference frame (1080p, the 60 degree duel camera, a 0.5 m
//! fighter). At that frame the armed pick returns exactly the band level, so
//! arming changes no shipped capture; away from it — a 540p frame, a narrow
//! lens — the pick follows the projected size instead of the metres. That is
//! the point of the pixel path: a level is chosen by how many pixels the
//! fighter covers, not by how many metres away it is.
//!
//! Units: metres for distances and radii, pixels for thresholds and the frame
//! height, radians for the field of view, metres for the band hysteresis.
//! Determinism: the mode and the numbers are plain data; the environment is
//! read once per frame by [`ArenaLod::live`] and never inside the pick.
//! Pixels out of a table: none — the pick is pure arithmetic on the camera.
//! Caller: the arena's kit draw builds its list through [`ArenaLod::build`];
//! [`LodMode::Bands`] (or `E2040_LOD=bands`) restores the old pick for
//! comparison without touching a call site.

use std::fmt;

use crate::draw::{
    lod_by_pixel_error, Camera, Draw, DrawError, DrawList, LodTable, SceneSnapshot,
    DEFAULT_SCREEN_H,
};

/// Distance bands the arena picks on when the pixel table is off. These are
/// the bands the duel table shipped with: near duel, mid, far crowd.
pub const ARENA_BANDS_M: [f32; 3] = [10.0, 20.0, 40.0];

/// Half-width of the hysteresis band around each boundary, in metres; the
/// same hold the band pick has always had.
pub const ARENA_HYSTERESIS_M: f32 = 0.5;

/// Radius of one fighter, in metres: the storm fixture's fighter node and the
/// kit's body bounds both come out at half a metre.
pub const BODY_RADIUS_M: f32 = 0.5;

/// The duel camera's full vertical field of view: `FRAC_PI_3`, the default
/// `crates/game/src/camera.rs` gives a live duel camera.
pub const ARENA_FOV_Y_RAD: f32 = std::f32::consts::FRAC_PI_3;

/// The frame height the reference projection uses: the arena's 1080p default,
/// which is also what [`crate::draw::DrawList::build`] assumes.
pub const ARENA_SCREEN_H: u32 = DEFAULT_SCREEN_H;

/// The arena's projected-size boundaries in pixels, ascending, coarsest
/// first. Each is one of [`ARENA_BANDS_M`] projected at the reference frame
/// (1080p, [`ARENA_FOV_Y_RAD`], [`BODY_RADIUS_M`]): 40 m -> 23.38 px,
/// 20 m -> 46.77 px, 10 m -> 93.53 px, by
/// `radius * screen_h / (distance * tan(fov / 2))`.
pub const ARENA_THRESHOLDS_PX: [f32; 3] = [23.38, 46.77, 93.53];

/// Environment variable that switches the pick: `bands` restores the distance
/// bands, anything else (or unset) keeps the pixel table armed.
pub const LOD_MODE_ENV: &str = "E2040_LOD";

/// Which LOD pick the frame uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodMode {
    /// Project the fighter to pixels and pick by that size (the arena's one
    /// armed path).
    Pixels,
    /// The distance bands, unchanged since before the pixel path.
    Bands,
}

impl LodMode {
    /// Parse the environment value: `bands` (any case, surrounding spaces
    /// allowed) selects [`LodMode::Bands`]; anything else keeps the armed
    /// path, so a typo can never silently turn the feature off.
    #[must_use]
    pub fn from_value(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("bands") {
            Self::Bands
        } else {
            Self::Pixels
        }
    }

    /// The environment's mode, read once per frame by [`ArenaLod::live`].
    /// An unset or non-Unicode value keeps the armed path.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var(LOD_MODE_ENV) {
            Ok(value) => Self::from_value(&value),
            Err(_) => Self::Pixels,
        }
    }

    /// True when the distance bands decide.
    #[must_use]
    pub fn is_bands(self) -> bool {
        self == Self::Bands
    }
}

impl fmt::Display for LodMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pixels => f.write_str("pixels"),
            Self::Bands => f.write_str("bands"),
        }
    }
}

/// The arena's LOD table: armed with [`ARENA_THRESHOLDS_PX`], or the same
/// bands with the pixel path off.
#[must_use]
pub fn arena_table(mode: LodMode) -> LodTable {
    LodTable {
        id: ARENA_TABLE_ID,
        bands_m: ARENA_BANDS_M.to_vec(),
        hysteresis_m: ARENA_HYSTERESIS_M,
        pixel_thresholds_px: match mode {
            LodMode::Pixels => Some(ARENA_THRESHOLDS_PX.to_vec()),
            LodMode::Bands => None,
        },
    }
}

/// Identity of the arena's table; every kit node names it.
pub const ARENA_TABLE_ID: u64 = 0x51A9_0001;

/// The armed table and the live camera it picks through.
#[derive(Clone, Debug)]
pub struct ArenaLod {
    table: LodTable,
    mode: LodMode,
    screen_h: u32,
    tan_half_fov_y: f32,
}

impl ArenaLod {
    /// The policy for one frame: `mode` chooses the table, `screen_h` and the
    /// camera's vertical field of view are what the pixel pick projects
    /// through.
    #[must_use]
    pub fn new(camera: &Camera, screen_h: u32, mode: LodMode) -> Self {
        Self {
            table: arena_table(mode),
            mode,
            screen_h,
            tan_half_fov_y: (camera.fov_y_rad * 0.5).tan(),
        }
    }

    /// The policy the live frame uses: mode from [`LOD_MODE_ENV`], camera and
    /// frame height from the frame being drawn.
    #[must_use]
    pub fn live(camera: &Camera, screen_h: u32) -> Self {
        Self::new(camera, screen_h, LodMode::from_env())
    }

    /// Which pick this frame uses.
    #[must_use]
    pub fn mode(&self) -> LodMode {
        self.mode
    }

    /// The table handed to the draw list.
    #[must_use]
    pub fn table(&self) -> &LodTable {
        &self.table
    }

    /// Frame height the pick projects through.
    #[must_use]
    pub fn screen_h(&self) -> u32 {
        self.screen_h
    }

    /// The level one fighter takes at `distance_m`. Armed: the projected size
    /// against [`ARENA_THRESHOLDS_PX`], with `prev` held inside the same
    /// relative hysteresis the bands use. Bands: [`crate::draw::pick_lod`].
    ///
    /// Errors: [`DrawError::BadPixelLod`] from the armed pick on a degenerate
    /// input (zero distance, zero levels); the band path cannot fail.
    pub fn pick(
        &self,
        radius_m: f32,
        distance_m: f32,
        levels: u8,
        prev: u8,
    ) -> Result<u8, DrawError> {
        if self.mode.is_bands() {
            return Ok(crate::draw::pick_lod(
                distance_m,
                prev,
                levels,
                &self.table.bands_m,
                self.table.hysteresis_m,
            ));
        }
        // The metre band at the finest boundary, in pixels: the same relative
        // hold the band path gives, so arming does not start popping.
        let hyst_px = crate::draw::pixel_hysteresis_px(
            radius_m,
            self.screen_h,
            self.tan_half_fov_y,
            &ARENA_THRESHOLDS_PX,
            self.table.hysteresis_m,
        );
        lod_by_pixel_error(
            radius_m,
            distance_m.max(crate::draw::MIN_NEAR_M),
            self.screen_h,
            self.tan_half_fov_y,
            levels,
            prev,
            &ARENA_THRESHOLDS_PX,
            hyst_px,
        )
    }

    /// Build the arena's draw list for this frame: [`DrawList::build_at`] with
    /// this policy's frame height and the policy's table.
    pub fn build<'a>(
        &self,
        list: &'a mut DrawList,
        snapshot: &SceneSnapshot,
        camera: &Camera,
    ) -> Result<&'a [Draw], DrawError> {
        list.build_at(snapshot, camera, &self.table, self.screen_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::SceneNode;

    fn camera(fov_y_rad: f32) -> Camera {
        Camera {
            pos_m: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y_rad,
            aspect: 16.0 / 9.0,
            near_m: 0.1,
            far_m: 200.0,
        }
    }

    fn fighter(id: u64, distance_m: f32) -> SceneNode {
        SceneNode {
            id,
            mesh: id + 1000,
            center_m: [0.0, 0.0, -distance_m],
            radius_m: BODY_RADIUS_M,
            lod_levels: 4,
            lod_table: ARENA_TABLE_ID,
            prev_lod: 0,
        }
    }

    /// The projected size of a fighter at `distance_m`, in pixels, at the
    /// reference frame: `radius * screen_h / (distance * tan(fov / 2))`.
    fn projected_px(distance_m: f32, screen_h: u32) -> f32 {
        BODY_RADIUS_M * f32::from(u16::try_from(screen_h).expect("height fits u16"))
            / (distance_m * (ARENA_FOV_Y_RAD * 0.5).tan())
    }

    #[test]
    fn arena_lod_thresholds_ascend_and_project_the_shipped_bands() {
        assert!(
            ARENA_THRESHOLDS_PX.windows(2).all(|w| w[0] < w[1]),
            "the pick walks the boundaries, so they must ascend"
        );
        assert!(ARENA_THRESHOLDS_PX.iter().all(|t| *t > 0.0));
        // Each boundary is one shipped band projected at the reference frame;
        // the bands themselves stay coarsest-first, so the pair is reversed.
        let projected: Vec<f32> = ARENA_BANDS_M
            .iter()
            .rev()
            .map(|band| projected_px(*band, ARENA_SCREEN_H))
            .collect();
        for (got, want) in ARENA_THRESHOLDS_PX.iter().zip(&projected) {
            assert!(
                (got - want).abs() < 0.01,
                "threshold {got} px must be the projection of its band ({want} px)"
            );
        }
        assert_eq!(ARENA_SCREEN_H, 1080);
        assert!((ARENA_FOV_Y_RAD.to_degrees() - 60.0).abs() < 1e-4);
        assert_eq!(ARENA_TABLE_ID, arena_table(LodMode::Pixels).id);
    }

    #[test]
    fn arena_lod_fighter_at_60m_picks_coarser_than_at_5m() {
        let frame = ArenaLod::new(&camera(ARENA_FOV_Y_RAD), ARENA_SCREEN_H, LodMode::Pixels);
        let near = frame
            .pick(BODY_RADIUS_M, 5.0, 4, 0)
            .expect("near fighter picks");
        let far = frame
            .pick(BODY_RADIUS_M, 60.0, 4, 0)
            .expect("far fighter picks");
        assert_eq!(near, 0, "187 px at 5 m is the hero level");
        assert_eq!(far, 3, "15.6 px at 60 m is the coarsest level");
        assert!(far > near, "a far fighter is never the finer level");
        // The same fighter through the band switch keeps the band level.
        let bands = ArenaLod::new(&camera(ARENA_FOV_Y_RAD), ARENA_SCREEN_H, LodMode::Bands);
        assert_eq!(bands.pick(BODY_RADIUS_M, 5.0, 4, 0).expect("5 m"), 0);
        assert_eq!(bands.pick(BODY_RADIUS_M, 60.0, 4, 0).expect("60 m"), 3);
    }

    #[test]
    fn arena_lod_540p_never_picks_finer_than_1080p() {
        let hd = ArenaLod::new(&camera(ARENA_FOV_Y_RAD), 1080, LodMode::Pixels);
        let half = ArenaLod::new(&camera(ARENA_FOV_Y_RAD), 540, LodMode::Pixels);
        let mut coarser_somewhere = false;
        for step in 0..120u32 {
            let distance_m = 2.0 + f32::from(u16::try_from(step).expect("step fits u16")) * 0.5;
            for prev in 0..4u8 {
                let fine = hd
                    .pick(BODY_RADIUS_M, distance_m, 4, prev)
                    .expect("1080p picks");
                let coarse = half
                    .pick(BODY_RADIUS_M, distance_m, 4, prev)
                    .expect("540p picks");
                assert!(
                    coarse >= fine,
                    "540p took level {coarse} finer than 1080p's {fine} at {distance_m} m"
                );
                coarser_somewhere |= coarse > fine;
            }
        }
        assert!(
            coarser_somewhere,
            "the frame height must move at least one pick"
        );
    }

    #[test]
    fn arena_lod_armed_table_reproduces_the_bands_at_the_reference_frame() {
        // At 1080p and the duel lens the armed table returns today's digits
        // wherever no fighter sits inside a hysteresis hold, so arming changes
        // no shipped capture; the hold itself is the one designed difference
        // (it is measured in pixels, so it is a little wider than the metre
        // band) and it only ever holds a level, never switches sooner.
        let distances = [3.0f32, 8.0, 15.0, 25.0, 35.0, 40.0];
        let snapshot = SceneSnapshot {
            nodes: distances
                .iter()
                .enumerate()
                .map(|(i, d)| fighter(u64::try_from(i).expect("small index"), *d))
                .collect(),
        };
        let duel = camera(ARENA_FOV_Y_RAD);
        let mut armed = DrawList::new();
        let armed_frame = ArenaLod::new(&duel, 1080, LodMode::Pixels);
        armed_frame
            .build(&mut armed, &snapshot, &duel)
            .expect("armed build");
        let mut today = DrawList::new();
        let today_frame = ArenaLod::new(&duel, 1080, LodMode::Bands);
        today_frame
            .build(&mut today, &snapshot, &duel)
            .expect("band build");
        assert_eq!(
            armed.order_digest(),
            today.order_digest(),
            "the armed table reproduces today's picks at the reference frame"
        );
        // The plain `build` against the band switch is the same bytes too.
        let mut plain = DrawList::new();
        plain
            .build(&snapshot, &duel, &arena_table(LodMode::Bands))
            .expect("plain build");
        assert_eq!(plain.order_digest(), today.order_digest());

        // The pixel hold is measured in pixels, so just past a band boundary
        // it keeps the finer level a little longer: the picks differ inside
        // the hold and nowhere else. 45 m is past the 40 m band and inside it.
        let armed_45 = armed_frame
            .pick(BODY_RADIUS_M, 45.0, 4, 0)
            .expect("armed");
        let band_45 = today_frame
            .pick(BODY_RADIUS_M, 45.0, 4, 0)
            .expect("band");
        assert_eq!((armed_45, band_45), (2, 3), "the hold is in pixels now");
        assert_ne!(armed_45, band_45, "the armed pick is live, not a copy");

        // A 540p frame must not reproduce them, or the height is not wired.
        let mut short = DrawList::new();
        let short_frame = ArenaLod::new(&duel, 540, LodMode::Pixels);
        short_frame
            .build(&mut short, &snapshot, &duel)
            .expect("540p build");
        assert_ne!(
            short.order_digest(),
            today.order_digest(),
            "the frame height reaches the LOD pick"
        );
    }

    #[test]
    fn arena_lod_mode_reads_the_switch_and_defaults_to_armed() {
        assert_eq!(LodMode::from_value("bands"), LodMode::Bands);
        assert_eq!(LodMode::from_value("  BANDS "), LodMode::Bands);
        assert_eq!(LodMode::from_value("pixels"), LodMode::Pixels);
        assert_eq!(
            LodMode::from_value(""),
            LodMode::Pixels,
            "a typo stays armed"
        );
        assert_eq!(LodMode::from_value("band"), LodMode::Pixels);
        assert!(LodMode::Bands.is_bands());
        assert!(!LodMode::Pixels.is_bands());
        assert_eq!(LodMode::Pixels.to_string(), "pixels");
        assert_eq!(arena_table(LodMode::Bands).pixel_thresholds_px, None);
        assert_eq!(
            arena_table(LodMode::Pixels).pixel_thresholds_px,
            Some(ARENA_THRESHOLDS_PX.to_vec())
        );
    }
}
