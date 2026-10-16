//! The whole surface a script can touch: read a fighter, read a few derived numbers, emit an
//! action. Nothing else is registered (the engine is built raw, see `script::build_engine`).
//!
//! Units: positions in millimetres, facing in centidegrees, health in permille, time in
//! fixed ticks. Every value a script sees or returns is an `i64`.

use std::cell::{Cell, RefCell};

use engine_core::mix64;
use rhai::{Dynamic, Engine, EvalAltResult, Position};

use crate::error::{HostFault, RulesError};
use crate::limits::{Limits, MAX_FIGHTERS, MEM_SLOTS};
use crate::rules::{ActorExplain, Fault};

/// Button bit: attack pressed this tick.
pub const BTN_ATTACK: u8 = 0x01;
/// Button bit: block held this tick.
pub const BTN_BLOCK: u8 = 0x02;
/// Button bit: dodge pressed this tick.
pub const BTN_DODGE: u8 = 0x04;

/// Largest damage, heal or stamina amount one action may carry (permille / points).
pub const MAX_AMOUNT: i64 = 1_000;
/// Longest stun or shield one action may set, in ticks (10 s at 60 Hz).
pub const MAX_TICKS: i64 = 600;
/// Largest push on one axis, in millimetres.
pub const MAX_PUSH_MM: i64 = 5_000;

/// What a script may read about one fighter. Plain integers, copied in per call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActorView {
    /// Slot, `0..MAX_FIGHTERS`.
    pub id: u8,
    /// Team; fighters on different teams are foes.
    pub team: u8,
    /// False once out of the fight; dead fighters run no script.
    pub alive: bool,
    /// Position east, millimetres.
    pub x_mm: i32,
    /// Position north, millimetres.
    pub z_mm: i32,
    /// Facing in centidegrees.
    pub facing_cdeg: i16,
    /// Health, permille.
    pub hp_permille: u16,
    /// Stamina points.
    pub stamina: u16,
    /// Ticks since the attack started.
    pub attack_age: u32,
    /// Attack in flight.
    pub attacking: bool,
    /// Block held.
    pub blocking: bool,
    /// Dodge in flight.
    pub dodging: bool,
    /// Stagger ticks left.
    pub react: u32,
    /// Knockdown ticks left.
    pub knockdown: u32,
    /// Shield ticks left (rule-granted guard).
    pub shield: u32,
    /// Forward wish this tick.
    pub mx: i8,
    /// Right wish this tick.
    pub mz: i8,
    /// Button bits this tick (`BTN_*`).
    pub buttons: u8,
}

/// One tick's input to the rules: the tick number, the match seed and every fighter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct World {
    /// Zero-based fixed tick.
    pub tick: u32,
    /// Match seed (feeds `roll`).
    pub seed: u64,
    /// Actors in slot order; `fighters[i].id == i`.
    pub fighters: Vec<ActorView>,
}

/// An effect a script asked for. The arena applies these in order; rules never write state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Remove health, permille.
    Damage {
        /// Actor hit.
        target: u8,
        /// Permille, `0..=MAX_AMOUNT`.
        permille: u16,
    },
    /// Restore health, permille.
    Heal {
        /// Actor healed.
        target: u8,
        /// Permille, `0..=MAX_AMOUNT`.
        permille: u16,
    },
    /// Set stagger ticks.
    Stun {
        /// Actor staggered.
        target: u8,
        /// Ticks, `0..=MAX_TICKS`.
        ticks: u16,
    },
    /// Shove a fighter.
    Push {
        /// Actor shoved.
        target: u8,
        /// East, millimetres.
        dx_mm: i32,
        /// North, millimetres.
        dz_mm: i32,
    },
    /// Remove stamina.
    DrainStamina {
        /// Actor drained.
        target: u8,
        /// Points, `0..=MAX_AMOUNT`.
        amount: u16,
    },
    /// Grant a rule shield.
    Shield {
        /// Actor shielded.
        target: u8,
        /// Ticks, `0..=MAX_TICKS`.
        ticks: u16,
    },
    /// Input: the emitting actor's movement wish this tick (`walk`). Used by a controller
    /// that drives a real arena slot; the reference arena ignores the three input actions.
    Wish {
        /// Forward wish, `-100..=100`.
        mx: i8,
        /// Right wish, `-100..=100`.
        mz: i8,
    },
    /// Input: the emitting actor's facing this tick, centidegrees (`face`).
    Face {
        /// `-18000..=18000`.
        cdeg: i16,
    },
    /// Input: the emitting actor's button bits this tick (`press`).
    Press {
        /// `BTN_*` bits, `0..=7`.
        buttons: u8,
    },
}

impl Action {
    /// True for the three input actions (`Wish`, `Face`, `Press`): they write the emitting
    /// actor's own input, never another fighter's state.
    pub const fn is_input(&self) -> bool {
        matches!(
            self,
            Self::Wish { .. } | Self::Face { .. } | Self::Press { .. }
        )
    }
}

impl Action {
    /// Fold into a hasher: tag byte, then fields little-endian (layout frozen).
    pub fn write_into(&self, hasher: &mut engine_core::Fnv64) {
        match *self {
            Self::Damage { target, permille } => {
                hasher.write_u8(1);
                hasher.write_u8(target);
                hasher.write_u16(permille);
            }
            Self::Heal { target, permille } => {
                hasher.write_u8(2);
                hasher.write_u8(target);
                hasher.write_u16(permille);
            }
            Self::Stun { target, ticks } => {
                hasher.write_u8(3);
                hasher.write_u8(target);
                hasher.write_u16(ticks);
            }
            Self::Push {
                target,
                dx_mm,
                dz_mm,
            } => {
                hasher.write_u8(4);
                hasher.write_u8(target);
                hasher.write_i32(dx_mm);
                hasher.write_i32(dz_mm);
            }
            Self::DrainStamina { target, amount } => {
                hasher.write_u8(5);
                hasher.write_u8(target);
                hasher.write_u16(amount);
            }
            Self::Shield { target, ticks } => {
                hasher.write_u8(6);
                hasher.write_u8(target);
                hasher.write_u16(ticks);
            }
            Self::Wish { mx, mz } => {
                hasher.write_u8(7);
                hasher.write_u8(mx.to_le_bytes()[0]);
                hasher.write_u8(mz.to_le_bytes()[0]);
            }
            Self::Face { cdeg } => {
                hasher.write_u8(8);
                hasher.write_u16(u16::from_le_bytes(cdeg.to_le_bytes()));
            }
            Self::Press { buttons } => {
                hasher.write_u8(9);
                hasher.write_u8(buttons);
            }
        }
    }
}

/// An action plus the fighter whose script emitted it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Emitted {
    /// Actor whose `on_tick` ran.
    pub actor: u8,
    /// The effect.
    pub action: Action,
}

/// One host call made by a script, kept only while explaining.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCall {
    /// Host function name.
    pub name: &'static str,
    /// Up to three arguments (unused ones are 0).
    pub args: [i64; 3],
    /// Value returned (0 for actions).
    pub ret: i64,
}

/// Variables every script sees, re-bound for each actor before its script runs (read-only
/// constants: `hp`, `blocking`, ...). The order is part of the ABI; `prop_value` follows it.
/// Only the names a script reads are bound, so an early-exit script pays for a few of them.
pub const PROPS: &[&str] = &[
    "id",
    "team",
    "x",
    "z",
    "facing",
    "hp",
    "stamina",
    "attack_age",
    "attacking",
    "blocking",
    "dodging",
    "react",
    "knockdown",
    "shielded",
    "mx",
    "mz",
    "buttons",
    "attack",
    "block",
    "dodge",
];

/// The value of `PROPS[index]` for `view`; an index past the list gives `()`.
pub(crate) fn prop_value(index: usize, view: &ActorView) -> Dynamic {
    match index {
        0 => Dynamic::from(i64::from(view.id)),
        1 => Dynamic::from(i64::from(view.team)),
        2 => Dynamic::from(i64::from(view.x_mm)),
        3 => Dynamic::from(i64::from(view.z_mm)),
        4 => Dynamic::from(i64::from(view.facing_cdeg)),
        5 => Dynamic::from(i64::from(view.hp_permille)),
        6 => Dynamic::from(i64::from(view.stamina)),
        7 => Dynamic::from(i64::from(view.attack_age)),
        8 => Dynamic::from(view.attacking),
        9 => Dynamic::from(view.blocking),
        10 => Dynamic::from(view.dodging),
        11 => Dynamic::from(i64::from(view.react)),
        12 => Dynamic::from(i64::from(view.knockdown)),
        13 => Dynamic::from(i64::from(view.shield)),
        14 => Dynamic::from(i64::from(view.mx)),
        15 => Dynamic::from(i64::from(view.mz)),
        16 => Dynamic::from(i64::from(view.buttons)),
        17 => Dynamic::from(view.buttons & BTN_ATTACK != 0),
        18 => Dynamic::from(view.buttons & BTN_BLOCK != 0),
        19 => Dynamic::from(view.buttons & BTN_DODGE != 0),
        _ => Dynamic::UNIT,
    }
}
/// Host functions a script may call, with their argument counts. The static check refuses
/// every other name; `rules_abi` hashes this list into the tape header.
pub const HOST_FNS: &[(&str, usize)] = &[
    ("tick", 0),
    ("hp_of", 1),
    ("stamina_of", 1),
    ("team_of", 1),
    ("alive", 1),
    ("x_of", 1),
    ("z_of", 1),
    ("dist", 2),
    ("nearest_foe", 1),
    ("foes_within", 2),
    ("roll", 1),
    ("clamp", 3),
    ("mem_get", 1),
    ("mem_set", 2),
    ("damage", 2),
    ("heal", 2),
    ("stun", 2),
    ("push", 3),
    ("drain_stamina", 2),
    ("shield", 2),
];

/// Input host functions, added after ABI 3 without bumping it: they are additive (a script
/// that never calls them replays exactly as before, and a tape names its scripts by hash), so
/// they are left out of `rules_abi`. They write the calling actor's own input.
pub const INPUT_FNS: &[(&str, usize)] = &[("walk", 2), ("face", 1), ("press", 1)];

/// Largest wish on one axis, percent of top speed.
pub const MAX_WISH: i64 = 100;
/// Largest absolute facing, centidegrees.
pub const MAX_FACING_CDEG: i64 = 18_000;
/// All three button bits.
pub const BTN_ALL: i64 = 0x07;

/// State shared between the engine's registered closures and the `Rules` owner.
/// Single-threaded by construction (Rhai without `sync` is `!Send`).
pub(crate) struct HostCtx {
    pub(crate) world: RefCell<Vec<ActorView>>,
    pub(crate) tick: Cell<u32>,
    pub(crate) seed: Cell<u64>,
    pub(crate) me: Cell<u8>,
    pub(crate) rolls: Cell<u64>,
    /// Actions of the actor now running (moved to `out_actions` when it finishes cleanly).
    pub(crate) actions: RefCell<Vec<Action>>,
    pub(crate) mem: RefCell<Vec<[i64; MEM_SLOTS]>>,
    pub(crate) trace: Cell<bool>,
    pub(crate) calls: RefCell<Vec<HostCall>>,
    pub(crate) max_actions: usize,
    /// True between `begin` and the actor's flush or failure.
    in_actor: Cell<bool>,
    /// Memory writes of the actor now running, undone if it fails: (actor, slot, old value).
    undo: RefCell<Vec<(u8, usize, i64)>>,
    /// Actions of every actor that finished cleanly this run, in actor order.
    out_actions: RefCell<Vec<Emitted>>,
    /// Keep a per-actor trace (explain mode).
    pub(crate) collect: Cell<bool>,
    traces: RefCell<Vec<ActorExplain>>,
    /// Operation count last reported by the counting engine for the running actor, and the
    /// most any actor used this run.
    last_ops: Cell<u64>,
    max_seen: Cell<u64>,
}

impl HostCtx {
    pub(crate) fn new(limits: &Limits) -> Self {
        Self {
            world: RefCell::new(Vec::with_capacity(MAX_FIGHTERS)),
            tick: Cell::new(0),
            seed: Cell::new(0),
            me: Cell::new(0),
            rolls: Cell::new(0),
            actions: RefCell::new(Vec::with_capacity(limits.max_actions_per_call)),
            mem: RefCell::new(vec![[0; MEM_SLOTS]; MAX_FIGHTERS]),
            trace: Cell::new(false),
            calls: RefCell::new(Vec::new()),
            max_actions: limits.max_actions_per_call,
            in_actor: Cell::new(false),
            undo: RefCell::new(Vec::new()),
            out_actions: RefCell::new(Vec::new()),
            collect: Cell::new(false),
            traces: RefCell::new(Vec::new()),
            last_ops: Cell::new(0),
            max_seen: Cell::new(0),
        }
    }

    /// Start a run over a world: clear everything the previous run left.
    pub(crate) fn reset_run(&self, collect: bool, trace: bool) {
        self.in_actor.set(false);
        self.undo.borrow_mut().clear();
        self.actions.borrow_mut().clear();
        self.out_actions.borrow_mut().clear();
        self.traces.borrow_mut().clear();
        self.calls.borrow_mut().clear();
        self.collect.set(collect);
        self.trace.set(trace);
        self.max_seen.set(0);
        self.last_ops.set(0);
    }

    /// Engine progress hook of the counting engine: remember the actor's operation count so far.
    pub(crate) fn progress(&self, ops: u64) {
        self.last_ops.set(ops);
    }

    /// The actor `id` starts: reset per-actor state.
    pub(crate) fn begin(&self, id: u8) {
        self.me.set(id);
        self.rolls.set(0);
        self.last_ops.set(0);
        self.actions.borrow_mut().clear();
        self.undo.borrow_mut().clear();
        // The call log only grows while tracing or collecting; a plain tick never reads it.
        if self.collect.get() || self.trace.get() {
            self.calls.borrow_mut().clear();
        }
        self.in_actor.set(true);
    }

    fn take_trace(&self, error: Option<RulesError>, actions: Vec<Action>) {
        let id = self.me.get();
        let ops = self.last_ops.get();
        self.max_seen.set(self.max_seen.get().max(ops));
        if self.collect.get() {
            let mem_after = self.mem.borrow()[usize::from(id)];
            let calls = std::mem::take(&mut *self.calls.borrow_mut());
            self.traces.borrow_mut().push(ActorExplain {
                id,
                ops,
                calls,
                actions,
                error,
                mem_after,
            });
        }
    }

    /// The running actor finished without error: keep its actions and memory writes.
    pub(crate) fn flush_ok(&self) {
        if !self.in_actor.replace(false) {
            return;
        }
        let id = self.me.get();
        let mut pending = self.actions.borrow_mut();
        let kept: Vec<Action> = if self.collect.get() {
            pending.drain(..).collect()
        } else {
            Vec::new()
        };
        let mut out = self.out_actions.borrow_mut();
        if self.collect.get() {
            out.extend(kept.iter().map(|&action| Emitted { actor: id, action }));
        } else {
            out.extend(
                pending
                    .drain(..)
                    .map(|action| Emitted { actor: id, action }),
            );
        }
        drop(out);
        drop(pending);
        self.undo.borrow_mut().clear();
        self.take_trace(None, kept);
    }
    /// The running actor failed: drop its actions, undo its memory writes, report who it was.
    /// `None` when no actor was running (the failure is the driver's own).
    pub(crate) fn fail(&self, error: &RulesError) -> Option<Fault> {
        if !self.in_actor.replace(false) {
            return None;
        }
        let actor = self.me.get();
        self.actions.borrow_mut().clear();
        {
            let mut mem = self.mem.borrow_mut();
            for (who, slot, old) in self.undo.borrow_mut().drain(..).rev() {
                mem[usize::from(who)][slot] = old;
            }
        }
        self.take_trace(Some(error.clone()), Vec::new());
        Some(Fault {
            actor,
            error: error.clone(),
        })
    }

    /// Everything the run produced: actions in actor order, traces, the most ops one actor used.
    pub(crate) fn finish_run(&self) -> (Vec<Emitted>, Vec<ActorExplain>, u64) {
        self.trace.set(false);
        self.collect.set(false);
        (
            std::mem::take(&mut *self.out_actions.borrow_mut()),
            std::mem::take(&mut *self.traces.borrow_mut()),
            self.max_seen.get(),
        )
    }

    fn note_call(&self, name: &'static str, args: [i64; 3], ret: i64) {
        if self.trace.get() {
            self.calls.borrow_mut().push(HostCall { name, args, ret });
        }
    }

    fn view(&self, id: i64) -> Result<ActorView, Box<EvalAltResult>> {
        let world = self.world.borrow();
        lookup(&world, id)
    }

    fn emit(&self, action: Action) -> Result<(), Box<EvalAltResult>> {
        let mut actions = self.actions.borrow_mut();
        if actions.len() >= self.max_actions {
            return Err(fault(HostFault::ActionCap));
        }
        actions.push(action);
        Ok(())
    }
}

#[expect(
    clippy::unnecessary_box_returns,
    reason = "Rhai host calls fail with Box<EvalAltResult>; this is the one place that builds it"
)]
fn fault(kind: HostFault) -> Box<EvalAltResult> {
    EvalAltResult::ErrorRuntime(Dynamic::from(kind), Position::NONE).into()
}

fn ranged(call: &'static str, value: i64, lo: i64, hi: i64) -> Result<i64, Box<EvalAltResult>> {
    if value < lo || value > hi {
        return Err(fault(HostFault::BadArg { call, value }));
    }
    Ok(value)
}

/// Target id check: any slot of the current world.
fn target(ctx: &HostCtx, id: i64) -> Result<u8, Box<EvalAltResult>> {
    ctx.view(id).map(|view| view.id)
}

/// Distance between two fighters in millimetres, floor.
pub(crate) fn distance_mm(a: &ActorView, b: &ActorView) -> i64 {
    i64::try_from(squared_dist_mm(a, b).isqrt()).unwrap_or(i64::MAX)
}

/// Squared distance in mm squared, exact (each axis fits i128 with room, never negative).
/// Scans compare squares instead of roots: the order matches the floored root exactly,
/// ties included, and no scan takes a square root.
pub(crate) fn squared_dist_mm(a: &ActorView, b: &ActorView) -> u128 {
    let dx = i128::from(a.x_mm) - i128::from(b.x_mm);
    let dz = i128::from(a.z_mm) - i128::from(b.z_mm);
    dx.unsigned_abs()
        .pow(2)
        .saturating_add(dz.unsigned_abs().pow(2))
}

/// Register the host functions into a raw engine.
pub(crate) fn register(engine: &mut Engine, ctx: &std::rc::Rc<HostCtx>) {
    register_reads(engine, ctx);
    register_actions(engine, ctx);
    register_inputs(engine, ctx);
}

fn register_inputs(engine: &mut Engine, ctx: &std::rc::Rc<HostCtx>) {
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "walk",
        move |mx: i64, mz: i64| -> Result<(), Box<EvalAltResult>> {
            let x = ranged("walk", mx, -MAX_WISH, MAX_WISH)?;
            let z = ranged("walk", mz, -MAX_WISH, MAX_WISH)?;
            c.note_call("walk", [mx, mz, 0], 0);
            c.emit(Action::Wish {
                mx: i8::try_from(x).unwrap_or(0),
                mz: i8::try_from(z).unwrap_or(0),
            })
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn("face", move |cdeg: i64| -> Result<(), Box<EvalAltResult>> {
        let f = ranged("face", cdeg, -MAX_FACING_CDEG, MAX_FACING_CDEG)?;
        c.note_call("face", [cdeg, 0, 0], 0);
        c.emit(Action::Face {
            cdeg: i16::try_from(f).unwrap_or(0),
        })
    });
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "press",
        move |buttons: i64| -> Result<(), Box<EvalAltResult>> {
            let b = ranged("press", buttons, 0, BTN_ALL)?;
            c.note_call("press", [buttons, 0, 0], 0);
            c.emit(Action::Press {
                buttons: u8::try_from(b).unwrap_or(0),
            })
        },
    );
}
fn register_reads(engine: &mut Engine, ctx: &std::rc::Rc<HostCtx>) {
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn("tick", move || -> i64 {
        let ret = i64::from(c.tick.get());
        c.note_call("tick", [0; 3], ret);
        ret
    });
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn("hp_of", move |id: i64| -> Result<i64, Box<EvalAltResult>> {
        let ret = i64::from(c.view(id)?.hp_permille);
        c.note_call("hp_of", [id, 0, 0], ret);
        Ok(ret)
    });
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "stamina_of",
        move |id: i64| -> Result<i64, Box<EvalAltResult>> {
            let ret = i64::from(c.view(id)?.stamina);
            c.note_call("stamina_of", [id, 0, 0], ret);
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "team_of",
        move |id: i64| -> Result<i64, Box<EvalAltResult>> {
            let ret = i64::from(c.view(id)?.team);
            c.note_call("team_of", [id, 0, 0], ret);
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "alive",
        move |id: i64| -> Result<bool, Box<EvalAltResult>> {
            let ret = c.view(id)?.alive;
            c.note_call("alive", [id, 0, 0], i64::from(ret));
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn("x_of", move |id: i64| -> Result<i64, Box<EvalAltResult>> {
        let ret = i64::from(c.view(id)?.x_mm);
        c.note_call("x_of", [id, 0, 0], ret);
        Ok(ret)
    });
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn("z_of", move |id: i64| -> Result<i64, Box<EvalAltResult>> {
        let ret = i64::from(c.view(id)?.z_mm);
        c.note_call("z_of", [id, 0, 0], ret);
        Ok(ret)
    });
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "dist",
        move |a: i64, b: i64| -> Result<i64, Box<EvalAltResult>> {
            let world = c.world.borrow();
            let first = lookup(&world, a)?;
            let second = lookup(&world, b)?;
            let ret = distance_mm(&first, &second);
            c.note_call("dist", [a, b, 0], ret);
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "nearest_foe",
        move |id: i64| -> Result<i64, Box<EvalAltResult>> {
            let world = c.world.borrow();
            let me = lookup(&world, id)?;
            // Lowest id wins a tie: the scan is in slot order and only a strictly closer foe replaces.
            // Squared distances order exactly like the root, so this scan takes no square root.
            let mut best: Option<(u128, u8)> = None;
            for other in world.iter().filter(|o| o.alive && o.team != me.team) {
                let squared = squared_dist_mm(&me, other);
                if best.is_none_or(|(bs, _)| squared < bs) {
                    best = Some((squared, other.id));
                }
            }
            let ret = best.map_or(-1, |(_, foe)| i64::from(foe));
            c.note_call("nearest_foe", [id, 0, 0], ret);
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "foes_within",
        move |id: i64, radius: i64| -> Result<i64, Box<EvalAltResult>> {
            let world = c.world.borrow();
            let me = lookup(&world, id)?;
            if radius < 0 {
                c.note_call("foes_within", [id, radius, 0], 0);
                return Ok(0);
            }
            // floor(sqrt(s)) <= radius ⟺ s < (radius + 1)^2: exact, no root per foe.
            let limit = (u128::from(radius.unsigned_abs()) + 1).pow(2);
            let count = world
                .iter()
                .filter(|o| o.alive && o.team != me.team && squared_dist_mm(&me, o) < limit)
                .count();
            let ret = i64::try_from(count).unwrap_or(i64::MAX);
            c.note_call("foes_within", [id, radius, 0], ret);
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn("roll", move |n: i64| -> Result<i64, Box<EvalAltResult>> {
        let n = ranged("roll", n, 1, 1_000_000)?;
        let counter = c.rolls.get();
        c.rolls.set(counter + 1);
        let mixed = mix64(
            c.seed.get()
                ^ mix64((u64::from(c.tick.get()) << 24) ^ (u64::from(c.me.get()) << 16) ^ counter),
        );
        // n is in 1..=1_000_000 so the remainder fits i64.
        let ret = i64::try_from(mixed % n.unsigned_abs()).unwrap_or(0);
        c.note_call("roll", [n, 0, 0], ret);
        Ok(ret)
    });
    engine.register_fn("clamp", |v: i64, lo: i64, hi: i64| -> i64 {
        v.max(lo).min(hi.max(lo))
    });
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "mem_get",
        move |slot: i64| -> Result<i64, Box<EvalAltResult>> {
            let index = mem_index(slot)?;
            let ret = c.mem.borrow()[usize::from(c.me.get())][index];
            c.note_call("mem_get", [slot, 0, 0], ret);
            Ok(ret)
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "mem_set",
        move |slot: i64, value: i64| -> Result<(), Box<EvalAltResult>> {
            let index = mem_index(slot)?;
            let me = c.me.get();
            let old = std::mem::replace(&mut c.mem.borrow_mut()[usize::from(me)][index], value);
            c.undo.borrow_mut().push((me, index, old));
            c.note_call("mem_set", [slot, value, 0], 0);
            Ok(())
        },
    );
}

fn lookup(world: &[ActorView], id: i64) -> Result<ActorView, Box<EvalAltResult>> {
    usize::try_from(id)
        .ok()
        .and_then(|index| world.get(index).copied())
        .ok_or_else(|| fault(HostFault::BadTarget(id)))
}

fn mem_index(slot: i64) -> Result<usize, Box<EvalAltResult>> {
    usize::try_from(slot)
        .ok()
        .filter(|index| *index < MEM_SLOTS)
        .ok_or_else(|| fault(HostFault::MemSlot(slot)))
}

fn register_actions(engine: &mut Engine, ctx: &std::rc::Rc<HostCtx>) {
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "damage",
        move |id: i64, permille: i64| -> Result<(), Box<EvalAltResult>> {
            let p = ranged("damage", permille, 0, MAX_AMOUNT)?;
            let t = target(&c, id)?;
            c.note_call("damage", [id, permille, 0], 0);
            c.emit(Action::Damage {
                target: t,
                permille: to_u16(p),
            })
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "heal",
        move |id: i64, permille: i64| -> Result<(), Box<EvalAltResult>> {
            let p = ranged("heal", permille, 0, MAX_AMOUNT)?;
            let t = target(&c, id)?;
            c.note_call("heal", [id, permille, 0], 0);
            c.emit(Action::Heal {
                target: t,
                permille: to_u16(p),
            })
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "stun",
        move |id: i64, ticks: i64| -> Result<(), Box<EvalAltResult>> {
            let n = ranged("stun", ticks, 0, MAX_TICKS)?;
            let t = target(&c, id)?;
            c.note_call("stun", [id, ticks, 0], 0);
            c.emit(Action::Stun {
                target: t,
                ticks: to_u16(n),
            })
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "push",
        move |id: i64, dx: i64, dz: i64| -> Result<(), Box<EvalAltResult>> {
            let x = ranged("push", dx, -MAX_PUSH_MM, MAX_PUSH_MM)?;
            let z = ranged("push", dz, -MAX_PUSH_MM, MAX_PUSH_MM)?;
            let t = target(&c, id)?;
            c.note_call("push", [id, dx, dz], 0);
            // Both axes are within +/-5000, so they fit i32.
            c.emit(Action::Push {
                target: t,
                dx_mm: i32::try_from(x).unwrap_or(0),
                dz_mm: i32::try_from(z).unwrap_or(0),
            })
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "drain_stamina",
        move |id: i64, amount: i64| -> Result<(), Box<EvalAltResult>> {
            let a = ranged("drain_stamina", amount, 0, MAX_AMOUNT)?;
            let t = target(&c, id)?;
            c.note_call("drain_stamina", [id, amount, 0], 0);
            c.emit(Action::DrainStamina {
                target: t,
                amount: to_u16(a),
            })
        },
    );
    let c = std::rc::Rc::clone(ctx);
    engine.register_fn(
        "shield",
        move |id: i64, ticks: i64| -> Result<(), Box<EvalAltResult>> {
            let n = ranged("shield", ticks, 0, MAX_TICKS)?;
            let t = target(&c, id)?;
            c.note_call("shield", [id, ticks, 0], 0);
            c.emit(Action::Shield {
                target: t,
                ticks: to_u16(n),
            })
        },
    );
}

/// Convert a value already range-checked to `0..=1000`.
fn to_u16(value: i64) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_and_fns_have_no_duplicates() {
        let mut names: Vec<_> = HOST_FNS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), HOST_FNS.len());
        let mut props = PROPS.to_vec();
        props.sort_unstable();
        props.dedup();
        assert_eq!(props.len(), PROPS.len());
    }
}
