//! fight (wasm32): the page plays the match it draws (WEB15).
//!
//! The arena page used to draw a stand-in ring with eight cosmetically swaying bodies, so a
//! key press moved nothing. This module holds the same session the native window holds
//! (`engine_game::App` over the kit world, duel rules), steps it at the fixed arena rate from
//! the page's own clock, and turns each step into the dots the native fight draws with. The
//! ring stays the procedural scenery until the cooked kit is fetched (WEB6/QW4-QW6); the
//! bodies come from the match, not from a loop.
//!
//! The tick's own fight cues feed the draw, so a hit flashes the body it landed on, and they
//! feed the shoulder camera's shake. [`Match::hud`] is a small snapshot of the round state the
//! page paints as two health bars, round pips and a K.O./WINNER banner.
//!
//! Names stay neutral here (seat, body), so the page does not grow a genre assumption: the
//! crate's own ledger lists the words it may use.
use std::fmt;

use engine_assets::AssetSource;
use engine_audio::FightCue;
use engine_game::arena_kit::{kit_match_world_or_empty, push_kit_draws, stage_kit_from, KitReport};
use engine_game::rounds::{RoundWinner, WINS_FOR_MATCH};
use engine_game::windowed::{live_fight_dots, push_fight_draws, stage_fight_bodies, BodyDot};
use engine_game::{App, GameConfig};
use engine_net::session::BodyInput;
use engine_render::present::{DrawList, Presenter};
use engine_render::procedural_arena::{ProceduralArena, ShoulderView};

use crate::input::Facing;

/// Seats in the page's match: the human seat plus one brain.
pub const SEATS: u8 = engine_game::page_duel::PAGE_DUEL_SEATS;
/// Fixed steps the page's match may run before it stops: ten minutes at 60 Hz, so a person
/// plays rather than watching a 600-tick receipt end.
pub const MATCH_TICKS: u32 = 36_000;
/// Seed of the page's match; engine-game owns it (`page_duel`), and picks the one seed whose
/// two seats start on the corridor's centre line with the rival in front of the camera.
pub const MATCH_SEED: u64 = engine_game::page_duel::PAGE_DUEL_SEED;
/// Half-extent of the arena the page plays in, meters; engine-game owns it (`page_duel`). The
/// duel spawns at random points in a box whose limit is this extent minus a fighter radius, so
/// the extent is big enough that the kit's own walls (z ±10 between x -6 and 2, stand backs at
/// x -14, gate from x 7) stop a fighter instead of an invisible box in the middle of the sand.
/// Thirteen meters, well over the config floor of 5.0 (`half_extent_m` is `5.0..=100.0`), so
/// the sim's own walls sit past the corridor's stone and the stone does the stopping.
pub const HALF_EXTENT_M: f32 = engine_game::page_duel::PAGE_DUEL_HALF_EXTENT_M;

/// The kit corridor's free ground, meters, with the eye kept a margin off its walls: the walls'
/// inner faces at z ±9.5, the gate's at x 7, the stands' at x -14.
const KIT_EYE_BOUNDS: engine_render::procedural_arena::EyeBounds =
    engine_render::procedural_arena::EyeBounds {
        min_x: -12.5,
        max_x: 6.0,
        min_z: -8.8,
        max_z: 8.8,
    };
/// Ticks a round-ending K.O. banner stays up, at the fixed 60 Hz rate: one and a quarter
/// seconds. The match pauses for it, so the last pose and the empty bar stay on screen.
pub const KO_HOLD_TICKS: u32 = 75;
/// Ticks the match-winner banner stays up before a fresh match starts: two and a half seconds.
pub const WINNER_HOLD_TICKS: u32 = 150;
/// Ticks a landed hit keeps the body white. Longer than one fixed step, because the page draws
/// at 30 Hz and steps up to two ticks per frame.
pub const HIT_FLASH_TICKS: u32 = 8;

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

/// The big word the played page shows when a round or the match ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HudBanner {
    /// A round ended on a knockout.
    Ko,
    /// A side took the match.
    Winner,
}

impl HudBanner {
    /// The page's text for the banner.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Ko => "K.O.",
            Self::Winner => "WINNER",
        }
    }
}

/// The played page's whole HUD, from the match's own state. Small and `Copy`, so the caller can
/// compare it with the last frame's snapshot and touch the DOM only when something changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MatchHud {
    /// Health in permille for (human, rival).
    pub hp: [u16; 2],
    /// Rounds won by (human, rival) this match.
    pub wins: [u32; 2],
    /// Rounds played including ties, from the app's own round state.
    pub rounds_played: u32,
    /// `Some(0)` for the human, `Some(1)` for the rival once the match is decided.
    pub winner: Option<u8>,
    /// The K.O./WINNER flash while it holds.
    pub banner: Option<HudBanner>,
}

impl MatchHud {
    /// Pips for one side of a best-of-five: filled for each win, hollow for the rest.
    #[must_use]
    pub fn pips(&self, side: usize) -> String {
        let won = self
            .wins
            .get(side)
            .copied()
            .unwrap_or(0)
            .min(WINS_FOR_MATCH);
        let left = WINS_FOR_MATCH - won;
        let mut out = String::with_capacity(3 * usize::try_from(WINS_FOR_MATCH).unwrap_or(3));
        for _ in 0..won {
            out.push('●');
        }
        for _ in 0..left {
            out.push('○');
        }
        out
    }

    /// The next round's number as the page shows it (ties replay, so it can pass five).
    #[must_use]
    pub fn round_number(&self) -> u32 {
        self.rounds_played.saturating_add(1)
    }
}

/// One played match on the page plus the scenery it is drawn over.
pub struct Match {
    app: App,
    arena: ProceduralArena,
    tick: u32,
    facing: Facing,
    /// The human seat's facing for the last step, centidegrees; the shoulder camera uses it.
    facing_cdeg: i16,
    /// This tick's fight cues, so hits flash on the frame that follows them.
    cues: Vec<FightCue>,
    /// Ticks each seat's body still flashes white, index by slot.
    flash: [u32; 2],
    /// Round wins by (human, rival) this match, folded from the app's round verdicts.
    wins: [u32; 2],
    /// The banner up right now, and the ticks it has left.
    banner: Option<HudBanner>,
    hold: u32,
    /// The HUD snapshot the page was last told about, so its DOM is touched only on a change.
    painted: Option<MatchHud>,
    /// Bodies the presenter accepted on the last frame, and bodies the list drew.
    staged: usize,
    drawn: usize,
    /// Last frame's camera as `[eye xyz, target xyz]`, so the page's line can be checked
    /// against the seats it frames.
    cam: [f32; 6],
    /// The staged arena kit (WEB17): `None` until the fetched bundle is imported, and `None`
    /// for good when the fetch or the import was refused (the procedural ring then stands in).
    kit: Option<KitReport>,
    /// Kit draws resident on the last frame.
    kit_drawn: usize,
}

impl Match {
    /// Build the session, or say why the world or the rules refused it. When `kit` is `Some`,
    /// the arena kit is imported and staged once here (cold init); a refused import leaves the
    /// match playable over the procedural ring instead of failing the page.
    ///
    /// # Errors
    /// [`MatchError::Session`] or [`MatchError::Scenery`] when the session or the ring refuse.
    pub fn new(
        kit: Option<&dyn AssetSource>,
        presenter: &mut Presenter,
    ) -> Result<Self, MatchError> {
        let app = Self::session()?;
        let arena = ProceduralArena::build().map_err(|e| MatchError::Scenery(e.to_string()))?;
        let kit = kit.and_then(|source| stage_kit_from(presenter, source).ok());
        Ok(Self {
            app,
            arena,
            tick: 0,
            facing: Facing::default(),
            facing_cdeg: 0,
            cues: Vec::new(),
            flash: [0; 2],
            wins: [0; 2],
            banner: None,
            hold: 0,
            painted: None,
            staged: 0,
            drawn: 0,
            cam: [0.0; 6],
            kit,
            kit_drawn: 0,
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
        Ok(App::with_world(config, kit_match_world_or_empty())
            .map_err(|e| MatchError::Session(e.to_string()))?
            .into_duel()
            .map_err(|e| MatchError::Session(e.to_string()))?
            // The page paints none of the extras (no crowd draw, no director cut, no
            // checkpoint log), so it skips them: the same accepted tick, cues and hp.
            .without_presentation_extras())
    }

    /// The next tick the match will accept.
    #[must_use]
    pub fn next_tick(&self) -> u32 {
        self.tick
    }

    /// The fight cues of the last tick the match ran, so the page can name what happened (the
    /// parry, the block, the hit) instead of showing a silent fight (PW37).
    #[must_use]
    pub fn cues(&self) -> &[FightCue] {
        &self.cues
    }

    /// The kit draws resident on the last frame: zero while the stand-in ring still draws.
    #[must_use]
    pub fn kit_drawn(&self) -> usize {
        self.kit_drawn
    }

    /// The seat's facing for this tick: the mouse look plus a tick of Q/E, as the native window
    /// and the net duel do. Kept for the camera, which sits behind this heading.
    pub fn facing_for(&mut self, look_yaw_mdeg: i32, left: bool, right: bool) -> i16 {
        let facing = self.facing.turn(look_yaw_mdeg, left, right);
        self.facing_cdeg = facing;
        facing
    }

    /// Step one fixed tick with the human seat's input; the brains fill every other slot. A
    /// decided round closes and starts the next one, and a session at its bound starts a fresh
    /// match, so an idle or long-running page keeps playing instead of freezing.
    ///
    /// A round that ends on a knockout holds the match for [`KO_HOLD_TICKS`] (or
    /// [`WINNER_HOLD_TICKS`] for the match itself) so the last pose, the empty bar and the
    /// banner are readable; the hold never advances the sim.
    pub fn step(&mut self, input: BodyInput) {
        for left in &mut self.flash {
            *left = left.saturating_sub(1);
        }
        if self.hold > 0 {
            self.hold -= 1;
            if self.hold == 0 {
                self.finish_hold();
            }
            return;
        }
        let outcome = self.app.tick(input);
        if !outcome.input_accepted {
            self.restart();
            return;
        }
        self.tick = self.tick.saturating_add(1);
        self.cues = outcome.cues;
        self.latch_flash();
        let hp = self.app.hp();
        if hp.first().copied().unwrap_or(0) == 0 || hp.get(1).copied().unwrap_or(0) == 0 {
            match self.app.close_round() {
                RoundWinner::Slot0 => self.wins[0] = self.wins[0].saturating_add(1),
                RoundWinner::Slot1 => self.wins[1] = self.wins[1].saturating_add(1),
                RoundWinner::Tie => {}
            }
            if self.app.match_winner().is_some() {
                self.banner = Some(HudBanner::Winner);
                self.hold = WINNER_HOLD_TICKS;
            } else {
                self.banner = Some(HudBanner::Ko);
                self.hold = KO_HOLD_TICKS;
            }
        }
    }

    /// Light the flash of every seat the tick's cues hit.
    fn latch_flash(&mut self) {
        for cue in &self.cues {
            let victim = match *cue {
                FightCue::Hit { victim, .. } | FightCue::Knockdown { victim, .. } => victim,
                FightCue::Blocked { .. }
                | FightCue::Parried { .. }
                | FightCue::Dodged { .. }
                | FightCue::StaminaOut { .. } => continue,
            };
            if let Some(left) = self.flash.get_mut(usize::from(victim)) {
                *left = HIT_FLASH_TICKS;
            }
        }
    }

    /// The hold after a round or match ended: clear the banner, then start the next round or a
    /// fresh match.
    fn finish_hold(&mut self) {
        self.banner = None;
        if self.app.match_winner().is_some() {
            self.restart();
        } else {
            self.app.reset_for_next_round();
        }
    }

    /// A fresh match on the same scenery: the session is rebuilt, the ring is kept.
    fn restart(&mut self) {
        if let Ok(app) = Self::session() {
            self.app = app;
            self.tick = 0;
            self.cues.clear();
            self.flash = [0; 2];
            self.wins = [0; 2];
            self.banner = None;
            self.hold = 0;
            self.painted = None;
        }
    }

    /// The match's live bodies: one dot per seat, with this tick's hit and the flash the last
    /// few steps latched (so a hit reads at the page's 30 Hz draw rate too).
    fn dots(&self) -> Vec<BodyDot> {
        let mut dots = live_fight_dots(&self.app, &self.cues);
        for dot in &mut dots {
            if self.flash.get(usize::from(dot.slot)).copied().unwrap_or(0) > 0 {
                dot.hit = true;
            }
        }
        dots
    }

    /// The camera shake weight, `0.0..=1.0`: the strongest hit flash still lit.
    fn shake(&self) -> f32 {
        let left = self.flash.iter().copied().max().unwrap_or(0);
        f32::from(u16::try_from(left).unwrap_or(0))
            / f32::from(u16::try_from(HIT_FLASH_TICKS).unwrap_or(1))
    }

    /// The HUD snapshot the page paints: health, round wins, the round number and the banner.
    #[must_use]
    pub fn hud(&self) -> MatchHud {
        let hp = self.app.hp();
        MatchHud {
            hp: [
                hp.first().copied().unwrap_or(0),
                hp.get(1).copied().unwrap_or(0),
            ],
            wins: self.wins,
            rounds_played: self.app.rounds_played(),
            winner: self.app.match_winner().and_then(|winner| match winner {
                RoundWinner::Slot0 => Some(0),
                RoundWinner::Slot1 => Some(1),
                RoundWinner::Tie => None,
            }),
            banner: self.banner,
        }
    }

    /// The HUD snapshot when it changed since the last call, so the page touches its DOM only
    /// when something really moved; `None` on a quiet frame.
    pub fn hud_change(&mut self) -> Option<MatchHud> {
        let hud = self.hud();
        if self.painted == Some(hud) {
            return None;
        }
        self.painted = Some(hud);
        Some(hud)
    }

    /// The seat's line: the tick, both seats' hp, the round score and where they stand, so a
    /// run is readable without a GPU (and so a capture's bodies can be checked against the sim).
    #[must_use]
    pub fn line(&self) -> String {
        let hp = self.app.hp();
        let at = self
            .dots()
            .iter()
            .map(|dot| format!("({:.1},{:.1})", dot.pos_m[0], dot.pos_m[2]))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "match: tick={} hp={}/{} rounds={}/{} bodies={}/{} kit={}/{} at={at} cam=({:.1},{:.1},{:.1})->({:.1},{:.1},{:.1})",
            self.app.cursor(),
            hp.first().copied().unwrap_or(0),
            hp.get(1).copied().unwrap_or(0),
            self.wins[0],
            self.wins[1],
            self.drawn,
            self.staged,
            self.kit_drawn,
            self.kit.as_ref().map_or(0, KitReport::draw_count),
            self.cam[0],
            self.cam[1],
            self.cam[2],
            self.cam[3],
            self.cam[4],
            self.cam[5],
        )
    }

    /// One frame: the scenery list, then the match's own bodies staged over it, under a camera
    /// behind the human seat.
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
        let dots = self.dots();
        self.staged =
            stage_fight_bodies(presenter, &dots).map_err(|e| MatchError::Stage(e.to_string()))?;
        let resident = presenter.mesh_keys();
        let mut list = self
            .arena
            .list_scenery(width, height, t_s)
            .map_err(|e| MatchError::Scenery(e.to_string()))?;
        // WEB17: once the cooked kit is staged, it is the arena: the procedural ring's items go
        // and the kit's own draws take their place, under the same camera and light.
        if let Some(kit) = &self.kit {
            list.items.clear();
            self.kit_drawn = push_kit_draws(&mut list, &resident, kit);
        }
        // Third person: behind the human seat's facing, over its shoulder, at the rival.
        if let (Some(a), Some(b)) = (dots.first(), dots.get(1)) {
            let view = ShoulderView {
                human_m: a.pos_m,
                rival_m: b.pos_m,
                facing_cdeg: self.facing_cdeg,
                shake: self.shake(),
                t_s,
            };
            // The kit's corridor is a rectangle, not a ring: keep the eye inside its walls too.
            let camera = if self.kit.is_some() {
                self.arena
                    .shoulder_camera_in(width, height, view, KIT_EYE_BOUNDS)
            } else {
                self.arena.shoulder_camera(width, height, view)
            };
            list.camera = camera.map_err(|e| MatchError::Camera(e.to_string()))?;
        }
        self.cam = [
            list.camera.eye.x,
            list.camera.eye.y,
            list.camera.eye.z,
            list.camera.target.x,
            list.camera.target.y,
            list.camera.target.z,
        ];
        self.drawn = push_fight_draws(&mut list, &resident, &dots)
            .map_err(|e| MatchError::Bodies(e.to_string()))?;
        Ok(list)
    }
}
