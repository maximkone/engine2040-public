//! e2e_ladder: DOOR7 - six fixed prompts that measure the model lane, not
//! our mood.
//!
//! Consumer: `crates/endpoint/src/bin/e2040-ladder.rs` for the operator, and
//! `tools/agent-ladder.mjs`, which runs that binary twice and compares the
//! tape blocks. Every rung drives a real door verb through
//! [`PackRegistry::invoke`] - the same locked, gated, audited path a
//! `tools/call` takes - so a rung passes only when the door itself answers.
//! The model half is [`engine_provider::agent_loop`]'s scripted planner,
//! which is this repo's mock provider: offline, deterministic, no sampling
//! and no network.
//!
//! Units: counts for tool calls, the loop's token estimate (one per four
//! bytes) for cost, milliseconds for wall clock. Determinism: the tape block
//! carries rung, status, calls, tokens, transcript digest, the refusing verb
//! and the missed gate - never wall clock - so the same ladder twice gives
//! the same tape; the operator table prints wall clock beside the tape.
//! Errors: a failing rung names the verb that refused it and the machine
//! code, read from [`McpToolError::code`](crate::mcp::McpToolError::code).
//! Gates are all-or-nothing: one missed check fails the rung. A leaf a fresh
//! document draws on every build — a node id — reaches the planner as
//! `<minted>`, so the tape repeats while the gate still sees the real value.
//!
//! [`PackRegistry::invoke`]: crate::packs::PackRegistry::invoke

use std::fmt;
use std::time::Instant;

use engine_core::hash_bytes;
use engine_provider::agent_loop::{
    estimate_tokens, AgentLoop, Door, LoopBudget, LoopError, Plan, ScriptedPlanner,
};
use rmcp::model::JsonObject;
use rmcp::serde_json::{from_str, Value};

use crate::mcp::McpServer;
use crate::packs::PackRegistry;

/// Tape header magic: a reader refuses anything else.
pub const LADDER_MAGIC: &str = "e2040-e2e-ladder-ev1";
/// Tape format version, printed in the header row beside the magic.
pub const LADDER_VERSION: u32 = 1;
/// Longest observation one step may carry, in chars (the loop's own bound).
pub const MAX_OBSERVATION_CHARS: usize = 2048;
/// Most elements the projector reads out of one array.
pub const PROJECT_ARRAY_ELEMENTS: usize = 8;
/// Arrays at least this long and all numbers read as one count (PNG receipts).
pub const PROJECT_BLOB_MIN: usize = 32;
/// Deepest object nesting the projector walks.
pub const PROJECT_MAX_DEPTH: usize = 4;
/// Most door calls one rung's loop may take.
pub const RUNG_MAX_STEPS: u32 = 8;
/// Token ceiling for one rung's loop.
pub const RUNG_MAX_TOKENS: u32 = 4000;
/// Door verbs cost this many tokens before their answer is known.
pub const VERB_COST_TOKENS: u64 = 40;
/// Idempotency keys start here: sixteen per rung, rung order fixed.
pub const RUNG_KEY_BASE: u64 = 100;
/// Keys one rung may use before the next rung's block starts.
pub const RUNG_KEY_STRIDE: u64 = 16;

/// One door call a scripted prompt makes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RungStep {
    /// Exact door verb name (`sim.step`).
    pub verb: &'static str,
    /// The verb's own JSON args; empty means `{}`.
    pub args: &'static str,
}

/// One all-or-nothing assertion over one rung's own door outputs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Check {
    /// Some output reads `path` exactly as `want`.
    Equals {
        /// Dotted path, `*` for every element of an array.
        path: &'static str,
        /// Scalar text to match.
        want: &'static str,
    },
    /// Some output reads `path` as a number above `want`.
    Above {
        /// Dotted path.
        path: &'static str,
        /// Exclusive lower bound.
        want: f64,
    },
    /// Some output reads `path` as a number below `want`.
    Below {
        /// Dotted path.
        path: &'static str,
        /// Exclusive upper bound.
        want: f64,
    },
    /// `path` resolves in at least two outputs and differs between the first
    /// and the last: the world moved under the rung's own calls.
    Moved {
        /// Dotted path.
        path: &'static str,
    },
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Equals { path, want } => write!(f, "{path} == {want}"),
            Self::Above { path, want } => write!(f, "{path} > {want}"),
            Self::Below { path, want } => write!(f, "{path} < {want}"),
            Self::Moved { path } => write!(f, "{path} moves"),
        }
    }
}

/// One fixed rung: its prompt, its scripted door calls and its gate.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rung {
    /// Short id, `E0`..=`E5`.
    pub id: &'static str,
    /// Snake-case rung name, the tape's second column.
    pub name: &'static str,
    /// The goal text the loop reads as its prompt.
    pub goal: &'static str,
    /// Door calls the scripted provider makes, in order.
    pub steps: &'static [RungStep],
    /// All must hold or the rung fails.
    pub checks: &'static [Check],
}

/// The seed and arena every sim rung plays: the golden seed at the golden
/// extent, so a rung measures the shipped tick path. `SIM_RESET` is the whole
/// `sim.reset` args; the step rungs concat the same head with their own tape
/// because `concat!` takes only literals.
const SIM_RESET: &str = "{\"seed\":\"2040\",\"participants\":2,\"half_extent_m\":20.0}";

/// The six rungs, in ladder order. E0 proves the door is up; E1 to E3 prove
/// the fight verbs answer; E4 builds a camp; E5 runs the slice.
pub const RUNGS: [Rung; 6] = [
    Rung {
        id: "E0",
        name: "door_alive",
        goal: "read the door status and report the advertised verb count",
        steps: &[RungStep {
            verb: "door.status",
            args: "{}",
        }],
        checks: &[
            Check::Equals {
                path: "session",
                want: "7",
            },
            Check::Above {
                path: "tools",
                want: 0.0,
            },
        ],
    },
    Rung {
        id: "E1",
        name: "player_walks",
        goal: "reset the bout, push the player forward and read where they ended up",
        steps: &[
            RungStep {
                verb: "sim.reset",
                args: SIM_RESET,
            },
            RungStep {
                verb: "sim.step",
                args: concat!(
                    "{\"seed\":\"2040\",\"participants\":2,\"half_extent_m\":20.0",
                    ",\"ticks\":12,\"events\":0,\"inputs\":[",
                    "{\"tick\":0,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":1,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":2,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":3,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":4,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":5,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":6,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":7,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":8,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":9,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":10,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0},",
                    "{\"tick\":11,\"fighter\":0,\"mx\":100,\"mz\":0,\"facing\":0,\"buttons\":0}",
                    "]}"
                ),
            },
        ],
        checks: &[Check::Moved {
            path: "fighters.0.pos.0",
        }],
    },
    Rung {
        id: "E2",
        name: "two_fighters_trade",
        goal: "press both fighters to attack and read the damage they take",
        // The gap is a measurement: a fighter hit on tick 4 cannot start its
        // own attack before tick 14, so 14 is the earliest trade.
        steps: &[RungStep {
            verb: "sim.step",
            args: concat!(
                "{\"seed\":\"2040\",\"participants\":2,\"half_extent_m\":20.0",
                ",\"ticks\":40,\"events\":16,\"inputs\":[",
                "{\"tick\":0,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":1},",
                "{\"tick\":14,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":1}",
                "]}"
            ),
        }],
        checks: &[
            Check::Equals {
                path: "events.*.kind",
                want: "hit",
            },
            Check::Below {
                path: "fighters.0.hp",
                want: 1000.0,
            },
            Check::Below {
                path: "fighters.1.hp",
                want: 1000.0,
            },
        ],
    },
    Rung {
        id: "E3",
        name: "parry_lands",
        goal: "hold the guard into the incoming attack and read the parry",
        steps: &[RungStep {
            verb: "sim.step",
            args: concat!(
                "{\"seed\":\"2040\",\"participants\":2,\"half_extent_m\":20.0",
                ",\"ticks\":8,\"events\":8,\"inputs\":[",
                "{\"tick\":0,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":1},",
                "{\"tick\":0,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":1,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":1,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":2},",
                "{\"tick\":2,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":2,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":2},",
                "{\"tick\":3,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":3,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":2},",
                "{\"tick\":4,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":4,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":2},",
                "{\"tick\":5,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":5,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":2},",
                "{\"tick\":6,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":6,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":7,\"fighter\":0,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0},",
                "{\"tick\":7,\"fighter\":1,\"mx\":0,\"mz\":0,\"facing\":0,\"buttons\":0}",
                "]}"
            ),
        }],
        checks: &[
            Check::Equals {
                path: "events.*.kind",
                want: "parry",
            },
            Check::Equals {
                path: "fighters.1.hp",
                want: "1000",
            },
        ],
    },
    Rung {
        id: "E4",
        name: "camp_work_builds",
        goal: "build the camp brief and read the landed placement count",
        steps: &[RungStep {
            verb: "level.build",
            args: concat!(
                "{\"brief\":\"prop brazier brazier_a\\nprop barrel barrel_a\\n",
                "prop banner banner_a\\nwant barrel_a next_to object:brazier_a\\n",
                "want banner_a facing object:barrel_a\\nspawn barrel_a\",",
                "\"seed\":2040,\"expected_revision\":0,\"idempotency_key\":7}"
            ),
        }],
        checks: &[
            Check::Equals {
                path: "verdict",
                want: "pass",
            },
            Check::Above {
                path: "revision",
                want: 0.0,
            },
        ],
    },
    Rung {
        id: "E5",
        name: "slice_runs",
        goal: "run the six hundred tick slice and read its digest",
        steps: &[RungStep {
            verb: "sim.step",
            args: concat!(
                "{\"seed\":\"2040\",\"participants\":2,\"half_extent_m\":20.0",
                ",\"ticks\":600,\"events\":0}"
            ),
        }],
        checks: &[
            Check::Equals {
                path: "tick",
                want: "600",
            },
            Check::Above {
                path: "digest",
                want: 0.0,
            },
        ],
    },
];

/// The verb plus the machine code that refused a rung.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refusal {
    /// Door verb that refused (`sim.step`).
    pub verb: String,
    /// Machine code from the typed refusal (`bad_ticks`).
    pub code: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.verb, self.code)
    }
}

/// What one rung measured.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RungRecord {
    /// Rung id.
    pub id: &'static str,
    /// Rung name.
    pub name: &'static str,
    /// Prompt the loop ran.
    pub prompt: &'static str,
    /// True when the loop finished and every check held.
    pub passed: bool,
    /// Door calls that ran.
    pub tool_calls: usize,
    /// Estimated tokens the loop spent.
    pub tokens: u32,
    /// Wall milliseconds for this rung; printed, never in the tape.
    pub wall_ms: u64,
    /// Transcript digest, hex, stable across runs.
    pub digest: String,
    /// The verb and code that refused, when one did.
    pub refused: Option<Refusal>,
    /// The first gate that missed, when the door answered and the gate did not.
    pub gate_miss: Option<String>,
}

impl fmt::Display for RungRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.passed { "pass" } else { "fail" };
        let refused = self
            .refused
            .as_ref()
            .map_or_else(|| "-".to_owned(), |refusal| refusal.to_string());
        let gate = self.gate_miss.clone().unwrap_or_else(|| "-".to_owned());
        write!(
            f,
            "{:<3} {:<20} {:<4} {:>4} {:>6} {:<16} {:<28} {:<24}",
            self.id, self.name, status, self.tool_calls, self.tokens, self.digest, refused, gate
        )
    }
}

/// Every rung measured in ladder order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LadderReport {
    /// One record per rung, in [`RUNGS`] order.
    pub records: Vec<RungRecord>,
}

impl LadderReport {
    /// Rungs whose loop finished and whose gate held.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.records.iter().filter(|record| record.passed).count()
    }

    /// Rungs that failed, whatever the reason.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.records.len() - self.passed()
    }

    /// Door calls across every rung.
    #[must_use]
    pub fn tool_calls(&self) -> usize {
        self.records.iter().map(|record| record.tool_calls).sum()
    }

    /// Estimated tokens across every rung.
    #[must_use]
    pub fn tokens(&self) -> u32 {
        self.records
            .iter()
            .map(|record| record.tokens)
            .fold(0, u32::saturating_add)
    }

    /// Wall milliseconds in rung order, for the operator line only.
    fn wall_ms_line(&self) -> String {
        let rows: Vec<String> = self
            .records
            .iter()
            .map(|record| format!("{}={}", record.id, record.wall_ms))
            .collect();
        rows.join(" ")
    }

    /// The compared artifact: one header row plus one tab-separated row per
    /// rung, with no wall clock, so two runs of the same ladder print the
    /// same tape.
    #[must_use]
    pub fn tape(&self) -> String {
        let mut out = format!("{LADDER_MAGIC}\t{LADDER_VERSION}\n");
        for record in &self.records {
            let status = if record.passed { "pass" } else { "fail" };
            let refused = record
                .refused
                .as_ref()
                .map_or_else(|| "-".to_owned(), |refusal| refusal.to_string());
            let gate = record.gate_miss.clone().unwrap_or_else(|| "-".to_owned());
            out.push_str(&format!(
                "{}\t{}\t{status}\t{}\t{}\t{}\t{refused}\t{gate}\n",
                record.id, record.name, record.tool_calls, record.tokens, record.digest
            ));
        }
        out
    }
}

impl fmt::Display for LadderReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{:<3} {:<20} {:<4} {:>4} {:>6} {:<16} {:<28} {:<24}",
            "rung",
            "name",
            "gate",
            "calls",
            "tokens",
            "transcript",
            "refused verb:code",
            "missed check"
        )?;
        for record in &self.records {
            writeln!(f, "{record}")?;
        }
        let calls = self.tool_calls();
        let tokens = self.tokens();
        writeln!(
            f,
            "\n{} of {} rungs pass, {} failed; {calls} door calls, {tokens} est tokens",
            self.passed(),
            self.records.len(),
            self.failed()
        )?;
        writeln!(f, "wall ms per rung: {}", self.wall_ms_line())?;
        writeln!(f, "tape (the compared artifact, no wall clock):")?;
        write!(f, "{}", self.tape())
    }
}

/// Door half of the loop over the real verb registry: every call takes the
/// locked, screened, audited tool path, and the observation handed back to
/// the planner is a bounded projection of the typed output.
pub struct LadderDoor<'a> {
    server: &'a McpServer,
    replays: Vec<(u64, String)>,
    outputs: Vec<Value>,
    refusals: Vec<Refusal>,
}

impl<'a> LadderDoor<'a> {
    /// Wrap one live door session.
    #[must_use]
    pub fn new(server: &'a McpServer) -> Self {
        Self {
            server,
            replays: Vec::new(),
            outputs: Vec::new(),
            refusals: Vec::new(),
        }
    }

    /// The typed output of each call that ran, in order.
    #[must_use]
    pub fn outputs(&self) -> &[Value] {
        &self.outputs
    }

    /// The last verb that refused, if any.
    #[must_use]
    pub fn last_refusal(&self) -> Option<Refusal> {
        self.refusals.last().cloned()
    }

    /// Estimated tokens across every observation this door returned.
    #[must_use]
    pub fn tokens_spent(&self) -> u32 {
        self.replays
            .iter()
            .map(|(_, observation)| estimate_tokens(observation))
            .fold(0, u32::saturating_add)
    }

    /// Digest of what this door saw plus the text the loop stopped with, so a
    /// refused rung still gets a stable tape row.
    #[must_use]
    pub fn error_digest(&self, error: &str) -> String {
        let mut joined = String::new();
        for (_, observation) in &self.replays {
            joined.push_str(observation);
            joined.push(';');
        }
        joined.push_str(error);
        format!("{:016x}", hash_bytes(joined.as_bytes()))
    }
}

impl Door for LadderDoor<'_> {
    fn tool_names(&self) -> Vec<String> {
        PackRegistry::shipped()
            .verb_names()
            .into_iter()
            .map(|name| name.to_owned())
            .collect()
    }

    fn cost_of(&self, tool: &str) -> u64 {
        u64::from(estimate_tokens(tool)).saturating_add(VERB_COST_TOKENS)
    }

    fn act(&mut self, tool: &str, input: &str, key: u64) -> Result<String, LoopError> {
        if let Some((_, observation)) = self.replays.iter().find(|(seen, _)| *seen == key) {
            return Ok(observation.clone());
        }
        let args = parse_args(input).ok_or_else(|| LoopError::ToolRefused {
            tool: tool.to_owned(),
            reason: String::from("bad_json"),
        })?;
        match PackRegistry::shipped().invoke(self.server, tool, args) {
            Ok(value) => {
                let observation = project(&value);
                self.outputs.push(value);
                self.replays.push((key, observation.clone()));
                Ok(observation)
            }
            Err(refused) => {
                self.refusals.push(Refusal {
                    verb: tool.to_owned(),
                    code: refused.code.to_owned(),
                });
                Err(LoopError::ToolRefused {
                    tool: tool.to_owned(),
                    reason: refused.code.to_owned(),
                })
            }
        }
    }

    /// The ladder issues no keyed writes; the door's own undo verbs own
    /// rollback, so this never reports an undo it did not do.
    fn undo(&mut self, _key: u64) -> bool {
        false
    }
}

/// Run every rung against one door session.
#[must_use]
pub fn run_ladder(server: &McpServer) -> LadderReport {
    let records = RUNGS
        .iter()
        .enumerate()
        .map(|(index, rung)| {
            let rung_slots = u64::try_from(index).unwrap_or(u64::MAX);
            run_rung(server, rung, RUNG_KEY_BASE + rung_slots * RUNG_KEY_STRIDE)
        })
        .collect();
    LadderReport { records }
}

/// Run one rung: scripted calls through the loop, then its gate.
#[must_use]
pub fn run_rung(server: &McpServer, rung: &Rung, key_base: u64) -> RungRecord {
    let started = Instant::now();
    let mut door = LadderDoor::new(server);
    let mut planner = ScriptedPlanner::new(plans_for(rung, key_base));
    let outcome = AgentLoop::new(LoopBudget::new(RUNG_MAX_STEPS, RUNG_MAX_TOKENS)).run(
        rung.goal,
        &mut planner,
        &mut door,
    );
    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let refused = door.last_refusal();
    let gate_miss = match &outcome {
        Ok(_) => rung
            .checks
            .iter()
            .find(|check| !check_passes(check, door.outputs()))
            .map(|check| check.to_string()),
        Err(_) => None,
    };
    let (tool_calls, tokens, digest) = match &outcome {
        Ok(loop_run) => (
            loop_run.steps().len(),
            loop_run.tokens_used,
            format!("{:016x}", loop_run.checkpoint()),
        ),
        Err(error) => (
            door.outputs().len(),
            door.tokens_spent(),
            door.error_digest(&error.to_string()),
        ),
    };
    RungRecord {
        id: rung.id,
        name: rung.name,
        prompt: rung.goal,
        passed: outcome.is_ok() && refused.is_none() && gate_miss.is_none(),
        tool_calls,
        tokens,
        wall_ms,
        digest,
        refused,
        gate_miss,
    }
}

/// The scripted prompt: one door call per step, then the finish.
fn plans_for(rung: &Rung, key_base: u64) -> Vec<Plan> {
    let mut plans: Vec<Plan> = Vec::with_capacity(rung.steps.len() + 1);
    for (index, step) in rung.steps.iter().enumerate() {
        let slot = u64::try_from(index).unwrap_or(u64::MAX);
        plans.push(Plan::ToolCall {
            tool: step.verb.to_owned(),
            input: step.args.to_owned(),
            idempotency_key: key_base + slot,
        });
    }
    plans.push(Plan::Done {
        answer: String::from("ladder"),
    });
    plans
}

/// True when one check holds over the rung's own outputs.
fn check_passes(check: &Check, outputs: &[Value]) -> bool {
    match check {
        Check::Equals { path, want } => outputs.iter().any(|output| {
            select(output, path)
                .into_iter()
                .any(|value| scalar_text(value) == *want)
        }),
        Check::Above { path, want } => outputs.iter().any(|output| {
            select(output, path)
                .into_iter()
                .any(|value| number_of(value).is_some_and(|number| number > *want))
        }),
        Check::Below { path, want } => outputs.iter().any(|output| {
            select(output, path)
                .into_iter()
                .any(|value| number_of(value).is_some_and(|number| number < *want))
        }),
        Check::Moved { path } => {
            let Some(first) = outputs.first() else {
                return false;
            };
            let read = read_at(first, path);
            outputs.len() > 1
                && !read.is_empty()
                && outputs.iter().any(|last| read_at(last, path) != read)
        }
    }
}

/// Every value a dotted path selects; `*` walks a whole array.
fn select<'a>(root: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut frontier = vec![root];
    for segment in path.split('.') {
        let mut next: Vec<&Value> = Vec::new();
        for node in frontier {
            match node {
                Value::Object(map) => {
                    if segment != "*" {
                        if let Some(child) = map.get(segment) {
                            next.push(child);
                        }
                    }
                }
                Value::Array(items) => {
                    if segment == "*" {
                        next.extend(items.iter());
                    } else if let Ok(index) = segment.parse::<usize>() {
                        next.extend(items.get(index));
                    }
                }
                _ => {}
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    frontier
}

/// The selected values as one joined text: the readout a `Moved` compares.
fn read_at(output: &Value, path: &str) -> String {
    select(output, path)
        .iter()
        .map(|value| scalar_text(value))
        .collect::<Vec<String>>()
        .join(",")
}

/// Scalar text of one JSON leaf; anything structural reads as its kind.
fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => String::from("null"),
        Value::Object(_) => String::from("{object}"),
        Value::Array(_) => String::from("[array]"),
    }
}

/// A leaf as a number: JSON numbers, and decimal strings the door uses for
/// 64-bit digests.
fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

/// Parse one step's args; empty text means an empty object.
fn parse_args(input: &str) -> Option<Value> {
    if input.trim().is_empty() {
        return Some(Value::Object(JsonObject::new()));
    }
    from_str(input).ok()
}

/// Bounded, order-stable summary of one door output for the planner.
fn project(value: &Value) -> String {
    let mut tokens: Vec<String> = Vec::new();
    walk("", value, 0, &mut tokens);
    tokens.sort();
    let stamp = format!(";#all={:016x}", hash_bytes(tokens.join(";").as_bytes()));
    let mut out = String::new();
    let mut cut = false;
    for token in tokens {
        if out.len() + token.len() + stamp.len() + 1 > MAX_OBSERVATION_CHARS {
            cut = true;
            break;
        }
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(&token);
    }
    if cut {
        out.push_str(";#cut");
    }
    out.push_str(&stamp);
    out
}

/// Projection text of one leaf: freshly minted identity text reads as a
/// marker, because a new scene document draws new node ids on every build and
/// a tape that carries them can never repeat. The gates still read the value.
fn leaf_text(value: &Value) -> String {
    let text = scalar_text(value);
    if minted(&text) {
        String::from("<minted>")
    } else {
        text
    }
}

/// True for the 8-4-4-4-12 hex form a scene document gives a new node.
fn minted(text: &str) -> bool {
    const DASHES: [usize; 4] = [8, 13, 18, 23];
    text.len() == 36
        && DASHES
            .iter()
            .all(|index| text.as_bytes().get(*index) == Some(&b'-'))
        && text
            .bytes()
            .enumerate()
            .all(|(index, byte)| DASHES.contains(&index) || byte.is_ascii_hexdigit())
}

/// Collect `path=value` tokens over one output subtree.
fn walk(path: &str, value: &Value, depth: usize, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = child_path(path, key);
                if depth + 1 > PROJECT_MAX_DEPTH {
                    out.push(format!("{child_path}#deep"));
                } else {
                    walk(&child_path, child, depth + 1, out);
                }
            }
        }
        Value::Array(items) => {
            if items.len() >= PROJECT_BLOB_MIN && items.iter().all(Value::is_number) {
                out.push(format!("{path}.len={}", items.len()));
                return;
            }
            for (index, child) in items.iter().enumerate() {
                if index >= PROJECT_ARRAY_ELEMENTS {
                    out.push(format!("{path}.len={}", items.len()));
                    return;
                }
                walk(&child_path(path, &index.to_string()), child, depth + 1, out);
            }
        }
        leaf => out.push(format!("{path}={}", leaf_text(leaf))),
    }
}

/// One dotted child path; the root keeps bare keys.
fn child_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// One test output, parsed from the wire text a verb would answer with.
    fn value(text: &str) -> Value {
        from_str(text).expect("test json parses")
    }

    #[test]
    fn three_rungs_pass_over_the_real_door_verbs() {
        let report = run_ladder(&server());
        assert_eq!(report.records.len(), RUNGS.len(), "one row per rung");
        for id in ["E0", "E2", "E3"] {
            let record = report
                .records
                .iter()
                .find(|record| record.id == id)
                .expect("rung row");
            assert!(
                record.passed,
                "{id} failed: refused {:?} gate {:?}",
                record.refused, record.gate_miss
            );
            assert!(record.tool_calls > 0, "{id} called no verb");
        }
        assert!(report.passed() >= 3, "pass count {}", report.passed());
        let table = report.to_string();
        for id in ["E0", "E1", "E2", "E3", "E4", "E5"] {
            assert!(table.contains(id), "the table lost {id}: {table}");
        }
        assert!(table.contains("door calls"), "the table lost its counts");
        println!("{table}");
    }

    #[test]
    fn a_refused_verb_names_itself_in_the_row() {
        let rung = Rung {
            id: "EX",
            name: "bad_ticks",
            goal: "step zero ticks and read the refusal",
            steps: &[RungStep {
                verb: "sim.step",
                args: "{\"seed\":\"2040\",\"participants\":2,\"half_extent_m\":20.0,\"ticks\":0}",
            }],
            checks: &[Check::Equals {
                path: "tick",
                want: "0",
            }],
        };
        let record = run_rung(&server(), &rung, 900);
        assert!(!record.passed, "a refused call cannot pass");
        let refusal = record.refused.clone().expect("the row names the refusal");
        assert_eq!(refusal.verb, "sim.step");
        assert!(!refusal.code.is_empty(), "the code is empty");
        assert!(
            record.to_string().contains("sim.step:"),
            "the row hides the verb: {record}"
        );
    }

    #[test]
    fn an_unknown_verb_refuses_by_name() {
        let rung = Rung {
            id: "EZ",
            name: "no_such_verb",
            goal: "call a verb the door does not serve",
            steps: &[RungStep {
                verb: "no.such.verb",
                args: "{}",
            }],
            checks: &[Check::Equals {
                path: "tick",
                want: "1",
            }],
        };
        let record = run_rung(&server(), &rung, 920);
        assert!(!record.passed);
        assert_eq!(
            record.refused.expect("unknown verb refuses").code,
            "unknown_verb",
            "the door's own code reaches the row"
        );
    }

    #[test]
    fn the_same_ladder_twice_gives_the_same_tape() {
        let first = run_ladder(&server()).tape();
        let second = run_ladder(&server()).tape();
        assert_eq!(first, second, "the tape moved between runs");
        assert!(
            first.starts_with(&format!("{LADDER_MAGIC}\t{LADDER_VERSION}\n")),
            "the header row drifted: {first}"
        );
    }

    #[test]
    fn the_tape_has_eight_columns_and_no_wall_clock() {
        let report = run_ladder(&server());
        let tape = report.tape();
        let rows: Vec<&str> = tape.lines().skip(1).collect();
        assert_eq!(rows.len(), RUNGS.len(), "the tape lost a row");
        for row in &rows {
            assert_eq!(
                row.split('\t').count(),
                8,
                "a tape row changed shape: {row}"
            );
        }
        // A wall clock this loud only exists in the operator table.
        let mut loud = report
            .records
            .first()
            .cloned()
            .expect("the ladder ran a rung");
        loud.wall_ms = 9_999_999;
        let tape = LadderReport {
            records: vec![loud],
        }
        .tape();
        assert!(!tape.contains("9999999"), "wall ms leaked: {tape}");
        assert!(
            !report.tape().contains("wall ms"),
            "the tape carries the operator's line"
        );
    }

    #[test]
    fn projector_marks_a_minted_node_id_and_keeps_a_slot_number() {
        let drawn = value(
            "{\"placements\":[{\"id\":\"2d9f4e6c-4452-4657-84b7-f1b875f3f71b\",\"name\":\"barrel\"}]}",
        );
        let another = value(
            "{\"placements\":[{\"id\":\"4c44c084-1b5a-4fc5-871a-9236e5d8cf12\",\"name\":\"barrel\"}]}",
        );
        let text = project(&drawn);
        assert_eq!(text, project(&another), "a freshly drawn id moved the tape");
        assert!(
            text.contains("placements.0.id=<minted>"),
            "no marker for the drawn id: {text}"
        );
        assert!(
            !text.contains("2d9f4e6c"),
            "the drawn id leaked into the observation: {text}"
        );
        let seated = value("{\"fighters\":[{\"id\":0,\"hp\":1000}]}");
        assert!(
            project(&seated).contains("fighters.0.id=0"),
            "a fighter slot number read as minted: {}",
            project(&seated)
        );
    }

    #[test]
    fn projector_reads_a_byte_blob_as_one_count() {
        let bytes: Vec<Value> = (0..600_u64).map(|byte| Value::from(byte % 256)).collect();
        let mut receipt = JsonObject::new();
        receipt.insert("png_bytes".to_owned(), Value::Array(bytes));
        receipt.insert("tick".to_owned(), Value::from(3));
        let observation = project(&Value::Object(receipt));
        assert!(
            observation.contains("png_bytes.len=600"),
            "the blob was walked: {observation}"
        );
        assert!(
            observation.len() <= MAX_OBSERVATION_CHARS,
            "the observation broke the bound: {}",
            observation.len()
        );
        assert!(
            observation.contains("#all="),
            "no stable digest: {observation}"
        );
    }

    #[test]
    fn projector_is_stable_under_map_order() {
        assert_eq!(
            project(&value("{\"b\":2,\"a\":1}")),
            project(&value("{\"a\":1,\"b\":2}")),
            "token order moved"
        );
    }

    #[test]
    fn select_walks_arrays_with_a_star() {
        let seen = value("{\"events\":[{\"kind\":\"hit\"},{\"kind\":\"parry\"}]}");
        let kinds: Vec<String> = select(&seen, "events.*.kind")
            .into_iter()
            .map(scalar_text)
            .collect();
        assert_eq!(kinds, ["hit", "parry"]);
        let indexed = select(&seen, "events.1.kind");
        assert_eq!(scalar_text(indexed[0]), "parry");
    }

    #[test]
    fn moved_needs_two_outputs_that_differ() {
        let check = Check::Moved { path: "tick" };
        let same = vec![value("{\"tick\":4}"), value("{\"tick\":4}")];
        let apart = vec![value("{\"tick\":0}"), value("{\"tick\":4}")];
        assert!(!check_passes(&check, &same), "an unchanged path moved");
        assert!(check_passes(&check, &apart), "a moved path held still");
        assert!(
            !check_passes(&check, &[value("{\"tick\":4}")]),
            "one output cannot move"
        );
    }

    #[test]
    fn door_replays_a_finished_key_without_a_second_call() {
        let server = server();
        let mut door = LadderDoor::new(&server);
        let first = door
            .act("door.status", "{}", 5)
            .expect("status answers the door");
        let second = door
            .act("door.status", "{}", 5)
            .expect("a finished key replays");
        assert_eq!(first, second, "the replay changed the text");
        assert_eq!(
            door.outputs().len(),
            1,
            "a replay must not run the verb twice"
        );
        assert!(!door.undo(5), "the ladder undo never claims a rollback");
    }

    #[test]
    fn bad_json_refuses_before_the_door_runs() {
        let server = server();
        let mut door = LadderDoor::new(&server);
        let refused = door
            .act("door.status", "{", 9)
            .expect_err("unparseable args never reach a verb");
        assert!(matches!(
            refused,
            LoopError::ToolRefused { tool, reason }
                if tool == "door.status" && reason == "bad_json"
        ));
        assert!(door.outputs().is_empty(), "a bad call wrote an output");
    }

    #[test]
    fn tool_names_come_from_the_shipped_registry() {
        let server = server();
        let door = LadderDoor::new(&server);
        let names = door.tool_names();
        assert!(names.iter().any(|name| name == "sim.step"), "no sim.step");
        assert!(names.iter().any(|name| name == "door.status"));
        assert_eq!(
            names.len(),
            PackRegistry::shipped().verb_names().len(),
            "the door listed a different verb set"
        );
    }
}
