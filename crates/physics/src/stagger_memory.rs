//! stagger memory: cumulative pain, servo fatigue and per-bone injury.
//!
//! Consumer: the arena hit pipeline feeds each landed blow to both
//! [`crate::balance::BalanceState`] (the blow's own level) and
//! [`StaggerMemory`] (what the blow leaves behind) through
//! [`crate::balance::BalanceState::apply_memory_hit`]; presentation reads
//! [`StaggerMemory::servo_strength_q8`] to soften the powered-ragdoll drive
//! and [`StaggerMemory::limp_mm`] to pose the limp. Owner: this module owns
//! [`StaggerMemory`], [`StaggerLevel`] and the pain, fatigue and injury
//! budgets. Units: Q8 impulse units where 256 reads unity, ticks at 60 Hz,
//! millimetres of limp. Determinism: integer counters plus comparisons only,
//! fixed bone order, no clock and no dice — one hit sequence hashes one way.
//! Errors: [`crate::balance::BalanceError`], never a panic on caller data.
//! Budget: the record plus step path writes the struct only and allocates
//! nothing; ten hits in two seconds escalate inside the tick budget.

use crate::balance::{BalanceError, MAX_IMPULSE_Q8};
use engine_core::Fnv64;

/// Bone slots tracked for injury, indices `0..MAX_MEMORY_BONES`: the
/// six-bone fighter rig fits with room, and wider rigs map their strike
/// bones into these slots. Fixed so the tick path never allocates.
pub const MAX_MEMORY_BONES: usize = 8;
/// Pain ceiling, Q8 impulse units: past this further blows hold the peak
/// instead of wrapping, so a beating reads ragdoll rather than overflowing.
pub const PAIN_CAP_Q8: u16 = 4096;
/// Pain that answers a stagger, Q8 units: at or past this the fighter
/// answers late, matching the balance stagger window.
pub const STAGGER_PAIN_Q8: u16 = 512;
/// Pain that answers a full ragdoll, Q8 units: at or past this the body
/// goes down with no dice, matching the balance fall.
pub const RAGDOLL_PAIN_Q8: u16 = 896;
/// Pain shed per fixed tick, Q8 units: ten 192-unit hits spaced 12 ticks
/// apart still climb from flinch through stagger to ragdoll, while a lone
/// hit clears back to flinch inside four seconds.
pub const PAIN_DECAY_PER_TICK_Q8: u16 = 8;
/// Fatigue added per landed hit, Q8 units: ten hits nearly fill the cap, so
/// a real exchange always tires the servo.
pub const FATIGUE_PER_HIT_Q8: u16 = 24;
/// Fatigue ceiling, Q8 units: past this the servo holds its weakest drive
/// instead of going limp, so the fighter can still be driven.
pub const FATIGUE_CAP_Q8: u16 = 256;
/// Fatigue ticks per point shed: fatigue clears thirty-two times slower
/// than pain, so tired arms outlast the flinch that tired them.
pub const FATIGUE_DECAY_EVERY_TICKS: u64 = 4;
/// Injury ceiling per bone, Q8 units: a beaten leg holds its worst limp
/// instead of wrapping, and clears inside [`LIMP_BUDGET_TICKS`].
pub const INJURY_CAP_Q8: u16 = 1024;
/// Injury shed per fixed tick, Q8 units: the worst limp clears in 256
/// ticks, inside [`LIMP_BUDGET_TICKS`].
pub const INJURY_DECAY_PER_TICK_Q8: u16 = 4;
/// Limp budget, ticks at 60 Hz: every limp clears inside five seconds, so a
/// leg hit never limps forever.
pub const LIMP_BUDGET_TICKS: u32 = 300;
/// Full servo drive, Q8 units: a rested fighter tracks the animated pose at
/// unity, and fatigue subtracts from here.
pub const SERVO_STRENGTH_FULL_Q8: u16 = 256;
/// Limp scale, injury units per millimetre: the worst limp reads 16 mm of
/// offset, a visible but recoverable drag of the struck leg.
pub const LIMP_MM_PER_UNIT: u16 = 64;

/// Pain verdict: what the accumulated beating warrants. Flinch is a wince
/// that never breaks the stance; stagger answers late; ragdoll goes down.
/// No dice: the same pain reads the same level twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaggerLevel {
    /// Under the stagger pain: the stance holds, the hit still aches.
    Flinch,
    /// At or past the stagger pain: answers late like a balance stagger.
    Stagger,
    /// At or past the ragdoll pain: the body goes down like a balance fall.
    Ragdoll,
}

impl StaggerLevel {
    /// Stable hash code: flinch reads 0 through ragdoll reads 2, so one
    /// pain level hashes one way.
    const fn code(self) -> u8 {
        match self {
            StaggerLevel::Flinch => 0,
            StaggerLevel::Stagger => 1,
            StaggerLevel::Ragdoll => 2,
        }
    }
}

/// Cumulative beating state: pain that escalates flinch to stagger to
/// ragdoll, fatigue that softens the servo drive, and per-bone injury that
/// outlasts the spring. Integer counters only, fixed bone order, hashed as
/// a whole beside the balance hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaggerMemory {
    /// Accumulated pain, Q8 units; decays every tick.
    pain_q8: u16,
    /// Accumulated fatigue, Q8 units; decays slowly.
    fatigue_q8: u16,
    /// Per-bone injury, Q8 units; decays inside [`LIMP_BUDGET_TICKS`].
    injury_q8: [u16; MAX_MEMORY_BONES],
    /// Ticks stepped since spawn; drives the slow fatigue decay.
    tick: u64,
    /// Recorded-hit counter; folded into the hash so hit order shows.
    hits: u64,
}

impl StaggerMemory {
    /// Fresh memory: no pain, a rested servo, sound bones.
    pub fn new() -> Self {
        Self {
            pain_q8: 0,
            fatigue_q8: 0,
            injury_q8: [0; MAX_MEMORY_BONES],
            tick: 0,
            hits: 0,
        }
    }

    /// Current pain, Q8 impulse units.
    pub fn pain_q8(&self) -> u16 {
        self.pain_q8
    }

    /// Current fatigue, Q8 units; [`StaggerMemory::servo_strength_q8`]
    /// turns it into drive.
    pub fn fatigue_q8(&self) -> u16 {
        self.fatigue_q8
    }

    /// Recorded-hit counter.
    pub fn hit_count(&self) -> u64 {
        self.hits
    }

    /// Ticks stepped since spawn.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Pain verdict for the current accumulation: flinch under
    /// [`STAGGER_PAIN_Q8`], stagger under [`RAGDOLL_PAIN_Q8`], ragdoll at or
    /// past it. Pure, no allocation.
    pub fn level(&self) -> StaggerLevel {
        if self.pain_q8 >= RAGDOLL_PAIN_Q8 {
            StaggerLevel::Ragdoll
        } else if self.pain_q8 >= STAGGER_PAIN_Q8 {
            StaggerLevel::Stagger
        } else {
            StaggerLevel::Flinch
        }
    }

    /// Record one landed hit on one bone: pain climbs toward the cap,
    /// fatigue climbs toward its own cap, and the struck bone takes the
    /// blow's impulse as injury. Impulses past the ceiling and bones past
    /// the slots are refused with nothing changed. Scalar fields only, no
    /// allocation.
    pub fn record_hit(
        &mut self,
        impulse_q8: u16,
        bone: usize,
    ) -> Result<StaggerLevel, BalanceError> {
        if impulse_q8 > MAX_IMPULSE_Q8 {
            return Err(BalanceError::BadImpulse(impulse_q8));
        }
        if bone >= MAX_MEMORY_BONES {
            return Err(BalanceError::BadBone {
                bone,
                bones: MAX_MEMORY_BONES,
            });
        }
        self.pain_q8 = self.pain_q8.saturating_add(impulse_q8).min(PAIN_CAP_Q8);
        self.fatigue_q8 = self
            .fatigue_q8
            .saturating_add(FATIGUE_PER_HIT_Q8)
            .min(FATIGUE_CAP_Q8);
        let slot = &mut self.injury_q8[bone];
        *slot = slot.saturating_add(impulse_q8).min(INJURY_CAP_Q8);
        self.hits = self.hits.wrapping_add(1);
        Ok(self.level())
    }

    /// Advance one fixed tick: pain sheds every tick, fatigue sheds one
    /// point every [`FATIGUE_DECAY_EVERY_TICKS`] ticks, and every bone
    /// sheds injury inside [`LIMP_BUDGET_TICKS`]. Scalar fields only, fixed
    /// bone order, no allocation.
    pub fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.pain_q8 = self.pain_q8.saturating_sub(PAIN_DECAY_PER_TICK_Q8);
        if self.tick.is_multiple_of(FATIGUE_DECAY_EVERY_TICKS) {
            self.fatigue_q8 = self.fatigue_q8.saturating_sub(1);
        }
        for slot in self.injury_q8.iter_mut() {
            *slot = slot.saturating_sub(INJURY_DECAY_PER_TICK_Q8);
        }
    }

    /// Servo drive left, Q8 units where 256 reads full: fatigue subtracts
    /// half its weight, so ten hits nearly halve the drive and a rested
    /// fighter reads unity. Pure, no allocation.
    pub fn servo_strength_q8(&self) -> u16 {
        SERVO_STRENGTH_FULL_Q8.saturating_sub(self.fatigue_q8 / 2)
    }

    /// Injury on one bone, Q8 units; bones past the slots are refused.
    /// Pure, no allocation.
    pub fn injury_q8(&self, bone: usize) -> Result<u16, BalanceError> {
        if bone >= MAX_MEMORY_BONES {
            return Err(BalanceError::BadBone {
                bone,
                bones: MAX_MEMORY_BONES,
            });
        }
        Ok(self.injury_q8[bone])
    }

    /// Limp offset on one bone, millimetres: injury divided into whole
    /// millimetres, so a fresh bruise drags and a sound bone reads zero. A
    /// leg hit leaves this standing after the spring recovers, and it
    /// clears inside [`LIMP_BUDGET_TICKS`]. Pure, no allocation.
    pub fn limp_mm(&self, bone: usize) -> Result<u16, BalanceError> {
        Ok(self.injury_q8(bone)? / LIMP_MM_PER_UNIT)
    }

    /// Canonical hash over pain, fatigue, every bone injury and both
    /// counters: one hit sequence hashes one way, and an extra hit moves
    /// it. Pure, no allocation.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u16(self.pain_q8);
        h.write_u16(self.fatigue_q8);
        h.write_u8(self.level().code());
        for injury in self.injury_q8 {
            h.write_u16(injury);
        }
        h.write_u64(self.tick);
        h.write_u64(self.hits);
        h.finish()
    }
}

impl Default for StaggerMemory {
    /// Fresh memory, like [`StaggerMemory::new`].
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    /// Ten light blows spaced 12 ticks apart: 120 ticks of fight, the same
    /// script the integration leg drives from outside the crate.
    fn acceptance_run(memory: &mut StaggerMemory) -> [StaggerLevel; 10] {
        let mut levels = [StaggerLevel::Flinch; 10];
        for slot in levels.iter_mut() {
            *slot = memory.record_hit(192, 0).expect("bounded hit");
            for _ in 0..12 {
                memory.step();
            }
        }
        levels
    }

    /// Severity rank for the escalation proof: flinch reads 0 through
    /// ragdoll reads 2, so a sorted rank row means an ordered escalation.
    fn severity(level: StaggerLevel) -> u8 {
        match level {
            StaggerLevel::Flinch => 0,
            StaggerLevel::Stagger => 1,
            StaggerLevel::Ragdoll => 2,
        }
    }

    #[test]
    fn stagger_memory_escalates_ten_hits_in_two_seconds() {
        let mut memory = StaggerMemory::new();
        assert_eq!(memory.level(), StaggerLevel::Flinch);
        let levels = acceptance_run(&mut memory);
        assert_eq!(levels[0], StaggerLevel::Flinch);
        assert_eq!(levels[9], StaggerLevel::Ragdoll);
        let ranks = levels.map(severity);
        let mut ordered = ranks;
        ordered.sort_unstable();
        assert_eq!(ranks, ordered, "escalation never steps down");
        assert_eq!(
            ranks,
            [0, 0, 0, 0, 1, 1, 1, 1, 2, 2],
            "ten light hits visit flinch, stagger, ragdoll in that order"
        );
        // Twins hash twice; one more hit moves the hash.
        let mut twin = StaggerMemory::new();
        let twin_levels = acceptance_run(&mut twin);
        assert_eq!(levels, twin_levels);
        assert_eq!(memory, twin);
        assert_eq!(
            memory.state_hash(),
            twin.state_hash(),
            "same hit sequence hashes twice"
        );
        twin.record_hit(192, 0).expect("extra hit");
        assert_ne!(
            memory.state_hash(),
            twin.state_hash(),
            "hit order must reach the hash"
        );
    }

    #[test]
    fn stagger_memory_leg_hit_leaves_limp_inside_budget() {
        let mut memory = StaggerMemory::new();
        assert_eq!(
            memory.record_hit(512, 1).expect("leg hit"),
            StaggerLevel::Stagger
        );
        assert_eq!(memory.injury_q8(1).expect("leg injury"), 512);
        assert_eq!(memory.limp_mm(1).expect("leg limp"), 8);
        assert_eq!(memory.injury_q8(2).expect("calm bone"), 0);
        assert_eq!(memory.limp_mm(2).expect("calm limp"), 0);
        // The spring recovers first: pain falls under the stagger line while
        // the limp still stands.
        while memory.level() != StaggerLevel::Flinch {
            memory.step();
        }
        assert!(
            memory.limp_mm(1).expect("leg limp") > 0,
            "injury persists after the spring recovers"
        );
        // Then the limp clears inside its own budget.
        let mut stepped: u32 = 0;
        while memory.limp_mm(1).expect("leg limp") > 0 {
            memory.step();
            stepped = stepped.saturating_add(1);
        }
        assert!(stepped > 0, "the limp must take ticks to clear");
        assert!(
            stepped <= LIMP_BUDGET_TICKS,
            "limp took {stepped} ticks, past budget {LIMP_BUDGET_TICKS}"
        );
    }

    #[test]
    fn stagger_memory_fatigue_lowers_servo_strength_slowly() {
        let mut memory = StaggerMemory::new();
        assert_eq!(memory.servo_strength_q8(), SERVO_STRENGTH_FULL_Q8);
        acceptance_run(&mut memory);
        assert_eq!(memory.fatigue_q8(), 210);
        assert_eq!(memory.servo_strength_q8(), 151);
        // Pain clears in seconds; fatigue does not clear with it.
        for _ in 0..120 {
            memory.step();
        }
        assert_eq!(memory.level(), StaggerLevel::Flinch);
        assert!(
            memory.servo_strength_q8() < SERVO_STRENGTH_FULL_Q8,
            "fatigue must outlast pain"
        );
        for _ in 0..1200 {
            memory.step();
        }
        assert_eq!(memory.servo_strength_q8(), SERVO_STRENGTH_FULL_Q8);
    }

    #[test]
    fn stagger_memory_rejects_bad_input_untouched() {
        let mut memory = StaggerMemory::new();
        let before = memory.state_hash();
        assert_eq!(
            memory.record_hit(MAX_IMPULSE_Q8 + 1, 0).unwrap_err(),
            BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
        );
        assert_eq!(
            memory.record_hit(192, MAX_MEMORY_BONES).unwrap_err(),
            BalanceError::BadBone {
                bone: MAX_MEMORY_BONES,
                bones: MAX_MEMORY_BONES,
            }
        );
        assert_eq!(
            memory.injury_q8(MAX_MEMORY_BONES).unwrap_err(),
            BalanceError::BadBone {
                bone: MAX_MEMORY_BONES,
                bones: MAX_MEMORY_BONES,
            }
        );
        assert_eq!(
            memory.limp_mm(MAX_MEMORY_BONES).unwrap_err(),
            BalanceError::BadBone {
                bone: MAX_MEMORY_BONES,
                bones: MAX_MEMORY_BONES,
            }
        );
        assert_eq!(memory.hit_count(), 0);
        assert_eq!(memory, StaggerMemory::new());
        assert_eq!(memory.state_hash(), before);
        // Caps hold the peak instead of wrapping.
        for _ in 0..30 {
            memory.record_hit(MAX_IMPULSE_Q8, 0).expect("capped hit");
        }
        assert_eq!(memory.pain_q8(), PAIN_CAP_Q8);
        assert_eq!(memory.fatigue_q8(), FATIGUE_CAP_Q8);
        assert_eq!(memory.injury_q8(0).expect("beaten bone"), INJURY_CAP_Q8);
        assert_eq!(memory.level(), StaggerLevel::Ragdoll);
        assert_eq!(memory.servo_strength_q8(), 128);
    }

    #[test]
    fn stagger_memory_tick_path_stays_alloc_free() {
        let ((), allocs) = alloc_probe::measure(|| {
            let mut memory = StaggerMemory::new();
            acceptance_run(&mut memory);
            let _ = memory.servo_strength_q8();
            let _ = memory.limp_mm(1);
            let _ = memory.state_hash();
        });
        assert_eq!(
            allocs.calls, 0,
            "memory path must not allocate, got {allocs:?}"
        );
    }
}
