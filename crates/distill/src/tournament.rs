//! Fork tournament: student vs scripted baseline vs recorded teacher, and
//! the strength gate: the student net in real arena bouts.
//!
//! [`strength_gate`] plays the exported net against the arena depth brain
//! (actuated like the student, or raw) through the public controller seam (see [`crate::duel`]): N bouts
//! (default [`DEFAULT_BOUTS`]), sides swapped every bout, wins, losses and
//! draws per side, a Wilson 95% interval and an exact two-sided binomial
//! p-value against 50% (gogame 10.3.1-10.3.2). The agreement tournament
//! below predates it and still scores recorded rows only.
//!
//! Consumer: the ship gate and the neural-policy evidence — the
//! report must show the exported student winning at least as often as the
//! scripted baseline over (by default) 100 fork matches. Owner: this module
//! owns the bout scoring, the lane-local scripted stand-in and the
//! balance-verifier-mirrored bounds check. Units: observation features are
//! unitless `f32` in `[-MAX_FEATURE_ABS, MAX_FEATURE_ABS]` (the training
//! range); actions are class ids (`u8`); rates are permille (`0..=1000`) in
//! integer math. Determinism scope: same bytes plus same scripts give the
//! same report on every machine — runtime inference is integer-only, the scripted
//! stand-in is integer-only, and scoring uses integer counts only; no wall
//! clock, no threads, no float division. Errors: every bad input maps to
//! [`TournamentError`]; [`tournament`] never panics.
//!
//! ## Lane-local mirrors (no new dependency edges)
//!
//! The agreement tournament predates the arena edge (the strength gate uses
//! it), so its fork, net policy and balance verifier are mirrored here as
//! plain-data shapes, never imported:
//!
//! - A fork match is one recorded [`MatchScript`]: seeded observations plus
//!   the seed that produced them, scored against recorded teacher actions
//!   (decision rows mapped to plain features, the same mapping the trainer
//!   uses).
//! - Each bout is scored with [`Comparison`], which copies the arena brain
//!   compare harness shape field for field (`total`, `agree`,
//!   `agree_rate_permille`, `first_divergent_index`): agreement with the
//!   recorded teacher over the shared script, ties drawn.
//! - The bounds check copies the reference-arena verifier's semantics as a
//!   plain comparison: the student's bout-win rate must sit inside
//!   `[BALANCE_MIN_PERMILLE, BALANCE_MAX_PERMILLE]` (`[350, 650]`, the
//!   documented reference bounds). The pin test below keeps the literals
//!   honest without importing `engine-playtest`.
//!
//! ## Bout structure
//!
//! Every match plays three pairwise bouts over one shared observation script:
//! student-vs-scripted (higher teacher-agreement banks the bout, ties drawn),
//! student-vs-teacher and scripted-vs-teacher (the recorded teacher always
//! scores every row, so a challenger banks the bout exactly when it equals
//! the teacher on every row, otherwise the teacher banks it). Wins therefore
//! count bouts, not matches: `student_wins + scripted_wins + teacher_wins`
//! is `3 * matches` minus the drawn challenger bouts. The student's rate is
//! `student_wins * 1000 / (3 * matches)` in integer math, mirroring the
//! verifier's permille convention.
//!
//! The scripted stand-in ([`scripted_decide`]) is deliberately unaligned with
//! any teacher: a fixed deterministic hash of the quantized lanes, so beating
//! it proves the student carried teacher knowledge through the int8 export.
//! The real scripted-vs-net duel still belongs to the arena brain harness; this
//! gate proves the tournament machinery plus student competitiveness on a
//! fixed recorded fixture.

use crate::data::MAX_FEATURE_ABS;
use crate::duel::{play_bout, BoutOutcome, DuelError, Opponent};
use crate::export::quantize_input;

/// Default matches per tournament: the neural-policy proof counts wins over
/// 100 fork matches.
pub const DEFAULT_MATCHES: u32 = 100;

/// Upper bound on matches per call: mirrors the verifier's `winrates::MAX_MATCHES` so a
/// tournament stays a bounded content check, not an open loop.
pub const MAX_MATCHES: u32 = 100;

/// Minimum acceptable student bout-win rate in permille: mirrors the
/// reference-arena verifier's lower bound (`BalanceConfig.min_win_permille`).
pub const BALANCE_MIN_PERMILLE: u16 = 350;

/// Maximum acceptable student bout-win rate in permille: mirrors the
/// reference-arena verifier's upper bound (`BalanceConfig.max_win_permille`).
pub const BALANCE_MAX_PERMILLE: u16 = 650;

/// Largest action-class count the tournament scores: teacher actions travel
/// as `u8`, so wider nets have no addressable class ids here.
pub const MAX_CLASSES: usize = 256;

/// Largest observation count accepted per match: one decision per tick for a
/// 60 s bout at 60 Hz, mirroring the bout replay length. Keeps the full-100
/// tournament a small bounded CPU check.
pub const MAX_OBSERVATIONS_PER_MATCH: usize = 3_600;

/// One fork match fixture: recorded observations plus the seed that produced
/// them, so the report's seed list replays every match exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchScript {
    /// Deterministic stream this script's observations came from
    /// (`base.wrapping_add(index)` by convention, mirroring the verifier's seeds).
    pub seed: u64,
    /// Observation rows, one feature vector each; every row must be exactly
    /// the student input dim wide, finite and within `±MAX_FEATURE_ABS`.
    pub features: Vec<Vec<f32>>,
}

/// Per-policy score over one shared script: the arena brain comparison harness shape
/// (`total`, `agree`, `agree_rate_permille`, `first_divergent_index`),
/// measured against the recorded teacher actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comparison {
    /// Rows compared (the script length).
    pub total: usize,
    /// Rows where the policy picked the recorded teacher action.
    pub agree: usize,
    /// `agree * 1000 / total` (1000 when the script is empty, mirroring the
    /// arena brain harness's vacuous-agreement convention).
    pub agree_rate_permille: u32,
    /// First row index where the policy missed the teacher, if any.
    pub first_divergent_index: Option<usize>,
}

/// Score one decided action sequence against the recorded teacher actions.
/// Total over the teacher slice: short decision slices count missing rows as
/// misses, long ones ignore the excess. Pure and total; never panics.
pub fn score_against_teacher(decided: &[u8], teacher: &[u8]) -> Comparison {
    let mut agree = 0usize;
    let mut first_divergent_index: Option<usize> = None;
    for (index, want) in teacher.iter().enumerate() {
        if decided.get(index) == Some(want) {
            agree += 1;
        } else if first_divergent_index.is_none() {
            first_divergent_index = Some(index);
        }
    }
    let total = teacher.len();
    let agree_rate_permille = if total == 0 {
        1000
    } else {
        // u64 keeps giant caller slices from overflowing; the quotient fits
        // u32 because `agree <= total` bounds it by 1000.
        let rate = (agree as u64 * 1000) / (total as u64);
        rate as u32
    };
    Comparison {
        total,
        agree,
        agree_rate_permille,
        first_divergent_index,
    }
}

/// Lane-local scripted stand-in: a fixed deterministic function of the
/// quantized lanes (integer lane sum modulo the class count). Total for any
/// input; always returns a class below `classes` (0 when `classes` is 0).
/// This stands in for the arena's scripted Brain, which this crate cannot import;
/// it is deliberately unaligned with any teacher so a distilling student must
/// beat it with carried teacher knowledge, not with the heuristic itself.
pub fn scripted_decide(quantized: &[i8], classes: usize) -> u8 {
    if classes == 0 {
        return 0;
    }
    let cap = classes.min(MAX_CLASSES) as i64;
    let sum: i64 = quantized.iter().map(|lane| i64::from(*lane)).sum();
    // `rem_euclid` yields `0..cap` with `cap <= 256`, so at most 255 fits u8.
    sum.rem_euclid(cap) as u8
}

/// Student decision: runtime integer inference over the quantized row, first-max
/// wins (the arena's `argmax_intent` tie-break convention). Returns `None` only
/// when the net emits no logits, which runtime-validated shapes rule out; the
/// caller maps that to [`TournamentError::EmptyLogits`] instead of panicking.
fn student_class(model: &engine_nn::Model, quantized: &[i8]) -> Option<u8> {
    let logits = engine_nn::infer(model, quantized);
    crate::student::argmax(&logits).and_then(|index| u8::try_from(index).ok())
}

/// Fork-tournament outcome: bout-win tallies plus the measured student rate
/// and the verifier-mirrored bounds verdict. Wins count pairwise bouts (three per
/// match); drawn challenger bouts bank nothing, so the tallies sum to
/// `3 * matches` minus draws. `seeds` is the exact played seed list, in match
/// order, so the report replays exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TournamentReport {
    /// Bouts the student banked (challenger bout plus perfect-teacher ties).
    pub student_wins: u32,
    /// Bouts the scripted stand-in banked.
    pub scripted_wins: u32,
    /// Bouts the recorded teacher banked (every bout it strictly outscored).
    pub teacher_wins: u32,
    /// Matches played (always equal to `seeds.len()`).
    pub matches: u32,
    /// Total observation rows scored across all matches.
    pub observations: u32,
    /// `student_wins * 1000 / (3 * matches)` in integer math.
    pub student_rate_permille: u16,
    /// True exactly when the student rate sits inside
    /// `[BALANCE_MIN_PERMILLE, BALANCE_MAX_PERMILLE]`.
    pub within_bounds: bool,
    /// Played seeds in match order (`scripts[i].seed`).
    pub seeds: Vec<u64>,
}

/// Tournament input failures. Every variant names the match/row at fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TournamentError {
    /// Match count is zero or above [`MAX_MATCHES`]; carries the count.
    BadMatches(u32),
    /// Fewer observation scripts than matches; carries `(have, want)`.
    NotEnoughScripts {
        /// Scripts supplied.
        have: usize,
        /// Matches requested.
        want: usize,
    },
    /// Fewer teacher action sequences than matches; carries `(have, want)`.
    NotEnoughTeacher {
        /// Sequences supplied.
        have: usize,
        /// Matches requested.
        want: usize,
    },
    /// Student bytes are not a valid `.e2nn` file; the runtime's cause is carried.
    BadStudent(engine_nn::NnErr),
    /// Student output dim exceeds [`MAX_CLASSES`]; teacher actions are `u8`.
    BadClassCap(usize),
    /// Match script has no rows; carries the match index.
    EmptyScript {
        /// Match index.
        index: usize,
    },
    /// Match script exceeds [`MAX_OBSERVATIONS_PER_MATCH`]; carries index
    /// and row count.
    TooManyObservations {
        /// Match index.
        index: usize,
        /// Rows on the script.
        got: usize,
    },
    /// Row width differs from the student input dim; carries match, row,
    /// `(got, want)`.
    BadFeatureLen {
        /// Match index.
        index: usize,
        /// Row index.
        row: usize,
        /// Features on the row.
        got: usize,
        /// Student input dim.
        want: usize,
    },
    /// Row holds a NaN, infinite or out-of-range feature; carries match/row.
    BadFeature {
        /// Match index.
        index: usize,
        /// Row index.
        row: usize,
    },
    /// Teacher action count differs from the observation count; carries the
    /// match index and both lengths.
    RowMismatch {
        /// Match index.
        index: usize,
        /// Observation rows.
        features: usize,
        /// Teacher actions.
        teacher: usize,
    },
    /// Teacher action names no student class; carries match, row, `(got,
    /// cap)`.
    BadTeacherClass {
        /// Match index.
        index: usize,
        /// Row index.
        row: usize,
        /// Action class recorded.
        got: usize,
        /// Student output dim.
        cap: usize,
    },
    /// Net emitted no logits on a row (defensive: runtime-validated shapes rule
    /// this out); carries match/row.
    EmptyLogits {
        /// Match index.
        index: usize,
        /// Row index.
        row: usize,
    },
}

impl core::fmt::Display for TournamentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TournamentError::BadMatches(n) => {
                write!(f, "match count {n} outside 1..={MAX_MATCHES}")
            }
            TournamentError::NotEnoughScripts { have, want } => {
                write!(f, "have {have} scripts, need {want}")
            }
            TournamentError::NotEnoughTeacher { have, want } => {
                write!(f, "have {have} teacher sequences, need {want}")
            }
            TournamentError::BadStudent(cause) => write!(f, "bad student bytes: {cause}"),
            TournamentError::BadClassCap(got) => {
                write!(f, "student output dim {got} above {MAX_CLASSES}")
            }
            TournamentError::EmptyScript { index } => {
                write!(f, "match {index} has no rows")
            }
            TournamentError::TooManyObservations { index, got } => write!(
                f,
                "match {index} has {got} rows, cap {MAX_OBSERVATIONS_PER_MATCH}"
            ),
            TournamentError::BadFeatureLen {
                index,
                row,
                got,
                want,
            } => write!(f, "match {index} row {row}: feature len {got}, want {want}"),
            TournamentError::BadFeature { index, row } => {
                write!(
                    f,
                    "match {index} row {row}: feature non-finite or past ±{MAX_FEATURE_ABS}"
                )
            }
            TournamentError::RowMismatch {
                index,
                features,
                teacher,
            } => write!(
                f,
                "match {index}: {features} observations but {teacher} teacher actions"
            ),
            TournamentError::BadTeacherClass {
                index,
                row,
                got,
                cap,
            } => write!(
                f,
                "match {index} row {row}: teacher class {got} at or above {cap}"
            ),
            TournamentError::EmptyLogits { index, row } => {
                write!(f, "match {index} row {row}: net emitted no logits")
            }
        }
    }
}

impl std::error::Error for TournamentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TournamentError::BadStudent(cause) => Some(cause),
            _ => None,
        }
    }
}

/// Play the fork tournament: the exported student (runtime integer inference plus
/// first-max-wins) and the scripted stand-in decide every recorded
/// observation, both scored against the recorded teacher actions, three
/// pairwise bouts per match. Plays the first `matches` entries of `scripts`
/// zipped with the first `matches` entries of `teacher_script`.
///
/// `student_bytes` must be single-dense-layer `.e2nn` bytes as [`export`](crate::export)
/// emits (any runtime-valid dims work; the input dim sets the row width and the
/// output dim sets the class count). `teacher_script[m]` is match `m`'s
/// recorded teacher action per observation row. Pure and deterministic: the
/// same inputs give the same report on every machine.
pub fn tournament(
    student_bytes: &[u8],
    teacher_script: &[Vec<u8>],
    scripts: &[MatchScript],
    matches: u32,
) -> Result<TournamentReport, TournamentError> {
    if matches == 0 || matches > MAX_MATCHES {
        return Err(TournamentError::BadMatches(matches));
    }
    let want = matches as usize;
    if scripts.len() < want {
        return Err(TournamentError::NotEnoughScripts {
            have: scripts.len(),
            want,
        });
    }
    if teacher_script.len() < want {
        return Err(TournamentError::NotEnoughTeacher {
            have: teacher_script.len(),
            want,
        });
    }
    let model = engine_nn::load(student_bytes).map_err(TournamentError::BadStudent)?;
    let in_dim = model.input_dim();
    let out_dim = model.output_dim();
    if out_dim > MAX_CLASSES {
        return Err(TournamentError::BadClassCap(out_dim));
    }

    let mut student_wins = 0u32;
    let mut scripted_wins = 0u32;
    let mut teacher_wins = 0u32;
    let mut observations = 0u32;
    let mut seeds = Vec::with_capacity(want);
    for (index, (script, teacher)) in scripts
        .iter()
        .zip(teacher_script.iter())
        .enumerate()
        .take(want)
    {
        let rows = script.features.len();
        if rows == 0 {
            return Err(TournamentError::EmptyScript { index });
        }
        if rows > MAX_OBSERVATIONS_PER_MATCH {
            return Err(TournamentError::TooManyObservations { index, got: rows });
        }
        if teacher.len() != rows {
            return Err(TournamentError::RowMismatch {
                index,
                features: rows,
                teacher: teacher.len(),
            });
        }
        let mut student_seq = Vec::with_capacity(rows);
        let mut scripted_seq = Vec::with_capacity(rows);
        for (row, (features, want_class)) in script.features.iter().zip(teacher.iter()).enumerate()
        {
            if features.len() != in_dim {
                return Err(TournamentError::BadFeatureLen {
                    index,
                    row,
                    got: features.len(),
                    want: in_dim,
                });
            }
            if !features
                .iter()
                .all(|v| v.is_finite() && v.abs() <= MAX_FEATURE_ABS)
            {
                return Err(TournamentError::BadFeature { index, row });
            }
            if usize::from(*want_class) >= out_dim {
                return Err(TournamentError::BadTeacherClass {
                    index,
                    row,
                    got: usize::from(*want_class),
                    cap: out_dim,
                });
            }
            let quantized = quantize_input(features);
            let student = student_class(&model, &quantized)
                .ok_or(TournamentError::EmptyLogits { index, row })?;
            student_seq.push(student);
            scripted_seq.push(scripted_decide(&quantized, out_dim));
        }
        let student_cmp = score_against_teacher(&student_seq, teacher);
        let scripted_cmp = score_against_teacher(&scripted_seq, teacher);
        let (agree_student, agree_scripted) = (student_cmp.agree, scripted_cmp.agree);
        // Challenger bout: higher teacher-agreement banks it; ties draw.
        if agree_student > agree_scripted {
            student_wins += 1;
        } else if agree_scripted > agree_student {
            scripted_wins += 1;
        }
        // Ceiling bouts: the recorded teacher scores every row, so a
        // challenger banks its bout exactly when it equals the teacher
        // everywhere; otherwise the teacher banks it.
        if agree_student == rows {
            student_wins += 1;
        } else {
            teacher_wins += 1;
        }
        if agree_scripted == rows {
            scripted_wins += 1;
        } else {
            teacher_wins += 1;
        }
        observations += rows as u32;
        seeds.push(script.seed);
    }
    // `matches >= 1`, so the divisor is at least 3; `student_wins <= 3 *
    // matches <= 300`, so the permille fits u16.
    let student_rate_permille = (student_wins * 1000 / (3 * matches)) as u16;
    let within_bounds =
        (BALANCE_MIN_PERMILLE..=BALANCE_MAX_PERMILLE).contains(&student_rate_permille);
    Ok(TournamentReport {
        student_wins,
        scripted_wins,
        teacher_wins,
        matches,
        observations,
        student_rate_permille,
        within_bounds,
        seeds,
    })
}

// --- Strength gate: real arena bouts (gogame 10.3) -------------------------

/// Default strength-gate bouts: gogame 10.3.2 asks for about a thousand
/// games before a few-percent edge means anything (53 of 100 gives p = 0.62).
pub const DEFAULT_BOUTS: u32 = 1000;
/// Most bouts one gate call plays.
pub const MAX_BOUTS: u32 = 20_000;
/// Default bout length: 60 s at 60 Hz.
pub const DEFAULT_BOUT_TICKS: u32 = 3_600;
/// Two-sided 95% normal quantile for the Wilson interval.
const Z95: f64 = 1.959_963_984_540_054;

/// Strength-gate outcome: the student net against the arena depth brain
/// over `bouts` real arena bouts, sides swapped every bout. Bout `i` plays
/// seed `seed_base + i / 2` with the student in slot `i % 2`, so every seed
/// is fought from both sides. Draws (same-tick double knockout or equal
/// health at time) are reported and left out of the binomial test.
#[derive(Clone, Debug, PartialEq)]
pub struct StrengthReport {
    /// FNV-1a 64 of the student `.e2nn` bytes.
    pub net_digest: u64,
    /// Who the student fought.
    pub opponent: Opponent,
    /// First bout seed.
    pub seed_base: u64,
    /// Bouts played.
    pub bouts: u32,
    /// Ticks per bout.
    pub bout_ticks: u32,
    /// Student wins by slot (`[slot 0, slot 1]`).
    pub wins: [u32; 2],
    /// Student losses by slot.
    pub losses: [u32; 2],
    /// Draws by slot.
    pub draws: [u32; 2],
    /// Wilson 95% interval for the student's share of decisive bouts.
    pub wilson95: (f64, f64),
    /// Exact two-sided binomial p-value of the decisive split against 50%.
    pub p_value: f64,
}

impl StrengthReport {
    /// Total student wins.
    pub fn total_wins(&self) -> u32 {
        self.wins[0] + self.wins[1]
    }
    /// Total student losses.
    pub fn total_losses(&self) -> u32 {
        self.losses[0] + self.losses[1]
    }
    /// Total draws.
    pub fn total_draws(&self) -> u32 {
        self.draws[0] + self.draws[1]
    }
}

impl core::fmt::Display for StrengthReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(
            f,
            "strength gate: net {:016x} vs {:?}, {} bouts x {} ticks, seeds {:#x}.., sides swapped every bout",
            self.net_digest, self.opponent, self.bouts, self.bout_ticks, self.seed_base
        )?;
        writeln!(f, "            wins  losses  draws")?;
        for slot in 0..2 {
            writeln!(
                f,
                "as slot {slot}  {:>5}  {:>6}  {:>5}",
                self.wins[slot], self.losses[slot], self.draws[slot]
            )?;
        }
        writeln!(
            f,
            "total      {:>5}  {:>6}  {:>5}",
            self.total_wins(),
            self.total_losses(),
            self.total_draws()
        )?;
        let decisive = self.total_wins() + self.total_losses();
        let share = if decisive == 0 {
            0.5
        } else {
            f64::from(self.total_wins()) / f64::from(decisive)
        };
        let p = if self.p_value < 1e-4 {
            format!("{:.2e}", self.p_value)
        } else {
            format!("{:.4}", self.p_value)
        };
        write!(
            f,
            "decisive win share {share:.4}, Wilson 95% [{:.4}, {:.4}], binomial p = {p} (two-sided vs 0.5)",
            self.wilson95.0, self.wilson95.1
        )
    }
}

/// Strength-gate refusals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StrengthError {
    /// Bout count is zero or above [`MAX_BOUTS`].
    BadBouts(u32),
    /// Student bytes are not a valid `.e2nn` file.
    BadStudent(engine_nn::NnErr),
    /// The arena refused a bout (bad ticks, wrong net shape, rejected run).
    Bout(DuelError),
}

impl core::fmt::Display for StrengthError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StrengthError::BadBouts(n) => write!(f, "bout count {n} outside 1..={MAX_BOUTS}"),
            StrengthError::BadStudent(e) => write!(f, "bad student bytes: {e}"),
            StrengthError::Bout(e) => write!(f, "bout refused: {e}"),
        }
    }
}

impl std::error::Error for StrengthError {}

/// Wilson score 95% interval for `successes` out of `n`; `(0, 1)` for an
/// empty sample. Report math only, never in the tick.
pub fn wilson95(successes: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let n_f = f64::from(n);
    let p = f64::from(successes.min(n)) / n_f;
    let z2 = Z95 * Z95;
    let center = p + z2 / (2.0 * n_f);
    let half = Z95 * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt();
    let denom = 1.0 + z2 / n_f;
    (
        ((center - half) / denom).max(0.0),
        ((center + half) / denom).min(1.0),
    )
}

/// Exact two-sided binomial p-value for `k` successes out of `n` against a
/// fair coin: twice the far tail `P(X >= max(k, n - k))`, capped at 1.
/// `n == 0` gives 1. Log-space sums keep n = 20 000 finite.
pub fn binomial_two_sided_p(k: u32, n: u32) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let k = k.min(n);
    let (Ok(n_idx), Ok(far)) = (usize::try_from(n), usize::try_from(k.max(n - k))) else {
        return 1.0;
    };
    // ln(j!) for j in 0..=n, summed in order.
    let mut ln_fact = Vec::with_capacity(n_idx + 1);
    let (mut acc, mut j) = (0.0f64, 0.0f64);
    ln_fact.push(acc);
    for _ in 0..n_idx {
        j += 1.0;
        acc += j.ln();
        ln_fact.push(acc);
    }
    let ln_half_n = f64::from(n) * core::f64::consts::LN_2;
    let mut tail = 0.0f64;
    for i in far..=n_idx {
        let ln_c = ln_fact[n_idx] - ln_fact[i] - ln_fact[n_idx - i];
        tail += (ln_c - ln_half_n).exp();
    }
    (2.0 * tail).min(1.0)
}

/// Play the strength gate: the student `.e2nn` (6 lanes in, 5 intents out)
/// against `opponent` (the depth brain, actuated like the student or raw)
/// over `bouts` arena bouts of `bout_ticks` ticks, sides swapped every bout
/// (see [`StrengthReport`]). Deterministic: the same bytes, opponent, seed
/// base, bout count and length give the same report.
pub fn strength_gate(
    student_bytes: &[u8],
    opponent: Opponent,
    seed_base: u64,
    bouts: u32,
    bout_ticks: u32,
) -> Result<StrengthReport, StrengthError> {
    if bouts == 0 || bouts > MAX_BOUTS {
        return Err(StrengthError::BadBouts(bouts));
    }
    let model = engine_nn::load(student_bytes).map_err(StrengthError::BadStudent)?;
    let mut wins = [0u32; 2];
    let mut losses = [0u32; 2];
    let mut draws = [0u32; 2];
    for bout in 0..bouts {
        let slot = u8::from(bout % 2 == 1);
        let seed = seed_base.wrapping_add(u64::from(bout / 2));
        let outcome =
            play_bout(&model, seed, slot, bout_ticks, opponent).map_err(StrengthError::Bout)?;
        let tally = match outcome {
            BoutOutcome::Win => &mut wins,
            BoutOutcome::Loss => &mut losses,
            BoutOutcome::Draw => &mut draws,
        };
        tally[usize::from(slot)] += 1;
    }
    let won = wins[0] + wins[1];
    let decisive = won + losses[0] + losses[1];
    Ok(StrengthReport {
        net_digest: model.digest(),
        opponent,
        seed_base,
        bouts,
        bout_ticks,
        wins,
        losses,
        draws,
        wilson95: wilson95(won, decisive),
        p_value: binomial_two_sided_p(won, decisive),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::StudentSpec;
    use crate::student::Student;

    /// Tiny SplitMix64 for fixtures (own 10-line copy; the arena fixture
    /// expander stays behind the arena boundary, as in the train tests).
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
    }

    /// Fixed teacher: the same separable 3-class probe over 4 features the
    /// training test distills, so the tournament fixture tells one story from
    /// training through the tournament. Class = first-max-wins argmax.
    const TEACHER: [[f32; 4]; 3] = [
        [1.5, -1.0, 0.5, 0.25],
        [-1.0, 1.5, -0.5, 0.75],
        [0.25, 0.5, 1.5, -1.5],
    ];

    fn teacher_class(features: &[f32]) -> u8 {
        let mut best = 0u8;
        let mut top = f32::NEG_INFINITY;
        for (o, row) in TEACHER.iter().enumerate() {
            let mut acc = 0.0f32;
            for (w, v) in row.iter().zip(features.iter()) {
                acc += w * v;
            }
            if acc > top {
                top = acc;
                best = o as u8;
            }
        }
        best
    }

    /// Fixed near-teacher student: the teacher weights with one lane nudged
    /// (`+0.3` on `w[0][0]`) so near-tie rows flip through the int8 export
    /// while the bulk pattern survives — strong (most rows match) but never
    /// bit-perfect (the ceiling still holds some bouts).
    fn fixture_student_bytes() -> Vec<u8> {
        let spec = StudentSpec::new(4, 3, 0x70u64).expect("fixture spec is legal");
        let mut weights = Vec::with_capacity(12);
        for row in TEACHER.iter() {
            weights.extend_from_slice(row);
        }
        weights[0] += 0.3;
        let student =
            Student::new(spec, weights, vec![0.0; 3], 0).expect("fixture params are legal");
        crate::export::export(&student, &[crate::data::DatasetManifest::fixture()])
            .expect("fixture student exports")
    }

    /// Fixed tournament fixture: `matches` scripts of `per_match` seeded
    /// observations on exact `1/16` steps (so runtime quantization is exact),
    /// labeled by the fixed teacher. Script `i` streams from
    /// `base.wrapping_add(i)`, mirroring the verifier's seed convention.
    fn fixture_scripts(
        base: u64,
        matches: usize,
        per_match: usize,
    ) -> (Vec<MatchScript>, Vec<Vec<u8>>) {
        let mut scripts = Vec::with_capacity(matches);
        let mut teacher = Vec::with_capacity(matches);
        for i in 0..matches {
            let seed = base.wrapping_add(i as u64);
            let mut rng = Rng(seed);
            let mut features = Vec::with_capacity(per_match);
            let mut actions = Vec::with_capacity(per_match);
            for _ in 0..per_match {
                let mut row = Vec::with_capacity(4);
                for _ in 0..4 {
                    let k = (rng.next_u64() % 255) as i32 - 127;
                    row.push((k as f32) / 16.0);
                }
                actions.push(teacher_class(&row));
                features.push(row);
            }
            scripts.push(MatchScript { seed, features });
            teacher.push(actions);
        }
        (scripts, teacher)
    }

    #[test]
    fn mirrors_pin_the_vf1_literals() {
        // Cross-check: these literals mirror `engine-playtest` (which this
        // crate must not import): `reference_arena` pins
        // `BalanceConfig { matches: 20, min_win_permille: 350,
        // max_win_permille: 650, .. }` in `crates/playtest/src/lib.rs` and
        // `winrates::MAX_MATCHES` is 100 in `crates/playtest/src/winrates.rs`.
        assert_eq!(BALANCE_MIN_PERMILLE, 350);
        assert_eq!(BALANCE_MAX_PERMILLE, 650);
        assert_eq!(MAX_MATCHES, 100);
        assert_eq!(DEFAULT_MATCHES, 100);
    }

    #[test]
    fn comparison_names_first_divergence() {
        assert_eq!(
            score_against_teacher(&[0, 1, 2], &[0, 1, 2]),
            Comparison {
                total: 3,
                agree: 3,
                agree_rate_permille: 1000,
                first_divergent_index: None,
            }
        );
        assert_eq!(
            score_against_teacher(&[0, 2, 2], &[0, 1, 2]),
            Comparison {
                total: 3,
                agree: 2,
                agree_rate_permille: 666,
                first_divergent_index: Some(1),
            }
        );
        // Empty scripts agree vacuously, mirroring the arena brain harness.
        assert_eq!(
            score_against_teacher(&[], &[]),
            Comparison {
                total: 0,
                agree: 0,
                agree_rate_permille: 1000,
                first_divergent_index: None,
            }
        );
        // Short decision slices count missing rows as misses, never panic.
        assert_eq!(
            score_against_teacher(&[0], &[0, 1]),
            Comparison {
                total: 2,
                agree: 1,
                agree_rate_permille: 500,
                first_divergent_index: Some(1),
            }
        );
    }

    #[test]
    fn scripted_stand_in_is_deterministic_and_total() {
        assert_eq!(scripted_decide(&[0, 0], 2), 0);
        assert_eq!(scripted_decide(&[1, 0], 2), 1);
        // Negative sums wrap deterministically instead of panicking.
        assert_eq!(scripted_decide(&[-1, 0], 2), 1);
        assert_eq!(scripted_decide(&[], 0), 0);
        for classes in [1usize, 2, 3, 5, 256] {
            let class = scripted_decide(&[112, 127, -128, 3], classes);
            assert!(usize::from(class) < classes, "class {class} < {classes}");
            assert_eq!(class, scripted_decide(&[112, 127, -128, 3], classes));
        }
    }

    #[test]
    fn bad_inputs_rejected_with_cause() {
        let bytes = fixture_student_bytes();
        let (scripts, teacher) = fixture_scripts(0xF100, 2, 4);
        // Corrupt bytes name the runtime cause.
        assert_eq!(
            tournament(&[], &teacher, &scripts, 2),
            Err(TournamentError::BadStudent(engine_nn::NnErr::Truncated))
        );
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'Z';
        assert_eq!(
            tournament(&bad_magic, &teacher, &scripts, 2),
            Err(TournamentError::BadStudent(engine_nn::NnErr::BadMagic))
        );
        // Match-count envelope.
        assert_eq!(
            tournament(&bytes, &teacher, &scripts, 0),
            Err(TournamentError::BadMatches(0))
        );
        assert_eq!(
            tournament(&bytes, &teacher, &scripts, MAX_MATCHES + 1),
            Err(TournamentError::BadMatches(MAX_MATCHES + 1))
        );
        // Short inputs name what is missing.
        assert_eq!(
            tournament(&bytes, &teacher, &scripts[..1], 2),
            Err(TournamentError::NotEnoughScripts { have: 1, want: 2 })
        );
        assert_eq!(
            tournament(&bytes, &teacher[..1], &scripts, 2),
            Err(TournamentError::NotEnoughTeacher { have: 1, want: 2 })
        );
        // Wrong-dim students fail before any row simulates.
        let narrow_spec = StudentSpec::new(2, 2, 1).expect("legal spec");
        let narrow = crate::export::export(
            &Student::zeros(narrow_spec),
            &[crate::data::DatasetManifest::fixture()],
        )
        .expect("zeros export");
        assert_eq!(
            tournament(&narrow, &teacher, &scripts, 1),
            Err(TournamentError::BadFeatureLen {
                index: 0,
                row: 0,
                got: 4,
                want: 2,
            })
        );
        // Row faults name match and row.
        let mut wide = scripts.clone();
        wide[1].features[2] = vec![0.0; 9];
        assert_eq!(
            tournament(&bytes, &teacher, &wide, 2),
            Err(TournamentError::BadFeatureLen {
                index: 1,
                row: 2,
                got: 9,
                want: 4,
            })
        );
        let mut nasty = scripts.clone();
        nasty[0].features[1][3] = f32::NAN;
        assert_eq!(
            tournament(&bytes, &teacher, &nasty, 1),
            Err(TournamentError::BadFeature { index: 0, row: 1 })
        );
        let mut wild = scripts.clone();
        wild[0].features[0][0] = 8.5;
        assert_eq!(
            tournament(&bytes, &teacher, &wild, 1),
            Err(TournamentError::BadFeature { index: 0, row: 0 })
        );
        let mut short_teacher = teacher.clone();
        short_teacher[0].pop();
        assert_eq!(
            tournament(&bytes, &short_teacher, &scripts, 1),
            Err(TournamentError::RowMismatch {
                index: 0,
                features: 4,
                teacher: 3,
            })
        );
        let mut bad_class = teacher.clone();
        bad_class[1][0] = 7;
        assert_eq!(
            tournament(&bytes, &bad_class, &scripts, 2),
            Err(TournamentError::BadTeacherClass {
                index: 1,
                row: 0,
                got: 7,
                cap: 3,
            })
        );
        let empty = vec![
            MatchScript {
                seed: 1,
                features: Vec::new(),
            },
            scripts[1].clone(),
        ];
        let empty_teacher = vec![Vec::new(), teacher[1].clone()];
        assert_eq!(
            tournament(&bytes, &empty_teacher, &empty, 1),
            Err(TournamentError::EmptyScript { index: 0 })
        );
        // Wide nets name the class cap instead of truncating teacher ids.
        let wide_spec = StudentSpec::new(1, 300, 2).expect("legal spec");
        let wide_bytes = crate::export::export(
            &Student::zeros(wide_spec),
            &[crate::data::DatasetManifest::fixture()],
        )
        .expect("wide zeros export");
        let one_wide = vec![MatchScript {
            seed: 3,
            features: vec![vec![0.0]],
        }];
        let one_teacher = vec![vec![0u8]];
        assert_eq!(
            tournament(&wide_bytes, &one_teacher, &one_wide, 1),
            Err(TournamentError::BadClassCap(300))
        );
    }

    #[test]
    fn tiny_tournament_scores_exact_bouts() {
        // Zero student: every logit ties at 0, first-max-wins always picks
        // class 0. Hand-computed below, so the bout rules are pinned.
        let spec = StudentSpec::new(2, 2, 4).expect("legal spec");
        let bytes = crate::export::export(
            &Student::zeros(spec),
            &[crate::data::DatasetManifest::fixture()],
        )
        .expect("zeros export");
        let scripts = vec![
            MatchScript {
                seed: 11,
                features: vec![vec![0.0, 0.0], vec![1.0, 1.0]],
            },
            MatchScript {
                seed: 12,
                features: vec![vec![7.0, 8.0]],
            },
        ];
        // Match 0: student always 0 (1/2 rows); scripted sums to 0 then 32,
        // both even, so also class 0 (1/2 rows). Challenger bout draws;
        // neither equals the teacher, so the teacher banks both ceiling
        // bouts: student 0, scripted 0, teacher 2.
        // Match 1: teacher 1; student 0 (miss); scripted quantizes to
        // [112, 127], sums to 239, odd, so class 1 (perfect). Challenger
        // bout goes scripted, ceiling bouts split scripted/teacher:
        // student 0, scripted 2, teacher 1.
        let teacher = vec![vec![0u8, 1u8], vec![1u8]];
        let report = tournament(&bytes, &teacher, &scripts, 2).expect("tiny fixture plays");
        assert_eq!(
            report,
            TournamentReport {
                student_wins: 0,
                scripted_wins: 2,
                teacher_wins: 3,
                matches: 2,
                observations: 3,
                student_rate_permille: 0,
                within_bounds: false,
                seeds: vec![11, 12],
            }
        );
        // Same inputs twice give the same report (replay bit-exactly).
        assert_eq!(tournament(&bytes, &teacher, &scripts, 2), Ok(report));
    }

    #[test]
    fn fixed_fixture_student_matches_scripted_over_100() {
        let bytes = fixture_student_bytes();
        let (scripts, teacher) = fixture_scripts(0xD7300, DEFAULT_MATCHES as usize, 12);
        // The recorded fixture exercises every teacher action.
        let mut class_counts = [0usize; 3];
        for actions in teacher.iter() {
            for action in actions.iter() {
                class_counts[usize::from(*action)] += 1;
            }
        }
        assert!(
            class_counts.iter().all(|count| *count >= 50),
            "fixture must exercise every class: {class_counts:?}"
        );
        let report =
            tournament(&bytes, &teacher, &scripts, DEFAULT_MATCHES).expect("fixture plays");
        assert_eq!(report.matches, DEFAULT_MATCHES);
        assert_eq!(report.observations, DEFAULT_MATCHES * 12);
        assert_eq!(report.seeds.len(), DEFAULT_MATCHES as usize);
        assert_eq!(
            report.seeds[0], 0xD7300,
            "first seed opens the recorded range"
        );
        assert_eq!(
            report.seeds[DEFAULT_MATCHES as usize - 1],
            0xD7300u64.wrapping_add(u64::from(DEFAULT_MATCHES - 1)),
            "last seed closes the recorded range"
        );
        // The ship gate: the student wins at least as often as the scripted
        // stand-in, and the bout rate sits inside the verifier's balance bounds.
        assert!(
            report.student_wins >= report.scripted_wins,
            "student must at least match scripted: {report:?}"
        );
        assert!(
            report.within_bounds,
            "fixture rate must hold bounds: {report:?}"
        );
        assert!(
            (BALANCE_MIN_PERMILLE..=BALANCE_MAX_PERMILLE).contains(&report.student_rate_permille),
            "rate must read inside the mirrored bounds: {report:?}"
        );
        assert!(
            report.teacher_wins > 0,
            "ceiling must hold bouts: {report:?}"
        );
        // Same fixture twice replays bit-exactly (the neural-policy proof tail).
        assert_eq!(
            tournament(&bytes, &teacher, &scripts, DEFAULT_MATCHES),
            Ok(report)
        );
    }

    #[test]
    fn scripted_baseline_stays_weak_on_the_fixture() {
        // The gate above is only meaningful when the stand-in does not carry
        // teacher knowledge itself: on the fixed fixture its agreement must
        // sit far below the student's.
        let bytes = fixture_student_bytes();
        let model = engine_nn::load(&bytes).expect("fixture bytes load");
        let (scripts, teacher) = fixture_scripts(0xD7300, 10, 12);
        let mut student_agree = 0usize;
        let mut scripted_agree = 0usize;
        let mut total = 0usize;
        for (script, actions) in scripts.iter().zip(teacher.iter()) {
            for (features, want) in script.features.iter().zip(actions.iter()) {
                let quantized = quantize_input(features);
                let student = student_class(&model, &quantized).expect("logits load");
                total += 1;
                if student == *want {
                    student_agree += 1;
                }
                if scripted_decide(&quantized, 3) == *want {
                    scripted_agree += 1;
                }
            }
        }
        assert_eq!(total, 120, "ten scripts of twelve rows");
        assert!(
            student_agree * 1000 / total >= 700,
            "student must carry the teacher: {student_agree}/{total}"
        );
        assert!(
            scripted_agree * 1000 / total <= 600,
            "stand-in must stay weak: {scripted_agree}/{total}"
        );
        assert!(
            student_agree > scripted_agree,
            "student must out-agree the stand-in: {student_agree} vs {scripted_agree}"
        );
    }

    #[cfg(feature = "train")]
    #[test]
    fn trained_student_plays_a_tiny_tournament() {
        use crate::data::Sample;
        use crate::train::train;

        // Full train -> tournament flow on a small slice: the fixed teacher labels a
        // tiny recorded dataset, `train` fits it, `export` quantizes it, and
        // the tournament scores it against the stand-in. Match counts stay
        // tiny here; the full-100 gate lives in the default suite above.
        let spec = StudentSpec::new(4, 3, 0x7EA1).expect("legal spec");
        let mut rng = Rng(0xD71E_5EED_2026_0923);
        let mut dataset = Vec::new();
        for _ in 0..48 {
            let mut features = Vec::with_capacity(4);
            for _ in 0..4 {
                let k = (rng.next_u64() % 255) as i32 - 127;
                features.push((k as f32) / 16.0);
            }
            let target = teacher_class(&features);
            dataset.push(Sample::new(features, target, &spec).expect("fixture rows are legal"));
        }
        let manifest = crate::data::DatasetManifest::fixture();
        let student = train(&dataset, &spec, 1000, Some(&manifest)).expect("fixture trains");
        let bytes = crate::export::export(&student, &[manifest]).expect("trained student exports");
        let (scripts, teacher) = fixture_scripts(0x7EED, 5, 8);
        let report = tournament(&bytes, &teacher, &scripts, 5).expect("tiny bracket plays");
        assert_eq!(report.matches, 5);
        assert_eq!(report.seeds.len(), 5);
        assert!(
            report.student_wins >= report.scripted_wins,
            "trained student must at least match scripted: {report:?}"
        );
        assert_eq!(tournament(&bytes, &teacher, &scripts, 5), Ok(report));
    }

    /// Teacher rows the QAT trainer fits: flat features plus class ids drawn
    /// from the same fixed teacher on a seed range apart from the tournament's.
    fn qat_training_rows() -> (Vec<f32>, Vec<u8>) {
        let (scripts, teacher) = fixture_scripts(0x9A7, 20, 12);
        let features = scripts
            .iter()
            .flat_map(|s| s.features.iter().flatten().copied())
            .collect();
        (features, teacher.concat())
    }

    /// Rows where the exported net picks the recorded teacher action.
    fn teacher_agreement(bytes: &[u8], scripts: &[MatchScript], teacher: &[Vec<u8>]) -> usize {
        let model = engine_nn::load(bytes).expect("bytes load");
        let mut agree = 0usize;
        for (script, actions) in scripts.iter().zip(teacher.iter()) {
            for (features, want) in script.features.iter().zip(actions.iter()) {
                let quantized = quantize_input(features);
                if student_class(&model, &quantized) == Some(*want) {
                    agree += 1;
                }
            }
        }
        agree
    }

    #[test]
    fn qat_student_beats_post_training_quantization_on_the_tournament() {
        use engine_nn::{export_qat, train_student, QatConfig};

        let (features, targets) = qat_training_rows();
        let ptq = QatConfig::new(4, 3, 0x51, 600, false).expect("legal");
        let qat = QatConfig::new(4, 3, 0x51, 600, true).expect("legal");
        let ptq_bytes = export_qat(
            &train_student(&features, &targets, &ptq).expect("trains"),
            4,
            3,
        )
        .expect("exports");
        let qat_bytes = export_qat(
            &train_student(&features, &targets, &qat).expect("trains"),
            4,
            3,
        )
        .expect("exports");
        let (scripts, teacher) = fixture_scripts(0xD7300, DEFAULT_MATCHES as usize, 12);
        let ptq_agree = teacher_agreement(&ptq_bytes, &scripts, &teacher);
        let qat_agree = teacher_agreement(&qat_bytes, &scripts, &teacher);
        let qat_report =
            tournament(&qat_bytes, &teacher, &scripts, DEFAULT_MATCHES).expect("plays");
        let ptq_report =
            tournament(&ptq_bytes, &teacher, &scripts, DEFAULT_MATCHES).expect("plays");
        // 1200 scored rows: the QAT student carries more of the teacher
        // through the int8 export and banks more bouts than the same
        // recipe quantized only after training.
        assert!(
            qat_agree > ptq_agree,
            "qat {qat_agree} must out-agree ptq {ptq_agree} of 1200"
        );
        assert!(
            qat_report.student_wins > ptq_report.student_wins,
            "qat {qat_report:?} must out-win ptq {ptq_report:?}"
        );
        assert!(qat_report.student_wins >= qat_report.scripted_wins);
    }

    #[test]
    fn binomial_p_matches_the_book_example() {
        // gogame 10.3.2: 53 wins in 100 fair games is p ~ 0.62.
        let p = binomial_two_sided_p(53, 100);
        assert!((p - 0.6173).abs() < 1e-3, "p = {p}");
        assert_eq!(binomial_two_sided_p(47, 100), p, "symmetric");
        assert_eq!(binomial_two_sided_p(50, 100), 1.0);
        assert_eq!(binomial_two_sided_p(0, 0), 1.0);
        // 10 of 10: 2 / 1024.
        assert!((binomial_two_sided_p(10, 10) - 2.0 / 1024.0).abs() < 1e-12);
        // 550 of 1000 is significant at 1%, 520 of 1000 is not at 5%.
        assert!(binomial_two_sided_p(550, 1000) < 0.01);
        assert!(binomial_two_sided_p(520, 1000) > 0.05);
        assert!(binomial_two_sided_p(10_000, 20_000) <= 1.0);
    }

    #[test]
    fn wilson_interval_brackets_the_rate() {
        let (lo, hi) = wilson95(53, 100);
        assert!(
            (lo - 0.4328).abs() < 1e-3 && (hi - 0.6248).abs() < 1e-3,
            "{lo} {hi}"
        );
        let (lo, hi) = wilson95(0, 10);
        assert_eq!(lo, 0.0);
        assert!(hi > 0.25 && hi < 0.35, "{hi}");
        let (lo, hi) = wilson95(10, 10);
        assert!(lo > 0.65 && hi > 0.999_999 && hi <= 1.0, "{lo} {hi}");
        assert_eq!(wilson95(0, 0), (0.0, 1.0));
    }

    /// Fixed 6 -> 5 net that always attacks (bias only).
    fn always_attack_bytes() -> Vec<u8> {
        let spec = StudentSpec::new(6, 5, 0).expect("fighter shape is legal");
        let mut student = Student::zeros(spec);
        student.bias[0] = 1.0;
        crate::export::export(&student, &[crate::data::DatasetManifest::fixture()])
            .expect("exports")
    }

    #[test]
    fn strength_gate_swaps_sides_and_repeats_exactly() {
        let bytes = always_attack_bytes();
        let first =
            strength_gate(&bytes, Opponent::BrainIntents, 0x5EED, 4, 240).expect("short gate runs");
        let second = strength_gate(&bytes, Opponent::BrainIntents, 0x5EED, 4, 240)
            .expect("short gate runs again");
        assert_eq!(first, second, "same seeds give the same table");
        assert_eq!(first.to_string(), second.to_string());
        assert_eq!(first.bouts, 4);
        assert_eq!(first.net_digest, engine_core::hash::hash_bytes(&bytes));
        // Two bouts per side.
        for slot in 0..2 {
            assert_eq!(first.wins[slot] + first.losses[slot] + first.draws[slot], 2);
        }
        let table = first.to_string();
        assert!(
            table.contains("as slot 0") && table.contains("binomial p"),
            "{table}"
        );
    }

    #[test]
    fn strength_gate_refuses_bad_inputs() {
        let bytes = always_attack_bytes();
        assert_eq!(
            strength_gate(&bytes, Opponent::ArenaBrain, 1, 0, 60),
            Err(StrengthError::BadBouts(0))
        );
        assert_eq!(
            strength_gate(&bytes, Opponent::ArenaBrain, 1, MAX_BOUTS + 1, 60),
            Err(StrengthError::BadBouts(MAX_BOUTS + 1))
        );
        assert_eq!(
            strength_gate(&[1, 2, 3], Opponent::ArenaBrain, 1, 2, 60),
            Err(StrengthError::BadStudent(engine_nn::NnErr::Truncated))
        );
        match strength_gate(&bytes, Opponent::ArenaBrain, 1, 2, 0) {
            Err(StrengthError::Bout(DuelError::BadConfig(_))) => {}
            other => panic!("zero ticks must be refused, got {other:?}"),
        }
    }
}
