//! match_wire: arena call-site over the landed GP1/GP2 resolvers.
//! Owner: engine-arena, consumer: Sim::step plus run_match_events.
//! Units: arena meters in, sweep millimetres out, ticks at 60 Hz.
//! Determinism: integer math, fighter-ID order; same inputs => same bytes.
//! Errors: none returned; bad counts refuse construction, resolvers never panic.
//! Budget: O(fighters^2) over <=32 slots; no heap per tick (caller buffers).

use engine_combat::arena::{
    canonical_bytes, match_riposte, match_step, MatchState, MatchTables, MATCH_V1,
};
use engine_combat::exchange::{
    phased_row, ExchangeError, ExchangeEvent, ExchangeIntent, MovePhase, PhasedFighter,
    EXCHANGE_V2, MAX_EVENTS_PER_TICK as EXCHANGE_CAP, NO_MOVE, PHASED_V1,
};
use engine_combat::sweep::{
    sweep_live_for, SweepEvent, SweepFighter, WeaponSweep, MAX_SWEEPS_PER_TICK as SWEEP_IN_CAP,
    MAX_SWEEP_EVENTS_PER_TICK as SWEEP_CAP,
};
use engine_input::fighter::{ATTACK, BLOCK};
use engine_input::{FighterInput, MAX_PARTICIPANTS};

use crate::FighterState;

/// Hurt-capsule half-height for every arena pose, in mm. One owner: the
/// sweep rows built here and the game's debug draw both read this value.
pub const HURT_HALF_MM: u32 = 800;
/// Hurt-capsule radius for every arena pose, in mm. One owner: the sweep
/// rows built here and the game's debug draw both read this value.
pub const HURT_RADIUS_MM: u32 = 150;
/// Shield-cone half-angle for every arena pose, in degrees.
pub(crate) const SHIELD_HALF_DEG: u8 = 60;
/// Blade half-thickness for every arena sweep, in mm.
pub(crate) const BLADE_HALF_MM: u32 = 10;
/// Blade reach ahead of the attacker, in mm (inside the 6000 mm travel bound).
pub(crate) const BLADE_REACH_MM: i32 = 1500;
/// Blade start ahead of the attacker, in mm.
pub(crate) const BLADE_BASE_MM: i32 = 200;

/// Intake path behind the version flag: every press reaches the resolvers.
pub const MATCH_WIRE_VERSION_V0: u32 = 0;
/// Intake path behind the version flag: phased rows gate presses and steel
/// (attack intakes forward only on live ticks with the age as timing, sweeps
/// only while [`sweep_live_for`] holds, cancels ride via `try_cancel`).
pub const MATCH_WIRE_VERSION_V1: u32 = 1;
/// Intake path behind the version flag: the phased intake of V1 with the
/// rules-v2 exchange rows, so a clash throws both blades back for
/// [`CLASH_RECOIL_TICKS`](engine_combat::exchange::CLASH_RECOIL_TICKS). V0 and
/// V1 keep the v1 rows (no recoil), so their bytes never move.
pub const MATCH_WIRE_VERSION_V2: u32 = 2;

/// The rules-v2 match rows: the shipped sweep rows with the recoil exchange row.
const MATCH_V2: MatchTables = MatchTables {
    exchange: EXCHANGE_V2,
    ..MatchTables::v1()
};

/// Map one merged tick input to its exchange intent: an attack press
/// strikes with fixed timing, a held block guards in-window, everything
/// else idles. Attack wins over block when both bits hold; dodge never
/// touches the exchange track.
pub(crate) fn exchange_intent(input: &FighterInput) -> ExchangeIntent {
    if input.buttons.0 & ATTACK != 0 {
        ExchangeIntent::Attack { timing: 1 }
    } else if input.buttons.0 & BLOCK != 0 {
        ExchangeIntent::Block { age: 1 }
    } else {
        ExchangeIntent::Idle
    }
}

/// Integer facing vector for one slot: the quantized wish when driving,
/// else the absolute-facing quadrant. Scale-free, never zero, no trig, so
/// the authoritative tick keeps its `+ - * /` discipline.
pub(crate) fn facing_vector(mx: i8, mz: i8, facing: i16) -> (i32, i32) {
    if mx != 0 || mz != 0 {
        (i32::from(mx) * 10, i32::from(mz) * 10)
    } else if (4501..=13500).contains(&facing) {
        (1000, 0)
    } else if !(-13500..=13500).contains(&facing) {
        (0, -1000)
    } else if !(-4500..=4500).contains(&facing) {
        (-1000, 0)
    } else {
        (0, 1000)
    }
}

/// Arena meters to sweep millimetres, clamped to the sweep position bound.
/// Non-finite input maps to zero; positions are finite by construction.
pub(crate) fn pos_mm(pos: [f32; 3]) -> [i32; 3] {
    const BOUND: f32 = 200_000.0;
    let conv = |v: f32| {
        if !v.is_finite() {
            return 0;
        }
        (v * 1000.0).clamp(-BOUND, BOUND) as i32
    };
    [conv(pos[0]), conv(pos[1]), conv(pos[2])]
}

/// Floating metres from integer millimetres, for a float pose or speed. The
/// one conversion the millimetres side of the sim writes back.
pub(crate) fn metres(mm: i32) -> f32 {
    mm as f32 / 1000.0
}

/// Sweep pose for one slot: arena position in mm, integer facing, the
/// standard hurt capsule; the shield rises exactly while block is held.
pub(crate) fn sweep_fighter(id: u8, state: &FighterState, input: &FighterInput) -> SweepFighter {
    let (fx, fz) = facing_vector(input.mx, input.mz, input.facing);
    SweepFighter {
        id,
        pos_mm: pos_mm(state.pos),
        facing_x: fx,
        facing_z: fz,
        hurt_half_height_mm: HURT_HALF_MM,
        hurt_radius_mm: HURT_RADIUS_MM,
        shield_half_deg: SHIELD_HALF_DEG,
        shield_up: input.buttons.0 & BLOCK != 0,
    }
}

/// Blade segment for an attacking slot: from a base point to full reach
/// along the facing octant sign. Short by construction (diagonal reach
/// stays under 2 m), so the travel bound can never fire on arena sweeps.
pub(crate) fn sweep_for_attacker(id: u8, origin_mm: [i32; 3], facing: (i32, i32)) -> WeaponSweep {
    let (sx, sz) = (facing.0.signum(), facing.1.signum());
    WeaponSweep {
        attacker: id,
        p0_mm: [
            origin_mm[0] + sx * BLADE_BASE_MM,
            origin_mm[1],
            origin_mm[2] + sz * BLADE_BASE_MM,
        ],
        p1_mm: [
            origin_mm[0] + sx * BLADE_REACH_MM,
            origin_mm[1],
            origin_mm[2] + sz * BLADE_REACH_MM,
        ],
        blade_half_mm: BLADE_HALF_MM,
    }
}

/// Arena-owned match track: the combat match state plus caller-owned event
/// buffers, resolved once per tick beside the fight path. Authority stays
/// on the fight step until the golden migrates; this track is observed
/// through [`run_match_events`](crate::run_match_events), never hashed.
#[derive(Clone, Copy)]
pub(crate) struct MatchTrack {
    state: MatchState,
    exchange_buf: [ExchangeEvent; EXCHANGE_CAP],
    sweep_buf: [SweepEvent; SWEEP_CAP],
    exchange_n: usize,
    sweep_n: usize,
    /// Intake path flag: `V0` legacy, `V1` phased. V0 is the default so old
    /// tapes resolve byte-identical without opting in.
    version: u32,
    /// One phased cursor per slot; stepped only while `version` is V1, so
    /// the legacy path never advances a clock it did not have before.
    phased: [PhasedFighter; MAX_PARTICIPANTS as usize],
}

impl MatchTrack {
    /// Fresh track for `count` fighters at full poise; `None` on a bad count.
    pub(crate) fn new(count: u8) -> Option<Self> {
        const EXCHANGE_DUMMY: ExchangeEvent = ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        };
        const SWEEP_DUMMY: SweepEvent = SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        };
        MatchState::new(count, Self::tables())
            .ok()
            .map(|state| Self {
                state,
                exchange_buf: [EXCHANGE_DUMMY; EXCHANGE_CAP],
                sweep_buf: [SWEEP_DUMMY; SWEEP_CAP],
                exchange_n: 0,
                sweep_n: 0,
                version: MATCH_WIRE_VERSION_V0,
                phased: [PhasedFighter::fresh(NO_MOVE); MAX_PARTICIPANTS as usize],
            })
    }

    /// Rebuild the caller buffers around a restored state plus its intake
    /// flag and phased cursors (snapshots carry all three; buffers are tick
    /// scratch). The caller passes a version already validated as V0/V1.
    pub(crate) fn from_state(
        state: &MatchState,
        version: u32,
        phased: [PhasedFighter; crate::MAX_SLOTS],
    ) -> Self {
        const EXCHANGE_DUMMY: ExchangeEvent = ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        };
        const SWEEP_DUMMY: SweepEvent = SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        };
        Self {
            state: *state,
            exchange_buf: [EXCHANGE_DUMMY; EXCHANGE_CAP],
            sweep_buf: [SWEEP_DUMMY; SWEEP_CAP],
            exchange_n: 0,
            sweep_n: 0,
            version,
            phased,
        }
    }

    /// Pin the intake path behind the version flag. Only V0/V1/V2 land;
    /// anything else returns `false` and keeps the old path, never panics.
    pub(crate) fn set_version(&mut self, version: u32) -> bool {
        if version != MATCH_WIRE_VERSION_V0
            && version != MATCH_WIRE_VERSION_V1
            && version != MATCH_WIRE_VERSION_V2
        {
            return false;
        }
        self.version = version;
        true
    }

    /// Current intake path flag.
    pub(crate) fn version(&self) -> u32 {
        self.version
    }

    /// Phased cursors, live only while the version is V1 or V2.
    pub(crate) fn phased(&self) -> &[PhasedFighter; crate::MAX_SLOTS] {
        &self.phased
    }

    /// The shipped GP1/GP2 rows this track resolves under (rules v1).
    pub(crate) fn tables() -> &'static MatchTables {
        &MATCH_V1
    }

    /// The rows one intake version resolves under: V2 carries the clash
    /// recoil, V0 and V1 keep the v1 rows.
    fn tables_for(version: u32) -> &'static MatchTables {
        if version == MATCH_WIRE_VERSION_V2 {
            &MATCH_V2
        } else {
            Self::tables()
        }
    }

    /// True on the phased intakes (V1 and V2).
    fn phased_intake(&self) -> bool {
        self.version == MATCH_WIRE_VERSION_V1 || self.version == MATCH_WIRE_VERSION_V2
    }

    /// Current match state (clocks, poise, binds).
    pub(crate) fn state(&self) -> &MatchState {
        &self.state
    }

    /// Resolve one tick through both landed resolvers into the owned
    /// buffers. No heap, no floats, no panics. V0 forwards every intake
    /// untouched; V1 and V2 gate intakes through the phased rows first. The
    /// resolvers themselves are never re-entered differently: both paths end
    /// in the same [`match_step`] call.
    pub(crate) fn resolve(
        &mut self,
        intents: &[ExchangeIntent; MAX_PARTICIPANTS as usize],
        fighters: &[SweepFighter],
        sweeps: &[WeaponSweep],
    ) {
        if self.phased_intake() {
            self.resolve_phased(intents, fighters, sweeps);
        } else {
            self.resolve_legacy(intents, fighters, sweeps);
        }
    }

    /// Legacy intake path: byte-identical to the pre-flag call-site.
    fn resolve_legacy(
        &mut self,
        intents: &[ExchangeIntent; MAX_PARTICIPANTS as usize],
        fighters: &[SweepFighter],
        sweeps: &[WeaponSweep],
    ) {
        let counts = match_step(
            &mut self.state,
            intents,
            fighters,
            sweeps,
            Self::tables_for(self.version),
            &mut self.exchange_buf,
            &mut self.sweep_buf,
        );
        self.exchange_n = counts.exchange_n;
        self.sweep_n = counts.sweep_n;
    }

    /// Phased intake path: presses start a jab only from a done cursor,
    /// attack intakes forward only on live ticks (carrying the phased age as
    /// clash timing), sweeps forward only while their slot carries live
    /// steel, and a live attack tick attempts its row's own cancel route for
    /// the next tick. Blocks and idles pass through; every gate is an
    /// integer compare on stack arrays, so the tick still allocates nothing.
    fn resolve_phased(
        &mut self,
        intents: &[ExchangeIntent; MAX_PARTICIPANTS as usize],
        fighters: &[SweepFighter],
        sweeps: &[WeaponSweep],
    ) {
        let set = &PHASED_V1;
        // Fresh presses: a done cursor on an attack intake starts the jab.
        // `move 0` is the survey opener; a set without one leaves the cursor
        // done and the intake below gates to idle.
        for (slot, intent) in intents.iter().enumerate() {
            if matches!(intent, ExchangeIntent::Attack { .. })
                && self.phased[slot].phase(set) == MovePhase::Done
                && phased_row(set, 0).is_some()
            {
                self.phased[slot] = PhasedFighter::fresh(0);
            }
        }
        // Gate exchange intakes on the live window.
        let mut gated = [ExchangeIntent::Idle; MAX_PARTICIPANTS as usize];
        for (slot, intent) in intents.iter().enumerate() {
            match *intent {
                ExchangeIntent::Attack { .. } => {
                    let cursor = self.phased[slot];
                    if cursor.phase(set) == MovePhase::Active {
                        gated[slot] = ExchangeIntent::Attack { timing: cursor.age };
                    }
                }
                other => gated[slot] = other,
            }
        }
        // Gate sweeps to live steel under the same ages.
        const SWEEP_IDLE: WeaponSweep = WeaponSweep {
            attacker: 0,
            p0_mm: [0, 0, 0],
            p1_mm: [0, 0, 0],
            blade_half_mm: 0,
        };
        let mut live = [SWEEP_IDLE; SWEEP_IN_CAP];
        let mut live_n: usize = 0;
        for sweep in sweeps {
            let slot = sweep.attacker as usize;
            if live_n < live.len() && slot < self.phased.len() {
                let cursor = self.phased[slot];
                if sweep_live_for(set, cursor.move_id, cursor.age) {
                    live[live_n] = *sweep;
                    live_n += 1;
                }
            }
        }
        // Window-gated cancels ride into the next tick: a live attack tick
        // attempts exactly its row's route, and `try_cancel` refuses anything
        // outside the window, off the route, or on a routeless move.
        for (slot, intent) in intents.iter().enumerate() {
            if matches!(intent, ExchangeIntent::Attack { .. }) {
                let cursor = &mut self.phased[slot];
                if cursor.phase(set) == MovePhase::Active {
                    if let Some(row) = cursor.row(set) {
                        if row.cancel_into != NO_MOVE {
                            let target = row.cancel_into;
                            let _ = cursor.try_cancel(target, set);
                        }
                    }
                }
            }
        }
        let counts = match_step(
            &mut self.state,
            &gated,
            fighters,
            &live[..live_n],
            Self::tables_for(self.version),
            &mut self.exchange_buf,
            &mut self.sweep_buf,
        );
        self.exchange_n = counts.exchange_n;
        self.sweep_n = counts.sweep_n;
    }

    /// Exchange events from the last resolved tick, in fighter-ID order.
    pub(crate) fn exchange_events(&self) -> &[ExchangeEvent] {
        &self.exchange_buf[..self.exchange_n]
    }

    /// Sweep events from the last resolved tick, in ID order.
    pub(crate) fn sweep_events(&self) -> &[SweepEvent] {
        &self.sweep_buf[..self.sweep_n]
    }

    /// Locked riposte through the match state; refusals carry static tags.
    pub(crate) fn riposte(
        &mut self,
        attacker: u8,
        victim: u8,
    ) -> Result<ExchangeEvent, ExchangeError> {
        match_riposte(
            &mut self.state,
            Self::tables_for(self.version),
            attacker,
            victim,
        )
    }

    /// Advance one match tick: decay binds, run the clock. Saturating.
    /// The phased cursors age only on the V1 and V2 paths, so the legacy track
    /// keeps exactly the clocks it always had.
    pub(crate) fn advance(&mut self) {
        if self.phased_intake() {
            for cursor in self.phased.iter_mut() {
                cursor.advance();
            }
        }
        self.state.advance();
    }

    /// Append canonical bytes for the last tick plus state (tests/replays
    /// own the buffer; the per-tick gate never calls this).
    pub(crate) fn append_bytes(&self, out: &mut Vec<u8>) {
        canonical_bytes(
            &self.state,
            self.exchange_events(),
            self.sweep_events(),
            out,
        );
    }
}

/// Largest blade travel the arena mapping can emit (diagonal full reach
/// minus base), in mm; asserts below pin it inside the table travel bound.
#[cfg(test)]
const MAX_ARENA_SWEEP_LEN_MM: i64 = (BLADE_REACH_MM as i64 - BLADE_BASE_MM as i64) * 2;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_combat::arena::MatchTableSet;
    use engine_combat::exchange::ExchangeIntent as Intent;
    use engine_combat::exchange::CLASH_RECOIL_TICKS;

    fn idle() -> [Intent; MAX_PARTICIPANTS as usize] {
        [Intent::Idle; MAX_PARTICIPANTS as usize]
    }

    fn pose(id: u8, x: i32, z: i32, fx: i32, fz: i32, shield_up: bool) -> SweepFighter {
        SweepFighter {
            id,
            pos_mm: [x, 0, z],
            facing_x: fx,
            facing_z: fz,
            hurt_half_height_mm: HURT_HALF_MM,
            hurt_radius_mm: HURT_RADIUS_MM,
            shield_half_deg: SHIELD_HALF_DEG,
            shield_up,
        }
    }

    fn duel() -> [SweepFighter; 2] {
        [pose(0, -1000, 0, 1, 0, false), pose(1, 0, 0, -1, 0, false)]
    }

    fn blade(attacker: u8, p0: [i32; 3], p1: [i32; 3]) -> WeaponSweep {
        WeaponSweep {
            attacker,
            p0_mm: p0,
            p1_mm: p1,
            blade_half_mm: BLADE_HALF_MM,
        }
    }

    fn hit_blade() -> WeaponSweep {
        blade(0, [-100, 0, 0], [100, 0, 0])
    }

    fn miss_blade() -> WeaponSweep {
        blade(0, [-100, 0, 1000], [100, 0, 1000])
    }

    /// One scripted tick through the arena track; returns canonical bytes.
    fn step_bytes(
        track: &mut MatchTrack,
        intents: &[Intent; MAX_PARTICIPANTS as usize],
        fighters: &[SweepFighter],
        sweeps: &[WeaponSweep],
    ) -> Vec<u8> {
        track.resolve(intents, fighters, sweeps);
        let mut bytes = Vec::new();
        track.append_bytes(&mut bytes);
        track.advance();
        bytes
    }

    fn attack_tick(timing: u32, block_age: Option<u32>) -> [Intent; MAX_PARTICIPANTS as usize] {
        let mut intents = idle();
        intents[0] = Intent::Attack { timing };
        if let Some(age) = block_age {
            intents[1] = Intent::Block { age };
        }
        intents
    }

    #[test]
    fn shipped_tables_validate_and_build() {
        assert_eq!(MatchTableSet::v1().validate(), Ok(()));
        assert!(MatchTrack::new(0).is_none());
        assert!(MatchTrack::new(MAX_PARTICIPANTS + 1).is_none());
        assert!(MatchTrack::new(2).is_some());
        // Arena blades stay inside the table travel bound by construction.
        assert!(
            (MAX_ARENA_SWEEP_LEN_MM * MAX_ARENA_SWEEP_LEN_MM)
                < (MatchTrack::tables().sweep.max_sweep_len_mm as i64
                    * MatchTrack::tables().sweep.max_sweep_len_mm as i64)
        );
    }

    #[test]
    fn input_mapping_covers_buttons() {
        let mk = |buttons: u8| FighterInput {
            tick: 0,
            fighter: engine_input::FighterId::new(0).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons(buttons),
        };
        assert!(matches!(
            exchange_intent(&mk(ATTACK)),
            Intent::Attack { .. }
        ));
        assert!(
            matches!(exchange_intent(&mk(ATTACK | BLOCK)), Intent::Attack { .. }),
            "attack wins over block"
        );
        assert!(matches!(exchange_intent(&mk(BLOCK)), Intent::Block { .. }));
        assert_eq!(
            exchange_intent(&mk(engine_input::fighter::DODGE)),
            Intent::Idle,
            "dodge never touches exchange"
        );
        assert_eq!(exchange_intent(&mk(0)), Intent::Idle);
    }

    #[test]
    fn facing_vector_never_zero_without_trig() {
        // Driving wishes pass through scaled; idle facing covers quadrants.
        assert_eq!(facing_vector(100, 0, 0), (1000, 0));
        assert_eq!(facing_vector(0, -100, 0), (0, -1000));
        assert_eq!(facing_vector(0, 0, 0), (0, 1000));
        assert_eq!(facing_vector(0, 0, 9000), (1000, 0));
        assert_eq!(facing_vector(0, 0, -9000), (-1000, 0));
        assert_eq!(facing_vector(0, 0, 18000), (0, -1000));
        assert_eq!(facing_vector(0, 0, -18000), (0, -1000));
        for facing in [
            -18000, -13501, -13500, -4501, -4500, 0, 4500, 4501, 13500, 13501,
        ] {
            let (fx, fz) = facing_vector(0, 0, facing);
            assert!(fx != 0 || fz != 0, "zero facing at {facing}");
        }
    }

    #[test]
    fn arena_sweep_geometry_hits_and_blocks() {
        // The mapped blade reaches a foe 1 m ahead and misses behind.
        let origin = [0, 0, 0];
        let fwd = sweep_for_attacker(0, origin, (1000, 0));
        assert_eq!(fwd.p0_mm, [200, 0, 0]);
        assert_eq!(fwd.p1_mm, [1500, 0, 0]);
        let fighters = duel();
        let mut track = MatchTrack::new(2).unwrap();
        let mut intents = idle();
        track.resolve(&intents, &fighters, &[hit_blade()]);
        assert_eq!(track.sweep_events().len(), 1);
        assert!(matches!(
            track.sweep_events()[0],
            SweepEvent::Hit {
                attacker: 0,
                victim: 1
            }
        ));
        // Same steel against a raised shield blocks instead of hitting.
        let mut guard = duel();
        guard[1].shield_up = true;
        intents = idle();
        track.resolve(&intents, &guard, &[hit_blade()]);
        assert_eq!(
            track.sweep_events(),
            &[SweepEvent::Blocked {
                attacker: 0,
                victim: 1
            }]
        );
        let _ = fwd;
    }

    #[test]
    fn parry_tape_resolves_identical_bytes_twice() {
        let fighters = duel();
        let tape: Vec<([Intent; MAX_PARTICIPANTS as usize], Vec<WeaponSweep>)> = vec![
            (attack_tick(0, Some(1)), vec![hit_blade()]),
            (attack_tick(0, Some(9)), vec![miss_blade()]),
            (attack_tick(2, Some(2)), vec![hit_blade()]),
            (idle(), vec![]),
        ];
        let run = || {
            let mut track = MatchTrack::new(2).unwrap();
            let mut all = Vec::new();
            for (intents, sweeps) in &tape {
                all.push(step_bytes(&mut track, intents, &fighters, sweeps));
            }
            (all, *track.state())
        };
        let (a_bytes, a_state) = run();
        let (b_bytes, b_state) = run();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a_state, b_state);
        let flat: Vec<u8> = a_bytes.concat();
        assert!(flat.contains(&10), "no parry byte on the tape");
        assert!(flat.contains(&20), "no sweep hit byte on the tape");
    }

    #[test]
    fn clash_tape_mirrors_and_replays() {
        let fighters = duel();
        let clash_tick = || {
            let mut intents = idle();
            intents[0] = Intent::Attack { timing: 1 };
            intents[1] = Intent::Attack { timing: 1 };
            intents
        };
        let run_clash = |poise0: u16, poise1: u16| {
            let mut track = MatchTrack::new(2).unwrap();
            track.state.exchange.fighters[0].poise = poise0;
            track.state.exchange.fighters[1].poise = poise1;
            track.resolve(&clash_tick(), &fighters, &[]);
            (track.exchange_events().to_vec(), *track.state())
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
            400 - MatchTrack::tables().exchange.clash_poise_damage
        );
        // Swapping the poise mirrors the winner.
        let (bwd_events, _) = run_clash(400, 700);
        assert_eq!(
            bwd_events,
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 1
            }]
        );
        // Earlier timing wins at equal poise.
        let timed = |ta: u32, tb: u32| {
            let mut intents = idle();
            intents[0] = Intent::Attack { timing: ta };
            intents[1] = Intent::Attack { timing: tb };
            let mut track = MatchTrack::new(2).unwrap();
            track.resolve(&intents, &fighters, &[]);
            track.exchange_events().to_vec()
        };
        assert!(matches!(
            timed(2, 7)[..],
            [ExchangeEvent::Clash { winner: 0, .. }]
        ));
        assert!(matches!(
            timed(7, 2)[..],
            [ExchangeEvent::Clash { winner: 1, .. }]
        ));
        // The clash tape replays byte-identical twice.
        let tape = vec![clash_tick(), idle(), clash_tick()];
        let run = || {
            let mut track = MatchTrack::new(2).unwrap();
            let mut all = Vec::new();
            for intents in &tape {
                all.push(step_bytes(&mut track, intents, &fighters, &[]));
            }
            (all, *track.state())
        };
        let (a_bytes, a_state) = run();
        let (b_bytes, b_state) = run();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a_state, b_state);
        assert!(a_bytes.concat().contains(&12), "no clash byte on the tape");
    }

    #[test]
    fn sweep_tape_resolves_identical_bytes_twice() {
        let fighters = duel();
        let tape: Vec<([Intent; MAX_PARTICIPANTS as usize], Vec<WeaponSweep>)> = vec![
            (idle(), vec![hit_blade()]),
            (idle(), vec![miss_blade()]),
            (
                idle(),
                vec![hit_blade(), blade(1, [-1100, 0, 0], [-900, 0, 0])],
            ),
            (idle(), vec![]),
        ];
        let run = || {
            let mut track = MatchTrack::new(2).unwrap();
            let mut all = Vec::new();
            for (intents, sweeps) in &tape {
                all.push(step_bytes(&mut track, intents, &fighters, sweeps));
            }
            (all, *track.state())
        };
        let (a_bytes, a_state) = run();
        let (b_bytes, b_state) = run();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a_state, b_state);
        assert!(a_bytes[0].contains(&20), "first sweep tick must hit");
        assert!(!a_bytes[1].contains(&20) && !a_bytes[1].contains(&21));
    }

    #[test]
    fn riposte_without_parry_is_denied_never_panics() {
        let mut track = MatchTrack::new(2).unwrap();
        assert_eq!(track.riposte(1, 0), Err(ExchangeError::Denied("no-parry")));
        assert_eq!(track.riposte(0, 0), Err(ExchangeError::Denied("self")));
        assert_eq!(track.riposte(7, 0), Err(ExchangeError::Denied("range")));
        // A parry opens the window; the riposte then lands through the track.
        let fighters = duel();
        let mut intents = idle();
        intents[0] = Intent::Attack { timing: 0 };
        intents[1] = Intent::Block { age: 1 };
        track.resolve(&intents, &fighters, &[]);
        assert_eq!(
            track.exchange_events(),
            &[ExchangeEvent::Parried {
                attacker: 0,
                victim: 1
            }]
        );
        assert!(matches!(
            track.riposte(1, 0),
            Ok(ExchangeEvent::Riposte { .. })
        ));
    }

    #[test]
    fn match_track_allocates_nothing_per_tick() {
        let fighters = duel();
        let sweeps = [hit_blade(), miss_blade()];
        let mut intents = idle();
        intents[0] = Intent::Attack { timing: 1 };
        intents[1] = Intent::Block { age: 1 };
        let mut track = MatchTrack::new(2).unwrap();
        track.resolve(&intents, &fighters, &sweeps);
        track.advance();
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            track.resolve(&intents, &fighters, &sweeps);
        });
        assert_eq!(
            allocs.calls, 0,
            "match track allocated {} times per tick",
            allocs.calls
        );
        // A clash tick plus a riposte attempt also stays allocation-free.
        let mut clash = idle();
        clash[0] = Intent::Attack { timing: 0 };
        clash[1] = Intent::Attack { timing: 0 };
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            track.resolve(&clash, &fighters, &sweeps);
            let _ = track.riposte(1, 0);
        });
        assert_eq!(
            allocs.calls, 0,
            "clash/riposte path allocated {} times",
            allocs.calls
        );
    }

    #[test]
    fn version_flag_defaults_to_legacy_and_rejects_unknown() {
        let mut track = MatchTrack::new(2).unwrap();
        assert_eq!(track.version(), MATCH_WIRE_VERSION_V0);
        assert!(!track.set_version(7));
        assert_eq!(
            track.version(),
            MATCH_WIRE_VERSION_V0,
            "unknown version must keep the old path"
        );
        assert!(track.set_version(MATCH_WIRE_VERSION_V1));
        assert_eq!(track.version(), MATCH_WIRE_VERSION_V1);
        assert!(track.set_version(MATCH_WIRE_VERSION_V0));
        assert_eq!(track.version(), MATCH_WIRE_VERSION_V0);
    }

    #[test]
    fn v1_gates_startup_sweeps_and_cancels_into_cross() {
        // Held attack: startup ticks emit no steel, the first live tick hits
        // and cancels the jab into the cross, whose own live window hits.
        let fighters = duel();
        let mut track = MatchTrack::new(2).unwrap();
        assert!(track.set_version(MATCH_WIRE_VERSION_V1));
        let mut intents = idle();
        intents[0] = Intent::Attack { timing: 1 };
        for tick in 0..4 {
            track.resolve(&intents, &fighters, &[hit_blade()]);
            assert!(
                track.sweep_events().is_empty(),
                "startup tick {tick} must not emit"
            );
            assert_eq!(track.phased[0].move_id, 0, "jab in flight at tick {tick}");
            track.advance();
        }
        // Jab age 4: live steel hits, then the cancel rides into the cross.
        track.resolve(&intents, &fighters, &[hit_blade()]);
        assert_eq!(
            track.sweep_events(),
            &[SweepEvent::Hit {
                attacker: 0,
                victim: 1
            }]
        );
        assert_eq!(
            track.phased[0].move_id, 1,
            "live tick must cancel jab into cross"
        );
        track.advance();
        // Cross startup stays quiet; its live window (ages 6..=8) hits.
        for tick in 5..10 {
            track.resolve(&intents, &fighters, &[hit_blade()]);
            assert!(
                track.sweep_events().is_empty(),
                "cross startup tick {tick} must not emit"
            );
            track.advance();
        }
        for tick in 10..13 {
            track.resolve(&intents, &fighters, &[hit_blade()]);
            assert_eq!(
                track.sweep_events(),
                &[SweepEvent::Hit {
                    attacker: 0,
                    victim: 1
                }],
                "cross live tick {tick} must hit"
            );
            track.advance();
        }
        // Cross endlag commits again.
        track.resolve(&intents, &fighters, &[hit_blade()]);
        assert!(track.sweep_events().is_empty());
    }

    #[test]
    fn v1_clash_fires_only_on_shared_live_ticks_and_replays_x2() {
        // Both slots hold attack: legacy clashes on tick 0, phased stays
        // quiet through startup and clashes first on the shared live tick.
        let fighters = duel();
        let clash_intents = || {
            let mut intents = idle();
            intents[0] = Intent::Attack { timing: 1 };
            intents[1] = Intent::Attack { timing: 1 };
            intents
        };
        let run = |version: u32| {
            let mut track = MatchTrack::new(2).unwrap();
            assert!(track.set_version(version));
            let mut clash_ticks = Vec::new();
            let mut bytes = Vec::new();
            for tick in 0..16 {
                track.resolve(&clash_intents(), &fighters, &[]);
                if track
                    .exchange_events()
                    .iter()
                    .any(|e| matches!(e, ExchangeEvent::Clash { .. }))
                {
                    clash_ticks.push(tick);
                }
                track.append_bytes(&mut bytes);
                track.advance();
            }
            (clash_ticks, bytes)
        };
        let (v0_ticks, v0_bytes) = run(MATCH_WIRE_VERSION_V0);
        assert_eq!(v0_ticks[0], 0, "legacy clashes on the first tick");
        let (v1_ticks, v1_bytes) = run(MATCH_WIRE_VERSION_V1);
        // Live ticks 4 and 10 clash; the clash bind paces the rest, so the
        // cross window contributes one clash, not three.
        assert_eq!(
            v1_ticks,
            vec![4, 10],
            "phased clashes only on shared live ticks"
        );
        assert_ne!(v0_bytes, v1_bytes, "flag must move the observer bytes");
        // Both paths replay byte-identical twice.
        assert_eq!(run(MATCH_WIRE_VERSION_V0).1, v0_bytes);
        assert_eq!(run(MATCH_WIRE_VERSION_V1), (v1_ticks, v1_bytes));
    }

    #[test]
    fn v2_clash_throws_both_blades_back_the_same_ticks_and_cues_once() {
        // Same shared live ticks as the V1 clash. Under V2 both blades are
        // thrown back for the same ticks, each clash emits one Clash event
        // (one cue), and the rows replay byte-identical. V1 throws nothing.
        let fighters = duel();
        let clash_intents = || {
            let mut intents = idle();
            intents[0] = Intent::Attack { timing: 1 };
            intents[1] = Intent::Attack { timing: 1 };
            intents
        };
        let run = |version: u32| {
            let mut track = MatchTrack::new(2).unwrap();
            assert!(track.set_version(version));
            let mut clash_ticks: Vec<u32> = Vec::new();
            let mut recoil: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
            let mut bytes = Vec::new();
            for tick in 0u32..16 {
                track.resolve(&clash_intents(), &fighters, &[]);
                let clashes = track
                    .exchange_events()
                    .iter()
                    .filter(|e| matches!(e, ExchangeEvent::Clash { .. }))
                    .count();
                if clashes > 0 {
                    assert_eq!(clashes, 1, "one clash per tick");
                    clash_ticks.push(tick);
                }
                let exchange = track.state().exchange;
                let (first, second) = (exchange.is_recoiling(0), exchange.is_recoiling(1));
                assert_eq!(first, second, "both blades thrown back on tick {tick}");
                if first {
                    recoil[0].push(tick);
                }
                if second {
                    recoil[1].push(tick);
                }
                track.append_bytes(&mut bytes);
                track.advance();
            }
            (clash_ticks, recoil, bytes)
        };
        let (clash_ticks, recoil, v2_bytes) = run(MATCH_WIRE_VERSION_V2);
        assert_eq!(
            clash_ticks,
            vec![4, 10],
            "V2 clashes on the same live ticks"
        );
        // Each clash holds its recoil for the recoil row, from the clash tick.
        let expected: Vec<u32> = (0u32..16)
            .filter(|t| {
                clash_ticks
                    .iter()
                    .any(|c| *t >= *c && *t < *c + CLASH_RECOIL_TICKS)
            })
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(recoil[0], expected, "slot 0 recoil window");
        assert_eq!(recoil[1], expected, "slot 1 recoil window");
        // Replays byte-identical twice.
        assert_eq!(run(MATCH_WIRE_VERSION_V2), (clash_ticks, recoil, v2_bytes));
        // V1 binds the same clashes and throws nothing back.
        let (v1_ticks, v1_recoil, _) = run(MATCH_WIRE_VERSION_V1);
        assert_eq!(v1_ticks, vec![4, 10]);
        assert!(v1_recoil[0].is_empty() && v1_recoil[1].is_empty());
    }

    #[test]
    fn v1_phased_tick_allocates_nothing() {
        let fighters = duel();
        let sweeps = [hit_blade(), miss_blade()];
        let mut intents = idle();
        intents[0] = Intent::Attack { timing: 1 };
        intents[1] = Intent::Block { age: 1 };
        let mut track = MatchTrack::new(2).unwrap();
        assert!(track.set_version(MATCH_WIRE_VERSION_V1));
        track.resolve(&intents, &fighters, &sweeps);
        track.advance();
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            track.resolve(&intents, &fighters, &sweeps);
            track.advance();
        });
        assert_eq!(
            allocs.calls, 0,
            "phased tick allocated {} times",
            allocs.calls
        );
    }
}
