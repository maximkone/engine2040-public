//! Public-benchmark task catalog: the 30 stable task descriptors behind the
//! HS20 mirror, one row per task with its suite, score bucket and
//! public-suite shape.
//!
//! Consumer: `bench_mirror` joins this catalog with the live mirror rows, so
//! the score table and the execution can never disagree on task ids. Owner:
//! the three slice modules own the tasks and their run code; this module owns
//! only the joined descriptor view and copies each shape string from the
//! slice table (no second text to drift). Units: counts of tasks.
//! Determinism: descriptors are `&'static str` reads of the slice tables; the
//! same build lists the same 30 rows.

use crate::mirror_kit::MirrorTask;
use crate::{mirror_craft, mirror_gdb, mirror_logic};

/// Tasks in the mirror; the proof expects three slices of ten.
pub const TASK_COUNT: usize = 30;
/// Suites behind the mirror.
pub const SUITE_COUNT: usize = 3;
/// Tasks per suite.
pub const SUITE_TASKS: usize = 10;

/// One mirrored task descriptor: identity and shape only, no run code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TaskDef {
    /// Stable task id (`gdb01`-`gdb10`, `cr01`-`cr10`, `lg01`-`lg10`).
    pub id: &'static str,
    /// Suite name as published (`GameDevBench`, `CraftBench-UE`,
    /// `GameLogicBench`).
    pub suite: &'static str,
    /// Score bucket inside the suite.
    pub category: &'static str,
    /// Public-suite shape this task mirrors, in a few words.
    pub shape: &'static str,
}

/// Copy one slice's descriptors into the catalog at `start`.
fn copy_slice(
    out: &mut [TaskDef; TASK_COUNT],
    start: usize,
    suite: &'static str,
    slice: &[MirrorTask],
) {
    for (offset, task) in slice.iter().enumerate() {
        out[start + offset] = TaskDef {
            id: task.id,
            suite,
            category: task.category,
            shape: task.shape,
        };
    }
}

/// All 30 descriptors in report order: ten per slice, suite order matching
/// the mirror aggregate.
pub fn tasks() -> [TaskDef; TASK_COUNT] {
    let blank = TaskDef {
        id: "",
        suite: "",
        category: "",
        shape: "",
    };
    let mut out = [blank; TASK_COUNT];
    copy_slice(&mut out, 0, mirror_gdb::SUITE.name, &mirror_gdb::TASKS);
    copy_slice(
        &mut out,
        SUITE_TASKS,
        mirror_craft::SUITE.name,
        &mirror_craft::TASKS,
    );
    copy_slice(
        &mut out,
        2 * SUITE_TASKS,
        mirror_logic::SUITE.name,
        &mirror_logic::TASKS,
    );
    out
}

/// Descriptor for one task id, or `None` for an id the mirror never runs.
pub fn task(id: &str) -> Option<TaskDef> {
    tasks().into_iter().find(|task| task.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_holds_thirty_tasks_ten_per_suite() {
        let catalog = tasks();
        assert_eq!(catalog.len(), TASK_COUNT);
        for suite in [
            mirror_gdb::SUITE.name,
            mirror_craft::SUITE.name,
            mirror_logic::SUITE.name,
        ] {
            let count = catalog.iter().filter(|task| task.suite == suite).count();
            assert_eq!(count, SUITE_TASKS, "{suite} must list ten tasks");
        }
        assert_eq!(SUITE_COUNT * SUITE_TASKS, TASK_COUNT);
    }

    #[test]
    fn task_ids_are_unique_and_fully_described() {
        let catalog = tasks();
        let mut ids: Vec<&str> = catalog.iter().map(|task| task.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TASK_COUNT, "a task id is used twice");
        for task in &catalog {
            assert!(!task.id.is_empty(), "every task names its id");
            assert!(!task.category.is_empty(), "every task names its bucket");
            assert!(!task.shape.is_empty(), "every task names its shape");
        }
    }

    #[test]
    fn lookup_finds_every_task_and_refuses_unknown_ids() {
        for def in tasks() {
            assert_eq!(task(def.id), Some(def), "lookup must return {}", def.id);
        }
        assert_eq!(task("zzz"), None, "unknown ids refuse with None");
        assert_eq!(task(""), None, "the blank id refuses with None");
    }
}
