//! DOOR1 Accept: a scripted session builds floor, player, enemy and goal, then plays the
//! level and reads a win or a loss. The same level is played twice, so the verdict is a
//! function of the level and the seed, not of the run.
//!
//! Run: `cargo test -p engine-endpoint --features "mcp,rules" --test it game_build`.
//! Units: meters in the brief, ticks at 60 Hz. Determinism: same level twice, same digest.

#![cfg(all(feature = "mcp", feature = "rules"))]

use engine_editor::capability::Capability;
use engine_editor::commands::{Document, Journal};
use engine_endpoint::build_level::{BuildLevelArgs, BuildLevelOutput, BuildLevelTool};
use engine_endpoint::game_play::{GamePlayArgs, GamePlayOutput, GamePlayTool, TOOL_GAME_PLAY};
use engine_endpoint::mcp::{McpServer, SceneReadArgs, SceneReadTool};
use rmcp::handler::server::router::tool::SyncTool;
use rmcp::handler::server::router::tool::ToolBase;

const SESSION: u64 = 7;

fn new_server() -> McpServer {
    McpServer::with_document(
        Document::new(),
        Journal::new(),
        SESSION,
        Capability::unrestricted(SESSION),
    )
}

fn try_build(server: &McpServer, brief: &str, key: u64) -> Result<BuildLevelOutput, String> {
    let rev = SceneReadTool::invoke(
        server,
        SceneReadArgs {
            limit: Some(64),
            ..SceneReadArgs::default()
        },
    )
    .expect("the scene reads")
    .revision;
    BuildLevelTool::invoke(
        server,
        BuildLevelArgs {
            brief: brief.to_owned(),
            seed: 2040,
            expected_revision: rev,
            idempotency_key: key,
        },
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

fn build(server: &McpServer, brief: &str, key: u64) -> BuildLevelOutput {
    try_build(server, brief, key).unwrap_or_else(|error| panic!("the level builds: {error}"))
}

fn try_play(server: &McpServer, seed: u64) -> Result<GamePlayOutput, String> {
    GamePlayTool::invoke(
        server,
        GamePlayArgs {
            seed,
            ticks: None,
            scene_rev: None,
        },
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

fn play(server: &McpServer, seed: u64) -> GamePlayOutput {
    try_play(server, seed).unwrap_or_else(|error| panic!("the game plays: {error}"))
}

/// The player on the west wall and the goal on the east wall: the player can walk there.
const PLAYER_AND_GOAL: &str = "prop player hero\n\
                               want hero next_to wall:west [0,60]\n\
                               prop goal keep\n\
                               want keep next_to wall:east [0,60]\n\
                               spawn hero";

/// The enemy stands in front of the player and the goal sits behind it, so the chase
/// catches the player on the way: the level reads as a loss.
const PLAYER_ENEMY_GOAL: &str = "prop player hero\n\
                                want hero next_to wall:west [0,60]\n\
                                prop enemy brute\n\
                                want brute next_to object:hero [100,150]\n\
                                prop goal keep\n\
                                want keep next_to wall:east [0,60]\n\
                                spawn hero";

#[test]
fn a_scripted_session_builds_floor_player_enemy_and_goal_then_plays_it() {
    let door = new_server();
    // 1. Build the level: player, goal and the enemy that guards the way.
    let built = build(&door, PLAYER_ENEMY_GOAL, 11);
    assert_eq!(built.verdict, "pass", "findings: {:?}", built.findings);
    assert!(built.solved);
    let names: Vec<&str> = built
        .placements
        .iter()
        .map(|placement| placement.name.as_str())
        .collect();
    for want in ["hero", "keep", "brute"] {
        assert!(
            names.contains(&want),
            "the level placed `{want}`: {names:?}"
        );
    }

    // 2. Play it. Every game part is seated where the level put it, and the verdict is
    //    one of the two outcomes the row asks for.
    let first = play(&door, 2040);
    assert!(
        first.outcome == "win" || first.outcome == "lose",
        "the match read a verdict: {}",
        first.summary
    );
    assert!(first.ticks > 0, "the verdict took ticks: {}", first.summary);
    let roles: Vec<&str> = first.parts.iter().map(|part| part.role.as_str()).collect();
    assert_eq!(roles, vec!["enemy", "goal", "player"], "the parts played");

    // 3. The same level and seed give the same verdict and digest, twice.
    let again = play(&door, 2040);
    assert_eq!(again.outcome, first.outcome);
    assert_eq!(again.digest, first.digest);
    assert_eq!(again.ticks, first.ticks);

    // 4. A level with no enemy at all exercises the win path directly: the player walks
    //    into the goal and the door answers a win.
    let open = new_server();
    let built = build(&open, PLAYER_AND_GOAL, 12);
    assert_eq!(built.verdict, "pass", "findings: {:?}", built.findings);
    let won = play(&open, 2040);
    assert_eq!(won.outcome, "win", "{}", won.summary);
    assert!(won.player_hp > 0, "the player lived: {}", won.summary);
    assert_eq!(
        won.ticks,
        play(&open, 2040).ticks,
        "the walk is not seed-driven"
    );
}

#[test]
fn a_guarded_level_reads_a_loss_when_the_clock_runs_out() {
    // The objective has a time limit: reaching the goal is the win, and running out of
    // ticks short of it is the loss. Five ticks is not enough to cross the room, so the
    // door answers a loss with the player still alive and short of the goal.
    let door = new_server();
    let built = build(&door, PLAYER_AND_GOAL, 13);
    assert_eq!(built.verdict, "pass", "findings: {:?}", built.findings);
    let lost = GamePlayTool::invoke(
        &door,
        GamePlayArgs {
            seed: 2040,
            ticks: Some(5),
            scene_rev: None,
        },
    )
    .expect("the game plays");
    assert_eq!(lost.outcome, "lose", "{}", lost.summary);
    assert!(lost.player_hp > 0, "the player lived: {}", lost.summary);
    assert!(
        lost.player_goal_cm > 0,
        "the player was still short of the goal: {}",
        lost.summary
    );
    // The same level with the full clock wins, so the loss is the clock's, not the level's.
    assert_eq!(play(&door, 2040).outcome, "win");
}

#[test]
fn game_play_refuses_before_a_level_is_built_and_on_bad_args() {
    let door = new_server();
    // Nothing built yet: the refusal names the verb to call first.
    let none = try_play(&door, 2040).expect_err("nothing to play");
    assert!(none.starts_with("no_game"), "{none}");
    assert!(none.contains("level.build"), "{none}");

    // A level with no goal refuses by name.
    let built = build(&door, "prop player hero\nspawn hero", 21);
    assert_eq!(built.verdict, "pass", "findings: {:?}", built.findings);
    let no_goal = try_play(&door, 2040).expect_err("no goal");
    assert!(no_goal.starts_with("no_goal"), "{no_goal}");
    assert!(no_goal.contains("prop goal"), "{no_goal}");

    // A tick count outside the bound refuses with the legal range.
    let bad = GamePlayTool::invoke(
        &door,
        GamePlayArgs {
            seed: 2040,
            ticks: Some(0),
            scene_rev: None,
        },
    )
    .expect_err("zero ticks");
    assert_eq!(bad.code, "bad_ticks");

    // A stale revision refuses and names the level's own revision.
    let target = new_server();
    build(&target, PLAYER_AND_GOAL, 22);
    let stale = GamePlayTool::invoke(
        &target,
        GamePlayArgs {
            seed: 2040,
            ticks: None,
            scene_rev: Some(0),
        },
    )
    .expect_err("stale");
    assert_eq!(stale.code, "stale_scene_rev");
}

#[test]
fn the_verb_is_in_the_router_and_says_what_it_does() {
    let names = engine_endpoint::mcp::tool_names();
    assert!(
        names.contains(&TOOL_GAME_PLAY),
        "game.play is reachable by name"
    );
    let description = GamePlayTool::description().expect("describes itself");
    for word in ["goal", "player", "enemy", "win", "lose"] {
        assert!(
            description.contains(word),
            "the description names `{word}`: {description}"
        );
    }
}
