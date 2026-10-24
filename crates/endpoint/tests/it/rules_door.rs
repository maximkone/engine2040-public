//! Rules through the door. A script set with `rules.set` drives fighter slots in
//! `play.run`; a bad script is refused and the last good one stays.
//!
//! Run: `cargo test -p engine-endpoint --features rules --test it rules_door::`.
//! Units: ticks, meters. Determinism: same seed twice gives equal checkpoints.

#![cfg(feature = "rules")]

use std::cell::RefCell;
use std::rc::Rc;

use engine_arena::spine_step::SpineMatch;
use engine_arena::{ArenaConfig, FighterTape, FIGHTER_RADIUS_M, MAX_PARTICIPANTS};
use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_endpoint::mcp::McpServer;
use engine_endpoint::packs::rules::{
    set_approval_hash, RulesApproval, RulesExplainArgs, RulesExplainTool, RulesGetArgs,
    RulesGetTool, RulesSetArgs, RulesSetTool, RulesTestArgs, RulesTestTool, RulesUndoArgs,
    RulesUndoTool,
};
use engine_endpoint::play::world_load;
use engine_endpoint::play::{CheckpointDto, PlayRunArgs, PlayRunTool};
use engine_rules::arena::RulesController;
use engine_rules::{Limits, Rules};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;
use rmcp::handler::server::router::tool::SyncTool;

const SESSION: u64 = 7;
const TICKS: u32 = 600;
const BASH: &str = include_str!("../../../rules/scripts/shield_bash_arena.rhai");

/// A session at revision 1 whose scene plays as the empty arena: the one node
/// hangs above head height, so it blocks no fighter but gives the run a
/// revision to pin.
fn server() -> McpServer {
    let (mut doc, mut journal) = (Document::new(), Journal::new());
    let marker = Batch {
        ops: vec![Op::Insert {
            id: ObjectId::new(),
            parent: ObjectId::NIL,
            local: Transform::new([0.0, 10.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        }],
        base_revision: 0,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &marker).expect("marker applies");
    McpServer::with_document(doc, journal, SESSION, Capability::unrestricted(SESSION))
}

fn play(server: &McpServer, seed: u64) -> (Vec<CheckpointDto>, Vec<String>) {
    play_at(server, seed, 1)
}

/// [`play`] on the scene revision the caller pinned (a document with a wall
/// sits at revision 2).
fn play_at(server: &McpServer, seed: u64, scene_rev: u64) -> (Vec<CheckpointDto>, Vec<String>) {
    let start = PlayRunArgs {
        action: "start".to_owned(),
        seed: Some(seed),
        ticks: Some(TICKS),
        fighters: Some(2),
        half_extent_m: Some(12.0),
        inputs: Some(Vec::new()),
        scene_rev: Some(scene_rev),
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
    // AD13: an install is Commit class; the helper approves the exact text
    // against whatever revision is installed now.
    let rev = RulesGetTool::invoke(server, RulesGetArgs::default()).map_or(0, |info| info.rev);
    RulesSetTool::invoke(
        server,
        RulesSetArgs {
            source: source.to_owned(),
            slots,
            approval: Some(RulesApproval {
                script_hash: set_approval_hash(source),
                rules_revision: rev,
            }),
            expected_hash: None,
            no_op: None,
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
    let outside = vec![MAX_PARTICIPANTS];
    let error = set(&server, BASH, Some(outside)).expect_err("a slot past the roster is refused");
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

/// A session at revision 2: the marker node of [`server`] plus one 2 m long,
/// 0.4 m thick, 2 m tall wall across the arena center (grounded roots sit at
/// y = 0, the spatial check's floor convention).
fn walled_server() -> (McpServer, u64) {
    let (mut doc, mut journal) = (Document::new(), Journal::new());
    let marker = Batch {
        ops: vec![Op::Insert {
            id: ObjectId::new(),
            parent: ObjectId::NIL,
            local: Transform::new([0.0, 10.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        }],
        base_revision: 0,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &marker).expect("marker applies");
    let wall = Batch {
        ops: vec![Op::Insert {
            id: ObjectId::new(),
            parent: ObjectId::NIL,
            local: Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [2.0, 2.0, 0.4]),
        }],
        base_revision: 1,
        idempotency_key: 2,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &wall).expect("wall applies");
    let rev = doc.revision;
    assert_eq!(rev, 2, "the marker and the wall are two revisions");
    (
        McpServer::with_document(doc, journal, SESSION, Capability::unrestricted(SESSION)),
        rev,
    )
}

#[test]
fn rules_drive_on_a_walled_scene() {
    let (server, rev) = walled_server();
    // The plain walled run, then the same scene with the shield bash on slot 0.
    let plain = play_at(&server, 2040, rev).0;
    assert_eq!(plain.len(), TICKS as usize);
    set(&server, BASH, Some(vec![0])).expect("the shield bash installs");
    let scripted = play_at(&server, 2040, rev).0;
    assert_ne!(
        scripted, plain,
        "the scripted slot changed the walled match"
    );
    // Same seed twice: every checkpoint equal.
    let again = play_at(&server, 2040, rev).0;
    assert_eq!(scripted, again, "same seed twice on the walled scene");

    // The wall still keeps both fighters out while the script drives slot 0:
    // 18 of 20 seeds never put a fighter inside it. Driven through the same
    // controller hook the door installs (`engine_arena::controller`).
    let mut rules = Rules::new(Limits::default());
    assert!(rules.set(BASH, 0).is_ok(), "the shield bash compiles");
    let rules = Rc::new(RefCell::new(rules));
    let mut doc = Document::new();
    doc.hierarchy
        .insert(
            ObjectId::new(),
            ObjectId::NIL,
            Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [2.0, 2.0, 0.4]),
        )
        .expect("wall inserts");
    let world = world_load::match_world(&doc).expect("the walled scene loads");
    assert_eq!(world.obstacles().len(), 1);
    let mut clean = 0;
    for seed in 0..20u64 {
        let config = ArenaConfig::try_new(5000 + seed, TICKS, 2, 12.0).expect("config");
        let controller = RulesController::new(Rc::clone(&rules), &[0]);
        let mut sim = SpineMatch::build(&config, &FighterTape(Vec::new()), world.clone())
            .expect("the walled match builds")
            .with_controller(Box::new(controller));
        let mut inside = false;
        while sim.tick() < TICKS {
            sim.step().expect("steps");
            for t in sim.spine().transforms() {
                inside |= world.overlaps([t.pos[0], t.pos[2]], FIGHTER_RADIUS_M);
            }
        }
        if !inside {
            clean += 1;
        }
    }
    assert!(
        clean >= 18,
        "{clean} of 20 scripted fights kept every fighter out of the wall"
    );
}

#[test]
fn rules_undo_reads_back_and_walks_the_installs() {
    const A: &str = "if stamina > 500 { press(1); }";
    const B: &str = "if stamina <= 500 { press(2); }";
    let server = server();

    // Nothing installed: the read-back names the next step, undo has nothing to walk.
    let none = RulesGetTool::invoke(&server, RulesGetArgs {}).expect_err("nothing installed");
    assert_eq!(none.code, "no_script");
    let empty = RulesUndoTool::invoke(&server, RulesUndoArgs {}).expect_err("one install only");
    assert_eq!(empty.code, "nothing_to_undo");

    let hash_a = set(&server, A, Some(vec![0])).expect("A installs");
    let hash_b = set(&server, B, Some(vec![1])).expect("B installs");
    let read = RulesGetTool::invoke(&server, RulesGetArgs {}).expect("B reads back");
    assert_eq!(read.hash, hash_b, "the read-back is the installed script");
    assert!(read.source.contains("stamina <= 500"), "{read:?}");
    assert_eq!(read.slots, vec![1]);

    let back = RulesUndoTool::invoke(&server, RulesUndoArgs {}).expect("A comes back");
    assert_eq!(back.hash, hash_a, "undo reinstalls A");
    assert!(back.source.contains("stamina > 500"), "{back:?}");
    assert_eq!(back.slots, vec![0], "A brings its slots back");
    let read = RulesGetTool::invoke(&server, RulesGetArgs {}).expect("A reads back");
    assert_eq!(read.hash, hash_a);

    let forward = RulesUndoTool::invoke(&server, RulesUndoArgs {}).expect("B comes back");
    assert_eq!(forward.hash, hash_b, "a second undo walks forward again");
    assert_eq!(forward.slots, vec![1]);
}
