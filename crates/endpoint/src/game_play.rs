//! `game.play`: play the level a session built and read a win or a loss.
//!
//! The door builds a game in two steps: `level.build` places the floor, the walls, the
//! player, the enemy and the goal (kinds that carry a [`Role`]) and keeps those parts on
//! the session, and this verb plays them. It seeds the reference arena from the authored
//! poses and runs the objective (`engine_rules::objective`): reaching the goal while alive
//! wins, dying loses, every enemy walks at the player and swings in reach. Read-only: no
//! document write, no revision move.
//!
//! Units: metres in the payload (the scene's unit), ticks at 60 Hz, millimetres inside
//! the objective. Determinism: the same scene revision, seed and tick count give the same
//! verdict and the same digest; no wall clock, no provider. Errors: `no_game` (nothing
//! built yet), `no_goal`, `stale_scene_rev`, `bad_ticks`, `play_refused`.

use engine_rules::objective::{GoalZone, ObjectiveMatch, ObjectiveOutcome};
use engine_rules::{Limits, Rules};
use rmcp::handler::server::router::tool::ToolBase;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::build_level::{GamePart, Role};
use super::mcp::ServerState;
use crate::mcp::{McpServer, McpToolError};

/// Tool: play the built level and read the outcome.
pub const TOOL_GAME_PLAY: &str = "game.play";
/// Default and cap for one play proof: 900 ticks (15 s at 60 Hz).
pub const MAX_GAME_TICKS: u32 = 900;
/// Player seat in the reference arena.
const PLAYER_SLOT: u8 = 0;

/// Args for `game.play`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GamePlayArgs {
    /// Seed for the match (the same seed twice gives the same verdict).
    pub seed: u64,
    /// Ticks to play at most (default and cap 900).
    pub ticks: Option<u32>,
    /// Scene revision to play; must be the current one.
    pub scene_rev: Option<u64>,
}

/// One game part as the level placed it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GamePartDto {
    /// Role: `player`, `enemy`, `goal`, `floor` or `wall`.
    pub role: String,
    /// Prop name from the brief.
    pub name: String,
    /// Live document id (UUID text).
    pub id: String,
    /// Position meters.
    pub x: f32,
    /// Position meters.
    pub z: f32,
    /// East-west size meters.
    pub sx: f32,
    /// North-south size meters.
    pub sz: f32,
}

impl From<&GamePart> for GamePartDto {
    fn from(part: &GamePart) -> Self {
        Self {
            role: part.role.as_str().to_owned(),
            name: part.name.clone(),
            id: part.id.clone(),
            x: part.x,
            z: part.z,
            sx: part.sx,
            sz: part.sz,
        }
    }
}

/// Result of `game.play`: the parts it played, the verdict and the numbers behind it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GamePlayOutput {
    /// `win`, `lose` or `running` (the tick cap ran out undecided).
    pub outcome: String,
    /// Ticks played.
    pub ticks: u32,
    /// Objective digest (16 hex digits).
    pub digest: String,
    /// The parts the level contributed, in role order then name.
    pub parts: Vec<GamePartDto>,
    /// Player health at the end, permille.
    pub player_hp: u16,
    /// Player distance to the goal centre, centimetres.
    pub player_goal_cm: i32,
    /// One line a human reads: what happened and what to change.
    pub summary: String,
}

/// The parts a `game.play` reads: the session's newest build, or a refusal naming what to
/// build first.
fn parts_or_refuse(state: &ServerState) -> Result<&[GamePart], McpToolError> {
    if state.game_parts.is_empty() {
        return Err(McpToolError::new(
            "no_game",
            "the session has built no game yet; call `level.build` with kinds floor, wall, \
             player, enemy and goal, then play it",
        ));
    }
    Ok(&state.game_parts)
}

/// Play the built level: seed the arena from the authored poses and run the objective.
pub(crate) fn game_play(
    server: &McpServer,
    args: &GamePlayArgs,
) -> Result<GamePlayOutput, McpToolError> {
    let ticks = args.ticks.unwrap_or(MAX_GAME_TICKS);
    if ticks == 0 || ticks > MAX_GAME_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("ticks {ticks} is outside 1..={MAX_GAME_TICKS} (60 ticks = one second)"),
        ));
    }
    server.dispatch(TOOL_GAME_PLAY, |state| {
        if let Some(rev) = args.scene_rev {
            if rev != state.doc.revision || rev != state.game_rev {
                return Err(McpToolError::new(
                    "stale_scene_rev",
                    format!(
                        "scene_rev {rev} is not the level this session built (revision {}); \
                         read the scene, build again, then play",
                        state.game_rev
                    ),
                ));
            }
        }
        let parts = parts_or_refuse(state)?;
        let goal = parts
            .iter()
            .find(|part| part.role == Role::Goal)
            .ok_or_else(|| {
                McpToolError::new(
                    "no_goal",
                    "the level holds no goal; add `prop goal <name>` to the brief so the \
                     player has a zone to reach",
                )
            })?;
        let zone = zone_of(goal);
        // Every enemy on the level chases: seat them all on the arena's slots after the
        // player, so a level with three enemies is a fight the player can lose.
        let enemies: Vec<u8> = (1..=engine_rules::MAX_BODIES.saturating_sub(1))
            .map(|slot| u8::try_from(slot).unwrap_or(u8::MAX))
            .collect();
        let fighters = 2.max(parts.iter().filter(|part| part.role == Role::Enemy).count() + 1);
        let mut match_ =
            ObjectiveMatch::new(args.seed, zone, &enemies, fighters).with_autopilot(true);
        let mut seats: Vec<(u8, i32, i32)> = Vec::new();
        if let Some(player) = parts.iter().find(|part| part.role == Role::Player) {
            seats.push((PLAYER_SLOT, m_to_mm(player.x), m_to_mm(player.z)));
        }
        for (index, enemy) in parts
            .iter()
            .filter(|part| part.role == Role::Enemy)
            .enumerate()
        {
            let slot = u8::try_from(index + 1).unwrap_or(u8::MAX);
            seats.push((slot, m_to_mm(enemy.x), m_to_mm(enemy.z)));
        }
        let placed = match_.place_actors(&seats);
        let mut rules = Rules::new(Limits::default());
        match_
            .run_until_decided(ticks, &[], &mut rules)
            .map_err(|error| {
                McpToolError::new(
                    "play_refused",
                    format!("the rules refused a tick: {error}; the level still stands"),
                )
            })?;
        let player_view = match_.player_view().copied();
        let outcome = match_.outcome();
        let player_goal_cm =
            player_view.map_or(-1, |view| goal_distance_cm(zone, view.x_mm, view.z_mm));
        Ok(GamePlayOutput {
            outcome: outcome.as_str().to_owned(),
            ticks: match_.ticks(),
            digest: format!("{:016x}", match_.digest()),
            parts: parts.iter().map(GamePartDto::from).collect(),
            player_hp: player_view.map_or(0, |view| view.hp_permille),
            player_goal_cm,
            summary: summary_of(outcome, match_.ticks(), placed, player_goal_cm),
        })
    })
}

/// The goal zone a placed goal part describes: its footprint, inclusive on all sides.
fn zone_of(goal: &GamePart) -> GoalZone {
    let half_x = goal.sx.abs() * 0.5;
    let half_z = goal.sz.abs() * 0.5;
    GoalZone::new(
        m_to_mm(goal.x - half_x),
        m_to_mm(goal.z - half_z),
        m_to_mm(goal.x + half_x),
        m_to_mm(goal.z + half_z),
    )
}

/// Distance from a point to the goal centre, centimetres.
fn goal_distance_cm(zone: GoalZone, x_mm: i32, z_mm: i32) -> i32 {
    let (west, north, east, south) = zone.rect();
    let cx = i64::from(west.saturating_add(east) / 2);
    let cz = i64::from(north.saturating_add(south) / 2);
    let dx = i64::from(x_mm) - cx;
    let dz = i64::from(z_mm) - cz;
    i32::try_from((dx * dx + dz * dz).isqrt() / 10).unwrap_or(i32::MAX)
}

/// One human line: the verdict, how long it took and what a caller can change.
fn summary_of(outcome: ObjectiveOutcome, ticks: u32, placed: u32, player_goal_cm: i32) -> String {
    let head = match outcome {
        ObjectiveOutcome::Win => format!(
            "win at tick {ticks}: the player stood in the goal zone ({player_goal_cm} cm \
             from its centre)"
        ),
        ObjectiveOutcome::Lose if player_goal_cm > 0 => format!(
            "lose at tick {ticks}: the goal went unreached {player_goal_cm} cm away; the \
             player died or ran out of ticks, so bring the goal closer, clear the way or \
             add ticks"
        ),
        ObjectiveOutcome::Lose => {
            format!("lose at tick {ticks}: the player was hit down at the goal line")
        }
        ObjectiveOutcome::Running => format!(
            "running after {ticks} ticks: the player is {player_goal_cm} cm from the goal \
             centre; give it more ticks or move the goal closer"
        ),
    };
    format!("{head}; started from {placed} level seat(s)")
}

/// Metres to millimetres via the shared metres-to-cm owner (`engine_spatial::index::to_cm`),
/// so the door and the spatial checks land on the same integer for a level pose.
fn m_to_mm(meters: f32) -> i32 {
    engine_spatial::to_cm(meters).saturating_mul(10)
}

/// `game.play`: play the level this session built and read a win or a loss.
pub struct GamePlayTool;

impl ToolBase for GamePlayTool {
    type Parameter = GamePlayArgs;
    type Output = GamePlayOutput;
    type Error = McpToolError;

    fn name() -> std::borrow::Cow<'static, str> {
        TOOL_GAME_PLAY.into()
    }

    fn description() -> Option<std::borrow::Cow<'static, str>> {
        Some(
            "Play the level this session built and read a win or a loss. Build it first with \
             `level.build` using the kinds floor, wall, player, enemy and goal; this verb seeds \
             the arena from the placed seats and runs the objective: the player wins by \
             standing in the goal zone while alive, loses when it dies, and every enemy walks \
             at it and swings in reach. Read-only; the same scene revision, seed and ticks give \
             the same verdict and digest. Answers outcome win/lose/running, the parts it played, \
             the player's health and its distance to the goal."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        let mut annotations = rmcp::model::ToolAnnotations::default();
        annotations.read_only_hint = Some(true);
        annotations.open_world_hint = Some(false);
        Some(annotations)
    }
}

impl rmcp::handler::server::router::tool::SyncTool<McpServer> for GamePlayTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        game_play(service, &param)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(role: Role, x: f32, z: f32, sx: f32, sz: f32) -> GamePart {
        GamePart {
            role,
            name: role.as_str().to_owned(),
            id: "id".to_owned(),
            x,
            z,
            sx,
            sz,
        }
    }

    #[test]
    fn metres_convert_to_millimetres_and_never_panic() {
        assert_eq!(m_to_mm(0.0), 0);
        assert_eq!(m_to_mm(1.5), 1_500);
        assert_eq!(m_to_mm(-2.25), -2_250);
        assert_eq!(m_to_mm(f32::NAN), 0);
        assert_eq!(m_to_mm(f32::INFINITY), 0);
    }

    #[test]
    fn a_goal_part_becomes_its_footprint_zone() {
        let zone = zone_of(&part(Role::Goal, 2.0, -1.0, 2.0, 1.0));
        assert_eq!(zone.rect(), (1_000, -1_500, 3_000, -500));
        assert!(zone.contains(2_000, -1_000));
        assert!(!zone.contains(3_001, -1_000));
        assert_eq!(goal_distance_cm(zone, 2_000, -1_000), 0);
        assert_eq!(goal_distance_cm(zone, 3_000, -1_000), 100);
    }

    #[test]
    fn every_summary_arm_names_the_verdict_and_the_seats() {
        let win = summary_of(ObjectiveOutcome::Win, 42, 3, 10);
        assert!(win.starts_with("win at tick 42"), "{win}");
        assert!(win.contains("3 level seat"), "{win}");
        let lose = summary_of(ObjectiveOutcome::Lose, 90, 2, 900);
        assert!(lose.starts_with("lose at tick 90"), "{lose}");
        assert!(lose.contains("move the goal closer"), "{lose}");
        let running = summary_of(ObjectiveOutcome::Running, 900, 2, 1_200);
        assert!(running.starts_with("running after 900 ticks"), "{running}");
    }

    #[test]
    fn the_verb_names_itself() {
        assert_eq!(GamePlayTool::name(), TOOL_GAME_PLAY);
        assert_eq!(TOOL_GAME_PLAY, "game.play");
    }
}
