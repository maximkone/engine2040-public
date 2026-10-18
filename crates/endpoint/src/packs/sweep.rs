//! `play.sweep`: seeded bots play N headless bouts and answer one envelope.
//!
//! Consumer: a playtest agent that wants one set of numbers for "does a batch
//! of bouts crash, drift or freeze". Data owner for the simulation is
//! `engine-arena` ([`GymLive`], whose brains decide every slot); this file
//! owns the verb DTOs, the sweep loop and the evidence file. Units: bouts,
//! ticks, fighter-ticks. Every bout runs twice from its own seed: a bout the
//! arena refuses or that stops early is a crash, a bout whose two runs
//! disagree on any tick digest is a hash divergence, and a stuck tick is one
//! live fighter moving slower than [`STUCK_SPEED_MPS`]. Same arguments twice
//! give the same envelope. No clock is read. Evidence: the envelope is also
//! written to `target/reports/sweep-<seed>.json`. DTO docs off.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use engine_arena::fork::STUCK_SPEED_MPS;
use engine_arena::gym::GymLive;
use engine_arena::{ArenaConfig, BoutTape, MAX_PARTICIPANTS};
use engine_core::splitmix64;
use rmcp::schemars::JsonSchema;
// `self` for schemars derive with `serde(with)`.
use rmcp::serde::{self, Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError};
use crate::packs::{bad_args, PackDef, PackError, VerbSpec};
use crate::play::play_session::dec_u64;
use crate::play::{build_tape, PlayInputDto};
use crate::tool_capabilities::CapabilityCard;

/// Verb: seeded bots play N bouts, one envelope back.
pub const TOOL_PLAY_SWEEP: &str = "play.sweep";
/// Most bouts one sweep plays.
pub const MAX_SWEEP_BOUTS: u32 = 20;
/// Bouts when the call names none.
pub const DEFAULT_SWEEP_BOUTS: u32 = 5;
/// Shortest bout in ticks.
pub const MIN_SWEEP_TICKS: u32 = 60;
/// Longest bout in ticks (the `play.run` bound).
pub const MAX_SWEEP_TICKS: u32 = crate::play::MAX_PLAY_TICKS;
/// Bout length when the call names none.
pub const DEFAULT_SWEEP_TICKS: u32 = 600;
/// Sweep seed when the call names none.
pub const DEFAULT_SWEEP_SEED: u64 = 2040;
/// Arena half-extent of every swept bout, meters.
const SWEEP_HALF_EXTENT_M: f32 = 20.0;

/// Register `play.sweep` on the play pack under construction.
pub fn add_verb(pack: &mut PackDef) -> Result<(), PackError> {
    pack.add(VerbSpec::of::<PlaySweepTool>(
        TOOL_PLAY_SWEEP,
        super::PACK,
        CapabilityCard::Observe,
    )?);
    Ok(())
}

/// Request: sweep seed, bout count, bout length and headcount.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlaySweepArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default)]
    pub bouts: Option<u32>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default, alias = "participants")]
    pub fighters: Option<u8>,
}

/// The envelope: counts over all bouts, plus where the evidence file went.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlaySweepOutput {
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub seed: u64,
    pub bouts: u32,
    pub ticks: u32,
    pub crashes: u32,
    pub hash_divergences: u32,
    pub stuck_ticks: u64,
    pub evidence: String,
}

/// A validated sweep.
#[derive(Debug, Clone, Copy)]
struct Spec {
    seed: u64,
    bouts: u32,
    ticks: u32,
    fighters: u8,
}

fn spec_of(args: &PlaySweepArgs) -> Result<Spec, McpToolError> {
    let bouts = args.bouts.unwrap_or(DEFAULT_SWEEP_BOUTS);
    if !(1..=MAX_SWEEP_BOUTS).contains(&bouts) {
        return Err(bad_args(format!(
            "bouts {bouts} is outside 1..={MAX_SWEEP_BOUTS}; re-run with a bout count in range"
        )));
    }
    let ticks = args.ticks.unwrap_or(DEFAULT_SWEEP_TICKS);
    if !(MIN_SWEEP_TICKS..=MAX_SWEEP_TICKS).contains(&ticks) {
        return Err(bad_args(format!(
            "ticks {ticks} is outside {MIN_SWEEP_TICKS}..={MAX_SWEEP_TICKS}; re-run with a bout length in range"
        )));
    }
    let fighters = args.fighters.unwrap_or(2);
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(bad_args(format!(
            "fighters {fighters} is outside 2..={MAX_PARTICIPANTS}; re-run with a headcount in range"
        )));
    }
    Ok(Spec {
        seed: args.seed.unwrap_or(DEFAULT_SWEEP_SEED),
        bouts,
        ticks,
        fighters,
    })
}

/// What one bout run saw: the digest after every tick and the stuck count.
struct Run {
    digests: Vec<u64>,
    stuck: u64,
}

/// Play one bout headless, the bots deciding every slot, plus the `extra`
/// inputs of tick 0. `None` when the arena refuses the config or stops early.
fn play_bout(config: &ArenaConfig, extra: &BoutTape) -> Option<Run> {
    let mut env = GymLive::new(config)?;
    let still_sq = STUCK_SPEED_MPS * STUCK_SPEED_MPS;
    let mut run = Run {
        digests: Vec::with_capacity(usize::try_from(config.ticks).unwrap_or(0)),
        stuck: 0,
    };
    for tick in 0..config.ticks {
        let pinned = if tick == 0 { extra.0.as_slice() } else { &[] };
        env.step(pinned)?;
        let snapshot = env.snapshot();
        run.digests.push(snapshot.digest);
        for fighter in &snapshot.fighters {
            let [vx, _, vz] = fighter.velocity;
            if fighter.hp > 0 && vx * vx + vz * vz < still_sq {
                run.stuck += 1;
            }
        }
    }
    Some(run)
}

/// A bout run behind a panic fence: a panic is a crash, never an abort.
fn fenced(config: &ArenaConfig, extra: &BoutTape) -> Option<Run> {
    catch_unwind(AssertUnwindSafe(|| play_bout(config, extra))).unwrap_or(None)
}

/// Play the sweep. `tamper` names a bout whose replay gets one extra input at
/// tick 0, the way a nondeterministic step would drift: the seam the
/// divergence test uses; the verb passes `None`.
fn sweep(spec: &Spec, tamper: Option<u32>) -> PlaySweepOutput {
    let (mut crashes, mut hash_divergences, mut stuck_ticks) = (0u32, 0u32, 0u64);
    for bout in 0..spec.bouts {
        let config = ArenaConfig {
            seed: splitmix64(spec.seed.wrapping_add(u64::from(bout))),
            ticks: spec.ticks,
            fighters: spec.fighters,
            half_extent_m: SWEEP_HALF_EXTENT_M,
            table_version: 1,
        };
        let drift = if tamper == Some(bout) {
            let shove = PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: -100,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            };
            build_tape(&[shove], config.ticks, config.fighters).unwrap_or_default()
        } else {
            BoutTape::default()
        };
        match (
            fenced(&config, &BoutTape::default()),
            fenced(&config, &drift),
        ) {
            (Some(first), Some(replay)) => {
                hash_divergences += u32::from(first.digests != replay.digests);
                stuck_ticks += first.stuck;
            }
            _ => crashes += 1,
        }
    }
    PlaySweepOutput {
        seed: spec.seed,
        bouts: spec.bouts,
        ticks: spec.ticks,
        crashes,
        hash_divergences,
        stuck_ticks,
        evidence: String::new(),
    }
}

/// Write the envelope under `root/target/reports` and return its relative path.
fn write_evidence(root: &Path, out: &PlaySweepOutput) -> Result<String, McpToolError> {
    let rel = format!("target/reports/sweep-{}.json", out.seed);
    let dir = root.join("target/reports");
    let failed = |why: std::io::Error| {
        McpToolError::new(
            "evidence_unwritten",
            format!("could not write {rel} ({why}); re-run after freeing target/reports"),
        )
    };
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let body = rmcp::serde_json::to_string_pretty(out).unwrap_or_default();
    std::fs::write(root.join(&rel), body).map_err(failed)?;
    Ok(rel)
}

/// Run one request: validate, sweep, write the evidence under `root`.
fn run_sweep(args: &PlaySweepArgs, root: &Path) -> Result<PlaySweepOutput, McpToolError> {
    let mut out = sweep(&spec_of(args)?, None);
    out.evidence = write_evidence(root, &out)?;
    Ok(out)
}

/// `play.sweep` tool: seeded bots play N bouts, one envelope back.
pub struct PlaySweepTool;

impl rmcp::handler::server::router::tool::ToolBase for PlaySweepTool {
    type Parameter = PlaySweepArgs;
    type Output = PlaySweepOutput;
    type Error = McpToolError;

    fn name() -> std::borrow::Cow<'static, str> {
        TOOL_PLAY_SWEEP.into()
    }

    fn description() -> Option<std::borrow::Cow<'static, str>> {
        Some(
            "Seeded bots play N headless bouts twice each: returns bouts, crashes, \
             hash_divergences and stuck_ticks, and writes target/reports/sweep-<seed>.json. \
             Read-only on the scene."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl rmcp::handler::server::router::tool::SyncTool<McpServer> for PlaySweepTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PLAY_SWEEP, |_state| run_sweep(&param, Path::new(".")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn spec(bouts: u32) -> Spec {
        Spec {
            seed: 7,
            bouts,
            ticks: 120,
            fighters: 2,
        }
    }

    /// A 5-bout sweep on seed 7 returns the same envelope twice and counts
    /// nothing wrong in a healthy arena.
    #[test]
    fn sweep_five_bouts_seed_7_repeats_its_envelope() {
        let first = sweep(&spec(5), None);
        let second = sweep(&spec(5), None);
        assert_eq!(first, second);
        assert_eq!((first.bouts, first.ticks), (5, 120));
        assert_eq!((first.crashes, first.hash_divergences), (0, 0));
        eprintln!("sweep seed 7: {first:?}");
    }

    /// One tampered replay is exactly one hash divergence.
    #[test]
    fn an_injected_divergence_reports_one_hash_divergence() {
        let out = sweep(&spec(5), Some(2));
        assert_eq!(out.hash_divergences, 1);
        assert_eq!(out.crashes, 0);
        assert_eq!(out.bouts, 5);
    }

    /// An arena the config check refuses is a crash, counted, not a panic.
    #[test]
    fn a_refused_bout_is_a_counted_crash() {
        let out = sweep(
            &Spec {
                ticks: 0,
                ..spec(3)
            },
            None,
        );
        assert_eq!((out.crashes, out.hash_divergences), (3, 0));
    }

    #[test]
    fn bad_numbers_refuse_with_the_legal_range() {
        let refuse = |args: PlaySweepArgs| spec_of(&args).expect_err("refused");
        let bouts = refuse(PlaySweepArgs {
            bouts: Some(21),
            ..Default::default()
        });
        assert!(bouts.message.contains("1..=20"), "{}", bouts.message);
        let ticks = refuse(PlaySweepArgs {
            ticks: Some(10),
            ..Default::default()
        });
        assert!(ticks.message.contains("60..=3600"), "{}", ticks.message);
        let fighters = refuse(PlaySweepArgs {
            fighters: Some(1),
            ..Default::default()
        });
        assert!(fighters.message.contains("2..="), "{}", fighters.message);
    }

    /// The verb is listed by `api.describe`, runs through the door and writes
    /// the evidence file next to the working directory it was given.
    #[test]
    fn the_verb_is_listed_and_writes_its_evidence() {
        use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
        let server = McpServer::new(7, Capability::unrestricted(7));
        let entry = ApiDescribeTool::invoke(
            &server,
            ApiDescribeArgs {
                path: TOOL_PLAY_SWEEP.to_owned(),
            },
        )
        .expect("play.sweep describes");
        assert!(entry.docs.contains("bouts:"), "{}", entry.docs);

        let root = std::env::temp_dir().join(format!("e2040-sweep-{}", std::process::id()));
        let args = PlaySweepArgs {
            seed: Some(7),
            bouts: Some(2),
            ticks: Some(60),
            fighters: None,
        };
        let out = run_sweep(&args, &root).expect("sweeps");
        assert_eq!(out.evidence, "target/reports/sweep-7.json");
        let text = std::fs::read_to_string(root.join(&out.evidence)).expect("evidence reads");
        assert!(text.contains("\"hash_divergences\": 0"), "{text}");
        assert!(text.contains("\"bouts\": 2"), "{text}");
        let again = run_sweep(&args, &root).expect("sweeps again");
        assert_eq!(out, again);
        let _ = std::fs::remove_dir_all(&root);
    }
}
