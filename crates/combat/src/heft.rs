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
//!
//! BG15: a row also names the condition its steel arrives with
//! (`edge_wear_permille`), and [`Heft::edge_wear_q16`](crate::heft::Heft::edge_wear_q16) turns it into the
//! dulled fraction the cut resolver already scales a swing by. A row that
//! names no wear is a fresh edge (`0`), so an un-marked mod table cuts as it
//! did before.

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
    /// How far the row's edge is dulled when the kit draws it, in permille
    /// of a fully dulled edge (`0..=1000`, `0` is fresh; BG15).
    pub edge_wear_permille: u16,
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
    /// `speed_ticks`, `recovery_ticks`, `stagger_permille` and, when the row
    /// names it, `edge_wear_permille`; other keys are the design tests'
    /// business and are skipped here. A row with no `edge_wear_permille` is a
    /// fresh edge (`0`), so an older table arms exactly as it did.
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
        let value = |key: &'static str| {
            line.split_whitespace()
                .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
        };
        let field = |key: &'static str, range: (u32, u32)| -> Result<u32, HeftError> {
            let text = value(key).ok_or_else(|| HeftError::Missing {
                id: id.to_string(),
                key,
            })?;
            text.parse::<u32>()
                .ok()
                .filter(|number| (range.0..=range.1).contains(number))
                .ok_or_else(|| HeftError::Bad {
                    id: id.to_string(),
                    key,
                    value: text.to_string(),
                    range,
                })
        };
        let damage = field("damage_permille", (1, u32::from(MAX_HP_PERMILLE)))?;
        let stagger = field("stagger_permille", (0, 1000))?;
        // Optional, so a mod table that predates BG15 still arms: absent or
        // `0` is a fresh edge.
        let wear = match field("edge_wear_permille", (0, 1000)) {
            Ok(number) => number,
            Err(HeftError::Missing { .. }) => 0,
            Err(bad) => return Err(bad),
        };
        Ok(Self {
            mass_g: field("mass_g", (1, MAX_MASS_G))?,
            damage: u16::try_from(damage).unwrap_or(MAX_HP_PERMILLE),
            startup_ticks: field("speed_ticks", (1, MAX_STARTUP_TICKS))?,
            recovery_ticks: field("recovery_ticks", (0, MAX_RECOVERY_TICKS))?,
            react_ticks: react_ticks_for(stagger),
            edge_wear_permille: u16::try_from(wear).unwrap_or(1000),
        })
    }

    /// The row's dulled edge as the cut resolver wants it: Q16 of one
    /// (`0..=65536`), so `500` permille reads `32_768`. Pure integer, exact
    /// at both ends. Determinism scope: no float, identical on every PC.
    #[must_use]
    pub fn edge_wear_q16(&self) -> u32 {
        u32::from(self.edge_wear_permille).saturating_mul(crate::cut::MAX_WEAR_Q16) / 1000
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

/// The base hit impulse under rules v1 (and v2 with no weapon armed): the
/// constant the arena feeds every blow. A v1 bout or an unarmed seat reads
/// exactly this, so those replays never move (FEEL2's momentum path only
/// opens for an armed weapon).
pub const BASE_HIT_Q8: u16 = 384;

/// Blade travel the momentum reads, capped (millimetres per tick): a
/// teleporting blade is not a swing.
pub const MAX_BLADE_TRAVEL_MM: u32 = 128;

/// Largest momentum bonus one blow may add to the base impulse, in Q8
/// units: base plus bonus stays well inside the balance ceiling
/// (`engine_physics::balance::MAX_IMPULSE_Q8`, 4096 Q8).
pub const MAX_MOMENTUM_BONUS_Q8: u16 = 1024;

/// Q8 impulse per gram-millimetre of blade momentum, as a right shift:
/// mass times this tick's blade travel, divided by 256. The arena's real
/// swing - the blade's span over the weapon's startup ticks, 54 to 108 mm
/// a tick - puts the shipped rows between 126 and 442 Q8 over the base, so
/// a dagger stumbles (510) and a sword floors (719 to 826) without the
/// cap ever binding.
pub const MOMENTUM_Q8_SHIFT: u32 = 8;

/// The hit impulse one blow gives the victim: the base impulse plus the
/// weapon's momentum - its mass times this tick's blade travel - capped.
/// `heft` is `None` for v1 rows and an unarmed seat, which reads exactly
/// the base, so those bouts replay byte for byte.
///
/// Monotone in mass at fixed travel (a heavier blow lands harder) and in
/// travel at fixed mass (a faster swing lands harder); the balance layer
/// turns the impulse into distance. Determinism: one multiply, one shift,
/// no float and no clock.
pub fn hit_impulse_q8(heft: Option<Heft>, blade_travel_mm: u32) -> u16 {
    let Some(heft) = heft else {
        return BASE_HIT_Q8;
    };
    let travel = blade_travel_mm.min(MAX_BLADE_TRAVEL_MM);
    let momentum = heft.mass_g.saturating_mul(travel) >> MOMENTUM_Q8_SHIFT;
    let bonus = u16::try_from(momentum)
        .unwrap_or(MAX_MOMENTUM_BONUS_Q8)
        .min(MAX_MOMENTUM_BONUS_Q8);
    BASE_HIT_Q8.saturating_add(bonus)
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
    fn momentum_impulse_reads_the_base_without_an_armed_weapon() {
        // Rules v1 and an unarmed seat: the constant, byte for byte.
        assert_eq!(hit_impulse_q8(None, 30), BASE_HIT_Q8);
        assert_eq!(hit_impulse_q8(None, 0), BASE_HIT_Q8);
        assert_eq!(BASE_HIT_Q8, 384);
    }

    #[test]
    fn momentum_impulse_follows_the_shipped_rows_over_their_own_swings() {
        // The arena's real swing: the blade's 1 300 mm span over each
        // weapon's startup ticks, so the travel shrinks as the weapon grows.
        // The momentum (mass times travel) still grows with the weight, so
        // every row lands harder than the one below it and no row reaches
        // the bonus cap.
        let rows: [(&str, u32, u32); 5] = [
            ("pugio", 300, 12),
            ("gladius", 1_000, 15),
            ("spatha", 1_300, 18),
            ("clava", 1_700, 21),
            ("pilum", 2_100, 24),
        ];
        let mut last: Option<(u16, u64)> = None;
        for (id, mass, startup) in rows {
            let heft = Heft::from_rows(ROWS, id).expect("the shipped row parses");
            assert_eq!(heft.mass_g, mass, "{id} weighs {mass} g");
            let travel = 1_300 / startup;
            let impulse = hit_impulse_q8(Some(heft), travel);
            let momentum = u64::from(mass) * u64::from(travel);
            println!("{id}: travel {travel} mm, momentum {momentum}, impulse {impulse}");
            assert!(
                impulse < BASE_HIT_Q8 + MAX_MOMENTUM_BONUS_Q8,
                "{id} must stay under the bonus cap"
            );
            if let Some((last_impulse, last_momentum)) = last {
                assert!(momentum > last_momentum, "{id} must carry more momentum");
                assert!(impulse > last_impulse, "{id} must land harder");
            }
            last = Some((impulse, momentum));
        }
    }

    #[test]
    fn momentum_impulse_heavier_lands_harder_at_fixed_travel() {
        // The shipped rows in file order: pugio 300 g, gladius 1000 g,
        // clava 1700 g, pilum 2100 g. Heavier lands harder at one travel.
        let light = hit_impulse_q8(Heft::from_rows(ROWS, "pugio").ok(), 30);
        let mid = hit_impulse_q8(Heft::from_rows(ROWS, "gladius").ok(), 30);
        let clava = hit_impulse_q8(Heft::from_rows(ROWS, "clava").ok(), 30);
        let heavy = hit_impulse_q8(Heft::from_rows(ROWS, "pilum").ok(), 30);
        assert_eq!(light, 419);
        assert_eq!(mid, 501);
        assert_eq!(clava, 583);
        assert_eq!(heavy, 630);
        assert!(
            light < mid && mid < clava && clava < heavy,
            "{light} < {mid} < {clava} < {heavy}"
        );
    }

    #[test]
    fn momentum_impulse_faster_swing_lands_harder_at_fixed_mass() {
        let gladius = Heft::from_rows(ROWS, "gladius").expect("the gladius row parses");
        let slow = hit_impulse_q8(Some(gladius), 10);
        let fast = hit_impulse_q8(Some(gladius), 60);
        assert!(slow < fast, "{slow} < {fast}");
    }

    #[test]
    fn momentum_impulse_caps_a_huge_travel_or_mass() {
        let mut huge = Heft::from_rows(ROWS, "pilum").expect("the pilum row parses");
        huge.mass_g = MAX_MASS_G;
        assert_eq!(
            hit_impulse_q8(Some(huge), u32::MAX),
            BASE_HIT_Q8 + MAX_MOMENTUM_BONUS_Q8
        );
    }

    /// BG15: a row names the condition its steel arrives in, and the cut
    /// resolver's dulled fraction is that permille of a fully dulled edge.
    #[test]
    fn edge_wear_reads_the_rows_and_scales_the_edge() {
        use crate::cut::MAX_WEAR_Q16;
        // Every shipped row parses with the new key inside its range.
        for (id, permille) in [
            ("pugio", 80u16),
            ("gladius", 120),
            ("spatha", 90),
            ("clava", 0),
            ("pilum", 150),
        ] {
            let row = Heft::from_rows(ROWS, id).expect("the shipped row parses");
            assert_eq!(row.edge_wear_permille, permille, "{id} ships its wear");
            assert!(row.edge_wear_q16() <= MAX_WEAR_Q16, "{id} stays in range");
        }
        // The conversion is the permille of the resolver's full dulled edge.
        let gladius = Heft::from_rows(ROWS, "gladius").expect("the gladius row parses");
        assert_eq!(gladius.edge_wear_q16(), MAX_WEAR_Q16 * 120 / 1000);
        // A blunt weapon has no edge to dull, and an unmarked row is fresh
        // steel, so a mod table written before BG15 arms exactly as it did.
        assert_eq!(
            Heft::from_rows(ROWS, "clava")
                .expect("the clava row parses")
                .edge_wear_q16(),
            0
        );
        let fresh = Heft::from_rows(
            "table weapons version 2\nold mass_g=1000 damage_permille=120 speed_ticks=15 recovery_ticks=8 stagger_permille=400",
            "old",
        )
        .expect("a row without the key parses");
        assert_eq!(fresh.edge_wear_q16(), 0);
        // Wear outside the permille scale is the rows' own refusal.
        assert!(Heft::from_rows(
            "table weapons version 2\nbad mass_g=1000 damage_permille=120 speed_ticks=15 recovery_ticks=8 stagger_permille=400 edge_wear_permille=1001",
            "bad",
        )
        .is_err());
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
