//! depth: integer combat-depth cores over the base fight table core.
//!
//! Consumer: the arena steps [`DepthSet::tick`] beside the fight step,
//! records strikes, and routes tick events through [`DepthSet::observe`];
//! [`HitResolver`] answers damage through the fight tables. Owner:
//! [`DepthSet`] owns per-slot pools; [`ComboChain`] owns its patterns.
//! Units: damage permille, stamina/poise points, all timing fixed ticks
//! (60 Hz). Determinism: integer `+ - * /` only, slot order, no floats,
//! no clock. Errors: [`DepthError`]; bad slots/kinds/patterns refuse.
//! Budget: one tick touches [`MAX_SLOTS`] slots, no heap, < 2 KiB state.
//! PORT(maximkone/flax-mcp@a26f960:plugins/gameplay/GameSide/
//! Game.Shared.Combat/Mechanics/Combat{HitResolver,Stamina,Poise,
//! ComboChain}*.cs), MIT; clean-room integer rewrite, no vendor bytes.

use crate::fight::{
    FightEvent, FightState, Tables, MAX_HP_PERMILLE, MAX_STAMINA, SLOT_COUNT,
    STAMINA_REGEN_PER_TICK, TABLES_V1,
};
use engine_input::MAX_PARTICIPANTS;

/// Slots covered by [`DepthSet`]: one pool pair per match slot.
pub const MAX_SLOTS: usize = SLOT_COUNT;
/// Longest tracked strike chain (oldest trimmed past this).
pub const MAX_CHAIN: usize = 8;
/// Most combo patterns held at once; matches fire in registration order.
pub const MAX_PATTERNS: usize = 8;
/// Longest single pattern; patterns index the chain tail.
pub const MAX_PATTERN_LEN: usize = 8;
/// Light strike kind for [`ComboChain::input`].
pub const INPUT_LIGHT: u8 = 0;
/// Heavy strike kind for [`ComboChain::input`].
pub const INPUT_HEAVY: u8 = 1;
/// Special strike kind for [`ComboChain::input`].
pub const INPUT_SPECIAL: u8 = 2;
/// Chain window mirroring the 0.8 s donor default at 60 Hz, in fixed ticks.
pub const CHAIN_TIMEOUT_TICKS: u32 = 48;
/// Poise cap for [`PoiseMeter::default_row`], in points.
pub const DEFAULT_MAX_POISE: u16 = 100;
/// Poise regained per quiet tick by [`PoiseMeter::default_row`], in points.
pub const DEFAULT_POISE_REGEN_PER_TICK: u16 = 1;
/// Quiet ticks before [`PoiseMeter::default_row`] regens (1.5 s at 60 Hz).
pub const DEFAULT_POISE_DELAY_TICKS: u32 = 90;

/// Bounded depth failures; invalid caller data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthError {
    /// Slot outside the live `0..slots` range; culprit carried.
    BadSlot(u8),
    /// Strike kind above [`INPUT_SPECIAL`]; culprit carried.
    BadKind(u8),
    /// Pattern with no strikes.
    EmptyPattern,
    /// Pattern longer than [`MAX_PATTERN_LEN`]; length carried.
    PatternTooLong(usize),
    /// No free pattern slot left; pattern id carried.
    TooManyPatterns(u8),
}

/// Integer hit resolver over the fight tables.
///
/// Damage goes through the caller's [`Tables`] row (chip for held blocks,
/// clamped raw otherwise), so mitigation can never drift from the fight
/// step that owns the row. No floats, no RNG, no clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HitResolver {
    /// Table row damage derives from; swaps like any other tuning.
    pub tables: Tables,
    /// `resolve` calls so far, including zero-damage early-outs.
    pub resolve_count: u32,
    /// Raw damage of the most recent call.
    pub last_raw: u16,
    /// Final damage of the most recent call.
    pub last_final: u16,
}

impl HitResolver {
    /// Resolver over an explicit table row.
    pub const fn new(tables: Tables) -> Self {
        Self {
            tables,
            resolve_count: 0,
            last_raw: 0,
            last_final: 0,
        }
    }

    /// Resolver over the shipped V1 row.
    pub const fn v1() -> Self {
        Self::new(TABLES_V1)
    }

    /// Resolve `raw` permille to final damage: chip through a held block,
    /// clamped raw otherwise. Zero in, zero out. Never panics.
    pub fn resolve(&mut self, raw: u16, blocked: bool) -> u16 {
        self.resolve_count = self.resolve_count.saturating_add(1);
        self.last_raw = raw;
        let out = if raw == 0 {
            0
        } else if blocked {
            self.tables.chip(raw)
        } else {
            raw.min(MAX_HP_PERMILLE)
        };
        self.last_final = out;
        out
    }

    /// Clear diagnostics; keeps the table row.
    pub fn reset_diagnostics(&mut self) {
        self.resolve_count = 0;
        self.last_raw = 0;
        self.last_final = 0;
    }
}

/// Integer stamina pool with delayed regen.
///
/// Drains that would go negative are refused and counted; regen resumes
/// only after [`StaminaPool::regen_delay_ticks`] quiet ticks. Step one
/// fixed tick per fight tick. No floats, no clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StaminaPool {
    /// Cap in points.
    pub max: u16,
    /// Current points, always `<= max`.
    pub current: u16,
    /// Points regained per quiet tick.
    pub regen_per_tick: u16,
    /// Quiet ticks before regen resumes.
    pub regen_delay_ticks: u32,
    /// Ticks since the last accepted drain.
    pub ticks_since_drain: u32,
    /// Accepted drains so far.
    pub drain_count: u32,
    /// Refused drains so far.
    pub denial_count: u32,
}

impl StaminaPool {
    /// Pool with explicit tuning; starts full with the window closed.
    pub const fn new(max: u16, regen_per_tick: u16, regen_delay_ticks: u32) -> Self {
        Self {
            max,
            current: max,
            regen_per_tick,
            regen_delay_ticks,
            ticks_since_drain: 0,
            drain_count: 0,
            denial_count: 0,
        }
    }

    /// Pool mirroring the fight economy: full cap, per-tick regen from the
    /// row, no extra delay (the fight core regens every tick).
    pub const fn fight_mirror() -> Self {
        Self::new(MAX_STAMINA, STAMINA_REGEN_PER_TICK, 0)
    }

    /// Spend `amount`, or refuse when it would go negative. Zero spends
    /// nothing and reports false without counting a denial.
    pub fn try_drain(&mut self, amount: u16) -> bool {
        if amount == 0 {
            return false;
        }
        if self.current < amount {
            self.denial_count = self.denial_count.saturating_add(1);
            return false;
        }
        self.current -= amount;
        self.ticks_since_drain = 0;
        self.drain_count = self.drain_count.saturating_add(1);
        true
    }

    /// Advance one fixed tick: open the regen window, then refill to max.
    pub fn tick(&mut self) {
        self.ticks_since_drain = self.ticks_since_drain.saturating_add(1);
        if self.ticks_since_drain < self.regen_delay_ticks {
            return;
        }
        self.current = self
            .current
            .saturating_add(self.regen_per_tick)
            .min(self.max);
    }

    /// Refill with the regen window open.
    pub fn refill(&mut self) {
        self.current = self.max;
        self.ticks_since_drain = self.regen_delay_ticks;
    }

    /// Restore full with the window closed and counters cleared.
    pub fn reset(&mut self) {
        self.current = self.max;
        self.ticks_since_drain = 0;
        self.drain_count = 0;
        self.denial_count = 0;
    }
}

/// Integer poise meter with stagger trips.
///
/// Hits drain poise; zero trips a stagger that holds until
/// [`PoiseMeter::recover_from_stagger`]. Regen resumes after
/// [`PoiseMeter::regen_delay_ticks`] quiet ticks. No floats, no clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PoiseMeter {
    /// Cap in points.
    pub max: u16,
    /// Current points, always `<= max`.
    pub current: u16,
    /// Points regained per quiet tick.
    pub regen_per_tick: u16,
    /// Quiet ticks before regen resumes.
    pub regen_delay_ticks: u32,
    /// Ticks since the last absorbed hit.
    pub ticks_since_hit: u32,
    /// Whether the meter is holding a stagger trip.
    pub staggered: bool,
    /// Stagger trips so far.
    pub stagger_trips: u32,
    /// Hits absorbed so far (including ones taken while staggered).
    pub hits_absorbed: u32,
}

impl PoiseMeter {
    /// Meter with explicit tuning; starts full with the window closed.
    pub const fn new(max: u16, regen_per_tick: u16, regen_delay_ticks: u32) -> Self {
        Self {
            max,
            current: max,
            regen_per_tick,
            regen_delay_ticks,
            ticks_since_hit: 0,
            staggered: false,
            stagger_trips: 0,
            hits_absorbed: 0,
        }
    }

    /// Shipped integer row: 100 cap, 1 point per quiet tick after 90
    /// quiet ticks. Integerized from the 25/s donor default, which has
    /// no exact per-tick form; retune through [`PoiseMeter::new`].
    pub const fn default_row() -> Self {
        Self::new(
            DEFAULT_MAX_POISE,
            DEFAULT_POISE_REGEN_PER_TICK,
            DEFAULT_POISE_DELAY_TICKS,
        )
    }

    /// Apply `poise_damage`; returns true exactly on the trip into
    /// stagger. Zero damage is ignored. Hits while staggered count as
    /// absorbed but trip nothing new. Never panics.
    pub fn apply_hit(&mut self, poise_damage: u16) -> bool {
        if poise_damage == 0 {
            return false;
        }
        self.ticks_since_hit = 0;
        self.hits_absorbed = self.hits_absorbed.saturating_add(1);
        if self.staggered {
            return false;
        }
        self.current = self.current.saturating_sub(poise_damage);
        if self.current == 0 {
            self.staggered = true;
            self.stagger_trips = self.stagger_trips.saturating_add(1);
            return true;
        }
        false
    }

    /// Leave stagger at full poise with the regen window open.
    pub fn recover_from_stagger(&mut self) {
        self.staggered = false;
        self.current = self.max;
        self.ticks_since_hit = self.regen_delay_ticks;
    }

    /// Advance one fixed tick: staggered meters hold until recovered,
    /// quiet ones refill toward max.
    pub fn tick(&mut self) {
        self.ticks_since_hit = self.ticks_since_hit.saturating_add(1);
        if self.staggered {
            return;
        }
        if self.ticks_since_hit < self.regen_delay_ticks {
            return;
        }
        self.current = self
            .current
            .saturating_add(self.regen_per_tick)
            .min(self.max);
    }

    /// Restore full, unstaggered, with counters cleared.
    pub fn reset(&mut self) {
        self.current = self.max;
        self.ticks_since_hit = 0;
        self.staggered = false;
        self.stagger_trips = 0;
        self.hits_absorbed = 0;
    }
}

/// One registered strike pattern: the id it fires plus its exact tail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ComboPattern {
    id: u8,
    len: u8,
    seq: [u8; MAX_PATTERN_LEN],
}

/// Rolling strike-chain tracker over fixed ticks.
///
/// Records strike kinds in a sliding window; the tail fires registered
/// patterns in registration order, first match per input. Timeout and
/// explicit resets clear the chain, never the patterns. Fixed arrays
/// only, so the tick path allocates nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ComboChain {
    chain: [u8; MAX_CHAIN],
    len: u8,
    timeout_ticks: u32,
    elapsed_ticks: u32,
    patterns: [ComboPattern; MAX_PATTERNS],
    pattern_count: u8,
    /// Strikes recorded so far.
    pub total_inputs: u32,
    /// Patterns fired so far.
    pub completed: u32,
    /// Timeout clears so far.
    pub timeout_resets: u32,
    /// Id fired by the most recent input, if any.
    pub last_completed: Option<u8>,
}

impl ComboChain {
    /// Chain with an explicit window; a zero window expires on the next
    /// tick. Starts empty with no patterns.
    pub const fn new(timeout_ticks: u32) -> Self {
        const NO_PATTERN: ComboPattern = ComboPattern {
            id: 0,
            len: 0,
            seq: [0; MAX_PATTERN_LEN],
        };
        Self {
            chain: [0; MAX_CHAIN],
            len: 0,
            timeout_ticks,
            elapsed_ticks: 0,
            patterns: [NO_PATTERN; MAX_PATTERNS],
            pattern_count: 0,
            total_inputs: 0,
            completed: 0,
            timeout_resets: 0,
            last_completed: None,
        }
    }

    /// Chain with the shipped 48-tick window.
    pub const fn with_defaults() -> Self {
        Self::new(CHAIN_TIMEOUT_TICKS)
    }

    /// Register the tail `seq` (each kind `<= INPUT_SPECIAL`) firing `id`.
    /// True on insert, false when an existing id is replaced in place.
    /// Refuses empty, overlong, mis-kinded and overflowing patterns.
    pub fn register_pattern(&mut self, id: u8, seq: &[u8]) -> Result<bool, DepthError> {
        if seq.is_empty() {
            return Err(DepthError::EmptyPattern);
        }
        if seq.len() > MAX_PATTERN_LEN {
            return Err(DepthError::PatternTooLong(seq.len()));
        }
        for kind in seq.iter() {
            if *kind > INPUT_SPECIAL {
                return Err(DepthError::BadKind(*kind));
            }
        }
        for pattern in self
            .patterns
            .iter_mut()
            .take(usize::from(self.pattern_count))
        {
            if pattern.id == id {
                pattern.len = seq.len() as u8;
                pattern.seq = [0; MAX_PATTERN_LEN];
                for (i, kind) in seq.iter().enumerate() {
                    pattern.seq[i] = *kind;
                }
                return Ok(false);
            }
        }
        if usize::from(self.pattern_count) >= MAX_PATTERNS {
            return Err(DepthError::TooManyPatterns(id));
        }
        let slot = &mut self.patterns[usize::from(self.pattern_count)];
        slot.id = id;
        slot.len = seq.len() as u8;
        slot.seq = [0; MAX_PATTERN_LEN];
        for (i, kind) in seq.iter().enumerate() {
            slot.seq[i] = *kind;
        }
        self.pattern_count += 1;
        Ok(true)
    }

    /// Record one strike; trims the oldest past [`MAX_CHAIN`]. Returns the
    /// fired pattern id, if the tail matches one. Refuses bad kinds.
    pub fn input(&mut self, kind: u8) -> Result<Option<u8>, DepthError> {
        if kind > INPUT_SPECIAL {
            return Err(DepthError::BadKind(kind));
        }
        self.total_inputs = self.total_inputs.saturating_add(1);
        if usize::from(self.len) >= MAX_CHAIN {
            for i in 1..MAX_CHAIN {
                self.chain[i - 1] = self.chain[i];
            }
            self.chain[MAX_CHAIN - 1] = kind;
        } else {
            self.chain[usize::from(self.len)] = kind;
            self.len += 1;
        }
        self.elapsed_ticks = 0;
        self.last_completed = None;
        for pattern in self.patterns.iter().take(usize::from(self.pattern_count)) {
            let n = usize::from(pattern.len);
            if n <= usize::from(self.len) {
                let base = usize::from(self.len) - n;
                let mut hit = true;
                for i in 0..n {
                    if self.chain[base + i] != pattern.seq[i] {
                        hit = false;
                        break;
                    }
                }
                if hit {
                    self.completed = self.completed.saturating_add(1);
                    self.last_completed = Some(pattern.id);
                    return Ok(Some(pattern.id));
                }
            }
        }
        Ok(None)
    }

    /// Advance one fixed tick; true exactly when the window clears a
    /// non-empty chain. Empty chains and non-positive deltas never clear.
    pub fn tick(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.elapsed_ticks = self.elapsed_ticks.saturating_add(1);
        if self.elapsed_ticks >= self.timeout_ticks {
            self.len = 0;
            self.elapsed_ticks = 0;
            self.timeout_resets = self.timeout_resets.saturating_add(1);
            return true;
        }
        false
    }

    /// Clear the chain; keeps patterns and diagnostics.
    pub fn reset(&mut self) {
        self.len = 0;
        self.elapsed_ticks = 0;
        self.last_completed = None;
    }

    /// Clear chain, patterns and every counter.
    pub fn reset_all(&mut self) {
        self.reset();
        self.pattern_count = 0;
        self.total_inputs = 0;
        self.completed = 0;
        self.timeout_resets = 0;
    }

    /// Patterns currently registered.
    pub const fn pattern_count(&self) -> usize {
        self.pattern_count as usize
    }
}

/// Per-slot depth state stepped beside the fight step.
///
/// The arena mirrors authority stamina in, records strikes as inputs are
/// built, steps this beside the fight step, then routes the tick events
/// back through [`DepthSet::observe`]. The mirror is one way: the fight
/// step stays the authority and these pools never write back into it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DepthSet {
    stamina: [StaminaPool; MAX_SLOTS],
    poise: [PoiseMeter; MAX_SLOTS],
    chain: ComboChain,
    resolver: HitResolver,
    slots: u8,
}

impl DepthSet {
    /// Depth state for `slots` live slots under `tables`; out-of-range
    /// counts fall back to the full cap instead of panicking.
    ///
    /// Poise meters open with [`poise_regen_delay_for_tables`](crate::fight::poise_regen_delay_for_tables):
    /// 30 quiet ticks under V2 rows, 90 otherwise, so V1 meters keep their
    /// survey window and V1 bouts replay byte-identical.
    pub fn new(tables: Tables, slots: u8) -> Self {
        let live = if slots == 0 || slots > MAX_PARTICIPANTS {
            MAX_PARTICIPANTS
        } else {
            slots
        };
        let poise_delay = crate::fight::poise_regen_delay_for_tables(&tables);
        Self {
            stamina: [StaminaPool::fight_mirror(); MAX_SLOTS],
            poise: [PoiseMeter::new(DEFAULT_MAX_POISE, DEFAULT_POISE_REGEN_PER_TICK, poise_delay);
                MAX_SLOTS],
            chain: ComboChain::with_defaults(),
            resolver: HitResolver::new(tables),
            slots: live,
        }
    }

    /// Live slot count, always `1..=MAX_PARTICIPANTS`.
    pub const fn slots(&self) -> u8 {
        self.slots
    }

    fn check(&self, slot: u8) -> Result<usize, DepthError> {
        if slot >= self.slots {
            return Err(DepthError::BadSlot(slot));
        }
        Ok(usize::from(slot))
    }

    /// Gate a depth action on slot stamina; refusals count in the pool.
    pub fn try_spend(&mut self, slot: u8, amount: u16) -> Result<bool, DepthError> {
        let i = self.check(slot)?;
        Ok(self.stamina[i].try_drain(amount))
    }

    /// Apply one strike kind to victim poise; true exactly on the stagger trip.
    ///
    /// Consumer: the arena records the kind with [`DepthSet::record_strike`]
    /// then applies it here beside the fight step; the fight step stays the
    /// authority and this meter never writes back into it (read-only past the
    /// observe boundary; arena adopts next, remainder). Units: poise points.
    /// Determinism: integer `saturating_sub` plus slot order only. Errors:
    /// [`DepthError::BadSlot`] past the live slots,
    /// [`DepthError::BadKind`] unless light or heavy. V1 rows carry no poise
    /// damage, so V1 always returns false and never staggers.
    #[cfg(test)]
    fn apply_poise_kind(&mut self, victim: u8, kind: u8) -> Result<bool, DepthError> {
        if kind != INPUT_LIGHT && kind != INPUT_HEAVY {
            return Err(DepthError::BadKind(kind));
        }
        let i = self.check(victim)?;
        let damage = crate::fight::poise_damage_for_kind(&self.resolver.tables, kind);
        Ok(self.poise[i].apply_hit(damage))
    }

    /// Spend one strike kind from slot stamina; false when it would go negative.
    ///
    /// Consumer and remainder match [`DepthSet::apply_poise_kind`]. Units:
    /// stamina points. Determinism: same tables plus same kind give same cost.
    /// Errors: [`DepthError::BadSlot`] and [`DepthError::BadKind`] like above.
    /// V2 heavy costs more than light; V1 uses its single attack cost for both.
    #[cfg(test)]
    fn spend_stamina_kind(&mut self, slot: u8, kind: u8) -> Result<bool, DepthError> {
        if kind != INPUT_LIGHT && kind != INPUT_HEAVY {
            return Err(DepthError::BadKind(kind));
        }
        let i = self.check(slot)?;
        let cost = crate::fight::stamina_cost_for_kind(&self.resolver.tables, kind);
        Ok(self.stamina[i].try_drain(cost))
    }

    /// Record one thrown strike of `kind` in the shared chain.
    pub fn record_strike(&mut self, kind: u8) -> Result<Option<u8>, DepthError> {
        self.chain.input(kind)
    }

    /// Register the tail `seq` firing `id` in the shared chain.
    pub fn register_pattern(&mut self, id: u8, seq: &[u8]) -> Result<bool, DepthError> {
        self.chain.register_pattern(id, seq)
    }

    /// Mirror authority stamina one way; the fight step stays the source.
    pub fn sync_stamina_from(&mut self, state: &FightState) {
        let n = usize::from(self.slots.min(state.fighter_count)).min(MAX_SLOTS);
        for i in 0..n {
            self.stamina[i].current = state.fighters[i].stamina.min(self.stamina[i].max);
        }
    }

    /// Route tick events into poise: full damage and chip drain the victim
    /// meter; parries, dodges and bookkeeping leave it alone. Reads the
    /// events in order, so equal tapes give equal meters.
    pub fn observe(&mut self, events: &[FightEvent]) {
        for event in events.iter() {
            match *event {
                FightEvent::Hit { victim, damage, .. } => {
                    if victim < self.slots {
                        self.poise[usize::from(victim)].apply_hit(damage);
                    }
                }
                FightEvent::Blocked { victim, chip, .. } => {
                    if victim < self.slots {
                        self.poise[usize::from(victim)].apply_hit(chip);
                    }
                }
                FightEvent::Parried { .. }
                | FightEvent::Dodged { .. }
                | FightEvent::StaminaOut { .. }
                | FightEvent::Knockdown { .. } => {}
            }
        }
    }

    /// Advance every pool, meter and the chain by one fixed tick.
    pub fn tick(&mut self) {
        for pool in self.stamina.iter_mut().take(usize::from(self.slots)) {
            pool.tick();
        }
        for meter in self.poise.iter_mut().take(usize::from(self.slots)) {
            meter.tick();
        }
        let _ = self.chain.tick();
    }

    /// Resolve `raw` permille through the fight row (held block chips).
    pub fn resolve(&mut self, raw: u16, blocked: bool) -> u16 {
        self.resolver.resolve(raw, blocked)
    }

    /// Read one stamina pool, or `None` past the live slots.
    pub fn stamina(&self, slot: u8) -> Option<&StaminaPool> {
        if slot < self.slots {
            Some(&self.stamina[usize::from(slot)])
        } else {
            None
        }
    }

    /// Read one poise meter, or `None` past the live slots.
    pub fn poise(&self, slot: u8) -> Option<&PoiseMeter> {
        if slot < self.slots {
            Some(&self.poise[usize::from(slot)])
        } else {
            None
        }
    }

    /// Read the shared chain.
    pub const fn chain(&self) -> &ComboChain {
        &self.chain
    }

    /// Read the resolver (diagnostics included).
    pub const fn resolver(&self) -> &HitResolver {
        &self.resolver
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::{
        block_chip, step_with_tables_into, ATTACK_DAMAGE, ATTACK_STARTUP_TICKS, PARRY_WINDOW_TICKS,
    };
    use engine_input::fighter::{BLOCK, DODGE};
    use engine_input::{Buttons, FighterId, FighterInput};

    fn mk(slot: u8, buttons: u8) -> FighterInput {
        FighterInput {
            tick: 0,
            fighter: FighterId::new(slot).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(buttons),
        }
    }

    #[test]
    fn resolver_matches_fight_row() {
        let mut r = HitResolver::v1();
        for raw in [0, 1, 24, 120, 1000, 5000] {
            assert_eq!(r.resolve(raw, true), TABLES_V1.chip(raw));
            // The free fn reads the same shipped row, so they agree exactly.
            assert_eq!(r.resolve(raw, true), block_chip(raw));
            let open = r.resolve(raw, false);
            assert_eq!(
                open,
                if raw == 0 {
                    0
                } else {
                    raw.min(MAX_HP_PERMILLE)
                }
            );
        }
        assert_eq!(r.resolve_count, 18);
        assert_eq!((r.last_raw, r.last_final), (5000, MAX_HP_PERMILLE));
        r.reset_diagnostics();
        assert_eq!((r.resolve_count, r.last_raw, r.last_final), (0, 0, 0));
    }

    #[test]
    fn resolver_delegates_to_fight_step_end_to_end() {
        // Real fight tick ending in a held-block chip: the resolver must
        // answer the same chip the step emits.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        state.fighters[1].is_blocking = true;
        state.fighters[1].block_age = PARRY_WINDOW_TICKS + 1;
        let mut events = Vec::new();
        step_with_tables_into(
            &[mk(0, 0), mk(1, BLOCK)],
            &mut state,
            &TABLES_V1,
            &mut events,
        );
        assert_eq!(events.len(), 1);
        let chip = match events[0] {
            FightEvent::Blocked { chip, .. } => chip,
            ref other => panic!("expected Blocked, got {other:?}"),
        };
        let mut r = HitResolver::v1();
        assert_eq!(r.resolve(ATTACK_DAMAGE, true), chip);
        assert_eq!(chip, block_chip(ATTACK_DAMAGE));
    }

    #[test]
    fn resolver_custom_row_stays_delegated() {
        let mut tables = TABLES_V1;
        tables.block_chip_num = 1;
        tables.block_chip_den = 2;
        let mut r = HitResolver::new(tables);
        assert_eq!(r.resolve(120, true), tables.chip(120));
        assert_eq!(r.resolve(120, true), 60);
    }

    #[test]
    fn stamina_blocks_drain_when_negative() {
        let mut pool = StaminaPool::fight_mirror();
        pool.current = 24;
        assert!(!pool.try_drain(25));
        assert_eq!(pool.denial_count, 1);
        assert_eq!(pool.current, 24);
        assert!(!pool.try_drain(0));
        assert_eq!(pool.denial_count, 1);
        assert!(pool.try_drain(24));
        assert_eq!((pool.current, pool.drain_count), (0, 1));
        assert!(!pool.try_drain(1));
    }

    #[test]
    fn stamina_regen_waits_out_delay_then_clamps() {
        let mut pool = StaminaPool::new(100, 20, 60);
        assert!(pool.try_drain(50));
        pool.tick();
        assert_eq!(pool.current, 50);
        for _ in 1..60 {
            pool.tick();
        }
        assert_eq!(pool.current, 70);
        pool.refill();
        assert_eq!(pool.current, 100);
        pool.tick();
        assert_eq!(pool.current, 100);
        pool.reset();
        assert_eq!(
            (
                pool.current,
                pool.ticks_since_drain,
                pool.drain_count,
                pool.denial_count
            ),
            (100, 0, 0, 0)
        );
    }

    #[test]
    fn poise_trips_once_then_holds_until_recovered() {
        let mut meter = PoiseMeter::default_row();
        assert!(!meter.apply_hit(0));
        assert_eq!(meter.hits_absorbed, 0);
        assert!(!meter.apply_hit(60));
        assert!(!meter.staggered);
        assert!(meter.apply_hit(40));
        assert!(meter.staggered);
        assert_eq!(meter.stagger_trips, 1);
        assert!(!meter.apply_hit(40));
        assert_eq!(meter.stagger_trips, 1);
        assert_eq!(meter.hits_absorbed, 3);
        for _ in 0..200 {
            meter.tick();
        }
        assert!(meter.staggered);
        meter.recover_from_stagger();
        assert!(!meter.staggered);
        assert_eq!(meter.current, meter.max);
    }

    #[test]
    fn poise_regens_after_quiet_window() {
        let mut meter = PoiseMeter::new(100, 5, 10);
        assert!(!meter.apply_hit(30));
        for _ in 0..9 {
            meter.tick();
        }
        assert_eq!(meter.current, 70);
        meter.tick();
        assert_eq!(meter.current, 75);
        meter.reset();
        assert_eq!(
            (
                meter.current,
                meter.staggered,
                meter.stagger_trips,
                meter.hits_absorbed
            ),
            (100, false, 0, 0)
        );
    }

    #[test]
    fn combo_fires_first_registered_pattern() {
        let mut chain = ComboChain::with_defaults();
        assert_eq!(
            chain.register_pattern(7, &[INPUT_LIGHT, INPUT_LIGHT, INPUT_HEAVY]),
            Ok(true)
        );
        assert_eq!(
            chain.register_pattern(3, &[INPUT_LIGHT, INPUT_HEAVY]),
            Ok(true)
        );
        assert_eq!(chain.input(INPUT_LIGHT), Ok(None));
        assert_eq!(chain.input(INPUT_LIGHT), Ok(None));
        // Tail matches both tails; registration order wins.
        assert_eq!(chain.input(INPUT_HEAVY), Ok(Some(7)));
        assert_eq!(chain.last_completed, Some(7));
        assert_eq!(chain.completed, 1);
        // Replace keeps the slot: id 3 now fires the long tail too.
        assert_eq!(
            chain.register_pattern(3, &[INPUT_LIGHT, INPUT_LIGHT, INPUT_HEAVY]),
            Ok(false)
        );
        assert_eq!(chain.pattern_count(), 2);
    }

    #[test]
    fn combo_timeout_clears_and_full_chain_trims() {
        let mut chain = ComboChain::new(3);
        assert_eq!(chain.input(INPUT_LIGHT), Ok(None));
        assert!(!chain.tick());
        assert!(!chain.tick());
        assert!(chain.tick());
        assert_eq!(chain.timeout_resets, 1);
        assert!(!chain.tick());
        assert_eq!(
            chain.register_pattern(5, &[INPUT_HEAVY, INPUT_SPECIAL]),
            Ok(true)
        );
        for _ in 0..MAX_CHAIN {
            chain.input(INPUT_HEAVY).unwrap();
        }
        assert_eq!(chain.input(INPUT_SPECIAL), Ok(Some(5)));
        assert_eq!(chain.total_inputs, 10);
        assert_eq!(chain.completed, 1);
        chain.reset();
        assert_eq!(chain.last_completed, None);
        chain.reset_all();
        assert_eq!(
            (chain.pattern_count(), chain.total_inputs, chain.completed),
            (0, 0, 0)
        );
    }

    #[test]
    fn combo_refuses_bad_patterns_and_kinds() {
        let mut chain = ComboChain::with_defaults();
        assert_eq!(
            chain.register_pattern(1, &[]),
            Err(DepthError::EmptyPattern)
        );
        assert_eq!(
            chain.register_pattern(1, &[INPUT_LIGHT; MAX_PATTERN_LEN + 1]),
            Err(DepthError::PatternTooLong(MAX_PATTERN_LEN + 1))
        );
        assert_eq!(chain.register_pattern(1, &[9]), Err(DepthError::BadKind(9)));
        assert_eq!(chain.input(9), Err(DepthError::BadKind(9)));
        for id in 0..MAX_PATTERNS as u8 {
            assert_eq!(chain.register_pattern(id, &[INPUT_LIGHT]), Ok(true));
        }
        assert_eq!(
            chain.register_pattern(99, &[INPUT_LIGHT]),
            Err(DepthError::TooManyPatterns(99))
        );
    }

    #[test]
    fn depth_set_routes_events_into_victim_poise() {
        let mut set = DepthSet::new(TABLES_V1, 2);
        assert_eq!(set.slots(), 2);
        set.observe(&[FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: 30,
        }]);
        assert_eq!(set.poise(1).unwrap().current, 70);
        set.observe(&[FightEvent::Blocked {
            attacker: 0,
            victim: 1,
            chip: 24,
        }]);
        assert_eq!(set.poise(1).unwrap().current, 46);
        set.observe(&[
            FightEvent::Parried {
                attacker: 0,
                victim: 1,
            },
            FightEvent::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightEvent::StaminaOut { fighter: 1 },
            FightEvent::Knockdown {
                attacker: 0,
                victim: 1,
            },
        ]);
        assert_eq!(set.poise(1).unwrap().current, 46);
        assert_eq!(set.poise(1).unwrap().hits_absorbed, 2);
        assert_eq!(set.resolve(ATTACK_DAMAGE, true), block_chip(ATTACK_DAMAGE));
    }

    #[test]
    fn depth_set_guards_slots_and_mirrors_stamina() {
        let mut set = DepthSet::new(TABLES_V1, 2);
        assert_eq!(set.try_spend(9, 10), Err(DepthError::BadSlot(9)));
        assert_eq!(set.stamina(9), None);
        assert_eq!(set.poise(9), None);
        assert_eq!(set.try_spend(0, 25), Ok(true));
        assert_eq!(set.stamina(0).unwrap().current, MAX_STAMINA - 25);
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].stamina = 10;
        set.sync_stamina_from(&state);
        assert_eq!(set.stamina(0).unwrap().current, 10);
        assert_eq!(state.fighters[0].stamina, 10);
        assert_eq!(
            set.register_pattern(5, &[INPUT_LIGHT, INPUT_HEAVY]),
            Ok(true)
        );
        assert_eq!(set.record_strike(INPUT_LIGHT), Ok(None));
        assert_eq!(set.record_strike(INPUT_HEAVY), Ok(Some(5)));
        assert_eq!(set.record_strike(9), Err(DepthError::BadKind(9)));
        // Idle dodge intent still ticks every pool exactly once.
        let stamina_before = set.stamina(1).unwrap().current;
        let _ = mk(1, DODGE);
        set.tick();
        assert!(set.stamina(1).unwrap().current >= stamina_before);
    }

    #[test]
    fn depth_budgets_hold() {
        const { assert!(MAX_PATTERN_LEN <= MAX_CHAIN) };
        assert_eq!(MAX_SLOTS, 16);
        assert!(core::mem::size_of::<DepthSet>() <= 2048);
        assert!(core::mem::size_of::<ComboChain>() <= 256);
    }

    #[test]
    fn depth_script_replays_identically() {
        let run = || {
            let mut set = DepthSet::new(TABLES_V1, 2);
            set.register_pattern(1, &[INPUT_LIGHT, INPUT_LIGHT])
                .unwrap();
            let mut all = Vec::new();
            for tick in 0..48u32 {
                if tick % 3 == 0 {
                    all.push(set.record_strike(INPUT_LIGHT).unwrap());
                }
                if tick == 10 {
                    set.observe(&[FightEvent::Hit {
                        attacker: 0,
                        victim: 1,
                        damage: 40,
                    }]);
                }
                let _ = set.try_spend(0, 5);
                set.tick();
            }
            (all, set)
        };
        let (a_events, a_state) = run();
        let (b_events, b_state) = run();
        assert_eq!(a_events, b_events);
        assert_eq!(a_state, b_state);
        // Sixteen LIGHT strikes under the two-light tail: every strike past
        // the first completes, so fifteen completions on both runs.
        assert_eq!(a_state.chain().completed, 15);
    }

    #[test]
    fn poise_kinds_heavy_staggers_light_does_not() {
        use crate::fight::TABLES_V2;
        use crate::tables::{
            HEAVY_POISE_DAMAGE_V2, HEAVY_STAMINA_COST_V2, LIGHT_POISE_DAMAGE_V2,
            LIGHT_STAMINA_COST_V2,
        };
        const {
            assert!(HEAVY_POISE_DAMAGE_V2 > DEFAULT_MAX_POISE / 2);
            assert!(LIGHT_POISE_DAMAGE_V2 < DEFAULT_MAX_POISE);
            assert!(HEAVY_POISE_DAMAGE_V2 > LIGHT_POISE_DAMAGE_V2);
            assert!(HEAVY_STAMINA_COST_V2 > LIGHT_STAMINA_COST_V2);
        }
        let run = |kind: u8| {
            let mut set = DepthSet::new(TABLES_V2, 2);
            let tripped = set.apply_poise_kind(1, kind).expect("slot and kind valid");
            let held = set.poise(1).expect("slot live").staggered;
            (tripped, held)
        };
        let (heavy_trip, heavy_hold) = run(INPUT_HEAVY);
        assert!(heavy_trip, "heavy once must stagger a full meter");
        assert!(heavy_hold);
        let (light_trip, light_hold) = run(INPUT_LIGHT);
        assert!(!light_trip, "single light must not stagger");
        assert!(!light_hold);
        assert_eq!(run(INPUT_HEAVY), run(INPUT_HEAVY));
        assert_eq!(run(INPUT_LIGHT), run(INPUT_LIGHT));
    }

    #[test]
    fn poise_kinds_two_lights_in_60_ticks_stagger() {
        use crate::fight::TABLES_V2;
        let run = || {
            let mut set = DepthSet::new(TABLES_V2, 2);
            let first = set
                .apply_poise_kind(1, INPUT_LIGHT)
                .expect("first light valid");
            for _ in 0..10 {
                set.tick();
            }
            let second = set
                .apply_poise_kind(1, INPUT_LIGHT)
                .expect("second light valid");
            let held = set.poise(1).expect("slot live").staggered;
            (first, second, held)
        };
        let (first, second, held) = run();
        assert!(!first, "first light alone must not stagger");
        assert!(second, "second light inside 60 ticks must stagger");
        assert!(held);
        assert_eq!(run(), run(), "two-light bout replays exactly");
    }

    #[test]
    fn poise_kinds_heavy_costs_more_stamina_than_light() {
        use crate::fight::{stamina_cost_for_kind, TABLES_V2};
        use crate::tables::{HEAVY_STAMINA_COST_V2, LIGHT_STAMINA_COST_V2};
        const {
            assert!(HEAVY_STAMINA_COST_V2 > LIGHT_STAMINA_COST_V2);
        }
        assert_eq!(
            stamina_cost_for_kind(&TABLES_V2, INPUT_LIGHT),
            LIGHT_STAMINA_COST_V2
        );
        assert_eq!(
            stamina_cost_for_kind(&TABLES_V2, INPUT_HEAVY),
            HEAVY_STAMINA_COST_V2
        );
        let mut light_set = DepthSet::new(TABLES_V2, 2);
        assert_eq!(light_set.spend_stamina_kind(0, INPUT_LIGHT), Ok(true));
        let after_light = light_set.stamina(0).expect("slot live").current;
        let mut heavy_set = DepthSet::new(TABLES_V2, 2);
        assert_eq!(heavy_set.spend_stamina_kind(0, INPUT_HEAVY), Ok(true));
        let after_heavy = heavy_set.stamina(0).expect("slot live").current;
        assert!(
            after_heavy < after_light,
            "heavy must drain more stamina than light"
        );
        assert_eq!(
            light_set.spend_stamina_kind(9, INPUT_LIGHT),
            Err(DepthError::BadSlot(9))
        );
        assert_eq!(
            light_set.spend_stamina_kind(0, 9),
            Err(DepthError::BadKind(9))
        );
        assert_eq!(
            light_set.spend_stamina_kind(0, INPUT_SPECIAL),
            Err(DepthError::BadKind(INPUT_SPECIAL))
        );
    }

    #[test]
    fn poise_kinds_regen_waits_for_v2_delay() {
        use crate::fight::{poise_regen_delay_for_tables, TABLES_V1, TABLES_V2};
        use crate::tables::POISE_REGEN_DELAY_TICKS_V2;
        assert_eq!(POISE_REGEN_DELAY_TICKS_V2, 30);
        assert_eq!(poise_regen_delay_for_tables(&TABLES_V2), 30);
        assert_eq!(
            poise_regen_delay_for_tables(&TABLES_V1),
            DEFAULT_POISE_DELAY_TICKS
        );
        let run = || {
            let mut set = DepthSet::new(TABLES_V2, 2);
            set.apply_poise_kind(1, INPUT_LIGHT).expect("light valid");
            let hurt = set.poise(1).expect("slot live").current;
            for _ in 0..29 {
                set.tick();
            }
            let before = set.poise(1).expect("slot live").current;
            set.tick();
            let after = set.poise(1).expect("slot live").current;
            (hurt, before, after)
        };
        let (hurt, before, after) = run();
        assert_eq!(before, hurt, "no regen inside the 30-tick delay");
        assert_eq!(after, hurt + 1, "regen starts after the delay");
        assert_eq!(run(), run(), "regen bout replays exactly");
    }

    #[test]
    fn poise_kinds_v1_carries_no_poise_damage() {
        use crate::fight::{poise_damage_for_kind, TABLES_V1, TABLES_V2};
        use crate::tables::{HEAVY_POISE_DAMAGE_V2, LIGHT_POISE_DAMAGE_V2};
        assert_eq!(poise_damage_for_kind(&TABLES_V1, INPUT_LIGHT), 0);
        assert_eq!(poise_damage_for_kind(&TABLES_V1, INPUT_HEAVY), 0);
        assert_eq!(
            poise_damage_for_kind(&TABLES_V2, INPUT_LIGHT),
            LIGHT_POISE_DAMAGE_V2
        );
        assert_eq!(
            poise_damage_for_kind(&TABLES_V2, INPUT_HEAVY),
            HEAVY_POISE_DAMAGE_V2
        );
        let mut set = DepthSet::new(TABLES_V1, 2);
        assert_eq!(set.apply_poise_kind(1, INPUT_HEAVY), Ok(false));
        assert!(!set.poise(1).expect("slot live").staggered);
        assert_eq!(
            set.apply_poise_kind(9, INPUT_LIGHT),
            Err(DepthError::BadSlot(9))
        );
        assert_eq!(set.apply_poise_kind(1, 9), Err(DepthError::BadKind(9)));
    }
}
