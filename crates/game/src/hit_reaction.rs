//! hit_reaction: the struck body's flinch picked by hit direction and weight
//! (`content/design/hit-reactions.rows`).
//!
//! Until now every blow flinched the victim the same way: a 5 cm shove away
//! from the attacker, a 1.5 cm dip and an 11 degree lean, over 450 ms, whether
//! a dagger tapped the chest or a club caught the back. The Unreal "Ultimate
//! Hit Reaction System" idea is a chooser table, (direction x intensity) to a
//! reaction; this module is that table with parameters in place of mocap
//! clips: how far the pelvis is shoved, how deep it dips, how far the torso
//! and head lean, how much the torso twists away from a flank blow, and how
//! long it reads. The light front row is the reference flinch, so a duel's
//! light hits look as they did; a heavy blow reads bigger and longer, a hit
//! from behind arches the back, a flank blow turns the body.
//!
//! Consumer: the fighter look picks a [`Shape`] when a hit lands
//! (`remember_hit`) and the flinch pose reads it (`apply_hit_flinch`). Data
//! owner: `content/design/hit-reactions.rows`, embedded at build time. Units:
//! percent of the reference flinch, milliradians, ticks at 60 Hz. Determinism:
//! pure integer rows plus one float sector pick; presentation only, the sim
//! and its hash never read it. Errors: [`ReactionError`] names the line and the
//! key; the shipped table is pinned by a test and the neutral built-in stands
//! in if it ever fails to parse.

use std::sync::OnceLock;

/// The shipped rows, embedded at build time.
pub const REACTION_ROWS: &str = include_str!("../../../content/design/hit-reactions.rows");

/// The reference flinch window, ticks (about 450 ms, AM23).
pub const REFERENCE_WINDOW_TICKS: u32 = 27;

/// Where a blow came from, relative to the way the victim faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sector {
    /// From in front (within 45 degrees of the facing).
    Front,
    /// From behind.
    Back,
    /// From the victim's left flank.
    Left,
    /// From the victim's right flank.
    Right,
}

/// How hard it landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// A light blow.
    Light,
    /// A heavy blow.
    Heavy,
    /// The blow that floors the victim.
    Down,
}

const SECTORS: [Sector; 4] = [Sector::Front, Sector::Back, Sector::Left, Sector::Right];
const WEIGHTS: [Weight; 3] = [Weight::Light, Weight::Heavy, Weight::Down];

impl Sector {
    fn key(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Front => 0,
            Self::Back => 1,
            Self::Left => 2,
            Self::Right => 3,
        }
    }

    /// The sector of a blow whose push on the victim is the unit `push` (away
    /// from the attacker, on the sand plane) for a victim facing `yaw` (zero
    /// faces +z, a quarter turn faces +x). A zero or non-finite push has no
    /// side and reads as front, the reference flinch.
    #[must_use]
    pub fn of(yaw: f32, push: [f32; 2]) -> Self {
        if !yaw.is_finite() || !push.iter().all(|v| v.is_finite()) {
            return Self::Front;
        }
        let (sin, cos) = yaw.sin_cos();
        // The blow came from the attacker, opposite the push.
        let from = [-push[0], -push[1]];
        let ahead = from[0] * sin + from[1] * cos;
        let right = from[0] * cos - from[1] * sin;
        if ahead * ahead + right * right < 1.0e-12 {
            return Self::Front;
        }
        if ahead >= right.abs() {
            Self::Front
        } else if -ahead >= right.abs() {
            Self::Back
        } else if right > 0.0 {
            Self::Right
        } else {
            Self::Left
        }
    }
}

impl Weight {
    /// The weight of a blow: one that floors the victim is the hardest, then a
    /// heavy blow (the sim holds it six ticks), else a light one.
    #[must_use]
    pub const fn of(heavy: bool, floors: bool) -> Self {
        if floors {
            Self::Down
        } else if heavy {
            Self::Heavy
        } else {
            Self::Light
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Heavy => "heavy",
            Self::Down => "down",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Light => 0,
            Self::Heavy => 1,
            Self::Down => 2,
        }
    }
}

/// The index-selected blend selector a blow from `sector` at `weight` names:
/// the table cell in table order, sector major then weight, `0..=11`.
#[must_use]
pub fn selector(sector: Sector, weight: Weight) -> u8 {
    // Twelve cells fit a byte; a refused conversion reads as no cell.
    u8::try_from(sector.index() * WEIGHTS.len() + weight.index()).unwrap_or(u8::MAX)
}

/// One reaction: multipliers on the reference flinch, a twist, a length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// Pelvis shove, 1.0 is the reference.
    pub push: f32,
    /// Pelvis dip, 1.0 is the reference.
    pub dip: f32,
    /// Torso and head lean, 1.0 is the reference.
    pub lean: f32,
    /// Torso turn about the vertical, radians, positive toward the victim's
    /// right.
    pub twist_rad: f32,
    /// Ticks the flinch reads.
    pub window_ticks: u32,
}

impl Shape {
    /// The reference flinch the look always had.
    pub const NEUTRAL: Self = Self {
        push: 1.0,
        dip: 1.0,
        lean: 1.0,
        twist_rad: 0.0,
        window_ticks: REFERENCE_WINDOW_TICKS,
    };

    /// The reference envelope position `elapsed` real ticks into this flinch,
    /// as whole ticks plus a fraction: a longer window plays the same envelope
    /// slower. Integer math, so the reference row (27 ticks) returns `elapsed`
    /// and a zero fraction exactly.
    #[must_use]
    pub fn envelope_at(&self, elapsed: u32) -> (u32, f32) {
        let window = u64::from(self.window_ticks.max(1));
        let scaled = u64::from(elapsed) * u64::from(REFERENCE_WINDOW_TICKS);
        let whole = u32::try_from(scaled / window).unwrap_or(u32::MAX);
        let rest = u16::try_from(scaled % window).unwrap_or(0);
        let width = u16::try_from(window).unwrap_or(u16::MAX);
        (whole, f32::from(rest) / f32::from(width))
    }
}

/// Why a row was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReactionError {
    /// The header `table hit-reaction version 1` is missing.
    Header,
    /// A row names no sector or no weight.
    Word {
        /// One-based line number.
        line: usize,
        /// The word that is neither.
        word: String,
    },
    /// A row lacks a key.
    Missing {
        /// One-based line number.
        line: usize,
        /// The missing key.
        key: &'static str,
    },
    /// A key is not a whole number inside its range.
    Bad {
        /// One-based line number.
        line: usize,
        /// The offending key.
        key: &'static str,
        /// The text the row holds.
        value: String,
    },
    /// A (sector, weight) has no row, or two.
    Cell(&'static str, &'static str),
}

impl core::fmt::Display for ReactionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Header => f.write_str(
                "hit reaction rows: the `table hit-reaction version 1` header is missing",
            ),
            Self::Word { line, word } => write!(
                f,
                "hit reaction rows line {line}: `{word}` is not a sector or a weight"
            ),
            Self::Missing { line, key } => {
                write!(f, "hit reaction rows line {line}: key `{key}` is missing")
            }
            Self::Bad { line, key, value } => write!(
                f,
                "hit reaction rows line {line}: `{key}={value}` is out of range"
            ),
            Self::Cell(sector, weight) => write!(
                f,
                "hit reaction rows: `{sector} {weight}` needs exactly one row"
            ),
        }
    }
}

impl std::error::Error for ReactionError {}

/// The twelve reactions: four sectors by three weights.
#[derive(Clone, Debug, PartialEq)]
pub struct ReactionTable {
    cells: [[Shape; 3]; 4],
}

impl ReactionTable {
    /// Every cell the reference flinch: what the look did before this table.
    #[must_use]
    pub const fn neutral() -> Self {
        Self {
            cells: [[Shape::NEUTRAL; 3]; 4],
        }
    }

    /// The shipped table, parsed once; the neutral table stands in if the
    /// embedded text ever failed to parse (a test pins that it does not).
    #[must_use]
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<ReactionTable> = OnceLock::new();
        SHIPPED.get_or_init(|| Self::parse(REACTION_ROWS).unwrap_or_else(|_| Self::neutral()))
    }

    /// The reaction of a blow from `sector` at `weight`.
    #[must_use]
    pub fn shape(&self, sector: Sector, weight: Weight) -> Shape {
        self.cells[sector.index()][weight.index()]
    }

    /// The cell an index-selected blend node names, in table order: sector
    /// major, weight minor. Out-of-range selectors name no cell, so a stale
    /// index is visible instead of silently reading a neighbour.
    #[must_use]
    pub fn shape_by_selector(&self, selector: u8) -> Option<Shape> {
        let cell = usize::from(selector);
        let weight = cell % WEIGHTS.len();
        let sector = *SECTORS.get(cell / WEIGHTS.len())?;
        Some(self.cells[sector.index()][weight])
    }

    /// Parse rows text: the header, then twelve rows, one per cell.
    ///
    /// # Errors
    ///
    /// [`ReactionError`] for a missing header, an unknown word, a missing or
    /// out-of-range key, or a cell without exactly one row.
    pub fn parse(text: &str) -> Result<Self, ReactionError> {
        let mut cells: [[Option<Shape>; 3]; 4] = [[None; 3]; 4];
        let mut header = false;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            if !header {
                if raw.split_whitespace().collect::<Vec<_>>()
                    != ["table", "hit-reaction", "version", "1"]
                {
                    return Err(ReactionError::Header);
                }
                header = true;
                continue;
            }
            let mut words = raw.split_whitespace();
            let (first, second) = (
                words.next().unwrap_or_default(),
                words.next().unwrap_or_default(),
            );
            let sector = SECTORS
                .iter()
                .copied()
                .find(|sector| sector.key() == first)
                .ok_or_else(|| ReactionError::Word {
                    line,
                    word: first.to_string(),
                })?;
            let weight = WEIGHTS
                .iter()
                .copied()
                .find(|weight| weight.key() == second)
                .ok_or_else(|| ReactionError::Word {
                    line,
                    word: second.to_string(),
                })?;
            let field = |key: &'static str, range: (i32, i32)| -> Result<i32, ReactionError> {
                let text = raw
                    .split_whitespace()
                    .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
                    .ok_or(ReactionError::Missing { line, key })?;
                text.parse::<i32>()
                    .ok()
                    .filter(|number| (range.0..=range.1).contains(number))
                    .ok_or_else(|| ReactionError::Bad {
                        line,
                        key,
                        value: text.to_string(),
                    })
            };
            let pct = |value: i32| f32::from(i16::try_from(value).unwrap_or(0)) / 100.0;
            let shape = Shape {
                push: pct(field("push_pct", (0, 400))?),
                dip: pct(field("dip_pct", (0, 400))?),
                lean: pct(field("lean_pct", (0, 250))?),
                twist_rad: f32::from(i16::try_from(field("twist_mrad", (-300, 300))?).unwrap_or(0))
                    / 1000.0,
                window_ticks: u32::try_from(field("window_ticks", (10, 90))?)
                    .unwrap_or(REFERENCE_WINDOW_TICKS),
            };
            let cell = &mut cells[sector.index()][weight.index()];
            if cell.is_some() {
                return Err(ReactionError::Cell(sector.key(), weight.key()));
            }
            *cell = Some(shape);
        }
        if !header {
            return Err(ReactionError::Header);
        }
        let mut out = Self::neutral();
        for sector in SECTORS {
            for weight in WEIGHTS {
                out.cells[sector.index()][weight.index()] =
                    cells[sector.index()][weight.index()]
                        .ok_or_else(|| ReactionError::Cell(sector.key(), weight.key()))?;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_rows_parse_and_the_light_front_row_is_the_reference_flinch() {
        let table = ReactionTable::parse(REACTION_ROWS).expect("the shipped rows");
        assert_eq!(
            ReactionTable::shipped(),
            &table,
            "the look reads the shipped rows"
        );
        assert_eq!(
            table.shape(Sector::Front, Weight::Light),
            Shape::NEUTRAL,
            "a light frontal blow flinches exactly as it always did"
        );
        assert_eq!(Shape::NEUTRAL.window_ticks, REFERENCE_WINDOW_TICKS);
    }

    #[test]
    fn a_harder_blow_reads_bigger_and_longer_from_every_side() {
        let table = ReactionTable::shipped();
        for sector in SECTORS {
            let [light, heavy, down] = WEIGHTS.map(|weight| table.shape(sector, weight));
            for (soft, hard) in [(light, heavy), (heavy, down)] {
                assert!(hard.push > soft.push, "{sector:?} push");
                assert!(hard.dip > soft.dip, "{sector:?} dip");
                assert!(hard.lean > soft.lean, "{sector:?} lean");
                assert!(hard.window_ticks > soft.window_ticks, "{sector:?} window");
                assert!(
                    hard.twist_rad.abs() >= soft.twist_rad.abs(),
                    "{sector:?} twist"
                );
            }
        }
    }

    #[test]
    fn direction_changes_the_reaction() {
        let table = ReactionTable::shipped();
        let front = table.shape(Sector::Front, Weight::Heavy);
        let back = table.shape(Sector::Back, Weight::Heavy);
        let (left, right) = (
            table.shape(Sector::Left, Weight::Heavy),
            table.shape(Sector::Right, Weight::Heavy),
        );
        assert!(
            back.lean > front.lean,
            "a hit from behind arches the back further"
        );
        assert_eq!(front.twist_rad, 0.0);
        assert_eq!(back.twist_rad, 0.0);
        assert!(
            left.twist_rad > 0.0 && right.twist_rad < 0.0,
            "a flank blow turns the body away"
        );
        assert_eq!(left.twist_rad, -right.twist_rad, "the flanks mirror");
        assert_eq!((left.push, left.lean), (right.push, right.lean));
    }

    #[test]
    fn the_weight_of_a_blow_is_the_hardest_it_earned() {
        assert_eq!(Weight::of(false, false), Weight::Light);
        assert_eq!(Weight::of(true, false), Weight::Heavy);
        assert_eq!(Weight::of(false, true), Weight::Down);
        assert_eq!(Weight::of(true, true), Weight::Down);
    }

    #[test]
    fn the_sector_follows_the_facing() {
        // A victim facing +z (yaw 0), +x being its right.
        let yaw = 0.0;
        // An attacker at +z pushes the victim toward -z: from the front.
        assert_eq!(Sector::of(yaw, [0.0, -1.0]), Sector::Front);
        // An attacker behind (at -z) pushes toward +z.
        assert_eq!(Sector::of(yaw, [0.0, 1.0]), Sector::Back);
        // An attacker at +x (the victim's right) pushes toward -x.
        assert_eq!(Sector::of(yaw, [-1.0, 0.0]), Sector::Right);
        assert_eq!(Sector::of(yaw, [1.0, 0.0]), Sector::Left);
        // Turn the victim a quarter toward +x: the +x attacker is now ahead.
        let quarter = std::f32::consts::FRAC_PI_2;
        assert_eq!(Sector::of(quarter, [-1.0, 0.0]), Sector::Front);
        assert_eq!(Sector::of(quarter, [1.0, 0.0]), Sector::Back);
        // The corners belong to the front and back, so a diagonal duel reads as front.
        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(Sector::of(yaw, [-diagonal, -diagonal]), Sector::Front);
        // No push, no side, bad numbers: the reference sector.
        assert_eq!(Sector::of(yaw, [0.0, 0.0]), Sector::Front);
        assert_eq!(Sector::of(f32::NAN, [0.0, 1.0]), Sector::Front);
        assert_eq!(Sector::of(yaw, [f32::INFINITY, 0.0]), Sector::Front);
    }

    #[test]
    fn bad_rows_are_refused_with_the_line_and_the_key() {
        assert_eq!(
            ReactionTable::parse("front light"),
            Err(ReactionError::Header)
        );
        let head = "table hit-reaction version 1\n";
        assert!(matches!(
            ReactionTable::parse(&format!("{head}up light push_pct=1")),
            Err(ReactionError::Word { line: 2, .. })
        ));
        assert!(matches!(
            ReactionTable::parse(&format!("{head}front light push_pct=100")),
            Err(ReactionError::Missing {
                line: 2,
                key: "dip_pct"
            })
        ));
        let row = REACTION_ROWS
            .lines()
            .find(|line| line.starts_with("front light"))
            .expect("the reference row");
        assert!(matches!(
            ReactionTable::parse(&format!(
                "{head}{}",
                row.replace("lean_pct=100", "lean_pct=900")
            )),
            Err(ReactionError::Bad {
                key: "lean_pct",
                ..
            })
        ));
        // Twelve cells, each once.
        assert_eq!(
            ReactionTable::parse(&format!("{head}{row}\n{row}")),
            Err(ReactionError::Cell("front", "light"))
        );
        assert!(matches!(
            ReactionTable::parse(&format!("{head}{row}")),
            Err(ReactionError::Cell(..))
        ));
    }

    #[test]
    fn a_longer_window_stretches_the_envelope() {
        // The reference row reads the envelope tick for tick, exactly.
        for elapsed in 0..60 {
            assert_eq!(Shape::NEUTRAL.envelope_at(elapsed), (elapsed, 0.0));
        }
        // A 40-tick window plays the 27-tick envelope slower: 40 ticks in is
        // the end of it, 20 ticks in is just past its middle.
        let down = ReactionTable::shipped().shape(Sector::Front, Weight::Down);
        assert_eq!(down.window_ticks, 40);
        assert_eq!(down.envelope_at(40), (27, 0.0));
        let (whole, frac) = down.envelope_at(20);
        assert_eq!(whole, 13);
        assert!((frac - 0.5).abs() < 1e-6, "{frac}");
        let mut last = (0, 0.0_f32);
        for elapsed in 0..=40 {
            let now = down.envelope_at(elapsed);
            assert!(
                now.0 > last.0 || (now.0 == last.0 && now.1 >= last.1),
                "monotonic at {elapsed}"
            );
            last = now;
        }
    }

    #[test]
    fn every_cell_has_its_own_index_blend_selector() {
        let table = ReactionTable::shipped();
        let mut seen = [false; 12];
        for sector in SECTORS {
            for weight in WEIGHTS {
                let cell = selector(sector, weight);
                assert!(usize::from(cell) < seen.len(), "{sector:?} {weight:?}");
                assert!(!seen[usize::from(cell)], "two cells share {cell}");
                seen[usize::from(cell)] = true;
                assert_eq!(
                    table.shape_by_selector(cell),
                    Some(table.shape(sector, weight)),
                    "the selector reads its own cell"
                );
            }
        }
        assert!(seen.iter().all(|hit| *hit), "all twelve cells are named");
        assert_eq!(
            table.shape_by_selector(12),
            None,
            "a stale index names no cell"
        );
    }
}
