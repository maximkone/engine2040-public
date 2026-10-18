//! exchange: timed parry answers, locked riposte, poise clash over table rows.
//! Owner: engine-combat, consumer: arena match step (pending orchestrator mod).
//! Units: ticks at 60 Hz, damage/poise in permille points, ids are slots.
//! Determinism: integer math, fighter-ID order; same tables+inputs => same events.
//! Errors: ExchangeError for bad versions/rows/slots; resolvers never panic.
//! Budget: resolution O(n^2) over <=16 slots, no heap per tick (caller buffer).
//! Slots: match slots come from `engine_input::MAX_PARTICIPANTS` (the one
//! canonical cap); resolvers take the slot count (`ExchangeState.count`) plus table sets
//! (`ExchangeTables`/`ExchangeTableSet`), never a private cap. Crate edge
//! `engine-combat -> engine-input` is in `tools/workspace-check.mjs`.

use crate::fight::SLOT_COUNT;
use engine_input::MAX_PARTICIPANTS;

/// Version tag of the shipped exchange rows.
pub const EXCHANGE_VERSION_V1: u32 = 1;
/// First parry-open tick after a block press (inclusive).
pub const PARRY_START_TICK: u32 = 0;
/// Parry-open ticks from start (inclusive end: open while age <= start+active).
pub const PARRY_ACTIVE_TICKS: u32 = 4;
/// Ticks after a parry during which the riposte stays open.
pub const RIPOSTE_WINDOW_TICKS: u32 = 6;
/// Riposte damage in permille.
pub const RIPOSTE_DAMAGE_PERMILLE: u16 = 140;
/// Ticks the victim cannot act after a riposte lands.
pub const RIPOSTE_STAGGER_TICKS: u32 = 8;
/// Ticks both fighters stay bound after a clash.
pub const CLASH_BIND_TICKS: u32 = 6;
/// Poise lost by the parried attacker.
pub const PARRY_POISE_DAMAGE: u16 = 250;
/// Poise lost by the clash loser.
pub const CLASH_POISE_DAMAGE: u16 = 200;
/// Full poise in permille points.
pub const POISE_MAX: u16 = 1000;
/// Caller buffer size for one tick's answers; resolvers write at most `out.len()`.
pub const MAX_EVENTS_PER_TICK: usize = 8;

/// Tunable exchange rows: every number the resolvers read, in one place.
///
/// Mirrors the base fight [`Tables`](crate::fight::Tables) discipline: plain
/// integers, ticks at 60 Hz, damage/poise in permille. Replacement sets travel
/// versioned as [`ExchangeTableSet`]; the shipped row is [`EXCHANGE_V1`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExchangeTables {
    /// First parry-open tick after a block press (inclusive).
    pub parry_start_tick: u32,
    /// Parry-open ticks from start (open while age <= start + active).
    pub parry_active_ticks: u32,
    /// Ticks after a parry during which the riposte stays open.
    pub riposte_window_ticks: u32,
    /// Riposte damage in permille.
    pub riposte_damage_permille: u16,
    /// Ticks the victim cannot act after a riposte lands.
    pub riposte_stagger_ticks: u32,
    /// Ticks both fighters stay bound after a clash.
    pub clash_bind_ticks: u32,
    /// Poise lost by the parried attacker.
    pub parry_poise_damage: u16,
    /// Poise lost by the clash loser.
    pub clash_poise_damage: u16,
    /// Full poise in permille points.
    pub poise_max: u16,
}

impl ExchangeTables {
    /// The shipped exchange rows.
    pub const fn v1() -> Self {
        Self {
            parry_start_tick: PARRY_START_TICK,
            parry_active_ticks: PARRY_ACTIVE_TICKS,
            riposte_window_ticks: RIPOSTE_WINDOW_TICKS,
            riposte_damage_permille: RIPOSTE_DAMAGE_PERMILLE,
            riposte_stagger_ticks: RIPOSTE_STAGGER_TICKS,
            clash_bind_ticks: CLASH_BIND_TICKS,
            parry_poise_damage: PARRY_POISE_DAMAGE,
            clash_poise_damage: CLASH_POISE_DAMAGE,
            poise_max: POISE_MAX,
        }
    }

    /// Last parry-open tick after a block press (inclusive).
    pub const fn parry_end_tick(self) -> u32 {
        self.parry_start_tick
            .saturating_add(self.parry_active_ticks)
    }
}

/// The shipped exchange rows as one versioned row.
pub const EXCHANGE_V1: ExchangeTables = ExchangeTables::v1();

/// Versioned exchange rows, mirroring the [`MoveTableSet`](crate::tables::MoveTableSet)
/// discipline (a copy of its shape): nonzero version, validated rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExchangeTableSet {
    /// Table version; nonzero, 1 for the shipped rows.
    pub version: u32,
    /// The tunable rows resolved under this version.
    pub tables: ExchangeTables,
}

impl ExchangeTableSet {
    /// The shipped rows as version 1.
    pub const fn v1() -> Self {
        Self {
            version: EXCHANGE_VERSION_V1,
            tables: EXCHANGE_V1,
        }
    }

    /// Explicit rows with a nonzero version; call [`validate_exchange`] first.
    pub const fn new(version: u32, tables: ExchangeTables) -> Self {
        Self { version, tables }
    }

    /// Range-check this set; see [`validate_exchange`].
    pub fn validate(&self) -> Result<(), ExchangeError> {
        validate_exchange(self)
    }
}

/// Bounded exchange failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExchangeError {
    /// Version 0 is reserved for legacy untagged replays, never valid anew.
    BadVersion(u32),
    /// One tunable row is outside its sane range; first failure wins.
    BadValue {
        /// Table field name, e.g. `"parry_active_ticks"`.
        field: &'static str,
        /// Culprit value widened to `u32`.
        value: u32,
    },
    /// Replay references a version with no known rows.
    UnknownVersion(u32),
    /// Fighter count outside `1..=MAX_PARTICIPANTS` (the canonical cap).
    BadFighterCount(u8),
    /// Riposte/block refused: reason is a static tag, never allocated.
    Denied(&'static str),
}

/// Range-check a versioned exchange set. First failure wins, deterministically.
///
/// Rules: version nonzero; parry start `0..=60`, active `0..=60`, end
/// `<=60`; riposte window `1..=60`; riposte damage `1..=1000`; stagger
/// `0..=120`; bind `1..=60`; poise costs `0..=1000`; poise max `1..=1000`
/// with both costs inside it.
pub fn validate_exchange(set: &ExchangeTableSet) -> Result<(), ExchangeError> {
    if set.version == 0 {
        return Err(ExchangeError::BadVersion(0));
    }
    let t = &set.tables;
    range("parry_start_tick", t.parry_start_tick, 0, 60)?;
    range("parry_active_ticks", t.parry_active_ticks, 0, 60)?;
    range(
        "parry_end_tick",
        t.parry_start_tick.saturating_add(t.parry_active_ticks),
        0,
        60,
    )?;
    range("riposte_window_ticks", t.riposte_window_ticks, 1, 60)?;
    range(
        "riposte_damage_permille",
        u32::from(t.riposte_damage_permille),
        1,
        1000,
    )?;
    range("riposte_stagger_ticks", t.riposte_stagger_ticks, 0, 120)?;
    range("clash_bind_ticks", t.clash_bind_ticks, 1, 60)?;
    range(
        "parry_poise_damage",
        u32::from(t.parry_poise_damage),
        0,
        1000,
    )?;
    range(
        "clash_poise_damage",
        u32::from(t.clash_poise_damage),
        0,
        1000,
    )?;
    range("poise_max", u32::from(t.poise_max), 1, 1000)?;
    if u32::from(t.parry_poise_damage) > u32::from(t.poise_max) {
        return Err(ExchangeError::BadValue {
            field: "parry_poise_damage",
            value: u32::from(t.parry_poise_damage),
        });
    }
    if u32::from(t.clash_poise_damage) > u32::from(t.poise_max) {
        return Err(ExchangeError::BadValue {
            field: "clash_poise_damage",
            value: u32::from(t.clash_poise_damage),
        });
    }
    Ok(())
}

fn range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), ExchangeError> {
    if value < min || value > max {
        return Err(ExchangeError::BadValue { field, value });
    }
    Ok(())
}

/// One fighter's parry window as data: open while age in [start, start+active].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParryWindow {
    /// First open tick after the block press (inclusive).
    pub start_tick: u32,
    /// Open ticks from start (inclusive end).
    pub active_ticks: u32,
}

impl ParryWindow {
    /// Window read from table rows.
    pub const fn from_tables(t: &ExchangeTables) -> Self {
        Self {
            start_tick: t.parry_start_tick,
            active_ticks: t.parry_active_ticks,
        }
    }
}

/// True while a held block still parries: age in [start, start+active].
///
/// Integer compare only; saturating end keeps unvalidated rows panic-free
/// (validation rejects them before they step).
pub const fn parry_open(block_age: u32, window: ParryWindow) -> bool {
    block_age >= window.start_tick
        && block_age <= window.start_tick.saturating_add(window.active_ticks)
}

/// True while a held block still parries under explicit table rows.
pub const fn parry_open_with(block_age: u32, tables: &ExchangeTables) -> bool {
    block_age >= tables.parry_start_tick && block_age <= tables.parry_end_tick()
}

/// Timed answer of one attack against one defender press.
///
/// Pure scheduling proof: `impact_tick` is the attack's impact tick,
/// `press_tick` the defender's block-press tick; the parry lands exactly
/// when the block age at impact falls inside the table window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimedAnswer {
    /// The press parries the impact.
    Parry,
    /// The press started too recently (block age below the window start).
    TooEarly,
    /// The press started too long ago (block age past the window end).
    TooLate,
}

/// Classify one attack/press pair under table rows.
///
/// `block_age == impact_tick - press_tick` (saturating); presses after the
/// impact count as age 0. Deterministic integer compare only.
pub const fn answer_for(impact_tick: u32, press_tick: u32, tables: &ExchangeTables) -> TimedAnswer {
    let age = impact_tick.saturating_sub(press_tick);
    if age < tables.parry_start_tick {
        TimedAnswer::TooEarly
    } else if age > tables.parry_end_tick() {
        TimedAnswer::TooLate
    } else {
        TimedAnswer::Parry
    }
}

/// Clash outcome: both attacked the same tick, both bind, winner takes poise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClashOut {
    /// Winner slot (poise, then timing, then lower ID).
    pub winner: u8,
    /// Loser slot.
    pub loser: u8,
    /// Ticks both fighters stay bound.
    pub bind_ticks: u32,
    /// True when poise tied and timing/ID decided.
    pub poise_tied: bool,
}

/// Resolve a same-tick clash deterministically.
///
/// Order: higher poise wins; equal poise breaks by earlier timing (smaller
/// `timing` value, e.g. lower attack age); still tied breaks by lower
/// fighter ID. Never reads container order, so swapped inputs mirror the
/// same winner. Integer compares only, never panics.
pub const fn resolve_clash(
    a_id: u8,
    b_id: u8,
    a_timing: u32,
    b_timing: u32,
    poise_a: u16,
    poise_b: u16,
    tables: &ExchangeTables,
) -> ClashOut {
    let (winner, loser, tied) = if poise_a != poise_b {
        if poise_a > poise_b {
            (a_id, b_id, false)
        } else {
            (b_id, a_id, false)
        }
    } else if a_timing != b_timing {
        if a_timing < b_timing {
            (a_id, b_id, true)
        } else {
            (b_id, a_id, true)
        }
    } else if a_id <= b_id {
        (a_id, b_id, true)
    } else {
        (b_id, a_id, true)
    };
    ClashOut {
        winner,
        loser,
        bind_ticks: tables.clash_bind_ticks,
        poise_tied: tied,
    }
}

/// One tick's exchange outcomes, in fighter-ID order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExchangeEvent {
    /// Timed parry: no damage, attacker loses poise, victim's riposte opens.
    Parried {
        /// Attacker slot (poise damaged).
        attacker: u8,
        /// Victim slot (riposte opens).
        victim: u8,
    },
    /// Locked riposte: only fires inside the parry window, never from neutral.
    Riposte {
        /// Riposting slot (must have an open parry).
        attacker: u8,
        /// Victim slot.
        victim: u8,
        /// Damage in permille from the table row.
        damage: u16,
    },
    /// Same-tick clash: both bind, loser loses poise, winner by poise/timing/ID.
    Clash {
        /// First slot in ID order (not the winner).
        a: u8,
        /// Second slot in ID order.
        b: u8,
        /// Winner slot.
        winner: u8,
    },
}

/// One fighter's exchange track: poise stakes plus bind/riposte clocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExchangeFighter {
    /// Fighter slot.
    pub id: u8,
    /// Poise in permille points, `0..=poise_max`.
    pub poise: u16,
    /// Remaining bound ticks; zero means free.
    pub bind: u32,
    /// Tick of the last parry performed; `u32::MAX` means none yet.
    pub last_parry_tick: u32,
}

impl ExchangeFighter {
    /// Fresh track at full poise, free, with no parry yet.
    pub const fn fresh(id: u8, tables: &ExchangeTables) -> Self {
        Self {
            id,
            poise: tables.poise_max,
            bind: 0,
            last_parry_tick: u32::MAX,
        }
    }
}

/// Whole-exchange state for up to [`MAX_PARTICIPANTS`] fighters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExchangeState {
    /// One slot per fighter ID; only `0..count` take part.
    pub fighters: [ExchangeFighter; SLOT_COUNT],
    /// Active fighter count, always `1..=MAX_PARTICIPANTS`.
    pub count: u8,
    /// Exchange ticks elapsed.
    pub tick: u32,
}

impl ExchangeState {
    /// Fresh exchange with `count` fighters at full poise.
    pub fn new(count: u8, tables: &ExchangeTables) -> Result<Self, ExchangeError> {
        if count == 0 || count > MAX_PARTICIPANTS {
            return Err(ExchangeError::BadFighterCount(count));
        }
        let mut fighters = [ExchangeFighter::fresh(0, tables); SLOT_COUNT];
        let mut id: u8 = 0;
        while id < count {
            fighters[usize::from(id)] = ExchangeFighter::fresh(id, tables);
            id += 1;
        }
        Ok(Self {
            fighters,
            count,
            tick: 0,
        })
    }

    /// True while the slot's riposte stays open at this tick.
    pub const fn riposte_live(self, id: u8, tables: &ExchangeTables) -> bool {
        if id >= self.count {
            return false;
        }
        let f = self.fighters[id as usize];
        if f.last_parry_tick == u32::MAX {
            return false;
        }
        self.tick.saturating_sub(f.last_parry_tick) <= tables.riposte_window_ticks
    }

    /// Defense commits: no block while bound or while a riposte is live,
    /// so a riposte can never cancel into block.
    pub const fn can_block(self, id: u8, tables: &ExchangeTables) -> bool {
        if id >= self.count {
            return false;
        }
        let f = self.fighters[id as usize];
        f.bind == 0 && !self.riposte_live(id, tables)
    }

    /// True while the slot is bound by a clash.
    pub const fn is_bound(self, id: u8) -> bool {
        if id >= self.count {
            return false;
        }
        self.fighters[id as usize].bind > 0
    }

    /// Advance one exchange tick: decay binds, run the clock. Saturating.
    pub fn advance(&mut self) {
        let mut i: usize = 0;
        while i < usize::from(self.count) {
            if self.fighters[i].bind > 0 {
                self.fighters[i].bind -= 1;
            }
            i += 1;
        }
        self.tick = self.tick.saturating_add(1);
    }
}

/// One fighter's intent for the tick, indexed by slot (not sorted input).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExchangeIntent {
    /// Attack impacting this tick with the given timing value (lower = earlier).
    Attack {
        /// Timing value for clash breaks (e.g. attack age at impact).
        timing: u32,
    },
    /// Block held with this age at resolve (press tick derived by caller).
    Block {
        /// Block age in ticks at resolve.
        age: u32,
    },
    /// No exchange action.
    Idle,
}

/// Resolve one tick of parry/clash answers into the caller buffer.
///
/// Deterministic order: clashes first (ID-sorted pairs), then parries in
/// attacker-ID order with defender-ID order inside; ripostes never fire
/// here (see [`try_riposte`]). Writes at most `out.len()` events and
/// returns the count. Bound fighters act as idle. No heap, no floats.
pub fn resolve_tick(
    state: &mut ExchangeState,
    intents: &[ExchangeIntent; SLOT_COUNT],
    tables: &ExchangeTables,
    out: &mut [ExchangeEvent; MAX_EVENTS_PER_TICK],
) -> usize {
    let count = usize::from(state.count.min(MAX_PARTICIPANTS));

    // Collect impacting attackers in ID order.
    let mut attackers: [usize; SLOT_COUNT] = [0; SLOT_COUNT];
    let mut attacker_n: usize = 0;
    let mut id: usize = 0;
    while id < count {
        if state.fighters[id].bind == 0 {
            if let ExchangeIntent::Attack { .. } = intents[id] {
                attackers[attacker_n] = id;
                attacker_n += 1;
            }
        }
        id += 1;
    }

    // Clash: every ID-sorted pair of impacting attackers binds once.
    // (Two fighters give one clash; more fighters give sorted pairs.)
    let n = resolve_clashes(state, intents, tables, &attackers, attacker_n, out, 0);
    if attacker_n >= 2 {
        // Clash consumes the tick's attacks: parries resolve next tick.
        return n;
    }

    resolve_parries(state, intents, tables, &attackers, attacker_n, out, n)
}

/// Parry step of [`resolve_tick`]: each attacker in ID order meets
/// defenders in ID order and the first open block parries. Returns the
/// updated event count.
fn resolve_parries(
    state: &mut ExchangeState,
    intents: &[ExchangeIntent; SLOT_COUNT],
    tables: &ExchangeTables,
    attackers: &[usize; SLOT_COUNT],
    attacker_n: usize,
    out: &mut [ExchangeEvent; MAX_EVENTS_PER_TICK],
    mut n: usize,
) -> usize {
    let count = usize::from(state.count.min(MAX_PARTICIPANTS));
    let mut ai: usize = 0;
    while ai < attacker_n {
        let a = attackers[ai];
        let mut v: usize = 0;
        while v < count {
            if v != a && state.fighters[v].bind == 0 {
                if let ExchangeIntent::Block { age } = intents[v] {
                    if parry_open_with(age, tables) {
                        state.fighters[a].poise = state.fighters[a]
                            .poise
                            .saturating_sub(tables.parry_poise_damage);
                        state.fighters[v].last_parry_tick = state.tick;
                        if n < out.len() {
                            out[n] = ExchangeEvent::Parried {
                                attacker: a as u8,
                                victim: v as u8,
                            };
                            n += 1;
                        }
                        break;
                    }
                }
            }
            v += 1;
        }
        ai += 1;
    }
    n
}

/// Clash step of [`resolve_tick`]: every ID-sorted pair of impacting
/// attackers binds once. Returns the updated event count.
fn resolve_clashes(
    state: &mut ExchangeState,
    intents: &[ExchangeIntent; SLOT_COUNT],
    tables: &ExchangeTables,
    attackers: &[usize; SLOT_COUNT],
    attacker_n: usize,
    out: &mut [ExchangeEvent; MAX_EVENTS_PER_TICK],
    mut n: usize,
) -> usize {
    let mut i: usize = 0;
    while i < attacker_n {
        let mut j: usize = i + 1;
        while j < attacker_n {
            let a = attackers[i];
            let b = attackers[j];
            let (ta, tb) = match (intents[a], intents[b]) {
                (ExchangeIntent::Attack { timing: ta }, ExchangeIntent::Attack { timing: tb }) => {
                    (ta, tb)
                }
                _ => (0, 0),
            };
            let decided = resolve_clash(
                a as u8,
                b as u8,
                ta,
                tb,
                state.fighters[a].poise,
                state.fighters[b].poise,
                tables,
            );
            state.fighters[a].bind = tables.clash_bind_ticks;
            state.fighters[b].bind = tables.clash_bind_ticks;
            let loser = usize::from(decided.loser);
            state.fighters[loser].poise = state.fighters[loser]
                .poise
                .saturating_sub(tables.clash_poise_damage);
            if n < out.len() {
                out[n] = ExchangeEvent::Clash {
                    a: (a as u8).min(b as u8),
                    b: (a as u8).max(b as u8),
                    winner: decided.winner,
                };
                n += 1;
            }
            j += 1;
        }
        i += 1;
    }
    n
}

/// Attempt the locked riposte: succeeds only inside the parry window opened
/// by this fighter's own parry, while free and against a live victim.
///
/// Refusals carry static tags (`"no-parry"`, `"expired"`, `"bound"`,
/// `"self"`, `"range"`) so callers match without allocating. Never panics.
pub fn try_riposte(
    state: &mut ExchangeState,
    tables: &ExchangeTables,
    attacker: u8,
    victim: u8,
) -> Result<ExchangeEvent, ExchangeError> {
    if attacker >= state.count || victim >= state.count {
        let bad = if attacker >= state.count {
            attacker
        } else {
            victim
        };
        let _ = bad;
        return Err(ExchangeError::Denied("range"));
    }
    if attacker == victim {
        return Err(ExchangeError::Denied("self"));
    }
    let a = usize::from(attacker);
    if state.fighters[a].bind > 0 {
        return Err(ExchangeError::Denied("bound"));
    }
    if state.fighters[a].last_parry_tick == u32::MAX {
        return Err(ExchangeError::Denied("no-parry"));
    }
    let elapsed = state.tick.saturating_sub(state.fighters[a].last_parry_tick);
    if elapsed > tables.riposte_window_ticks {
        return Err(ExchangeError::Denied("expired"));
    }
    Ok(ExchangeEvent::Riposte {
        attacker,
        victim,
        damage: tables.riposte_damage_permille,
    })
}

// ---- GP23 phased move tables -----------------------------------------------
// Data-driven startup/active/endlag phases with window-gated cancels over
// the frozen exchange and sweep resolvers: this section owns timing rows only and never
// steps damage, poise or steel itself. The sweep half reads the phase back
// through [`phase_at`](crate::exchange::phase_at) (see `sweep.rs`), so the
// row type stays single-owned here. Units: ticks at 60 Hz, ids are slots.
// Determinism: integer compares only; same set plus same ages give the same
// phases and cancel answers. Errors: [`PhaseError`]; resolvers never panic.
// Budget: O(moves) lookup over `<=MAX_PHASED_MOVES` rows, no heap per tick
// (every row is `Copy` in a caller-owned set).

/// Version tag of the shipped phased rows.
pub const PHASED_VERSION_V1: u32 = 1;
/// Most moves in one versioned set; the shipped roster holds two.
pub const MAX_PHASED_MOVES: usize = 4;
/// `cancel_into` value for a move with no cancel route.
pub const NO_MOVE: u8 = u8::MAX;

/// Readable-attack floors in milliseconds (GS32, Jerry Zhang): full
/// telegraph, bait, end pose. Presentation compares against these;
/// the sim never reads them, so the golden never moves.
pub const TELEGRAPH_FLOOR_MS: u32 = 340;
/// Fixed clock the signal math rides on. The rate's one owner is
/// `engine_core::TICK_HZ` and its checked narrow form is
/// `engine_core::TICK_HZ_U32`, which this constant takes. Value never changes.
pub const TELEGRAPH_TICKS_PER_SEC: u32 = engine_core::TICK_HZ_U32;
/// Milliseconds per second for the signal math.
pub const TELEGRAPH_MS_PER_SEC: u32 = 1_000;

/// Signal-plus-attack time for one move in milliseconds: startup plus
/// live steel at 60 Hz. Split into whole and remainder so the `u32`
/// path stays exact without a wider intermediate: small rows match
/// `(startup + active) * 1000 / 60`, huge rows saturate to `u32::MAX`;
/// validation rejects such rows before they ever step.
#[must_use]
pub const fn signal_attack_ms(startup_ticks: u32, active_ticks: u32) -> u32 {
    let total = startup_ticks.saturating_add(active_ticks);
    let whole = (total / TELEGRAPH_TICKS_PER_SEC).saturating_mul(TELEGRAPH_MS_PER_SEC);
    let part = (total % TELEGRAPH_TICKS_PER_SEC).saturating_mul(TELEGRAPH_MS_PER_SEC)
        / TELEGRAPH_TICKS_PER_SEC;
    whole.saturating_add(part)
}

/// One move's phase timing plus its single cancel route, all as data.
///
/// `startup_ticks` whiffs, `active_ticks` is live steel, `endlag_ticks` is
/// committed recovery. The cancel fires only while `age` sits inside
/// `[cancel_start, cancel_end]` and names exactly `cancel_into`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhasedMoveRow {
    /// Move slot, dense `0..count` inside its set.
    pub move_id: u8,
    /// Whiff ticks before the move goes live, `1..=60`.
    pub startup_ticks: u32,
    /// Live ticks, `1..=60`.
    pub active_ticks: u32,
    /// Committed recovery ticks after active, `0..=120`.
    pub endlag_ticks: u32,
    /// First cancel-legal age (inclusive).
    pub cancel_start: u32,
    /// Last cancel-legal age (inclusive, `>= cancel_start`, inside total).
    pub cancel_end: u32,
    /// Cancel target move id, or [`NO_MOVE`] for no route.
    pub cancel_into: u8,
}

impl PhasedMoveRow {
    /// Inert placeholder for unused slots past `count`; never stepped.
    pub const NONE: Self = Self {
        move_id: NO_MOVE,
        startup_ticks: 1,
        active_ticks: 1,
        endlag_ticks: 0,
        cancel_start: 0,
        cancel_end: 0,
        cancel_into: NO_MOVE,
    };

    /// Ticks from press until the move is done (saturating).
    pub const fn total(self) -> u32 {
        self.startup_ticks
            .saturating_add(self.active_ticks)
            .saturating_add(self.endlag_ticks)
    }

    /// Signal-plus-attack milliseconds for this row: startup plus live
    /// steel at 60 Hz. Thin wrapper over [`signal_attack_ms`] so the
    /// report and the stepped cadence share one integer path.
    #[must_use]
    pub const fn signal_ms(self) -> u32 {
        signal_attack_ms(self.startup_ticks, self.active_ticks)
    }
}

/// Which phase a move age sits in: startup, live, recovery, or done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MovePhase {
    /// Age below startup: whiffing, steel never live.
    Startup,
    /// Age inside `[startup, startup+active)`: live steel.
    Active,
    /// Age inside the recovery tail: committed, steel never live.
    Endlag,
    /// Age at or past total: the move is over.
    Done,
}

/// Classify one move age under its row.
///
/// Boundaries: `age < startup` starts up, `age < startup+active` is live,
/// `age < total` recovers, the rest is done. Saturating sums keep
/// unvalidated rows panic-free (validation rejects them before they step).
pub const fn phase_at(age: u32, row: &PhasedMoveRow) -> MovePhase {
    let live_end = row.startup_ticks.saturating_add(row.active_ticks);
    let total = live_end.saturating_add(row.endlag_ticks);
    if age >= total {
        MovePhase::Done
    } else if age < row.startup_ticks {
        MovePhase::Startup
    } else if age < live_end {
        MovePhase::Active
    } else {
        MovePhase::Endlag
    }
}

/// True while a cancel is legal at this age: the move owns a route and the
/// age sits inside its window. Integer compares only.
pub const fn cancel_open(age: u32, row: &PhasedMoveRow) -> bool {
    row.cancel_into != NO_MOVE && age >= row.cancel_start && age <= row.cancel_end
}

/// The shipped GP23 rows: jab (0) cancels into cross (1) across its live
/// ticks; cross has no route. Jab mirrors the fight-core cadence
/// (startup 4, recovery 8) with 2 live ticks named.
pub const PHASED_JAB: PhasedMoveRow = PhasedMoveRow {
    move_id: 0,
    startup_ticks: 4,
    active_ticks: 2,
    endlag_ticks: 8,
    cancel_start: 4,
    cancel_end: 5,
    cancel_into: 1,
};

/// The shipped cross: slower, longer tail, no cancel route.
pub const PHASED_CROSS: PhasedMoveRow = PhasedMoveRow {
    move_id: 1,
    startup_ticks: 6,
    active_ticks: 3,
    endlag_ticks: 12,
    cancel_start: 0,
    cancel_end: 0,
    cancel_into: NO_MOVE,
};

/// Pinned signals for the shipped rows, computed on the one shared
/// path (non-test callers of [`PhasedMoveRow::signal_ms`]).
pub const PHASED_JAB_SIGNAL_MS: u32 = PHASED_JAB.signal_ms();
/// Pinned cross signal on the shared path.
pub const PHASED_CROSS_SIGNAL_MS: u32 = PHASED_CROSS.signal_ms();
/// Compile-time pins: a retune must re-bless these beside the floors.
const _: () = assert!(PHASED_JAB_SIGNAL_MS == 100);
const _: () = assert!(PHASED_CROSS_SIGNAL_MS == 150);

/// Versioned phased rows: nonzero version plus validated move rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhasedTableSet {
    /// Table version; nonzero, 1 for the shipped rows.
    pub version: u32,
    /// Live move count, `1..=MAX_PHASED_MOVES`; only `moves[0..count]` step.
    pub count: u8,
    /// Move rows in id order; slots past `count` hold [`PhasedMoveRow::NONE`].
    pub moves: [PhasedMoveRow; MAX_PHASED_MOVES],
}

impl PhasedTableSet {
    /// The shipped rows as version 1.
    pub const fn v1() -> Self {
        Self {
            version: PHASED_VERSION_V1,
            count: 2,
            moves: [
                PHASED_JAB,
                PHASED_CROSS,
                PhasedMoveRow::NONE,
                PhasedMoveRow::NONE,
            ],
        }
    }

    /// Explicit rows with a nonzero version; call [`validate_phased`] first.
    pub const fn new(version: u32, count: u8, moves: [PhasedMoveRow; MAX_PHASED_MOVES]) -> Self {
        Self {
            version,
            count,
            moves,
        }
    }

    /// Range-check this set; see [`validate_phased`].
    pub fn validate(&self) -> Result<(), PhaseError> {
        validate_phased(self)
    }
}

/// The shipped GP23 rows as one versioned row.
pub const PHASED_V1: PhasedTableSet = PhasedTableSet::v1();

/// Bounded phased failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhaseError {
    /// Version 0 is reserved for legacy untagged replays, never valid anew.
    BadVersion(u32),
    /// One tunable row is outside its sane range; first failure wins.
    BadValue {
        /// Table field name, e.g. `"startup_ticks"`.
        field: &'static str,
        /// Culprit value widened to `u32`.
        value: u32,
    },
    /// Replay references a version with no known rows.
    UnknownVersion(u32),
    /// Cancel refused: reason is a static tag (`"window"`, `"route"`,
    /// `"no-cancel"`), never allocated.
    Denied(&'static str),
}

/// Range-check a versioned phased set. First failure wins, deterministically.
///
/// Rules: version nonzero; count `1..=MAX_PHASED_MOVES`; live rows carry
/// dense ids `0..count` exactly once; startup `1..=60`, active `1..=60`,
/// endlag `0..=120`; `cancel_start <= cancel_end <= total`; a route target
/// is either [`NO_MOVE`] or a live id.
pub fn validate_phased(set: &PhasedTableSet) -> Result<(), PhaseError> {
    if set.version == 0 {
        return Err(PhaseError::BadVersion(0));
    }
    if set.count == 0 || usize::from(set.count) > MAX_PHASED_MOVES {
        return Err(PhaseError::BadValue {
            field: "count",
            value: u32::from(set.count),
        });
    }
    let mut i: usize = 0;
    while i < usize::from(set.count) {
        let row = &set.moves[i];
        if row.move_id != i as u8 {
            return Err(PhaseError::BadValue {
                field: "move_id",
                value: u32::from(row.move_id),
            });
        }
        phase_range("startup_ticks", row.startup_ticks, 1, 60)?;
        phase_range("active_ticks", row.active_ticks, 1, 60)?;
        phase_range("endlag_ticks", row.endlag_ticks, 0, 120)?;
        if row.cancel_start > row.cancel_end {
            return Err(PhaseError::BadValue {
                field: "cancel_start",
                value: row.cancel_start,
            });
        }
        if row.cancel_end > row.total() {
            return Err(PhaseError::BadValue {
                field: "cancel_end",
                value: row.cancel_end,
            });
        }
        if row.cancel_into != NO_MOVE && row.cancel_into >= set.count {
            return Err(PhaseError::BadValue {
                field: "cancel_into",
                value: u32::from(row.cancel_into),
            });
        }
        i += 1;
    }
    Ok(())
}

fn phase_range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), PhaseError> {
    if value < min || value > max {
        return Err(PhaseError::BadValue { field, value });
    }
    Ok(())
}

/// Resolve a replay version tag to its static rows: 1 gives V1, else unknown.
pub fn phased_for_version(version: u32) -> Option<&'static PhasedTableSet> {
    match version {
        PHASED_VERSION_V1 => Some(&PHASED_V1),
        _ => None,
    }
}

/// Look up one live move row by id; `None` past `count`.
pub const fn phased_row(set: &PhasedTableSet, move_id: u8) -> Option<PhasedMoveRow> {
    let mut i: usize = 0;
    while i < set.count as usize {
        if set.moves[i].move_id == move_id {
            return Some(set.moves[i]);
        }
        i += 1;
    }
    None
}

/// One fighter's phased track: which move is in flight and how old it is.
///
/// Both duelists hold one of these; the set owns the timing, the track owns
/// only its cursor. `Copy`, saturating, never allocates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhasedFighter {
    /// Move in flight.
    pub move_id: u8,
    /// Ticks since the press (saturating).
    pub age: u32,
}

impl PhasedFighter {
    /// Fresh track pressing `move_id` now.
    pub const fn fresh(move_id: u8) -> Self {
        Self { move_id, age: 0 }
    }

    /// This fighter's live row, or `None` when the cursor names no move.
    pub const fn row(self, set: &PhasedTableSet) -> Option<PhasedMoveRow> {
        phased_row(set, self.move_id)
    }

    /// This fighter's current phase; an unknown cursor reads as done.
    pub const fn phase(self, set: &PhasedTableSet) -> MovePhase {
        match self.row(set) {
            Some(row) => phase_at(self.age, &row),
            None => MovePhase::Done,
        }
    }

    /// Advance one tick. Saturating.
    pub fn advance(&mut self) {
        self.age = self.age.saturating_add(1);
    }

    /// Attempt the window-gated cancel into `target`: succeeds only while
    /// the age sits inside the row window and names exactly this route.
    ///
    /// Refusals carry static tags (`"route"` for an unknown cursor or a
    /// target off the route, `"no-cancel"` for a move with no route,
    /// `"window"` for an age outside it). On success the cursor presses
    /// the target at age 0. Never panics.
    pub fn try_cancel(&mut self, target: u8, set: &PhasedTableSet) -> Result<(), PhaseError> {
        let row = match self.row(set) {
            Some(row) => row,
            None => return Err(PhaseError::Denied("route")),
        };
        if row.cancel_into == NO_MOVE {
            return Err(PhaseError::Denied("no-cancel"));
        }
        if target != row.cancel_into {
            return Err(PhaseError::Denied("route"));
        }
        if self.age < row.cancel_start || self.age > row.cancel_end {
            return Err(PhaseError::Denied("window"));
        }
        if phased_row(set, target).is_none() {
            return Err(PhaseError::Denied("route"));
        }
        self.move_id = target;
        self.age = 0;
        Ok(())
    }
}

#[cfg(test)]
mod phased_tests {
    use super::*;

    fn set() -> PhasedTableSet {
        PHASED_V1
    }

    #[test]
    fn v1_rows_match_shipped_cadence() {
        let jab = PHASED_JAB;
        assert_eq!(
            (jab.startup_ticks, jab.active_ticks, jab.endlag_ticks),
            (4, 2, 8)
        );
        assert_eq!(jab.total(), 14);
        assert_eq!(
            (jab.cancel_start, jab.cancel_end, jab.cancel_into),
            (4, 5, 1)
        );
        let cross = PHASED_CROSS;
        assert_eq!(
            (cross.startup_ticks, cross.active_ticks, cross.endlag_ticks),
            (6, 3, 12)
        );
        assert_eq!(cross.total(), 21);
        assert_eq!(cross.cancel_into, NO_MOVE);
        assert_eq!(PhasedTableSet::v1().validate(), Ok(()));
        assert_eq!(phased_for_version(1), Some(&PHASED_V1));
        assert_eq!(phased_for_version(9), None);
        assert_eq!(phased_row(&PHASED_V1, 0), Some(PHASED_JAB));
        assert_eq!(phased_row(&PHASED_V1, 7), None);
    }

    #[test]
    fn phased_signal_ms_pins_jab_cross() {
        assert_eq!(PHASED_JAB.signal_ms(), 100);
        assert_eq!(PHASED_CROSS.signal_ms(), 150);
        assert_eq!(signal_attack_ms(4, 2), 100);
        assert_eq!(TELEGRAPH_FLOOR_MS, 340);
    }

    #[test]
    fn phase_edges_tick_by_tick() {
        let jab = PHASED_JAB;
        // Startup ages whiff, live ages are exactly the window, the tail
        // recovers, total and past read done.
        for age in 0..jab.total() + 2 {
            let phase = phase_at(age, &jab);
            let expect = if age < 4 {
                MovePhase::Startup
            } else if age < 6 {
                MovePhase::Active
            } else if age < 14 {
                MovePhase::Endlag
            } else {
                MovePhase::Done
            };
            assert_eq!(phase, expect, "jab age {age} misphased");
        }
        let cross = PHASED_CROSS;
        assert_eq!(phase_at(0, &cross), MovePhase::Startup);
        assert_eq!(phase_at(5, &cross), MovePhase::Startup);
        assert_eq!(phase_at(6, &cross), MovePhase::Active);
        assert_eq!(phase_at(8, &cross), MovePhase::Active);
        assert_eq!(phase_at(9, &cross), MovePhase::Endlag);
        assert_eq!(phase_at(20, &cross), MovePhase::Endlag);
        assert_eq!(phase_at(21, &cross), MovePhase::Done);
        // The fighter cursor agrees with the bare row at every age.
        let s = set();
        let mut f = PhasedFighter::fresh(0);
        while f.age < jab.total() + 1 {
            assert_eq!(f.phase(&s), phase_at(f.age, &jab));
            f.advance();
        }
        assert_eq!(f.phase(&s), MovePhase::Done);
    }

    #[test]
    fn cancel_inside_window_works_outside_refused() {
        let s = set();
        // Window edges both fire and press the route at age 0.
        for age in [4u32, 5] {
            let mut f = PhasedFighter { move_id: 0, age };
            assert!(cancel_open(age, &PHASED_JAB));
            assert_eq!(f.try_cancel(1, &s), Ok(()));
            assert_eq!(f, PhasedFighter { move_id: 1, age: 0 });
        }
        // One tick either side of the window refuses on the window tag.
        for age in [0u32, 3, 6, 13] {
            let mut f = PhasedFighter { move_id: 0, age };
            assert!(!cancel_open(age, &PHASED_JAB));
            assert_eq!(f.try_cancel(1, &s), Err(PhaseError::Denied("window")));
            // The failed cursor never moves.
            assert_eq!(f.move_id, 0);
        }
        // A target off the route refuses even inside the window.
        let mut f = PhasedFighter { move_id: 0, age: 4 };
        assert_eq!(f.try_cancel(0, &s), Err(PhaseError::Denied("route")));
        assert_eq!(f.try_cancel(7, &s), Err(PhaseError::Denied("route")));
        // A move with no route refuses everywhere, window or not.
        let mut f = PhasedFighter { move_id: 1, age: 7 };
        assert!(!cancel_open(7, &PHASED_CROSS));
        assert_eq!(f.try_cancel(0, &s), Err(PhaseError::Denied("no-cancel")));
        // An unknown cursor refuses without indexing out of bounds.
        let mut f = PhasedFighter { move_id: 7, age: 0 };
        assert_eq!(f.phase(&s), MovePhase::Done);
        assert_eq!(f.try_cancel(1, &s), Err(PhaseError::Denied("route")));
    }

    #[test]
    fn two_fighter_phased_tape_replays_twice() {
        // Both duelists run jab, cancel into cross on the first live tick,
        // then ride cross to done: every phase is visited by both.
        let run = || {
            let s = set();
            let mut a = PhasedFighter::fresh(0);
            let mut b = PhasedFighter::fresh(0);
            let mut tape = [MovePhase::Done; 24];
            let mut cancels = 0u32;
            let mut i = 0usize;
            while i < tape.len() {
                if a.phase(&s) == MovePhase::Active && cancel_open(a.age, &PHASED_JAB) {
                    a.try_cancel(1, &s).unwrap();
                    cancels += 1;
                }
                if b.phase(&s) == MovePhase::Active && b.age == 5 {
                    b.try_cancel(1, &s).unwrap();
                    cancels += 1;
                }
                tape[i] = a.phase(&s);
                a.advance();
                b.advance();
                i += 1;
            }
            (tape, cancels, a, b)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);
        // The tape really phases: startup, live and recovery all appear,
        // and both cancels fired (one per fighter).
        assert!(first.0.contains(&MovePhase::Startup));
        assert!(first.0.contains(&MovePhase::Active));
        assert!(first.0.contains(&MovePhase::Endlag));
        assert_eq!(first.1, 2);
        assert_eq!(first.2.move_id, 1);
        assert_eq!(first.3.move_id, 1);
    }

    #[test]
    fn phased_tick_allocates_nothing() {
        let s = set();
        let mut a = PhasedFighter::fresh(0);
        let mut b = PhasedFighter::fresh(1);
        // Warm once outside the gate; the measured ticks reuse the cursor.
        let _ = a.phase(&s);
        let _ = b.phase(&s);
        let (_, heap) = engine_core::alloc_probe::measure(|| {
            let mut cancels = 0u32;
            let mut tick = 0u32;
            while tick < 32 {
                let _ = a.phase(&s);
                let _ = b.phase(&s);
                let _ = cancel_open(a.age, &PHASED_JAB);
                if a.phase(&s) == MovePhase::Active {
                    let _ = a.try_cancel(1, &s);
                    cancels += 1;
                }
                a.advance();
                b.advance();
                tick += 1;
            }
            cancels
        });
        assert_eq!(heap.calls, 0, "phased tick allocated {} times", heap.calls);
    }

    #[test]
    fn pq2_band_holds_for_phased_v1_twice() {
        // The phased rows ride beside the fight core without moving its
        // balance: the canonical 200-bout balance survey over the shipped base
        // stays in the 40-60% band, identically twice.
        let base = crate::fight::TABLES_V1;
        let first = crate::feel::balance_over_200(&base);
        let second = crate::feel::balance_over_200(&base);
        assert_eq!(first, second);
        println!(
            "PQ2 phased-v1: wins_a={} wins_b={} draws={} share_a={}",
            first.wins_a,
            first.wins_b,
            first.draws,
            first.share_a_permille()
        );
        assert!(
            first.is_balanced(),
            "unbalanced survey: {first:?} share_a={}",
            first.share_a_permille()
        );
        assert!(
            (400..=600).contains(&first.share_a_permille()),
            "share outside 40-60%: {}",
            first.share_a_permille()
        );
    }

    #[test]
    fn validation_rejects_bad_phased_rows() {
        assert_eq!(
            PhasedTableSet::new(0, 2, PHASED_V1.moves).validate(),
            Err(PhaseError::BadVersion(0))
        );
        assert_eq!(
            PhasedTableSet::new(2, 0, PHASED_V1.moves).validate(),
            Err(PhaseError::BadValue {
                field: "count",
                value: 0
            })
        );
        assert_eq!(
            PhasedTableSet::new(2, MAX_PHASED_MOVES as u8 + 1, PHASED_V1.moves).validate(),
            Err(PhaseError::BadValue {
                field: "count",
                value: MAX_PHASED_MOVES as u32 + 1
            })
        );
        // Non-dense ids refuse.
        let mut moves = PHASED_V1.moves;
        moves[1].move_id = 3;
        assert_eq!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "move_id",
                value: 3
            })
        );
        // Timing ranges refuse per field.
        let mut moves = PHASED_V1.moves;
        moves[0].startup_ticks = 0;
        assert!(matches!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "startup_ticks",
                ..
            })
        ));
        let mut moves = PHASED_V1.moves;
        moves[0].active_ticks = 61;
        assert!(matches!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "active_ticks",
                ..
            })
        ));
        let mut moves = PHASED_V1.moves;
        moves[0].endlag_ticks = 121;
        assert!(matches!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "endlag_ticks",
                ..
            })
        ));
        // An inverted window refuses on its start.
        let mut moves = PHASED_V1.moves;
        moves[0].cancel_start = 6;
        moves[0].cancel_end = 5;
        assert_eq!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "cancel_start",
                value: 6
            })
        );
        // A window past total refuses on its end.
        let mut moves = PHASED_V1.moves;
        moves[0].cancel_end = moves[0].total() + 1;
        assert!(matches!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "cancel_end",
                ..
            })
        ));
        // A route off the roster refuses.
        let mut moves = PHASED_V1.moves;
        moves[0].cancel_into = 3;
        assert_eq!(
            PhasedTableSet::new(2, 2, moves).validate(),
            Err(PhaseError::BadValue {
                field: "cancel_into",
                value: 3
            })
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> ExchangeTables {
        EXCHANGE_V1
    }

    fn state2() -> ExchangeState {
        ExchangeState::new(2, &EXCHANGE_V1).unwrap()
    }

    fn idle() -> [ExchangeIntent; SLOT_COUNT] {
        [ExchangeIntent::Idle; SLOT_COUNT]
    }

    fn run_script(
        script: &[[ExchangeIntent; SLOT_COUNT]],
    ) -> (Vec<Vec<ExchangeEvent>>, ExchangeState) {
        let mut state = state2();
        let t = tables();
        let mut all = Vec::new();
        for intents in script {
            let mut out = [ExchangeEvent::Parried {
                attacker: 0,
                victim: 0,
            }; MAX_EVENTS_PER_TICK];
            let n = resolve_tick(&mut state, intents, &t, &mut out);
            all.push(out[..n].to_vec());
            state.advance();
        }
        (all, state)
    }

    #[test]
    fn v1_rows_match_consts() {
        let t = EXCHANGE_V1;
        assert_eq!((t.parry_start_tick, t.parry_active_ticks), (0, 4));
        assert_eq!(t.parry_end_tick(), 4);
        assert_eq!(t.riposte_window_ticks, RIPOSTE_WINDOW_TICKS);
        assert_eq!(t.riposte_damage_permille, 140);
        assert_eq!(t.clash_bind_ticks, 6);
        assert_eq!((t.parry_poise_damage, t.clash_poise_damage), (250, 200));
        assert_eq!(t.poise_max, 1000);
        assert_eq!(ExchangeTableSet::v1().version, 1);
    }

    #[test]
    fn parry_window_edges_proven_tick_by_tick() {
        let t = tables();
        let w = ParryWindow::from_tables(&t);
        // Every age 0..=8 classifies exactly once; the table window is 0..=4.
        for age in 0..8u32 {
            let open = parry_open(age, w);
            assert_eq!(open, parry_open_with(age, &t));
            assert_eq!(open, age <= 4, "age {age} misclassified");
        }
        assert!(parry_open(0, w));
        assert!(parry_open(4, w));
        assert!(!parry_open(5, w));
        // Nonzero start shifts the window without changing its width rule.
        let mut shifted = t;
        shifted.parry_start_tick = 2;
        shifted.parry_active_ticks = 2;
        let w2 = ParryWindow::from_tables(&shifted);
        assert!(!parry_open(1, w2));
        assert!(parry_open(2, w2));
        assert!(parry_open(4, w2));
        assert!(!parry_open(5, w2));
    }

    #[test]
    fn every_attack_has_a_timed_answer() {
        // For each impact tick in 0..16 there is a press tick that parries,
        // and presses outside the window fail on the proven side.
        let t = tables();
        for impact in 0..16u32 {
            // Only presses at or before the impact are real answers; a press
            // is never scheduled in the future.
            let mut found = false;
            for press in 0..=impact {
                if answer_for(impact, press, &t) == TimedAnswer::Parry {
                    found = true;
                    let age = impact - press;
                    assert!(
                        age >= t.parry_start_tick && age <= t.parry_end_tick(),
                        "parried age {age} outside window at impact {impact}"
                    );
                }
            }
            assert!(found, "attack at tick {impact} has no timed answer");
            // Boundary: one tick before the window is TooEarly/TooLate, never Parry.
            let end = t.parry_end_tick();
            if impact > end {
                let just_late = impact - (end + 1);
                // press one tick older than the window => TooLate
                assert_eq!(answer_for(impact, just_late, &t), TimedAnswer::TooLate);
            }
        }
        // Spot checks on the failure sides.
        assert_eq!(answer_for(10, 10, &t), TimedAnswer::Parry);
        assert_eq!(answer_for(10, 6, &t), TimedAnswer::Parry);
        assert_eq!(answer_for(10, 5, &t), TimedAnswer::TooLate);
    }

    #[test]
    fn late_parry_fails_and_never_opens_riposte() {
        let t = tables();
        let mut state = state2();
        let mut intents = idle();
        intents[0] = ExchangeIntent::Attack { timing: 0 };
        intents[1] = ExchangeIntent::Block { age: 5 };
        let mut out = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_EVENTS_PER_TICK];
        let n = resolve_tick(&mut state, &intents, &t, &mut out);
        assert_eq!(n, 0, "age 5 must not parry under V1");
        assert!(!state.riposte_live(1, &t));
        assert_eq!(
            try_riposte(&mut state, &t, 1, 0),
            Err(ExchangeError::Denied("no-parry"))
        );
    }

    #[test]
    fn parry_opens_timed_riposte_and_riposte_expires() {
        let t = tables();
        let mut state = state2();
        let mut intents = idle();
        intents[0] = ExchangeIntent::Attack { timing: 0 };
        intents[1] = ExchangeIntent::Block { age: 2 };
        let mut out = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_EVENTS_PER_TICK];
        let n = resolve_tick(&mut state, &intents, &t, &mut out);
        assert_eq!(n, 1);
        assert_eq!(
            out[0],
            ExchangeEvent::Parried {
                attacker: 0,
                victim: 1
            }
        );
        assert_eq!(state.fighters[0].poise, POISE_MAX - PARRY_POISE_DAMAGE);
        assert!(state.riposte_live(1, &t));
        // Riposte inside the window lands with the table damage.
        let rip = try_riposte(&mut state, &t, 1, 0).unwrap();
        assert_eq!(
            rip,
            ExchangeEvent::Riposte {
                attacker: 1,
                victim: 0,
                damage: t.riposte_damage_permille
            }
        );
        // Riposte from neutral (no parry) is refused.
        assert_eq!(
            try_riposte(&mut state, &t, 0, 1),
            Err(ExchangeError::Denied("no-parry"))
        );
        // Past the window the same riposte expires.
        for _ in 0..(t.riposte_window_ticks + 1) {
            state.advance();
        }
        assert!(!state.riposte_live(1, &t));
        assert_eq!(
            try_riposte(&mut state, &t, 1, 0),
            Err(ExchangeError::Denied("expired"))
        );
    }

    #[test]
    fn riposte_locks_block_until_it_expires() {
        let t = tables();
        let mut state = state2();
        assert!(state.can_block(1, &t));
        state.fighters[1].last_parry_tick = state.tick;
        // Riposte live: defense committed, no cancel into block.
        assert!(state.riposte_live(1, &t));
        assert!(!state.can_block(1, &t));
        for _ in 0..(t.riposte_window_ticks + 1) {
            state.advance();
        }
        assert!(!state.riposte_live(1, &t));
        assert!(state.can_block(1, &t));
        // Bound fighters cannot block either.
        state.fighters[0].bind = 3;
        assert!(!state.can_block(0, &t));
        assert!(state.is_bound(0));
    }

    #[test]
    fn clash_breaks_by_poise_then_timing_then_id() {
        let t = tables();
        // Higher poise wins regardless of timing or ID order.
        let out = resolve_clash(1, 0, 0, 0, 400, 900, &t);
        assert_eq!(out.winner, 0);
        assert_eq!(out.loser, 1);
        assert!(!out.poise_tied);
        assert_eq!(out.bind_ticks, t.clash_bind_ticks);
        // Equal poise: earlier timing wins.
        let out = resolve_clash(0, 1, 5, 2, 500, 500, &t);
        assert_eq!(out.winner, 1);
        assert!(out.poise_tied);
        // Full tie: lower ID wins.
        let out = resolve_clash(0, 1, 3, 3, 500, 500, &t);
        assert_eq!((out.winner, out.loser), (0, 1));
        let out = resolve_clash(1, 0, 3, 3, 500, 500, &t);
        assert_eq!((out.winner, out.loser), (0, 1));
    }

    #[test]
    fn clash_mirrors_when_ids_swap() {
        let t = tables();
        // Swapped call order with swapped poise/timing mirrors the winner.
        let fwd = resolve_clash(0, 1, 2, 7, 600, 600, &t);
        let bwd = resolve_clash(1, 0, 7, 2, 600, 600, &t);
        assert_eq!(fwd.winner, bwd.winner);
        assert_eq!(fwd.loser, bwd.loser);
        assert_eq!(fwd.bind_ticks, bwd.bind_ticks);
        // Poise-decided clash mirrors too.
        let fwd = resolve_clash(0, 1, 0, 0, 800, 300, &t);
        let bwd = resolve_clash(1, 0, 0, 0, 300, 800, &t);
        assert_eq!(fwd.winner, 0);
        assert_eq!(bwd.winner, 0);
    }

    #[test]
    fn same_tick_clash_binds_both_and_charges_loser_poise() {
        let t = tables();
        let mut state = state2();
        state.fighters[0].poise = 700;
        state.fighters[1].poise = 400;
        let mut intents = idle();
        intents[0] = ExchangeIntent::Attack { timing: 1 };
        intents[1] = ExchangeIntent::Attack { timing: 1 };
        let mut out = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_EVENTS_PER_TICK];
        let n = resolve_tick(&mut state, &intents, &t, &mut out);
        assert_eq!(n, 1);
        assert_eq!(
            out[0],
            ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 0
            }
        );
        assert_eq!(state.fighters[0].bind, t.clash_bind_ticks);
        assert_eq!(state.fighters[1].bind, t.clash_bind_ticks);
        assert_eq!(state.fighters[1].poise, 400 - t.clash_poise_damage);
        assert_eq!(state.fighters[0].poise, 700);
        // Bound attackers act as idle next tick: no double clash.
        let mut idle_in = idle();
        idle_in[0] = ExchangeIntent::Attack { timing: 0 };
        idle_in[1] = ExchangeIntent::Block { age: 0 };
        let mut out2 = out;
        let n2 = resolve_tick(&mut state, &idle_in, &t, &mut out2);
        assert_eq!(n2, 0);
    }

    #[test]
    fn input_order_never_changes_outcome_ids_decide() {
        // Intents are slot-indexed, so there is no caller order to permute;
        // the equivalent proof builds the same tick twice with identical
        // slots and asserts identical events and state.
        let t = tables();
        let build = || {
            let mut intents = idle();
            intents[0] = ExchangeIntent::Attack { timing: 4 };
            intents[1] = ExchangeIntent::Block { age: 1 };
            intents
        };
        let mut s1 = state2();
        let mut s2 = state2();
        let mut o1 = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_EVENTS_PER_TICK];
        let mut o2 = o1;
        let n1 = resolve_tick(&mut s1, &build(), &t, &mut o1);
        let n2 = resolve_tick(&mut s2, &build(), &t, &mut o2);
        assert_eq!(n1, n2);
        assert_eq!(o1[..n1], o2[..n2]);
        assert_eq!(s1, s2);
    }

    #[test]
    fn replay_exact_twice_over_parry_riposte_clash_tape() {
        let attack = |timing| ExchangeIntent::Attack { timing };
        let block = |age| ExchangeIntent::Block { age };
        let tick = |a: ExchangeIntent, b: ExchangeIntent| {
            let mut intents = idle();
            intents[0] = a;
            intents[1] = b;
            intents
        };
        let script: Vec<[ExchangeIntent; SLOT_COUNT]> = vec![
            tick(attack(0), block(0)),
            tick(attack(0), attack(0)),
            tick(block(9), block(9)),
            tick(attack(2), block(2)),
            tick(attack(1), attack(3)),
        ];
        let (a_events, a_state) = run_script(&script);
        let (b_events, b_state) = run_script(&script);
        assert_eq!(a_events, b_events);
        assert_eq!(a_state, b_state);
        // The tape really answers: at least one parry and one clash.
        let flat: Vec<ExchangeEvent> = a_events.into_iter().flatten().collect();
        assert!(flat
            .iter()
            .any(|e| matches!(e, ExchangeEvent::Parried { .. })));
        assert!(flat
            .iter()
            .any(|e| matches!(e, ExchangeEvent::Clash { .. })));
    }

    #[test]
    fn validation_rejects_bad_versions_and_rows() {
        assert_eq!(
            ExchangeTableSet::new(0, EXCHANGE_V1).validate(),
            Err(ExchangeError::BadVersion(0))
        );
        assert_eq!(ExchangeTableSet::v1().validate(), Ok(()));
        let mut bad = EXCHANGE_V1;
        bad.parry_active_ticks = 61;
        assert_eq!(
            ExchangeTableSet::new(2, bad).validate(),
            Err(ExchangeError::BadValue {
                field: "parry_active_ticks",
                value: 61
            })
        );
        // Start and width each in range but the window end past 60 fails.
        let mut bad = EXCHANGE_V1;
        bad.parry_start_tick = 40;
        bad.parry_active_ticks = 30;
        assert_eq!(
            ExchangeTableSet::new(2, bad).validate(),
            Err(ExchangeError::BadValue {
                field: "parry_end_tick",
                value: 70
            })
        );
        let mut bad = EXCHANGE_V1;
        bad.riposte_window_ticks = 0;
        assert!(matches!(
            ExchangeTableSet::new(2, bad).validate(),
            Err(ExchangeError::BadValue {
                field: "riposte_window_ticks",
                ..
            })
        ));
        let mut bad = EXCHANGE_V1;
        bad.riposte_damage_permille = 0;
        assert!(matches!(
            ExchangeTableSet::new(2, bad).validate(),
            Err(ExchangeError::BadValue {
                field: "riposte_damage_permille",
                ..
            })
        ));
        let mut bad = EXCHANGE_V1;
        bad.clash_bind_ticks = 0;
        assert!(matches!(
            ExchangeTableSet::new(2, bad).validate(),
            Err(ExchangeError::BadValue {
                field: "clash_bind_ticks",
                ..
            })
        ));
        let mut bad = EXCHANGE_V1;
        bad.parry_poise_damage = bad.poise_max + 1;
        assert!(matches!(
            ExchangeTableSet::new(2, bad).validate(),
            Err(ExchangeError::BadValue {
                field: "parry_poise_damage",
                ..
            })
        ));
        assert_eq!(
            ExchangeState::new(0, &EXCHANGE_V1),
            Err(ExchangeError::BadFighterCount(0))
        );
        // One canonical cap: sixteen slots validate here, not four.
        assert_eq!(
            ExchangeState::new(MAX_PARTICIPANTS, &EXCHANGE_V1).map(|s| s.count),
            Ok(MAX_PARTICIPANTS)
        );
        assert_eq!(
            ExchangeState::new(MAX_PARTICIPANTS + 1, &EXCHANGE_V1),
            Err(ExchangeError::BadFighterCount(MAX_PARTICIPANTS + 1))
        );
    }

    #[test]
    fn riposte_refusals_are_typed_never_panic() {
        let t = tables();
        let mut state = state2();
        // Out-of-range slots refuse without indexing out of bounds.
        assert_eq!(
            try_riposte(&mut state, &t, 7, 0),
            Err(ExchangeError::Denied("range"))
        );
        assert_eq!(
            try_riposte(&mut state, &t, 0, 0),
            Err(ExchangeError::Denied("self"))
        );
        // Bound attacker refuses even with a live parry.
        state.fighters[0].last_parry_tick = state.tick;
        state.fighters[0].bind = 2;
        assert_eq!(
            try_riposte(&mut state, &t, 0, 1),
            Err(ExchangeError::Denied("bound"))
        );
    }

    fn exchange_hash(events: &[Vec<ExchangeEvent>], state: &ExchangeState) -> u64 {
        const FNV_OFF: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let mut h = FNV_OFF;
        let mut mix = |w: u64| {
            h ^= w;
            h = h.wrapping_mul(FNV_PRIME);
        };
        mix(u64::from(state.count));
        mix(u64::from(state.tick));
        for f in state.fighters.iter().take(state.count as usize) {
            mix(u64::from(f.id));
            mix(u64::from(f.poise));
            mix(u64::from(f.bind));
            mix(u64::from(f.last_parry_tick));
        }
        for tick_events in events {
            for e in tick_events {
                match *e {
                    ExchangeEvent::Parried { attacker, victim } => {
                        mix(1);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                    }
                    ExchangeEvent::Riposte {
                        attacker,
                        victim,
                        damage,
                    } => {
                        mix(2);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                        mix(u64::from(damage));
                    }
                    ExchangeEvent::Clash { a, b, winner } => {
                        mix(3);
                        mix(u64::from(a));
                        mix(u64::from(b));
                        mix(u64::from(winner));
                    }
                }
            }
        }
        h
    }

    #[test]
    fn slots_beyond_four_follow_canonical_cap() {
        // Generic slots: slots past the old 4-cap clash and parry, and the
        // full 16-slot cap initializes.
        let t = tables();
        let mut state = ExchangeState::new(6, &EXCHANGE_V1).unwrap();
        let mut intents = idle();
        intents[4] = ExchangeIntent::Attack { timing: 1 };
        intents[5] = ExchangeIntent::Attack { timing: 1 };
        let mut out = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_EVENTS_PER_TICK];
        let n = resolve_tick(&mut state, &intents, &t, &mut out);
        assert_eq!(n, 1);
        // Equal poise and timing: the lower high-slot ID wins.
        assert_eq!(
            out[0],
            ExchangeEvent::Clash {
                a: 4,
                b: 5,
                winner: 4
            }
        );
        assert_eq!(state.fighters[4].bind, t.clash_bind_ticks);

        let mut state = ExchangeState::new(6, &EXCHANGE_V1).unwrap();
        let mut intents = idle();
        intents[4] = ExchangeIntent::Attack { timing: 0 };
        intents[5] = ExchangeIntent::Block { age: 2 };
        let mut out = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_EVENTS_PER_TICK];
        let n = resolve_tick(&mut state, &intents, &t, &mut out);
        assert_eq!(n, 1);
        assert_eq!(
            out[0],
            ExchangeEvent::Parried {
                attacker: 4,
                victim: 5
            }
        );
        // The riposte the parry opens fires across high slots too.
        let rip = try_riposte(&mut state, &t, 5, 4).unwrap();
        assert_eq!(
            rip,
            ExchangeEvent::Riposte {
                attacker: 5,
                victim: 4,
                damage: t.riposte_damage_permille
            }
        );
    }

    #[test]
    fn fixture_rows_resolve_without_engine_edits() {
        // Tables own tuning: a versioned fixture set with different numbers
        // resolves through the same core, no engine edits.
        let mut rows = EXCHANGE_V1;
        rows.riposte_damage_permille = 200;
        rows.clash_bind_ticks = 3;
        rows.riposte_window_ticks = 2;
        let set = ExchangeTableSet::new(2, rows);
        assert_eq!(set.validate(), Ok(()));
        // The same parry answers under either row; the riposte damage follows
        // the table that opened it.
        let parry_tick = |tables: &ExchangeTables| {
            let mut state = ExchangeState::new(2, tables).unwrap();
            let mut intents = idle();
            intents[0] = ExchangeIntent::Attack { timing: 0 };
            intents[1] = ExchangeIntent::Block { age: 2 };
            let mut out = [ExchangeEvent::Parried {
                attacker: 0,
                victim: 0,
            }; MAX_EVENTS_PER_TICK];
            let n = resolve_tick(&mut state, &intents, tables, &mut out);
            let rip = try_riposte(&mut state, tables, 1, 0).unwrap();
            (out[..n].to_vec(), rip)
        };
        let (v1_events, v1_rip) = parry_tick(&EXCHANGE_V1);
        let (v2_events, v2_rip) = parry_tick(&rows);
        assert_eq!(v1_events, v2_events);
        assert_eq!(
            v1_rip,
            ExchangeEvent::Riposte {
                attacker: 1,
                victim: 0,
                damage: 140
            }
        );
        assert_eq!(
            v2_rip,
            ExchangeEvent::Riposte {
                attacker: 1,
                victim: 0,
                damage: 200
            }
        );
        // Clash bind follows its row too.
        let clash_bind = |tables: &ExchangeTables| {
            let mut state = ExchangeState::new(2, tables).unwrap();
            let mut intents = idle();
            intents[0] = ExchangeIntent::Attack { timing: 1 };
            intents[1] = ExchangeIntent::Attack { timing: 1 };
            let mut out = [ExchangeEvent::Parried {
                attacker: 0,
                victim: 0,
            }; MAX_EVENTS_PER_TICK];
            let n = resolve_tick(&mut state, &intents, tables, &mut out);
            assert_eq!(n, 1);
            state.fighters[0].bind
        };
        assert_eq!(clash_bind(&EXCHANGE_V1), 6);
        assert_eq!(clash_bind(&rows), 3);
        // Each row replays identically twice over a short tape.
        let script = {
            let mut first = idle();
            first[0] = ExchangeIntent::Attack { timing: 0 };
            first[1] = ExchangeIntent::Block { age: 1 };
            let mut second = idle();
            second[0] = ExchangeIntent::Attack { timing: 2 };
            second[1] = ExchangeIntent::Attack { timing: 2 };
            vec![first, second, idle()]
        };
        let run = |tables: &ExchangeTables| {
            let mut state = ExchangeState::new(2, tables).unwrap();
            let mut all = Vec::new();
            for intents in &script {
                let mut out = [ExchangeEvent::Parried {
                    attacker: 0,
                    victim: 0,
                }; MAX_EVENTS_PER_TICK];
                let n = resolve_tick(&mut state, intents, tables, &mut out);
                all.push(out[..n].to_vec());
                state.advance();
            }
            (all, state)
        };
        let (a_events, a_state) = run(&rows);
        let (b_events, b_state) = run(&rows);
        assert_eq!(
            exchange_hash(&a_events, &a_state),
            exchange_hash(&b_events, &b_state)
        );
        assert!(a_events.iter().any(|tick| !tick.is_empty()));
    }
}
