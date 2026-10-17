//! depth: gladiator depth over the scripted utility core.
//!
//! Consumer: arena match brains feed one [`DepthBrain`] per fighter, one
//! [`Squad`] per team and one [`Ladder`] per progression. The arena composes
//! the Rome scoring inputs (phase, feint, stamina, role) as one
//! [`ArenaPolicy`] value and installs it on the brain; the generic integer
//! core in `gladiator.rs` never names them. Data owner: [`ArenaPolicy`]
//! owns phase/feint/stamina/role inputs, [`Squad`] owns roles, [`Ladder`]
//! owns the rung; match state stays in the arena. Units: ticks, morale
//! `0..=1000`, stamina `0..=1000`. Determinism: same percepts plus same
//! trigger calls give byte-identical inputs; ties break by fighter id.
//! Errors: [`DepthError`]; unknown phase/role/rung refuses.

use crate::curves::{parse_dec_u32, AiConfig};
use crate::gladiator::{sanitize_dist_mm, Brain, Intent, Percept};
use crate::melee::add_bias;
use crate::personality::Personality;
use engine_input::{FighterId, FighterInput, MAX_PARTICIPANTS};

// Steering lives in depth/steer.rs (directory child module so the
// workspace module gate resolves it; NOT `legion` — that name belongs to
// the squad module in ai/src/legion.rs).
#[path = "depth/steer.rs"]
mod steer;
pub use steer::{steer_wish, DEFAULT_HALF_EXTENT_MM, SEEK_MM, WALL_PROX_MM};

/// Score-lane order shared with the scripted utility table.
const IDX_ATTACK: usize = 0;
/// Score-lane order shared with the scripted utility table.
const IDX_BLOCK: usize = 1;
/// Score-lane order shared with the scripted utility table.
const IDX_DODGE: usize = 2;
/// Score-lane order shared with the scripted utility table.
const IDX_ADVANCE: usize = 3;
/// Score-lane order shared with the scripted utility table.
const IDX_RETREAT: usize = 4;

/// Close range in millimetres (mirrors the scripted strike bands).
const CLOSE_MM: i32 = 4_000;
/// Feint window length in ticks after an attack release.
const FEINT_WINDOW_TICKS: u32 = 12;
/// Feint cooldown in ticks before another feint may trigger.
const FEINT_COOLDOWN_TICKS: u32 = 36;
/// Attack bonus while the feint window is open.
const FEINT_ATTACK_BONUS: i32 = 120;
/// Stamina below which the fighter is winded (`400..` is fresh).
const STAMINA_WINDED_BELOW: i32 = 400;
/// Stamina below which the fighter is gassed.
const STAMINA_GASSED_BELOW: i32 = 150;
/// Stamina at or below which the fighter has collapsed (squad trigger).
pub const STAMINA_COLLAPSE_AT: i32 = 100;
/// Stamina bounds.
pub const STAMINA_MIN: i32 = 0;
/// Stamina bounds.
pub const STAMINA_MAX: i32 = 1_000;
/// Stamina recovered every decide tick after the intent cost. Set above the
/// cheapest costs (block 10, advance/reposition 12, retreat 8) so backing
/// off recovers, while striking (40) and evading (30) still drain: attack
/// spam gasses a fresh fighter in about 30 ticks, retreat recovers it.
const STAMINA_RECOVERY: i32 = 14;

/// General morale presets: named starting morale values (`0..=1000`).
/// Caesar pushes (aggressive), Fabius holds, Hannibal circles (flank).
/// Data only; pick one with `set_morale`, no behavior change by itself.
pub const MORALE_CAESAR: i32 = 750;
/// General morale preset: Fabius holds at neutral morale.
pub const MORALE_FABIUS: i32 = 500;
/// General morale preset: Hannibal flanks above neutral morale.
pub const MORALE_HANNIBAL: i32 = 650;

/// Gladiator-depth failures. Invalid external input returns these; the
/// simulation never panics on them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthError {
    /// Phase byte outside `0..=2`; culprit carried.
    UnknownPhase {
        /// The rejected phase byte.
        value: u8,
    },
    /// Role byte outside `0..=1`; culprit carried.
    UnknownRole {
        /// The rejected role byte.
        value: u8,
    },
    /// Ladder rung above [`Ladder::MAX_RUNG`]; culprit carried.
    UnknownRung {
        /// The rejected rung.
        rung: u8,
    },
    /// Tension-table entry outside its fence; rung, field and value carried.
    TableOutOfFence {
        /// Rung holding the bad entry.
        rung: u8,
        /// Field name (`crowd_volume`, `spawn_ticks` or `morale_drift`).
        field: &'static str,
        /// The rejected value.
        value: i32,
    },
    /// Squad built with no members.
    EmptySquad,
    /// Fighter slot outside the squad or `>= MAX_PARTICIPANTS`; culprit carried.
    UnknownFighter {
        /// The rejected slot.
        id: u8,
    },
    /// Fighter listed twice in a squad; culprit carried.
    DuplicateFighter {
        /// The duplicated slot.
        id: u8,
    },
    /// An attack token asked for a fighter's own slot; culprit carried.
    SelfTarget {
        /// The slot that named itself.
        id: u8,
    },
}

impl core::fmt::Display for DepthError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DepthError::UnknownPhase { value } => {
                write!(f, "gladiator depth: unknown phase {value}")
            }
            DepthError::UnknownRole { value } => {
                write!(f, "gladiator depth: unknown role {value}")
            }
            DepthError::UnknownRung { rung } => {
                write!(f, "gladiator depth: unknown ladder rung {rung}")
            }
            DepthError::TableOutOfFence { rung, field, value } => write!(
                f,
                "gladiator depth: rung {rung} field {field} value {value} outside its fence"
            ),
            DepthError::EmptySquad => write!(f, "gladiator depth: squad has no members"),
            DepthError::UnknownFighter { id } => {
                write!(f, "gladiator depth: unknown fighter {id}")
            }
            DepthError::DuplicateFighter { id } => {
                write!(f, "gladiator depth: duplicate fighter {id}")
            }
            DepthError::SelfTarget { id } => {
                write!(f, "gladiator depth: fighter {id} cannot target itself")
            }
        }
    }
}

impl std::error::Error for DepthError {}

/// Match phase: open (feel out), pressure (trade blows), finish (go for
/// the win). A scoring input only; it never re-times the perception clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Opening exchanges: close distance, probe the guard.
    Open,
    /// Middle bout: trade blows.
    Pressure,
    /// Closing bout: commit to the strike.
    Finish,
}

impl Phase {
    /// Parse a phase byte (`0` open, `1` pressure, `2` finish). Anything
    /// else refuses loudly with [`DepthError::UnknownPhase`].
    pub fn from_u8(value: u8) -> Result<Self, DepthError> {
        match value {
            0 => Ok(Phase::Open),
            1 => Ok(Phase::Pressure),
            2 => Ok(Phase::Finish),
            _ => Err(DepthError::UnknownPhase { value }),
        }
    }

    /// Phase byte round-trip for tapes (`0`/`1`/`2`).
    pub const fn as_u8(self) -> u8 {
        match self {
            Phase::Open => 0,
            Phase::Pressure => 1,
            Phase::Finish => 2,
        }
    }

    /// Phase from round progress: open in the first third, pressure in the
    /// second, finish in the last. Integer math only; `round_ticks == 0`
    /// (unknown length) stays [`Phase::Open`].
    pub fn for_tick(tick: u32, round_ticks: u32) -> Self {
        if round_ticks == 0 {
            return Phase::Open;
        }
        // `u64` so full-range ticks cannot overflow the thirds.
        match u64::from(tick) * 3 / u64::from(round_ticks) {
            0 => Phase::Open,
            1 => Phase::Pressure,
            _ => Phase::Finish,
        }
    }

    /// Additive score bias in scripted table order
    /// (attack, block, dodge, advance, retreat). Bounded to `+/-150` so
    /// curves still dominate; the finish commit is the largest swing.
    pub const fn score_bias(self) -> [i32; 5] {
        match self {
            Phase::Open => [-50, 25, 0, 75, 25],
            Phase::Pressure => [75, 50, 25, 0, -50],
            Phase::Finish => [150, -25, -25, 50, -75],
        }
    }
}

/// Squad role: one holder pins the center, flankers circle for side arcs.
/// A scoring input only; reassignment lives on [`Squad`], never mid-swing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Pins the center and trades blows.
    Holder,
    /// Circles for side arcs.
    Flanker,
}

impl Role {
    /// Parse a role byte (`0` holder, `1` flanker). Anything else refuses
    /// loudly with [`DepthError::UnknownRole`].
    pub fn from_u8(value: u8) -> Result<Self, DepthError> {
        match value {
            0 => Ok(Role::Holder),
            1 => Ok(Role::Flanker),
            _ => Err(DepthError::UnknownRole { value }),
        }
    }

    /// Role byte round-trip for tapes (`0`/`1`).
    pub const fn as_u8(self) -> u8 {
        match self {
            Role::Holder => 0,
            Role::Flanker => 1,
        }
    }

    /// Additive score bias in scripted table order. Bounded to `+/-75`.
    pub const fn score_bias(self) -> [i32; 5] {
        match self {
            Role::Holder => [25, 75, -25, -25, 0],
            Role::Flanker => [0, -25, 50, 75, 0],
        }
    }
}

/// Duel tier from designer rows: skill-first enemy kinds.
///
/// Consumer: the arena duel builds one [`DepthBrain`] per foe and installs
/// the tier with [`DepthBrain::set_tier`]; the scored decide path reads it
/// as role plus temperament into the gladiator core, so tiers climb in
/// skill while health stays in the narrow band. Data owner: [`TierRow`]
/// owns health, tell, stagger and counter; role, reaction and aggression
/// derive from the tier number. Units: health, stagger and aggression in
/// permille, tell and reaction in ticks at 60 Hz, counter 1 parry 2 dodge
/// 3 block. Determinism: same text gives same rows; same tier gives the
/// same role and temperament; integer only. Errors: [`TierError`] with the
/// 1-based line. Budget: parse runs once at match build; a decide reads a
/// few integer ops, no allocation. The tension table stays morale-only;
/// tiers are a separate axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TierRow {
    /// Tier number (`1`, `2`, `3` or the primus `4`).
    pub tier: u8,
    /// Health in permille, narrow `900..=1200` so skill decides.
    pub hp_permille: u16,
    /// Readable startup in ticks, `15..=60`.
    pub tell_ticks: u32,
    /// Poise hold in permille, `0..=1000`.
    pub stagger_permille: u16,
    /// Answer from the player kit (`1` parry, `2` dodge, `3` block).
    pub counter: u8,
}

/// Preferred swing range for every tier in millimetres.
pub const TIER_RANGE_MM: u16 = 1_200;

impl TierRow {
    /// Squad role for this tier: ends hold, middles circle.
    pub const fn role(self) -> Role {
        match self.tier {
            2 => Role::Flanker,
            _ => Role::Holder,
        }
    }

    /// Ticks before this tier notices the foe blow: faster climbs.
    pub const fn reaction_ticks(self) -> u8 {
        match self.tier {
            1 => 4,
            2 => 2,
            _ => 1,
        }
    }

    /// Nerve in permille: calm, even, bold.
    pub const fn aggression_permille(self) -> u16 {
        match self.tier {
            1 => 400,
            2 => 500,
            _ => 700,
        }
    }

    /// Nerve in permille for `phase`: tiers 1-3 ignore the phase and read
    /// [`TierRow::aggression_permille`]; tier 4 keeps 700 in phase 1 and
    /// lifts to 750 in phase 2 (boldest, inside the 250-750 card range).
    /// Integer only.
    pub const fn aggression_for_phase(self, phase: u8) -> u16 {
        if self.tier == 4 && phase == 2 {
            750
        } else {
            self.aggression_permille()
        }
    }

    /// Temperament for this tier at the shared swing range.
    pub fn personality(self) -> Personality {
        Personality::new(
            self.reaction_ticks(),
            TIER_RANGE_MM,
            self.aggression_permille(),
        )
    }

    /// Temperament for this tier in `phase` at the shared swing range:
    /// the primus keeps reaction 1 and lifts nerve 700 to 750 in phase 2.
    pub fn personality_for_phase(self, phase: u8) -> Personality {
        Personality::new(
            self.reaction_ticks(),
            TIER_RANGE_MM,
            self.aggression_for_phase(phase),
        )
    }

    /// Parse one `tier_<n> hp_permille=<int> tell_ticks=<int>
    /// stagger_permille=<int> counter=<int>` row with its 1-based line.
    /// Whole numbers only; fences refuse as [`TierError::OutOfFence`].
    /// Accepts `tier_1` to `tier_4`; [`parse_tiers`] seats all four and
    /// defaults a missing `tier_4` to [`PRIMUS_TIER`].
    pub fn from_row(line: &str, line_no: usize) -> Result<Self, TierError> {
        let mut words = line.split_whitespace();
        let id = words.next().ok_or(TierError::RowId { line: line_no })?;
        let num = id
            .strip_prefix("tier_")
            .ok_or(TierError::RowId { line: line_no })?;
        let tier = parse_tier_no(num).ok_or(TierError::RowId { line: line_no })?;
        if !(1..=4).contains(&tier) {
            return Err(TierError::RowId { line: line_no });
        }
        let mut hp: Option<u16> = None;
        let mut tell: Option<u32> = None;
        let mut stagger: Option<u16> = None;
        let mut counter: Option<u8> = None;
        for token in words {
            let (key, val) = token
                .split_once('=')
                .ok_or(TierError::UnknownKey { line: line_no })?;
            if val.contains('.') {
                return Err(TierError::Float { line: line_no });
            }
            let num = parse_dec_u32(val).ok_or(TierError::BadInt { line: line_no })?;
            if key == "hp_permille" {
                if hp.is_some() {
                    return Err(TierError::UnknownKey { line: line_no });
                }
                hp = Some(u16::try_from(num).map_err(|_| TierError::BadInt { line: line_no })?);
            } else if key == "tell_ticks" {
                if tell.is_some() {
                    return Err(TierError::UnknownKey { line: line_no });
                }
                tell = Some(num);
            } else if key == "stagger_permille" {
                if stagger.is_some() {
                    return Err(TierError::UnknownKey { line: line_no });
                }
                stagger =
                    Some(u16::try_from(num).map_err(|_| TierError::BadInt { line: line_no })?);
            } else if key == "counter" {
                if counter.is_some() {
                    return Err(TierError::UnknownKey { line: line_no });
                }
                counter = Some(u8::try_from(num).map_err(|_| TierError::BadInt { line: line_no })?);
            } else {
                return Err(TierError::UnknownKey { line: line_no });
            }
        }
        let hp = hp.ok_or(TierError::MissingKey { line: line_no })?;
        let tell = tell.ok_or(TierError::MissingKey { line: line_no })?;
        let stagger = stagger.ok_or(TierError::MissingKey { line: line_no })?;
        let counter = counter.ok_or(TierError::MissingKey { line: line_no })?;
        if !(900..=1_200).contains(&hp) {
            return Err(TierError::OutOfFence {
                line: line_no,
                field: "hp_permille",
                value: i32::from(hp),
            });
        }
        let tell_i = i32::try_from(tell).map_err(|_| TierError::BadInt { line: line_no })?;
        if !(15..=60).contains(&tell_i) {
            return Err(TierError::OutOfFence {
                line: line_no,
                field: "tell_ticks",
                value: tell_i,
            });
        }
        if stagger > 1_000 {
            return Err(TierError::OutOfFence {
                line: line_no,
                field: "stagger_permille",
                value: i32::from(stagger),
            });
        }
        if !(1..=3).contains(&counter) {
            return Err(TierError::OutOfFence {
                line: line_no,
                field: "counter",
                value: i32::from(counter),
            });
        }
        Ok(Self {
            tier,
            hp_permille: hp,
            tell_ticks: tell,
            stagger_permille: stagger,
            counter,
        })
    }
}

/// Shipped duel tiers matching the designer rows file.
pub const SHIPPED_TIERS: [TierRow; 3] = [
    TierRow {
        tier: 1,
        hp_permille: 950,
        tell_ticks: 15,
        stagger_permille: 400,
        counter: 1,
    },
    TierRow {
        tier: 2,
        hp_permille: 1_000,
        tell_ticks: 15,
        stagger_permille: 500,
        counter: 2,
    },
    TierRow {
        tier: 3,
        hp_permille: 1_050,
        tell_ticks: 15,
        stagger_permille: 600,
        counter: 1,
    },
];

/// Final-foe primus row: skill, not stats. Health sits inside the narrow
/// `900..=1200` band, the tell matches the ladder's readable 15 ticks,
/// and the phase-1 answer is parry (`1`), the same school as the T3
/// elite. Notice is faster (reaction 1) and nerve is bolder (700, 750 in
/// phase 2). The designer file carries this row as `tier_4`; a file
/// without one still parses, defaulting the fourth slot to this const.
pub const PRIMUS_TIER: TierRow = TierRow {
    tier: 4,
    hp_permille: 1100,
    tell_ticks: 15,
    stagger_permille: 700,
    counter: 1,
};

/// Primus phase from remaining health in permille: phase 2 at or below
/// 500, else phase 1. One integer compare; integer only, never panics.
pub const fn primus_phase(hp_permille: u16) -> u8 {
    if hp_permille <= 500 {
        2
    } else {
        1
    }
}

/// Primus answer from its phase: phase 1 parries (`1`, the primus row),
/// phase 2 dodges (`2`, the new answer). Any other phase returns the
/// phase-1 answer. Never panics, never guesses.
pub const fn primus_counter(phase: u8) -> u8 {
    if phase == 2 {
        2
    } else {
        1
    }
}

/// Tier rows parse failures with the 1-based line number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TierError {
    /// First content line is not `table enemies version <n>`.
    Header { line: usize },
    /// Header version is zero or unreadable.
    Version { line: usize },
    /// Row id is not `tier_1` to `tier_4`.
    RowId { line: usize },
    /// Key is unknown or repeated.
    UnknownKey { line: usize },
    /// An expected key never appears, or a tier never appears.
    MissingKey { line: usize },
    /// Value is not ASCII digits.
    BadInt { line: usize },
    /// Value holds a float point.
    Float { line: usize },
    /// Same tier appears twice.
    Duplicate { line: usize },
    /// Value sits outside its fence; field and value carried.
    OutOfFence {
        line: usize,
        field: &'static str,
        value: i32,
    },
}

impl core::fmt::Display for TierError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Header { line } => {
                write!(f, "tier rows line {line}: want `table enemies version <n>`")
            }
            Self::Version { line } => write!(f, "tier rows line {line}: version is zero"),
            Self::RowId { line } => {
                write!(f, "tier rows line {line}: want row `tier_1` to `tier_4`")
            }
            Self::UnknownKey { line } => write!(f, "tier rows line {line}: unknown key"),
            Self::MissingKey { line } => write!(f, "tier rows line {line}: missing key"),
            Self::BadInt { line } => write!(f, "tier rows line {line}: want digits"),
            Self::Float { line } => write!(f, "tier rows line {line}: no floats"),
            Self::Duplicate { line } => write!(f, "tier rows line {line}: duplicate tier"),
            Self::OutOfFence { line, field, value } => {
                write!(
                    f,
                    "tier rows line {line}: {field} value {value} outside its fence"
                )
            }
        }
    }
}

impl std::error::Error for TierError {}

/// Tier number `1..=4` from digits; `None` otherwise.
fn parse_tier_no(text: &str) -> Option<u8> {
    let n = parse_dec_u32(text)?;
    u8::try_from(n).ok()
}

/// Header `table enemies version <n>` to its nonzero version.
fn tier_header(line: &str, line_no: usize) -> Result<u32, TierError> {
    let mut parts = line.split_whitespace();
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(a), Some(name), Some(b), Some(num), None)
            if a == "table" && name == "enemies" && b == "version" =>
        {
            if num.contains('.') {
                return Err(TierError::Float { line: line_no });
            }
            let v = parse_dec_u32(num).ok_or(TierError::Header { line: line_no })?;
            if v == 0 {
                return Err(TierError::Version { line: line_no });
            }
            Ok(v)
        }
        _ => Err(TierError::Header { line: line_no }),
    }
}

/// Parse the designer rows file into the four duel tiers in tier order.
///
/// Layout mirrors the combat rows: line 1 `table enemies version <n>`,
/// then one `tier_<n>` row per tier; `#` comments and blank lines ignored.
/// Whole numbers only; a bad row refuses with its line number. Rows
/// `tier_1` to `tier_3` are required; a missing `tier_4` falls back to
/// [`PRIMUS_TIER`], so the ladder always seats four rows.
pub fn parse_tiers(text: &str) -> Result<[TierRow; 4], TierError> {
    let mut header: Option<u32> = None;
    let mut header_no: usize = 0;
    let mut slots: [Option<TierRow>; 4] = [None, None, None, None];
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx.saturating_add(1);
        let cut = raw.find('#').map_or(raw, |i| raw.split_at(i).0);
        let line = cut.trim();
        if line.is_empty() {
            continue;
        }
        if header.is_none() {
            header = Some(tier_header(line, line_no)?);
            header_no = line_no;
        } else {
            let row = TierRow::from_row(line, line_no)?;
            let slot = usize::from(row.tier).saturating_sub(1);
            if slot >= slots.len() {
                return Err(TierError::RowId { line: line_no });
            }
            if slots[slot].is_some() {
                return Err(TierError::Duplicate { line: line_no });
            }
            slots[slot] = Some(row);
        }
    }
    header.ok_or(TierError::Header { line: 1 })?;
    let a = slots[0].ok_or(TierError::MissingKey { line: header_no })?;
    let b = slots[1].ok_or(TierError::MissingKey { line: header_no })?;
    let c = slots[2].ok_or(TierError::MissingKey { line: header_no })?;
    let d = slots[3].unwrap_or(PRIMUS_TIER);
    Ok([a, b, c, d])
}

/// Feint state: a brief attack bonus after the foe releases an attack in
/// close range (a probe the foe reacted to, then a commit). A scoring
/// input only; it never re-times the perception clock and never hides
/// information (it reads the same [`Percept`] the core scores).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FeintState {
    window: u32,
    cooldown: u32,
}

impl FeintState {
    /// Idle feint state (no window, no cooldown).
    pub const fn new() -> Self {
        Self {
            window: 0,
            cooldown: 0,
        }
    }

    /// True while the strike bonus applies.
    pub const fn is_feinting(self) -> bool {
        self.window > 0
    }

    /// Advance one tick: a close attack-release opens the window, the
    /// window ticks down while the cooldown waits, then the cooldown ticks
    /// down. Total over all inputs.
    pub fn update(&mut self, foe_attacking: bool, foe_was_attacking: bool, dist_mm: i32) {
        if self.window > 0 {
            self.window -= 1;
        } else if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        if self.window == 0
            && self.cooldown == 0
            && foe_was_attacking
            && !foe_attacking
            && dist_mm <= CLOSE_MM
        {
            self.window = FEINT_WINDOW_TICKS;
            self.cooldown = FEINT_COOLDOWN_TICKS;
        }
    }

    /// Additive score bias in scripted table order: attack only, while feinting.
    pub const fn score_bias(self) -> [i32; 5] {
        if self.window > 0 {
            [FEINT_ATTACK_BONUS, 0, 0, 0, 0]
        } else {
            [0; 5]
        }
    }
}

impl Default for FeintState {
    fn default() -> Self {
        Self::new()
    }
}

/// Stamina cost per emitted intent. The arena applies this same table to
/// the player, so the AI paces under the same rules (no stat cheats).
pub const fn stamina_cost(intent: Intent) -> i32 {
    match intent {
        Intent::Attack => 40,
        Intent::Dodge => 30,
        Intent::Block => 10,
        Intent::Advance => 12,
        Intent::Retreat => 8,
        Intent::Reposition => 12,
    }
}

/// Stamina pacing bias in scripted table order: fresh fighters fence nothing,
/// winded fighters guard and give ground, gassed fighters stop committing.
pub const fn stamina_gate_bias(stamina: i32) -> [i32; 5] {
    if stamina >= STAMINA_WINDED_BELOW {
        [0; 5]
    } else if stamina >= STAMINA_GASSED_BELOW {
        [-100, 50, -75, -25, 50]
    } else {
        [-250, 100, -200, -100, 125]
    }
}

/// Arena-composed Rome policy: the four gladiator scoring inputs (match
/// phase, feint window, stamina pacing, squad role) as one arena-owned
/// value. The arena builds it, drives it from round progress and squad
/// triggers, and installs it on a [`DepthBrain`]; the generic integer core
/// in `gladiator.rs` only ever sees the summed `[i32; 5]` bias through
/// [`Brain::decide_with_bias`](crate::gladiator::Brain::decide_with_bias),
/// so a fixture genre composes a different policy over the same core.
/// Integer math only, zero allocation. Budget: a dozen saturating adds per
/// bias read, O(1) feint update per tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArenaPolicy {
    phase: Phase,
    role: Role,
    feint: FeintState,
    stamina: i32,
}

impl ArenaPolicy {
    /// New arena policy: [`Phase::Open`], [`Role::Flanker`], full stamina,
    /// idle feint (exactly the [`DepthBrain`] construction defaults).
    pub const fn new() -> Self {
        Self {
            phase: Phase::Open,
            role: Role::Flanker,
            feint: FeintState::new(),
            stamina: STAMINA_MAX,
        }
    }

    /// Current phase.
    pub const fn phase(self) -> Phase {
        self.phase
    }

    /// Install the phase (the arena drives this from round progress).
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    /// Current squad role.
    pub const fn role(self) -> Role {
        self.role
    }

    /// Install the role from the [`SquadN`] assignment (reassign boundaries
    /// only, never mid-swing).
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Current stamina (`0..=1000`).
    pub const fn stamina(self) -> i32 {
        self.stamina
    }

    /// Set stamina, clamped to `0..=1000`. Returns the clamped value.
    pub fn set_stamina(&mut self, stamina: i32) -> i32 {
        self.stamina = stamina.clamp(STAMINA_MIN, STAMINA_MAX);
        self.stamina
    }

    /// True at or below [`STAMINA_COLLAPSE_AT`] (squad reassign trigger).
    pub const fn is_collapsed(self) -> bool {
        self.stamina <= STAMINA_COLLAPSE_AT
    }

    /// True while the feint strike bonus applies.
    pub const fn is_feinting(self) -> bool {
        self.feint.is_feinting()
    }

    /// Advance the feint detector one tick: a close attack-release opens
    /// the window, the window then times out under cooldown. Total over
    /// all inputs.
    pub fn update_feint(&mut self, foe_attacking: bool, foe_was_attacking: bool, dist_mm: i32) {
        self.feint.update(foe_attacking, foe_was_attacking, dist_mm);
    }

    /// Round boundary: install the next phase and clear the feint window.
    /// Stamina carries over; the squad reassigns separately.
    pub fn end_round(&mut self, phase: Phase) {
        self.phase = phase;
        self.feint = FeintState::new();
    }

    /// Combined bias in scripted table order: phase plus feint plus stamina
    /// gate plus role, saturating. Bounded to `+/-445`.
    pub fn bias(self) -> [i32; 5] {
        let phase = self.phase.score_bias();
        let feint = self.feint.score_bias();
        let gate = stamina_gate_bias(self.stamina);
        let role = self.role.score_bias();
        [
            phase[IDX_ATTACK]
                .saturating_add(feint[IDX_ATTACK])
                .saturating_add(gate[IDX_ATTACK])
                .saturating_add(role[IDX_ATTACK]),
            phase[IDX_BLOCK]
                .saturating_add(feint[IDX_BLOCK])
                .saturating_add(gate[IDX_BLOCK])
                .saturating_add(role[IDX_BLOCK]),
            phase[IDX_DODGE]
                .saturating_add(feint[IDX_DODGE])
                .saturating_add(gate[IDX_DODGE])
                .saturating_add(role[IDX_DODGE]),
            phase[IDX_ADVANCE]
                .saturating_add(feint[IDX_ADVANCE])
                .saturating_add(gate[IDX_ADVANCE])
                .saturating_add(role[IDX_ADVANCE]),
            phase[IDX_RETREAT]
                .saturating_add(feint[IDX_RETREAT])
                .saturating_add(gate[IDX_RETREAT])
                .saturating_add(role[IDX_RETREAT]),
        ]
    }
}

impl Default for ArenaPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Gladiator-depth brain: the scripted [`Brain`] plus an arena-composed
/// [`ArenaPolicy`] scoring input. The core keeps its perception clock,
/// far-streak anti-stuck and tie-break order; depth only adds the composed
/// bounded integer bias through [`Brain::decide_with_bias`], then steers the
/// emitted wish through the steer wall-escape/foe-seeking post-pass (wish
/// vector only; scores, buttons and facing pass through untouched).
#[derive(Clone, Debug)]
pub struct DepthBrain {
    inner: Brain,
    fighter: FighterId,
    policy: ArenaPolicy,
    foe_was_attacking: bool,
    half_extent_mm: i32,
    personality: Option<Personality>,
    threat_history: u8,
    tier: Option<TierRow>,
}

impl DepthBrain {
    /// New depth brain for `fighter`; `period_ticks == 0` means every tick.
    /// Starts in [`Phase::Open`] as a [`Role::Flanker`] at full stamina.
    pub fn new(fighter: FighterId, period_ticks: u32) -> Self {
        Self {
            inner: Brain::new(fighter, period_ticks),
            fighter,
            policy: ArenaPolicy::new(),
            foe_was_attacking: false,
            half_extent_mm: DEFAULT_HALF_EXTENT_MM,
            personality: None,
            threat_history: 0,
            tier: None,
        }
    }

    /// Depth brain from an immutable [`AiConfig`] snapshot (same fallback
    /// rule as [`Brain::with_config`]: invalid snapshots use scripted
    /// defaults). Never panics.
    pub fn with_config(fighter: FighterId, period_ticks: u32, config: AiConfig) -> Self {
        Self {
            inner: Brain::with_config(fighter, period_ticks, config),
            fighter,
            policy: ArenaPolicy::new(),
            foe_was_attacking: false,
            half_extent_mm: DEFAULT_HALF_EXTENT_MM,
            personality: None,
            threat_history: 0,
            tier: None,
        }
    }

    /// The installed arena policy (the composed Rome scoring inputs).
    pub const fn policy(&self) -> ArenaPolicy {
        self.policy
    }

    /// Bound fighter slot.
    pub const fn fighter(&self) -> FighterId {
        self.fighter
    }

    /// The installed temperament, if any (`None` decides exactly as a brain
    /// built before temperaments existed).
    pub const fn personality(&self) -> Option<Personality> {
        self.personality
    }

    /// Install a temperament: its reaction offset delays when the foe's blow
    /// is noticed, its preferred range holds the swing beyond it and its
    /// aggression leans the score lanes. Set before the first tick; the
    /// arena derives it from the match seed and slot.
    pub fn set_personality(&mut self, personality: Personality) {
        self.personality = Some(personality);
    }

    /// Current phase.
    pub const fn phase(&self) -> Phase {
        self.policy.phase
    }

    /// Set the phase (the arena drives this from round progress).
    pub fn set_phase(&mut self, phase: Phase) {
        self.policy.set_phase(phase);
    }

    /// Current squad role.
    pub const fn role(&self) -> Role {
        self.policy.role
    }

    /// Set the role from the [`SquadN`] assignment (reassign boundaries
    /// only, never mid-swing).
    pub fn set_role(&mut self, role: Role) {
        self.policy.set_role(role);
    }

    /// Installed duel tier, if any (`None` decides as before).
    pub const fn tier(&self) -> Option<TierRow> {
        self.tier
    }

    /// Install a duel tier: its role pins the policy and its reaction plus
    /// aggression shape the temperament. The scored decide path re-reads
    /// both every tick into the gladiator core, so tiers climb in skill
    /// while health stays in the narrow band. Set before the first tick.
    pub fn set_tier(&mut self, tier: TierRow) {
        self.tier = Some(tier);
        self.policy.set_role(tier.role());
        self.personality = Some(tier.personality());
    }

    /// Arena half-extent in metres for the wall-escape band.
    pub fn half_extent_m(&self) -> f32 {
        self.half_extent_mm as f32 / 1000.0
    }

    /// Set the arena half-extent in metres. Finite values inside the arena
    /// range `5.0..=100.0` stick; anything else refuses-and-falls-back to
    /// the standard arena (`20.0`). Returns the stored extent. Never panics.
    pub fn set_half_extent_m(&mut self, half_extent_m: f32) -> f32 {
        self.half_extent_mm = if half_extent_m.is_finite() && (5.0..=100.0).contains(&half_extent_m)
        {
            // Finite and in range: the truncating cast cannot saturate.
            (half_extent_m * 1000.0) as i32
        } else {
            DEFAULT_HALF_EXTENT_MM
        };
        self.half_extent_m()
    }

    /// Current stamina (`0..=1000`).
    pub const fn stamina(&self) -> i32 {
        self.policy.stamina
    }

    /// Set stamina, clamped to `0..=1000`. Returns the clamped value.
    pub fn set_stamina(&mut self, stamina: i32) -> i32 {
        self.policy.set_stamina(stamina)
    }

    /// True at or below [`STAMINA_COLLAPSE_AT`] (squad reassign trigger).
    pub const fn is_collapsed(&self) -> bool {
        self.policy.is_collapsed()
    }

    /// True while the feint strike bonus applies.
    pub const fn is_feinting(&self) -> bool {
        self.policy.is_feinting()
    }

    /// Current morale (`0..=1000`, owned by the inner scripted core).
    pub const fn morale(&self) -> i32 {
        self.inner.morale()
    }

    /// Set morale, clamped to `0..=1000`. Returns the clamped value.
    pub fn set_morale(&mut self, morale: i32) -> i32 {
        self.inner.set_morale(morale)
    }

    /// Apply one tension-table morale step, clamped. Returns the result.
    pub fn apply_morale_drift(&mut self, delta: i32) -> i32 {
        self.inner
            .set_morale(self.inner.morale().saturating_add(delta))
    }

    /// Last decided intent (initially [`Intent::Advance`]).
    pub const fn intent(&self) -> Intent {
        self.inner.intent()
    }

    /// Round boundary: install the next phase and clear the feint window.
    /// Stamina and morale carry over; the squad reassigns separately.
    pub fn end_round(&mut self, phase: Phase) {
        self.policy.end_round(phase);
        self.foe_was_attacking = false;
    }

    /// Combined depth bias in scripted table order: the installed
    /// [`ArenaPolicy`] sum (phase plus feint plus stamina gate plus role,
    /// saturating). Bounded to `+/-445`.
    pub fn score_bias(&self) -> [i32; 5] {
        self.policy.bias()
    }

    /// Decide one [`FighterInput`] for `percept`: track the feint release,
    /// score through the scripted core with the depth bias, steer the emitted
    /// wish off blocked axes and toward a far foe, then map a tier brain's
    /// facing-relative wish into the world axes a legacy arena reads, and
    /// charge stamina for the emitted intent. Always deterministic and always
    /// passes [`FighterInput::validate`]. Budget: the steering post-pass plus
    /// the tier wish map are a few dozen integer ops with zero allocation.
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    pub fn decide(&mut self, percept: &Percept) -> FighterInput {
        if let Some(tier) = self.tier {
            self.policy.set_role(tier.role());
            self.personality = Some(tier.personality());
        }
        let dist_mm = sanitize_dist_mm(percept.dist_m);
        let seen = self.perceive(percept);
        let percept = &seen;
        self.policy
            .update_feint(percept.foe_attacking, self.foe_was_attacking, dist_mm);
        self.foe_was_attacking = percept.foe_attacking;
        let bias = self.edge_bias(percept, dist_mm);
        let scored = self.inner.decide_with_bias(percept, bias);
        // Steering: re-aim the scored wish only. Scores, buttons and
        // facing pass through untouched, so the curves still decide every
        // fight action; the wish changes solely on wall contact (a wish the
        // clamp would eat slides off the wall) or far-foe stillness (a dead
        // wish beyond legion range closes in).
        let (mx, mz) = steer::steer_wish(
            pos_mm(percept.self_pos[0]),
            pos_mm(percept.self_pos[2]),
            pos_mm(percept.foe_pos[0]),
            pos_mm(percept.foe_pos[2]),
            scored.mx,
            scored.mz,
            self.half_extent_mm,
        );
        let input = FighterInput { mx, mz, ..scored };
        // A tier brain is the shipped duel enemy: when its arena hands it no
        // melee view (the legacy rules that read the wish pair as world axes,
        // not the facing-relative scored wish), it maps its own wish into the
        // foe frame so the rung can close and fence. A rules-v2 arena maps
        // the same wish itself and attaches a live view, so this never doubles.
        let input = if self.tier.is_some() && !percept.melee.is_live() {
            let mapped = tier_wish_to_world(input, self.inner.intent(), percept, dist_mm);
            let (mx, mz) = steer::steer_wish(
                pos_mm(percept.self_pos[0]),
                pos_mm(percept.self_pos[2]),
                pos_mm(percept.foe_pos[0]),
                pos_mm(percept.foe_pos[2]),
                mapped.mx,
                mapped.mz,
                self.half_extent_mm,
            );
            FighterInput { mx, mz, ..mapped }
        } else {
            input
        };
        let drained = self
            .policy
            .stamina()
            .saturating_sub(stamina_cost(self.inner.intent()))
            .saturating_add(STAMINA_RECOVERY);
        self.policy.set_stamina(drained);
        input
    }
}

impl DepthBrain {
    /// What this brain notices of `percept`. A legacy percept with no
    /// temperament passes through untouched. Otherwise the foe's threat (a
    /// swing in windup or on its impact tick, from the melee view when one is
    /// attached) enters a shift register and the brain sees it
    /// `reaction_ticks` late; the tick the delayed threat first shows, the
    /// perception clock is interrupted so the reply is not left waiting for
    /// the next scheduled look.
    fn perceive(&mut self, percept: &Percept) -> Percept {
        let live = percept.melee.is_live();
        if !live && self.personality.is_none() {
            return *percept;
        }
        let threat = if live {
            percept.melee.target().phase.is_threat()
        } else {
            percept.foe_attacking
        };
        self.threat_history = (self.threat_history << 1) | u8::from(threat);
        let lag = u32::from(self.personality.map_or(0, |p| p.reaction_ticks()));
        let now = (self.threat_history >> lag) & 1 == 1;
        let before = (self.threat_history >> (lag + 1)) & 1 == 1;
        if now && !before {
            self.inner.rearm();
        }
        Percept {
            foe_attacking: now,
            ..*percept
        }
    }

    /// Depth bias for this tick: the installed arena policy, the melee view
    /// (the real stamina replaces the brain's running estimate while one is
    /// attached) and the temperament, summed with saturating adds.
    fn edge_bias(&mut self, percept: &Percept, dist_mm: i32) -> [i32; 5] {
        if percept.melee.is_live() {
            self.policy
                .set_stamina(i32::from(percept.melee.own_stamina_permille()));
        }
        let mut bias = self.policy.bias();
        add_bias(&mut bias, percept.melee.score_bias(dist_mm));
        if let Some(personality) = &self.personality {
            add_bias(&mut bias, personality.score_bias(dist_mm));
        }
        bias
    }
}

/// Ground-plane coordinate in millimetres for the steering post-pass;
/// non-finite maps to arena center, arena scale clamps first so the
/// truncating cast cannot wrap.
fn pos_mm(coord_m: f32) -> i32 {
    if !coord_m.is_finite() {
        return 0;
    }
    (coord_m.clamp(-1_000.0, 1_000.0) * 1000.0) as i32
}

/// Map one decided [`FighterInput`]'s facing-relative wish into world axes.
///
/// The scored wish is facing-relative (`mx` forward at the foe, `mz` to the
/// side) while the arena's locomotion reads `FighterInput::mx`/`mz` as world
/// axes. Rules v2 rebuilds the wish from the intent in the foe frame, so the
/// brain must not; the legacy v1 arena reads the pair as world motion, so a
/// tier brain maps its own wish or `Intent::Advance` walks along world +X
/// instead of at the foe and the pair never closes. Magnitudes match the
/// scripted emit table; an attack beyond the tier's own swing spacing closes
/// in at full wish instead of holding (the strike band was tuned for the v1
/// rangeless hit). Facing, buttons and the tick pass through. Integer-only
/// (`isqrt`), no allocation, coincident fighters stand still.
fn tier_wish_to_world(
    mut decided: FighterInput,
    intent: Intent,
    percept: &Percept,
    dist_mm: i32,
) -> FighterInput {
    let me = [pos_mm(percept.self_pos[0]), pos_mm(percept.self_pos[2])];
    let foe = [pos_mm(percept.foe_pos[0]), pos_mm(percept.foe_pos[2])];
    let dx = i64::from(foe[0]) - i64::from(me[0]);
    let dz = i64::from(foe[1]) - i64::from(me[1]);
    let len = i64::try_from(u64::try_from(dx * dx + dz * dz).unwrap_or(0).isqrt()).unwrap_or(0);
    if len == 0 {
        decided.mx = 0;
        decided.mz = 0;
        return decided;
    }
    let side = if decided.mz < 0 { -1 } else { 1 };
    let (forward, aside): (i64, i64) = match intent {
        Intent::Attack if dist_mm > i32::from(TIER_RANGE_MM) => (100, 0),
        Intent::Attack => (30, 0),
        Intent::Block => (0, 0),
        Intent::Dodge => (0, 100 * side),
        Intent::Advance => (100, 0),
        Intent::Retreat => (-80, 0),
        Intent::Reposition => (60, 80 * side),
    };
    let axis = |v: i64| i8::try_from((v / len).clamp(-100, 100)).unwrap_or(0);
    decided.mx = axis(forward * dx + aside * dz);
    decided.mz = axis(forward * dz - aside * dx);
    decided
}

/// Squad: one holder pins the center, the rest circle as flankers. This is
/// the arena-width squad ([`MAX_PARTICIPANTS`] slots); a fixture genre
/// seats any smaller team in [`SquadN`] (a duel is a `SquadN<2>`). Member
/// slots always validate against [`MAX_PARTICIPANTS`]. Members still decide
/// alone; the squad only assigns roles. Arbitration rule: roles reassign
/// only on knockdown, stamina collapse or a round boundary (the three
/// `on_*` methods) — never mid-swing, never on a timer tick — so squad
/// behavior replays exactly from the same tape. Zero allocation: two
/// inline arrays plus a length.
///
/// [`MAX_PARTICIPANTS`]: engine_input::MAX_PARTICIPANTS
pub type Squad = SquadN<{ MAX_PARTICIPANTS as usize }>;

/// Length-parameterized squad: one holder pins the center, up to `N - 1`
/// flankers circle. The arena seats [`Squad`]; fixture genres seat any
/// smaller width without paying arena capacity. Same arbitration rule as
/// [`Squad`], same errors, same determinism. Zero allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SquadN<const N: usize> {
    members: [FighterId; N],
    roles: [Role; N],
    len: usize,
    tokens: [[u8; MAX_ATTACKERS_PER_TARGET]; SLOT_ROWS],
}

/// Attackers that may engage one target at once: the attack-token cap. A
/// third attacker waits until a holder releases.
pub const MAX_ATTACKERS_PER_TARGET: usize = 2;

/// Empty attack-token cell.
const NO_TOKEN: u8 = u8::MAX;

/// Arena slots as an array length, counted up in const context (no cast).
const SLOT_ROWS: usize = {
    let mut rows = 0usize;
    let mut id = 0u8;
    while id < MAX_PARTICIPANTS {
        rows += 1;
        id += 1;
    }
    rows
};

impl<const N: usize> SquadN<N> {
    /// New squad: members sort by fighter id, the lowest id holds, the
    /// rest flank. Empty input, squads longer than `N`, out-of-range slots
    /// and duplicates refuse loudly. Never panics.
    pub fn new(members: &[FighterId]) -> Result<Self, DepthError> {
        if members.is_empty() {
            return Err(DepthError::EmptySquad);
        }
        if members.len() > N {
            return Err(DepthError::UnknownFighter { id: members[N].0 });
        }
        let mut sorted: [FighterId; N] = [FighterId(0); N];
        for (index, member) in members.iter().enumerate() {
            if member.0 >= MAX_PARTICIPANTS {
                return Err(DepthError::UnknownFighter { id: member.0 });
            }
            sorted[index] = *member;
        }
        sorted[..members.len()].sort_by_key(|id| id.0);
        for window in sorted[..members.len()].windows(2) {
            if window[0] == window[1] {
                return Err(DepthError::DuplicateFighter { id: window[0].0 });
            }
        }
        let mut roles = [Role::Flanker; N];
        roles[0] = Role::Holder;
        Ok(Self {
            members: sorted,
            roles,
            len: members.len(),
            tokens: [[NO_TOKEN; MAX_ATTACKERS_PER_TARGET]; SLOT_ROWS],
        })
    }

    /// Squad members in fighter-id order.
    pub fn members(&self) -> &[FighterId] {
        &self.members[..self.len]
    }

    /// Roles parallel to [`SquadN::members`].
    pub fn roles(&self) -> &[Role] {
        &self.roles[..self.len]
    }

    /// Current holder (exactly one by construction).
    pub fn holder(&self) -> FighterId {
        for (index, member) in self.members().iter().enumerate() {
            if self.roles[index] == Role::Holder {
                return *member;
            }
        }
        // Unreachable: construction seats one holder and every reassign
        // preserves exactly one. Fall back to the lowest id, never panic.
        self.members[0]
    }

    /// Role of one member; non-members refuse with
    /// [`DepthError::UnknownFighter`].
    pub fn role_of(&self, id: FighterId) -> Result<Role, DepthError> {
        for (index, member) in self.members().iter().enumerate() {
            if *member == id {
                return Ok(self.roles[index]);
            }
        }
        Err(DepthError::UnknownFighter { id: id.0 })
    }

    /// Ask for an attack token on `target`: at most
    /// [`MAX_ATTACKERS_PER_TARGET`] attackers hold one per target, and an
    /// attacker holds one at a time. `Ok(true)` grants (or re-confirms) the
    /// token and drops the attacker's token on any other target; `Ok(false)`
    /// means the target is full and the attacker waits, keeping what it
    /// held. A non-member attacker, a target slot at or above
    /// [`MAX_PARTICIPANTS`] or a fighter naming itself refuses loudly.
    pub fn claim_token(
        &mut self,
        attacker: FighterId,
        target: FighterId,
    ) -> Result<bool, DepthError> {
        self.role_of(attacker)?;
        if target.0 >= MAX_PARTICIPANTS {
            return Err(DepthError::UnknownFighter { id: target.0 });
        }
        if attacker == target {
            return Err(DepthError::SelfTarget { id: target.0 });
        }
        if self.target_of(attacker) == Some(target) {
            return Ok(true);
        }
        let row = usize::from(target.0);
        let Some(free) = self.tokens[row].iter().position(|cell| *cell == NO_TOKEN) else {
            return Ok(false);
        };
        self.release_token(attacker);
        self.tokens[row][free] = attacker.0;
        Ok(true)
    }

    /// Drop whatever token `attacker` holds (a no-op when it holds none).
    pub fn release_token(&mut self, attacker: FighterId) {
        for row in &mut self.tokens {
            for cell in row.iter_mut() {
                if *cell == attacker.0 {
                    *cell = NO_TOKEN;
                }
            }
        }
    }

    /// Tokens currently held on `target`; never above
    /// [`MAX_ATTACKERS_PER_TARGET`]. A slot outside the arena holds none.
    pub fn tokens_on(&self, target: FighterId) -> usize {
        self.tokens.get(usize::from(target.0)).map_or(0, |row| {
            row.iter().filter(|cell| **cell != NO_TOKEN).count()
        })
    }

    /// The target `attacker` holds a token on, if any.
    pub fn target_of(&self, attacker: FighterId) -> Option<FighterId> {
        let row = self
            .tokens
            .iter()
            .position(|row| row.contains(&attacker.0))?;
        u8::try_from(row).ok().map(FighterId)
    }

    /// Release every token (a new engagement round).
    pub fn clear_tokens(&mut self) {
        self.tokens = [[NO_TOKEN; MAX_ATTACKERS_PER_TARGET]; SLOT_ROWS];
    }

    /// Stamina lookup over a member table; members absent from the table
    /// read as `0`. Total over all inputs.
    fn stamina_of(table: &[(FighterId, i32)], id: FighterId) -> i32 {
        for (member, stamina) in table {
            if *member == id {
                return *stamina;
            }
        }
        0
    }

    /// Every id in `ids` and `table` must be a member, else
    /// [`DepthError::UnknownFighter`].
    fn check_ids(&self, ids: &[FighterId], table: &[(FighterId, i32)]) -> Result<(), DepthError> {
        for id in ids {
            self.role_of(*id)?;
        }
        for (id, _) in table {
            self.role_of(*id)?;
        }
        Ok(())
    }

    /// Seat `holder` as the holder, everyone else as flankers.
    fn seat(&mut self, holder: FighterId) {
        for (index, member) in self.members[..self.len].iter().enumerate() {
            self.roles[index] = if *member == holder {
                Role::Holder
            } else {
                Role::Flanker
            };
        }
    }

    /// Pick the highest-stamina candidate; ties break by lowest fighter
    /// id. `exclude` marks ineligible members (down or collapsed).
    fn promote_pick(&self, table: &[(FighterId, i32)], exclude: &[FighterId]) -> Option<FighterId> {
        let mut best: Option<(FighterId, i32)> = None;
        // Members iterate in fighter-id order, so strict `>` keeps the
        // lowest id on stamina ties.
        for member in self.members() {
            if exclude.contains(member) {
                continue;
            }
            let stamina = Self::stamina_of(table, *member);
            match best {
                None => best = Some((*member, stamina)),
                Some((_, top)) if stamina > top => best = Some((*member, stamina)),
                _ => {}
            }
        }
        best.map(|(id, _)| id)
    }

    /// Knockdown reassign: a down holder promotes the highest-stamina
    /// standing flanker (ties to the lowest id). A standing holder, or a
    /// squad with nobody standing, keeps its roles.
    pub fn on_knockdown(
        &mut self,
        down: &[FighterId],
        stamina: &[(FighterId, i32)],
    ) -> Result<(), DepthError> {
        self.check_ids(down, stamina)?;
        if !down.contains(&self.holder()) {
            return Ok(());
        }
        if let Some(next) = self.promote_pick(stamina, down) {
            self.seat(next);
        }
        Ok(())
    }

    /// Stamina-collapse reassign: a collapsed holder (at or below
    /// [`STAMINA_COLLAPSE_AT`]) promotes the highest-stamina uncollapsed
    /// flanker. Nobody collapsed, or nobody left standing, keeps roles.
    pub fn on_stamina_collapse(&mut self, stamina: &[(FighterId, i32)]) -> Result<(), DepthError> {
        self.check_ids(&[], stamina)?;
        let mut buf = [FighterId(0); N];
        let mut count = 0usize;
        for member in self.members() {
            if Self::stamina_of(stamina, *member) <= STAMINA_COLLAPSE_AT {
                buf[count] = *member;
                count += 1;
            }
        }
        let down: &[FighterId] = &buf[..count];
        if !down.contains(&self.holder()) {
            return Ok(());
        }
        if let Some(next) = self.promote_pick(stamina, down) {
            self.seat(next);
        }
        Ok(())
    }
}

/// Difficulty tension row for one ladder rung: crowd volume (permille for
/// the crowd mix), spawn interval (ticks) and morale drift per minute.
/// Pacing and composition only — never stat cheats, never hidden
/// information; the same rules bind AI and player.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TensionRow {
    /// Crowd volume in permille (`0..=1000`).
    pub crowd_volume_permille: u16,
    /// Ticks between spawns.
    pub spawn_ticks: u32,
    /// Morale drift applied per 60 ticks (`-500..=500`).
    pub morale_drift: i32,
}

/// Ladder rungs (`0..=7`).
pub const LADDER_RUNGS: u8 = 8;
/// Highest ladder rung.
pub const MAX_RUNG: u8 = LADDER_RUNGS - 1;
/// Fence: quietest legal crowd.
pub const VOLUME_MIN_PERMILLE: u16 = 100;
/// Fence: loudest legal crowd.
pub const VOLUME_MAX_PERMILLE: u16 = 1_000;
/// Fence: fastest legal spawn cadence in ticks.
pub const SPAWN_MIN_TICKS: u32 = 30;
/// Fence: slowest legal spawn cadence in ticks.
pub const SPAWN_MAX_TICKS: u32 = 600;
/// Fence: most negative legal morale drift.
pub const DRIFT_MIN: i32 = -200;
/// Fence: most positive legal morale drift.
pub const DRIFT_MAX: i32 = 200;

/// Shipped tension ladder, rung `0` (quiet) to rung `7` (cauldron).
/// Volume climbs monotonically; every row sits inside its fences.
pub const TENSION_LADDER: [TensionRow; LADDER_RUNGS as usize] = [
    TensionRow {
        crowd_volume_permille: 200,
        spawn_ticks: 480,
        morale_drift: -100,
    },
    TensionRow {
        crowd_volume_permille: 300,
        spawn_ticks: 420,
        morale_drift: -50,
    },
    TensionRow {
        crowd_volume_permille: 400,
        spawn_ticks: 360,
        morale_drift: 0,
    },
    TensionRow {
        crowd_volume_permille: 500,
        spawn_ticks: 300,
        morale_drift: 0,
    },
    TensionRow {
        crowd_volume_permille: 600,
        spawn_ticks: 240,
        morale_drift: 50,
    },
    TensionRow {
        crowd_volume_permille: 700,
        spawn_ticks: 180,
        morale_drift: 75,
    },
    TensionRow {
        crowd_volume_permille: 850,
        spawn_ticks: 120,
        morale_drift: 100,
    },
    TensionRow {
        crowd_volume_permille: 1_000,
        spawn_ticks: 60,
        morale_drift: 150,
    },
];

/// Validate one tension row against its fences. Returns the first failure.
pub fn validate_tension_row(rung: u8, row: &TensionRow) -> Result<(), DepthError> {
    if row.crowd_volume_permille < VOLUME_MIN_PERMILLE
        || row.crowd_volume_permille > VOLUME_MAX_PERMILLE
    {
        return Err(DepthError::TableOutOfFence {
            rung,
            field: "crowd_volume",
            value: i32::from(row.crowd_volume_permille),
        });
    }
    if row.spawn_ticks < SPAWN_MIN_TICKS || row.spawn_ticks > SPAWN_MAX_TICKS {
        return Err(DepthError::TableOutOfFence {
            rung,
            field: "spawn_ticks",
            value: row.spawn_ticks as i32,
        });
    }
    if row.morale_drift < DRIFT_MIN || row.morale_drift > DRIFT_MAX {
        return Err(DepthError::TableOutOfFence {
            rung,
            field: "morale_drift",
            value: row.morale_drift,
        });
    }
    Ok(())
}

/// Tension row for `rung`. Unknown rungs refuse with
/// [`DepthError::UnknownRung`].
pub fn tension_row(rung: u8) -> Result<TensionRow, DepthError> {
    if rung >= LADDER_RUNGS {
        return Err(DepthError::UnknownRung { rung });
    }
    Ok(TENSION_LADDER[usize::from(rung)])
}

/// Crowd volume in permille for `rung` (data for the crowd mix, never a
/// stat).
pub fn crowd_volume(rung: u8) -> Result<u16, DepthError> {
    tension_row(rung).map(|row| row.crowd_volume_permille)
}

/// True once `spawn_ticks` have passed since `last_spawn_tick`.
/// Wrapping-safe so long tape runs cannot misfire.
pub fn spawn_due(rung: u8, tick: u32, last_spawn_tick: u32) -> Result<bool, DepthError> {
    tension_row(rung).map(|row| tick.wrapping_sub(last_spawn_tick) >= row.spawn_ticks)
}

/// Morale step for `rung` at `tick`: the rung drift once per 60 ticks,
/// `0` otherwise. No live bout calls this: morale moves only offline, through
/// the fork proposal (`SetAggression` in `engine-arena`) and the editor tuning
/// panel. The arena would add it through [`DepthBrain::apply_morale_drift`]
/// once the rung arrives with the match config.
pub fn morale_step(rung: u8, tick: u32) -> Result<i32, DepthError> {
    tension_row(rung).map(|row| {
        if tick.is_multiple_of(60) {
            row.morale_drift
        } else {
            0
        }
    })
}

/// Difficulty ladder: rung into the tension table. Replayable by
/// construction (same rung gives the same row).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ladder {
    rung: u8,
}

/// Rung recorded for the progression payload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LadderProgress {
    /// Current rung.
    pub rung: u8,
}

impl Ladder {
    /// Highest rung.
    pub const MAX_RUNG: u8 = MAX_RUNG;

    /// New ladder at `rung`. Unknown rungs refuse loudly.
    pub fn new(rung: u8) -> Result<Self, DepthError> {
        if rung >= LADDER_RUNGS {
            return Err(DepthError::UnknownRung { rung });
        }
        Ok(Self { rung })
    }

    /// Current rung.
    pub const fn rung(&self) -> u8 {
        self.rung
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::director::{pick_spawn, Budget, SpawnRow};
    use crate::melee::{FoeView, MeleeView, StrikePhase};
    use engine_input::fighter::{ATTACK, BLOCK, DODGE};

    fn fighter(id: u8) -> FighterId {
        FighterId::new(id).expect("slot is legal")
    }

    fn percept(tick: u32, dist_m: f32, foe_attacking: bool) -> Percept {
        Percept::new(
            [0.0, 0.0, 0.0],
            0.0,
            [dist_m, 0.0, 0.0],
            foe_attacking,
            dist_m,
            tick,
        )
    }

    /// Fixed 60 s script: integer math only, no RNG, no trig.
    fn script_percept(fighter_id: u8, tick: u32) -> Percept {
        let self_pos = [(tick % 13) as f32 * 0.5, 0.0, (tick % 11) as f32 * -0.25];
        let foe_pos = [
            (tick % 17) as f32 * 0.75 - 4.0 + fighter_id as f32,
            0.0,
            ((tick + u32::from(fighter_id)) % 7) as f32 - 3.0,
        ];
        let dist_m = 1.0 + ((tick * 37) % 120) as f32 * 0.1;
        Percept::new(
            self_pos,
            (tick % 5) as f32 * 0.3,
            foe_pos,
            tick.is_multiple_of(3),
            dist_m,
            tick,
        )
    }

    fn run_depth(id: u8, ticks: u32) -> Vec<FighterInput> {
        let mut brain = DepthBrain::new(fighter(id), 6);
        brain.set_phase(Phase::for_tick(0, ticks));
        (0..ticks)
            .map(|tick| {
                brain.set_phase(Phase::for_tick(tick, ticks));
                brain.apply_morale_drift(morale_step(3, tick).expect("rung 3 ships"));
                brain.decide(&script_percept(id, tick))
            })
            .collect()
    }

    #[test]
    fn unknown_phase_and_role_refuse_loudly() {
        assert_eq!(
            Phase::from_u8(3),
            Err(DepthError::UnknownPhase { value: 3 })
        );
        assert_eq!(
            Phase::from_u8(255),
            Err(DepthError::UnknownPhase { value: 255 })
        );
        assert_eq!(Phase::from_u8(0), Ok(Phase::Open));
        assert_eq!(Phase::from_u8(1), Ok(Phase::Pressure));
        assert_eq!(Phase::from_u8(2), Ok(Phase::Finish));
        assert_eq!(
            Phase::Open.as_u8() + Phase::Pressure.as_u8() + Phase::Finish.as_u8(),
            3
        );
        assert_eq!(Role::from_u8(2), Err(DepthError::UnknownRole { value: 2 }));
        assert_eq!(Role::from_u8(0), Ok(Role::Holder));
        assert_eq!(Role::from_u8(1), Ok(Role::Flanker));
        assert_eq!(Role::Holder.as_u8() + Role::Flanker.as_u8(), 1);
    }

    #[test]
    fn phase_from_round_thirds_with_goldens() {
        assert_eq!(Phase::for_tick(0, 900), Phase::Open);
        assert_eq!(Phase::for_tick(299, 900), Phase::Open);
        assert_eq!(Phase::for_tick(300, 900), Phase::Pressure);
        assert_eq!(Phase::for_tick(599, 900), Phase::Pressure);
        assert_eq!(Phase::for_tick(600, 900), Phase::Finish);
        assert_eq!(Phase::for_tick(899, 900), Phase::Finish);
        assert_eq!(Phase::for_tick(9_999, 900), Phase::Finish);
        assert_eq!(Phase::for_tick(0, 0), Phase::Open);
        // Finish commits hardest, open probes: bounded, ordered.
        let (open, pressure, finish) = (
            Phase::Open.score_bias(),
            Phase::Pressure.score_bias(),
            Phase::Finish.score_bias(),
        );
        for lane in [open, pressure, finish] {
            for bias in lane {
                assert!((-150..=150).contains(&bias), "{lane:?}");
            }
        }
        assert!(finish[IDX_ATTACK] > pressure[IDX_ATTACK]);
        assert!(pressure[IDX_ATTACK] > open[IDX_ATTACK]);
        assert!(open[IDX_ADVANCE] > finish[IDX_ADVANCE]);
    }

    #[test]
    fn feint_triggers_on_close_release_then_expires() {
        let mut brain = DepthBrain::new(fighter(0), 1);
        // Two close attacking ticks arm the release detector.
        brain.decide(&percept(0, 2.0, true));
        brain.decide(&percept(1, 2.0, true));
        assert!(!brain.is_feinting());
        // Release in close range opens the window.
        brain.decide(&percept(2, 2.0, false));
        assert!(brain.is_feinting());
        assert_eq!(
            brain.policy.feint.score_bias()[IDX_ATTACK],
            FEINT_ATTACK_BONUS
        );
        // The window lasts exactly FEINT_WINDOW_TICKS decides.
        for tick in 3..(3 + FEINT_WINDOW_TICKS) {
            let _ = brain.decide(&percept(tick, 2.0, false));
            if tick < 3 + FEINT_WINDOW_TICKS - 1 {
                assert!(brain.is_feinting(), "tick {tick}");
            }
        }
        assert!(!brain.is_feinting(), "window must close");
        // Cooldown blocks an immediate retrigger on the next release.
        brain.decide(&percept(100, 2.0, true));
        brain.decide(&percept(101, 2.0, false));
        assert!(!brain.is_feinting(), "cooldown must block");
    }

    #[test]
    fn feint_ignores_far_releases() {
        let mut brain = DepthBrain::new(fighter(0), 1);
        brain.decide(&percept(0, 20.0, true));
        brain.decide(&percept(1, 20.0, false));
        assert!(!brain.is_feinting(), "far releases never feint");
    }

    #[test]
    fn feint_flips_a_scored_decision() {
        // Mid-range calm: the plain core advances; a feinting finisher
        // strikes instead (feint + finish + holder outscore the advance).
        let probe = percept(7, 5.0, false);
        let mut plain = Brain::new(fighter(0), 1);
        let plain_out = plain.decide(&probe);
        assert_ne!(plain_out.buttons.0 & ATTACK, ATTACK);
        let mut deep = DepthBrain::new(fighter(0), 1);
        deep.set_phase(Phase::Finish);
        deep.set_role(Role::Holder);
        deep.decide(&percept(0, 2.0, true));
        deep.decide(&percept(1, 2.0, true));
        let struck = deep.decide(&percept(2, 2.0, false));
        assert!(deep.is_feinting());
        assert_eq!(struck.buttons.0 & ATTACK, ATTACK);
        // Same feint sequence twice strikes twice.
        let mut rerun = DepthBrain::new(fighter(0), 1);
        rerun.set_phase(Phase::Finish);
        rerun.set_role(Role::Holder);
        rerun.decide(&percept(0, 2.0, true));
        rerun.decide(&percept(1, 2.0, true));
        assert_eq!(rerun.decide(&percept(2, 2.0, false)), struck);
    }

    #[test]
    fn stamina_drains_costs_then_gates_and_collapses() {
        let mut brain = DepthBrain::new(fighter(0), 1);
        assert_eq!(brain.stamina(), STAMINA_MAX);
        // Fresh stamina fences no scores.
        assert_eq!(stamina_gate_bias(STAMINA_MAX), [0; 5]);
        // One close calm strike costs 40 minus 5 recovery.
        let out = brain.decide(&percept(0, 1.0, false));
        assert_eq!(out.buttons, engine_input::Buttons(ATTACK));
        assert_eq!(brain.stamina(), STAMINA_MAX - 40 + STAMINA_RECOVERY);
        // Winded bias guards instead of committing.
        brain.set_stamina(399);
        assert_eq!(stamina_gate_bias(399), [-100, 50, -75, -25, 50]);
        assert!(!brain.is_collapsed());
        // Gassed bias stops the commit almost everywhere.
        brain.set_stamina(149);
        assert_eq!(stamina_gate_bias(149), [-250, 100, -200, -100, 125]);
        brain.set_stamina(100);
        assert!(brain.is_collapsed());
        brain.set_stamina(0);
        assert!(brain.is_collapsed());
        // Clamps hold both ends.
        assert_eq!(brain.set_stamina(9_999), STAMINA_MAX);
        assert_eq!(brain.set_stamina(-9_999), STAMINA_MIN);
        // Costs are published for arena parity with the player table.
        assert_eq!(stamina_cost(Intent::Attack), 40);
        assert_eq!(stamina_cost(Intent::Dodge), 30);
    }

    #[test]
    fn squad_seats_lowest_id_and_refuses_bad_input() {
        let squad = Squad::new(&[fighter(2), fighter(0), fighter(3)]).expect("seats");
        assert_eq!(squad.members(), &[fighter(0), fighter(2), fighter(3)]);
        assert_eq!(squad.roles(), &[Role::Holder, Role::Flanker, Role::Flanker]);
        assert_eq!(squad.holder(), fighter(0));
        assert_eq!(squad.role_of(fighter(2)), Ok(Role::Flanker));
        assert_eq!(
            squad.role_of(fighter(1)),
            Err(DepthError::UnknownFighter { id: 1 })
        );
        assert_eq!(Squad::new(&[]), Err(DepthError::EmptySquad));
        assert_eq!(
            Squad::new(&[fighter(0), fighter(0)]),
            Err(DepthError::DuplicateFighter { id: 0 })
        );
        assert_eq!(
            Squad::new(&[FighterId(16)]),
            Err(DepthError::UnknownFighter { id: 16 })
        );
        assert_eq!(Role::Holder.score_bias()[IDX_BLOCK], 75);
        assert_eq!(Role::Flanker.score_bias()[IDX_ADVANCE], 75);
    }

    #[test]
    fn knockdown_promotes_best_stamina_tie_goes_to_lowest_id() {
        let mut squad = Squad::new(&[fighter(0), fighter(1), fighter(2)]).expect("seats");
        // Holder stands: a flanker falling changes nothing.
        squad
            .on_knockdown(&[fighter(2)], &[(fighter(0), 900), (fighter(2), 0)])
            .expect("standing holder keeps roles");
        assert_eq!(squad.holder(), fighter(0));
        // Holder falls: highest stamina stands in.
        squad
            .on_knockdown(&[fighter(0)], &[(fighter(1), 300), (fighter(2), 700)])
            .expect("holder down promotes");
        assert_eq!(squad.holder(), fighter(2));
        assert_eq!(squad.role_of(fighter(0)), Ok(Role::Flanker));
        // Tie on stamina promotes the lowest fighter id.
        let mut tied = Squad::new(&[fighter(0), fighter(1), fighter(2)]).expect("seats");
        tied.on_knockdown(&[fighter(0)], &[(fighter(1), 500), (fighter(2), 500)])
            .expect("tie promotes lowest id");
        assert_eq!(tied.holder(), fighter(1));
        // Everyone down: roles freeze deterministically.
        tied.on_knockdown(
            &[fighter(0), fighter(1), fighter(2)],
            &[(fighter(0), 0), (fighter(1), 0), (fighter(2), 0)],
        )
        .expect("all down keeps roles");
        assert_eq!(tied.holder(), fighter(1));
        // Strangers refuse loudly.
        assert_eq!(
            squad.on_knockdown(&[fighter(3)], &[]),
            Err(DepthError::UnknownFighter { id: 3 })
        );
    }

    #[test]
    fn stamina_collapse_reassign() {
        let mut squad = Squad::new(&[fighter(0), fighter(1), fighter(2)]).expect("seats");
        // Holder gassed: the freshest flanker takes the center.
        squad
            .on_stamina_collapse(&[(fighter(0), 50), (fighter(1), 800), (fighter(2), 600)])
            .expect("collapsed holder promotes");
        assert_eq!(squad.holder(), fighter(1));
        // Holder fresh: collapse elsewhere changes nothing.
        squad
            .on_stamina_collapse(&[(fighter(1), 900), (fighter(2), 10)])
            .expect("standing holder keeps roles");
        assert_eq!(squad.holder(), fighter(1));
        // Collapse tables name strangers loudly.
        assert_eq!(
            squad.on_stamina_collapse(&[(fighter(3), 999)]),
            Err(DepthError::UnknownFighter { id: 3 })
        );
    }

    #[test]
    fn depth_brain_60s_identical_twice_and_always_valid() {
        let (first, second) = (run_depth(0, 3_600), run_depth(0, 3_600));
        assert_eq!(first.len(), 3_600);
        assert_eq!(first, second);
        for input in &first {
            input.validate().expect("every depth output validates");
        }
        // Full squad script: four brains, same tape twice, identical.
        let bout = || (0..4).map(|id| run_depth(id, 3_600)).collect::<Vec<_>>();
        assert_eq!(bout(), bout());
    }

    #[test]
    fn no_stuck_fighters_every_window_moves() {
        // Each 180-tick window must carry movement wishes; block-only
        // turtles fail this gate.
        for id in 0..4 {
            let inputs = run_depth(id, 3_600);
            for (window, chunk) in inputs.chunks(180).enumerate() {
                assert!(
                    chunk.iter().any(|input| input.mx != 0 || input.mz != 0),
                    "fighter {id} stuck in window {window}"
                );
            }
        }
        // The far-streak anti-stuck survives every depth bias: a distant
        // foe still draws the wander intent, even gassed in the finish.
        let mut brain = DepthBrain::new(fighter(0), 1);
        brain.set_phase(Phase::Finish);
        brain.set_role(Role::Holder);
        brain.set_stamina(0);
        let mut last = brain.decide(&percept(0, 20.0, false));
        for tick in 1..=Brain::FAR_STREAK_TICKS {
            last = brain.decide(&percept(tick, 20.0, false));
        }
        assert_eq!(brain.intent(), Intent::Reposition, "{last:?}");
        assert_ne!((last.mx, last.mz), (0, 0));
        last.validate().expect("wander output validates");
    }

    #[test]
    fn decide_never_reassigns_roles_without_a_trigger() {
        // Arbitration rule as a test: 600 decides leave squad roles alone.
        let squad = Squad::new(&[fighter(0), fighter(1)]).expect("seats");
        let before = squad.clone();
        let mut brains = [
            DepthBrain::new(fighter(0), 6),
            DepthBrain::new(fighter(1), 6),
        ];
        for tick in 0..600 {
            for (id, brain) in brains.iter_mut().enumerate() {
                brain.set_role(squad.role_of(fighter(id as u8)).expect("member"));
                let _ = brain.decide(&script_percept(id as u8, tick));
            }
        }
        assert_eq!(squad, before, "decides must not reassign roles");
    }

    #[test]
    fn shipped_tension_ladder_validates_and_fences_bite() {
        // Every shipped row validates (the posted table, pinned here).
        for rung in 0..LADDER_RUNGS {
            let row = tension_row(rung).expect("shipped rung exists");
            validate_tension_row(rung, &row).expect("shipped row inside fences");
        }
        assert_eq!(tension_row(8), Err(DepthError::UnknownRung { rung: 8 }));
        // Volume climbs monotonically up the ladder.
        let volumes: Vec<u16> = (0..LADDER_RUNGS)
            .map(|rung| crowd_volume(rung).expect("shipped rung"))
            .collect();
        assert_eq!(volumes, vec![200, 300, 400, 500, 600, 700, 850, 1_000]);
        // Each fence bites with its field name.
        let base = tension_row(3).expect("rung 3 ships");
        let mut loud = base;
        loud.crowd_volume_permille = 50;
        assert_eq!(
            validate_tension_row(3, &loud),
            Err(DepthError::TableOutOfFence {
                rung: 3,
                field: "crowd_volume",
                value: 50
            })
        );
        let mut hasty = base;
        hasty.spawn_ticks = 5;
        assert_eq!(
            validate_tension_row(3, &hasty),
            Err(DepthError::TableOutOfFence {
                rung: 3,
                field: "spawn_ticks",
                value: 5
            })
        );
        let mut tilted = base;
        tilted.morale_drift = 500;
        assert_eq!(
            validate_tension_row(3, &tilted),
            Err(DepthError::TableOutOfFence {
                rung: 3,
                field: "morale_drift",
                value: 500
            })
        );
    }

    #[test]
    fn tension_hooks_compose_with_director_tables() {
        // Spawn timer: rung 7 fires every 60 ticks, rung 0 every 480.
        assert_eq!(spawn_due(7, 60, 0), Ok(true));
        assert_eq!(spawn_due(7, 59, 0), Ok(false));
        assert_eq!(spawn_due(0, 479, 0), Ok(false));
        assert_eq!(spawn_due(0, 480, 0), Ok(true));
        // Wrapping-safe: a wrapped tick still measures the interval.
        assert_eq!(spawn_due(7, 10, u32::MAX - 40), Ok(false));
        // Morale drift fires once per 60 ticks with the rung value.
        assert_eq!(morale_step(4, 0), Ok(50));
        assert_eq!(morale_step(4, 60), Ok(50));
        assert_eq!(morale_step(4, 61), Ok(0));
        assert_eq!(morale_step(0, 0), Ok(-100));
        // The drift lands through the brain clamp, never past bounds.
        let mut brain = DepthBrain::new(fighter(0), 1);
        brain.set_morale(990);
        brain.apply_morale_drift(morale_step(7, 0).expect("rung 7"));
        assert_eq!(brain.morale(), 1_000);
        brain.set_morale(10);
        brain.apply_morale_drift(morale_step(0, 0).expect("rung 0"));
        assert_eq!(brain.morale(), 0);
        // The timer gates the director table pick: due ticks may spend
        // the budget, quiet ticks never reach the table.
        let table = vec![
            SpawnRow {
                at_tension: 0.3,
                cost: 20.0,
                label: "scout".into(),
            },
            SpawnRow {
                at_tension: 0.8,
                cost: 60.0,
                label: "horde".into(),
            },
        ];
        let mut budget = Budget::new(100.0);
        budget.add(90.0);
        assert!(!spawn_due(0, 100, 0).expect("rung 0 waits"));
        assert!(spawn_due(7, 120, 60).expect("rung 7 fires"));
        assert_eq!(pick_spawn(&mut budget, &table), Some("scout".into()));
    }

    #[test]
    fn depth_errors_read_and_round_boundary_resets_feint() {
        assert_eq!(
            format!("{}", DepthError::UnknownPhase { value: 9 }),
            "gladiator depth: unknown phase 9"
        );
        assert_eq!(
            format!("{}", DepthError::UnknownRole { value: 9 }),
            "gladiator depth: unknown role 9"
        );
        assert_eq!(
            format!("{}", DepthError::UnknownRung { rung: 9 }),
            "gladiator depth: unknown ladder rung 9"
        );
        assert_eq!(
            format!("{}", DepthError::EmptySquad),
            "gladiator depth: squad has no members"
        );
        let mut brain = DepthBrain::new(fighter(0), 1);
        brain.set_phase(Phase::Finish);
        brain.decide(&percept(0, 2.0, true));
        brain.decide(&percept(1, 2.0, true));
        brain.decide(&percept(2, 2.0, false));
        assert!(brain.is_feinting());
        brain.end_round(Phase::Open);
        assert_eq!(brain.phase(), Phase::Open);
        assert!(!brain.is_feinting());
        // NaN separations sanitize instead of panicking.
        let nasty = Percept::new(
            [f32::NAN, 0.0, f32::INFINITY],
            f32::NAN,
            [f32::NEG_INFINITY, 0.0, f32::NAN],
            true,
            f32::NAN,
            3,
        );
        brain
            .decide(&nasty)
            .validate()
            .expect("sanitized depth output validates");
        // Attack and block buttons both appear across a period-1 bout
        // (period 6 only evaluates on attacking ticks, so it never strikes).
        let mut every_tick = DepthBrain::new(fighter(1), 1);
        let bout: Vec<FighterInput> = (0..600)
            .map(|tick| every_tick.decide(&script_percept(1, tick)))
            .collect();
        assert!(bout.iter().any(|i| (i.buttons.0 & ATTACK) == ATTACK));
        assert!(bout
            .iter()
            .any(|i| i.buttons == engine_input::Buttons(BLOCK)));
    }

    #[test]
    fn scored_mid_arena_wishes_survive_steering() {
        // Close calm strike: Attack (30,0) toward the foe passes through.
        let mut brain = DepthBrain::new(fighter(0), 1);
        let struck = brain.decide(&percept(0, 1.0, false));
        assert_eq!(struck.buttons.0 & ATTACK, ATTACK);
        assert_eq!((struck.mx, struck.mz), (30, 0));
        // Far calm advance toward the foe passes through (closing kept).
        let closing = brain.decide(&percept(1, 20.0, false));
        closing.validate().expect("closing wish validates");
        assert_eq!((closing.mx, closing.mz), (100, 0));
    }

    #[test]
    fn wall_pin_reaims_off_blocked_axis() {
        // Seed-2043 shape: spawned against the east wall, foe deep inside.
        // The scored Advance (+X into the clamp) must come back inward.
        let pin = Percept::new([19.36, 0.0, 0.5], 0.0, [5.0, 0.0, -2.0], false, 14.6, 300);
        let mut brain = DepthBrain::new(fighter(1), 1);
        let out = brain.decide(&pin);
        out.validate().expect("steered output validates");
        assert!(out.mx < 0, "must leave the wall: {out:?}");
        // Same pin twice steers twice (replay-exact).
        let mut rerun = DepthBrain::new(fighter(1), 1);
        assert_eq!(rerun.decide(&pin), out);
    }

    #[test]
    fn pinned_bout_keeps_wishing_off_the_clamp_and_replays() {
        // Wall-pin script: self glued east, foe roaming mid-arena. Every
        // steered wish validates, replays exactly, and never pushes outward
        // into the clamp it is pinned against (the stuck mechanism, fixed
        // at the wish instead of the motion).
        let bout = || {
            let mut brain = DepthBrain::new(fighter(1), 6);
            (0..600u32)
                .map(|tick| {
                    let foe_x = ((tick * 37) % 200) as f32 * 0.1 - 10.0;
                    let foe_z = ((tick * 53) % 200) as f32 * 0.1 - 10.0;
                    let (dx, dz) = (foe_x - 19.36, foe_z - 0.5);
                    brain.decide(&Percept::new(
                        [19.36, 0.0, 0.5],
                        0.0,
                        [foe_x, 0.0, foe_z],
                        tick.is_multiple_of(3),
                        (dx * dx + dz * dz).sqrt(),
                        tick,
                    ))
                })
                .collect::<Vec<_>>()
        };
        let (first, second) = (bout(), bout());
        assert_eq!(first.len(), 600);
        assert_eq!(first, second);
        for input in &first {
            input.validate().expect("steered pin output validates");
            assert!(input.mx <= 0, "never push into the east clamp: {input:?}");
        }
        assert!(
            first.iter().any(|input| input.mx < 0),
            "must wish off the wall"
        );
    }

    #[test]
    fn extent_setter_fences_and_drives_the_band() {
        let mut brain = DepthBrain::new(fighter(0), 1);
        assert_eq!(brain.half_extent_m(), 20.0);
        assert_eq!(brain.set_half_extent_m(10.0), 10.0);
        // Pinned at 9.5 m in a 10 m arena: inside the band, escapes inward.
        let pin = Percept::new([9.5, 0.0, 0.0], 0.0, [-5.0, 0.0, 0.0], false, 14.5, 0);
        let out = brain.decide(&pin);
        assert!(out.mx < 0, "10 m arena escape: {out:?}");
        // Refuse-and-fallback: NaN, negative and wild extents restore 20 m.
        assert_eq!(brain.set_half_extent_m(f32::NAN), 20.0);
        assert_eq!(brain.set_half_extent_m(-3.0), 20.0);
        assert_eq!(brain.set_half_extent_m(200.0), 20.0);
        assert_eq!(brain.half_extent_m(), 20.0);
    }

    #[test]
    fn arena_policy_bias_matches_setter_path_lane_for_lane() {
        // The composed policy sums the same lanes in the same order as the
        // brain setter path, so composition never drifts from behavior.
        let mut policy = ArenaPolicy::new();
        assert_eq!(policy.phase(), Phase::Open);
        assert_eq!(policy.role(), Role::Flanker);
        assert_eq!(policy.stamina(), STAMINA_MAX);
        assert!(!policy.is_feinting());
        assert!(!policy.is_collapsed());
        assert_eq!(policy, ArenaPolicy::default());
        policy.set_phase(Phase::Finish);
        policy.set_role(Role::Holder);
        policy.set_stamina(149);
        assert_eq!(policy.stamina(), 149);
        assert!(!policy.is_collapsed());
        policy.set_stamina(100);
        assert!(policy.is_collapsed());
        assert_eq!(policy.set_stamina(9_999), STAMINA_MAX);
        assert_eq!(policy.set_stamina(-9_999), STAMINA_MIN);
        // Bias equals the four published tables summed lane for lane.
        let mut full = ArenaPolicy::new();
        full.set_phase(Phase::Pressure);
        full.set_role(Role::Holder);
        full.set_stamina(STAMINA_MAX);
        let (phase, gate, role) = (
            Phase::Pressure.score_bias(),
            stamina_gate_bias(STAMINA_MAX),
            Role::Holder.score_bias(),
        );
        let expect = [
            phase[0] + gate[0] + role[0],
            phase[1] + gate[1] + role[1],
            phase[2] + gate[2] + role[2],
            phase[3] + gate[3] + role[3],
            phase[4] + gate[4] + role[4],
        ];
        assert_eq!(full.bias(), expect);
        for lane in full.bias() {
            assert!((-445..=445).contains(&lane), "{lane}");
        }
        // Feint opens on the same close release the brain detects.
        full.update_feint(true, false, 2_000);
        assert!(!full.is_feinting());
        full.update_feint(false, true, 2_000);
        assert!(full.is_feinting());
        assert_eq!(
            full.bias()[IDX_ATTACK],
            expect[IDX_ATTACK] + FEINT_ATTACK_BONUS
        );
        full.end_round(Phase::Open);
        assert_eq!(full.phase(), Phase::Open);
        assert!(!full.is_feinting());
    }

    #[test]
    fn squads_parameterize_by_length() {
        // A duel fixture seats two in a `SquadN<2>`: lowest id holds.
        let duel = SquadN::<2>::new(&[fighter(3), fighter(1)]).expect("duel seats");
        assert_eq!(duel.members(), &[fighter(1), fighter(3)]);
        assert_eq!(duel.roles(), &[Role::Holder, Role::Flanker]);
        assert_eq!(duel.holder(), fighter(1));
        // Capacity is the parameter: a third member refuses loudly.
        assert_eq!(
            SquadN::<2>::new(&[fighter(0), fighter(1), fighter(2)]),
            Err(DepthError::UnknownFighter { id: 2 })
        );
        assert_eq!(SquadN::<2>::new(&[]), Err(DepthError::EmptySquad));
        // Full arena squads still seat sixteen with the same arbitration.
        let full: SquadN<16> =
            Squad::new(&[fighter(7), fighter(0), fighter(15), fighter(2), fighter(9)])
                .expect("arena seats");
        assert_eq!(full.holder(), fighter(0));
        assert_eq!(full.members().len(), 5);
        let mut wide = full.clone();
        wide.on_knockdown(&[fighter(0)], &[(fighter(2), 700), (fighter(15), 100)])
            .expect("holder down promotes");
        assert_eq!(wide.holder(), fighter(2));
        // The default squad is the arena width: `Squad` is `SquadN<16>`.
        let classic = Squad::new(&[fighter(2), fighter(0), fighter(3)]).expect("seats");
        assert_eq!(classic.members().len(), 3);
        assert_eq!(classic.holder(), fighter(0));
        // Slot validity still follows the canonical cap at any width.
        assert_eq!(
            SquadN::<2>::new(&[FighterId(16)]),
            Err(DepthError::UnknownFighter { id: 16 })
        );
        assert_eq!(
            SquadN::<4>::new(&[fighter(0), fighter(0)]),
            Err(DepthError::DuplicateFighter { id: 0 })
        );
    }

    // ---- melee: attack tokens, the quantized view and the temperament ----

    fn foe_view(slot: u8, dist_cm: u16, phase: StrikePhase) -> FoeView {
        FoeView {
            slot,
            dist_cm,
            hp_permille: 1_000,
            phase,
            frames_to_active: 0,
        }
    }

    #[test]
    fn melee_percept_third_attacker_waits_and_tokens_stay_at_two() {
        let mut squad =
            Squad::new(&[fighter(0), fighter(1), fighter(2), fighter(3)]).expect("seats");
        let target = fighter(9);
        assert_eq!(squad.claim_token(fighter(0), target), Ok(true));
        assert_eq!(squad.claim_token(fighter(1), target), Ok(true));
        assert_eq!(
            squad.claim_token(fighter(2), target),
            Ok(false),
            "third waits"
        );
        assert_eq!(squad.claim_token(fighter(3), target), Ok(false));
        assert_eq!(squad.tokens_on(target), MAX_ATTACKERS_PER_TARGET);
        assert_eq!(
            squad.target_of(fighter(2)),
            None,
            "the waiter holds nothing"
        );
        // Asking again never grows the count past two.
        for _ in 0..5 {
            assert_eq!(
                squad.claim_token(fighter(0), target),
                Ok(true),
                "holder re-confirms"
            );
            assert_eq!(squad.claim_token(fighter(2), target), Ok(false));
            assert!(squad.tokens_on(target) <= MAX_ATTACKERS_PER_TARGET);
        }
    }

    #[test]
    fn melee_percept_a_release_hands_the_token_to_the_waiting_attacker() {
        let mut squad = Squad::new(&[fighter(0), fighter(1), fighter(2)]).expect("seats");
        let target = fighter(5);
        squad.claim_token(fighter(0), target).expect("member");
        squad.claim_token(fighter(1), target).expect("member");
        assert_eq!(squad.claim_token(fighter(2), target), Ok(false));
        squad.release_token(fighter(0));
        assert_eq!(squad.tokens_on(target), 1);
        assert_eq!(squad.claim_token(fighter(2), target), Ok(true));
        assert_eq!(squad.tokens_on(target), 2);
        squad.clear_tokens();
        assert_eq!(squad.tokens_on(target), 0);
    }

    #[test]
    fn melee_percept_an_attacker_holds_one_token_at_a_time() {
        let mut squad = Squad::new(&[fighter(0), fighter(1), fighter(2)]).expect("seats");
        assert_eq!(squad.claim_token(fighter(0), fighter(4)), Ok(true));
        assert_eq!(squad.claim_token(fighter(0), fighter(5)), Ok(true));
        assert_eq!(squad.tokens_on(fighter(4)), 0, "the old token was dropped");
        assert_eq!(squad.tokens_on(fighter(5)), 1);
        assert_eq!(squad.target_of(fighter(0)), Some(fighter(5)));
        // A full target refuses without costing the waiter what it holds.
        assert_eq!(squad.claim_token(fighter(1), fighter(4)), Ok(true));
        assert_eq!(squad.claim_token(fighter(2), fighter(5)), Ok(true));
        assert_eq!(squad.tokens_on(fighter(5)), 2);
        assert_eq!(squad.claim_token(fighter(1), fighter(5)), Ok(false));
        assert_eq!(
            squad.target_of(fighter(1)),
            Some(fighter(4)),
            "kept its old token"
        );
    }

    #[test]
    fn melee_percept_token_requests_refuse_bad_slots_loudly() {
        let mut squad = Squad::new(&[fighter(0), fighter(1)]).expect("seats");
        assert_eq!(
            squad.claim_token(fighter(7), fighter(1)),
            Err(DepthError::UnknownFighter { id: 7 }),
            "non-member attacker"
        );
        assert_eq!(
            squad.claim_token(fighter(0), FighterId(16)),
            Err(DepthError::UnknownFighter { id: 16 }),
            "target beyond the arena"
        );
        assert_eq!(
            squad.claim_token(fighter(0), fighter(0)),
            Err(DepthError::SelfTarget { id: 0 })
        );
        assert_eq!(squad.tokens_on(FighterId(200)), 0);
        for id in 0..16 {
            assert_eq!(squad.tokens_on(fighter(id)), 0, "refusals grant nothing");
        }
    }

    #[test]
    fn melee_percept_a_waiting_brain_gives_ground_instead_of_swinging() {
        let near = foe_view(1, 150, StrikePhase::Idle);
        let view = MeleeView::nearest(1_000, 1_000, &[near]);
        let mut eager = DepthBrain::new(fighter(0), 1);
        let mut waiting = DepthBrain::new(fighter(0), 1);
        let base = percept(0, 1.5, false);
        eager.decide(&base.with_melee(view));
        waiting.decide(&base.with_melee(view.targeting(1, false)));
        assert_eq!(
            eager.intent(),
            Intent::Attack,
            "a token holder swings at 1.5 m"
        );
        assert_eq!(waiting.intent(), Intent::Retreat, "a waiter backs off");
        let ring = MeleeView::nearest(1_000, 1_000, &[foe_view(1, 320, StrikePhase::Idle)]);
        waiting.decide(&percept(1, 3.2, false).with_melee(ring.targeting(1, false)));
        assert_eq!(
            waiting.intent(),
            Intent::Block,
            "on the ring it stands guard"
        );
        let out = MeleeView::nearest(1_000, 1_000, &[foe_view(1, 600, StrikePhase::Idle)]);
        waiting.decide(&percept(2, 6.0, false).with_melee(out.targeting(1, false)));
        assert_eq!(waiting.intent(), Intent::Advance, "farther out it walks in");
    }

    #[test]
    fn melee_percept_a_recovering_foe_draws_a_punish_instead_of_a_guard() {
        let recovering = foe_view(1, 150, StrikePhase::Recovery);
        let view = MeleeView::nearest(1_000, 1_000, &[recovering]);
        let mut brain = DepthBrain::new(fighter(0), 1);
        brain.decide(&percept(0, 1.5, true).with_melee(view));
        assert_eq!(
            brain.intent(),
            Intent::Attack,
            "the blow already landed: punish"
        );
        let mut legacy = DepthBrain::new(fighter(0), 1);
        legacy.decide(&percept(0, 1.5, true));
        assert_ne!(
            legacy.intent(),
            Intent::Attack,
            "a legacy brain still guards"
        );
    }

    #[test]
    fn melee_percept_reaction_offset_delays_noticing_the_blow() {
        let first_guard = |reaction: u8| {
            let mut brain = DepthBrain::new(fighter(0), 6);
            brain.set_personality(Personality::new(reaction, 1_500, 500));
            let mut first = None;
            for tick in 0..24u32 {
                let phase = if (10..14).contains(&tick) {
                    StrikePhase::Windup
                } else {
                    StrikePhase::Idle
                };
                let view = MeleeView::nearest(1_000, 1_000, &[foe_view(1, 100, phase)]);
                brain.decide(&percept(tick, 1.0, false).with_melee(view));
                if first.is_none() && matches!(brain.intent(), Intent::Dodge | Intent::Block) {
                    first = Some(tick);
                }
            }
            first
        };
        assert_eq!(
            first_guard(0),
            Some(10),
            "a quick brain answers on the first look"
        );
        assert_eq!(first_guard(2), Some(12));
        assert_eq!(first_guard(3), Some(13));
        // The windup spans ticks 10..14, so a five-tick delay answers a blow that has landed.
        assert_eq!(first_guard(5), Some(15));
    }

    #[test]
    fn melee_percept_range_and_nerve_make_two_brains_decide_differently() {
        let sees = percept(0, 1.4, false);
        let mut short = DepthBrain::new(fighter(0), 1);
        short.set_personality(Personality::new(0, 1_000, 500));
        let mut long = DepthBrain::new(fighter(1), 1);
        long.set_personality(Personality::new(0, 1_500, 500));
        short.decide(&sees);
        long.decide(&sees);
        assert_ne!(
            short.intent(),
            Intent::Attack,
            "1.4 m is beyond a 1.0 m brain's range"
        );
        assert_eq!(long.intent(), Intent::Attack, "and inside a 1.5 m brain's");
        assert_eq!(
            short.personality().map(|p| p.preferred_range_mm()),
            Some(1_000)
        );
        assert!(DepthBrain::new(fighter(0), 1).personality().is_none());
    }

    #[test]
    fn melee_percept_a_legacy_percept_decides_as_it_always_did() {
        let mut plain = DepthBrain::new(fighter(0), 6);
        let mut empty = DepthBrain::new(fighter(0), 6);
        for tick in 0..120 {
            let legacy = script_percept(0, tick);
            let a = plain.decide(&legacy);
            let b = empty.decide(&legacy.with_melee(MeleeView::EMPTY));
            assert_eq!(a, b, "tick {tick}");
        }
    }

    #[test]
    fn tier_rows_match_the_designer_file_and_climb_in_skill() {
        let text = include_str!("../../../content/design/enemies.rows");
        let rows = parse_tiers(text).expect("designer tiers parse");
        assert_eq!(rows[..3], SHIPPED_TIERS);
        assert_eq!(rows[3], PRIMUS_TIER);
        assert_eq!(
            (rows[0].tier, rows[1].tier, rows[2].tier, rows[3].tier),
            (1, 2, 3, 4)
        );
        assert_eq!(
            (
                rows[0].hp_permille,
                rows[1].hp_permille,
                rows[2].hp_permille
            ),
            (950, 1_000, 1_050)
        );
        assert_eq!(rows[0].role(), Role::Holder);
        assert_eq!(rows[1].role(), Role::Flanker);
        assert_eq!(rows[2].role(), Role::Holder);
        let reactions = [
            rows[0].reaction_ticks(),
            rows[1].reaction_ticks(),
            rows[2].reaction_ticks(),
        ];
        assert!(
            reactions[0] > reactions[1] && reactions[1] > reactions[2],
            "{reactions:?}"
        );
        let nerves = [
            rows[0].aggression_permille(),
            rows[1].aggression_permille(),
            rows[2].aggression_permille(),
        ];
        assert!(nerves[0] < nerves[1] && nerves[1] < nerves[2], "{nerves:?}");
        assert_eq!(
            (rows[0].tell_ticks, rows[1].tell_ticks, rows[2].tell_ticks),
            (15, 15, 15)
        );
        assert_eq!(
            (rows[0].counter, rows[1].counter, rows[2].counter),
            (1, 2, 1)
        );
    }

    #[test]
    fn tier_install_drives_the_scored_decide() {
        let mut low = DepthBrain::new(fighter(0), 1);
        let mut high = DepthBrain::new(fighter(0), 1);
        assert!(low.tier().is_none());
        low.set_tier(SHIPPED_TIERS[0]);
        high.set_tier(SHIPPED_TIERS[2]);
        assert_eq!(low.tier(), Some(SHIPPED_TIERS[0]));
        assert_eq!(low.role(), Role::Holder);
        assert_eq!(low.personality().map(|p| p.reaction_ticks()), Some(4));
        assert_eq!(
            high.personality().map(|p| p.aggression_permille()),
            Some(700)
        );
        // Close incoming blow: the calm holder guards, the even flanker
        // sidesteps. Same percept after the delay warms, different answers.
        let mut calm = DepthBrain::new(fighter(0), 1);
        calm.set_tier(SHIPPED_TIERS[0]);
        let mut even = DepthBrain::new(fighter(0), 1);
        even.set_tier(SHIPPED_TIERS[1]);
        let mut calm_out = calm.decide(&percept(0, 1.0, true));
        let mut even_out = even.decide(&percept(0, 1.0, true));
        for tick in 1..10u32 {
            calm_out = calm.decide(&percept(tick, 1.0, true));
            even_out = even.decide(&percept(tick, 1.0, true));
        }
        assert_eq!(calm_out.buttons, engine_input::Buttons(BLOCK));
        assert_eq!(even_out.buttons, engine_input::Buttons(DODGE));
        let run = |tier: TierRow| {
            let mut brain = DepthBrain::new(fighter(0), 1);
            brain.set_tier(tier);
            (0..600u32)
                .map(|tick| brain.decide(&script_percept(0, tick)))
                .collect::<Vec<_>>()
        };
        let (low_first, low_second) = (run(SHIPPED_TIERS[0]), run(SHIPPED_TIERS[0]));
        assert_eq!(low_first, low_second);
        let mid_run = run(SHIPPED_TIERS[1]);
        assert_ne!(low_first, mid_run, "tiers must fence differently");
        for input in low_first.iter().chain(mid_run.iter()) {
            input.validate().expect("tier output validates");
        }
        let mut mid = DepthBrain::new(fighter(1), 1);
        mid.set_tier(SHIPPED_TIERS[1]);
        assert_eq!(mid.role(), Role::Flanker);
    }

    /// A north foe at `z` metres with the brain itself at the origin.
    fn north_percept(z: f32) -> Percept {
        Percept::new([0.0, 0.0, 0.0], 0.0, [0.0, 0.0, z], false, z, 0)
    }

    #[test]
    fn tier_brain_maps_its_wish_into_the_foe_frame() {
        // Legacy arena (no melee view): the foe stands 20 m north, so the
        // mapped advance must point north. Without the map the brain emits
        // the facing-relative (100, 0) and walks world +X instead, which is
        // why the pair never closed and every duel ended 0-0.
        let north = north_percept(20.0);
        let mut tiered = DepthBrain::new(fighter(0), 6);
        tiered.set_tier(SHIPPED_TIERS[0]);
        let closing = tiered.decide(&north);
        closing.validate().expect("mapped tier output validates");
        assert_eq!(
            (closing.mx, closing.mz),
            (0, 100),
            "tier advance must close on the foe: {closing:?}"
        );
        // A brain with no tier keeps the legacy world reading byte-identical:
        // the map belongs to the shipped duel rung only.
        let mut plain = DepthBrain::new(fighter(0), 6);
        let legacy = plain.decide(&north);
        assert_eq!((legacy.mx, legacy.mz), (100, 0));
        // Rules v2 attaches a live view and maps the same wish itself, so a
        // tier brain must leave the facing-relative pair alone (no double map).
        let mut v2 = DepthBrain::new(fighter(0), 6);
        v2.set_tier(SHIPPED_TIERS[0]);
        let live = north.with_melee(MeleeView::nearest(
            1_000,
            1_000,
            &[FoeView::quantize(1, 20_000, 1_000, StrikePhase::Idle, 0)],
        ));
        let held = v2.decide(&live);
        assert_eq!(
            (held.mx, held.mz),
            (100, 0),
            "a v2 arena maps the wish; the brain must not map it twice"
        );
    }

    #[test]
    fn tier_wish_map_follows_the_foe_frame_and_its_fences() {
        let wish = FighterInput {
            tick: 0,
            fighter: fighter(0),
            mx: 100,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons::NONE,
        };
        let at = |x: f32, z: f32| Percept::new([0.0, 0.0, 0.0], 0.0, [x, 0.0, z], false, 10.0, 0);
        let out = |intent: Intent, percept: &Percept, dist_mm: i32| {
            let mapped = tier_wish_to_world(wish, intent, percept, dist_mm);
            mapped.validate().expect("mapped wish validates");
            (mapped.mx, mapped.mz)
        };
        let north = at(0.0, 10.0);
        assert_eq!(out(Intent::Advance, &north, 10_000), (0, 100));
        assert_eq!(out(Intent::Retreat, &north, 10_000), (0, -80));
        assert_eq!(out(Intent::Reposition, &north, 10_000), (80, 60));
        assert_eq!(out(Intent::Block, &north, 10_000), (0, 0));
        // The four faces: the wish follows the foe, never a fixed world axis.
        assert_eq!(out(Intent::Advance, &at(10.0, 0.0), 10_000), (100, 0));
        assert_eq!(out(Intent::Advance, &at(-10.0, 0.0), 10_000), (-100, 0));
        assert_eq!(out(Intent::Advance, &at(0.0, -10.0), 10_000), (0, -100));
        // Dodge steps onto the right-hand perpendicular of the foe bearing:
        // due east means the dodge goes south, due north due east.
        assert_eq!(out(Intent::Dodge, &at(10.0, 0.0), 10_000), (0, -100));
        assert_eq!(out(Intent::Dodge, &north, 10_000), (100, 0));
        // Attack inside the tier's own swing spacing steps in at (30, 0);
        // beyond it the swing closes at full wish instead of holding.
        assert_eq!(
            out(Intent::Attack, &north, i32::from(TIER_RANGE_MM)),
            (0, 30)
        );
        assert_eq!(
            out(Intent::Attack, &north, i32::from(TIER_RANGE_MM) + 1),
            (0, 100)
        );
        // Coincident fighters stand still rather than dividing by zero.
        assert_eq!(out(Intent::Advance, &at(0.0, 0.0), 0), (0, 0));
        // Every intent over every bearing stays inside the wish fence.
        for intent in [
            Intent::Attack,
            Intent::Block,
            Intent::Dodge,
            Intent::Advance,
            Intent::Retreat,
            Intent::Reposition,
        ] {
            for (x, z) in [
                (0.0f32, 9.0f32),
                (9.0, 0.0),
                (-9.0, 0.0),
                (0.0, -9.0),
                (7.0, -7.0),
                (-5.0, 5.0),
            ] {
                let (mx, mz) = out(intent, &at(x, z), 9_000);
                assert!(
                    (-100..=100).contains(&mx) && (-100..=100).contains(&mz),
                    "{intent:?} at ({x},{z}) left the fence: ({mx},{mz})"
                );
            }
        }
    }

    #[test]
    fn tier_rows_refuse_bad_lines_with_the_line() {
        let good_1 = "tier_1 hp_permille=950 tell_ticks=15 stagger_permille=400 counter=1";
        let good_2 = "tier_2 hp_permille=1000 tell_ticks=15 stagger_permille=500 counter=2";
        let good_3 = "tier_3 hp_permille=1050 tell_ticks=15 stagger_permille=600 counter=1";
        let file = ["table enemies version 1", good_1, good_2, good_3].join("\n");
        let good = parse_tiers(&file).expect("good file");
        assert_eq!(good[..3], SHIPPED_TIERS);
        assert_eq!(
            good[3], PRIMUS_TIER,
            "a file without tier_4 falls back to the primus row"
        );
        let bad_header = ["table combat version 1", good_1, good_2, good_3].join("\n");
        assert_eq!(parse_tiers(&bad_header), Err(TierError::Header { line: 1 }));
        let unknown = file.replace("hp_permille=950", "health=950");
        assert_eq!(
            parse_tiers(&unknown),
            Err(TierError::UnknownKey { line: 2 })
        );
        let trimmed = good_3.replace(" counter=1", "");
        let missing = ["table enemies version 1", good_1, good_2, trimmed.as_str()].join("\n");
        assert_eq!(
            parse_tiers(&missing),
            Err(TierError::MissingKey { line: 4 })
        );
        let float = file.replace("tell_ticks=15", "tell_ticks=15.0");
        assert_eq!(parse_tiers(&float), Err(TierError::Float { line: 2 }));
        let thin = file.replace("hp_permille=950", "hp_permille=800");
        assert_eq!(
            parse_tiers(&thin),
            Err(TierError::OutOfFence {
                line: 2,
                field: "hp_permille",
                value: 800
            })
        );
        let dup = [file.as_str(), good_1].join("\n");
        assert_eq!(parse_tiers(&dup), Err(TierError::Duplicate { line: 5 }));
        let tier_5 = [
            file.as_str(),
            "tier_5 hp_permille=1100 tell_ticks=15 stagger_permille=700 counter=1",
        ]
        .join("\n");
        assert_eq!(parse_tiers(&tier_5), Err(TierError::RowId { line: 5 }));
        assert_eq!(
            TierRow::from_row(
                "tier_9 hp_permille=950 tell_ticks=15 stagger_permille=400 counter=1",
                7
            ),
            Err(TierError::RowId { line: 7 })
        );
    }

    #[test]
    fn tier_primus_phase_starts_at_500_permille() {
        assert_eq!(primus_phase(501), 1);
        assert_eq!(primus_phase(500), 2);
        assert_eq!(primus_phase(0), 2);
        assert_eq!(primus_phase(1_000), 1);
        assert_eq!(primus_phase(u16::MAX), 1);
    }

    #[test]
    fn tier_primus_answers_differ_by_phase() {
        assert_eq!(
            PRIMUS_TIER,
            TierRow {
                tier: 4,
                hp_permille: 1100,
                tell_ticks: 15,
                stagger_permille: 700,
                counter: 1,
            }
        );
        assert!((900..=1_200).contains(&PRIMUS_TIER.hp_permille));
        assert_eq!(PRIMUS_TIER.counter, SHIPPED_TIERS[2].counter);
        assert_eq!(primus_counter(1), 1);
        assert_eq!(primus_counter(2), 2);
        assert_ne!(primus_counter(1), primus_counter(2));
        for answer in [primus_counter(1), primus_counter(2)] {
            assert!((1..=3).contains(&answer), "every T4 attack has an answer");
        }
        assert_eq!(primus_counter(0), 1);
        assert_eq!(primus_counter(3), 1);
        assert_eq!(primus_counter(u8::MAX), 1);
    }

    #[test]
    fn tier_primus_skill_climbs_past_t3_and_the_file_carries_it() {
        let t3 = SHIPPED_TIERS[2];
        assert!(PRIMUS_TIER.reaction_ticks() <= t3.reaction_ticks());
        assert!(PRIMUS_TIER.aggression_for_phase(2) >= t3.aggression_permille());
        assert_eq!(PRIMUS_TIER.reaction_ticks(), 1);
        assert_eq!(PRIMUS_TIER.aggression_for_phase(1), 700);
        assert_eq!(PRIMUS_TIER.aggression_for_phase(2), 750);
        assert!((250..=750).contains(&PRIMUS_TIER.aggression_for_phase(2)));
        assert_eq!(PRIMUS_TIER.personality_for_phase(2).reaction_ticks(), 1);
        assert_eq!(
            PRIMUS_TIER.personality_for_phase(2).aggression_permille(),
            750
        );
        let row = TierRow::from_row(
            "tier_4 hp_permille=1100 tell_ticks=15 stagger_permille=700 counter=1",
            5,
        )
        .expect("tier_4 parses");
        assert_eq!(row, PRIMUS_TIER);
        let file = [
            "table enemies version 1",
            "tier_1 hp_permille=950 tell_ticks=15 stagger_permille=400 counter=1",
            "tier_2 hp_permille=1000 tell_ticks=15 stagger_permille=500 counter=2",
            "tier_3 hp_permille=1050 tell_ticks=15 stagger_permille=600 counter=1",
            "tier_4 hp_permille=1100 tell_ticks=15 stagger_permille=700 counter=1",
        ]
        .join("\n");
        let four = parse_tiers(&file).expect("four-row file parses");
        assert_eq!(four[..3], SHIPPED_TIERS);
        assert_eq!(four[3], PRIMUS_TIER);
        let text = include_str!("../../../content/design/enemies.rows");
        let shipped = parse_tiers(text).expect("shipped file parses to four rows");
        assert_eq!(shipped[..3], SHIPPED_TIERS);
        assert_eq!(shipped[3], PRIMUS_TIER);
    }
}
