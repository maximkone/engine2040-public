//! scale: whole-scene unit-mismatch detector.
//! Owner: spatial crate. Consumer: the `scene.spatial` door (AD-07), which
//! calls it first and lists its single finding first, so one unit slip reads
//! as one root cause instead of hundreds of per-object nudges; until the door
//! lands it is wired in-test below against the real median inputs.
//! Units: object sizes and the layout extent in integer centimetres; the
//! reported ratio is integer permille of the extent.
//! Evidence only: this module never rescales, moves or mutates a scene, it
//! returns at most one finding, and it is deliberately absent from
//! `SPATIAL_CHECKS` (length pinned at 18) and from the report hash.
//! Determinism: pure, integer math only, order-invariant over `sizes_cm` (the
//! slice is sorted inside), no float on the compare path; the same inputs
//! twice give the same answer. Budget: one sort of a room-scale slice, never
//! the tick.
//! Errors: none; an empty slice, a short population or a zero extent read
//! `None`, never a guess.

/// Object sizes a scene must hold before a median can prove a scene-wide unit
/// error. Below this population the detector stays silent
/// ([`unit_scale_finding`] returns `None`): a handful of objects is too weak
/// a sample to call a unit slip.
pub const MIN_POPULATION: usize = 16;

/// Mismatch band, in permille of the layout extent: a median object this many
/// permille of (or above) the extent names the root cause. At the band the
/// median equals the whole extent; twice the band means the typical object is
/// twice the layout. Below the band the detector stays silent.
pub const MIXED_UNITS_PERMILLE: u32 = 1_000;

/// Frozen root-cause line carried by every finding: one unit error makes the
/// whole scene read wrong, so the caller fixes the scene scale first, not the
/// individual objects.
pub const MIXED_UNITS_ROOT_CAUSE: &str =
    "scene mixes units: median object size at or above the layout extent";

/// One red root-cause finding: the median object size sits at or above the
/// layout extent, so the scene almost certainly arrived in the wrong unit.
/// Evidence only: the median, the extent and their integer ratio; nothing
/// here rescales the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleFinding {
    /// Why the scene reads mixed: the frozen root-cause line.
    pub root_cause: &'static str,
    /// Median object size over the population, integer centimetres.
    pub median_cm: u32,
    /// Layout extent the median was compared against, integer centimetres.
    pub extent_cm: u32,
    /// `median_cm` as permille of `extent_cm`, integer, saturating at
    /// `u32::MAX`; the compare uses the wide (u64) ratio, never this clamp.
    pub ratio_permille: u32,
    /// Always `"red"`: a unit error is a root cause, not a yellow advisory.
    pub severity: &'static str,
}

/// Lower median of the object sizes, in integer centimetres, or `None` for an
/// empty slice.
///
/// The lower median is the crate's single stated choice: for an even count the
/// smaller of the two middle values is returned (index `(len - 1) / 2` of the
/// ascending sort), so a size in the list is always reported and the answer
/// never split a value. Order-invariant: the slice is sorted internally, so
/// any input order gives the same number. Integer compare only; no float.
pub fn median_cm(sizes_cm: &[u32]) -> Option<u32> {
    if sizes_cm.is_empty() {
        return None;
    }
    let mut sorted = sizes_cm.to_vec();
    sorted.sort_unstable();
    sorted.get((sorted.len() - 1) / 2).copied()
}

/// One red finding when the scene mixes units, else `None`.
///
/// Rule, in numbers: with at least [`MIN_POPULATION`] object sizes the lower
/// median ([`median_cm`]) is taken; when that median is at or above the layout
/// extent (its integer ratio reaching [`MIXED_UNITS_PERMILLE`]) the scene
/// reads mixed and one red [`ScaleFinding`] is returned. Fewer than
/// [`MIN_POPULATION`] sizes, an empty slice or a zero extent read `None`: no
/// guess, no false alarm on a small scene or a scene with no layout.
///
/// Evidence only: the result is a value the caller reports; this function
/// never rescales, moves or mutates anything.
pub fn unit_scale_finding(sizes_cm: &[u32], extent_cm: u32) -> Option<ScaleFinding> {
    if sizes_cm.len() < MIN_POPULATION || extent_cm == 0 {
        return None;
    }
    let median_cm = median_cm(sizes_cm)?;
    let ratio_wide = u64::from(median_cm).saturating_mul(1_000) / u64::from(extent_cm);
    if ratio_wide < u64::from(MIXED_UNITS_PERMILLE) {
        return None;
    }
    Some(ScaleFinding {
        root_cause: MIXED_UNITS_ROOT_CAUSE,
        median_cm,
        extent_cm,
        ratio_permille: u32::try_from(ratio_wide).unwrap_or(u32::MAX),
        severity: "red",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_scale_x100_scene_flags_one_red() {
        // The old failure shape: 52 props authored x100 (a unit slip) inside a
        // 12 m room. The median object dwarfs the layout, so one red root-cause
        // finding fires instead of one nudge per prop.
        let sizes_cm: Vec<u32> = (0..52).map(|_| 40 * 100).collect();
        let finding = unit_scale_finding(&sizes_cm, 1_200).expect("x100 scene mixes units");
        assert_eq!(finding.severity, "red");
        assert_eq!(finding.root_cause, MIXED_UNITS_ROOT_CAUSE);
        assert_eq!(finding.median_cm, 4_000);
        assert_eq!(finding.extent_cm, 1_200);
        assert!(
            finding.ratio_permille >= MIXED_UNITS_PERMILLE,
            "x100 median sits above the band, got {}",
            finding.ratio_permille
        );
        // Stable across runs: pure, no state to drift.
        assert_eq!(Some(finding), unit_scale_finding(&sizes_cm, 1_200));
    }

    #[test]
    fn unit_scale_small_population_is_silent() {
        // Four props, however huge, are too weak a sample: no guess.
        let four = vec![9_999_u32; 4];
        assert_eq!(unit_scale_finding(&four, 1_200), None);
        // The floor is exact: 15 sizes stay silent, the 16th flips it on.
        let fifteen = vec![4_000_u32; 15];
        assert_eq!(unit_scale_finding(&fifteen, 1_200), None);
        let sixteen = vec![4_000_u32; 16];
        assert!(unit_scale_finding(&sixteen, 1_200).is_some());
        // An empty slice and a zero extent never guess.
        assert_eq!(unit_scale_finding(&[], 1_200), None);
        assert_eq!(unit_scale_finding(&sixteen, 0), None);
    }

    #[test]
    fn unit_scale_consistent_scene_is_silent() {
        // 52 normal 40 cm props in a 12 m room: the median sits far under the
        // extent, so a well-formed scene never flags.
        let sizes_cm: Vec<u32> = (0..52).map(|_| 40).collect();
        assert_eq!(unit_scale_finding(&sizes_cm, 1_200), None);
    }

    #[test]
    fn unit_scale_band_is_exact() {
        // A median exactly at the extent fires at the band; one centimetre
        // short stays silent. The boundary is the constant, not a guess.
        let at = vec![1_200_u32; 16];
        let finding = unit_scale_finding(&at, 1_200).expect("median == extent fires");
        assert_eq!(finding.ratio_permille, MIXED_UNITS_PERMILLE);
        let below = vec![1_199_u32; 16];
        assert_eq!(unit_scale_finding(&below, 1_200), None);
    }

    #[test]
    fn unit_scale_a_single_outlier_does_not_flag() {
        // Median, not mean: 15 normal props plus one huge prop still reads
        // well under the extent, so one bad asset never cries unit error.
        let mut sizes_cm = vec![40_u32; 15];
        sizes_cm.push(1_000_000);
        assert_eq!(unit_scale_finding(&sizes_cm, 1_200), None);
    }

    #[test]
    fn median_cm_is_lower_median_and_order_invariant() {
        assert_eq!(median_cm(&[]), None);
        assert_eq!(median_cm(&[7]), Some(7));
        assert_eq!(median_cm(&[3, 1, 2]), Some(2));
        // Even count: the lower median is the smaller middle value, one in the
        // list, never the mean of the two middles.
        assert_eq!(median_cm(&[1, 2, 3, 4]), Some(2));
        assert_eq!(median_cm(&[4, 3, 2, 1]), Some(2));
        // Order-invariant across a shuffle.
        let mut sizes_cm = vec![50_u32, 10, 40, 20, 30];
        assert_eq!(median_cm(&sizes_cm), Some(30));
        sizes_cm.reverse();
        assert_eq!(median_cm(&sizes_cm), Some(30));
    }
}
