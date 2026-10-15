//! arena: match-step wiring over the exchange and sweep resolvers.
//! Owner: engine-combat, consumer: arena match step (this module is the
//!   production call-site shape; engine-arena wires it next, resolvers frozen).
//! Units: ticks at 60 Hz, sweep lengths in mm, poise/damage in permille.
//! Determinism: integer math, fighter-ID order; same tables+inputs => same bytes.
//! Errors: MatchError for bad versions/counts; the step never panics, never allocs.
//! Budget: O(fighters^2 + sweeps*fighters) over <=16 slots and <=8 sweeps,
//!   no heap per tick (every event buffer is caller-owned).

use crate::exchange::{
    validate_exchange, ExchangeError, ExchangeEvent, ExchangeIntent, ExchangeState, ExchangeTables,
    EXCHANGE_V1, MAX_EVENTS_PER_TICK as EXCHANGE_MAX_EVENTS,
};
use crate::fight::SLOT_COUNT;
use crate::sweep::{
    validate_sweep_tables, SweepError, SweepEvent, SweepParticipant, SweepTables, WeaponSweep,
    MAX_SWEEP_EVENTS_PER_TICK as SWEEP_MAX_EVENTS, SWEEP_V1,
};

/// Version tag of the shipped match rows (exchange V1 + sweep V1 together).
pub const MATCH_VERSION_V1: u32 = 1;

/// Tunable match rows: the two landed sets side by side, never merged.
/// Reuses [`ExchangeTables`] and [`SweepTables`] directly (no mirrored rows).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MatchTables {
    /// Timed parry/riposte/clash rows.
    pub exchange: ExchangeTables,
    /// Continuous sweep rows.
    pub sweep: SweepTables,
}

impl MatchTables {
    /// The shipped exchange and sweep rows.
    pub const fn v1() -> Self {
        Self {
            exchange: EXCHANGE_V1,
            sweep: SWEEP_V1,
        }
    }
}

/// The shipped match rows as one static row.
pub const MATCH_V1: MatchTables = MatchTables::v1();

/// Versioned match rows: nonzero version plus both validated sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MatchTableSet {
    /// Table version; nonzero, 1 for the shipped rows.
    pub version: u32,
    /// The tunable rows resolved under this version.
    pub tables: MatchTables,
}

impl MatchTableSet {
    /// The shipped rows as version 1.
    pub const fn v1() -> Self {
        Self {
            version: MATCH_VERSION_V1,
            tables: MATCH_V1,
        }
    }

    /// Explicit rows with a nonzero version; call [`validate_match`] first.
    pub const fn new(version: u32, tables: MatchTables) -> Self {
        Self { version, tables }
    }

    /// Range-check this set; see [`validate_match`].
    pub fn validate(&self) -> Result<(), MatchError> {
        validate_match(self)
    }
}

/// Bounded match failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchError {
    /// Version 0 is reserved for legacy untagged replays, never valid anew.
    BadVersion(u32),
    /// Participant count outside `1..=MAX_PARTICIPANTS`, culprit carried.
    BadParticipantCount(u8),
    /// The exchange half of the set failed validation.
    Exchange(ExchangeError),
    /// The sweep half of the set failed validation.
    Sweep(SweepError),
}

/// Range-check a versioned match set. First failure wins, deterministically:
/// version, then the exchange rows, then the sweep rows.
pub fn validate_match(set: &MatchTableSet) -> Result<(), MatchError> {
    if set.version == 0 {
        return Err(MatchError::BadVersion(0));
    }
    validate_exchange(&crate::exchange::ExchangeTableSet::new(
        set.version,
        set.tables.exchange,
    ))
    .map_err(MatchError::Exchange)?;
    validate_sweep_tables(&crate::sweep::SweepTableSet::new(
        set.version,
        set.tables.sweep,
    ))
    .map_err(MatchError::Sweep)?;
    Ok(())
}

/// Whole-match exchange track for up to [`MAX_PARTICIPANTS`] fighters.
/// Sweep is stateless per tick (poses in, events out), so the match owns
/// only the exchange state; the tick is the exchange clock, never a copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MatchState {
    /// Parry/riposte/clash track; tick and bind clocks live here.
    pub exchange: ExchangeState,
}

impl MatchState {
    /// Fresh match with `count` fighters at full poise.
    pub fn new(count: u8, tables: &MatchTables) -> Result<Self, MatchError> {
        ExchangeState::new(count, &tables.exchange)
            .map(|exchange| Self { exchange })
            .map_err(|_| MatchError::BadParticipantCount(count))
    }

    /// Exchange ticks elapsed.
    pub const fn tick(self) -> u32 {
        self.exchange.tick
    }

    /// Advance one match tick: decay binds, run the clock. Saturating.
    pub fn advance(&mut self) {
        self.exchange.advance();
    }
}

/// Event counts from one [`match_step`]: positions into the caller buffers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MatchCounts {
    /// Exchange events written to the exchange buffer.
    pub exchange_n: usize,
    /// Sweep events written to the sweep buffer.
    pub sweep_n: usize,
}

/// Resolve one match tick through both landed resolvers into caller buffers.
///
/// Order per tick: exchange answers first (parry/clash over slot intents in
/// fighter-ID order), then sweep contacts (segment-vs-capsule in attacker-ID
/// order). The two halves read the same tick independently: exchange binds
/// never suppress sweep steel and sweep hits never rewrite exchange poise;
/// the arena applies damage after this call. Writes at most `out.len()`
/// events per half and returns both counts. No heap, no floats, no panics.
pub fn match_step(
    state: &mut MatchState,
    exchange_intents: &[ExchangeIntent; SLOT_COUNT],
    sweep_fighters: &[SweepParticipant],
    sweeps: &[WeaponSweep],
    tables: &MatchTables,
    exchange_out: &mut [ExchangeEvent; EXCHANGE_MAX_EVENTS],
    sweep_out: &mut [SweepEvent; SWEEP_MAX_EVENTS],
) -> MatchCounts {
    let exchange_n = crate::exchange::resolve_tick(
        &mut state.exchange,
        exchange_intents,
        &tables.exchange,
        exchange_out,
    );
    let sweep_n = crate::sweep::resolve_tick(sweep_fighters, sweeps, &tables.sweep, sweep_out);
    MatchCounts {
        exchange_n,
        sweep_n,
    }
}

/// Attempt the locked riposte through the match state: succeeds only inside
/// the parry window this fighter opened. Refusals carry the static
/// [`ExchangeError::Denied`] tags, never allocate, never panic.
pub fn match_riposte(
    state: &mut MatchState,
    tables: &MatchTables,
    attacker: u8,
    victim: u8,
) -> Result<ExchangeEvent, ExchangeError> {
    crate::exchange::try_riposte(&mut state.exchange, &tables.exchange, attacker, victim)
}

/// Append canonical bytes for one tick plus state: exchange events, sweep
/// events, then the exchange track. Tags keep the halves apart
/// (exchange 10/11/12, sweep 20/21); integers only, fighter-ID order in.
pub fn canonical_bytes(
    state: &MatchState,
    exchange_events: &[ExchangeEvent],
    sweep_events: &[SweepEvent],
    out: &mut Vec<u8>,
) {
    for e in exchange_events {
        match *e {
            ExchangeEvent::Parried { attacker, victim } => {
                out.extend_from_slice(&[10, attacker, victim]);
            }
            ExchangeEvent::Riposte {
                attacker,
                victim,
                damage,
            } => {
                out.extend_from_slice(&[11, attacker, victim]);
                out.extend_from_slice(&damage.to_le_bytes());
            }
            ExchangeEvent::Clash { a, b, winner } => {
                out.extend_from_slice(&[12, a, b, winner]);
            }
        }
    }
    for e in sweep_events {
        match *e {
            SweepEvent::Hit { attacker, victim } => {
                out.extend_from_slice(&[20, attacker, victim]);
            }
            SweepEvent::Blocked { attacker, victim } => {
                out.extend_from_slice(&[21, attacker, victim]);
            }
        }
    }
    out.extend_from_slice(&[30, state.exchange.count]);
    out.extend_from_slice(&state.exchange.tick.to_le_bytes());
    let mut i: usize = 0;
    while i < usize::from(state.exchange.count) {
        let f = state.exchange.fighters[i];
        out.extend_from_slice(&[31, f.id]);
        out.extend_from_slice(&f.poise.to_le_bytes());
        out.extend_from_slice(&f.bind.to_le_bytes());
        out.extend_from_slice(&f.last_parry_tick.to_le_bytes());
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exchange::{answer_for, resolve_clash, ExchangeIntent as Intent, TimedAnswer};
    use engine_input::MAX_PARTICIPANTS;

    fn tables() -> MatchTables {
        MATCH_V1
    }

    fn idle() -> [Intent; SLOT_COUNT] {
        [Intent::Idle; SLOT_COUNT]
    }

    fn ex_out() -> [ExchangeEvent; EXCHANGE_MAX_EVENTS] {
        [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; EXCHANGE_MAX_EVENTS]
    }

    fn sw_out() -> [SweepEvent; SWEEP_MAX_EVENTS] {
        [SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        }; SWEEP_MAX_EVENTS]
    }

    fn pose(id: u8, x: i32, z: i32, fx: i32, fz: i32, shield_up: bool) -> SweepParticipant {
        SweepParticipant {
            id,
            pos_mm: [x, 0, z],
            facing_x: fx,
            facing_z: fz,
            hurt_half_height_mm: 800,
            hurt_radius_mm: 150,
            shield_half_deg: 60,
            shield_up,
        }
    }

    fn duel() -> [SweepParticipant; 2] {
        [pose(0, -1000, 0, 1, 0, false), pose(1, 0, 0, -1, 0, false)]
    }

    fn blade(attacker: u8, p0: [i32; 3], p1: [i32; 3]) -> WeaponSweep {
        WeaponSweep {
            attacker,
            p0_mm: p0,
            p1_mm: p1,
            blade_half_mm: 10,
        }
    }

    fn hit_blade() -> WeaponSweep {
        blade(0, [-100, 0, 0], [100, 0, 0])
    }

    fn miss_blade() -> WeaponSweep {
        blade(0, [-100, 0, 1000], [100, 0, 1000])
    }

    /// One scripted tick through the match step; returns the canonical bytes.
    fn step_bytes(
        state: &mut MatchState,
        intents: &[Intent; SLOT_COUNT],
        fighters: &[SweepParticipant],
        sweeps: &[WeaponSweep],
        tables: &MatchTables,
    ) -> Vec<u8> {
        let mut eo = ex_out();
        let mut so = sw_out();
        let counts = match_step(state, intents, fighters, sweeps, tables, &mut eo, &mut so);
        let mut bytes = Vec::new();
        canonical_bytes(
            state,
            &eo[..counts.exchange_n],
            &so[..counts.sweep_n],
            &mut bytes,
        );
        state.advance();
        // Riposte window check rides the same tick when a parry opened one.
        let mut slot: u8 = 0;
        while slot < state.exchange.count {
            if state.exchange.riposte_live(slot, &tables.exchange) {
                let victim = u8::from(slot == 0);
                if let Ok(ExchangeEvent::Riposte { damage, .. }) =
                    match_riposte(state, tables, slot, victim)
                {
                    bytes.extend_from_slice(&[40, slot, victim]);
                    bytes.extend_from_slice(&damage.to_le_bytes());
                    break;
                }
            }
            slot += 1;
        }
        bytes
    }

    fn exchange_tick(attack_timing: u32, block_age: Option<u32>) -> [Intent; SLOT_COUNT] {
        let mut intents = idle();
        intents[0] = Intent::Attack {
            timing: attack_timing,
        };
        if let Some(age) = block_age {
            intents[1] = Intent::Block { age };
        }
        intents
    }

    #[test]
    fn version_rows_validate_and_resolve() {
        assert_eq!(MatchTableSet::v1().validate(), Ok(()));
        assert_eq!(
            MatchTableSet::new(0, MATCH_V1).validate(),
            Err(MatchError::BadVersion(0))
        );
        let mut bad = MATCH_V1;
        bad.exchange.parry_active_ticks = 61;
        assert!(matches!(
            MatchTableSet::new(2, bad).validate(),
            Err(MatchError::Exchange(_))
        ));
        let mut bad = MATCH_V1;
        bad.sweep.contact_slack_mm = 17;
        assert!(matches!(
            MatchTableSet::new(2, bad).validate(),
            Err(MatchError::Sweep(_))
        ));
        assert_eq!(
            MatchState::new(0, &MATCH_V1),
            Err(MatchError::BadParticipantCount(0))
        );
        assert_eq!(
            MatchState::new(MAX_PARTICIPANTS + 1, &MATCH_V1),
            Err(MatchError::BadParticipantCount(MAX_PARTICIPANTS + 1))
        );
        // Every attack still has a timed answer when routed via the match tables.
        let t = &MATCH_V1.exchange;
        for impact in 0..16u32 {
            let mut found = false;
            for press in 0..=impact {
                if answer_for(impact, press, t) == TimedAnswer::Parry {
                    found = true;
                }
            }
            assert!(found, "attack at tick {impact} has no timed answer");
        }
    }

    #[test]
    fn parry_tape_resolves_identical_bytes_twice() {
        let t = tables();
        let fighters = duel();
        // Tick plan: parry opens (attack vs in-window block) with steel on
        // target, then a reach miss, then a second parry + riposte check.
        let tape: Vec<([Intent; SLOT_COUNT], Vec<WeaponSweep>)> = vec![
            (exchange_tick(0, Some(1)), vec![hit_blade()]),
            (exchange_tick(0, Some(9)), vec![miss_blade()]),
            (exchange_tick(2, Some(2)), vec![hit_blade()]),
            (idle(), vec![]),
        ];
        let run = || {
            let mut state = MatchState::new(2, &t).unwrap();
            let mut all = Vec::new();
            for (intents, sweeps) in &tape {
                all.push(step_bytes(&mut state, intents, &fighters, sweeps, &t));
            }
            (all, state)
        };
        let (a_bytes, a_state) = run();
        let (b_bytes, b_state) = run();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a_state, b_state);
        // The tape really answers: a parry byte (10) and a riposte byte (40)
        // plus a sweep hit byte (20) all appear.
        let flat: Vec<u8> = a_bytes.concat();
        assert!(flat.contains(&10), "no parry on the tape");
        assert!(flat.contains(&40), "no riposte window on the tape");
        assert!(flat.contains(&20), "no sweep hit on the tape");
    }

    #[test]
    fn clash_tape_mirrors_and_replays() {
        let t = tables();
        let fighters = duel();
        // Same-tick clash through the match step binds both and names the
        // higher-poise winner; swapping the poise mirrors the winner.
        let clash_tick = || {
            let mut intents = idle();
            intents[0] = Intent::Attack { timing: 1 };
            intents[1] = Intent::Attack { timing: 1 };
            intents
        };
        let run_clash = |poise0: u16, poise1: u16| {
            let mut state = MatchState::new(2, &t).unwrap();
            state.exchange.fighters[0].poise = poise0;
            state.exchange.fighters[1].poise = poise1;
            let mut eo = ex_out();
            let mut so = sw_out();
            let counts = match_step(
                &mut state,
                &clash_tick(),
                &fighters,
                &[],
                &t,
                &mut eo,
                &mut so,
            );
            (eo[..counts.exchange_n].to_vec(), state)
        };
        let (fwd_events, fwd_state) = run_clash(700, 400);
        assert_eq!(
            fwd_events,
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 0
            }]
        );
        assert_eq!(
            fwd_state.exchange.fighters[1].poise,
            400 - t.exchange.clash_poise_damage
        );
        let (bwd_events, _) = run_clash(400, 700);
        assert_eq!(
            bwd_events,
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 1
            }]
        );
        // Timing break mirrors too: earlier timing wins at equal poise, and
        // the step agrees with the bare resolver on the same inputs.
        let timed = |ta: u32, tb: u32| {
            let mut intents = idle();
            intents[0] = Intent::Attack { timing: ta };
            intents[1] = Intent::Attack { timing: tb };
            let mut state = MatchState::new(2, &t).unwrap();
            let mut eo = ex_out();
            let mut so = sw_out();
            let counts = match_step(&mut state, &intents, &fighters, &[], &t, &mut eo, &mut so);
            eo[..counts.exchange_n].to_vec()
        };
        assert_eq!(
            timed(2, 7),
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 0
            }]
        );
        assert_eq!(
            timed(7, 2),
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 1
            }]
        );
        let direct = resolve_clash(0, 1, 2, 7, 600, 600, &t.exchange);
        assert_eq!(direct.winner, 0);
        // The clash tape replays byte-identical twice.
        let tape = vec![clash_tick(), idle(), clash_tick()];
        let run = || {
            let mut state = MatchState::new(2, &t).unwrap();
            let mut all = Vec::new();
            for intents in &tape {
                all.push(step_bytes(&mut state, intents, &fighters, &[], &t));
            }
            (all, state)
        };
        let (a_bytes, a_state) = run();
        let (b_bytes, b_state) = run();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a_state, b_state);
        assert!(a_bytes.concat().contains(&12), "no clash byte on the tape");
    }

    #[test]
    fn sweep_tape_resolves_identical_bytes_twice() {
        let t = tables();
        let fighters = duel();
        let tape: Vec<([Intent; SLOT_COUNT], Vec<WeaponSweep>)> = vec![
            (idle(), vec![hit_blade()]),
            (idle(), vec![miss_blade()]),
            (
                idle(),
                vec![hit_blade(), blade(1, [-1100, 0, 0], [-900, 0, 0])],
            ),
            (idle(), vec![]),
        ];
        let run = || {
            let mut state = MatchState::new(2, &t).unwrap();
            let mut all = Vec::new();
            for (intents, sweeps) in &tape {
                all.push(step_bytes(&mut state, intents, &fighters, sweeps, &t));
            }
            (all, state)
        };
        let (a_bytes, a_state) = run();
        let (b_bytes, b_state) = run();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a_state, b_state);
        assert_eq!(
            a_bytes[0], b_bytes[0],
            "first sweep tick must hit identically"
        );
        // Hit byte on tick 0, pure state bytes on the reach-miss tick.
        assert!(a_bytes[0].contains(&20));
        assert!(!a_bytes[1].contains(&20) && !a_bytes[1].contains(&21));
        // Shield up on the defender turns the same steel into a block byte.
        let mut guard = duel();
        guard[1].shield_up = true;
        let mut state = MatchState::new(2, &t).unwrap();
        let mut eo = ex_out();
        let mut so = sw_out();
        let counts = match_step(
            &mut state,
            &idle(),
            &guard,
            &[hit_blade()],
            &t,
            &mut eo,
            &mut so,
        );
        assert_eq!(counts.sweep_n, 1);
        assert_eq!(
            so[0],
            SweepEvent::Blocked {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn riposte_without_parry_is_denied_never_panics() {
        let t = tables();
        let mut state = MatchState::new(2, &t).unwrap();
        assert_eq!(
            match_riposte(&mut state, &t, 1, 0),
            Err(ExchangeError::Denied("no-parry"))
        );
        assert_eq!(
            match_riposte(&mut state, &t, 0, 0),
            Err(ExchangeError::Denied("self"))
        );
        assert_eq!(
            match_riposte(&mut state, &t, 7, 0),
            Err(ExchangeError::Denied("range"))
        );
        // Bound attacker refuses even with a live parry.
        state.exchange.fighters[0].last_parry_tick = state.exchange.tick;
        state.exchange.fighters[0].bind = 2;
        assert_eq!(
            match_riposte(&mut state, &t, 0, 1),
            Err(ExchangeError::Denied("bound"))
        );
    }

    #[test]
    fn match_step_allocates_nothing_per_tick() {
        let t = tables();
        let fighters = duel();
        let sweeps = [hit_blade(), miss_blade()];
        let mut intents = idle();
        intents[0] = Intent::Attack { timing: 1 };
        intents[1] = Intent::Block { age: 1 };
        // Buffers reserved once outside the gate; the warm tick settles state.
        let mut state = MatchState::new(2, &t).unwrap();
        let mut eo = ex_out();
        let mut so = sw_out();
        match_step(
            &mut state, &intents, &fighters, &sweeps, &t, &mut eo, &mut so,
        );
        state.advance();
        let (counts, heap) = engine_core::alloc_probe::measure(|| {
            match_step(
                &mut state, &intents, &fighters, &sweeps, &t, &mut eo, &mut so,
            )
        });
        assert!(counts.exchange_n <= eo.len());
        assert!(counts.sweep_n <= so.len());
        assert_eq!(
            heap.calls, 0,
            "match step allocated {} times per tick",
            heap.calls
        );
        // A clash tick plus a riposte attempt also stays allocation-free.
        let mut clash = idle();
        clash[0] = Intent::Attack { timing: 0 };
        clash[1] = Intent::Attack { timing: 0 };
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            let c = match_step(&mut state, &clash, &fighters, &sweeps, &t, &mut eo, &mut so);
            let _ = (c.exchange_n, c.sweep_n);
            let _ = match_riposte(&mut state, &t, 1, 0);
        });
        assert_eq!(
            heap.calls, 0,
            "clash/riposte path allocated {} times",
            heap.calls
        );
    }
}
