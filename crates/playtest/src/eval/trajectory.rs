//! AD23 trajectory checks: an eval task can require a call order (a read
//! before a write), and a write that names an object id no earlier read
//! returned fails as `ungrounded: <id>`.
//!
//! Consumer: the authoring rows of the EV1 eval (the apply tasks run their
//! trajectory through [`task_trajectory`]) and the harness rows that follow.
//! Owner: the door executor logs each call and the ids a read returned; this
//! module only judges the log. Determinism: the same log gives the same
//! answer; no clock, no map order.

use super::*;
use crate::door::{fixture_scene, set_label, Call, CallKind, Door, Submit};

impl EvalTask {
    /// Call order this task requires of its trajectory; empty means any.
    /// A clean apply reads the scene first, then writes ids the read returned.
    pub(crate) const fn required_calls(&self) -> &'static [CallKind] {
        match (self.flow, self.script) {
            (Flow::Apply, TaskScript::Clean) => &[CallKind::Read, CallKind::Write],
            _ => &[],
        }
    }
}

/// Why a trajectory failed its task: the call order or an id nobody gave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TrajectoryFault {
    /// The required call never came in its place in the order.
    Order(CallKind),
    /// A write named this object id and no earlier read returned it.
    Ungrounded(u64),
}

impl fmt::Display for TrajectoryFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Order(CallKind::Read) => write!(f, "order: a read must come first"),
            Self::Order(CallKind::Write) => write!(f, "order: a write must follow the read"),
            Self::Ungrounded(id) => write!(f, "ungrounded: {id}"),
        }
    }
}

/// Check a door call log against a task: the `required` kinds must appear in
/// that order, and every id a write names must have been returned by an
/// earlier read. Order is judged first. Pure: the same log gives the same
/// answer.
pub(crate) fn check_trajectory(required: &[CallKind], log: &[Call]) -> Result<(), TrajectoryFault> {
    let mut wanted = required.iter().peekable();
    for call in log {
        if wanted.peek().is_some_and(|kind| **kind == call.kind()) {
            wanted.next();
        }
    }
    if let Some(missing) = wanted.next() {
        return Err(TrajectoryFault::Order(*missing));
    }
    let mut seen = std::collections::BTreeSet::new();
    for call in log {
        match call {
            Call::Read { ids } => seen.extend(ids.iter().copied()),
            Call::Write { ids } => {
                if let Some(ghost) = ids.iter().find(|id| !seen.contains(*id)) {
                    return Err(TrajectoryFault::Ungrounded(*ghost));
                }
            }
        }
    }
    Ok(())
}

/// The trajectory behind a task on the fixture scene: read the scene, then
/// relabel the first prop the read returned. Tasks that require no call
/// order pass without a scene.
pub(super) fn task_trajectory(
    task: &EvalTask,
    index: u64,
    backend: Backend,
) -> Result<(), TrajectoryFault> {
    let required = task.required_calls();
    if required.is_empty() {
        return Ok(());
    }
    let mut scene = fixture_scene();
    let mut door = Door::new(backend, index);
    let ids = door.read(&scene);
    let actions: Vec<_> = ids
        .first()
        .map(|id| set_label(*id, "relabeled"))
        .into_iter()
        .collect();
    let revision = scene.revision;
    door.submit(&mut scene, &Submit::clean(revision, &actions));
    check_trajectory(required, door.log())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prop ids of the fixture scene, the ids a read returns.
    fn fixture_ids() -> Vec<u64> {
        let scene = fixture_scene();
        scene
            .variant
            .props
            .iter()
            .map(|p| u64::from(p.id))
            .collect()
    }

    /// A door run on the fixture scene: an optional read, then one write
    /// relabeling `target`. Returns the door's call log.
    fn trajectory_run(read_first: bool, target: u64) -> Vec<Call> {
        let mut scene = fixture_scene();
        let mut door = Door::new(Backend::Mock, 0);
        if read_first {
            door.read(&scene);
        }
        let actions = [set_label(target, "relabeled")];
        let revision = scene.revision;
        door.submit(&mut scene, &Submit::clean(revision, &actions));
        door.log().to_vec()
    }

    const READ_THEN_WRITE: [CallKind; 2] = [CallKind::Read, CallKind::Write];

    #[test]
    fn trajectory_read_then_write_passes() {
        let log = trajectory_run(true, fixture_ids()[0]);
        assert_eq!(check_trajectory(&READ_THEN_WRITE, &log), Ok(()));
    }

    #[test]
    fn trajectory_ghost_id_fails_ungrounded() {
        let ghost = fixture_ids().into_iter().max().expect("fixture props") + 1;
        let log = trajectory_run(true, ghost);
        let fault = check_trajectory(&READ_THEN_WRITE, &log).expect_err("a ghost id fails");
        assert_eq!(fault, TrajectoryFault::Ungrounded(ghost));
        assert_eq!(fault.to_string(), format!("ungrounded: {ghost}"));
    }

    #[test]
    fn trajectory_skipped_read_fails_the_order_check() {
        let id = fixture_ids()[0];
        let log = trajectory_run(false, id);
        assert_eq!(
            check_trajectory(&READ_THEN_WRITE, &log),
            Err(TrajectoryFault::Order(CallKind::Read))
        );
        // A read that comes after the write is not a read first either.
        let mut late = log;
        late.push(Call::Read { ids: vec![id] });
        assert_eq!(
            check_trajectory(&READ_THEN_WRITE, &late),
            Err(TrajectoryFault::Order(CallKind::Write))
        );
    }

    #[test]
    fn trajectory_one_flipped_id_bit_flips_the_result() {
        let id = fixture_ids()[0];
        let grounded = trajectory_run(true, id);
        assert_eq!(check_trajectory(&READ_THEN_WRITE, &grounded), Ok(()));
        let flipped = trajectory_run(true, id ^ 0x80);
        assert_eq!(
            check_trajectory(&READ_THEN_WRITE, &flipped),
            Err(TrajectoryFault::Ungrounded(id ^ 0x80))
        );
    }

    #[test]
    fn trajectory_apply_rows_hold_the_order_and_replay_equal() {
        let apply: Vec<&EvalTask> = AUTHORING_TASKS
            .iter()
            .filter(|task| !task.required_calls().is_empty())
            .collect();
        assert_eq!(apply.len(), 2, "a13 and a14 require read then write");
        for task in apply {
            for backend in BACKENDS {
                let first = run_authoring_row(task, 13, backend);
                assert!(first.completed, "{}: {}", task.id, first.detail);
                assert_eq!(first.detail, "apply ok: 1 action landed (simulated)");
                assert_eq!(first, run_authoring_row(task, 13, backend));
            }
        }
    }
}
