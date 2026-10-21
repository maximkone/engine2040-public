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

use crate::selection::{
    command, house_json, house_state, select_at, selection_json, unit_json, unit_state, HouseState,
    Order, Selection, UnitState,
};

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
    /// What the page picked, oldest first: the panel shows the newest one and
    /// the orders go to every soldier in the list.
    selection: Vec<Selection>,
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
            selection: Vec::new(),
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

    /// The camera the page looks through.
    #[must_use]
    pub const fn camera(&self) -> &IsoCamera {
        &self.camera
    }

    /// What stands under the cursor at `(x, z)`, without touching the
    /// selection.
    ///
    /// The page asks this before it decides what a click means: a foe under the
    /// cursor is struck by the soldiers already picked, so the pick may not
    /// change the selection on the way.
    #[must_use]
    pub fn pick_at(&self, x: i32, z: i32) -> Option<Selection> {
        select_at(self.game.sim(), x, z)
    }

    /// Pick what stands under the cursor at `(x, z)`; returns what it holds.
    ///
    /// A plain pick replaces the selection, so a click on open ground clears
    /// it; `add` keeps the picks and adds this one, which is the shift-click of
    /// the page. A thing picked twice is picked once.
    pub fn select_at(&mut self, x: i32, z: i32, add: bool) -> Option<Selection> {
        let picked = self.pick_at(x, z);
        if add {
            if let Some(picked) = picked {
                if !self.selection.contains(&picked) {
                    self.selection.push(picked);
                }
            }
        } else {
            self.selection.clear();
            if let Some(picked) = picked {
                self.selection.push(picked);
            }
        }
        picked
    }

    /// The picked things, oldest first.
    #[must_use]
    pub fn selection(&self) -> &[Selection] {
        &self.selection
    }

    /// The newest pick, the one the panel shows.
    #[must_use]
    pub fn picked(&self) -> Option<Selection> {
        self.selection.last().copied()
    }

    /// Give one order to every picked soldier; the first refusal of the sim
    /// is the answer of the page.
    ///
    /// With no soldier picked the sim is still asked, so the refusal carries
    /// the sim's own words and never the page's.
    pub fn order_selected(&mut self, order: Order, x: i32, z: i32) -> Option<Refusal> {
        let soldiers: Vec<u32> = self
            .selection
            .iter()
            .filter_map(|picked| match picked {
                Selection::Unit(id) => Some(*id),
                Selection::House(_) => None,
            })
            .collect();
        let mut first = None;
        for id in &soldiers {
            let cmd = command(self.game.sim(), Some(Selection::Unit(*id)), order, x, z);
            if let Err(refusal) = self.game.order_for(PLAYER_SIDE, cmd) {
                if first.is_none() {
                    first = Some(refusal);
                }
            }
        }
        if soldiers.is_empty() {
            let cmd = command(self.game.sim(), None, order, x, z);
            if let Err(refusal) = self.game.order_for(PLAYER_SIDE, cmd) {
                first = Some(refusal);
            }
        }
        first
    }

    /// The panel's numbers for one house; `None` when no house has that id.
    #[must_use]
    pub fn house_state(&self, id: u32) -> Option<HouseState> {
        house_state(self.game.sim(), id)
    }

    /// The panel's numbers for one unit; `None` when no unit has that id.
    #[must_use]
    pub fn unit_state(&self, id: u32) -> Option<UnitState> {
        unit_state(self.game.sim(), id)
    }

    /// The panel's line for one house; empty when no house has that id.
    #[must_use]
    pub fn house_json(&self, id: u32) -> String {
        house_json(self.game.sim(), id)
    }

    /// The panel's line for one unit; empty when no unit has that id.
    #[must_use]
    pub fn unit_json(&self, id: u32) -> String {
        unit_json(self.game.sim(), id)
    }

    /// The minimap as one JSON line: the field, its markers and the window the
    /// camera looks at.
    #[must_use]
    pub fn minimap_json(&self) -> String {
        crate::minimap::minimap_json(self.game.sim(), &self.camera)
    }

    /// The selection as one JSON list of ids, the newest pick last.
    #[must_use]
    pub fn selection_json(&self) -> String {
        selection_json(&self.selection)
    }
}

#[cfg(test)]
mod tests {
    use super::{Shell, MAX_CATCHUP, PAN_LIMIT, TICK_MS};
    use crate::selection::{Order, Selection};
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

    #[test]
    fn a_click_picks_what_stands_under_it_and_open_ground_clears_the_pick() {
        let mut shell = shell();
        assert_eq!(shell.select_at(3, 9, false), Some(Selection::Unit(0)));
        assert_eq!(shell.selection(), &[Selection::Unit(0)]);
        assert_eq!(shell.picked(), Some(Selection::Unit(0)));
        assert_eq!(shell.select_at(0, 3, false), Some(Selection::House(0)));
        assert_eq!(shell.selection(), &[Selection::House(0)]);
        assert_eq!(shell.select_at(15, 11, false), None);
        assert!(shell.selection().is_empty(), "open ground clears the pick");
    }

    #[test]
    fn a_shift_click_keeps_the_picks_and_adds_one() {
        let mut shell = shell();
        shell.select_at(3, 9, false);
        shell.select_at(5, 9, true);
        assert_eq!(shell.selection(), &[Selection::Unit(0), Selection::Unit(1)]);
        shell.select_at(5, 9, true);
        assert_eq!(
            shell.selection().len(),
            2,
            "the same soldier is picked once"
        );
    }

    #[test]
    fn the_query_under_the_cursor_never_moves_the_selection() {
        let mut shell = shell();
        shell.select_at(3, 9, false);
        assert_eq!(shell.pick_at(11, 9), Some(Selection::Unit(2)));
        assert_eq!(
            shell.selection(),
            &[Selection::Unit(0)],
            "a foe under the cursor does not become the pick"
        );
    }

    #[test]
    fn an_order_goes_to_every_picked_soldier() {
        let mut shell = shell();
        shell.select_at(3, 9, false);
        shell.select_at(5, 9, true);
        assert_eq!(shell.order_selected(Order::AttackMove, 7, 9), None);
        for id in [0, 1] {
            let state = shell.unit_state(id).expect("the soldier lives");
            assert!(state.moving, "soldier {id} walks to the named tile");
        }
    }

    #[test]
    fn a_house_takes_no_order_and_the_sim_says_so() {
        let mut shell = shell();
        shell.select_at(0, 3, false);
        assert_eq!(
            shell.order_selected(Order::AttackMove, 7, 9),
            Some(Refusal::NoUnit),
            "a house is not a soldier, so the sim hears no unit"
        );
    }

    #[test]
    fn an_order_without_a_pick_is_the_sim_s_own_refusal() {
        let mut shell = shell();
        assert_eq!(
            shell.order_selected(Order::AttackMove, 7, 9),
            Some(Refusal::NoUnit),
            "the sim refuses in its own words"
        );
    }

    #[test]
    fn the_panel_reads_the_numbers_of_the_picked_thing() {
        let mut shell = shell();
        shell.select_at(3, 9, false);
        let soldier = shell.unit_state(0).expect("the panel reads the soldier");
        assert_eq!(soldier.max_hp, 30);
        assert_eq!(soldier.hp, 30);
        let json = shell.unit_json(0);
        assert!(json.contains("\"name\":\"swordsman\""), "{json}");
        shell.select_at(0, 3, false);
        let house = shell.house_state(0).expect("the panel reads the warehouse");
        assert_eq!(house.kind, HouseKind::Warehouse);
        assert!(
            shell.house_json(0).contains("\"name\":\"warehouse\""),
            "the panel names the house"
        );
    }

    #[test]
    fn the_selection_line_names_the_picks_newest_last() {
        let mut shell = shell();
        shell.select_at(3, 9, false);
        shell.select_at(5, 9, true);
        assert_eq!(shell.selection_json(), "[\"u0\",\"u1\"]");
    }

    #[test]
    fn the_minimap_line_carries_the_field_and_the_window() {
        let shell = shell();
        let json = shell.minimap_json();
        assert!(json.contains("\"w\":160"), "{json}");
        assert!(json.contains("\"h\":120"), "{json}");
        assert!(
            json.contains("\"rect\":[40.00,30.00,80.00,60.00]"),
            "the window sits where the camera looks: {json}"
        );
    }
}
