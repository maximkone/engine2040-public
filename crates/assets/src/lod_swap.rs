//! lod_swap: runtime LOD swap over cooked LOD chains with hysteresis.
//!
//! Consumer: game/editor loading picks the resident LOD per fighter each tick
//! through [`LodSwapper`]. Data owner: `engine-assets`; chain contents stay
//! owned by `mesh_cook` (this module only reads [`MeshCookOutput`]). Units:
//! integer millimeters for every distance and band. Determinism scope: the
//! same actor set plus the same `(tick, distance_mm)` sequence always yields
//! the same LODs and the same [`SwapEvent`] log (integer compares and stable
//! actor order only; no floats, no hash-map iteration, no wall clock).
//! Errors: typed [`LodSwapError`]; bad chains and bad inputs are refused,
//! per-tick swap overflow is deferred, never dropped.
//!
//! Budget: each applied swap costs one event record plus a few integer
//! compares; [`poll_into`] applies at most `min(budget, MAX_SWAPS_PER_TICK)`
//! swaps, pre-reserves the event log, and allocates nothing per tick after
//! construction (proven by the steady-state test below).

use super::mesh_cook::{check_budget, tri_count, MeshCookError, MeshCookOutput, MAX_LOD_LEVELS};
use std::fmt;

/// Distance at which one LOD coarsens to the next, in millimeters: LOD0 below
/// 15 m, LOD1 to 30 m, LOD2 to 60 m, LOD3 beyond. Length is always
/// `MAX_LOD_LEVELS - 1` (static assert below); the cooker is never mirrored.
pub const LOD_BAND_MM: [u32; 3] = [15_000, 30_000, 60_000];
/// Hysteresis depth in millimeters: refining back to a finer LOD needs the
/// distance strictly below `band - HYSTERESIS_MM`, so jitter inside the depth
/// never oscillates. Coarsening applies exactly at the band.
pub const HYSTERESIS_MM: u32 = 2_000;
/// Most swaps applied by one [`poll_into`] call; extra demand waits for the
/// next tick in actor order, so a full-roster snap never hitches one frame.
pub const MAX_SWAPS_PER_TICK: usize = 8;
/// Most actors tracked by one [`LodSwapper`]; bounds state scans.
pub const MAX_ACTORS: usize = 256;
/// Most swap events retained for replay; a swap refused by a full log applies
/// nothing, so the log never diverges from state.
pub const MAX_SWAP_EVENTS: usize = 1024;

/// Bands always cover every coarsening step of a full cooked chain.
const _: () = assert!(LOD_BAND_MM.len() + 1 == MAX_LOD_LEVELS);

/// Why LOD registration, validation or a swap step failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LodSwapError {
    /// The chain holds no LODs.
    EmptyChain,
    /// The chain holds more LODs than [`MAX_LOD_LEVELS`]; count carried.
    TooManyLevels {
        /// LOD count on the rejected chain.
        levels: usize,
    },
    /// Hull count differs from LOD count; both counts carried.
    HullMismatch {
        /// LOD count on the rejected chain.
        lods: usize,
        /// Hull count on the rejected chain.
        hulls: usize,
    },
    /// A finer LOD holds more triangles than its parent; level carried.
    GrowingChain {
        /// First level whose triangle count exceeds its parent.
        level: usize,
    },
    /// LOD0 exceeds its class budget (the mesh cook's import gate, rechecked here).
    OverBudget {
        /// Triangles in LOD0.
        tris: u32,
        /// Budget of the chain class.
        max: u32,
    },
    /// A foreign cook error that is not an over-budget refusal.
    InvalidBase,
    /// Registration asked for zero or over-limit levels; count carried.
    BadLevels {
        /// Requested level count.
        levels: usize,
    },
    /// Actor id already registered; id carried.
    DuplicateActor(u32),
    /// Actor id is not registered; id carried.
    UnknownActor(u32),
    /// One poll batch names the same actor twice; id carried.
    DuplicateInput(u32),
    /// The swapper already tracks [`MAX_ACTORS`]; count carried.
    TooManyActors {
        /// Actor count at refusal time.
        count: usize,
    },
    /// One poll batch names more actors than [`MAX_ACTORS`]; count carried.
    TooManyInputs {
        /// Input pair count.
        count: usize,
    },
    /// Non-finite float in the named lane of [`distance_mm_between`].
    NonFinite(&'static str),
    /// The event log is full; nothing swapped, replay stays exact.
    LogFull {
        /// Retained event capacity.
        capacity: usize,
    },
}

impl fmt::Display for LodSwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyChain => write!(f, "lod swap chain holds no LODs"),
            Self::TooManyLevels { levels } => {
                write!(
                    f,
                    "lod swap chain holds {levels} LODs over {MAX_LOD_LEVELS}"
                )
            }
            Self::HullMismatch { lods, hulls } => {
                write!(f, "lod swap chain holds {lods} LODs but {hulls} hulls")
            }
            Self::GrowingChain { level } => {
                write!(f, "lod swap chain grows at level {level}")
            }
            Self::OverBudget { tris, max } => {
                write!(f, "lod swap base {tris} tris over {max}")
            }
            Self::InvalidBase => write!(f, "lod swap base mesh refused"),
            Self::BadLevels { levels } => write!(f, "lod swap bad level count {levels}"),
            Self::DuplicateActor(actor) => write!(f, "lod swap duplicate actor {actor}"),
            Self::UnknownActor(actor) => write!(f, "lod swap unknown actor {actor}"),
            Self::DuplicateInput(actor) => write!(f, "lod swap duplicate input {actor}"),
            Self::TooManyActors { count } => write!(f, "lod swap actor cap reached: {count}"),
            Self::TooManyInputs { count } => write!(f, "lod swap too many inputs: {count}"),
            Self::NonFinite(lane) => write!(f, "lod swap non-finite {lane}"),
            Self::LogFull { capacity } => write!(f, "lod swap event log full: {capacity}"),
        }
    }
}

impl std::error::Error for LodSwapError {}

/// Map the mesh cook's budget refusal onto the swap error without mirroring
/// its type.
fn map_budget_error(error: MeshCookError) -> LodSwapError {
    match error {
        MeshCookError::OverBudget { tris, max } => LodSwapError::OverBudget { tris, max },
        _ => LodSwapError::InvalidBase,
    }
}

/// Desired LOD for `distance_mm` given the `current` LOD of a `levels`-long
/// chain: coarsen at each band, refine only below `band - HYSTERESIS_MM`.
/// Short chains clamp to their finest/coarsest ends; out-of-range `current`
/// clamps into the chain first, so every input maps to a valid level.
pub fn target_lod(distance_mm: u32, current: usize, levels: usize) -> usize {
    let levels = levels.clamp(1, MAX_LOD_LEVELS);
    let mut lod = current.min(levels - 1);
    while lod + 1 < levels && lod < LOD_BAND_MM.len() && distance_mm >= LOD_BAND_MM[lod] {
        lod += 1;
    }
    while lod > 0 && distance_mm < LOD_BAND_MM[lod - 1].saturating_sub(HYSTERESIS_MM) {
        lod -= 1;
    }
    lod
}

/// Camera-to-actor distance in whole millimeters, rounded to nearest, from
/// meter positions. Uses `+ - * /` and `sqrt` only; non-finite lanes are
/// refused, never clamped into a swap.
pub fn distance_mm_between(a: [f32; 3], b: [f32; 3]) -> Result<u32, LodSwapError> {
    for value in a.iter().chain(b.iter()) {
        if !value.is_finite() {
            return Err(LodSwapError::NonFinite("position"));
        }
    }
    let dx = f64::from(a[0]) - f64::from(b[0]);
    let dy = f64::from(a[1]) - f64::from(b[1]);
    let dz = f64::from(a[2]) - f64::from(b[2]);
    let dist_m = (dx * dx + dy * dy + dz * dz).sqrt();
    if !dist_m.is_finite() {
        return Err(LodSwapError::NonFinite("distance"));
    }
    Ok((dist_m * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32)
}

/// Import-time gate for one cooked chain: non-empty, at most [`MAX_LOD_LEVELS`]
/// LODs, one hull per LOD, LOD0 inside its class budget (via the cooker check,
/// never a local copy), and triangle counts that never grow down the chain.
pub fn check_chain(chain: &MeshCookOutput) -> Result<(), LodSwapError> {
    if chain.lods.is_empty() {
        return Err(LodSwapError::EmptyChain);
    }
    if chain.lods.len() > MAX_LOD_LEVELS {
        return Err(LodSwapError::TooManyLevels {
            levels: chain.lods.len(),
        });
    }
    if chain.hulls.len() != chain.lods.len() {
        return Err(LodSwapError::HullMismatch {
            lods: chain.lods.len(),
            hulls: chain.hulls.len(),
        });
    }
    check_budget(&chain.lods[0], chain.class.max_tris()).map_err(map_budget_error)?;
    let mut prev = tri_count(&chain.lods[0]);
    for (level, lod) in chain.lods.iter().enumerate().skip(1) {
        let tris = tri_count(lod);
        if tris > prev {
            return Err(LodSwapError::GrowingChain { level });
        }
        prev = tris;
    }
    Ok(())
}

/// One recorded swap: replaying the same `(tick, distance_mm)` sequence emits
/// these in the same order, so a headless match capture replays LOD for LOD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapEvent {
    /// Caller match tick of the swap.
    pub tick: u64,
    /// Swapped actor id.
    pub actor: u32,
    /// LOD before the swap.
    pub from_lod: usize,
    /// LOD after the swap.
    pub to_lod: usize,
    /// Distance in millimeters that decided the swap.
    pub distance_mm: u32,
}

/// Per-actor swap state, kept sorted by actor so every poll iterates stably.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActorState {
    /// Actor id.
    actor: u32,
    /// Resident LOD level.
    lod: usize,
    /// Chain length registered for this actor.
    levels: usize,
}

/// Runtime LOD picker over cooked chains: hysteresis per actor, budgeted swaps
/// per tick, one replay log. State iterates in actor order; a poll batch in
/// any input order decides identically.
#[derive(Clone, Debug, Default)]
pub struct LodSwapper {
    /// Actor states, ascending by actor.
    states: Vec<ActorState>,
    /// Every applied swap, in application order, for replay.
    events: Vec<SwapEvent>,
}

impl LodSwapper {
    /// Empty picker with the replay log pre-reserved, so steady-state polls
    /// never grow an allocation.
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            events: Vec::with_capacity(MAX_SWAP_EVENTS),
        }
    }

    /// Tracked actor count.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// True when no actor is tracked.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Resident LOD of one actor, or `None` when unregistered.
    pub fn lod(&self, actor: u32) -> Option<usize> {
        self.states
            .binary_search_by_key(&actor, |s| s.actor)
            .ok()
            .map(|i| self.states[i].lod)
    }

    /// Every applied swap, in application order; the replay record.
    pub fn events(&self) -> &[SwapEvent] {
        &self.events
    }

    /// Register one actor with a `levels`-long chain starting at LOD0.
    /// Levels outside `1..=MAX_LOD_LEVELS` are refused, never clamped.
    pub fn register(&mut self, actor: u32, levels: usize) -> Result<(), LodSwapError> {
        if levels == 0 || levels > MAX_LOD_LEVELS {
            return Err(LodSwapError::BadLevels { levels });
        }
        match self.states.binary_search_by_key(&actor, |s| s.actor) {
            Ok(_) => Err(LodSwapError::DuplicateActor(actor)),
            Err(slot) => {
                if self.states.len() >= MAX_ACTORS {
                    return Err(LodSwapError::TooManyActors {
                        count: self.states.len(),
                    });
                }
                self.states.insert(
                    slot,
                    ActorState {
                        actor,
                        lod: 0,
                        levels,
                    },
                );
                Ok(())
            }
        }
    }

    /// Validate a cooked chain and register its actor with the chain length.
    pub fn register_chain(
        &mut self,
        actor: u32,
        chain: &MeshCookOutput,
    ) -> Result<(), LodSwapError> {
        check_chain(chain)?;
        self.register(actor, chain.lods.len())
    }

    /// Forget one actor; unknown ids are refused.
    pub fn unregister(&mut self, actor: u32) -> Result<(), LodSwapError> {
        match self.states.binary_search_by_key(&actor, |s| s.actor) {
            Ok(slot) => {
                self.states.remove(slot);
                Ok(())
            }
            Err(_) => Err(LodSwapError::UnknownActor(actor)),
        }
    }

    /// Step one actor at `tick` to `distance_mm`; returns the swap event when
    /// the hysteresis decision moves the LOD. A full log swaps nothing.
    pub fn update(
        &mut self,
        tick: u64,
        actor: u32,
        distance_mm: u32,
    ) -> Result<Option<SwapEvent>, LodSwapError> {
        let slot = self
            .states
            .binary_search_by_key(&actor, |s| s.actor)
            .map_err(|_| LodSwapError::UnknownActor(actor))?;
        let next = target_lod(distance_mm, self.states[slot].lod, self.states[slot].levels);
        if next == self.states[slot].lod {
            return Ok(None);
        }
        if self.events.len() >= MAX_SWAP_EVENTS {
            return Err(LodSwapError::LogFull {
                capacity: MAX_SWAP_EVENTS,
            });
        }
        let event = SwapEvent {
            tick,
            actor,
            from_lod: self.states[slot].lod,
            to_lod: next,
            distance_mm,
        };
        self.states[slot].lod = next;
        self.events.push(event);
        Ok(Some(event))
    }

    /// Step every `(actor, distance_mm)` pair at `tick`, appending applied
    /// swaps to `out` (caller buffer, reused across ticks). Input order never
    /// matters: the batch sorts by actor on a bounded stack copy, duplicates
    /// are refused, and at most `min(budget, MAX_SWAPS_PER_TICK)` swaps apply
    /// in actor order while the rest wait for the next tick. Unknown actors or
    /// a full log apply nothing. Returns the applied swap count.
    pub fn poll_into(
        &mut self,
        tick: u64,
        distances: &[(u32, u32)],
        budget: usize,
        out: &mut Vec<SwapEvent>,
    ) -> Result<usize, LodSwapError> {
        if distances.len() > MAX_ACTORS {
            return Err(LodSwapError::TooManyInputs {
                count: distances.len(),
            });
        }
        // Bounded stack copy sorted by actor: stable order, no heap.
        let mut order = [(u32::MAX, 0u32); MAX_ACTORS];
        for (slot, pair) in distances.iter().enumerate() {
            order[slot] = *pair;
        }
        let len = distances.len();
        for i in 1..len {
            let key = order[i];
            let mut j = i;
            while j > 0 && order[j - 1].0 > key.0 {
                order[j] = order[j - 1];
                j -= 1;
            }
            order[j] = key;
        }
        for pair in order.iter().take(len).skip(1).zip(order.iter().take(len)) {
            let ((actor, _), (prev, _)) = (pair.0, pair.1);
            if actor == prev {
                return Err(LodSwapError::DuplicateInput(*actor));
            }
        }
        // Validate every actor before touching state.
        let mut targets = [0usize; MAX_ACTORS];
        for (i, (actor, distance_mm)) in order.iter().take(len).enumerate() {
            let slot = self
                .states
                .binary_search_by_key(actor, |s| s.actor)
                .map_err(|_| LodSwapError::UnknownActor(*actor))?;
            targets[i] = target_lod(
                *distance_mm,
                self.states[slot].lod,
                self.states[slot].levels,
            );
        }
        let allowed = budget.min(MAX_SWAPS_PER_TICK);
        let mut plan: usize = 0;
        for i in 0..len {
            let slot = self
                .states
                .binary_search_by_key(&order[i].0, |s| s.actor)
                .map_err(|_| LodSwapError::UnknownActor(order[i].0))?;
            if targets[i] != self.states[slot].lod {
                plan += 1;
            }
            if plan >= allowed {
                break;
            }
        }
        // Count exactly the swaps this call will apply (actor order prefix).
        let mut applying: usize = 0;
        let mut seen: usize = 0;
        for i in 0..len {
            let slot = self
                .states
                .binary_search_by_key(&order[i].0, |s| s.actor)
                .map_err(|_| LodSwapError::UnknownActor(order[i].0))?;
            if targets[i] != self.states[slot].lod {
                seen += 1;
                if seen <= allowed {
                    applying += 1;
                }
            }
        }
        if self.events.len() + applying > MAX_SWAP_EVENTS {
            return Err(LodSwapError::LogFull {
                capacity: MAX_SWAP_EVENTS,
            });
        }
        let mut applied: usize = 0;
        for i in 0..len {
            if applied >= allowed {
                break;
            }
            let (actor, distance_mm) = order[i];
            let slot = self
                .states
                .binary_search_by_key(&actor, |s| s.actor)
                .map_err(|_| LodSwapError::UnknownActor(actor))?;
            if targets[i] == self.states[slot].lod {
                continue;
            }
            let event = SwapEvent {
                tick,
                actor,
                from_lod: self.states[slot].lod,
                to_lod: targets[i],
                distance_mm,
            };
            self.states[slot].lod = targets[i];
            self.events.push(event);
            out.push(event);
            applied += 1;
        }
        Ok(applied)
    }

    /// Allocating convenience over [`poll_into`] for captures and tests; the
    /// per-tick path reuses a buffer with `poll_into` instead.
    pub fn poll(
        &mut self,
        tick: u64,
        distances: &[(u32, u32)],
        budget: usize,
    ) -> Result<Vec<SwapEvent>, LodSwapError> {
        let mut out = Vec::new();
        self.poll_into(tick, distances, budget, &mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::cooked::CookedMesh;
    use super::super::mesh_cook::{cook_mesh, MeshClass};
    use super::*;

    /// Small grid mesh: `n` cells per side, `+Z` normals, `0..=1` UVs.
    fn grid_mesh(n: usize) -> CookedMesh {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        let mut tangents = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                let x = i as f32 / n as f32;
                let y = j as f32 / n as f32;
                positions.push([x, y, 0.0]);
                normals.push([0.0, 0.0, 1.0]);
                uv0.push([x, y]);
                tangents.push([1.0, 0.0, 0.0, 1.0]);
            }
        }
        let mut indices = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                let b = a + 1;
                let c = a + n as u32 + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        CookedMesh {
            positions,
            normals,
            uv0,
            tangents,
            indices,
        }
    }

    /// Picker with one 4-level actor.
    fn one_actor() -> LodSwapper {
        let mut swapper = LodSwapper::new();
        swapper.register(7, MAX_LOD_LEVELS).expect("register");
        swapper
    }

    #[test]
    fn bands_select_expected_lods_from_base() {
        // Coarsening applies exactly at each band edge.
        let cases: &[(u32, usize)] = &[
            (0, 0),
            (14_999, 0),
            (15_000, 1),
            (29_999, 1),
            (30_000, 2),
            (59_999, 2),
            (60_000, 3),
            (100_000, 3),
            (u32::MAX, 3),
        ];
        for (distance_mm, want) in cases {
            assert_eq!(
                target_lod(*distance_mm, 0, MAX_LOD_LEVELS),
                *want,
                "distance {distance_mm}mm"
            );
        }
    }

    #[test]
    fn hysteresis_holds_finer_level_inside_depth() {
        // At LOD1 just over the first band, small retreats hold, a retreat
        // past `band - HYSTERESIS_MM` refines.
        assert_eq!(target_lod(15_000, 1, MAX_LOD_LEVELS), 1);
        assert_eq!(target_lod(14_500, 1, MAX_LOD_LEVELS), 1);
        assert_eq!(target_lod(13_000, 1, MAX_LOD_LEVELS), 1);
        assert_eq!(target_lod(12_999, 1, MAX_LOD_LEVELS), 0);
        // Same shape at the far band from LOD3.
        assert_eq!(target_lod(60_000, 3, MAX_LOD_LEVELS), 3);
        assert_eq!(target_lod(59_000, 3, MAX_LOD_LEVELS), 3);
        assert_eq!(target_lod(58_000, 3, MAX_LOD_LEVELS), 3);
        assert_eq!(target_lod(57_999, 3, MAX_LOD_LEVELS), 2);
        // Release cascades one call when far inside the fine range.
        assert_eq!(target_lod(0, 3, MAX_LOD_LEVELS), 0);
        // Short chains clamp: two levels only ever pick 0 or 1.
        assert_eq!(target_lod(100_000, 0, 2), 1);
        assert_eq!(target_lod(100_000, 5, 2), 1);
        assert_eq!(target_lod(0, 3, 1), 0);
    }

    #[test]
    fn jitter_inside_hysteresis_never_swaps() {
        // 600-tick orbit at every band edge, alternating just inside the
        // hysteresis depth: after settling, no tick swaps.
        for (edge, settled) in [(15_000u32, 1usize), (30_000, 2), (60_000, 3)] {
            let mut swapper = one_actor();
            swapper
                .update(0, 7, edge)
                .expect("settle")
                .expect("settling swaps");
            assert_eq!(swapper.lod(7), Some(settled));
            let base_events = swapper.events().len();
            for tick in 1..=600u64 {
                let distance_mm = if tick % 2 == 0 {
                    edge + 500
                } else {
                    edge - 500
                };
                let event = swapper.update(tick, 7, distance_mm).expect("step");
                assert!(
                    event.is_none(),
                    "edge {edge} tick {tick} oscillated: {event:?}"
                );
            }
            assert_eq!(swapper.lod(7), Some(settled));
            assert_eq!(swapper.events().len(), base_events);
        }
    }

    #[test]
    fn headless_match_capture_lands_expected_lods() {
        // Arena capture: camera at center, 8 fighters downrange; distances
        // land each fighter on its band LOD through the integer path.
        let camera = [0.0, 1.6, 0.0];
        let fighters: [f32; 8] = [5.0, 14.0, 16.0, 29.0, 31.0, 45.0, 61.0, 100.0];
        let want: [usize; 8] = [0, 0, 1, 1, 2, 2, 3, 3];
        let mut swapper = LodSwapper::new();
        for (i, x) in fighters.iter().enumerate() {
            swapper
                .register(i as u32, MAX_LOD_LEVELS)
                .expect("register");
            let mm = distance_mm_between(camera, [*x, 1.6, 0.0]).expect("distance");
            assert!(
                (mm as f32 - x * 1000.0).abs() < 2.0,
                "fighter {i} at {x}m measures {mm}mm"
            );
        }
        let mut batch: Vec<(u32, u32)> = fighters
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mm = distance_mm_between(camera, [*x, 1.6, 0.0]).expect("distance");
                (i as u32, mm)
            })
            .collect();
        batch.reverse();
        let mut out = Vec::new();
        let applied = swapper
            .poll_into(600, &batch, MAX_SWAPS_PER_TICK, &mut out)
            .expect("capture poll");
        assert_eq!(applied, 6, "six fighters leave LOD0, got {out:?}");
        for (i, want_lod) in want.iter().enumerate() {
            assert_eq!(swapper.lod(i as u32), Some(*want_lod), "fighter {i}");
        }
        // Replaying the same capture on a fresh picker emits identical events.
        let mut replay = LodSwapper::new();
        for i in 0..8u32 {
            replay.register(i, MAX_LOD_LEVELS).expect("register");
        }
        let mut out2 = Vec::new();
        replay
            .poll_into(600, &batch, MAX_SWAPS_PER_TICK, &mut out2)
            .expect("replay poll");
        assert_eq!(out, out2, "same capture, same events");
        assert_eq!(swapper.events(), replay.events());
    }

    #[test]
    fn poll_budget_defers_in_actor_order() {
        let mut swapper = LodSwapper::new();
        for actor in 0..4u32 {
            swapper.register(actor, MAX_LOD_LEVELS).expect("register");
        }
        // Every actor wants LOD3; budget 2 applies the lowest ids first.
        let batch: Vec<(u32, u32)> = vec![(3, 100_000), (2, 100_000), (1, 100_000), (0, 100_000)];
        let mut out = Vec::new();
        let applied = swapper
            .poll_into(1, &batch, 2, &mut out)
            .expect("budgeted poll");
        assert_eq!(applied, 2);
        assert_eq!([out[0].actor, out[1].actor], [0, 1]);
        assert_eq!(out[0].from_lod, 0);
        assert_eq!(out[0].to_lod, 3);
        // Input order never matters: sorted input decides identically.
        let mut twin = LodSwapper::new();
        for actor in 0..4u32 {
            twin.register(actor, MAX_LOD_LEVELS).expect("register");
        }
        let mut out_sorted = Vec::new();
        twin.poll_into(
            1,
            &[(0, 100_000), (1, 100_000), (2, 100_000), (3, 100_000)],
            2,
            &mut out_sorted,
        )
        .expect("sorted poll");
        assert_eq!(out, out_sorted);
        // The deferred actors still want their swap next tick.
        let mut next = Vec::new();
        let applied = swapper
            .poll_into(2, &batch, 8, &mut next)
            .expect("next poll");
        assert_eq!(applied, 2);
        assert_eq!([next[0].actor, next[1].actor], [2, 3]);
        assert!(swapper
            .events()
            .windows(2)
            .all(|w| w[0].actor <= w[1].actor || w[0].tick < w[1].tick));
    }

    #[test]
    fn chains_from_the_cooker_register_and_refuse() {
        // A real mesh cook registers through its own length.
        let mesh = grid_mesh(8);
        let chain = cook_mesh(&mesh, MeshClass::Prop).expect("cook");
        assert_eq!(chain.lods.len(), MAX_LOD_LEVELS);
        let mut swapper = LodSwapper::new();
        swapper.register_chain(11, &chain).expect("register chain");
        assert_eq!(swapper.lod(11), Some(0));
        assert_eq!(
            swapper.register_chain(11, &chain),
            Err(LodSwapError::DuplicateActor(11))
        );
        // Hull-count mismatch refuses.
        let mut short_hulls = chain.clone();
        short_hulls.hulls.pop();
        assert_eq!(
            check_chain(&short_hulls),
            Err(LodSwapError::HullMismatch {
                lods: MAX_LOD_LEVELS,
                hulls: MAX_LOD_LEVELS - 1
            })
        );
        // Empty chains refuse.
        let empty = MeshCookOutput {
            class: MeshClass::Prop,
            base_tris: 0,
            lods: Vec::new(),
            hulls: Vec::new(),
            digest: 0,
        };
        assert_eq!(check_chain(&empty), Err(LodSwapError::EmptyChain));
        // LOD0 over its class budget refuses with both counts.
        let big = grid_mesh(64);
        let hero = cook_mesh(&big, MeshClass::Hero).expect("hero cook");
        let mut over = hero.clone();
        over.class = MeshClass::Prop;
        let tris = tri_count(&over.lods[0]);
        assert_eq!(
            check_chain(&over),
            Err(LodSwapError::OverBudget {
                tris,
                max: MeshClass::Prop.max_tris()
            })
        );
        // Growing chains refuse at the growing level.
        let mut growing = hero.clone();
        growing.lods.reverse();
        assert!(matches!(
            check_chain(&growing),
            Err(LodSwapError::GrowingChain { .. })
        ));
    }

    #[test]
    fn bad_inputs_refused_without_state_change() {
        let mut swapper = one_actor();
        assert_eq!(
            swapper.register(7, MAX_LOD_LEVELS),
            Err(LodSwapError::DuplicateActor(7))
        );
        assert_eq!(
            swapper.register(8, 0),
            Err(LodSwapError::BadLevels { levels: 0 })
        );
        assert_eq!(
            swapper.register(8, MAX_LOD_LEVELS + 1),
            Err(LodSwapError::BadLevels {
                levels: MAX_LOD_LEVELS + 1
            })
        );
        assert_eq!(
            swapper.update(1, 99, 1000),
            Err(LodSwapError::UnknownActor(99))
        );
        assert_eq!(
            swapper.poll(1, &[(7, 1000), (7, 2000)], 8),
            Err(LodSwapError::DuplicateInput(7))
        );
        assert_eq!(swapper.lod(7), Some(0), "refusals move nothing");
        assert!(swapper.events().is_empty());
        assert_eq!(
            distance_mm_between([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0]),
            Err(LodSwapError::NonFinite("position"))
        );
        assert_eq!(
            distance_mm_between([f32::INFINITY, 0.0, 0.0], [0.0, 0.0, 0.0]),
            Err(LodSwapError::NonFinite("position"))
        );
        // Same-point distance is exactly zero millimeters.
        assert_eq!(distance_mm_between([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), Ok(0));
    }

    #[test]
    fn full_event_log_applies_nothing() {
        let mut swapper = one_actor();
        let mut tick: u64 = 0;
        while swapper.events().len() < MAX_SWAP_EVENTS {
            tick += 1;
            let distance_mm = if tick % 2 == 1 { 100_000 } else { 0 };
            swapper
                .update(tick, 7, distance_mm)
                .expect("room remains")
                .expect("alternation always swaps");
        }
        assert_eq!(swapper.events().len(), MAX_SWAP_EVENTS);
        let lod_before = swapper.lod(7);
        assert_eq!(
            swapper.update(tick + 1, 7, 100_000),
            Err(LodSwapError::LogFull {
                capacity: MAX_SWAP_EVENTS
            })
        );
        assert_eq!(swapper.lod(7), lod_before, "full log moves nothing");
    }

    #[test]
    fn steady_state_poll_allocates_nothing() {
        let mut swapper = one_actor();
        swapper.update(0, 7, 5_000).expect("settle at LOD0");
        let mut out = Vec::with_capacity(16);
        let mut probe = vec![(7u32, 5_100u32)];
        // Warm one poll so buffers settle, then record capacities.
        swapper
            .poll_into(1, &probe, 8, &mut out)
            .expect("warm poll");
        let (states_cap, events_cap, out_cap) = (
            swapper.states.capacity(),
            swapper.events.capacity(),
            out.capacity(),
        );
        for tick in 2..=602u64 {
            probe[0].1 = if tick % 2 == 0 { 5_100 } else { 4_900 };
            let applied = swapper
                .poll_into(tick, &probe, 8, &mut out)
                .expect("steady poll");
            assert_eq!(applied, 0, "5 m jitter never swaps");
        }
        assert_eq!(swapper.states.capacity(), states_cap);
        assert_eq!(swapper.events.capacity(), events_cap);
        assert_eq!(out.capacity(), out_cap);
        assert!(swapper.events().is_empty());
    }
}
