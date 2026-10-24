//! layers: the collision layer table as plain integers.
//!
//! Consumer: the arena kit zone lets a fighter walk through only the layers it
//! does not block, the door's cast verbs filter by a layer scope, the cloth
//! step decides which colliders it may meet, and the cooked collider bytes
//! store one layer integer per collider. Data owner: this table; the asset cook
//! writes the integers and never names them. Units: one bit per layer in a
//! `u32`. Determinism: constants and bit math only, no state.
//! Errors: `LayerError` names the unknown layer word.
//!
//! A collider belongs to exactly one layer (its *group*) and carries an accept
//! *mask* of the groups it may meet. Two colliders collide when each one
//! accepts the other's group: the Bullet proxy pair rule, read live from
//! `btBroadphaseProxy.h` (zlib) on 2026-10-20. A sensor blocks nothing: it
//! only reports overlap.
//!
//! The table is written so both directions agree: layer `a` lists layer `b`
//! exactly when layer `b` lists layer `a`. The old one-way "a blocks b" read
//! is still there as `blocks`, so the arena kit zone and the door verbs keep
//! working unchanged.

use core::fmt;

/// Fighter capsules.
pub const FIGHTER: u32 = 1;
/// Neutral alias for [`FIGHTER`]: the body group. Files outside the genre
/// ledger name the group through this alias, so the word stays in one place.
pub const BODY: u32 = FIGHTER;
/// Dynamic props such as crates.
pub const PROP: u32 = 1 << 1;
/// Broken-off debris.
pub const DEBRIS: u32 = 1 << 2;
/// Trigger volumes: overlap is reported, nothing is blocked.
pub const SENSOR: u32 = 1 << 3;
/// Static kit pieces: walls, pillars, gates.
pub const WORLD: u32 = 1 << 4;
/// Cloth sheets: capes, banners, hair. Presentation only, never hashed.
pub const CLOTH: u32 = 1 << 5;
/// Soft bodies: flesh, muscle, jelly. Presentation only, never hashed.
pub const SOFT: u32 = 1 << 6;
/// Every known layer bit.
pub const ALL: u32 = FIGHTER | PROP | DEBRIS | SENSOR | WORLD | CLOTH | SOFT;

/// One row per layer: word, bit, and the layers it meets.
const TABLE: [(&str, u32, u32); 7] = [
    ("fighter", FIGHTER, FIGHTER | PROP | WORLD | CLOTH | SOFT),
    ("prop", PROP, FIGHTER | PROP | DEBRIS | WORLD | CLOTH | SOFT),
    ("debris", DEBRIS, PROP | DEBRIS | WORLD),
    ("sensor", SENSOR, 0),
    ("world", WORLD, FIGHTER | PROP | DEBRIS | CLOTH | SOFT),
    ("cloth", CLOTH, FIGHTER | PROP | WORLD | CLOTH),
    ("soft", SOFT, FIGHTER | PROP | WORLD | SOFT),
];

/// Why a layer word or mask was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerError {
    /// The word names no layer; carries the (bounded) word.
    Unknown(String),
    /// A scope named no layer at all.
    Empty,
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(word) => write!(
                f,
                "layer `{word}` is not one of {}",
                names().collect::<Vec<_>>().join(", ")
            ),
            Self::Empty => write!(f, "a layer scope names no layer"),
        }
    }
}

impl std::error::Error for LayerError {}

/// The layer words in table order.
pub fn names() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|row| row.0)
}

/// True when `layer` is exactly one known layer bit (the form a collider carries).
#[must_use]
pub fn is_single(layer: u32) -> bool {
    layer.is_power_of_two() && layer & ALL == layer
}

/// The word of one single layer bit.
#[must_use]
pub fn name(layer: u32) -> Option<&'static str> {
    TABLE.iter().find(|row| row.1 == layer).map(|row| row.0)
}

/// The layer bit of one word.
#[must_use]
pub fn layer(word: &str) -> Option<u32> {
    TABLE.iter().find(|row| row.0 == word).map(|row| row.1)
}

/// The union of the layers a scope of words names. Empty scope and unknown words
/// are refused, so a caller can never ask "everything" by naming nothing.
pub fn scope<S: AsRef<str>>(words: &[S]) -> Result<u32, LayerError> {
    let mut mask = 0;
    for word in words {
        let word = word.as_ref();
        mask |= layer(word).ok_or_else(|| LayerError::Unknown(word.chars().take(32).collect()))?;
    }
    if mask == 0 {
        return Err(LayerError::Empty);
    }
    Ok(mask)
}

/// The layers a set of layer bits physically blocks.
#[must_use]
pub fn blocks_mask(layers: u32) -> u32 {
    TABLE
        .iter()
        .filter(|row| layers & row.1 != 0)
        .fold(0, |mask, row| mask | row.2)
}

/// True when a collider on `a` and a collider on `b` block each other.
#[must_use]
pub fn blocks(a: u32, b: u32) -> bool {
    blocks_mask(a) & b != 0
}

/// The accept mask of one layer bit: the layers it meets. Zero for a bit that
/// names no layer, so an unknown group never collides with anything.
#[must_use]
pub fn default_mask(layer: u32) -> u32 {
    TABLE
        .iter()
        .find(|row| row.1 == layer)
        .map(|row| row.2)
        .unwrap_or(0)
}

/// One collider's collision pair: the group it is in and the groups it accepts.
///
/// The pair rule is Bullet's proxy rule, read live from `btBroadphaseProxy.h`
/// (zlib) on 2026-10-20: a pair collides when each side's group sits in the
/// other side's mask. The float world (Rapier) wants the same pair as
/// `InteractionGroups`, so the same bits travel to `crates/physfx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layers {
    /// The one group bit the collider is in.
    pub group: u32,
    /// The group bits the collider accepts.
    pub mask: u32,
}

impl Layers {
    /// The default pair for one group bit: the bit and the table's accept mask.
    ///
    /// # Errors
    /// [`LayerError::Unknown`] when `group` is not one known layer bit.
    pub fn new(group: u32) -> Result<Self, LayerError> {
        if !is_single(group) {
            return Err(LayerError::Unknown(format!("group {group}")));
        }
        Ok(Self {
            group,
            mask: default_mask(group),
        })
    }

    /// A pair with a custom accept mask: the group keeps its own bit, the mask
    /// says what this collider may meet.
    ///
    /// # Errors
    /// [`LayerError::Unknown`] when `group` is not one known layer bit.
    pub fn with_mask(group: u32, mask: u32) -> Result<Self, LayerError> {
        if !is_single(group) {
            return Err(LayerError::Unknown(format!("group {group}")));
        }
        Ok(Self { group, mask })
    }

    /// True when this collider accepts a collider in `group`.
    #[must_use]
    pub fn accepts(&self, group: u32) -> bool {
        self.mask & group != 0
    }
}

/// The Bullet proxy pair rule: two colliders collide when each side's group
/// sits in the other side's accept mask.
#[must_use]
pub fn collides(a: &Layers, b: &Layers) -> bool {
    a.accepts(b.group) && b.accepts(a.group)
}

/// The same rule with the table's default masks on both sides.
#[must_use]
pub fn collides_layers(a: u32, b: u32) -> bool {
    a & default_mask(b) != 0 && b & default_mask(a) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_seven_distinct_single_bits() {
        let mut seen = 0u32;
        for (word, bit, _) in TABLE {
            assert!(is_single(bit), "{word} is one bit");
            assert_eq!(seen & bit, 0, "{word} reuses a bit");
            seen |= bit;
        }
        assert_eq!(seen, ALL);
        assert_eq!(names().count(), 7);
        assert!(!is_single(0));
        assert!(!is_single(FIGHTER | PROP));
        assert!(!is_single(1 << 9));
    }

    #[test]
    fn blocking_is_symmetric_for_every_pair() {
        for (wa, a, _) in TABLE {
            for (wb, b, _) in TABLE {
                assert_eq!(blocks(a, b), blocks(b, a), "{wa} vs {wb}");
            }
        }
    }

    #[test]
    fn a_fighter_is_stopped_by_the_world_and_props_but_not_debris_or_sensors() {
        assert!(blocks(FIGHTER, WORLD));
        assert!(blocks(FIGHTER, PROP));
        assert!(blocks(FIGHTER, FIGHTER));
        assert!(!blocks(FIGHTER, DEBRIS));
        assert!(!blocks(FIGHTER, SENSOR));
    }

    #[test]
    fn a_sensor_blocks_nothing_and_the_world_holds_still() {
        for (_, bit, _) in TABLE {
            assert!(!blocks(SENSOR, bit));
        }
        assert!(
            !blocks(WORLD, WORLD),
            "static pieces never collide with each other"
        );
        assert!(blocks(DEBRIS, WORLD) && blocks(DEBRIS, PROP));
    }

    #[test]
    fn words_round_trip_and_scopes_refuse_the_empty_and_the_unknown() {
        for (word, bit, _) in TABLE {
            assert_eq!(layer(word), Some(bit));
            assert_eq!(name(bit), Some(word));
        }
        assert_eq!(name(FIGHTER | PROP), None, "a name is for one bit");
        assert_eq!(scope(&["fighter", "world"]), Ok(FIGHTER | WORLD));
        assert_eq!(scope::<&str>(&[]), Err(LayerError::Empty));
        let unknown = scope(&["fighter", "ghost"]).expect_err("ghost is not a layer");
        assert_eq!(unknown, LayerError::Unknown("ghost".to_owned()));
        assert!(unknown.to_string().contains("fighter, prop, debris"));
        let long = "x".repeat(200);
        let Err(LayerError::Unknown(echo)) = scope(&[long.as_str()]) else {
            panic!("a long unknown word refuses");
        };
        assert_eq!(echo.len(), 32, "the echo is bounded");
    }

    #[test]
    fn the_pair_rule_is_bullets_and_agrees_with_the_table() {
        // The pair rule and the table read must give the same answer for every
        // pair of layers, or the cook and the runtime would disagree.
        for (_, a, _) in TABLE {
            for (_, b, _) in TABLE {
                let pair = collides_layers(a, b);
                assert_eq!(pair, blocks(a, b), "group {a} vs group {b}");
            }
        }
        // Both directions must pass: a pair collides only when each side
        // accepts the other.
        let fighter = Layers::new(FIGHTER).expect("fighter group");
        let debris = Layers::new(DEBRIS).expect("debris group");
        assert!(
            !collides(&fighter, &debris),
            "a fighter does not meet debris"
        );
        let cloth = Layers::new(CLOTH).expect("cloth group");
        assert!(collides(&cloth, &fighter), "cloth meets a fighter");
        assert!(collides(&cloth, &cloth), "cloth meets cloth");
        assert!(
            !collides(&cloth, &debris),
            "cloth ignores debris: a cape must not be battered by pebbles"
        );
        let world = Layers::new(WORLD).expect("world group");
        assert!(!collides(&world, &world), "static holds still");
        assert!(collides(&cloth, &world), "cloth meets the world");
    }

    #[test]
    fn a_custom_mask_narrows_what_a_collider_meets() {
        let cloth = Layers::with_mask(CLOTH, FIGHTER | CLOTH).expect("cloth group");
        let world = Layers::new(WORLD).expect("world group");
        assert!(
            !collides(&cloth, &world),
            "a cloth collider with a narrow mask leaves the world alone"
        );
        let fighter = Layers::new(FIGHTER).expect("fighter group");
        assert!(collides(&cloth, &fighter));
    }

    #[test]
    fn an_unknown_group_is_refused() {
        assert!(Layers::new(0).is_err());
        assert!(Layers::new(FIGHTER | PROP).is_err());
        assert!(Layers::new(1 << 20).is_err());
        assert!(Layers::with_mask(0, FIGHTER).is_err());
    }
}
