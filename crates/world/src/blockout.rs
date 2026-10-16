//! blockout: recipe-keyed deterministic maps. Same seed + recipe + size = same bytes.
use crate::BakeErr;
use engine_core::hash_bytes;

/// Mix a recipe name into a seed: `seed ^ (fnv(recipe) * 0x9E3779B1)`.
pub fn effective_seed(seed: u64, recipe: &str) -> u64 {
    seed ^ hash_bytes(recipe.as_bytes()).wrapping_mul(0x9E3779B1)
}

/// Bake a blockout map: versioned bytes via the world height bake at the
/// effective seed. Deterministic on (seed, recipe, size). Errors:
/// [`BakeErr::TooLarge`] past `MAX_BAKE_SIZE`.
pub fn bake_blockout(seed: u64, recipe: &str, size: u16) -> Result<Vec<u8>, BakeErr> {
    crate::bake(effective_seed(seed, recipe), size)
}

/// Hash a baked blockout (bake SHA gate).
pub fn blockout_hash(bytes: &[u8]) -> u64 {
    crate::world_hash(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blockout(seed: u64, recipe: &str, size: u16) -> Vec<u8> {
        bake_blockout(seed, recipe, size).expect("size within MAX_BAKE_SIZE")
    }

    #[test]
    fn same_inputs_identical() {
        let (a, b) = (
            blockout(2040, "arena-blockout", 16),
            blockout(2040, "arena-blockout", 16),
        );
        assert_eq!(a, b);
        assert_eq!(blockout_hash(&a), blockout_hash(&b));
    }

    #[test]
    fn recipe_and_seed_split() {
        let base = blockout(2040, "arena-blockout", 16);
        assert_ne!(base, blockout(2040, "wfc-city-20", 16));
        assert_ne!(base, blockout(7, "arena-blockout", 16));
        assert_ne!(base, blockout(2040, "arena-blockout", 8));
    }

    #[test]
    fn oversize_blockout_refuses() {
        assert_eq!(
            bake_blockout(2040, "arena-blockout", crate::MAX_BAKE_SIZE + 1),
            Err(BakeErr::TooLarge)
        );
    }

    #[test]
    fn effective_seed_deterministic() {
        assert_eq!(effective_seed(7, "a"), effective_seed(7, "a"));
        assert_ne!(effective_seed(7, "a"), effective_seed(7, "b"));
    }
}
