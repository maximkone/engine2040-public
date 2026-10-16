//! Bot harness: hold F, delay D, six moves, batch receipts over fixed ticks.
use engine_arena::{ArenaConfig, BodyInput, BodyTape};
use engine_core::{splitmix64, Fnv64};
/// Config: hold F, delay D, bouts N, ticks per bout, first seed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BotHarness {
    pub hold_ticks: u32,
    pub delay_frames: u32,
    pub bouts: u32,
    pub ticks_per_bout: u32,
    pub base_seed: u64,
}
/// Config failures; divisibility fails before any simulation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HarnessError {
    BadField(u32),
    NotDivisible { delay: u32, hold: u32 },
    NoTemplate,
}
/// One bout receipt: winner, fixed ticks, strike counts, lag in ticks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HarnessReceipt {
    pub seed: u64,
    pub winner: Option<u8>,
    pub ticks: u32,
    pub strikes: [u32; 2],
    pub mean_lag_ticks: u32,
}
/// Batch aggregate in tournament vocabulary: permille, fixed ticks.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct HarnessAggregate {
    pub bouts: u32,
    pub ticks_per_bout: u32,
    pub win_permille: [u16; 2],
    pub mean_lag_ticks: u32,
    pub mean_lag_decisions: u32,
    pub stuck_bouts: u32,
    pub digest: u64,
    pub receipts: Vec<HarnessReceipt>,
}
/// Move table: hold, retreat, advance, strike, guarded retreat/advance.
pub fn move_input(t: &BodyInput, slot: u8, tick: u32, i: u8) -> Option<BodyInput> {
    let row = [(0, 0), (-100, 0), (100, 0), (0, 1), (-100, 3), (100, 3)];
    let (mx, byte) = *row.get(usize::try_from(i).unwrap_or(99))?;
    let mut out = *t;
    out.tick = tick;
    out.fighter = engine_arena::BodyId::new(slot)?;
    out.mx = mx;
    out.mz = 0;
    out.facing = 0;
    out.buttons.0 = byte;
    Some(out)
}
/// Delay selected decisions by lag; early outputs hold idle.
pub fn delay_selected(selected: &[u8], lag: usize) -> Vec<u8> {
    selected
        .iter()
        .enumerate()
        .map(|(k, _)| if k < lag { 0 } else { selected[k - lag] })
        .collect()
}
impl BotHarness {
    /// Fail before simulation on bad ranges or non-divisible delay.
    pub fn validate(&self) -> Result<(), HarnessError> {
        if self.hold_ticks == 0 || self.hold_ticks > 64 {
            return Err(HarnessError::BadField(self.hold_ticks));
        }
        if self.delay_frames > 64 || self.bouts == 0 || self.bouts > 100 {
            return Err(HarnessError::BadField(self.delay_frames));
        }
        if self.ticks_per_bout == 0 || self.ticks_per_bout > engine_arena::MAX_TICKS {
            return Err(HarnessError::BadField(self.ticks_per_bout));
        }
        if self.delay_frames % self.hold_ticks != 0 {
            return Err(HarnessError::NotDivisible {
                delay: self.delay_frames,
                hold: self.hold_ticks,
            });
        }
        Ok(())
    }
    fn bout_tape(&self, seed: u64, bout: u32, t: &BodyInput) -> Option<(BodyTape, [u32; 2])> {
        let lag_ticks = self.delay_frames.checked_div(self.hold_ticks).unwrap_or(0);
        let lag = usize::try_from(lag_ticks).unwrap_or(0);
        let count = self.ticks_per_bout / self.hold_ticks.max(1) + 1;
        let mut selected = [Vec::new(), Vec::new()];
        for k in 0..count {
            for slot in 0..2u8 {
                let mixed = seed
                    .wrapping_add(u64::from(bout).wrapping_mul(7919))
                    .wrapping_add(u64::from(slot).wrapping_mul(104_729))
                    .wrapping_add(u64::from(k).wrapping_mul(1_299_709));
                selected[usize::try_from(slot).unwrap_or(0)]
                    .push(u8::try_from(splitmix64(mixed) % 6).unwrap_or(0));
            }
        }
        let executed: Vec<Vec<u8>> = selected.iter().map(|s| delay_selected(s, lag)).collect();
        let (mut inputs, mut strikes) = (Vec::new(), [0u32; 2]);
        for tick in 0..self.ticks_per_bout {
            let k = usize::try_from(tick.checked_div(self.hold_ticks).unwrap_or(0)).unwrap_or(0);
            for slot in 0..2u8 {
                let s = usize::try_from(slot).unwrap_or(0);
                let input = move_input(t, slot, tick, *executed[s].get(k).unwrap_or(&0))?;
                strikes[s] += u32::from(input.buttons.0 & 1 != 0);
                inputs.push(input);
            }
        }
        inputs.sort_by_key(|v| (v.tick, v.fighter.0));
        let tape = BodyTape(inputs);
        tape.validate().ok()?;
        Some((tape, strikes))
    }
    /// Run N seeded bouts; winner holds more strikes, digest binds the run.
    pub fn run(&self) -> Result<HarnessAggregate, HarnessError> {
        self.validate()?;
        let (mut wins, mut draws, mut stuck) = ([0u32; 2], 0u32, 0u32);
        let mut receipts = Vec::new();
        let mut acc = Fnv64::new();
        for b in 0..self.bouts {
            let seed = self.base_seed.wrapping_add(u64::from(b));
            let config = ArenaConfig {
                seed,
                ticks: self.ticks_per_bout,
                fighters: 2,
                half_extent_m: 20.0,
                table_version: 1,
            };
            let template = crate::agent::template_input(&config).ok_or(HarnessError::NoTemplate)?;
            let pair = self
                .bout_tape(seed, b, &template)
                .ok_or(HarnessError::NoTemplate)?;
            let (tape, strikes) = pair;
            let points = engine_arena::run(&config, &tape);
            let last_hash = points
                .last()
                .map(|p| p.hash)
                .ok_or(HarnessError::NoTemplate)?;
            let winner = (strikes[0] != strikes[1]).then(|| u8::from(strikes[0] < strikes[1]));
            stuck += u32::from(strikes[0] + strikes[1] == 0);
            draws += u32::from(winner.is_none());
            wins[0] += u32::from(winner == Some(0));
            wins[1] += u32::from(winner == Some(1));
            let mark = winner.unwrap_or(2);
            acc.write_u64(seed.wrapping_add(u64::from(mark)));
            acc.write_u64((u64::from(strikes[0]) << 32) | u64::from(strikes[1]));
            acc.write_u64(last_hash);
            receipts.push(HarnessReceipt {
                seed,
                winner,
                ticks: self.ticks_per_bout,
                strikes,
                mean_lag_ticks: self.delay_frames,
            });
        }
        let total = 2u32.wrapping_mul(self.bouts);
        let perm = |w: u32| {
            let scored = 2u32.wrapping_mul(w).wrapping_add(draws);
            scored.wrapping_mul(1000).checked_div(total).unwrap_or(0)
        };
        Ok(HarnessAggregate {
            bouts: self.bouts,
            ticks_per_bout: self.ticks_per_bout,
            win_permille: [
                u16::try_from(perm(wins[0])).unwrap_or(0),
                u16::try_from(perm(wins[1])).unwrap_or(0),
            ],
            mean_lag_ticks: self.delay_frames,
            mean_lag_decisions: self.delay_frames.checked_div(self.hold_ticks).unwrap_or(0),
            stuck_bouts: stuck,
            digest: acc.finish(),
            receipts,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const TINY: BotHarness = BotHarness {
        hold_ticks: 2,
        delay_frames: 4,
        bouts: 2,
        ticks_per_bout: 60,
        base_seed: 1,
    };
    #[test]
    fn guard_rejects_non_divisible_before_sim() {
        let bad = BotHarness {
            hold_ticks: 3,
            ..TINY
        };
        assert!(bad.validate().is_err());
        assert!(bad.run().is_err());
    }
    #[test]
    fn lag_is_two_decisions_on_scripted_tape() {
        let executed = delay_selected(&[3u8, 0, 4, 0, 5], 2);
        assert_eq!(executed, vec![0u8, 0, 3, 0, 4]);
    }
    #[test]
    fn batch_permille_sums_and_digest_repeats() {
        let harness = BotHarness {
            bouts: 20,
            base_seed: 2040,
            ..TINY
        };
        let first = harness.run().expect("batch runs");
        let second = harness.run().expect("batch replays");
        assert_eq!(first, second);
        let total = u32::from(first.win_permille[0]) + u32::from(first.win_permille[1]);
        assert_eq!(total, 1000);
        assert_eq!(first.mean_lag_ticks, 4);
        assert_eq!(first.mean_lag_decisions, 2);
        assert_eq!(crate::bot_harness_fixture().bouts, 2);
    }
}
