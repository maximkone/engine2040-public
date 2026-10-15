//! director: auto spectator camera that cuts between presentation shots.
//!
//! Consumer: the game session holds one `Director` and drives it from real
//! fight cues, fighter laterals and health once per accepted tick; the
//! headless capture files the cut-log digest beside its frames. Data owner:
//! this module owns shot choice, the held side of the action line and the cut
//! log. Units: fixed ticks at the arena rate, lateral meters on the camera
//! axis, editorial weights in permille. Determinism: the same config plus the
//! same per-tick inputs give the same shots and cuts twice; presentation
//! only, never hashed. Errors: `DirectorError` at every boundary. Budget:
//! 0.05 ms per presented tick with no heap growth per tick after
//! construction (both proved by tests in this file).

use engine_audio::FightCue;
use engine_core::{Fnv64, Rng};
use engine_input::MAX_PARTICIPANTS;
use engine_spatial::compose::best_shot;

/// Fixed ticks per second: the hold floor below is one second of screen time.
/// Shortest held shot in ticks: cuts never land faster than this.
pub const MIN_SHOT_TICKS: u32 = 60;
/// Mean budget per presented tick in microseconds (the 0.05 ms rail).
pub const DIRECTOR_BUDGET_MICROS: u64 = 50;
/// Most slew steps one cut may travel along the framing rail.
pub const MAX_SLEW_STEPS: u8 = 2;
/// Lightest editorial weight that earns a cut (a glancing hit).
pub const CUT_MIN_WEIGHT: u32 = 500;
/// Beat older than this at cut time audits as stale (one hold floor).
pub const STALE_BEAT_TICKS: u32 = 60;
/// Cut-log slack past the one-cut-per-hold bound, so steady ticks never grow.
pub const LOG_SLACK_CUTS: u32 = 8;

/// Base weight of a landed hit; damage folds in on top, capped below.
pub const HIT_WEIGHT: u32 = 500;
/// Damage divisor folded into hit weight (`damage / 2`).
pub const HIT_DAMAGE_DIV: u32 = 2;
/// Heaviest plain hit; louder moments map to the comeback beat instead.
pub const HIT_WEIGHT_MAX: u32 = 800;
/// Base weight of a soaked block.
pub const BLOCK_WEIGHT: u32 = 300;
/// Chip divisor folded into block weight (`chip / 4`).
pub const BLOCK_CHIP_DIV: u32 = 4;
/// Heaviest block: blocks stay below the cut floor, never cut on their own.
pub const BLOCK_WEIGHT_MAX: u32 = 450;
/// Weight of a parry (an exchange worth reframing).
pub const PARRY_WEIGHT: u32 = 700;
/// Weight of a clean dodge (noted, never cut on its own).
pub const DODGE_WEIGHT: u32 = 200;
/// Weight of a knockdown (the heaviest beat on the card).
pub const KNOCKDOWN_WEIGHT: u32 = 1000;
/// Weight of a hurt fighter landing a loud hit.
pub const COMEBACK_WEIGHT: u32 = 900;
/// Weight of a crowd-swell beat (a cutaway, never the line).
pub const CROWD_WEIGHT: u32 = 600;
/// Damage at or above which a hit counts as loud, in permille.
pub const LOUD_DAMAGE: u16 = 120;
/// Health at or below which the attacker counts as the hurt fighter.
pub const COMEBACK_HP_MAX: u32 = 300;
/// Crowd-bed duck depth (`1 - bed_gain`) that counts as a swell.
pub const CROWD_SWELL_DEPTH: f32 = 0.6;

/// Director configuration: who to open on, how long to hold, how far to log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectorConfig {
    /// Session seed: draws the knockdown framing alternate, nothing else.
    pub seed: u64,
    /// Followed fighter slot, below `MAX_PARTICIPANTS`.
    pub focus: u8,
    /// Shortest held shot in ticks, `1..=max_ticks`.
    pub min_shot_ticks: u32,
    /// Longest session this director serves, `1..=MAX_SESSION_TICKS`.
    pub max_ticks: u32,
}

/// Longest session the director serves (mirrors the game session bound).
pub const MAX_SESSION_TICKS: u32 = 100_000;

impl DirectorConfig {
    /// Range-check every row; the culprit row name is carried.
    ///
    /// # Errors
    ///
    /// Fails with `DirectorError::BadConfig` naming the offending row.
    pub fn validate(self) -> Result<(), DirectorError> {
        if self.focus >= MAX_PARTICIPANTS {
            return Err(DirectorError::BadConfig("focus"));
        }
        if self.min_shot_ticks == 0 || self.min_shot_ticks > MAX_SESSION_TICKS {
            return Err(DirectorError::BadConfig("min_shot_ticks"));
        }
        if self.max_ticks == 0 || self.max_ticks > MAX_SESSION_TICKS {
            return Err(DirectorError::BadConfig("max_ticks"));
        }
        if self.min_shot_ticks > self.max_ticks {
            return Err(DirectorError::BadConfig("min_shot_ticks"));
        }
        Ok(())
    }
}

/// Director failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectorError {
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Ticks run forward only: `next` arrived after `last` was served.
    TickWentBackwards {
        /// Last tick served.
        last: u32,
        /// Rejected tick.
        next: u32,
    },
    /// Lateral position not finite, fighter id carried.
    BadLateral(u8),
}

impl core::fmt::Display for DirectorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadConfig(row) => write!(f, "bad director config ({row})"),
            Self::TickWentBackwards { last, next } => {
                write!(f, "director tick went backwards ({next} after {last})")
            }
            Self::BadLateral(id) => write!(f, "bad lateral for fighter ({id})"),
        }
    }
}

impl std::error::Error for DirectorError {}

/// One editorial beat folded from a tick: what happened, who to frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Beat {
    /// Fixed tick the beat landed on.
    pub tick: u32,
    /// What happened.
    pub kind: BeatKind,
    /// Fighter slot to frame.
    pub focus: u8,
    /// Editorial weight in permille (highest wins the tick).
    pub weight: u32,
}

/// Editorial beat kinds, one per cue shape plus comeback and crowd.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BeatKind {
    /// Full damage landed.
    Hit,
    /// Held block soaked the hit down to chip.
    Block,
    /// Block inside the parry window.
    Parry,
    /// Dodge negated the hit.
    Dodge,
    /// Victim dropped to zero.
    Knockdown,
    /// Hurt fighter landed a loud hit.
    Comeback,
    /// Crowd energy peaked.
    Crowd,
}

/// Post-tick health of one slot, or full health when the slot is unknown.
/// Unknown slots never trigger comebacks; they fail closed, never loudly.
fn hp_of(hp_permille: &[u16], slot: u8) -> u32 {
    match hp_permille.get(usize::from(slot)) {
        Some(hp) => u32::from(*hp),
        None => u32::from(u16::MAX),
    }
}

/// Map one fight cue onto its editorial beat. A loud hit from the hurt
/// fighter maps to a comeback instead of a plain hit; stamina whiffs map to
/// nothing (no contact, no cut). Pure over the inputs.
fn beat_for_cue(cue: FightCue, tick: u32, hp_permille: &[u16]) -> Option<Beat> {
    match cue {
        FightCue::Hit {
            attacker,
            victim,
            damage,
        } => {
            if u32::from(damage) >= u32::from(LOUD_DAMAGE)
                && hp_of(hp_permille, attacker) <= COMEBACK_HP_MAX
            {
                Some(Beat {
                    tick,
                    kind: BeatKind::Comeback,
                    focus: attacker,
                    weight: COMEBACK_WEIGHT,
                })
            } else {
                Some(Beat {
                    tick,
                    kind: BeatKind::Hit,
                    focus: victim,
                    weight: HIT_WEIGHT
                        .saturating_add(u32::from(damage) / HIT_DAMAGE_DIV)
                        .min(HIT_WEIGHT_MAX),
                })
            }
        }
        FightCue::Blocked { attacker, chip, .. } => Some(Beat {
            tick,
            kind: BeatKind::Block,
            focus: attacker,
            weight: BLOCK_WEIGHT
                .saturating_add(u32::from(chip) / BLOCK_CHIP_DIV)
                .min(BLOCK_WEIGHT_MAX),
        }),
        FightCue::Parried { victim, .. } => Some(Beat {
            tick,
            kind: BeatKind::Parry,
            focus: victim,
            weight: PARRY_WEIGHT,
        }),
        FightCue::Dodged { victim, .. } => Some(Beat {
            tick,
            kind: BeatKind::Dodge,
            focus: victim,
            weight: DODGE_WEIGHT,
        }),
        FightCue::Knockdown { victim, .. } => Some(Beat {
            tick,
            kind: BeatKind::Knockdown,
            focus: victim,
            weight: KNOCKDOWN_WEIGHT,
        }),
        FightCue::StaminaOut { .. } => None,
    }
}

/// Hero-focus pick over pre-scored candidate slots: the shared compose rank
/// owner chooses, this adapter keeps the game slot domain. Out-of-range ids
/// and an empty slate pick nothing (fail closed: a corrupt slate holds the
/// shot rather than framing a wrong hero).
///
/// Units: slot ids in, slot id out; totals are editorial permille weights.
/// Determinism: highest total wins, lowest slot on ties, in any slate order.
#[must_use]
pub fn pick_hero_focus(slate: &[(u32, u32)]) -> Option<u8> {
    let id = best_shot(slate)?;
    u8::try_from(id)
        .ok()
        .filter(|slot| *slot < MAX_PARTICIPANTS)
}

/// Strongest beat of one tick over cues plus crowd energy: contact outranks
/// beds, so a quiet room with a loud hit still cuts to the hit. Editorial
/// weight folds per fighter slot on the stack (bounded by the participant
/// cap, so any cue count folds with no heap), then the shared compose rank
/// owner picks the hero focus: highest weight wins, lowest slot on ties, in
/// any cue order. A fighter pick displaces the crowd swell only when strictly
/// heavier, so ties hold the room. A non-finite bed gain reads as calm (no
/// crowd beat); everything else about the fold is total. Cue slots outside
/// the participant cap never frame (fail closed: corrupt ids hold the shot).
fn select_peak(cues: &[FightCue], tick: u32, hp_permille: &[u16], bed_gain: f32) -> Option<Beat> {
    let depth = 1.0 - bed_gain;
    let crowd: Option<Beat> = if depth.is_finite() && depth >= CROWD_SWELL_DEPTH {
        Some(Beat {
            tick,
            kind: BeatKind::Crowd,
            focus: 0,
            weight: CROWD_WEIGHT,
        })
    } else {
        None
    };
    // Scratch bound for one entry per fighter slot: a literal (never a
    // numeric cast) pinned to the participant cap at compile time, so a
    // cap change fails loudly here instead of mis-sizing the fold.
    const SLOTS: usize = 16;
    const _: () = assert!(MAX_PARTICIPANTS == 16);
    let mut slot_total = [0u32; SLOTS];
    let mut slot_kind: [Option<BeatKind>; SLOTS] = [None; SLOTS];
    for cue in cues {
        let Some(beat) = beat_for_cue(*cue, tick, hp_permille) else {
            continue;
        };
        let index = usize::from(beat.focus);
        if index < slot_total.len() && beat.weight > slot_total[index] {
            slot_total[index] = beat.weight;
            slot_kind[index] = Some(beat.kind);
        }
    }
    let mut slate = [(0u32, 0u32); SLOTS];
    let mut count = 0usize;
    for slot in 0..MAX_PARTICIPANTS {
        let total = slot_total[usize::from(slot)];
        if total > 0 {
            slate[count] = (u32::from(slot), total);
            count += 1;
        }
    }
    let Some(slot) = pick_hero_focus(&slate[..count]) else {
        return crowd;
    };
    let index = usize::from(slot);
    // The slate holds exactly the slots scored above, so the winner carries
    // its recorded kind; the crowd holds on the unreachable miss instead of
    // panicking (presentation never fails loudly past validation).
    let Some(kind) = slot_kind[index] else {
        return crowd;
    };
    let fighter = Beat {
        tick,
        kind,
        focus: slot,
        weight: slot_total[index],
    };
    match crowd {
        Some(held) if fighter.weight <= held.weight => Some(held),
        _ => Some(fighter),
    }
}

/// Presentation shot framings the director cuts between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShotKind {
    /// Both fighters plus room (openers, resets).
    Wide,
    /// Both fighters tight (exchanges).
    Duel,
    /// One fighter large (hits, comebacks).
    Close,
    /// Low angle on the fallen fighter.
    Low,
    /// High angle on the fallen fighter (seeded alternate to low).
    High,
    /// Cutaway to the room (swells only, never crosses the line).
    Crowd,
}

/// Discriminant of a framing for the digest (order is the contract).
fn shot_index(kind: ShotKind) -> u8 {
    match kind {
        ShotKind::Wide => 0,
        ShotKind::Duel => 1,
        ShotKind::Close => 2,
        ShotKind::Low => 3,
        ShotKind::High => 4,
        ShotKind::Crowd => 5,
    }
}

/// Held side of the action line (the one-hundred-eighty-degree rule).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShotSide {
    /// First side (the cold-open side before any geometry arrives).
    Left,
    /// Second side.
    Right,
}

impl ShotSide {
    /// Discriminant for the digest.
    fn index(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

/// One held presentation shot: framing, focus and line side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shot {
    /// Framing served this tick.
    pub kind: ShotKind,
    /// Framed fighter slot.
    pub focus: u8,
    /// Held side of the action line.
    pub side: ShotSide,
    /// Fixed tick the shot cut in on.
    pub start_tick: u32,
}

/// Why a cut fired: the peak beat that earned it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CutReason {
    /// A landed hit peaked.
    Hit,
    /// A parry peaked.
    Parry,
    /// A knockdown peaked.
    Knockdown,
    /// A hurt fighter landed loud.
    Comeback,
    /// The room swelled.
    Crowd,
}

/// Discriminant of a cut reason for the digest (order is the contract).
fn reason_index(reason: CutReason) -> u8 {
    match reason {
        CutReason::Hit => 0,
        CutReason::Parry => 1,
        CutReason::Knockdown => 2,
        CutReason::Comeback => 3,
        CutReason::Crowd => 4,
    }
}

/// Map a peak beat onto its cut reason. Blocks, dodges and whiffs earn no
/// reason: they hold the shot, so silence at a hold boundary never cuts.
fn reason_for_beat(kind: BeatKind) -> Option<CutReason> {
    match kind {
        BeatKind::Hit => Some(CutReason::Hit),
        BeatKind::Parry => Some(CutReason::Parry),
        BeatKind::Knockdown => Some(CutReason::Knockdown),
        BeatKind::Comeback => Some(CutReason::Comeback),
        BeatKind::Crowd => Some(CutReason::Crowd),
        BeatKind::Block | BeatKind::Dodge => None,
    }
}

/// Framing for a cut reason. Knockdowns alternate low and high on the seeded
/// stream so back-to-back falls do not jump-cut on each other.
fn framing_for_reason(reason: CutReason, knock_high: bool) -> ShotKind {
    match reason {
        CutReason::Hit | CutReason::Comeback => ShotKind::Close,
        CutReason::Parry => ShotKind::Duel,
        CutReason::Knockdown => {
            if knock_high {
                ShotKind::High
            } else {
                ShotKind::Low
            }
        }
        CutReason::Crowd => ShotKind::Crowd,
    }
}

/// Rail order of a framing on the slew line (crowd rides apart).
fn slew_rail(kind: ShotKind) -> u8 {
    match kind {
        ShotKind::Crowd => 0,
        ShotKind::Wide => 1,
        ShotKind::Duel => 2,
        ShotKind::Close => 3,
        ShotKind::Low => 4,
        ShotKind::High => 5,
    }
}

/// Slew steps between two framings. Crowd cutaways ride apart from the rail
/// (they return to the same side), so they never count as slew. Pure.
#[must_use]
pub fn slew_steps(from: ShotKind, to: ShotKind) -> u8 {
    if from == ShotKind::Crowd || to == ShotKind::Crowd {
        return 0;
    }
    let low = slew_rail(from).min(slew_rail(to));
    let high = slew_rail(from).max(slew_rail(to));
    high.saturating_sub(low)
}

/// One fighter's lateral position on the camera axis for a tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FighterLateral {
    /// Fighter slot id.
    pub id: u8,
    /// Lateral position in meters along the camera axis.
    pub x_m: f32,
}

/// Held side from lateral geometry: the order of the two lowest-id slots
/// along the camera axis. Exact ties read as `Left` (deterministic under
/// any input order, since ids pick the pair, never container order).
/// Fewer than two laterals keep the held side (no geometry, no flip).
///
/// # Errors
///
/// Fails with `DirectorError::BadLateral` naming the first non-finite slot.
fn side_of(laterals: &[FighterLateral], keep: ShotSide) -> Result<ShotSide, DirectorError> {
    for lateral in laterals {
        if !lateral.x_m.is_finite() {
            return Err(DirectorError::BadLateral(lateral.id));
        }
    }
    let mut first: Option<&FighterLateral> = None;
    let mut second: Option<&FighterLateral> = None;
    for lateral in laterals {
        match (first, second) {
            (None, _) => first = Some(lateral),
            (Some(held), _) if lateral.id < held.id => {
                second = first;
                first = Some(lateral);
            }
            (_, None) => second = Some(lateral),
            (Some(_), Some(held)) if lateral.id < held.id => second = Some(lateral),
            _ => {}
        }
    }
    match (first, second) {
        (Some(low), Some(high)) => Ok(if low.x_m <= high.x_m {
            ShotSide::Left
        } else {
            ShotSide::Right
        }),
        _ => Ok(keep),
    }
}

/// One editorial cut: the framings, the line side and the peak behind it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cut {
    /// Fixed tick the cut fired on.
    pub tick: u32,
    /// Framing held before the cut.
    pub from_kind: ShotKind,
    /// Framing served after the cut.
    pub to_kind: ShotKind,
    /// Focus held before the cut.
    pub from_focus: u8,
    /// Focus served after the cut.
    pub to_focus: u8,
    /// Line side after the cut.
    pub side: ShotSide,
    /// True when the line side differs from the previous cut (the camera
    /// crossed the line between cuts).
    pub crossed_line: bool,
    /// Peak beat that earned the cut.
    pub reason: CutReason,
    /// Fixed tick the peak beat landed on (always the cut tick: peaks
    /// during a hold are absorbed, never queued).
    pub beat_tick: u32,
}

/// Continuity audit over a cut log: jump cuts, line crosses, short holds,
/// slew overs and stale beats each flag loudly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AuditReport {
    /// Cuts audited.
    pub cuts: u32,
    /// Cuts onto the same framing and focus (no new information).
    pub jump_cuts: u32,
    /// Cuts that crossed the action line.
    pub line_crosses: u32,
    /// Cuts that landed before the hold floor expired.
    pub short_holds: u32,
    /// Cuts that travelled too far along the framing rail.
    pub slew_overs: u32,
    /// Cuts whose peak beat landed too long before the cut.
    pub stale_cuts: u32,
}

impl AuditReport {
    /// True when no flag fired.
    #[must_use]
    pub fn is_clean(self) -> bool {
        self.total_flags() == 0
    }

    /// Every flag added up (saturating; the log is bounded).
    #[must_use]
    pub fn total_flags(self) -> u32 {
        self.jump_cuts
            .saturating_add(self.line_crosses)
            .saturating_add(self.short_holds)
            .saturating_add(self.slew_overs)
            .saturating_add(self.stale_cuts)
    }
}

/// Audit one cut log against the hold floor: same framing plus same focus is
/// a jump, a flipped side is a line cross, a short gap breaks the hold, a
/// long rail trip breaks the slew cap, an old peak is stale. The first cut
/// is exempt from the short-hold check (the cold open is editorially free).
/// Pure over the log.
#[must_use]
pub fn audit_cuts(cuts: &[Cut], min_shot_ticks: u32) -> AuditReport {
    let mut report = AuditReport {
        cuts: u32::try_from(cuts.len()).unwrap_or(u32::MAX),
        jump_cuts: 0,
        line_crosses: 0,
        short_holds: 0,
        slew_overs: 0,
        stale_cuts: 0,
    };
    let mut previous: Option<&Cut> = None;
    for cut in cuts {
        if cut.from_kind == cut.to_kind && cut.from_focus == cut.to_focus {
            report.jump_cuts = report.jump_cuts.saturating_add(1);
        }
        if cut.crossed_line {
            report.line_crosses = report.line_crosses.saturating_add(1);
        }
        if let Some(held) = previous {
            if cut.tick.saturating_sub(held.tick) < min_shot_ticks {
                report.short_holds = report.short_holds.saturating_add(1);
            }
        }
        if slew_steps(cut.from_kind, cut.to_kind) > MAX_SLEW_STEPS {
            report.slew_overs = report.slew_overs.saturating_add(1);
        }
        if cut.tick.saturating_sub(cut.beat_tick) > STALE_BEAT_TICKS {
            report.stale_cuts = report.stale_cuts.saturating_add(1);
        }
        previous = Some(cut);
    }
    report
}

/// Cut-log file schema filed beside the frames (`cutlog.json`).
pub const CUTLOG_SCHEMA: u32 = 1;
/// Most cuts one cut-log file holds (mirrors the session bound).
pub const MAX_CUTLOG_CUTS: usize = 100_000;

/// Cut-log binding failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CutlogError {
    /// Structural defect in the cut-log JSON, row name carried.
    BadSchema(&'static str),
    /// The filed digest or cut count does not match the cuts.
    BindingMismatch,
}

impl core::fmt::Display for CutlogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSchema(row) => write!(f, "bad cut log ({row})"),
            Self::BindingMismatch => write!(f, "cut log digest mismatch"),
        }
    }
}

impl std::error::Error for CutlogError {}

/// Canonical digest over a cut log plus the held shot: the binding the
/// capture receipt pins in its manifest. FNV-1a over little-endian rows,
/// same owner as the rest of the engine; any tick, framing, focus, side
/// or reason change moves it.
#[must_use]
pub fn cutlog_digest(cuts: &[Cut], shot: Shot) -> u64 {
    let mut hash = Fnv64::new();
    for cut in cuts {
        hash.write_u32(cut.tick);
        hash.write_u8(shot_index(cut.from_kind));
        hash.write_u8(shot_index(cut.to_kind));
        hash.write_u8(cut.from_focus);
        hash.write_u8(cut.to_focus);
        hash.write_u8(cut.side.index());
        hash.write_bool(cut.crossed_line);
        hash.write_u8(reason_index(cut.reason));
        hash.write_u32(cut.beat_tick);
    }
    hash.write_u8(shot_index(shot.kind));
    hash.write_u8(shot.focus);
    hash.write_u8(shot.side.index());
    hash.write_u32(shot.start_tick);
    hash.finish()
}

/// Canonical cut-log JSON: fixed field order, no timestamps, no whitespace
/// variance. The same cuts plus shot always give the same bytes, so the
/// digest filed beside them binds the log.
#[must_use]
pub fn cutlog_json(cuts: &[Cut], shot: Shot) -> String {
    let mut out = String::from("{\"schema\":");
    out.push_str(&CUTLOG_SCHEMA.to_string());
    let count = u32::try_from(cuts.len()).unwrap_or(u32::MAX);
    out.push_str(",\"cut_count\":");
    out.push_str(&count.to_string());
    out.push_str(",\"digest\":\"");
    out.push_str(&format!("{:016x}", cutlog_digest(cuts, shot)));
    out.push_str("\",\"cuts\":[");
    for (index, cut) in cuts.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"tick\":");
        out.push_str(&cut.tick.to_string());
        out.push_str(",\"from\":");
        out.push_str(&shot_index(cut.from_kind).to_string());
        out.push_str(",\"to\":");
        out.push_str(&shot_index(cut.to_kind).to_string());
        out.push_str(",\"from_focus\":");
        out.push_str(&cut.from_focus.to_string());
        out.push_str(",\"to_focus\":");
        out.push_str(&cut.to_focus.to_string());
        out.push_str(",\"side\":");
        out.push_str(&cut.side.index().to_string());
        out.push_str(",\"crossed\":");
        out.push(if cut.crossed_line { '1' } else { '0' });
        out.push_str(",\"reason\":");
        out.push_str(&reason_index(cut.reason).to_string());
        out.push_str(",\"beat\":");
        out.push_str(&cut.beat_tick.to_string());
        out.push('}');
    }
    out.push_str("],\"shot\":{\"kind\":");
    out.push_str(&shot_index(shot.kind).to_string());
    out.push_str(",\"focus\":");
    out.push_str(&shot.focus.to_string());
    out.push_str(",\"side\":");
    out.push_str(&shot.side.index().to_string());
    out.push_str(",\"start\":");
    out.push_str(&shot.start_tick.to_string());
    out.push_str("}}");
    out
}

/// Verify a filed cut log against the receipt rows: the schema, the cut
/// count and the digest must all match the cuts plus shot carried inside.
/// A forged tick, framing or count fails with [`CutlogError`].
pub fn verify_cutlog_json(
    text: &str,
    expect_digest_hex: &str,
    expect_cuts: u32,
) -> Result<(), CutlogError> {
    let schema = field_u32(text, "\"schema\":").ok_or(CutlogError::BadSchema("schema"))?;
    if schema != CUTLOG_SCHEMA {
        return Err(CutlogError::BadSchema("schema"));
    }
    let filed_count =
        field_u32(text, "\"cut_count\":").ok_or(CutlogError::BadSchema("cut_count"))?;
    if filed_count != expect_cuts {
        return Err(CutlogError::BindingMismatch);
    }
    let filed_digest =
        field_hex16(text, "\"digest\":\"").ok_or(CutlogError::BadSchema("digest"))?;
    let expect_digest =
        parse_hex16(expect_digest_hex).ok_or(CutlogError::BadSchema("expect_digest"))?;
    if filed_digest != expect_digest {
        return Err(CutlogError::BindingMismatch);
    }
    let (cuts, shot) = parse_cutlog(text)?;
    if u32::try_from(cuts.len()).unwrap_or(u32::MAX) != filed_count {
        return Err(CutlogError::BindingMismatch);
    }
    if cutlog_digest(&cuts, shot) != filed_digest {
        return Err(CutlogError::BindingMismatch);
    }
    Ok(())
}

/// Read a decimal `u32` filed as `"key":digits`.
fn field_u32(text: &str, key: &str) -> Option<u32> {
    let mut digits = field_digits(text, key)?;
    if digits.len() > 10 {
        return None;
    }
    let mut value: u32 = 0;
    // Borrowed digits stay borrowed: drain from the front without a copy.
    while !digits.is_empty() {
        let (head, tail) = digits.split_at(1);
        digits = tail;
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(head.as_bytes()[0] - b'0'))?;
    }
    Some(value)
}

/// Read a 16-char hex `u64` filed as `"key":"<hex>"`.
fn field_hex16(text: &str, key: &str) -> Option<u64> {
    let after = text.find(key)? + key.len();
    let hex = text.get(after..after.checked_add(16)?)?;
    let digest = parse_hex16(hex)?;
    if text.get(after.checked_add(16)?..after.checked_add(17)?) != Some("\"") {
        return None;
    }
    Some(digest)
}

/// Parse exactly 16 hex chars into a `u64`.
fn parse_hex16(hex: &str) -> Option<u64> {
    if hex.len() != 16 {
        return None;
    }
    let mut value: u64 = 0;
    for byte in hex.bytes() {
        let nibble = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'f' => u64::from(byte - b'a') + 10,
            b'A'..=b'F' => u64::from(byte - b'A') + 10,
            _ => return None,
        };
        value = value.checked_mul(16)?.checked_add(nibble)?;
    }
    Some(value)
}

/// Borrow the digit run after `"key":`, if it holds at least one digit.
fn field_digits<'text>(text: &'text str, key: &str) -> Option<&'text str> {
    let after = text.find(key)? + key.len();
    let rest = text.get(after..)?;
    let len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if len == 0 {
        return None;
    }
    rest.get(..len)
}

/// Decode the cuts plus shot inside canonical [`cutlog_json`] output.
/// Foreign shapes fail with the culprit row; ranges mirror the encoder.
fn parse_cutlog(text: &str) -> Result<(Vec<Cut>, Shot), CutlogError> {
    let array = cutlog_array(text)?;
    let mut cuts = Vec::new();
    for row in split_rows(array)? {
        cuts.push(parse_cut_row(row)?);
    }
    if cuts.len() > MAX_CUTLOG_CUTS {
        return Err(CutlogError::BadSchema("cuts"));
    }
    let shot_text = text
        .find("\"shot\":{")
        .and_then(|at| text.get(at + 7..))
        .and_then(|rest| rest.find('}').and_then(|end| rest.get(..end + 1)))
        .ok_or(CutlogError::BadSchema("shot"))?;
    let shot = Shot {
        kind: decode_shot(field_u32(shot_text, "\"kind\":"))?,
        focus: decode_focus(field_u32(shot_text, "\"focus\":"))?,
        side: decode_side(field_u32(shot_text, "\"side\":"))?,
        start_tick: field_u32(shot_text, "\"start\":").ok_or(CutlogError::BadSchema("shot"))?,
    };
    Ok((cuts, shot))
}

/// Borrow the row text between `"cuts":[` and its closing bracket.
fn cutlog_array(text: &str) -> Result<&str, CutlogError> {
    let start = text
        .find("\"cuts\":[")
        .and_then(|at| at.checked_add(8))
        .ok_or(CutlogError::BadSchema("cuts"))?;
    let rest = text.get(start..).ok_or(CutlogError::BadSchema("cuts"))?;
    let mut depth: u32 = 0;
    for (index, byte) in rest.bytes().enumerate() {
        match byte {
            b'{' => depth = depth.saturating_add(1),
            b'}' => depth = depth.saturating_sub(1),
            b']' if depth == 0 => {
                return rest.get(..index).ok_or(CutlogError::BadSchema("cuts"));
            }
            _ => {}
        }
    }
    Err(CutlogError::BadSchema("cuts"))
}

/// Split one row object per `{...}` span inside the cuts array text.
fn split_rows(array: &str) -> Result<Vec<&str>, CutlogError> {
    let mut rows = Vec::new();
    let mut rest = array;
    while !rest.is_empty() {
        if rest.as_bytes()[0] == b',' {
            rest = rest.get(1..).ok_or(CutlogError::BadSchema("cuts"))?;
        }
        if !rest.starts_with('{') {
            return Err(CutlogError::BadSchema("cuts"));
        }
        let end = rest
            .find('}')
            .and_then(|at| at.checked_add(1))
            .ok_or(CutlogError::BadSchema("cuts"))?;
        rows.push(rest.get(..end).ok_or(CutlogError::BadSchema("cuts"))?);
        rest = rest.get(end..).ok_or(CutlogError::BadSchema("cuts"))?;
    }
    Ok(rows)
}

/// Decode one cut row; every number is range-checked back into its enum.
fn parse_cut_row(row: &str) -> Result<Cut, CutlogError> {
    let crossed = field_u32(row, "\"crossed\":").ok_or(CutlogError::BadSchema("crossed"))?;
    Ok(Cut {
        tick: field_u32(row, "\"tick\":").ok_or(CutlogError::BadSchema("tick"))?,
        from_kind: decode_shot(field_u32(row, "\"from\":"))?,
        to_kind: decode_shot(field_u32(row, "\"to\":"))?,
        from_focus: decode_focus(field_u32(row, "\"from_focus\":"))?,
        to_focus: decode_focus(field_u32(row, "\"to_focus\":"))?,
        side: decode_side(field_u32(row, "\"side\":"))?,
        crossed_line: match crossed {
            0 => false,
            1 => true,
            _ => return Err(CutlogError::BadSchema("crossed")),
        },
        reason: decode_reason(field_u32(row, "\"reason\":"))?,
        beat_tick: field_u32(row, "\"beat\":").ok_or(CutlogError::BadSchema("beat"))?,
    })
}

/// Map a framing discriminant back; anything else is a foreign shape.
fn decode_shot(value: Option<u32>) -> Result<ShotKind, CutlogError> {
    match value {
        Some(0) => Ok(ShotKind::Wide),
        Some(1) => Ok(ShotKind::Duel),
        Some(2) => Ok(ShotKind::Close),
        Some(3) => Ok(ShotKind::Low),
        Some(4) => Ok(ShotKind::High),
        Some(5) => Ok(ShotKind::Crowd),
        _ => Err(CutlogError::BadSchema("shot")),
    }
}

/// Map a line-side discriminant back.
fn decode_side(value: Option<u32>) -> Result<ShotSide, CutlogError> {
    match value {
        Some(0) => Ok(ShotSide::Left),
        Some(1) => Ok(ShotSide::Right),
        _ => Err(CutlogError::BadSchema("side")),
    }
}

/// Map a cut-reason discriminant back.
fn decode_reason(value: Option<u32>) -> Result<CutReason, CutlogError> {
    match value {
        Some(0) => Ok(CutReason::Hit),
        Some(1) => Ok(CutReason::Parry),
        Some(2) => Ok(CutReason::Knockdown),
        Some(3) => Ok(CutReason::Comeback),
        Some(4) => Ok(CutReason::Crowd),
        _ => Err(CutlogError::BadSchema("reason")),
    }
}

/// Range-check a fighter slot back below the session bound.
fn decode_focus(value: Option<u32>) -> Result<u8, CutlogError> {
    match value {
        Some(slot) if slot < u32::from(MAX_PARTICIPANTS) => {
            u8::try_from(slot).map_err(|_| CutlogError::BadSchema("focus"))
        }
        _ => Err(CutlogError::BadSchema("focus")),
    }
}

/// Auto spectator camera: folds each tick's cues into one peak beat, tracks
/// the action-line side from lateral geometry, and cuts on peaks once the
/// hold floor expired. State is presentation only: the session records hits
/// elsewhere and feeds them here through `observe_tick`.
#[derive(Clone, Debug)]
pub struct Director {
    config: DirectorConfig,
    shot: Shot,
    side: ShotSide,
    cuts: Vec<Cut>,
    last_tick: Option<u32>,
    rng: Rng,
}

impl Director {
    /// Build a director over `config`, opening wide on the focus slot. The
    /// cut log is pre-sized for the worst case (one cut per hold window plus
    /// slack), so steady ticks never grow it.
    ///
    /// # Errors
    ///
    /// Fails with `DirectorError::BadConfig` when the config is out of range.
    pub fn new(config: DirectorConfig) -> Result<Self, DirectorError> {
        config.validate()?;
        let windows = config.max_ticks / config.min_shot_ticks;
        let capacity = usize::try_from(windows.saturating_add(LOG_SLACK_CUTS))
            .unwrap_or(usize::MAX)
            .min(usize::try_from(MAX_SESSION_TICKS).unwrap_or(usize::MAX));
        Ok(Self {
            config,
            shot: Shot {
                kind: ShotKind::Wide,
                focus: config.focus,
                side: ShotSide::Left,
                start_tick: 0,
            },
            side: ShotSide::Left,
            cuts: Vec::with_capacity(capacity),
            last_tick: None,
            rng: Rng::new(config.seed),
        })
    }

    /// Session configuration.
    #[must_use]
    pub fn config(&self) -> DirectorConfig {
        self.config
    }

    /// Shot served this tick (the opener before any tick ran).
    #[must_use]
    pub fn shot(&self) -> Shot {
        self.shot
    }

    /// Held side of the action line after the last tick.
    #[must_use]
    pub fn side(&self) -> ShotSide {
        self.side
    }

    /// Cut log in tick order.
    #[must_use]
    pub fn cuts(&self) -> &[Cut] {
        &self.cuts
    }

    /// Cuts fired so far.
    #[must_use]
    pub fn cut_count(&self) -> usize {
        self.cuts.len()
    }

    /// Last tick served, if any tick ran.
    #[must_use]
    pub fn last_tick(&self) -> Option<u32> {
        self.last_tick
    }

    /// Digest over the cut log plus the held shot: identical sessions give
    /// identical digests (the [`cutlog_digest`] binding the receipt pins).
    #[must_use]
    pub fn digest(&self) -> u64 {
        cutlog_digest(&self.cuts, self.shot)
    }

    /// Continuity audit over the cut log so far.
    #[must_use]
    pub fn audit(&self) -> AuditReport {
        audit_cuts(&self.cuts, self.config.min_shot_ticks)
    }

    /// Present one fixed tick: refresh the line side from lateral geometry,
    /// fold the tick into one peak beat, and cut when the peak earns it and
    /// the hold floor expired. Ticks run forward only; every lateral is
    /// validated, so a NaN fails loudly instead of flipping the side. Borrows
    /// plus a pre-sized push only, no heap growth per call at steady state.
    ///
    /// # Errors
    ///
    /// Fails with `DirectorError::TickWentBackwards` when `tick` runs behind
    /// the last served tick, or `DirectorError::BadLateral` on a non-finite
    /// lateral; nothing moves then.
    pub fn observe_tick(
        &mut self,
        tick: u32,
        cues: &[FightCue],
        hp_permille: &[u16],
        bed_gain: f32,
        laterals: &[FighterLateral],
    ) -> Result<Shot, DirectorError> {
        if let Some(last) = self.last_tick {
            if tick < last {
                return Err(DirectorError::TickWentBackwards { last, next: tick });
            }
        }
        let side = side_of(laterals, self.side)?;
        let peak = select_peak(cues, tick, hp_permille, bed_gain);
        self.side = side;
        self.last_tick = Some(tick);
        let Some(beat) = peak else {
            return Ok(self.shot);
        };
        if beat.weight < CUT_MIN_WEIGHT {
            return Ok(self.shot);
        }
        let Some(reason) = reason_for_beat(beat.kind) else {
            return Ok(self.shot);
        };
        let held = self.cuts.last().copied();
        let gap = match held {
            None => self.config.min_shot_ticks,
            Some(previous) => tick.saturating_sub(previous.tick),
        };
        if gap < self.config.min_shot_ticks {
            return Ok(self.shot);
        }
        let knock_high = match reason {
            CutReason::Knockdown => self.rng.next_u64() & 1 == 1,
            _ => false,
        };
        let kind = framing_for_reason(reason, knock_high);
        let crossed = match held {
            None => false,
            Some(previous) => previous.side != side,
        };
        let from = self.shot;
        self.shot = Shot {
            kind,
            focus: beat.focus,
            side,
            start_tick: tick,
        };
        // The log is pre-sized past the one-cut-per-hold bound, so this
        // push stays inside capacity on every reachable session.
        self.cuts.push(Cut {
            tick,
            from_kind: from.kind,
            to_kind: kind,
            from_focus: from.focus,
            to_focus: beat.focus,
            side,
            crossed_line: crossed,
            reason,
            beat_tick: tick,
        });
        Ok(self.shot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    const STREAM_TICKS: u32 = 600;

    fn config_with(seed: u64) -> DirectorConfig {
        DirectorConfig {
            seed,
            focus: 0,
            min_shot_ticks: 60,
            max_ticks: STREAM_TICKS,
        }
    }

    fn tight_config(seed: u64) -> DirectorConfig {
        DirectorConfig {
            seed,
            focus: 0,
            min_shot_ticks: 10,
            max_ticks: STREAM_TICKS,
        }
    }

    fn hit(attacker: u8, victim: u8, damage: u16) -> FightCue {
        FightCue::Hit {
            attacker,
            victim,
            damage,
        }
    }

    fn knock(attacker: u8, victim: u8) -> FightCue {
        FightCue::Knockdown { attacker, victim }
    }

    fn lateral(id: u8, x_m: f32) -> FighterLateral {
        FighterLateral { id, x_m }
    }

    fn duel_left() -> Vec<FighterLateral> {
        vec![lateral(0, -2.0), lateral(1, 2.0)]
    }

    fn duel_right() -> Vec<FighterLateral> {
        vec![lateral(0, 2.0), lateral(1, -2.0)]
    }

    fn hp_full() -> Vec<u16> {
        vec![1000, 1000]
    }

    #[test]
    fn rejects_bad_input_loudly() {
        let good = config_with(1);
        assert!(Director::new(good).is_ok());
        for bad in [
            DirectorConfig {
                focus: MAX_PARTICIPANTS,
                ..good
            },
            DirectorConfig {
                min_shot_ticks: 0,
                ..good
            },
            DirectorConfig {
                min_shot_ticks: MAX_SESSION_TICKS + 1,
                ..good
            },
            DirectorConfig {
                max_ticks: 0,
                ..good
            },
            DirectorConfig {
                min_shot_ticks: 10,
                max_ticks: 9,
                ..good
            },
        ] {
            assert!(Director::new(bad).is_err(), "config accepted: {bad:?}");
        }
        let mut director = Director::new(good).unwrap();
        let hp = hp_full();
        director
            .observe_tick(5, &[], &hp, 1.0, &duel_left())
            .unwrap();
        assert_eq!(director.last_tick(), Some(5));
        assert_eq!(
            director.observe_tick(4, &[], &hp, 1.0, &duel_left()),
            Err(DirectorError::TickWentBackwards { last: 5, next: 4 })
        );
        assert_eq!(director.last_tick(), Some(5));
        let nan = vec![lateral(0, f32::NAN), lateral(1, 2.0)];
        assert_eq!(
            director.observe_tick(6, &[], &hp, 1.0, &nan),
            Err(DirectorError::BadLateral(0))
        );
        assert_eq!(director.last_tick(), Some(5));
    }

    #[test]
    fn silence_holds_past_the_boundary() {
        // No cues at all: the hold floor expiring alone never cuts.
        let mut director = Director::new(config_with(7)).unwrap();
        let hp = hp_full();
        for tick in 0..STREAM_TICKS {
            let shot = director
                .observe_tick(tick, &[], &hp, 1.0, &duel_left())
                .unwrap();
            assert_eq!(shot, director.shot());
        }
        assert_eq!(director.cut_count(), 0);
        assert!(director.audit().is_clean());
        assert_eq!(director.cuts(), &[]);
        // Blocks, dodges and stamina whiffs fold below the cut floor.
        let quiet = vec![
            FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 10,
            },
            FightCue::Dodged {
                attacker: 1,
                victim: 0,
            },
            FightCue::StaminaOut { fighter: 0 },
        ];
        let mut second = Director::new(config_with(7)).unwrap();
        for tick in (0..STREAM_TICKS).step_by(61) {
            second
                .observe_tick(tick, &quiet, &hp, 1.0, &duel_left())
                .unwrap();
        }
        assert_eq!(second.cut_count(), 0, "minor beats must hold the shot");
    }

    #[test]
    fn peaks_cut_when_the_hold_allows() {
        let mut director = Director::new(tight_config(11)).unwrap();
        let hp = hp_full();
        let views = duel_left();
        let first = director
            .observe_tick(0, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        assert_eq!(first.kind, ShotKind::Close);
        assert_eq!(first.focus, 1);
        assert_eq!(director.cut_count(), 1);
        // A peak inside the hold is absorbed, never queued.
        let held = director
            .observe_tick(5, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        assert_eq!(held, first);
        assert_eq!(director.cut_count(), 1);
        // Past the floor the same peak cuts again (and flags the jump).
        director
            .observe_tick(10, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        assert_eq!(director.cut_count(), 2);
        let audit = director.audit();
        assert_eq!(audit.jump_cuts, 1);
        // A parry reframes the exchange instead of the victim.
        let parry = director
            .observe_tick(
                20,
                &[FightCue::Parried {
                    attacker: 1,
                    victim: 0,
                }],
                &hp,
                1.0,
                &views,
            )
            .unwrap();
        assert_eq!(parry.kind, ShotKind::Duel);
        assert_eq!(director.cuts().last().unwrap().reason, CutReason::Parry);
    }

    #[test]
    fn knockdown_framing_is_seeded() {
        // The same knockdown script alternates low and high by seed.
        let run = |seed: u64| {
            let mut director = Director::new(tight_config(seed)).unwrap();
            let hp = hp_full();
            let views = duel_left();
            let mut kinds = Vec::new();
            for tick in (0..60).step_by(10) {
                let shot = director
                    .observe_tick(tick, &[knock(0, 1)], &hp, 1.0, &views)
                    .unwrap();
                kinds.push(shot.kind);
            }
            kinds
        };
        let first = run(3);
        let second = run(3);
        assert_eq!(first, second, "one seed gives one framing run");
        assert!(first
            .iter()
            .all(|kind| matches!(kind, ShotKind::Low | ShotKind::High)));
        assert!(
            first.contains(&ShotKind::Low) && first.contains(&ShotKind::High),
            "the seeded alternate must visit both angles: {first:?}"
        );
        assert_ne!(run(3), run(9), "seeds must frame the falls differently");
    }

    #[test]
    fn comeback_and_crowd_beats_fold() {
        // Hurt fighter lands loud: comeback outranks a plain hit.
        let hurt = vec![200u16, 1000u16];
        let peak = select_peak(&[hit(0, 1, 150)], 12, &hurt, 1.0).unwrap();
        assert_eq!(peak.kind, BeatKind::Comeback);
        assert_eq!(peak.focus, 0);
        assert_eq!(peak.weight, COMEBACK_WEIGHT);
        // Healthy fighter lands loud: plain hit with damage folded in.
        let healthy = hp_full();
        let peak = select_peak(&[hit(0, 1, 150)], 12, &healthy, 1.0).unwrap();
        assert_eq!(peak.kind, BeatKind::Hit);
        assert_eq!(peak.weight, HIT_WEIGHT + 150 / HIT_DAMAGE_DIV);
        // Unknown health slot fails closed (no comeback).
        let peak = select_peak(&[hit(9, 1, 150)], 12, &healthy, 1.0).unwrap();
        assert_eq!(peak.kind, BeatKind::Hit);
        // Deep bed duck reads as a swell; a calm room adds nothing.
        let swell = select_peak(&[], 13, &healthy, 0.2).unwrap();
        assert_eq!(swell.kind, BeatKind::Crowd);
        assert!(select_peak(&[], 13, &healthy, 1.0).is_none());
        // A non-finite bed gain reads as calm, never as a swell.
        assert!(select_peak(&[], 13, &healthy, f32::NAN).is_none());
        // Stamina whiffs fold to nothing and hold the shot.
        assert!(select_peak(&[FightCue::StaminaOut { fighter: 0 }], 13, &healthy, 1.0).is_none());
        // Heavy chip still cannot cut on its own.
        let block = select_peak(
            &[FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 1000,
            }],
            13,
            &healthy,
            1.0,
        )
        .unwrap();
        assert_eq!(block.kind, BeatKind::Block);
        assert!(block.weight < CUT_MIN_WEIGHT);
    }

    #[test]
    fn hero_focus_picks_the_panel_top_over_twenty() {
        // Twenty scored hero framings with permuted ids and a graded ladder
        // of totals: the panel-top total wins wherever it sits in the slate.
        let mut slate = [(0u32, 0u32); 20];
        for rank in 0..20u32 {
            let id = (rank * 7 + 3) % 20;
            let index = usize::try_from(id).expect("panel ids fit the slate");
            slate[index] = (id, 900 - 37 * rank);
        }
        assert_eq!(pick_hero_focus(&slate), Some(3));
        slate.reverse();
        assert_eq!(
            pick_hero_focus(&slate),
            Some(3),
            "slate order never picks the hero"
        );
        // Failure rows: nothing scored picks nothing, and a corrupt winner id
        // holds the shot instead of framing outside the slots.
        assert_eq!(pick_hero_focus(&[]), None);
        assert_eq!(pick_hero_focus(&[(99, 5_000)]), None);
    }

    #[test]
    fn hero_focus_breaks_weight_ties_toward_the_lowest_slot() {
        let slate = [(4u32, 700u32), (1u32, 700u32), (9u32, 700u32)];
        assert_eq!(pick_hero_focus(&slate), Some(1));
        let mut flipped = slate;
        flipped.reverse();
        assert_eq!(
            pick_hero_focus(&flipped),
            Some(1),
            "ties ignore slate order"
        );
    }

    #[test]
    fn peak_ties_frame_the_lowest_slot_in_any_cue_order() {
        // Equal-weight hits on both fighters: the lower slot frames no matter
        // which cue arrives first.
        let healthy = hp_full();
        let forward = [hit(1, 0, 60), hit(0, 1, 60)];
        let backward = [hit(0, 1, 60), hit(1, 0, 60)];
        let first = select_peak(&forward, 12, &healthy, 1.0).expect("tied peaks pick");
        let second = select_peak(&backward, 12, &healthy, 1.0).expect("tied peaks pick");
        assert_eq!(first.focus, 0);
        assert_eq!(second.focus, 0);
        assert_eq!(first, second);
        // The heaviest beat still outranks slots: damage wins over position.
        let heavy = [hit(1, 0, 60), hit(0, 1, 150)];
        let peak = select_peak(&heavy, 12, &healthy, 1.0).expect("heavy peak picks");
        assert_eq!(peak.focus, 1);
        assert_eq!(peak.weight, HIT_WEIGHT + 150 / HIT_DAMAGE_DIV);
        // Corrupt slots never frame: out-of-range ids hold the shot.
        assert!(select_peak(&[hit(20, 21, 60)], 12, &healthy, 1.0).is_none());
    }

    #[test]
    fn real_line_cross_flags_on_fed_positions() {
        // Two fighters at distinct laterals: the held side is their order.
        let mut director = Director::new(tight_config(21)).unwrap();
        let hp = hp_full();
        director
            .observe_tick(0, &[hit(0, 1, 60)], &hp, 1.0, &duel_left())
            .unwrap();
        assert_eq!(director.side(), ShotSide::Left);
        assert!(!director.cuts().last().unwrap().crossed_line);
        // The pair swaps order past the hold: the next cut crosses the line.
        director
            .observe_tick(5, &[], &hp, 1.0, &duel_right())
            .unwrap();
        assert_eq!(director.side(), ShotSide::Right);
        assert_eq!(director.cut_count(), 1, "side alone never cuts");
        director
            .observe_tick(10, &[hit(1, 0, 60)], &hp, 1.0, &duel_right())
            .unwrap();
        assert_eq!(director.cut_count(), 2);
        let cross = director.cuts().last().unwrap();
        assert!(cross.crossed_line, "the swap must flag: {cross:?}");
        assert_eq!(cross.side, ShotSide::Right);
        let audit = director.audit();
        assert!(audit.line_crosses >= 1);
        // Shuffled lateral order gives the same side (ids pick the pair).
        let mut shuffled = duel_right();
        shuffled.reverse();
        assert_eq!(side_of(&shuffled, ShotSide::Left).unwrap(), ShotSide::Right);
        // One lateral keeps the held side; ties read as the first side.
        assert_eq!(
            side_of(&duel_left()[..1], ShotSide::Right).unwrap(),
            ShotSide::Right
        );
        let tie = vec![lateral(0, 1.0), lateral(1, 1.0)];
        assert_eq!(side_of(&tie, ShotSide::Right).unwrap(), ShotSide::Left);
    }

    #[test]
    fn seeded_cut_log_is_identical_twice() {
        let script = |tick: u32| {
            if tick.is_multiple_of(30) {
                vec![knock(0, 1)]
            } else if tick.is_multiple_of(7) {
                vec![hit(0, 1, 90)]
            } else if tick.is_multiple_of(11) {
                vec![FightCue::Parried {
                    attacker: 1,
                    victim: 0,
                }]
            } else {
                Vec::new()
            }
        };
        let run = || {
            let mut director = Director::new(config_with(2040)).unwrap();
            let hp = hp_full();
            for tick in 0..STREAM_TICKS {
                let cues = script(tick);
                // The pair drifts and laps once, so the log holds a cross.
                let x = if tick < 300 { -2.0 } else { 2.0 };
                let views = vec![lateral(0, x), lateral(1, -x)];
                director
                    .observe_tick(tick, &cues, &hp, 1.0, &views)
                    .unwrap();
            }
            director
        };
        let first = run();
        let second = run();
        assert_eq!(first.cuts(), second.cuts());
        assert_eq!(first.digest(), second.digest());
        assert!(!first.cuts().is_empty(), "the script must earn cuts");
        assert!(first.audit().line_crosses >= 1, "the lap must cross");
        assert_ne!(
            first.digest(),
            Director::new(config_with(2040)).unwrap().digest()
        );
    }

    #[test]
    fn audit_flags_every_continuity_break() {
        let open = Cut {
            tick: 0,
            from_kind: ShotKind::Wide,
            to_kind: ShotKind::Wide,
            from_focus: 0,
            to_focus: 0,
            side: ShotSide::Left,
            crossed_line: false,
            reason: CutReason::Hit,
            beat_tick: 0,
        };
        let jump = Cut {
            tick: 70,
            from_kind: ShotKind::Close,
            to_kind: ShotKind::Close,
            from_focus: 1,
            to_focus: 1,
            side: ShotSide::Left,
            crossed_line: false,
            reason: CutReason::Hit,
            beat_tick: 70,
        };
        let cross = Cut {
            tick: 140,
            from_kind: ShotKind::Close,
            to_kind: ShotKind::Duel,
            from_focus: 1,
            to_focus: 0,
            side: ShotSide::Right,
            crossed_line: true,
            reason: CutReason::Parry,
            beat_tick: 140,
        };
        let whip = Cut {
            tick: 200,
            from_kind: ShotKind::High,
            to_kind: ShotKind::Wide,
            from_focus: 1,
            to_focus: 1,
            side: ShotSide::Right,
            crossed_line: false,
            reason: CutReason::Knockdown,
            beat_tick: 0,
        };
        let report = audit_cuts(&[open, jump, cross, whip], MIN_SHOT_TICKS);
        assert_eq!(report.cuts, 4);
        assert!(report.jump_cuts >= 1, "jump cut must flag");
        assert!(report.line_crosses >= 1, "line cross must flag");
        assert!(report.slew_overs >= 1, "slew over must flag");
        assert!(report.stale_cuts >= 1, "stale beat must flag");
        assert!(!report.is_clean());
        assert_eq!(
            report.total_flags(),
            report
                .jump_cuts
                .saturating_add(report.line_crosses)
                .saturating_add(report.slew_overs)
                .saturating_add(report.stale_cuts)
        );
        assert!(audit_cuts(&[], MIN_SHOT_TICKS).is_clean());
    }

    #[test]
    fn cutlog_json_roundtrips_and_binds_digest() {
        let mut director = Director::new(tight_config(11)).unwrap();
        let hp = hp_full();
        let views = duel_left();
        director
            .observe_tick(0, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        director
            .observe_tick(10, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        let cuts = director.cuts().to_vec();
        let shot = director.shot();
        let filed = cutlog_json(&cuts, shot);
        let digest_hex = format!("{:016x}", director.digest());
        assert_eq!(
            cutlog_digest(&cuts, shot),
            director.digest(),
            "the filed digest is the session digest"
        );
        assert!(verify_cutlog_json(&filed, &digest_hex, cuts.len() as u32).is_ok());
        // Same script twice gives the same canonical bytes.
        let mut second = Director::new(tight_config(11)).unwrap();
        second
            .observe_tick(0, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        second
            .observe_tick(10, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        assert_eq!(filed, cutlog_json(second.cuts(), second.shot()));
    }

    #[test]
    fn cutlog_forgery_fails_loudly() {
        let mut director = Director::new(tight_config(11)).unwrap();
        let hp = hp_full();
        let views = duel_left();
        director
            .observe_tick(0, &[hit(0, 1, 60)], &hp, 1.0, &views)
            .unwrap();
        let cuts = director.cuts().to_vec();
        let shot = director.shot();
        let filed = cutlog_json(&cuts, shot);
        let digest_hex = format!("{:016x}", director.digest());
        let count = cuts.len() as u32;
        // The ADV52 forgery shape: the same log with one tick bumped keeps
        // the filed digest, so the binding must refuse it.
        let first_tick = cuts[0].tick;
        let forged = filed.replacen(
            &format!("\"tick\":{first_tick}"),
            &format!("\"tick\":{}", first_tick + 1),
            1,
        );
        assert_ne!(forged, filed);
        assert_eq!(
            verify_cutlog_json(&forged, &digest_hex, count),
            Err(CutlogError::BindingMismatch)
        );
        // A resealed count or digest beside the same bytes still fails: the
        // receipt rows must match the cuts, not just each other.
        assert_eq!(
            verify_cutlog_json(&filed, &digest_hex, count + 1),
            Err(CutlogError::BindingMismatch)
        );
        assert_eq!(
            verify_cutlog_json(&filed, "0000000000000000", count),
            Err(CutlogError::BindingMismatch)
        );
        // Foreign shapes fail as schema defects, never as clean.
        assert_eq!(
            verify_cutlog_json("{}", &digest_hex, count),
            Err(CutlogError::BadSchema("schema"))
        );
        assert_eq!(
            verify_cutlog_json(&filed, "not-hex-at-all!!!!", count),
            Err(CutlogError::BadSchema("expect_digest"))
        );
    }

    #[test]
    fn app_session_drives_director_from_real_inputs() {
        use crate::{App, GameConfig, HUMAN_SLOT};
        use engine_input::{Buttons, FighterId, FighterInput};

        let ticks = 600u32;
        let config = GameConfig {
            seed: 2040,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let assault = |tick: u32| FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).unwrap(),
            mx: 100,
            mz: 0,
            facing: 0,
            buttons: if tick.is_multiple_of(5) {
                Buttons(engine_input::fighter::ATTACK)
            } else {
                Buttons::NONE
            },
        };
        let run_once = || {
            let mut app = App::new(config).unwrap();
            for tick in 0..ticks {
                let outcome = app.tick(assault(tick));
                assert!(outcome.input_accepted);
            }
            app
        };
        let first = run_once();
        let second = run_once();
        assert_eq!(first.director_cuts(), second.director_cuts());
        assert_eq!(first.director_digest(), second.director_digest());
        assert_eq!(first.checkpoints(), second.checkpoints());
        assert!(
            !first.director_cuts().is_empty(),
            "a real fight must earn cuts"
        );
        assert_ne!(
            first.director_digest(),
            App::new(config).unwrap().director_digest(),
            "fights must differ from the cold open"
        );
    }

    #[test]
    fn frame_work_is_bounded() {
        let mut director = Director::new(config_with(5)).unwrap();
        let hp = hp_full();
        let burst = vec![
            hit(0, 1, 150),
            FightCue::Parried {
                attacker: 1,
                victim: 0,
            },
            knock(0, 1),
        ];
        for tick in 0..STREAM_TICKS {
            let before = director.cut_count();
            director
                .observe_tick(tick, &burst, &hp, 0.9, &duel_left())
                .unwrap();
            assert!(
                director.cut_count() - before <= 1,
                "at most one cut per tick"
            );
        }
        assert!(
            u32::try_from(director.cut_count()).unwrap_or(u32::MAX) <= STREAM_TICKS,
            "cut log stays within one cut per tick"
        );
    }

    #[test]
    fn steady_state_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut director = Director::new(config_with(11)).unwrap();
        let hp = hp_full();
        let cues = vec![hit(0, 1, 150)];
        let views = duel_left();
        director.observe_tick(0, &cues, &hp, 1.0, &views).unwrap();
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 1..STREAM_TICKS {
                let shot = director
                    .observe_tick(tick, &cues, &hp, 1.0, &views)
                    .unwrap();
                folded ^= director.digest().wrapping_add(u64::from(shot.start_tick));
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state director update must not allocate"
        );
        assert!(folded != 0, "sanity: shots advanced during the window");
    }

    #[test]
    #[ignore = "timing: director per-frame budget probe, run under --perf in release"]
    fn budget_probe_under_fifty_microseconds() {
        let cues = vec![hit(0, 1, 150)];
        let hp = hp_full();
        let views = duel_left();
        let mut director = Director::new(DirectorConfig {
            max_ticks: 10_000,
            ..config_with(3)
        })
        .unwrap();
        director.observe_tick(0, &cues, &hp, 1.0, &views).unwrap();
        let rounds = 4000u32;
        let mut folded = 0u64;
        let start = std::time::Instant::now();
        for tick in 1..=rounds {
            let shot = director
                .observe_tick(tick, &cues, &hp, 1.0, &views)
                .unwrap();
            folded ^= u64::from(shot.start_tick);
        }
        std::hint::black_box(folded);
        let micros = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
        let mean = micros / u64::from(rounds);
        eprintln!("director budget probe: mean {mean} us per update over {rounds} rounds");
        assert!(
            mean < DIRECTOR_BUDGET_MICROS,
            "mean {mean} microseconds per director update over budget"
        );
    }
}
