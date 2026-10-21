//! The browser shell: the KAM6 village match behind a fixed 10 Hz clock.
//!
//! One owner for the tick rule: [`Shell::advance_ms`] collects the page's time
//! deltas and plays whole [`TICK_MS`] ticks, never a partial one. The sim only
//! ever sees `advance(1)`, the same call the native window makes, so a paused or
//! a slow page changes how often it draws and nothing about the match result.
//! The camera is the fixed 2.5D camera of the window, panned by the arrow keys
//! within [`PAN_LIMIT`] world units; pan is presentation and never in the hash.

use engine_render::kam2_iso::IsoCamera;
use kam2_game::house::HouseManifest;
use kam2_game::scene::{build_with, Scene, Units};
use kam2_game::{stock_line, AiMatch, SoldierManifest};
use kam2_sim::{Command, HouseKind, Refusal, Ware};

/// One sim tick, in milliseconds: the style lock's 10 Hz shell tick.
pub const TICK_MS: u32 = 100;
/// Framebuffer width of the 2.5D scene, in pixels.
pub const VIEW_W: u32 = 1280;
/// Framebuffer height of the 2.5D scene, in pixels.
pub const VIEW_H: u32 = 720;
/// Furthest the view may leave the field center, in world units.
pub const PAN_LIMIT: f32 = 6.0;
/// Ticks one [`Shell::advance_ms`] call may play: a long stall drops the
/// backlog instead of bursting, so a tab that was hidden never fast-forwards.
pub const MAX_CATCHUP: u32 = 4;
/// The side the player sits in; the AI mayor holds the other seat.
pub const PLAYER_SIDE: u8 = 0;
/// The committed soldier manifest, parsed once per shell.
const SOLDIER_MANIFEST_JSON: &str = include_str!("../../../content/kam2/soldier/manifest.json");
/// The committed cottage manifest, parsed once per shell.
const HOUSE_MANIFEST_JSON: &str = include_str!("../../../content/kam2/house/manifest.json");

/// The match, the camera and the time the page has handed in so far.
pub struct Shell {
    game: AiMatch,
    units: Units,
    houses: HouseManifest,
    camera: IsoCamera,
    pan: [f32; 2],
    acc_ms: f64,
    paused: bool,
}

impl Shell {
    /// The start village of the window with the AI mayor in the side-1 seat.
    ///
    /// # Errors
    ///
    /// Returns the first [`Refusal`] when the fixed start village no longer fits.
    pub fn new() -> Result<Self, Refusal> {
        let units = Units::Soldier(
            SoldierManifest::parse(SOLDIER_MANIFEST_JSON)
                .expect("the committed soldier manifest parses"),
        );
        let houses = HouseManifest::parse(HOUSE_MANIFEST_JSON)
            .expect("the committed cottage manifest parses");
        Ok(Self {
            game: AiMatch::village()?,
            units,
            houses,
            camera: IsoCamera::new(VIEW_W, VIEW_H),
            pan: [0.0, 0.0],
            acc_ms: 0.0,
            paused: false,
        })
    }

    /// The live match, for the HUD and the tests.
    #[must_use]
    pub const fn game(&self) -> &AiMatch {
        &self.game
    }

    /// Ticks played so far.
    #[must_use]
    pub fn ticks(&self) -> u32 {
        self.game.sim().ticks()
    }

    /// True while the match is held.
    #[must_use]
    pub const fn paused(&self) -> bool {
        self.paused
    }

    /// Hold or release the match. Releasing starts a fresh tick, never a burst.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.acc_ms = 0.0;
        }
    }

    /// Flip the hold state; returns the new state.
    pub fn toggle_pause(&mut self) -> bool {
        let paused = !self.paused;
        self.set_paused(paused);
        paused
    }

    /// Hand in a frame's time delta in milliseconds; returns how many ticks it
    /// played. A non-finite or negative delta plays nothing, a stalled page
    /// plays at most [`MAX_CATCHUP`] and drops the rest of its backlog.
    pub fn advance_ms(&mut self, dt_ms: f64) -> u32 {
        if self.paused {
            return 0;
        }
        let dt = if dt_ms.is_finite() {
            dt_ms.clamp(0.0, 1000.0)
        } else {
            0.0
        };
        self.acc_ms += dt;
        let mut played = 0;
        while self.acc_ms >= f64::from(TICK_MS) && played < MAX_CATCHUP {
            self.game.advance(1);
            self.acc_ms -= f64::from(TICK_MS);
            played += 1;
        }
        if self.acc_ms >= f64::from(TICK_MS) {
            self.acc_ms = 0.0;
        }
        played
    }

    /// Shift the view by a world-space delta on the ground plane; returns the
    /// pan now in force. The fixed view direction never changes.
    pub fn pan(&mut self, dx: f32, dz: f32) -> [f32; 2] {
        if dx.is_finite() && dz.is_finite() {
            self.pan = [
                (self.pan[0] + dx).clamp(-PAN_LIMIT, PAN_LIMIT),
                (self.pan[1] + dz).clamp(-PAN_LIMIT, PAN_LIMIT),
            ];
            self.camera.pan_to(self.pan);
        }
        self.pan
    }

    /// The pan the camera holds, in world units.
    #[must_use]
    pub const fn pan_offset(&self) -> [f32; 2] {
        self.pan
    }

    /// The frame as the window builds it: ground diamonds, cottage cells and
    /// one soldier cell per unit, depth ordered.
    #[must_use]
    pub fn scene(&self) -> Scene {
        build_with(
            self.game.sim(),
            &self.camera,
            &self.units,
            &self.houses,
            self.game.sim().ticks(),
        )
    }

    /// The flat frame the page reads; layout in [`crate::scene_flat`].
    #[must_use]
    pub fn frame(&self) -> Vec<f32> {
        crate::scene_flat::flatten(&self.scene())
    }

    /// State digest of the match, sixteen lowercase hex digits.
    #[must_use]
    pub fn hash(&self) -> String {
        format!("{:016x}", self.game.sim().hash())
    }

    /// The stock line the HUD prints, in ware order.
    #[must_use]
    pub fn stock(&self) -> String {
        stock_line(self.game.sim())
    }

    /// Houses standing on the field.
    #[must_use]
    pub fn houses(&self) -> u32 {
        u32::try_from(self.game.sim().houses().len()).unwrap_or(u32::MAX)
    }

    /// Units on the field.
    #[must_use]
    pub fn units(&self) -> u32 {
        u32::try_from(self.game.sim().units().len()).unwrap_or(u32::MAX)
    }

    /// The sim's own answer about a house standing at `(x, z)`: `None` when the
    /// sim would take it, the refusal it gave otherwise.
    ///
    /// The answer is the sim's, never a copy of its rule: the probe runs the
    /// same command on a clone of the live match, so the hover footprint and the
    /// click can never disagree, and the live match never sees the probe.
    #[must_use]
    pub fn placement(&self, kind: HouseKind, x: i32, z: i32) -> Option<Refusal> {
        let mut probe = self.game.sim().clone();
        probe
            .apply_for(PLAYER_SIDE, Command::PlaceHouse { kind, x, y: z })
            .err()
    }

    /// Place a house for the player; `None` when the sim took it.
    pub fn build(&mut self, kind: HouseKind, x: i32, z: i32) -> Option<Refusal> {
        self.build_for(PLAYER_SIDE, kind, x, z)
    }

    /// Place a house for one side; `None` when the sim took it.
    pub fn build_for(&mut self, side: u8, kind: HouseKind, x: i32, z: i32) -> Option<Refusal> {
        self.game
            .order_for(side, Command::PlaceHouse { kind, x, y: z })
            .err()
    }

    /// How many of each ware one side holds, in ware order: wood, stone, grain,
    /// flour, bread.
    #[must_use]
    pub fn stock_counts(&self, side: u8) -> Vec<u32> {
        Ware::ALL
            .iter()
            .map(|ware| self.game.sim().stock_of(side, *ware))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Shell, MAX_CATCHUP, PAN_LIMIT, TICK_MS};
    use kam2_game::AiMatch;
    use kam2_sim::{HouseKind, Refusal};

    fn shell() -> Shell {
        Shell::new().expect("the start village places")
    }

    #[test]
    fn one_tick_needs_one_hundred_milliseconds() {
        let mut shell = shell();
        assert_eq!(shell.advance_ms(99.0), 0);
        assert_eq!(shell.ticks(), 0);
        assert_eq!(shell.advance_ms(1.0), 1);
        assert_eq!(shell.ticks(), 1);
    }

    #[test]
    fn short_frames_add_up_to_the_next_tick() {
        let mut shell = shell();
        for _ in 0..6 {
            assert_eq!(shell.advance_ms(16.0), 0, "a 60 Hz frame is under one tick");
        }
        assert_eq!(shell.advance_ms(3.0), 0, "99 ms is still short");
        assert_eq!(shell.advance_ms(1.0), 1, "the hundredth millisecond ticks");
    }

    #[test]
    fn one_hundred_and_twenty_frames_play_the_same_match_as_one_advance() {
        let mut shell = shell();
        for _ in 0..120 {
            shell.advance_ms(f64::from(TICK_MS));
        }
        let mut direct = AiMatch::village().expect("the start village places");
        direct.advance(120);
        assert_eq!(shell.ticks(), 120);
        assert_eq!(shell.hash(), format!("{:016x}", direct.sim().hash()));
    }

    #[test]
    fn a_paused_match_never_ticks_and_resumes_without_a_burst() {
        let mut shell = shell();
        assert!(shell.toggle_pause());
        assert_eq!(shell.advance_ms(1000.0), 0, "a paused shell plays nothing");
        assert_eq!(shell.ticks(), 0);
        assert!(!shell.toggle_pause());
        assert_eq!(shell.advance_ms(100.0), 1, "the first frame after resume");
        assert_eq!(shell.ticks(), 1);
    }

    #[test]
    fn a_stalled_page_catches_up_at_most_four_ticks() {
        let mut shell = shell();
        assert_eq!(shell.advance_ms(1000.0), MAX_CATCHUP);
        assert_eq!(shell.ticks(), MAX_CATCHUP);
        assert_eq!(shell.advance_ms(100.0), 1, "the backlog was dropped");
    }

    #[test]
    fn a_broken_delta_plays_nothing() {
        let mut shell = shell();
        assert_eq!(shell.advance_ms(f64::NAN), 0);
        assert_eq!(shell.advance_ms(f64::INFINITY), 0);
        assert_eq!(shell.advance_ms(-500.0), 0);
        assert_eq!(shell.ticks(), 0);
    }

    #[test]
    fn the_pan_stops_at_its_limit_and_moves_the_projection() {
        let mut shell = shell();
        let before = shell.scene().ground[0].corners[0];
        shell.pan(1.0, 0.0);
        let after = shell.scene().ground[0].corners[0];
        assert!(
            (after[0] - before[0] + 32.0).abs() < 0.01,
            "one world unit is 32 px"
        );
        let panned = shell.pan(PAN_LIMIT, PAN_LIMIT);
        assert_eq!(panned, [PAN_LIMIT, PAN_LIMIT]);
        shell.pan(PAN_LIMIT, PAN_LIMIT);
        assert_eq!(shell.pan_offset(), [PAN_LIMIT, PAN_LIMIT], "the pan clamps");
    }

    #[test]
    fn the_frame_holds_the_village_the_ai_grows() {
        let mut shell = shell();
        let start = shell.frame();
        let houses = shell.houses();
        assert_eq!(houses, 3, "the start village has three houses");
        assert_eq!(shell.units(), 4);
        assert!(start.len() > 2000, "the flat frame is more than a line");
        shell.advance_ms(f64::from(TICK_MS) * 200.0);
        assert!(shell.houses() >= houses, "the AI never removes a house");
        let later = shell.frame();
        assert!(
            later[0] > 0.0 && later[3] > 0.0,
            "the later frame still holds ground and a house cell"
        );
    }

    #[test]
    fn the_probe_answers_the_sim_and_never_moves_the_match() {
        let shell = shell();
        let hash = shell.hash();
        assert_eq!(
            shell.placement(HouseKind::Farm, 6, 4),
            None,
            "the sim takes a farm beside the road"
        );
        assert_eq!(
            shell.placement(HouseKind::Farm, 6, 6),
            Some(Refusal::NotBuildable),
            "the road cannot carry a house"
        );
        assert_eq!(shell.hash(), hash, "the probe never moves the live match");
    }

    #[test]
    fn a_farm_stands_beside_the_road_and_the_road_refuses_it() {
        let mut shell = shell();
        let houses = shell.houses();
        assert_eq!(shell.build(HouseKind::Farm, 6, 4), None);
        assert_eq!(shell.houses(), houses + 1, "the farm stands");
        assert_eq!(
            shell.build(HouseKind::Farm, 6, 6),
            Some(Refusal::NotBuildable),
            "the road refuses the second farm"
        );
        assert_eq!(shell.houses(), houses + 1, "the refused house never stands");
    }

    #[test]
    fn the_stock_counts_name_the_five_wares() {
        let shell = shell();
        let counts = shell.stock_counts(0);
        assert_eq!(counts.len(), 5, "wood, stone, grain, flour, bread");
        assert_eq!(counts[0], 18, "the village leaves side 0 eighteen wood");
        assert_eq!(counts[1], 15, "and fifteen stone");
        assert_eq!(counts[2..], [0, 0, 0], "no bread chain has run yet");
    }
}
