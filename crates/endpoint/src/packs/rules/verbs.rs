//! `rules.set`, `rules.test`, `rules.explain`, `rules.get`, `rules.undo`: scripted game
//! rules through the door.
//!
//! Consumer: an MCP client (an AI writing rules). Data owner: the session's rules worker
//! (`super::worker`), one thread that owns the `!Send` Rhai engine. A script is Rhai text over
//! 64-bit integers, vetted and smoke-run before it installs; any refusal keeps the last good
//! script. `rules.set` also names the fighter slots the script drives; every later `play.run`
//! of the session then plays those slots from the script (tape inputs still win), and the same
//! seed twice gives the same checkpoints. Units: ticks, millimetres, centidegrees, permille.
//! Errors: `rules_refused` (typed text, last good kept), `bad_slots`, `no_script`,
//! `bad_ticks`, `bad_fighters`, `bad_extent`, `rules_worker_down`.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::json;

use engine_arena::controller::SlotTape;
use engine_arena::world::MatchWorld;
use engine_arena::{ArenaConfig, Checkpoint, MAX_PARTICIPANTS};

use super::worker::{explanation, Worker};
use crate::mcp::{McpServer, McpToolError};
use crate::play::MAX_PLAY_TICKS;

/// Tool: install a rule script.
pub const TOOL_RULES_SET: &str = "rules.set";
/// Tool: dry-run a rule script.
pub const TOOL_RULES_TEST: &str = "rules.test";
/// Tool: explain the installed script at one tick of a match.
pub const TOOL_RULES_EXPLAIN: &str = "rules.explain";
/// Tool: read back the installed script and its slots.
pub const TOOL_RULES_GET: &str = "rules.get";
/// Tool: reinstall the script installed before the current one.
pub const TOOL_RULES_UNDO: &str = "rules.undo";

/// Most host calls one fighter's explanation lists.
const MAX_EXPLAIN_CALLS: usize = 64;
/// Slot driven when `rules.set` names none.
const DEFAULT_SLOTS: [u8; 1] = [0];

/// The session's rules worker, started on the first `rules.set`.
#[derive(Default)]
pub(crate) struct RulesSlot {
    worker: Option<Worker>,
}

/// The installed script as `play.run` sees it.
pub(crate) struct Scripted<'a> {
    worker: &'a Worker,
    hash: u64,
    slots: &'a [u8],
}

impl RulesSlot {
    /// The script that drives slots, if one is installed.
    pub(crate) fn scripted(&self) -> Option<Scripted<'_>> {
        let worker = self.worker.as_ref()?;
        let (hash, slots) = worker.installed()?;
        (!slots.is_empty()).then_some(Scripted {
            worker,
            hash: *hash,
            slots,
        })
    }

    fn worker_mut(&mut self) -> Result<&mut Worker, McpToolError> {
        if self.worker.is_none() {
            self.worker = Worker::spawn();
        }
        self.worker.as_mut().ok_or_else(|| {
            McpToolError::new(
                "rules_worker_down",
                "the OS refused to start a rules worker",
            )
        })
    }
}

impl Scripted<'_> {
    /// One log line for the job: which script drives which slots.
    pub(crate) fn note(&self) -> String {
        format!(
            "rules script {:016x} drives slots {:?}",
            self.hash, self.slots
        )
    }

    /// Simulate the match with the script driving its slots.
    pub(crate) fn run(
        &self,
        config: &ArenaConfig,
        tape: &SlotTape,
        world: Option<&MatchWorld>,
    ) -> Result<Vec<Checkpoint>, McpToolError> {
        Ok(self.worker.run(config, tape, world, None)?.checkpoints)
    }
}

// ---------------------------------------------------------------------------
// rules.set
// ---------------------------------------------------------------------------

/// Args for `rules.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesSetArgs {
    /// Rhai text over integers; defines no function, runs once per scripted fighter per tick.
    pub source: String,
    /// Slots the script drives in `play.run` (default `[0]`, empty drives none).
    pub slots: Option<Vec<u8>>,
}

/// Result of `rules.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesSetOutput {
    /// Revision of the installed script, from 1.
    pub rev: u32,
    /// Script hash (16 hex digits).
    pub hash: String,
    /// Hash of the script this one replaced.
    pub replaced: Option<String>,
    /// Smoke worlds it ran clean through.
    pub cases: u32,
    /// Most operations one fighter call spent (budget 2000).
    pub max_ops: u64,
    /// Slots it drives.
    pub slots: Vec<u8>,
}

fn rules_set(server: &McpServer, args: &RulesSetArgs) -> Result<RulesSetOutput, McpToolError> {
    let mut slots = args.slots.clone().unwrap_or_else(|| DEFAULT_SLOTS.to_vec());
    if let Some(bad) = slots.iter().find(|slot| **slot >= MAX_PARTICIPANTS) {
        return Err(McpToolError::new(
            "bad_slots",
            format!("slot {bad} is outside 0..{MAX_PARTICIPANTS}; submit slots in that range"),
        ));
    }
    slots.sort_unstable();
    slots.dedup();
    server.dispatch(TOOL_RULES_SET, |state| {
        let worker = state.rules.worker_mut()?;
        let kept = worker.installed().map(|(hash, _)| *hash);
        match worker.set(&args.source, slots.clone())? {
            Ok(receipt) => Ok(RulesSetOutput {
                rev: receipt.rev,
                hash: format!("{:016x}", receipt.hash),
                replaced: receipt.replaced.map(|hash| format!("{hash:016x}")),
                cases: u32::try_from(receipt.cases).unwrap_or(u32::MAX),
                max_ops: receipt.max_ops,
                slots,
            }),
            Err(error) => {
                let kept_text = kept.map_or_else(
                    || "no script was installed".to_owned(),
                    |hash| format!("last good script {hash:016x} kept"),
                );
                Err(
                    McpToolError::new("rules_refused", format!("{error}; {kept_text}")).with_data(
                        json!({
                            "error": error.to_string(),
                            "kept": kept.map(|hash| format!("{hash:016x}")),
                        }),
                    ),
                )
            }
        }
    })
}

crate::e2_tool!(
    /// `rules.set`: install a rule script that drives fighter slots.
    RulesSetTool,
    name = TOOL_RULES_SET,
    description = "Install a rule script (Rhai over integers) that drives the fighter slots it names in later play.run calls. Vetted and smoke-run first; a refusal keeps the last good script. Host calls: tick, nearest_foe, dist, hp_of, stamina_of, x_of, z_of, roll, clamp, mem_get, mem_set; input calls walk(mx,mz), face(cdeg), press(bits 1 attack 2 block 4 dodge).",
    readonly = false,
    args = RulesSetArgs,
    output = RulesSetOutput,
    invoke = rules_set
);

// ---------------------------------------------------------------------------
// rules.test
// ---------------------------------------------------------------------------

/// Args for `rules.test`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesTestArgs {
    /// Rhai text to dry-run.
    pub source: String,
}

/// One fixture world's result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesTestCase {
    /// Actions emitted across the world's fighters.
    pub actions: u32,
    /// Most operations one fighter call spent.
    pub max_ops: u64,
    /// First failure, if any.
    pub error: Option<String>,
}

/// Result of `rules.test`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesTestOutput {
    /// True when the text compiled and every world ran clean.
    pub ok: bool,
    /// Script hash (16 hex digits), zeros when it did not compile.
    pub hash: String,
    /// Why the text was refused before running, if it was.
    pub refused: Option<String>,
    /// One result per fixture world.
    pub cases: Vec<RulesTestCase>,
}

fn rules_test(server: &McpServer, args: &RulesTestArgs) -> Result<RulesTestOutput, McpToolError> {
    server.dispatch(TOOL_RULES_TEST, |state| {
        let report = state.rules.worker_mut()?.test(&args.source)?;
        Ok(RulesTestOutput {
            ok: report.ok(),
            hash: format!("{:016x}", report.hash),
            refused: report.refused.as_ref().map(ToString::to_string),
            cases: report
                .cases
                .iter()
                .map(|case| RulesTestCase {
                    actions: u32::try_from(case.actions).unwrap_or(u32::MAX),
                    max_ops: case.max_ops,
                    error: case.error.as_ref().map(|(_, error)| error.to_string()),
                })
                .collect(),
        })
    })
}

crate::e2_tool!(
    /// `rules.test`: dry-run a rule script over the fixture worlds, installing nothing.
    RulesTestTool,
    name = TOOL_RULES_TEST,
    description = "Dry-run a rule script over fixed fixture worlds and report actions, operations spent and the first failure. Installs nothing and changes no match.",
    readonly = true,
    args = RulesTestArgs,
    output = RulesTestOutput,
    invoke = rules_test
);

// ---------------------------------------------------------------------------
// rules.explain
// ---------------------------------------------------------------------------

/// Args for `rules.explain`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesExplainArgs {
    /// Tick to explain, `0..3600`.
    pub tick: u32,
    /// Match seed (default 0), as in play.run.
    pub seed: Option<u64>,
    /// Players, `2..=MAX_PARTICIPANTS` (default 2).
    pub fighters: Option<u8>,
    /// Arena half extent in meters, `5.0..=100.0` (default 12).
    pub half_extent_m: Option<f32>,
    /// Scene revision to play on, as in play.run.
    pub scene_rev: Option<u64>,
}

/// One host call the script made.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesCall {
    /// Host function.
    pub name: String,
    /// Arguments (unused ones 0).
    pub args: [i64; 3],
    /// Value returned.
    pub ret: i64,
}

/// One scripted fighter at the explained tick.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesActor {
    /// Slot.
    pub id: u8,
    /// Operations the call spent.
    pub ops: u64,
    /// Host calls in order (at most 64).
    pub calls: Vec<RulesCall>,
    /// What the script asked for.
    pub actions: Vec<String>,
    /// Why the call failed, if it did.
    pub error: Option<String>,
}

/// Result of `rules.explain`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesExplainOutput {
    /// Installed script hash (16 hex digits).
    pub hash: String,
    /// Installed revision.
    pub rev: u32,
    /// Tick explained.
    pub tick: u32,
    /// Slots the script drives.
    pub slots: Vec<u8>,
    /// One entry per living scripted fighter.
    pub fighters: Vec<RulesActor>,
}

fn rules_explain(
    server: &McpServer,
    args: &RulesExplainArgs,
) -> Result<RulesExplainOutput, McpToolError> {
    if args.tick >= MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("tick {} is outside 0..{MAX_PLAY_TICKS}", args.tick),
        ));
    }
    let fighters = args.fighters.unwrap_or(2);
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(McpToolError::new(
            "bad_fighters",
            format!("fighters {fighters} is outside 2..={MAX_PARTICIPANTS}"),
        ));
    }
    let half = args.half_extent_m.unwrap_or(12.0);
    if !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0 meters"),
        ));
    }
    let config = ArenaConfig {
        seed: args.seed.unwrap_or(0),
        ticks: args.tick + 1,
        fighters,
        half_extent_m: half,
        table_version: 1,
    };
    let world = match args.scene_rev {
        Some(rev) => Some(server.dispatch(TOOL_RULES_EXPLAIN, |state| {
            crate::play::world_load::world_for_revision(&state.doc, rev)
        })?),
        None => None,
    };
    server.dispatch(TOOL_RULES_EXPLAIN, |state| {
        let worker = state.rules.worker_mut()?;
        let Some((_, slots)) = worker.installed().filter(|(_, slots)| !slots.is_empty()) else {
            return Err(McpToolError::new(
                "no_script",
                "no rule script drives any slot; call rules.set first",
            ));
        };
        let slots = slots.clone();
        let reply = worker.run(
            &config,
            &SlotTape(Vec::new()),
            world.as_ref(),
            Some(args.tick),
        )?;
        if reply.checkpoints.is_empty() {
            return Err(McpToolError::new(
                "invalid_config",
                "arena refused this seed, fighters and extent",
            ));
        }
        let explained = explanation(&reply).ok_or_else(|| {
            McpToolError::new("no_script", "the script was not installed at that tick")
        })?;
        Ok(RulesExplainOutput {
            hash: format!("{:016x}", explained.script_hash),
            rev: explained.rev,
            tick: explained.tick,
            slots,
            fighters: explained
                .fighters
                .iter()
                .map(|fighter| RulesActor {
                    id: fighter.id,
                    ops: fighter.ops,
                    calls: fighter
                        .calls
                        .iter()
                        .take(MAX_EXPLAIN_CALLS)
                        .map(|call| RulesCall {
                            name: call.name.to_owned(),
                            args: call.args,
                            ret: call.ret,
                        })
                        .collect(),
                    actions: fighter.actions.iter().map(|a| format!("{a:?}")).collect(),
                    error: fighter.error.as_ref().map(ToString::to_string),
                })
                .collect(),
        })
    })
}

crate::e2_tool!(
    /// `rules.explain`: what the installed script did at one tick of a match.
    RulesExplainTool,
    name = TOOL_RULES_EXPLAIN,
    description = "Explain the installed rule script at one tick of a match (seed, fighters, extent, optional scene_rev as in play.run): every scripted fighter's host calls with arguments and results, operations spent and the actions asked for. Changes nothing.",
    readonly = true,
    args = RulesExplainArgs,
    output = RulesExplainOutput,
    invoke = rules_explain
);

// ---------------------------------------------------------------------------
// rules.get and rules.undo
// ---------------------------------------------------------------------------

/// Args for `rules.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesGetArgs {}

/// Args for `rules.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesUndoArgs {}

/// Result of `rules.get` and `rules.undo`: the script now installed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesScriptOutput {
    /// Revision of the installed script, from 1.
    pub rev: u32,
    /// Script hash (16 hex digits).
    pub hash: String,
    /// The installed text, as `rules.set` normalized it.
    pub source: String,
    /// Slots it drives.
    pub slots: Vec<u8>,
}

impl From<super::worker::ScriptInfo> for RulesScriptOutput {
    fn from(info: super::worker::ScriptInfo) -> Self {
        Self {
            rev: info.rev,
            hash: format!("{:016x}", info.hash),
            source: info.source,
            slots: info.slots,
        }
    }
}

fn rules_get(server: &McpServer, _args: &RulesGetArgs) -> Result<RulesScriptOutput, McpToolError> {
    server.dispatch(TOOL_RULES_GET, |state| {
        let worker = state.rules.worker.as_ref().ok_or_else(no_script)?;
        Ok(worker.get()?.into())
    })
}

fn no_script() -> McpToolError {
    McpToolError::new(
        "no_script",
        "no rule script is installed; call rules.set first",
    )
}

fn rules_undo(
    server: &McpServer,
    _args: &RulesUndoArgs,
) -> Result<RulesScriptOutput, McpToolError> {
    server.dispatch(TOOL_RULES_UNDO, |state| {
        let worker = state.rules.worker.as_mut().ok_or_else(|| {
            McpToolError::new(
                "nothing_to_undo",
                "one script is installed and none came before it; call rules.set first",
            )
        })?;
        Ok(worker.undo()?.into())
    })
}

crate::e2_tool!(
    /// `rules.get`: read back the installed rule script.
    RulesGetTool,
    name = TOOL_RULES_GET,
    description = "Read back the installed rule script: its revision, hash, normalized text and the fighter slots it drives. Changes nothing.",
    readonly = true,
    args = RulesGetArgs,
    output = RulesScriptOutput,
    invoke = rules_get
);

crate::e2_tool!(
    /// `rules.undo`: reinstall the script installed before the current one.
    RulesUndoTool,
    name = TOOL_RULES_UNDO,
    description = "Reinstall the rule script installed before the current one, with its slots and a fresh revision. A second undo walks forward again. With one install only it refuses nothing_to_undo.",
    readonly = false,
    args = RulesUndoArgs,
    output = RulesScriptOutput,
    invoke = rules_undo
);
