//! Draw/triangle/memory budgets as pure functions of scene data.
//!
//! Consumer: [`crate::verify`] plus the frame checks, which read [`Estimate`]
//! to size captures before rendering. Data owner: the caller owns the
//! [`crate::ArenaVariant`]. Units: draws (one per prop), triangles and
//! bytes, all integers with saturating sums. Determinism scope: total —
//! summation order is prop order, and the estimate is input-order invariant
//! anyway. Error type: [`crate::Failure`] rows, never panics.

use super::{ArenaVariant, Failure, Prop};

/// This verifier's name in [`crate::Failure::verifier`].
pub const VERIFIER: &str = "Budgets";

/// Budget estimate for a variant: pure function of the prop list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Estimate {
    /// One draw per prop.
    pub draws: u64,
    /// Saturating sum of [`Prop::triangles`].
    pub triangles: u64,
    /// Saturating sum of [`Prop::memory_bytes`].
    pub memory_bytes: u64,
}

/// Estimate draws, triangles and resident bytes for `props`.
pub fn estimate(props: &[Prop]) -> Estimate {
    let mut triangles = 0u64;
    let mut memory_bytes = 0u64;
    for prop in props {
        triangles = triangles.saturating_add(u64::from(prop.triangles));
        memory_bytes = memory_bytes.saturating_add(u64::from(prop.memory_bytes));
    }
    Estimate {
        draws: props.len() as u64,
        triangles,
        memory_bytes,
    }
}

fn fail(reason: String) -> Failure {
    Failure {
        verifier: VERIFIER,
        reason,
    }
}

/// Check the estimate against the variant caps in fixed
/// draws/triangles/memory order. Returns empty on a pass.
pub fn check(variant: &ArenaVariant) -> Vec<Failure> {
    let got = estimate(&variant.props);
    let caps = &variant.budgets;
    let mut out = Vec::new();
    if got.draws > u64::from(caps.max_draws) {
        out.push(fail(format!(
            "draws {} exceeds cap {} over {} props",
            got.draws,
            caps.max_draws,
            variant.props.len()
        )));
    }
    if got.triangles > caps.max_triangles {
        out.push(fail(format!(
            "triangles {} exceeds cap {}",
            got.triangles, caps.max_triangles
        )));
    }
    if got.memory_bytes > caps.max_memory_bytes {
        out.push(fail(format!(
            "memory {} bytes exceeds cap {} bytes",
            got.memory_bytes, caps.max_memory_bytes
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{reference_arena, Prop};
    use super::*;

    #[test]
    fn reference_arena_fits_its_caps() {
        let variant = reference_arena();
        let got = estimate(&variant.props);
        assert_eq!(got.draws, 4);
        assert_eq!(got.triangles, 4800);
        assert_eq!(got.memory_bytes, 192_000);
        assert!(check(&variant).is_empty());
    }

    #[test]
    fn each_overrun_is_its_own_failure() {
        let mut variant = reference_arena();
        variant.budgets.max_draws = 0;
        variant.budgets.max_triangles = 0;
        variant.budgets.max_memory_bytes = 0;
        let failures = check(&variant);
        assert_eq!(failures.len(), 3);
        assert!(failures.iter().all(|f| f.verifier == VERIFIER));
        assert!(failures[0].reason.contains("draws"));
        assert!(failures[1].reason.contains("triangles"));
        assert!(failures[2].reason.contains("memory"));
    }

    #[test]
    fn one_hungry_prop_blows_the_triangle_cap() {
        let mut variant = reference_arena();
        variant.props.push(Prop {
            id: 30,
            name: "statue".to_string(),
            x: 0.0,
            y: 1.0,
            z: -5.0,
            sx: 1.0,
            sy: 2.0,
            sz: 1.0,
            physical: false,
            triangles: 2_000_000,
            memory_bytes: 64,
        });
        let failures = check(&variant);
        assert_eq!(failures.len(), 1);
        assert!(failures[0]
            .reason
            .contains("triangles 2004800 exceeds cap 1000000"));
    }

    #[test]
    fn estimate_is_order_invariant_and_saturating() {
        let variant = reference_arena();
        let mut rev = variant.props.clone();
        rev.reverse();
        assert_eq!(estimate(&rev), estimate(&variant.props));
        let huge = vec![
            Prop {
                id: 0,
                name: "a".to_string(),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                physical: false,
                triangles: u32::MAX,
                memory_bytes: u32::MAX,
            },
            Prop {
                id: 1,
                name: "b".to_string(),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                physical: false,
                triangles: u32::MAX,
                memory_bytes: u32::MAX,
            },
        ];
        let got = estimate(&huge);
        assert_eq!(got.triangles, 2 * u64::from(u32::MAX));
        assert_eq!(got.draws, 2);
    }
}
