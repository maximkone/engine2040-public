//! Weapon weight in the fight (FEEL1): one weapon row's own startup,
//! recovery, damage and the stagger it deals, read from the designer rows in
//! `content/design/weapons.rows`.
//!
//! A swinger with no weapon armed (its `heft` is `None`) swings the
//! table row, so every match that never arms one steps exactly as before and
//! rules v1 replays never move. The gladius row equals the shipped table
//! (startup 15, recovery 8, damage 120, hit react 6): only the other weapons
//! change a bout. Integer only; parsing is a cold path, the step reads four
//! plain fields.

use crate::fight::{Tables, MAX_HP_PERMILLE};

/// Hit-react ticks a full-poise stagger (1000 permille) holds on the victim:
/// a quarter second at 60 Hz. The gladius row (400 permille) gives the
/// shipped 6 ticks; a heavier blow holds the victim longer.
pub const STAGGER_TICKS_AT_FULL: u32 = 15;

/// Heaviest weapon a row may name, in grams (a two-handed maul is near 10 kg).
pub const MAX_MASS_G: u32 = 20_000;

/// Slowest startup a row may name, in ticks (one second, as [`Tables`] rows).
const MAX_STARTUP_TICKS: u32 = 60;

/// Longest recovery a row may name, in ticks (two seconds, as [`Tables`] rows).
const MAX_RECOVERY_TICKS: u32 = 120;

/// One weapon's weight in the fight: what its swing costs in time and what
/// it does to the body it lands on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Heft {
    /// Mass of the weapon in grams: the momentum a blow carries (FEEL2).
    pub mass_g: u32,
    /// Damage of one clean hit in permille of full health.
    pub damage: u16,
    /// Ticks from the press to the impact: a heavier weapon starts later.
    pub startup_ticks: u32,
    /// Ticks the swinger is committed after the impact.
    pub recovery_ticks: u32,
    /// Ticks a clean hit holds the victim in hit react: the stagger it deals.
    pub react_ticks: u32,
}

/// Why a weapon row was refused: the row and the field, never a guess.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HeftError {
    /// The rows hold no weapon with this id.
    NoRow(String),
    /// The row lacks a field the fight needs.
    Missing {
        /// Weapon id.
        id: String,
        /// The missing key.
        key: &'static str,
    },
    /// A field is not a whole number inside its range.
    Bad {
        /// Weapon id.
        id: String,
        /// The offending key.
        key: &'static str,
        /// The text the row holds for it.
        value: String,
        /// The range the fight takes, as `min..=max`.
        range: (u32, u32),
    },
}

impl core::fmt::Display for HeftError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoRow(id) => write!(f, "weapons rows: no weapon `{id}`"),
            Self::Missing { id, key } => write!(f, "weapons rows: `{id}` has no `{key}=`"),
            Self::Bad {
                id,
                key,
                value,
                range: (min, max),
            } => write!(
                f,
                "weapons rows: `{id}` {key}={value} must be a whole number in {min}..={max}"
            ),
        }
    }
}

impl std::error::Error for HeftError {}

impl Heft {
    /// The weapon `id` from weapons rows text (`<id> key=int ...` lines, `#`
    /// comments, a `table` header). Reads `mass_g`, `damage_permille`,
    /// `speed_ticks`, `recovery_ticks` and `stagger_permille`; other keys are
    /// the design tests' business and are skipped here.
    ///
    /// # Errors
    ///
    /// [`HeftError`] names the row and the field when the id is absent, a
    /// field is missing, or a value is not a whole number in its range.
    pub fn from_rows(text: &str, id: &str) -> Result<Self, HeftError> {
        let line = text
            .lines()
            .map(str::trim)
            .find(|line| line.split_whitespace().next() == Some(id))
            .ok_or_else(|| HeftError::NoRow(id.to_string()))?;
        let field = |key: &'static str, range: (u32, u32)| -> Result<u32, HeftError> {
            let text = line
                .split_whitespace()
                .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
                .ok_or_else(|| HeftError::Missing {
                    id: id.to_string(),
                    key,
                })?;
            text.parse::<u32>()
                .ok()
                .filter(|value| (range.0..=range.1).contains(value))
                .ok_or_else(|| HeftError::Bad {
                    id: id.to_string(),
                    key,
                    value: text.to_string(),
                    range,
                })
        };
        let damage = field("damage_permille", (1, u32::from(MAX_HP_PERMILLE)))?;
        let stagger = field("stagger_permille", (0, 1000))?;
        Ok(Self {
            mass_g: field("mass_g", (1, MAX_MASS_G))?,
            damage: u16::try_from(damage).unwrap_or(MAX_HP_PERMILLE),
            startup_ticks: field("speed_ticks", (1, MAX_STARTUP_TICKS))?,
            recovery_ticks: field("recovery_ticks", (0, MAX_RECOVERY_TICKS))?,
            react_ticks: react_ticks_for(stagger),
        })
    }
}

/// Hit-react ticks for a stagger in permille, to the nearest tick:
/// `stagger * STAGGER_TICKS_AT_FULL / 1000`. Pure.
#[must_use]
pub const fn react_ticks_for(stagger_permille: u32) -> u32 {
    stagger_permille
        .saturating_mul(STAGGER_TICKS_AT_FULL)
        .saturating_add(500)
        / 1000
}

/// Ticks from the press to the impact for a swinger carrying `heft`: its
/// weapon's, or the table row's when it swings none.
#[must_use]
pub const fn startup(heft: Option<Heft>, tables: &Tables) -> u32 {
    match heft {
        Some(heft) => heft.startup_ticks,
        None => tables.attack_startup_ticks,
    }
}

/// Ticks a swinger carrying `heft` stays committed after its impact.
#[must_use]
pub const fn recovery(heft: Option<Heft>, tables: &Tables) -> u32 {
    match heft {
        Some(heft) => heft.recovery_ticks,
        None => tables.attack_recovery_ticks,
    }
}

/// Damage of one clean bare-row hit from a swinger carrying `heft`, permille.
#[must_use]
pub const fn damage(heft: Option<Heft>, tables: &Tables) -> u16 {
    match heft {
        Some(heft) => heft.damage,
        None => tables.attack_damage,
    }
}

/// Hit-react ticks a clean hit from a swinger carrying `heft` holds its victim.
#[must_use]
pub const fn react(heft: Option<Heft>, tables: &Tables) -> u32 {
    match heft {
        Some(heft) => heft.react_ticks,
        None => tables.hit_react_ticks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::{FighterState, TABLES_V2};

    /// The shipped designer rows: the numbers the live duel arms.
    const ROWS: &str = include_str!("../../../content/design/weapons.rows");

    /// Every weapon id in the rows, in file order.
    fn ids() -> Vec<&'static str> {
        ROWS.lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty() && !line.starts_with('#') && !line.starts_with("table ")
            })
            .filter_map(|line| line.split_whitespace().next())
            .collect()
    }

    #[test]
    fn the_gladius_row_is_the_shipped_table() {
        let gladius = Heft::from_rows(ROWS, "gladius").expect("the gladius row parses");
        assert_eq!(gladius.startup_ticks, TABLES_V2.attack_startup_ticks);
        assert_eq!(gladius.recovery_ticks, TABLES_V2.attack_recovery_ticks);
        assert_eq!(gladius.damage, TABLES_V2.attack_damage);
        assert_eq!(gladius.react_ticks, TABLES_V2.hit_react_ticks);
    }

    #[test]
    fn heavier_weapons_start_later_recover_longer_and_stagger_harder() {
        let mut rows: Vec<(&str, Heft)> = ids()
            .into_iter()
            .map(|id| {
                (
                    id,
                    Heft::from_rows(ROWS, id).expect("every shipped row parses"),
                )
            })
            .collect();
        assert!(
            rows.len() >= 5,
            "the rows hold a dagger, swords, a mace and a spear"
        );
        rows.sort_by_key(|(_, heft)| heft.mass_g);
        for pair in rows.windows(2) {
            let ((light_id, light), (heavy_id, heavy)) = (pair[0], pair[1]);
            assert!(
                heavy.mass_g > light.mass_g,
                "{heavy_id} and {light_id} weigh the same"
            );
            assert!(
                heavy.startup_ticks >= light.startup_ticks
                    && heavy.recovery_ticks >= light.recovery_ticks
                    && heavy.react_ticks >= light.react_ticks,
                "{heavy_id} ({} g) must start, recover and stagger no less than {light_id} ({} g): \
                 {heavy:?} vs {light:?}",
                heavy.mass_g,
                light.mass_g
            );
        }
    }

    #[test]
    fn a_fighter_with_no_weapon_swings_the_table_row() {
        let bare = FighterState::default();
        assert_eq!(bare.heft, None);
        assert_eq!(
            startup(bare.heft, &TABLES_V2),
            TABLES_V2.attack_startup_ticks
        );
        assert_eq!(
            recovery(bare.heft, &TABLES_V2),
            TABLES_V2.attack_recovery_ticks
        );
        assert_eq!(damage(bare.heft, &TABLES_V2), TABLES_V2.attack_damage);
        assert_eq!(react(bare.heft, &TABLES_V2), TABLES_V2.hit_react_ticks);
    }

    #[test]
    fn a_bad_row_names_the_weapon_and_the_field() {
        assert_eq!(
            Heft::from_rows(ROWS, "falx"),
            Err(HeftError::NoRow("falx".to_string()))
        );
        let text = "maul mass_g=9000 damage_permille=300 speed_ticks=90 recovery_ticks=20 stagger_permille=900";
        let err = Heft::from_rows(text, "maul").expect_err("a 90-tick startup is refused");
        assert_eq!(
            err.to_string(),
            "weapons rows: `maul` speed_ticks=90 must be a whole number in 1..=60"
        );
        let no_mass =
            "maul damage_permille=300 speed_ticks=30 recovery_ticks=20 stagger_permille=900";
        let err = Heft::from_rows(no_mass, "maul").expect_err("no mass");
        assert!(
            matches!(err, HeftError::Missing { key: "mass_g", .. }),
            "{err}"
        );
    }

    #[test]
    fn stagger_rounds_to_the_nearest_tick() {
        assert_eq!(react_ticks_for(0), 0);
        assert_eq!(react_ticks_for(300), 5);
        assert_eq!(react_ticks_for(400), 6);
        assert_eq!(react_ticks_for(1000), STAGGER_TICKS_AT_FULL);
    }
}
