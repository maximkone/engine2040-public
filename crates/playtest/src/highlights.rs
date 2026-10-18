//! SHOW3 highlight reel: rank the moments of tournament tapes by event
//! metadata and cut a reel inside a length budget.
//!
//! Consumer: devlog and trailer cuts. A reel is a shot list (tape seed plus a
//! tick range per shot) that the SHOW1 clip capture replays. Data owner: the
//! caller owns the tapes; the arena owns the events ([`run_events`]). The
//! tournament tapes are the PQ2 match configs replayed brains-only, so a
//! shot's seed is a PQ2 seed. Units: fixed ticks, hp permille, integer
//! scores. Determinism: the same tapes and budget give the same reel in the
//! same order; no float, no wall clock, no map order. Ranking reads event
//! metadata only: a vision score is an advisory note and never moves a shot.
//! Errors: [`Failure`] when a tournament config cannot give tapes.

use std::path::{Path, PathBuf};

use engine_arena::{run_events, ArenaConfig, BodyTape, TickEvents, MAX_PARTICIPANTS, TICK_HZ};
use engine_combat::{fight::MAX_HP_PERMILLE, FightEvent};

use crate::tournament::TournamentConfig;
use crate::Failure;

/// This module's name in [`Failure::verifier`].
pub const VERIFIER: &str = "HighlightReel";
/// Default reel length: 30 s of play in fixed ticks.
pub const REEL_BUDGET_TICKS: u32 = 30 * TICK_HZ;
/// Longest reel the cutter plans for (10 min); a larger budget is clamped.
pub const MAX_BUDGET_TICKS: u32 = 600 * TICK_HZ;
/// Longest single shot: merged moments stay under 5 s.
pub const MAX_SHOT_TICKS: u32 = 5 * TICK_HZ;
/// A parry at or below this hp (permille) is a clutch parry.
pub const CLUTCH_HP_PERMILLE: i32 = 300;
/// Trailing by this many hp permille and then taking the lead is a comeback.
pub const COMEBACK_GAP_PERMILLE: i32 = 250;
/// Landed hits in a row (same attacker, same victim) that make a long combo.
pub const COMBO_MIN_HITS: u32 = 3;
/// Longest gap between two hits of one combo, in ticks.
pub const COMBO_GAP_TICKS: u32 = TICK_HZ / 2;
/// Moments the knapsack considers: the best by score, so cost stays bounded.
pub const MAX_CANDIDATES: usize = 512;

/// Ticks of lead-in and tail kept around each kind of moment.
const KO_LEAD: u32 = 2 * TICK_HZ;
const KO_TAIL: u32 = TICK_HZ;
const COMEBACK_LEAD: u32 = 2 * TICK_HZ;
const COMEBACK_TAIL: u32 = TICK_HZ / 2;
const PARRY_LEAD: u32 = TICK_HZ * 3 / 4;
const PARRY_TAIL: u32 = TICK_HZ * 3 / 4;
const COMBO_LEAD: u32 = TICK_HZ / 2;
const COMBO_TAIL: u32 = TICK_HZ / 2;
/// Base scores: a knockout outranks a swing, a swing outranks a parry.
const KO_SCORE: u32 = 1000;
const COMEBACK_SCORE: u32 = 600;
const PARRY_SCORE: u32 = 500;
const COMBO_SCORE_PER_HIT: u32 = 200;

/// What makes a moment worth a shot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum MomentKind {
    /// A fighter dropped to zero hp.
    Knockout,
    /// A fighter that trailed by the comeback gap took the lead.
    Comeback,
    /// A parry landed while the parrier was nearly out of hp.
    ClutchParry,
    /// Several hits in a row by one fighter on one victim.
    LongCombo,
}

impl MomentKind {
    /// Stable lowercase name used in the shot list.
    pub fn name(self) -> &'static str {
        match self {
            Self::Knockout => "knockout",
            Self::Comeback => "comeback",
            Self::ClutchParry => "clutch_parry",
            Self::LongCombo => "long_combo",
        }
    }
}

/// One ranked moment: where it sits and how much it is worth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Moment {
    /// Index of the tape inside the batch.
    pub tape: u32,
    /// Match seed of that tape.
    pub seed: u64,
    /// First tick of the shot, inclusive.
    pub first_tick: u32,
    /// Last tick of the shot, inclusive.
    pub last_tick: u32,
    /// The strongest kind that makes up the moment.
    pub kind: MomentKind,
    /// Integer worth; merged moments add their scores.
    pub score: u32,
}

impl Moment {
    /// Shot length in ticks (`first_tick..=last_tick`).
    pub fn ticks(&self) -> u32 {
        self.last_tick - self.first_tick + 1
    }

    fn overlaps(&self, other: &Moment) -> bool {
        self.first_tick <= other.last_tick && other.first_tick <= self.last_tick
    }
}

/// One tape: its match seed, fighter count and per-tick combat events.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TapeEvents {
    /// Match seed.
    pub seed: u64,
    /// Fighters in the match (`2..=MAX_PARTICIPANTS`).
    pub fighters: u8,
    /// One entry per tick, tick order, as [`run_events`] returns them.
    pub events: Vec<TickEvents>,
}

/// A hit chain in progress: one attacker, one victim, hits close together.
#[derive(Clone, Copy)]
struct Chain {
    attacker: u8,
    victim: u8,
    first: u32,
    last: u32,
    hits: u32,
}

/// Event scan over one tape: tracks hp, the open hit chain and the worst
/// deficit of each fighter since it last led.
struct Scan {
    tape: u32,
    seed: u64,
    fighters: usize,
    last_tick: u32,
    hp: Vec<i32>,
    deficit: Vec<i32>,
    chain: Option<Chain>,
    out: Vec<Moment>,
}

impl Scan {
    /// Record a moment over the ticks `span` (first, last), padded by
    /// `pad` (lead-in, tail) and clamped to the tape.
    fn push(&mut self, kind: MomentKind, span: (u32, u32), pad: (u32, u32), score: u32) {
        self.out.push(Moment {
            tape: self.tape,
            seed: self.seed,
            first_tick: span.0.saturating_sub(pad.0),
            last_tick: (span.1 + pad.1).min(self.last_tick),
            kind,
            score,
        });
    }

    fn hurt(&mut self, victim: u8, amount: u16) {
        if let Some(hp) = self.hp.get_mut(usize::from(victim)) {
            *hp = (*hp - i32::from(amount)).max(0);
        }
    }

    fn hit(&mut self, tick: u32, attacker: u8, victim: u8) {
        let same = |chain: &Chain| {
            chain.attacker == attacker
                && chain.victim == victim
                && tick.saturating_sub(chain.last) <= COMBO_GAP_TICKS
        };
        if let Some(chain) = self.chain.as_mut().filter(|chain| same(chain)) {
            chain.last = tick;
            chain.hits += 1;
            return;
        }
        self.end_chain();
        self.chain = Some(Chain {
            attacker,
            victim,
            first: tick,
            last: tick,
            hits: 1,
        });
    }

    fn end_chain(&mut self) {
        let Some(chain) = self.chain.take() else {
            return;
        };
        if chain.hits >= COMBO_MIN_HITS {
            let score = COMBO_SCORE_PER_HIT * (chain.hits - COMBO_MIN_HITS + 1);
            let span = (chain.first, chain.last);
            self.push(MomentKind::LongCombo, span, (COMBO_LEAD, COMBO_TAIL), score);
        }
    }

    fn event(&mut self, tick: u32, event: &FightEvent) {
        match *event {
            FightEvent::Hit {
                attacker,
                victim,
                damage,
            } => {
                self.hurt(victim, damage);
                self.hit(tick, attacker, victim);
            }
            FightEvent::Blocked { victim, chip, .. } => self.hurt(victim, chip),
            FightEvent::Parried { victim, .. } => {
                let hp = self
                    .hp
                    .get(usize::from(victim))
                    .copied()
                    .unwrap_or(i32::MAX);
                if hp <= CLUTCH_HP_PERMILLE {
                    let score = PARRY_SCORE + points(CLUTCH_HP_PERMILLE - hp);
                    let span = (tick, tick);
                    self.push(
                        MomentKind::ClutchParry,
                        span,
                        (PARRY_LEAD, PARRY_TAIL),
                        score,
                    );
                }
            }
            FightEvent::Knockdown { .. } => {
                self.push(
                    MomentKind::Knockout,
                    (tick, tick),
                    (KO_LEAD, KO_TAIL),
                    KO_SCORE,
                );
            }
            FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => {}
        }
    }

    /// After a tick's events: a fighter that led again after trailing by the
    /// gap has made a comeback.
    fn lead_swing(&mut self, tick: u32) {
        for fighter in 0..self.fighters {
            let rival = (0..self.fighters)
                .filter(|&other| other != fighter)
                .map(|other| self.hp[other])
                .max()
                .unwrap_or(0);
            let behind = rival - self.hp[fighter];
            if behind >= COMEBACK_GAP_PERMILLE {
                self.deficit[fighter] = self.deficit[fighter].max(behind);
            } else if behind < 0 {
                let deficit = std::mem::take(&mut self.deficit[fighter]);
                if deficit >= COMEBACK_GAP_PERMILLE {
                    let score = COMEBACK_SCORE + points(deficit);
                    let span = (tick, tick);
                    self.push(
                        MomentKind::Comeback,
                        span,
                        (COMEBACK_LEAD, COMEBACK_TAIL),
                        score,
                    );
                }
            }
        }
    }
}

/// Moments of one tape, merged and in tick order. Moments that overlap fuse
/// into one shot when the shot stays under [`MAX_SHOT_TICKS`] (a knockout
/// that ends a comeback is worth both); otherwise the better one stays.
pub fn moments(tape: u32, source: &TapeEvents) -> Vec<Moment> {
    let fighters = usize::from(source.fighters).clamp(2, usize::from(MAX_PARTICIPANTS));
    let mut scan = Scan {
        tape,
        seed: source.seed,
        fighters,
        last_tick: source.events.last().map_or(0, |row| row.tick),
        hp: vec![i32::from(MAX_HP_PERMILLE); fighters],
        deficit: vec![0; fighters],
        chain: None,
        out: Vec::new(),
    };
    for row in &source.events {
        for event in &row.events {
            scan.event(row.tick, event);
        }
        scan.lead_swing(row.tick);
    }
    scan.end_chain();
    let mut found = scan.out;
    found.sort_by_key(|m| (m.first_tick, m.last_tick, m.kind));
    let mut merged: Vec<Moment> = Vec::with_capacity(found.len());
    for next in found {
        match merged.last_mut() {
            Some(prev) if prev.overlaps(&next) => *prev = fuse(*prev, next),
            _ => merged.push(next),
        }
    }
    merged
}

/// Two overlapping moments as one: fused when short enough, else the better.
fn fuse(a: Moment, b: Moment) -> Moment {
    let (best, other) = if rank_order(&a, &b).is_le() {
        (a, b)
    } else {
        (b, a)
    };
    let first_tick = a.first_tick.min(b.first_tick);
    let last_tick = a.last_tick.max(b.last_tick);
    if last_tick - first_tick < MAX_SHOT_TICKS {
        Moment {
            first_tick,
            last_tick,
            score: best.score.saturating_add(other.score),
            ..best
        }
    } else {
        best
    }
}

/// Best first: score high to low, then tape, tick and kind so ties are fixed.
fn rank_order(a: &Moment, b: &Moment) -> std::cmp::Ordering {
    b.score
        .cmp(&a.score)
        .then(a.tape.cmp(&b.tape))
        .then(a.first_tick.cmp(&b.first_tick))
        .then(a.kind.cmp(&b.kind))
}

/// A cut reel: the shots in playback order plus how they were chosen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reel {
    /// Budget the cut ran under, in ticks (clamped to [`MAX_BUDGET_TICKS`]).
    pub budget_ticks: u32,
    /// Moments found across all tapes before the cut.
    pub candidates: usize,
    /// Chosen shots in tape then tick order.
    pub shots: Vec<Moment>,
}

impl Reel {
    /// Ticks the shots add up to; never above the budget.
    pub fn used_ticks(&self) -> u32 {
        self.shots.iter().map(Moment::ticks).sum()
    }

    /// Score the shots add up to.
    pub fn total_score(&self) -> u64 {
        self.shots.iter().map(|m| u64::from(m.score)).sum()
    }

    /// The shot list: a header, one line per shot, one advisory line.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "SHOW3 highlight reel budget_ticks={} used_ticks={} shots={} candidates={} score={}\n",
            self.budget_ticks,
            self.used_ticks(),
            self.shots.len(),
            self.candidates,
            self.total_score(),
        );
        for shot in &self.shots {
            out.push_str(&format!(
                "shot tape={} seed={} ticks={}..{} kind={} score={}\n",
                shot.tape,
                shot.seed,
                shot.first_tick,
                shot.last_tick,
                shot.kind.name(),
                shot.score,
            ));
        }
        out.push_str(
            "advisory: ranking reads event metadata only; vision scores never move a shot\n",
        );
        out
    }
}

/// Cut the best reel the budget allows: a 0/1 knapsack over the ranked
/// moments (weight = shot ticks, value = score). Ties fall to the earlier
/// ranked moment, so the same tapes always give the same shots.
pub fn cut_reel(tapes: &[TapeEvents], budget_ticks: u32) -> Reel {
    let budget_ticks = budget_ticks.min(MAX_BUDGET_TICKS);
    let mut found: Vec<Moment> = tapes
        .iter()
        .enumerate()
        .flat_map(|(index, tape)| moments(u32::try_from(index).unwrap_or(u32::MAX), tape))
        .collect();
    let candidates = found.len();
    found.retain(|m| m.ticks() <= budget_ticks);
    found.sort_by(rank_order);
    found.truncate(MAX_CANDIDATES);
    let mut shots = knapsack(&found, slots(budget_ticks));
    shots.sort_by_key(|m| (m.tape, m.first_tick));
    Reel {
        budget_ticks,
        candidates,
        shots,
    }
}

/// Ticks as a table index.
fn slots(ticks: u32) -> usize {
    usize::try_from(ticks).unwrap_or(usize::MAX)
}

/// A non-negative hp amount as score points (a negative one scores none).
fn points(permille: i32) -> u32 {
    u32::try_from(permille).unwrap_or(0)
}

fn knapsack(items: &[Moment], capacity: usize) -> Vec<Moment> {
    let width = capacity + 1;
    let mut best = vec![0u64; width];
    let mut taken = vec![false; items.len() * width];
    for (index, item) in items.iter().enumerate() {
        let weight = slots(item.ticks());
        for room in (weight..width).rev() {
            let with = best[room - weight] + u64::from(item.score);
            if with > best[room] {
                best[room] = with;
                taken[index * width + room] = true;
            }
        }
    }
    let mut room = capacity;
    let mut chosen = Vec::new();
    for (index, item) in items.iter().enumerate().rev() {
        if taken[index * width + room] {
            chosen.push(*item);
            room -= slots(item.ticks());
        }
    }
    chosen
}

/// The tapes of one PQ2 difficulty: the tournament's own seeds
/// (`difficulty_seed(index) + i`), tick count and arena, replayed through
/// the arena event stream with the brains driving both fighters.
pub fn tournament_tapes(
    config: &TournamentConfig,
    index: usize,
) -> Result<Vec<TapeEvents>, Failure> {
    if index >= config.difficulties.len() {
        return Err(failure(format!("difficulty {index} is not in the config")));
    }
    let base = config.difficulty_seed(index);
    (0..config.matches)
        .map(|i| {
            let seed = base.wrapping_add(u64::from(i));
            let arena = ArenaConfig {
                seed,
                ticks: config.match_ticks,
                fighters: 2,
                half_extent_m: config.half_extent_m,
                table_version: 1,
            };
            let events = run_events(&arena, &BodyTape::default());
            if events.is_empty() {
                return Err(failure(format!("seed {seed} gave no events")));
            }
            Ok(TapeEvents {
                seed,
                fighters: 2,
                events,
            })
        })
        .collect()
}

fn failure(reason: String) -> Failure {
    Failure {
        verifier: VERIFIER,
        reason,
    }
}

/// Default report directory beside the other playtest reports.
pub fn default_report_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reports")
}

/// Write `reel.txt` (the shot list) under `dir` and return its path.
pub fn write_reel(dir: &Path, reel: &Reel) -> Result<PathBuf, Failure> {
    std::fs::create_dir_all(dir).map_err(|error| failure(error.to_string()))?;
    let path = dir.join("reel.txt");
    std::fs::write(&path, reel.to_text()).map_err(|error| failure(error.to_string()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(attacker: u8, victim: u8, damage: u16) -> FightEvent {
        FightEvent::Hit {
            attacker,
            victim,
            damage,
        }
    }

    /// A tape of `ticks` quiet ticks with the given `(tick, event)` pairs.
    fn tape(seed: u64, ticks: u32, at: &[(u32, FightEvent)]) -> TapeEvents {
        let events = (0..ticks)
            .map(|tick| TickEvents {
                tick,
                events: at
                    .iter()
                    .filter(|(when, _)| *when == tick)
                    .map(|(_, event)| *event)
                    .collect(),
            })
            .collect();
        TapeEvents {
            seed,
            fighters: 2,
            events,
        }
    }

    /// The seeded best moment: a fighter that trailed by 750 permille takes
    /// the lead and then the knockout, after a clutch parry earlier on.
    fn swing_tape(seed: u64) -> TapeEvents {
        tape(
            seed,
            700,
            &[
                (100, hit(0, 1, 400)),
                (200, hit(0, 1, 350)),
                (
                    300,
                    FightEvent::Parried {
                        attacker: 0,
                        victim: 1,
                    },
                ),
                (400, hit(1, 0, 500)),
                (500, hit(1, 0, 400)),
                (520, hit(1, 0, 200)),
                (
                    520,
                    FightEvent::Knockdown {
                        attacker: 1,
                        victim: 0,
                    },
                ),
            ],
        )
    }

    #[test]
    fn moments_find_the_clutch_parry_and_fuse_the_comeback_with_the_knockout() {
        let found = moments(7, &swing_tape(99));
        let shape: Vec<_> = found
            .iter()
            .map(|m| (m.kind, m.first_tick, m.last_tick, m.score))
            .collect();
        assert_eq!(
            shape,
            vec![
                // hp 250 at the parry: 500 + (300 - 250).
                (MomentKind::ClutchParry, 255, 345, 550),
                // comeback 600 + 750 deficit, fused with the 1000 knockout.
                (MomentKind::Comeback, 380, 580, 2350),
            ]
        );
        assert!(found.iter().all(|m| m.tape == 7 && m.seed == 99));
    }

    #[test]
    fn a_long_combo_needs_three_close_hits_by_one_attacker() {
        let kinds = |at: &[(u32, FightEvent)]| -> Vec<MomentKind> {
            moments(0, &tape(1, 400, at))
                .iter()
                .map(|m| m.kind)
                .collect()
        };
        let close = [
            (10, hit(0, 1, 50)),
            (30, hit(0, 1, 50)),
            (50, hit(0, 1, 50)),
        ];
        assert_eq!(kinds(&close), vec![MomentKind::LongCombo]);
        let gap = [
            (10, hit(0, 1, 50)),
            (30, hit(0, 1, 50)),
            (61, hit(0, 1, 50)),
        ];
        assert!(
            kinds(&gap).is_empty(),
            "a gap past the limit breaks the chain"
        );
        let answered = [
            (10, hit(0, 1, 50)),
            (30, hit(0, 1, 50)),
            (40, hit(1, 0, 50)),
            (50, hit(0, 1, 50)),
        ];
        assert!(kinds(&answered).is_empty(), "a hit back breaks the chain");
    }

    #[test]
    fn the_knapsack_beats_taking_the_best_score_first() {
        let item = |first_tick, ticks: u32, score| Moment {
            tape: 0,
            seed: 0,
            first_tick,
            last_tick: first_tick + ticks - 1,
            kind: MomentKind::LongCombo,
            score,
        };
        // Best score first takes the 10 and fills the room; two 6s are worth more.
        let items = [item(0, 6, 10), item(100, 3, 6), item(200, 3, 6)];
        let chosen = knapsack(&items, 6);
        assert_eq!(chosen.iter().map(|m| m.score).sum::<u32>(), 12);
        assert_eq!(knapsack(&items, 0), Vec::new());
    }

    #[test]
    fn two_hundred_tapes_give_the_same_reel_twice_and_the_seeded_moment_wins() {
        let mut config = TournamentConfig::standard();
        config.matches = 199;
        let mut tapes = tournament_tapes(&config, 0).expect("199 PQ2 tapes");
        tapes.push(swing_tape(4242));
        assert_eq!(tapes.len(), 200);
        let seeded = tapes.len() as u32 - 1;

        let reel = cut_reel(&tapes, REEL_BUDGET_TICKS);
        assert_eq!(reel, cut_reel(&tapes, REEL_BUDGET_TICKS), "same reel x2");
        assert_eq!(
            reel.to_text(),
            cut_reel(&tapes, REEL_BUDGET_TICKS).to_text()
        );
        assert!(reel.used_ticks() <= REEL_BUDGET_TICKS);
        let best = reel.shots.iter().max_by_key(|m| m.score).expect("shots");
        assert_eq!(
            (best.tape, best.score),
            (seeded, 2350),
            "seeded moment is best"
        );

        // A budget that fits one shot of that length picks the seeded one.
        let tight = cut_reel(&tapes, 201);
        assert_eq!(tight.shots.len(), 1);
        assert_eq!(tight.shots[0].tape, seeded);

        // Negative control: without the knockout the fused moment is gone.
        let mut weaker = tapes.clone();
        weaker[seeded as usize].events[520].events.pop();
        let after = cut_reel(&weaker, 201);
        assert_ne!(after.shots.first().map(|m| m.score), Some(2350));
    }

    #[test]
    fn the_reel_text_is_a_shot_list_and_writes_to_disk() {
        let reel = cut_reel(&[swing_tape(5)], REEL_BUDGET_TICKS);
        let text = reel.to_text();
        assert!(text.starts_with("SHOW3 highlight reel budget_ticks=1800 used_ticks=292 shots=2"));
        assert!(text.contains("shot tape=0 seed=5 ticks=380..580 kind=comeback score=2350"));
        assert!(text.ends_with(
            "vision scores never move a shot
"
        ));
        let dir = std::env::temp_dir().join("e2040-highlights-test");
        let path = write_reel(&dir, &reel).expect("reel writes");
        assert_eq!(std::fs::read_to_string(&path).expect("reads back"), text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_config_without_that_difficulty_is_refused() {
        let config = TournamentConfig::standard();
        let refused = tournament_tapes(&config, config.difficulties.len()).expect_err("refused");
        assert_eq!(refused.verifier, VERIFIER);
    }
}
