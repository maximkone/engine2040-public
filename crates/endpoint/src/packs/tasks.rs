//! `tasks`: the repo's build-task suite, named behind the door (BG14).
//!
//! Owner: `tools/tasks` — ten folders, each holding a `task.md` (the job a
//! solver reads) and a `referee.mjs` (a programmatic judge: it writes the tree,
//! checks it and exits 0 or 1, with no model and no LLM judge in the loop).
//! Data owner stays that directory; this module owns one read: `tasks.list`
//! names the jobs the suite holds.
//!
//! Consumer: an agent at the door that wants to know what the suite asks for
//! before it picks one. Running a task is not a door verb: the referee is Node
//! code, and `node tools/tasks/run.mjs --selftest` (which also solves the suite
//! with its two arms) is the one thing that calls it. The door lists; the tool
//! judges.
//!
//! The two rules here are the tool's rules, copied on purpose. A task folder is
//! the name shape `run.mjs` walks for, and a title is the first line of
//! `task.md` with a leading `# ` stripped — the same line `--list` prints. Two
//! readers of one suite that disagree about either would be two suites.
//!
//! Units: counts for rows, one line for a title. Determinism: rows follow
//! folder-name order, so the same tree lists the same bytes; no clock, no
//! network, no float, and no absolute path on the wire. Errors: a suite
//! directory that will not read refuses with `tasks_no_suite`, naming where the
//! suite lives; a task folder whose `task.md` will not read refuses with
//! `tasks_bad_task`, naming the folder, because a half-read suite would print a
//! count that is not the suite.

use std::path::{Path, PathBuf};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "tasks";
/// Verb: name every job the build-task suite holds.
pub const TOOL_TASKS_LIST: &str = "tasks.list";

/// The suite as the wire names it: repo-relative, so no build path is printed.
pub const SUITE_REL: &str = "tools/tasks";

/// `tasks.list` takes no argument: the suite is the scope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "a fieldless door argument DTO keeps the brace form its sibling DTOs use, so its JSON schema stays an object"
)]
pub struct TasksListArgs {}

/// One job the suite holds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TaskRowDto {
    /// The folder name, such as `06-slip-meter`.
    pub folder: String,
    /// The first line of its `task.md`, without the leading `# `.
    pub title: String,
    /// True when the folder also holds the `referee.mjs` that judges it.
    pub referee: bool,
}

/// `tasks.list` result: every job, in folder order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TasksListOutput {
    /// The suite, named repo-relative.
    pub suite: String,
    /// One row per task folder, in folder-name order.
    pub tasks: Vec<TaskRowDto>,
    /// How many jobs the suite holds.
    pub count: u32,
}

/// The suite root: `tools/tasks`, beside the crate the door lives in.
#[must_use]
pub fn suite_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("tasks")
}

/// True for a task folder name: two digits, a dash, then a lower-snake slug.
/// This is the shape `tools/tasks/run.mjs` walks for (`/^\d\d-[a-z0-9-]+$/`),
/// so the door and the tool count the same ten folders out of one directory.
#[must_use]
pub fn is_task_folder(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() < 4 || !bytes[..2].iter().all(u8::is_ascii_digit) || bytes[2] != b'-' {
        return false;
    }
    bytes[3..]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// The title of one `task.md`: its first line, a leading `# ` stripped.
fn task_title(text: &str) -> String {
    let first = text.lines().next().unwrap_or_default().trim();
    first.strip_prefix("# ").unwrap_or(first).trim().to_owned()
}

/// Read one suite directory into rows, in folder order.
///
/// `root` is a parameter so a test can judge a suite it wrote itself.
///
/// # Errors
/// [`McpToolError`] `tasks_no_suite` when the directory will not read,
/// `tasks_bad_task` when a task folder has no readable `task.md`.
pub fn list_tasks(root: &Path) -> Result<Vec<TaskRowDto>, McpToolError> {
    let entries = std::fs::read_dir(root).map_err(|error| {
        McpToolError::new(
            "tasks_no_suite",
            format!(
                "no build-task suite at `{}`; next: run the door from the repo checkout, the \
                 suite lives beside the crates (`{SUITE_REL}`) ({error})",
                root.display()
            ),
        )
        .with_data(rmcp::serde_json::json!({ "code": "tasks_no_suite" }))
    })?;
    let mut folders: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_task_folder(name))
        .collect();
    folders.sort();
    let mut rows = Vec::with_capacity(folders.len());
    for folder in folders {
        let task = root.join(&folder).join("task.md");
        let text = std::fs::read_to_string(&task).map_err(|error| {
            McpToolError::new(
                "tasks_bad_task",
                format!(
                    "task `{folder}` has no readable `task.md` at `{}`; next: write the job a \
                     solver reads, its first line is the title ({error})",
                    task.display()
                ),
            )
            .with_data(rmcp::serde_json::json!({ "code": "tasks_bad_task" }))
        })?;
        rows.push(TaskRowDto {
            referee: root.join(&folder).join("referee.mjs").is_file(),
            title: task_title(&text),
            folder,
        });
    }
    Ok(rows)
}

/// `tasks.list` handler: name every job the suite holds.
fn tasks_list(service: &McpServer, _args: &TasksListArgs) -> Result<TasksListOutput, McpToolError> {
    service.dispatch(TOOL_TASKS_LIST, |_state| {
        let tasks = list_tasks(&suite_path())?;
        Ok(TasksListOutput {
            suite: SUITE_REL.to_owned(),
            count: u32::try_from(tasks.len()).unwrap_or(u32::MAX),
            tasks,
        })
    })
}

crate::e2_tool!(
    /// `tasks.list`: every job the build-task suite holds.
    TasksListTool,
    name = TOOL_TASKS_LIST,
    description = "Name every job in the repo's build-task suite: one row per task folder, in folder order, with the title its `task.md` carries and whether the folder holds the `referee.mjs` that judges it. The suite is ten real engine jobs a solver is scored on. This verb lists them and runs nothing: `node tools/tasks/run.mjs --selftest` is the referee. Read-only.",
    readonly = true,
    args = TasksListArgs,
    output = TasksListOutput,
    invoke = tasks_list
);

/// The shipped tasks pack: the one read of the suite.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "tasks: the repo's build-task suite. tasks.list names every job folder in \
         `tools/tasks` with the title its task.md carries and whether a referee sits \
         beside it, in folder order; it lists the ten jobs and runs none of them",
        false,
    );
    pack.add(VerbSpec::of::<TasksListTool>(
        TOOL_TASKS_LIST,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    /// A temp suite root no other test run shares.
    fn scratch(kind: &str) -> PathBuf {
        std::env::temp_dir().join(format!("e2040-tasks-{}-{kind}", std::process::id()))
    }

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    #[test]
    fn the_suite_folder_shape_is_the_shape_the_tool_walks() {
        for name in ["01-door-verb", "06-slip-meter", "10-wasm-gate", "99-a1-b"] {
            assert!(is_task_folder(name), "{name} is a task folder");
        }
        for name in [
            "logs",
            "README.md",
            "lib.mjs",
            "9-foo",
            "0-foo",
            "-01-foo",
            "01_Foo",
            "01-",
            "",
            "run.mjs",
        ] {
            assert!(!is_task_folder(name), "{name} is not a task folder");
        }
    }

    #[test]
    fn pack_registers_the_one_read_with_the_observer_card() {
        let pack = pack().expect("the tasks pack builds");
        assert_eq!(pack.name(), PACK);
        assert_eq!(pack.verb_names(), vec![TOOL_TASKS_LIST]);
        assert!(!pack.default_loaded(), "the default list pays nothing");
        assert_eq!(
            pack.verbs()[0].card(),
            CapabilityCard::Observe,
            "listing the suite spends no capability"
        );
    }

    #[test]
    fn tasks_list_names_the_ten_jobs_the_shipped_suite_holds() {
        let answer = TasksListTool::invoke(&server(), TasksListArgs::default())
            .expect("the repo checkout holds the suite");
        assert_eq!(answer.suite, SUITE_REL);
        assert_eq!(
            answer.count as usize,
            answer.tasks.len(),
            "the count names the rows it printed"
        );
        assert_eq!(answer.count, 10, "the suite is ten jobs");
        let folders: Vec<&str> = answer
            .tasks
            .iter()
            .map(|row| row.folder.as_str())
            .collect::<Vec<&str>>();
        assert_eq!(folders[0], "01-door-verb");
        assert_eq!(folders[9], "10-wasm-gate");
        let mut sorted = folders.clone();
        sorted.sort_unstable();
        assert_eq!(folders, sorted, "rows come in folder order");
        for row in &answer.tasks {
            assert!(row.referee, "{} carries its referee", row.folder);
            assert!(!row.title.is_empty(), "{} carries a title", row.folder);
        }
    }

    #[test]
    fn a_written_suite_lists_in_folder_order_and_ignores_the_rest() {
        let root = scratch("w");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("02-beta")).expect("folder");
        std::fs::create_dir_all(root.join("01-alpha")).expect("folder");
        std::fs::create_dir_all(root.join("logs")).expect("junk folder");
        std::fs::write(
            root.join("02-beta").join("task.md"),
            "# Second job\n\nthe body nobody prints\n",
        )
        .expect("task.md");
        std::fs::write(root.join("02-beta").join("referee.mjs"), "// judge\n").expect("referee");
        std::fs::write(root.join("01-alpha").join("task.md"), "No heading marker\n")
            .expect("task.md");

        let rows = list_tasks(&root).expect("the written suite reads");
        assert_eq!(rows.len(), 2, "`logs` is not a task folder");
        assert_eq!(rows[0].folder, "01-alpha");
        assert_eq!(rows[0].title, "No heading marker");
        assert!(!rows[0].referee, "a job with no referee says so");
        assert_eq!(rows[1].folder, "02-beta");
        assert_eq!(rows[1].title, "Second job");
        assert!(rows[1].referee);

        std::fs::remove_dir_all(&root).expect("the written suite cleans up");
    }

    #[test]
    fn a_half_read_suite_refuses_by_name() {
        let root = scratch("r");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("01-no-job")).expect("folder");

        let missing = list_tasks(&root).expect_err("a folder with no task.md is not a suite");
        assert_eq!(missing.code, "tasks_bad_task");
        assert!(
            missing.message.contains("01-no-job"),
            "the refusal names the folder: {missing}"
        );

        std::fs::remove_dir_all(&root).expect("the written suite cleans up");

        let absent = list_tasks(&root.join("nope")).expect_err("no suite at all");
        assert_eq!(absent.code, "tasks_no_suite");
        assert!(
            absent.message.contains(SUITE_REL),
            "the refusal names where the suite lives: {absent}"
        );
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 26,
    name: PACK,
    default_loaded: false,
    tools: &["tasks.list"],
    build: pack,
};
