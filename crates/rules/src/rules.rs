//! `Rules`: the sandboxed rules engine with `set` / `test` / `explain` and a last-good script.
//!
//! One `Rules` owns one engine pair (a lean one for ticks, one that counts operations for tests and
//! explains), the per-fighter integer memory and the installed script. A failed
//! `set` changes nothing. Rules never write sim state: `tick` returns actions in a fixed
//! order and the arena applies them.
//!
//! Determinism: the engine is raw (no clock, no random, no float, no hash-order types), the
//! host API is integers, actions come out in `(actor id, emission)` order, and a fighter call
//! that fails discards its actions and memory writes, so the same world and script give the
//! same report on every machine.

use std::rc::Rc;

use engine_core::{mix64, Fnv64};
use rhai::{Engine, Scope};

use crate::error::{HostFault, LimitKind, RulesError};
use crate::fixtures::smoke_worlds;
use crate::host::{self, Action, Emitted, HostCall, HostCtx, World};
use crate::limits::{Limits, MAX_FIGHTERS, MEM_SLOTS};
use crate::script::{self, Compiled};

/// Bumped when the host API, action layout, memory layout or error policy changes.
pub const RULES_ABI_VERSION: u16 = 3;
/// The exact Rhai the ABI was written against (also pinned `=` in `Cargo.toml`).
pub const RHAI_VERSION: &str = "1.26.1";

/// The id a tape header stores: it changes whenever anything that can change a replay does.
/// Hashes the ABI version, the Rhai version, every limit, the host function table, the
/// property list and the memory shape.
pub fn rules_abi(limits: &Limits) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_str("engine-rules abi");
    hasher.write_u16(RULES_ABI_VERSION);
    hasher.write_str(RHAI_VERSION);
    limits.write_into(&mut hasher);
    hasher.write_u64(MAX_FIGHTERS as u64);
    hasher.write_u64(MEM_SLOTS as u64);
    for (name, arity) in host::HOST_FNS {
        hasher.write_str(name);
        hasher.write_u8(0);
        hasher.write_u64(*arity as u64);
    }
    for prop in host::PROPS {
        hasher.write_str(prop);
        hasher.write_u8(0);
    }
    hasher.finish()
}

/// One installed script in the order installed: the history `rules.set` keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Installed {
    /// Revision, counting from 1.
    pub rev: u32,
    /// Tick of the match when it was installed.
    pub tick: u32,
    /// Script hash.
    pub hash: u64,
}

/// What a successful `set` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetReceipt {
    /// New revision.
    pub rev: u32,
    /// New script hash.
    pub hash: u64,
    /// Hash that was replaced, if any.
    pub replaced: Option<u64>,
    /// Smoke worlds the script ran through.
    pub cases: usize,
    /// Most operations any single fighter call spent.
    pub max_ops: u64,
}

/// One fixture world's result inside `test`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseReport {
    /// Actions emitted across the world's fighters.
    pub actions: usize,
    /// Most operations a single fighter call spent.
    pub max_ops: u64,
    /// First fighter failure, if any.
    pub error: Option<(u8, RulesError)>,
}

/// What `test` reports: nothing is installed or changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestReport {
    /// Script hash (of the normalized text), 0 when it did not compile.
    pub hash: u64,
    /// Why the text was refused before running, if it was.
    pub refused: Option<RulesError>,
    /// One report per fixture world.
    pub cases: Vec<CaseReport>,
}

impl TestReport {
    /// True when the script compiled and every fixture ran clean.
    pub fn ok(&self) -> bool {
        self.refused.is_none() && self.cases.iter().all(|case| case.error.is_none())
    }

    /// First failure of any kind.
    pub fn first_error(&self) -> Option<RulesError> {
        self.refused.clone().or_else(|| {
            self.cases
                .iter()
                .find_map(|case| case.error.as_ref().map(|(_, e)| e.clone()))
        })
    }
}

/// A script call that failed at run time; its actions and memory writes were discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    /// Actor whose call failed.
    pub actor: u8,
    /// Why.
    pub error: RulesError,
}

/// What one tick produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Actions in `(actor id, emission order)`.
    pub actions: Vec<Emitted>,
    /// Failed calls, in actor order.
    pub faults: Vec<Fault>,
}

impl TickReport {
    /// FNV-1a 64 of every action and fault (fault detail by kind only).
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_len(self.actions.len());
        for emitted in &self.actions {
            hasher.write_u8(emitted.actor);
            emitted.action.write_into(&mut hasher);
        }
        hasher.write_len(self.faults.len());
        for fault in &self.faults {
            hasher.write_u8(fault.actor);
            write_fault_error(&mut hasher, &fault.error);
        }
        hasher.finish()
    }
}

/// Fold a fault's kind into a hasher without allocating.
/// Layout is frozen: one tag byte per `RulesError` variant (plus inner tag for
/// `Limit`/`Host`).
fn write_fault_error(hasher: &mut Fnv64, error: &RulesError) {
    match error {
        RulesError::TooLarge { .. } => hasher.write_u8(1),
        RulesError::Float { .. } => hasher.write_u8(2),
        RulesError::Syntax { .. } => hasher.write_u8(3),
        RulesError::UnknownVariable { .. } => hasher.write_u8(4),
        RulesError::UnknownHostCall { .. } => hasher.write_u8(5),
        RulesError::Forbidden { .. } => hasher.write_u8(6),
        RulesError::UnboundedLoop { .. } => hasher.write_u8(7),
        RulesError::Limit { kind } => {
            hasher.write_u8(8);
            hasher.write_u8(match kind {
                LimitKind::Operations => 0,
                LimitKind::CallDepth => 1,
                LimitKind::ExprDepth => 2,
                LimitKind::Data => 3,
            });
        }
        RulesError::Host(fault) => {
            hasher.write_u8(9);
            hasher.write_u8(match fault {
                HostFault::BadTarget(_) => 0,
                HostFault::BadArg { .. } => 1,
                HostFault::ActionCap => 2,
                HostFault::MemSlot(_) => 3,
            });
        }
        RulesError::Arithmetic { .. } => hasher.write_u8(10),
        RulesError::Thrown { .. } => hasher.write_u8(11),
        RulesError::Runtime { .. } => hasher.write_u8(12),
        RulesError::BadWorld { .. } => hasher.write_u8(13),
        RulesError::NoScript => hasher.write_u8(14),
        RulesError::BadSpec { .. } => hasher.write_u8(15),
        RulesError::NoEffect { cases } => {
            hasher.write_u8(16);
            hasher.write_u64(u64::try_from(*cases).unwrap_or(u64::MAX));
        }
    }
}

/// One fighter's part of an explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorExplain {
    /// Actor.
    pub id: u8,
    /// Operations the call spent.
    pub ops: u64,
    /// Host calls in order, with their arguments and results.
    pub calls: Vec<HostCall>,
    /// Actions that would be applied (empty when the call failed).
    pub actions: Vec<Action>,
    /// Why the call failed, if it did.
    pub error: Option<RulesError>,
    /// This fighter's memory after the call (kept only when the call succeeded).
    pub mem_after: [i64; MEM_SLOTS],
}

/// `rules.explain`: a dry run of the installed script on one world with every host call logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    /// Installed script hash.
    pub script_hash: u64,
    /// Installed revision.
    pub rev: u32,
    /// Tick explained.
    pub tick: u32,
    /// One entry per living fighter, in slot order.
    pub fighters: Vec<ActorExplain>,
}

/// A copy of the per-fighter memory, for rollback and resimulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemSnapshot(Vec<[i64; MEM_SLOTS]>);

struct Active {
    rev: u32,
    compiled: Compiled,
}

/// The rules engine.
pub struct Rules {
    ctx: Rc<HostCtx>,
    engine: Engine,
    probe: Engine,
    limits: Limits,
    active: Option<Active>,
    history: Vec<Installed>,
    refusals: u32,
    last_refusal: Option<RulesError>,
}

impl Default for Rules {
    fn default() -> Self {
        Self::new(Limits::DEFAULT)
    }
}

impl Rules {
    /// An engine with no script installed.
    pub fn new(limits: Limits) -> Self {
        let ctx = Rc::new(HostCtx::new(&limits));
        Self {
            engine: script::build_engine(&ctx, &limits, false),
            probe: script::build_engine(&ctx, &limits, true),
            ctx,
            limits,
            active: None,
            history: Vec::new(),
            refusals: 0,
            last_refusal: None,
        }
    }

    /// The limits this engine runs under.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The ABI id a tape header must carry for this engine.
    pub fn abi(&self) -> u64 {
        rules_abi(&self.limits)
    }

    /// Hash of the installed script.
    pub fn script_hash(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.compiled.hash)
    }

    /// Source of the installed script (normalized line endings).
    pub fn source(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.compiled.source.as_str())
    }

    /// Every install so far, oldest first.
    pub fn history(&self) -> &[Installed] {
        &self.history
    }

    /// Edits refused so far.
    pub fn refusals(&self) -> u32 {
        self.refusals
    }

    /// The most recent refusal, kept for the next `explain` of "why did my edit not apply".
    pub fn last_refusal(&self) -> Option<&RulesError> {
        self.last_refusal.as_ref()
    }

    /// `rules.set`: compile, vet, run the candidate through the smoke worlds, then install.
    /// Any failure refuses the edit and keeps the last good script untouched. A
    /// script that fires no action in any smoke world is refused
    /// [`RulesError::NoEffect`]: it would pace nothing.
    pub fn set(&mut self, source: &str, tick: u32) -> Result<SetReceipt, RulesError> {
        self.set_allowing_no_op(source, tick, false)
    }

    /// [`Rules::set`] with the no-op escape hatch: `no_op` true installs a
    /// script that deliberately fires no action (a probe, a placeholder), so a
    /// caller declares the intent instead of the door guessing it (AD13).
    pub fn set_allowing_no_op(
        &mut self,
        source: &str,
        tick: u32,
        no_op: bool,
    ) -> Result<SetReceipt, RulesError> {
        let outcome = self.vet(source, &smoke_worlds());
        let (compiled, max_ops, cases, actions) = match outcome {
            Ok(done) => done,
            Err(err) => {
                self.refusals += 1;
                self.last_refusal = Some(err.clone());
                return Err(err);
            }
        };
        if actions == 0 && !no_op {
            let err = RulesError::NoEffect { cases };
            self.refusals += 1;
            self.last_refusal = Some(err.clone());
            return Err(err);
        }
        let replaced = self.script_hash();
        let rev = self.history.last().map_or(1, |last| last.rev + 1);
        let hash = compiled.hash;
        self.history.push(Installed { rev, tick, hash });
        self.active = Some(Active { rev, compiled });
        Ok(SetReceipt {
            rev,
            hash,
            replaced,
            cases,
            max_ops,
        })
    }

    /// Install `source` without the smoke run: compile and vet only. Replay uses this to load
    /// the exact script a tape names (it already ran when the tape was recorded).
    pub fn load_exact(&mut self, source: &str, tick: u32) -> Result<SetReceipt, RulesError> {
        let compiled = script::compile(&self.engine, source, &self.limits)?;
        let replaced = self.script_hash();
        let rev = self.history.last().map_or(1, |last| last.rev + 1);
        let hash = compiled.hash;
        self.history.push(Installed { rev, tick, hash });
        self.active = Some(Active { rev, compiled });
        Ok(SetReceipt {
            rev,
            hash,
            replaced,
            cases: 0,
            max_ops: 0,
        })
    }

    /// `rules.test`: run `source` over `worlds` and report, installing nothing.
    pub fn test(&self, source: &str, worlds: &[World]) -> TestReport {
        let compiled = match script::compile(&self.engine, source, &self.limits) {
            Ok(compiled) => compiled,
            Err(err) => {
                return TestReport {
                    hash: 0,
                    refused: Some(err),
                    cases: Vec::new(),
                }
            }
        };
        let saved = self.snapshot();
        let cases = worlds
            .iter()
            .map(|world| self.run_case(&compiled, world))
            .collect();
        self.restore_unchecked(&saved);
        TestReport {
            hash: compiled.hash,
            refused: None,
            cases,
        }
    }

    /// `rules.explain`: dry-run the installed script on `world`, logging every host call and the
    /// operations spent. Memory and the engine are left as they were.
    pub fn explain(&self, world: &World) -> Result<Explanation, RulesError> {
        self.explain_slots(world, u64::MAX)
    }

    /// [`explain`](Self::explain) for the actors whose bit is set in `slots`.
    pub fn explain_slots(&self, world: &World, slots: u64) -> Result<Explanation, RulesError> {
        let active = self.active.as_ref().ok_or(RulesError::NoScript)?;
        let saved = self.snapshot();
        let run = self.run_world(&self.probe, &active.compiled, world, true, slots);
        self.restore_unchecked(&saved);
        Ok(Explanation {
            script_hash: active.compiled.hash,
            rev: active.rev,
            tick: world.tick,
            fighters: run?.traces,
        })
    }

    /// Run the installed script for every living fighter in slot order. With no script
    /// installed the report is empty. A failing call is a `Fault`, never a panic or an `Err`.
    pub fn tick(&mut self, world: &World) -> Result<TickReport, RulesError> {
        self.tick_slots(world, u64::MAX)
    }

    /// [`tick`](Self::tick) for the actors whose bit is set in `slots` (bit `i` is slot `i`);
    /// the other fighters stay readable but run no script. A controller that drives only some
    /// slots of a real arena uses this.
    pub fn tick_slots(&mut self, world: &World, slots: u64) -> Result<TickReport, RulesError> {
        let Some(active) = self.active.as_ref() else {
            return Ok(TickReport::default());
        };
        let run = self.run_world(&self.engine, &active.compiled, world, false, slots)?;
        Ok(TickReport {
            actions: run.actions,
            faults: run.faults,
        })
    }
    /// Copy the per-fighter memory (the only state rules keep between ticks).
    pub fn snapshot(&self) -> MemSnapshot {
        MemSnapshot(self.ctx.mem.borrow().clone())
    }

    /// Put the memory back (rollback and resimulation).
    pub fn restore(&mut self, snapshot: &MemSnapshot) {
        self.restore_unchecked(snapshot);
    }

    /// FNV-1a 64 of the per-fighter memory: part of the match state hash when rules are on.
    pub fn state_digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        for row in self.ctx.mem.borrow().iter() {
            for value in row {
                hasher.write_i64(*value);
            }
        }
        mix64(hasher.finish())
    }

    /// Forget all memory (a new match).
    pub fn reset_memory(&mut self) {
        for row in self.ctx.mem.borrow_mut().iter_mut() {
            *row = [0; MEM_SLOTS];
        }
    }

    fn restore_unchecked(&self, snapshot: &MemSnapshot) {
        self.ctx.mem.borrow_mut().clone_from(&snapshot.0);
    }

    fn load_world(&self, world: &World) -> Result<(), RulesError> {
        let ordered = world
            .fighters
            .iter()
            .enumerate()
            .all(|(i, view)| usize::from(view.id) == i);
        if world.fighters.len() > MAX_FIGHTERS || !ordered {
            return Err(RulesError::BadWorld {
                fighters: world.fighters.len(),
            });
        }
        self.ctx.tick.set(world.tick);
        self.ctx.seed.set(world.seed);
        let mut views = self.ctx.world.borrow_mut();
        views.clear();
        views.extend_from_slice(&world.fighters);
        Ok(())
    }

    /// Run the script once per living actor, in slot order. A failed actor is rolled back and
    /// recorded; the others still take their turn.
    fn run_world(
        &self,
        engine: &Engine,
        compiled: &Compiled,
        world: &World,
        explain: bool,
        slots: u64,
    ) -> Result<WorldRun, RulesError> {
        self.load_world(world)?;
        let ctx = &self.ctx;
        ctx.reset_run(explain, explain);
        let mut faults = Vec::new();
        let mut scope = Scope::new();
        let wanted = |id: u8| id >= 64 || (slots >> id) & 1 == 1;
        for view in world
            .fighters
            .iter()
            .filter(|view| view.alive && wanted(view.id))
        {
            ctx.begin(view.id);
            scope.rewind(0);
            script::push_vars(&mut scope, &compiled.vars, view);
            match engine.run_ast_with_scope(&mut scope, &compiled.ast) {
                Ok(()) => ctx.flush_ok(),
                Err(err) => {
                    if let Some(fault) = ctx.fail(&script::map_eval_error(&err)) {
                        faults.push(fault);
                    }
                }
            }
        }
        let (actions, traces, max_ops) = ctx.finish_run();
        Ok(WorldRun {
            actions,
            faults,
            traces,
            max_ops,
        })
    }

    /// Run one fixture world on the counting engine; memory is the caller's to restore.
    fn run_case(&self, compiled: &Compiled, world: &World) -> CaseReport {
        match self.run_world(&self.probe, compiled, world, false, u64::MAX) {
            Ok(run) => CaseReport {
                actions: run.actions.len(),
                max_ops: run.max_ops,
                error: run
                    .faults
                    .first()
                    .map(|fault| (fault.actor, fault.error.clone())),
            },
            Err(err) => CaseReport {
                actions: 0,
                max_ops: 0,
                error: Some((0, err)),
            },
        }
    }
    fn vet(
        &self,
        source: &str,
        worlds: &[World],
    ) -> Result<(Compiled, u64, usize, usize), RulesError> {
        let compiled = script::compile(&self.engine, source, &self.limits)?;
        let saved = self.snapshot();
        let mut max_ops = 0;
        let mut actions = 0;
        let mut failure = None;
        for world in worlds {
            let case = self.run_case(&compiled, world);
            max_ops = max_ops.max(case.max_ops);
            actions += case.actions;
            if let Some((_, err)) = case.error {
                failure = Some(err);
                break;
            }
        }
        self.restore_unchecked(&saved);
        failure.map_or(Ok((compiled, max_ops, worlds.len(), actions)), Err)
    }
}

/// What one pass over a world produced.
struct WorldRun {
    actions: Vec<Emitted>,
    faults: Vec<Fault>,
    traces: Vec<ActorExplain>,
    max_ops: u64,
}

#[cfg(test)]
mod tick_digest_tests {
    //! Caller-visible [`TickReport`](super::TickReport) digest: same report
    //! digests identically, any action or fault change moves it.
    //!
    //! Consumer: a replay or tape caller that hashes one tick through
    //! `engine_rules::TickReport::digest` and compares across runs. Units:
    //! FNV-1a64 digests. Determinism: pure construction, no clock, no IO.
    //! Errors are panics via asserts.
    //!
    //! Why this module exists: `crates/rules/src/rules.rs` changed 3 times
    //! in 30 days yet held zero `#[test]` and the crate has no `tests/it/`
    //! module naming it, so a digest that dropped faults would still compile
    //! the crate but break every caller comparing ticks. Each vector below
    //! enters through the public `TickReport` shape with hand-built reports,
    //! so a behavior break fails here first.

    use super::{Fault, TickReport};
    use crate::{Action, Emitted, HostFault, RulesError};

    fn damage(actor: u8, target: u8) -> Emitted {
        Emitted {
            actor,
            action: Action::Damage {
                target,
                permille: 10,
            },
        }
    }

    fn fault(actor: u8, error: RulesError) -> Fault {
        Fault { actor, error }
    }

    #[test]
    fn empty_digest_repeats_and_differs_from_work() {
        let empty = TickReport::default();
        assert!(
            empty.actions.is_empty() && empty.faults.is_empty(),
            "baseline is empty via digest"
        );
        assert_eq!(
            empty.digest(),
            empty.digest(),
            "same empty report digests identically via digest"
        );
        let work = TickReport {
            actions: vec![damage(0, 1)],
            faults: Vec::new(),
        };
        assert_ne!(
            empty.digest(),
            work.digest(),
            "work moves the digest via digest"
        );
    }

    #[test]
    fn action_shape_actor_and_order_move_digest() {
        let base = TickReport {
            actions: vec![damage(0, 1)],
            faults: Vec::new(),
        };
        assert_eq!(
            base.digest(),
            base.digest(),
            "same actions digest identically via digest"
        );
        let heal = TickReport {
            actions: vec![Emitted {
                actor: 0,
                action: Action::Heal {
                    target: 1,
                    permille: 10,
                },
            }],
            faults: Vec::new(),
        };
        assert_ne!(
            base.digest(),
            heal.digest(),
            "damage vs heal moves the digest via digest"
        );
        let other_actor = TickReport {
            actions: vec![damage(1, 1)],
            faults: Vec::new(),
        };
        assert_ne!(
            base.digest(),
            other_actor.digest(),
            "emitting actor moves the digest via digest"
        );
        let other_target = TickReport {
            actions: vec![damage(0, 2)],
            faults: Vec::new(),
        };
        assert_ne!(
            base.digest(),
            other_target.digest(),
            "action target moves the digest via digest"
        );
        let ab = TickReport {
            actions: vec![damage(0, 1), damage(1, 2)],
            faults: Vec::new(),
        };
        let ba = TickReport {
            actions: vec![damage(1, 2), damage(0, 1)],
            faults: Vec::new(),
        };
        assert_ne!(
            ab.digest(),
            ba.digest(),
            "emission order moves the digest via digest"
        );
        assert_eq!(
            ab.digest(),
            ab.digest(),
            "ordered report repeats via digest"
        );
    }

    #[test]
    fn faults_move_digest_by_count_actor_and_kind() {
        let clean = TickReport {
            actions: vec![damage(0, 1)],
            faults: Vec::new(),
        };
        let one_fault = TickReport {
            actions: vec![damage(0, 1)],
            faults: vec![fault(1, RulesError::Host(HostFault::BadTarget(40)))],
        };
        assert_ne!(
            clean.digest(),
            one_fault.digest(),
            "fault count moves the digest via digest"
        );
        assert_eq!(
            one_fault.digest(),
            one_fault.digest(),
            "faulted report repeats via digest"
        );
        let other_actor = TickReport {
            actions: vec![damage(0, 1)],
            faults: vec![fault(2, RulesError::Host(HostFault::BadTarget(40)))],
        };
        assert_ne!(
            one_fault.digest(),
            other_actor.digest(),
            "fault actor moves the digest via digest"
        );
        let other_kind = TickReport {
            actions: vec![damage(0, 1)],
            faults: vec![fault(1, RulesError::Host(HostFault::ActionCap))],
        };
        assert_ne!(
            one_fault.digest(),
            other_kind.digest(),
            "fault kind moves the digest via digest"
        );
        let float_only = TickReport {
            actions: Vec::new(),
            faults: vec![fault(0, RulesError::Float { line: 1 })],
        };
        assert_ne!(
            clean.digest(),
            float_only.digest(),
            "fault-only report differs from clean via digest"
        );
    }
}

#[cfg(test)]
mod no_effect_tests {
    //! AD13: a script that fires no action in any smoke world is refused
    //! `NoEffect` unless the caller declares it a no-op, so an edit that paces
    //! nothing cannot pass as a rule change.
    use super::*;

    #[test]
    fn no_effect_script_is_refused_and_a_declared_no_op_installs() {
        let mut rules = Rules::new(Limits::default());
        // The row's own text: it defines a function and fires no action.
        let refused = rules.set("fn on_tick(me){let x=1;}", 0);
        assert!(refused.is_err(), "a do-nothing script refuses: {refused:?}");
        // A body that compiles and still fires nothing is a typed NoEffect.
        let err = rules.set("let x = 1;", 0).expect_err("no action fires");
        assert!(matches!(err, RulesError::NoEffect { .. }), "{err:?}");
        assert_eq!(rules.script_hash(), None, "a refusal installs nothing");
        assert_eq!(rules.history().len(), 0, "no revision was spent");
        // Declared a no-op, the same text installs.
        let ok = rules
            .set_allowing_no_op("let x = 1;", 0, true)
            .expect("a declared no-op installs");
        assert_eq!(ok.rev, 1);
        assert!(rules.script_hash().is_some());
        // An acting script still installs without the flag.
        let acting = rules
            .set("press(1);", 0)
            .expect("an acting script installs");
        assert_eq!(acting.rev, 2);
    }
}
