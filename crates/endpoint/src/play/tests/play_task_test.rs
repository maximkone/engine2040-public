//! play_task tests: the task-shaped `play.run` answer (AX28) — a client that
//! declares the tasks extension gets a task with a slice-wise status and the
//! blocking run's digest, a client that does not gets the start answer byte
//! for byte.

// Declared from `play.rs`'s `tests` module (`#[path]`), so the fixtures in
// that module are in scope and `play.rs` stays inside the line cap.
use super::*;
use crate::play_task::{wants_tasks, TASKS_CAPABILITY};

/// AX28: a task-shaped `play.run` — a client that declares
/// `io.modelcontextprotocol/tasks` starts a 600-tick match and reads it as
/// a task (`tick 240/600` after the first poll, `tick 480/600` after the
/// second, the result on the third), while a client that does not declare
/// it gets the start answer byte for byte. The task digest is the blocking
/// run's, and starting, polling and cancelling a task never touch the
/// scene revision.
#[test]
fn task_play_run_returns_a_task_only_for_a_tasks_client() {
    // The exact extension name, once, is the declaration; a near miss is
    // not.
    assert!(wants_tasks(&[TASKS_CAPABILITY]));
    assert!(wants_tasks(&["roots", "sampling", TASKS_CAPABILITY]));
    assert!(!wants_tasks(&[]));
    assert!(!wants_tasks(&["tasks"]));
    assert!(!wants_tasks(&["io.modelcontextprotocol"]));

    // Two more sessions for the byte-equality check below, built before
    // the local `server` shadows the helper.
    let (plain_twin, task_twin) = (server(), server());
    let server = server();
    let scene_before = server.lock().expect("lock").doc.revision;
    let args = start_args(600);

    // A task start returns the handle at once and reports no ticks yet.
    let started = server
        .lock()
        .expect("lock")
        .play_run_for(&args, true)
        .expect("task start validates");
    let task_id = started.job_id;
    assert_eq!(started.status, "running");
    let fresh = server
        .lock()
        .expect("lock")
        .jobs
        .task_for(task_id)
        .expect("the task reads back");
    assert_eq!(fresh.task_id, task_id);
    assert_eq!(fresh.status, "tick 0/600");
    assert!(!fresh.done);

    // First task poll: one 240-tick slice, no result yet.
    let first = server
        .lock()
        .expect("lock")
        .play_run_for(&run_action("poll", Some(task_id)), true)
        .expect("task poll runs");
    assert_eq!(first.progress, 0.4);
    assert_eq!(first.status, "running");
    let mid = server
        .lock()
        .expect("lock")
        .jobs
        .task_for(task_id)
        .expect("the task reads back");
    assert_eq!(mid.status, "tick 240/600");
    assert!(!mid.done);

    // Second poll: the second slice; third poll: the result.
    let second = server
        .lock()
        .expect("lock")
        .play_run_for(&run_action("poll", Some(task_id)), true)
        .expect("second task poll runs");
    assert_eq!(second.progress, 0.8);
    assert_eq!(
        server
            .lock()
            .expect("lock")
            .jobs
            .task_for(task_id)
            .expect("the task reads back")
            .status,
        "tick 480/600"
    );
    let done = server
        .lock()
        .expect("lock")
        .play_run_for(&run_action("poll", Some(task_id)), true)
        .expect("third task poll completes");
    assert_eq!(done.status, "completed");
    let finished = server
        .lock()
        .expect("lock")
        .jobs
        .task_for(task_id)
        .expect("the task reads back");
    assert_eq!(finished.status, "completed");
    assert!(finished.done);
    let task_digest = done.final_hash.expect("the task carries the digest");

    // The same config and empty tape on the plain path gives that digest.
    let plain = server
        .lock()
        .expect("lock")
        .play_run(&args)
        .expect("plain start validates");
    assert_eq!(plain.progress, 0.0);
    let plain_id = plain.job_id;
    server
        .lock()
        .expect("lock")
        .play_run(&run_action("poll", Some(plain_id)))
        .expect("plain progress poll");
    let plain_done = server
        .lock()
        .expect("lock")
        .play_run(&run_action("poll", Some(plain_id)))
        .expect("plain completing poll");
    assert_eq!(plain_done.status, "completed");
    assert_eq!(
        Some(task_digest),
        plain_done.final_hash,
        "the task digest is the blocking run's digest"
    );

    // The start answer is byte-equal with and without the declaration: the
    // extension moves the poll cadence, never the tool shape.
    let plain_start = plain_twin
        .lock()
        .expect("lock")
        .play_run_for(&args, false)
        .expect("plain task-shaped start");
    let task_start = task_twin
        .lock()
        .expect("lock")
        .play_run_for(&args, true)
        .expect("task-shaped start");
    assert_eq!(
        plain_start, task_start,
        "a start answer is equal either way"
    );

    // A cancel ends the task `cancelled` and still moves no revision.
    let running = server
        .lock()
        .expect("lock")
        .play_run_for(&args, true)
        .expect("second task start validates");
    let running_id = running.job_id;
    let stopped = server
        .lock()
        .expect("lock")
        .play_run_for(&run_action("cancel", Some(running_id)), true)
        .expect("task cancel stops it");
    assert_eq!(stopped.status, "cancelled");
    let cancelled = server
        .lock()
        .expect("lock")
        .jobs
        .task_for(running_id)
        .expect("the task reads back");
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.done);
    assert!(
        server.lock().expect("lock").jobs.task_for(9999).is_none(),
        "an unknown task id reads back as None"
    );
    assert_eq!(
        server.lock().expect("lock").doc.revision,
        scene_before,
        "a task start, poll and cancel never touch the scene"
    );
    println!(
        "task_play_run: 600-tick task tick 240/600, digest equal to the blocking \
             run, cancel ends cancelled with the revision unchanged PASS"
    );
}
