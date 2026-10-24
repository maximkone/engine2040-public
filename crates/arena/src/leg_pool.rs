//! RC36 (decision D6): the per-fighter step runs on one shared job pool.
//!
//! One slot's leg — the IN1 wish into LOC1, its requested root delta through
//! PH2 collision — reads this tick's shared, read-only decisions and writes
//! only that slot's own fields. So the slots can run at the same time. A lane
//! owns a contiguous slot range: it holds disjoint `&mut` slices of every
//! field the leg writes, writes only its own index range, and the merge after
//! the join applies the results in slot order. Bytes are therefore equal at
//! every thread count, and a one-lane run is the old sequential loop.
//!
//! The pool is the process one ([`engine_ecs::pool::shared`]), sized by
//! `E2040_THREADS`: `1` keeps every job inline on the caller.

#[cfg(test)]
use super::TickMotion;
use super::{
    canon_zero, interrupt_for_slot, loco_wish, wall_limit, yaw_from_centideg, CombatInterrupt,
    FightEvent, FighterInput, FighterState, GaitPhase, GroundWarp, LocalPose, Locomotion, Sim,
    JUMP_RISE_MM_TICK, LOCO_LEG_JOINTS, MAX_SLOTS, THROW_SPEAR, TICK_DT,
};
#[cfg(feature = "sim")]
use super::{Controller, FIXED_DT};
#[cfg(not(feature = "sim"))]
use super::{MatchWorld, FIGHTER_RADIUS_M};
use crate::spear::SpearState;
use engine_input::fighter::THROW_NOOSE;
// Test-only counter of ticks that really fanned the leg out over the pool.
// Per thread, so a test proves its own sim forked and never reads another
// test's count.
#[cfg(test)]
thread_local! {
    static FORKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// How many ticks this thread has fanned out over a pool.
#[cfg(test)]
pub(crate) fn forks() -> u32 {
    FORKS.with(std::cell::Cell::get)
}

/// One slot's leg inputs, decided once before the lanes start.
#[derive(Clone, Copy)]
struct LegIn {
    /// IN1 wish this tick, after the pilum guard mask and the brain merge.
    input: FighterInput,
    /// False on a knocked-out fighter: its pose holds and its root freezes.
    alive: bool,
    /// Fallen or catching (EG34): the wish stops and the root holds.
    down: bool,
    /// Fresh jump press this tick (AIN7).
    jump_edge: bool,
    /// Jump released this tick: an early cut halves the rise (AIN6).
    jump_release: bool,
    /// Combat interrupt the locomotion machine takes this tick.
    interrupt: Option<CombatInterrupt>,
}

/// One slot's leg result, applied to the sim in slot order.
#[derive(Clone, Copy, Default)]
struct LegOut {
    /// Requested root delta (x, z) for the ORCA channel, in metres.
    requested: [f32; 2],
    /// False when the locomotion machine refused the step.
    ok: bool,
    /// Test-only per-slot collision record.
    #[cfg(test)]
    motion: TickMotion,
}

/// One slot's mutable leg state, borrowed from its lane.
struct LegSlot<'a> {
    state: &'a mut FighterState,
    loco: &'a mut Locomotion,
    /// Ground-feel warp; `Some` only on presentation runs (`record`).
    warp: Option<&'a mut GroundWarp>,
    /// Height channel vertical speed in mm per tick; `Some` only while the
    /// channel is armed.
    rise: Option<&'a mut i32>,
    /// This slot's recorded poses; `Some` only on presentation runs.
    history: Option<&'a mut Vec<LocalPose>>,
    #[cfg(feature = "sim")]
    controller: &'a mut Controller,
}

/// The tick's shared, read-only leg context: equal for every slot.
struct LegCtx {
    tick: u32,
    /// Wall-inset bound shared by both collision legs.
    limit: f32,
    /// True when this run records locomotion poses.
    record: bool,
    /// True once the height channel is armed.
    height: bool,
    /// True when the jump channel armed this tick (one shared write, hoisted).
    jump_armed: bool,
}

/// A contiguous slot range a lane owns, with disjoint borrows of every field
/// the leg writes. Splitting hands each lane its own range, so a lane holds no
/// lock and touches no other lane's slot.
struct LegLane<'a> {
    /// Absolute slot index of this lane's first slot.
    begin: usize,
    states: &'a mut [FighterState],
    locos: &'a mut [Locomotion],
    warps: &'a mut [GroundWarp],
    rises: &'a mut [i32],
    histories: Option<&'a mut [Vec<LocalPose>]>,
    #[cfg(feature = "sim")]
    controllers: &'a mut [Controller],
    outs: &'a mut [LegOut],
}

impl<'a> LegLane<'a> {
    fn slots(&self) -> usize {
        self.states.len()
    }

    /// Split into two lanes at `at` slots. `warps` and `rises` are shorter
    /// (empty or absent) when their channel is off, and are then never
    /// indexed, so they split at their own end.
    fn split_at(self, at: usize) -> (Self, Self) {
        let (states_a, states_b) = self.states.split_at_mut(at);
        let (locos_a, locos_b) = self.locos.split_at_mut(at);
        let (warps_a, warps_b) = self.warps.split_at_mut(at.min(self.warps.len()));
        let (rises_a, rises_b) = self.rises.split_at_mut(at.min(self.rises.len()));
        let (histories_a, histories_b) = match self.histories {
            Some(history) => {
                let (a, b) = history.split_at_mut(at.min(history.len()));
                (Some(a), Some(b))
            }
            None => (None, None),
        };
        #[cfg(feature = "sim")]
        let (controllers_a, controllers_b) = self.controllers.split_at_mut(at);
        let (outs_a, outs_b) = self.outs.split_at_mut(at);
        (
            Self {
                begin: self.begin,
                states: states_a,
                locos: locos_a,
                warps: warps_a,
                rises: rises_a,
                histories: histories_a,
                #[cfg(feature = "sim")]
                controllers: controllers_a,
                outs: outs_a,
            },
            Self {
                begin: self.begin + at,
                states: states_b,
                locos: locos_b,
                warps: warps_b,
                rises: rises_b,
                histories: histories_b,
                #[cfg(feature = "sim")]
                controllers: controllers_b,
                outs: outs_b,
            },
        )
    }
}

/// Split `slots` into at most `lanes` contiguous ranges by halving the widest
/// each time. Returns the ranges and how many were made: slot order is kept
/// and every slot lands in exactly one range. Stack only, because the
/// authority path must not allocate a byte per tick.
fn lane_bounds(slots: usize, lanes: usize) -> ([(usize, usize); MAX_SLOTS], usize) {
    let mut bounds = [(0usize, 0usize); MAX_SLOTS];
    bounds[0] = (0, slots);
    let mut count = 1;
    while count < lanes && count < MAX_SLOTS {
        let mut widest = 0;
        for index in 1..count {
            if bounds[index].1 > bounds[widest].1 {
                widest = index;
            }
        }
        let (start, len) = bounds[widest];
        let half = len / 2;
        if half == 0 {
            break;
        }
        for index in (widest + 1..=count).rev() {
            bounds[index] = bounds[index - 1];
        }
        bounds[widest] = (start, half);
        bounds[widest + 1] = (start + half, len - half);
        count += 1;
    }
    (bounds, count)
}

/// Run the per-fighter leg for every slot this tick, on the shared pool when
/// it has a worker, inline on the caller otherwise. `None` when a locomotion
/// machine refused the step (the step then fails, as the sequential loop did).
pub(crate) fn step_legs(
    sim: &mut Sim,
    inputs: &[FighterInput],
    jump_edges: &[bool; MAX_SLOTS],
    jump_releases: &[bool; MAX_SLOTS],
    events: &[FightEvent],
    #[cfg(test)] trace: &mut Vec<TickMotion>,
) -> Option<()> {
    let fighters = sim.fighters;
    let (legs, jump_armed) = legs_of_tick(sim, inputs, jump_edges, jump_releases, events)?;
    let mut outs = [LegOut::default(); MAX_SLOTS];
    run_legs(sim, &legs, jump_armed, &mut outs, fighters);
    merge_legs(
        sim,
        &legs,
        &outs,
        fighters,
        #[cfg(test)]
        trace,
    )
}

/// Decide every slot's leg inputs once. Then arm the height channel if any
/// slot takes off this tick: arming zero-fills the rise vector, so the slot
/// that used to arm it mid-loop and every later slot read the same channel.
fn legs_of_tick(
    sim: &mut Sim,
    inputs: &[FighterInput],
    jump_edges: &[bool; MAX_SLOTS],
    jump_releases: &[bool; MAX_SLOTS],
    events: &[FightEvent],
) -> Option<([Option<LegIn>; MAX_SLOTS], bool)> {
    let fighters = sim.fighters;
    let mut legs = [None; MAX_SLOTS];
    for slot in 0..fighters {
        let input = inputs.get(slot).copied()?;
        let alive = sim.fight.fighters[slot].hp_permille != 0;
        let down = alive
            && sim.rules.balance
            && sim.balance.get(slot).is_some_and(|state| {
                matches!(state.gait(), GaitPhase::Fallen | GaitPhase::Catching)
            });
        legs[slot] = Some(LegIn {
            input,
            alive,
            down,
            jump_edge: jump_edges[slot],
            jump_release: jump_releases[slot],
            interrupt: interrupt_for_slot(events, u8::try_from(slot).ok()?),
        });
    }
    let wants_jump = legs[..fighters]
        .iter()
        .flatten()
        .any(|leg| leg.alive && leg.jump_edge && !leg.down);
    let jump_armed = wants_jump && sim.arm_jump();
    Some((legs, jump_armed))
}

/// Cut the tick into lanes and run them: the first lane on the caller, the
/// rest on the pool's workers.
fn run_legs(
    sim: &mut Sim,
    legs: &[Option<LegIn>; MAX_SLOTS],
    jump_armed: bool,
    outs: &mut [LegOut; MAX_SLOTS],
    fighters: usize,
) {
    let ctx = LegCtx {
        tick: sim.tick,
        limit: wall_limit(sim.config.half_extent_m),
        record: sim.loco_history.is_some(),
        height: sim.height,
        jump_armed,
    };
    #[cfg(not(feature = "sim"))]
    let world = &sim.world;
    let pool = sim.pool;
    let warp_len = sim.warps.len().min(fighters);
    let rise_len = sim.rise.len().min(fighters);
    let lane = LegLane {
        begin: 0,
        states: &mut sim.states[..fighters],
        locos: &mut sim.locos[..fighters],
        warps: &mut sim.warps[..warp_len],
        rises: &mut sim.rise[..rise_len],
        histories: sim.loco_history.as_deref_mut(),
        #[cfg(feature = "sim")]
        controllers: &mut sim.controllers[..fighters],
        outs: &mut outs[..fighters],
    };
    // Fork only where the leg carries pose work. An authority tick must stay
    // at zero heap per tick (the steady-tick alloc floor): the pool's scope
    // state and its job boxes are heap. Presentation ticks record poses, so
    // they already allocate their history rows, and they are the ticks the
    // fork pays for.
    let lanes = match pool {
        Some(pool) if ctx.record => (pool.workers() + 1).min(fighters),
        _ => 1,
    };
    let Some(pool) = pool.filter(|_| lanes > 1) else {
        run_lane(
            &ctx,
            legs,
            #[cfg(not(feature = "sim"))]
            world,
            lane,
        );
        return;
    };
    let (bounds, lanes) = lane_bounds(fighters, lanes);
    let ctx = &ctx;
    pool.scope(|scope| {
        // The first lane stays with the caller, the rest go to the workers.
        let mut rest = lane;
        let mut caller = None;
        for (index, &(_, len)) in bounds[..lanes].iter().enumerate() {
            let (mine, other) = rest.split_at(len);
            rest = other;
            if index == 0 {
                caller = Some(mine);
            } else {
                scope.spawn(move || {
                    run_lane(
                        ctx,
                        legs,
                        #[cfg(not(feature = "sim"))]
                        world,
                        mine,
                    );
                });
            }
        }
        if let Some(caller) = caller {
            run_lane(
                ctx,
                legs,
                #[cfg(not(feature = "sim"))]
                world,
                caller,
            );
        }
    });
    #[cfg(test)]
    FORKS.with(|count| count.set(count.get() + 1));
}

/// Apply the leg results in slot order, whatever order the lanes finished in.
fn merge_legs(
    sim: &mut Sim,
    legs: &[Option<LegIn>; MAX_SLOTS],
    outs: &[LegOut; MAX_SLOTS],
    fighters: usize,
    #[cfg(test)] trace: &mut Vec<TickMotion>,
) -> Option<()> {
    let mut failed = false;
    for (slot, leg) in legs.iter().enumerate().take(fighters) {
        let Some(leg) = leg else { continue };
        let input = leg.input;
        sim.wishes[slot] = [input.mx, input.mz];
        // The spear bit only exists in a spear match; elsewhere it is masked
        // out so a stray press cannot reach the hash.
        let kept = if sim.spear.is_some() {
            input.buttons.0
        } else {
            input.buttons.0 & !THROW_SPEAR
        };
        // The noose bit likewise (row SG11): only a noose match latches it.
        sim.buttons[slot] = if sim.spear.as_ref().is_some_and(SpearState::noose_armed) {
            kept
        } else {
            kept & !THROW_NOOSE
        };
        let out = outs[slot];
        failed |= !out.ok;
        // A knocked-out fighter froze its root: the sequential loop left the
        // armed channel's last request in place and never wrote a new one.
        if leg.alive {
            if let Some(steering) = sim.steer.as_mut() {
                steering.requests[slot] = out.requested;
            }
        }
        #[cfg(test)]
        trace.push(out.motion);
    }
    if failed {
        return None;
    }
    Some(())
}

/// Run one lane's slots in ascending slot order.
fn run_lane(
    ctx: &LegCtx,
    legs: &[Option<LegIn>; MAX_SLOTS],
    #[cfg(not(feature = "sim"))] world: &MatchWorld,
    mut lane: LegLane<'_>,
) {
    for index in 0..lane.slots() {
        let slot = lane.begin + index;
        let Some(input) = legs.get(slot).copied().flatten() else {
            continue;
        };
        let produced = {
            let leg = LegSlot {
                state: &mut lane.states[index],
                loco: &mut lane.locos[index],
                warp: lane.warps.get_mut(index),
                rise: lane.rises.get_mut(index),
                history: lane
                    .histories
                    .as_deref_mut()
                    .and_then(|history| history.get_mut(index)),
                #[cfg(feature = "sim")]
                controller: &mut lane.controllers[index],
            };
            step_slot(
                leg,
                input,
                ctx,
                #[cfg(not(feature = "sim"))]
                world,
            )
        };
        if let Some(out) = lane.outs.get_mut(index) {
            *out = produced;
        }
    }
}

/// One slot's leg: LOC1 step, ground warp, then one PH2 collision resolve.
fn step_slot(
    leg: LegSlot<'_>,
    input: LegIn,
    ctx: &LegCtx,
    #[cfg(not(feature = "sim"))] world: &MatchWorld,
) -> LegOut {
    let mut out = LegOut::default();
    #[cfg(feature = "sim")]
    let controller = leg.controller;
    let state = leg.state;
    let loco = leg.loco;
    let warp = leg.warp;
    let rise = leg.rise;
    let history = leg.history;
    if !input.alive {
        // Knocked out: hold the pose, keep the recorded pose stream fed.
        if let Some(history) = history {
            if let Some(last) = history.last().cloned() {
                history.push(last);
            }
        }
        out.ok = true;
        return out;
    }
    // Airborne follows the controller's grounded flag from the last tick
    // (`sim` only); the default leg stays grounded on the flat floor.
    #[cfg(feature = "sim")]
    loco.set_airborne(!controller.grounded());
    let (speed, dir) = loco_wish(&input.input);
    let speed = if input.down { 0.0 } else { speed };
    // AIN7 live jump: a fresh press on the floor takes off through the height
    // channel; releasing early cuts the rise (the AIN6 hold cut).
    if let Some(rise) = rise {
        if input.jump_edge && !input.down && ctx.jump_armed && *rise == 0 && state.pos[1] <= 0.0 {
            *rise = JUMP_RISE_MM_TICK;
        }
        if input.jump_release && ctx.height && *rise > 0 {
            *rise /= 2;
        }
    }
    let Some(requested) = loco_step(loco, warp, history, speed, dir, input, ctx) else {
        return out;
    };
    let requested = if input.down { [0.0, 0.0] } else { requested };
    // Facing stays the absolute IN1 intent so AI `face_toward` keeps working;
    // locomotion turns its own gait facing toward the wish.
    state.yaw = yaw_from_centideg(input.input.facing);
    #[cfg(feature = "sim")]
    let (resolved, blocked) = place_by_controller(state, controller, requested, ctx.limit);
    #[cfg(not(feature = "sim"))]
    let (resolved, blocked) = place_in_world(state, requested, ctx.limit, world);
    #[cfg(test)]
    {
        out.motion = TickMotion {
            requested,
            resolved,
            blocked,
        };
    }
    #[cfg(not(test))]
    let _ = (resolved, blocked);
    out.requested = requested;
    out.ok = true;
    out
}

/// Advance the locomotion machine for one slot and return its requested root
/// delta. Presentation runs also warp the recorded pose and keep history.
fn loco_step(
    loco: &mut Locomotion,
    warp: Option<&mut GroundWarp>,
    history: Option<&mut Vec<LocalPose>>,
    speed: f32,
    dir: [f32; 2],
    input: LegIn,
    ctx: &LegCtx,
) -> Option<[f32; 2]> {
    // Authority ticks that record nothing take the root-only advance (0 heap
    // allocs); presentation runs keep `update` so pose/contacts land in
    // history.
    if !ctx.record {
        let root = loco
            .update_root(speed, dir, ctx.tick, input.interrupt)
            .ok()?;
        return Some(root.dp);
    }
    let mut posed = loco.update(speed, dir, ctx.tick, input.interrupt).ok()?;
    // Ground feel rides the recorded pose only: the root delta below is
    // authoritative and passes through bit-identical.
    let dp = posed.root.dp;
    let capsule = (dp[0] * dp[0] + dp[1] * dp[1]).sqrt() / TICK_DT;
    if let Some(warp) = warp {
        warp.warp_output(&mut posed, capsule, dir, &LOCO_LEG_JOINTS)
            .ok()?;
    }
    if let Some(history) = history {
        history.push(posed.pose.clone());
    }
    Some(posed.root.dp)
}

/// PH2 collision on the `sim` track: one controller move and slide, then the
/// shared backstop clamp. Returns the applied displacement and the block flag.
#[cfg(feature = "sim")]
fn place_by_controller(
    state: &mut FighterState,
    controller: &mut Controller,
    requested: [f32; 2],
    limit: f32,
) -> ([f32; 2], bool) {
    // Single resolution: the blocked remainder dies here and is never
    // reapplied on a later tick.
    let resolved = controller.move_and_slide(requested, FIXED_DT);
    let (dx, dz) = (resolved.displacement[0], resolved.displacement[1]);
    // Backstop clamp: controller walls sit one skin inside this limit, so it
    // never fires; it keeps the shared wall inset live on both legs.
    state.pos[0] = canon_zero(state.pos[0] + dx).clamp(-limit, limit);
    state.pos[2] = canon_zero(state.pos[2] + dz).clamp(-limit, limit);
    ([dx, dz], resolved.wall_normal.is_some())
}

/// PH2 collision without physics: one wall clamp, then the static world
/// push-out when a kit world is supplied. Returns the applied displacement
/// and the block flag.
#[cfg(not(feature = "sim"))]
fn place_in_world(
    state: &mut FighterState,
    requested: [f32; 2],
    limit: f32,
    world: &MatchWorld,
) -> ([f32; 2], bool) {
    let from = [state.pos[0], state.pos[2]];
    let raw = [state.pos[0] + requested[0], state.pos[2] + requested[1]];
    let to = [raw[0].clamp(-limit, limit), raw[1].clamp(-limit, limit)];
    let placed = if world.is_empty() {
        to
    } else {
        world.resolve(from, to, FIGHTER_RADIUS_M, limit)
    };
    let blocked = placed != to || to != raw;
    let (before_x, before_z) = (state.pos[0], state.pos[2]);
    state.pos[0] = canon_zero(placed[0]);
    state.pos[2] = canon_zero(placed[1]);
    ([state.pos[0] - before_x, state.pos[2] - before_z], blocked)
}
