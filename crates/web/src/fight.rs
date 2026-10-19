//! fight (wasm32): the page plays the match it draws (WEB15).
//!
//! The arena page used to draw a stand-in ring with eight cosmetically swaying bodies, so a
//! key press moved nothing. This module holds the same session the native window holds
//! (`engine_game::App` over the kit world, duel rules), steps it at the fixed arena rate from
//! the page's own clock, and turns each step into the dots the native fight draws with. The
//! ring stays the procedural scenery until the cooked kit is fetched (WEB6/QW4-QW6); the
//! bodies come from the match, not from a loop.
//!
//! Names stay neutral here (seat, body), so the page does not grow a genre assumption: the
//! crate's own ledger lists the words it may use.
use std::fmt;

use engine_game::arena_kit::kit_match_world_or_empty;
use engine_game::windowed::{live_fight_dots, push_fight_draws, stage_fight_bodies};
use engine_game::{App, GameConfig};
use engine_net::session::BodyInput;
use engine_render::present::{DrawList, Presenter};
use engine_render::procedural_arena::ProceduralArena;

use crate::input::Facing;

/// Seats in the page's match: the human seat plus one brain.
pub const SEATS: u8 = 2;
/// Fixed steps the page's match may run before it stops: ten minutes at 60 Hz, so a person
/// plays rather than watching a 600-tick receipt end.
pub const MATCH_TICKS: u32 = 36_000;
/// Seed of the page's match; the native arena's default seed.
pub const MATCH_SEED: u64 = 2040;
/// Half-extent of the arena the page plays in, meters.
pub const HALF_EXTENT_M: f32 = 12.0;

/// Why the page could not build or draw a match frame.
#[derive(Debug)]
pub enum MatchError {
    /// The session, the kit world or the duel rules refused the config.
    Session(String),
    /// The procedural scenery refused the frame.
    Scenery(String),
    /// The frame size was refused.
    Camera(String),
    /// The presenter refused a body upload.
    Stage(String),
    /// A body or the pairing needed to frame them was refused.
    Bodies(String),
}

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(why) => write!(f, "session: {why}"),
            Self::Scenery(why) => write!(f, "scenery: {why}"),
            Self::Camera(why) => write!(f, "camera: {why}"),
            Self::Stage(why) => write!(f, "stage: {why}"),
            Self::Bodies(why) => write!(f, "bodies: {why}"),
        }
    }
}

impl std::error::Error for MatchError {}

/// One played match on the page plus the scenery it is drawn over.
pub struct Match {
    app: App,
    arena: ProceduralArena,
    tick: u32,
    facing: Facing,
}

impl Match {
    /// Build the session, or say why the world or the rules refused it.
    ///
    /// # Errors
    /// [`MatchError::Session`] or [`MatchError::Scenery`] when the session or the ring refuse.
    pub fn new() -> Result<Self, MatchError> {
        let app = Self::session()?;
        let arena = ProceduralArena::build().map_err(|e| MatchError::Scenery(e.to_string()))?;
        Ok(Self {
            app,
            arena,
            tick: 0,
            facing: Facing::default(),
        })
    }

    /// One fresh session: the kit world, the duel rules, the page's own seed.
    ///
    /// # Errors
    /// [`MatchError::Session`] when the config, the world or the duel rules refuse.
    fn session() -> Result<App, MatchError> {
        let config = GameConfig {
            seed: MATCH_SEED,
            ticks: MATCH_TICKS,
            fighters: SEATS,
            half_extent_m: HALF_EXTENT_M,
        };
        App::with_world(config, kit_match_world_or_empty())
            .map_err(|e| MatchError::Session(e.to_string()))?
            .into_duel()
            .map_err(|e| MatchError::Session(e.to_string()))
    }

    /// The next tick the match will accept.
    #[must_use]
    pub fn next_tick(&self) -> u32 {
        self.tick
    }

    /// The seat's facing for this tick: the mouse look plus a tick of Q/E, as the native window
    /// and the net duel do.
    pub fn facing_for(&mut self, look_yaw_mdeg: i32, left: bool, right: bool) -> i16 {
        self.facing.turn(look_yaw_mdeg, left, right)
    }

    /// Step one fixed tick with the human seat's input; the brains fill every other slot. A
    /// decided round closes and starts the next one, and a session at its bound starts a fresh
    /// match, so an idle or long-running page keeps playing instead of freezing.
    pub fn step(&mut self, input: BodyInput) {
        let outcome = self.app.tick(input);
        if !outcome.input_accepted {
            self.restart();
            return;
        }
        self.tick = self.tick.saturating_add(1);
        let hp = self.app.hp();
        if hp.first().copied().unwrap_or(0) == 0 || hp.get(1).copied().unwrap_or(0) == 0 {
            self.app.close_round();
            self.app.reset_for_next_round();
            if self.app.match_winner().is_some() {
                self.restart();
            }
        }
    }

    /// A fresh match on the same scenery: the session is rebuilt, the ring is kept.
    fn restart(&mut self) {
        if let Ok(app) = Self::session() {
            self.app = app;
            self.tick = 0;
        }
    }

    /// The seat's line: the tick, both seats' hp and where they stand, so a run is readable
    /// without a GPU (and so a capture's bodies can be checked against the sim).
    #[must_use]
    pub fn line(&self) -> String {
        let hp = self.app.hp();
        let at = live_fight_dots(&self.app, &[])
            .iter()
            .map(|dot| format!("({:.1},{:.1})", dot.pos_m[0], dot.pos_m[2]))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "match: tick={} hp={}/{} at={at}",
            self.app.cursor(),
            hp.first().copied().unwrap_or(0),
            hp.get(1).copied().unwrap_or(0)
        )
    }

    /// One frame: the scenery list, then the match's own bodies staged over it.
    ///
    /// # Errors
    /// [`MatchError`] naming the render or game step that refused the frame.
    pub fn list(
        &mut self,
        presenter: &mut Presenter,
        width: u32,
        height: u32,
        t_s: f32,
    ) -> Result<DrawList, MatchError> {
        let dots = live_fight_dots(&self.app, &[]);
        stage_fight_bodies(presenter, &dots).map_err(|e| MatchError::Stage(e.to_string()))?;
        let resident = presenter.mesh_keys();
        let mut list = self
            .arena
            .list_scenery(width, height, t_s)
            .map_err(|e| MatchError::Scenery(e.to_string()))?;
        // Both seats in frame: their own feet drive the camera once a pose exists.
        if let (Some(a), Some(b)) = (dots.first(), dots.get(1)) {
            list.camera = self
                .arena
                .framing_camera(width, height, a.pos_m, b.pos_m)
                .map_err(|e| MatchError::Camera(e.to_string()))?;
        }
        push_fight_draws(&mut list, &resident, &dots)
            .map_err(|e| MatchError::Bodies(e.to_string()))?;
        Ok(list)
    }
}
