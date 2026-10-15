//! Per-kind hit-stop freeze rows: light taps, heavy lands, parry answers longest.
//!
//! Single home for the V2 kind-to-ticks map. The tick counts stay owned by
//! `crate::tables` (`LIGHT_HITSTOP_TICKS_V2` and friends); this module only
//! maps a strike kind to its row. Std-only, no heap, integer ticks only.
//! V1 rows keep the single `HITSTOP_TICKS` in `crate::fight` and never read
//! this map, so old tapes replay byte-identical.

use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
use crate::tables::{HEAVY_HITSTOP_TICKS_V2, LIGHT_HITSTOP_TICKS_V2, PARRY_HITSTOP_TICKS_V2};

/// Freeze ticks for one landed strike kind under V2 rows, at 60 Hz.
///
/// Light taps (3), heavy lands (6); unknown kinds never freeze (0), never
/// panic. Determinism: pure match on kind; same kind gives same freeze.
pub const fn for_kind(kind: u8) -> u32 {
    if kind == INPUT_LIGHT {
        LIGHT_HITSTOP_TICKS_V2
    } else if kind == INPUT_HEAVY {
        HEAVY_HITSTOP_TICKS_V2
    } else {
        0
    }
}

/// Freeze ticks for one parry under V2 rows, at 60 Hz.
///
/// Parry (8) freezes longest, so the answer reads. Determinism: pure const;
/// every call gives the same freeze.
pub const fn for_parry() -> u32 {
    PARRY_HITSTOP_TICKS_V2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hitstop_kinds() {
        // V2 per-kind rows: light 3, heavy 6, parry 8.
        assert_eq!(for_kind(INPUT_LIGHT), 3);
        assert_eq!(for_kind(INPUT_HEAVY), 6);
        assert_eq!(for_parry(), 8);
        // Rows stay single-owned in tables: map matches the consts.
        assert_eq!(for_kind(INPUT_LIGHT), LIGHT_HITSTOP_TICKS_V2);
        assert_eq!(for_kind(INPUT_HEAVY), HEAVY_HITSTOP_TICKS_V2);
        assert_eq!(for_parry(), PARRY_HITSTOP_TICKS_V2);
        // Failure case: unknown kind never freezes.
        assert_eq!(for_kind(9), 0);
        // Two builds give equal rows twice: deterministic construction.
        assert_eq!(for_kind(INPUT_HEAVY), for_kind(INPUT_HEAVY));
        assert_eq!(for_parry(), for_parry());
    }
}
