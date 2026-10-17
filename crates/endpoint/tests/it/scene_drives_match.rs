//! One world: an AI's scene edit changes what is played, undo restores it.
//!
//! Consumer: the one-world proof. Data owner: the editor document behind the door;
//! the match world comes from `play::world_load`. Units: meters, ticks.
//! Determinism: same revision and seed give the same 300 checkpoints; the
//! undone scene replays the empty scene's checkpoints exactly. Errors: typed
//! tool codes, asserted by name.

#![cfg(feature = "mcp")]

use engine_arena::spine_step::SpineMatch;
use engine_arena::{ArenaConfig, FighterTape};
use engine_editor::capability::Capability;
use engine_editor::commands::Document;
use engine_endpoint::mcp::{
    McpServer, OpDto, PoseDto, SceneReadArgs, SceneReadTool, TxApplyArgs, TxApplyTool,
    TxPreviewArgs, TxPreviewTool, TxUndoArgs, TxUndoTool,
};
use engine_endpoint::play::world_load::match_world;
use engine_endpoint::play::{CheckpointDto, PlayRunArgs, PlayRunTool};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;
use rmcp::handler::server::router::tool::SyncTool;

const SESSION: u64 = 7;
const TICKS: u32 = 300;
const HALF: f32 = 12.0;

fn server() -> McpServer {
    McpServer::new(SESSION, Capability::unrestricted(SESSION))
}

fn revision_of(server: &McpServer) -> u64 {
    SceneReadTool::invoke(server, SceneReadArgs::default())
        .expect("scene reads")
        .revision
}

/// A 2 m long, 0.4 m thick, 2 m tall wall across the arena center (grounded
/// roots sit at y = 0, the spatial check's floor convention).
fn wall_pose() -> PoseDto {
    PoseDto {
        pos: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: [2.0, 2.0, 0.4],
    }
}

/// A node hanging above head height: it blocks no fighter, so a scene holding
/// only this plays as the empty arena yet has a revision to pin.
fn marker_pose() -> PoseDto {
    PoseDto {
        pos: [0.0, 10.0, 0.0],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

fn insert_wall(server: &McpServer, key: u64) -> u64 {
    insert_node(server, key, wall_pose())
}

fn insert_node(server: &McpServer, key: u64, local: PoseDto) -> u64 {
    let rev = revision_of(server);
    let ops = vec![OpDto::Insert {
        id: ObjectId::new().to_string(),
        parent: ObjectId::NIL.to_string(),
        local,
    }];
    let approval = TxPreviewTool::invoke(
        server,
        TxPreviewArgs {
            expected_revision: rev,
            idempotency_key: key,
            ops: ops.clone(),
        },
    )
    .expect("wall previews")
    .approval;
    TxApplyTool::invoke(
        server,
        TxApplyArgs {
            expected_revision: rev,
            idempotency_key: key,
            ops,
            approval,
        },
    )
    .expect("wall applies")
    .revision
}

fn play(
    server: &McpServer,
    seed: u64,
    scene_rev: Option<u64>,
) -> Result<Vec<CheckpointDto>, String> {
    let start = PlayRunArgs {
        action: "start".to_owned(),
        seed: Some(seed),
        ticks: Some(TICKS),
        fighters: Some(2),
        half_extent_m: Some(HALF),
        inputs: Some(Vec::new()),
        ttl_polls: None,
        job_id: None,
        scene_rev,
    };
    let started = PlayRunTool::invoke(server, start)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let poll = |id| PlayRunArgs {
        action: "poll".to_owned(),
        job_id: Some(id),
        ..PlayRunArgs::default()
    };
    PlayRunTool::invoke(server, poll(started.job_id)).expect("first poll");
    let done = PlayRunTool::invoke(server, poll(started.job_id)).expect("second poll");
    assert_eq!(done.status, "completed");
    assert_eq!(
        done.scene_rev, scene_rev,
        "the receipt echoes the scene revision"
    );
    assert!(
        done.scene_digest.is_some(),
        "the manifest carries the scene pin digest"
    );
    Ok(done.checkpoints)
}

#[test]
fn scene_drives_match() {
    let server = server();

    // An unauthored arena (revision 0) never plays, pinned or not.
    for offered in [None, Some(0)] {
        let refused = play(&server, 2040, offered).expect_err("revision 0 refuses");
        assert!(refused.starts_with("empty_arena"), "{refused}");
    }

    // A run must name the scene it plays.
    let empty_rev = insert_node(&server, 40, marker_pose());
    let unpinned = play(&server, 2040, None).expect_err("an unpinned run refuses");
    assert!(unpinned.starts_with("unpinned_run"), "{unpinned}");

    // A scene with nothing solid is the legacy arena: same checkpoints.
    let empty = play(&server, 2040, Some(empty_rev)).expect("empty scene plays");
    let config = ArenaConfig::try_new(2040, TICKS, 2, HALF).expect("config");
    let legacy = engine_arena::run(&config, &FighterTape(Vec::new()));
    assert_eq!(empty.len(), TICKS as usize);
    for (dto, point) in empty.iter().zip(&legacy) {
        assert_eq!((dto.tick, dto.hash), (point.tick, point.hash));
    }

    // An AI edit: one 2 m wall through the door.
    let walled_rev = insert_wall(&server, 41);
    assert_eq!(walled_rev, empty_rev + 1);
    let walled = play(&server, 2040, Some(walled_rev)).expect("walled scene plays");
    assert_ne!(
        walled.last(),
        empty.last(),
        "revision N must not share a hash with N-1"
    );

    // A stale revision refuses and names the current one.
    let stale = play(&server, 2040, Some(empty_rev)).expect_err("stale revision refuses");
    assert!(stale.starts_with("stale_scene_rev"), "{stale}");
    assert!(
        stale.contains(&format!("current revision {walled_rev}")),
        "{stale}"
    );

    // Same revision and seed twice: 300 of 300 checkpoints equal.
    let again = play(&server, 2040, Some(walled_rev)).expect("replays");
    let equal = walled.iter().zip(&again).filter(|(a, b)| a == b).count();
    assert_eq!(
        equal, TICKS as usize,
        "{equal} of {TICKS} checkpoints equal"
    );

    // Undo restores the old hash exactly (the revision number moves on).
    let undone = TxUndoTool::invoke(&server, TxUndoArgs::default()).expect("undo applies");
    assert_eq!(undone.revision, walled_rev + 1, "revisions never repeat");
    let restored = play(&server, 2040, Some(undone.revision)).expect("restored plays");
    assert_eq!(restored, empty, "undo restores every checkpoint exactly");
}

#[test]
fn no_fighter_ends_up_inside_the_wall_on_twenty_seeds() {
    // The same loader the door uses, over a document holding the same wall.
    let mut doc = Document::new();
    let pose = wall_pose();
    doc.hierarchy
        .insert(
            ObjectId::new(),
            ObjectId::NIL,
            Transform::new(pose.pos, pose.rot, pose.scale),
        )
        .expect("wall inserts");
    let world = match_world(&doc).expect("scene loads");
    assert_eq!(world.obstacles().len(), 1);

    let mut clean = 0;
    for seed in 0..20u64 {
        let config = ArenaConfig::try_new(5000 + seed, TICKS, 2, HALF).expect("config");
        let mut sim = SpineMatch::build(&config, &FighterTape(Vec::new()), world.clone())
            .expect("match builds");
        let mut inside = false;
        while sim.tick() < TICKS {
            sim.step().expect("steps");
            for t in sim.spine().transforms() {
                inside |= world.overlaps([t.pos[0], t.pos[2]], engine_arena::FIGHTER_RADIUS_M);
            }
        }
        if !inside {
            clean += 1;
        }
    }
    assert!(
        clean >= 18,
        "{clean} of 20 fights kept every fighter out of the wall"
    );
}

#[test]
fn a_door_run_equals_a_direct_spine_run_on_the_same_world() {
    // Ties the door to the library: same world, same config, same hashes.
    let server = server();
    let rev = insert_wall(&server, 7);
    let via_door = play(&server, 77, Some(rev)).expect("plays");

    let mut doc = Document::new();
    let pose = wall_pose();
    doc.hierarchy
        .insert(
            ObjectId::new(),
            ObjectId::NIL,
            Transform::new(pose.pos, pose.rot, pose.scale),
        )
        .expect("wall inserts");
    let world = match_world(&doc).expect("loads");
    let config = ArenaConfig::try_new(77, TICKS, 2, HALF).expect("config");
    let direct = engine_arena::spine_step::run_spine(&config, &FighterTape(Vec::new()), &world);
    assert_eq!(via_door.len(), direct.len());
    for (dto, point) in via_door.iter().zip(&direct) {
        assert_eq!((dto.tick, dto.hash), (point.tick, point.hash));
    }
}
