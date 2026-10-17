//! layers: the collision layer table as plain integers.
//!
//! Consumer: the arena kit zone lets a fighter walk through only the layers it
//! does not block, the door's cast verbs filter by a layer scope, and the cooked
//! collider bytes store one layer integer per collider. Data owner: this table;
//! the asset cook writes the integers and never names them. Units: one bit per
//! layer in a `u32`. Determinism: constants and bit math only, no state.
//! Errors: [`LayerError`] names the unknown layer word.
//!
//! A collider belongs to exactly one layer. Two colliders block each other when
//! either layer's row lists the other layer; the rows are written symmetric, so
//! the answer never depends on argument order. A sensor blocks nothing: it only
//! reports overlap.

use core::fmt;

/// Fighter capsules.
pub const FIGHTER: u32 = 1;
/// Dynamic props such as crates.
pub const PROP: u32 = 1 << 1;
/// Broken-off debris.
pub const DEBRIS: u32 = 1 << 2;
/// Trigger volumes: overlap is reported, nothing is blocked.
pub const SENSOR: u32 = 1 << 3;
/// Static kit pieces: walls, pillars, gates.
pub const WORLD: u32 = 1 << 4;
/// Every known layer bit.
pub const ALL: u32 = FIGHTER | PROP | DEBRIS | SENSOR | WORLD;

/// One row per layer: word, bit, and the layers it physically blocks.
const TABLE: [(&str, u32, u32); 5] = [
    ("fighter", FIGHTER, FIGHTER | PROP | WORLD),
    ("prop", PROP, FIGHTER | PROP | DEBRIS | WORLD),
    ("debris", DEBRIS, PROP | DEBRIS | WORLD),
    ("sensor", SENSOR, 0),
    ("world", WORLD, FIGHTER | PROP | DEBRIS),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_five_distinct_single_bits() {
        let mut seen = 0u32;
        for (word, bit, _) in TABLE {
            assert!(is_single(bit), "{word} is one bit");
            assert_eq!(seen & bit, 0, "{word} reuses a bit");
            seen |= bit;
        }
        assert_eq!(seen, ALL);
        assert_eq!(names().count(), 5);
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
        assert!(!blocks(WORLD, WORLD), "static pieces never collide with each other");
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
}
