//! Rules through the door. A script set with `rules.set` drives fighter slots in
//! `play.run`; a bad script is refused and the last good one stays.
//!
//! Run: `cargo test -p engine-endpoint --features rules --test it rules_door::`.
//! Units: ticks, meters. Determinism: same seed twice gives equal checkpoints.

#![cfg(feature = "rules")]

use engine_editor::capability::Capability;
use engine_endpoint::mcp::McpServer;
use engine_endpoint::packs::rules::{
    RulesExplainArgs, RulesExplainTool, RulesSetArgs, RulesSetTool, RulesTestArgs, RulesTestTool,
};
use engine_endpoint::play::{CheckpointDto, PlayRunArgs, PlayRunTool};
use rmcp::handler::server::router::tool::SyncTool;

const SESSION: u64 = 7;
const TICKS: u32 = 600;
const BASH: &str = include_str!("../../../rules/scripts/shield_bash_arena.rhai");

fn server() -> McpServer {
    McpServer::new(SESSION, Capability::unrestricted(SESSION))
}

fn play(server: &McpServer, seed: u64) -> (Vec<CheckpointDto>, Vec<String>) {
    let start = PlayRunArgs {
        action: "start".to_owned(),
        seed: Some(seed),
        ticks: Some(TICKS),
        fighters: Some(2),
        half_extent_m: Some(12.0),
        inputs: Some(Vec::new()),
        ..PlayRunArgs::default()
    };
    let started = PlayRunTool::invoke(server, start).expect("play.run starts");
    let poll = |id| PlayRunArgs {
        action: "poll".to_owned(),
        job_id: Some(id),
        ..PlayRunArgs::default()
    };
    PlayRunTool::invoke(server, poll(started.job_id)).expect("first poll");
    let done = PlayRunTool::invoke(server, poll(started.job_id)).expect("second poll");
    assert_eq!(done.status, "completed");
    (done.checkpoints, done.logs)
}

fn set(server: &McpServer, source: &str, slots: Option<Vec<u8>>) -> Result<String, String> {
    RulesSetTool::invoke(
        server,
        RulesSetArgs {
            source: source.to_owned(),
            slots,
        },
    )
    .map(|out| out.hash)
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[test]
fn rules_set_makes_play_run_differ_replay_and_refuse_bad_scripts() {
    let server = server();
    let plain = play(&server, 2040).0;
    assert_eq!(plain.len(), TICKS as usize);

    // `rules.set` of the shield bash: a scripted slot 0 plays a different match.
    let hash = set(&server, BASH, Some(vec![0])).expect("the shield bash installs");
    // (The job log ring keeps the last 16 lines, so the "rules script ... drives slots" note is
    // evicted by the checkpoint lines of a 600-tick match; it shows on shorter matches.)
    let (scripted, _logs) = play(&server, 2040);
    assert_ne!(scripted, plain, "the scripted slot changed the match");

    // Same seed twice: 600 of 600 checkpoints equal.
    let (again, _) = play(&server, 2040);
    let equal = scripted.iter().zip(&again).filter(|(a, b)| a == b).count();
    assert_eq!(
        equal, TICKS as usize,
        "{equal} of {TICKS} checkpoints equal"
    );

    // A bad edit is refused with a typed code; the last good script stays and still drives.
    for bad in ["let x = 1.5;", "loop { }", "launch_missiles(1);"] {
        let error = set(&server, bad, None).expect_err("a bad script is refused");
        assert!(error.starts_with("rules_refused"), "{error}");
        assert!(
            error.contains(&hash),
            "the refusal names the kept script: {error}"
        );
    }
    assert_eq!(
        play(&server, 2040).0,
        scripted,
        "the last good script is still in force"
    );

    // Naming no slots drives none: the plain match comes back.
    set(&server, BASH, Some(Vec::new())).expect("an empty slot list installs");
    assert_eq!(play(&server, 2040).0, plain);

    // A slot outside the arena is refused before anything runs.
    let error = set(&server, BASH, Some(vec![16])).expect_err("slot 16 does not exist");
    assert!(error.starts_with("bad_slots"), "{error}");
}

#[test]
fn rules_test_and_explain_change_nothing() {
    let server = server();
    // Explain with no script installed names the next step.
    let none = RulesExplainTool::invoke(
        &server,
        RulesExplainArgs {
            tick: 30,
            ..RulesExplainArgs::default()
        },
    )
    .expect_err("nothing to explain yet");
    assert_eq!(none.code, "no_script");

    // `rules.test` dry-runs a script and installs nothing.
    let tested = RulesTestTool::invoke(
        &server,
        RulesTestArgs {
            source: BASH.to_owned(),
        },
    )
    .expect("test runs");
    assert!(tested.ok, "{tested:?}");
    assert!(!tested.cases.is_empty());
    let refused = RulesTestTool::invoke(
        &server,
        RulesTestArgs {
            source: "let x = 0.5;".to_owned(),
        },
    )
    .expect("test reports a refusal as data");
    assert!(!refused.ok);
    assert!(refused.refused.is_some());
    let plain = play(&server, 2040).0;
    assert_eq!(play(&server, 2040).0, plain, "test installed nothing");

    set(&server, BASH, None).expect("installs");
    let scripted = play(&server, 2040).0;
    let explained = RulesExplainTool::invoke(
        &server,
        RulesExplainArgs {
            tick: 30,
            seed: Some(2040),
            fighters: Some(2),
            half_extent_m: Some(12.0),
            scene_rev: None,
        },
    )
    .expect("explains tick 30");
    assert_eq!(explained.tick, 30);
    assert_eq!(explained.slots, vec![0]);
    assert_eq!(explained.fighters.len(), 1, "only the scripted slot");
    assert!(
        !explained.fighters[0].calls.is_empty(),
        "host calls are listed"
    );
    assert_eq!(play(&server, 2040).0, scripted, "explain changed no match");
}
