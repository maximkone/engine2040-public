//! curves: validated versioned replacement AI curves.
//!
//! Consumer: `arena` clones an [`AiConfig`] snapshot at match build and hands
//! one configured [`Brain`](crate::gladiator::Brain) per fighter; the live
//! tuning panel and running-game reload build new snapshots the
//! same way. Data owner: [`AiConfig`] owns its curve tables plus the default
//! morale; match state stays in the arena. Units: morale `0..=1000`, biases
//! `0..=1000` around a neutral 500, `engage_range_cm` in centimetres.
//! Determinism scope: same validated config plus same [`Percept`] sequence
//! gives byte-identical [`FighterInput`]s (integer math only). Errors:
//! [`validate_curves`] rejects bad tables, [`AiConfig::try_new`] rejects bad
//! configs; [`Brain`](crate::gladiator::Brain) construction falls back to
//! scripted defaults instead of panicking. Configuration flows one way into
//! the arena at match build: no match writes curves back.
//!
//! [`Percept`]: crate::gladiator::Percept
//! [`FighterInput`]: engine_input::FighterInput

/// Curve table lanes: morale `0..=1000` maps to lane `morale / 125` (capped).
pub const CURVE_LANES: usize = 8;
/// Neutral bias: table entries equal to this shift no score.
pub const BIAS_NEUTRAL: u16 = 500;
/// Largest legal table entry (biases are `0..=1000`).
pub const BIAS_MAX: u16 = 1_000;
/// Smallest sane engage range (50 cm: inside striking distance).
pub const ENGAGE_RANGE_MIN_CM: u32 = 50;
/// Largest sane engage range (60 m: the sanitize clamp for separations).
pub const ENGAGE_RANGE_MAX_CM: u32 = 6_000;
/// Scripted engage range: 250 cm, the legacy strike threshold in millimetres.
pub const DEFAULT_ENGAGE_RANGE_CM: u32 = 250;
/// Scripted curve-set version. Tapes pin the version they ran with.
pub const DEFAULT_CURVES_VERSION: u32 = 1;
/// Scripted default morale (neutral).
pub const DEFAULT_MORALE: i32 = 500;
/// Lowest legal morale default.
pub const MORALE_MIN: i32 = 0;
/// Highest legal morale default.
pub const MORALE_MAX: i32 = 1_000;
/// Scripted aggression lane: stepwise `morale - 500` (exact at neutral and
/// at both extremes, within half a lane elsewhere).
pub const DEFAULT_AGGRESSION: [u16; CURVE_LANES] = [0, 125, 250, 375, 500, 625, 750, 1_000];
/// Scripted caution lane: flat neutral, so morale shifts defense through the
/// legacy aggression coupling only.
pub const DEFAULT_CAUTION: [u16; CURVE_LANES] = [BIAS_NEUTRAL; CURVE_LANES];

/// ASCII digits to `u32`; `None` on empty, non-digit or overflow.
///
/// One scanner feeds every designer-rows parser: `depth` tier rows and
/// `curves_text` ai rows share this core and keep their own error type.
pub(crate) fn parse_dec_u32(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    let mut acc: u32 = 0;
    for b in text.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        let digit = u32::from(b - b'0');
        acc = acc.checked_mul(10)?.checked_add(digit)?;
    }
    Some(acc)
}

/// Versioned replacement AI curves: morale-to-bias tables plus the range at
/// which the fighter commits to a strike. Lane `i` covers morale
/// `i * 125..=(i * 125 + 124)` (lane 7 extends to 1000). `aggression` must be
/// non-decreasing with morale; `caution` is free-form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiCurves {
    /// Curve-set version; `0` is never valid (it means "unversioned").
    pub version: u32,
    /// Morale-to-attack-bias table, `0..=1000` per lane, non-decreasing.
    pub aggression: [u16; CURVE_LANES],
    /// Morale-to-caution table, `0..=1000` per lane, any shape.
    pub caution: [u16; CURVE_LANES],
    /// Strike-commit range in centimetres, `50..=6000`.
    pub engage_range_cm: u32,
}

impl AiCurves {
    /// Scripted defaults: the scripted-brain policy as a validated curve set.
    pub const fn default_curves() -> Self {
        Self {
            version: DEFAULT_CURVES_VERSION,
            aggression: DEFAULT_AGGRESSION,
            caution: DEFAULT_CAUTION,
            engage_range_cm: DEFAULT_ENGAGE_RANGE_CM,
        }
    }

    /// Morale bucket for a morale value: clamped `morale / 125`, capped at 7.
    /// Total over all `i32` inputs, including out-of-range morale.
    pub const fn morale_bucket(morale: i32) -> usize {
        let clamped = if morale < MORALE_MIN {
            MORALE_MIN
        } else if morale > MORALE_MAX {
            MORALE_MAX
        } else {
            morale
        };
        let bucket = (clamped as u32) / 125;
        if bucket > 7 {
            7
        } else {
            bucket as usize
        }
    }

    /// Attack bias at `morale`: table entry minus neutral, `-500..=500`.
    /// Defensive clamp keeps scoring total even for unvalidated tables.
    pub fn aggression_bias_at(&self, morale: i32) -> i32 {
        let lane = Self::morale_bucket(morale);
        i32::from(self.aggression[lane].min(BIAS_MAX)) - i32::from(BIAS_NEUTRAL)
    }

    /// Caution bias at `morale`: table entry minus neutral, `-500..=500`.
    /// Defensive clamp keeps scoring total even for unvalidated tables.
    pub fn caution_bias_at(&self, morale: i32) -> i32 {
        let lane = Self::morale_bucket(morale);
        i32::from(self.caution[lane].min(BIAS_MAX)) - i32::from(BIAS_NEUTRAL)
    }

    /// Strike-commit range in millimetres. Defensive clamp keeps scoring
    /// total even for unvalidated tables.
    pub fn engage_range_mm(&self) -> i32 {
        (self
            .engage_range_cm
            .clamp(ENGAGE_RANGE_MIN_CM, ENGAGE_RANGE_MAX_CM) as i32)
            * 10
    }
}

/// Curve validation failures. Never panics, never blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiCurveError {
    /// `version == 0` (unversioned tables never ship).
    ZeroVersion,
    /// An aggression lane lies outside `0..=1000`.
    AggressionOutOfRange {
        /// Lane index holding the bad value.
        index: usize,
        /// The rejected value.
        value: u16,
    },
    /// A caution lane lies outside `0..=1000`.
    CautionOutOfRange {
        /// Lane index holding the bad value.
        index: usize,
        /// The rejected value.
        value: u16,
    },
    /// Aggression decreases with morale (lane `index` below its predecessor).
    AggressionNotMonotonic {
        /// Lane index that dropped (`aggression[index] < aggression[index-1]`).
        index: usize,
        /// Value at `index - 1`.
        prev: u16,
        /// Value at `index`.
        next: u16,
    },
    /// Engage range lies outside `50..=6000` cm.
    EngageRangeOutOfRange {
        /// The rejected range.
        value: u32,
    },
    /// Default morale lies outside `0..=1000`.
    BadMoraleDefault {
        /// The rejected morale.
        value: i32,
    },
}

impl core::fmt::Display for AiCurveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AiCurveError::ZeroVersion => write!(f, "ai curves: version 0 is unversioned"),
            AiCurveError::AggressionOutOfRange { index, value } => write!(
                f,
                "ai curves: aggression lane {index} value {value} outside 0..=1000"
            ),
            AiCurveError::CautionOutOfRange { index, value } => write!(
                f,
                "ai curves: caution lane {index} value {value} outside 0..=1000"
            ),
            AiCurveError::AggressionNotMonotonic { index, prev, next } => write!(
                f,
                "ai curves: aggression drops at lane {index} ({prev} -> {next})"
            ),
            AiCurveError::EngageRangeOutOfRange { value } => {
                write!(f, "ai curves: engage range {value} cm outside 50..=6000")
            }
            AiCurveError::BadMoraleDefault { value } => {
                write!(f, "ai curves: morale default {value} outside 0..=1000")
            }
        }
    }
}

impl std::error::Error for AiCurveError {}

/// Validate one replacement curve set: nonzero version, lanes in
/// `0..=1000`, non-decreasing aggression, sane engage range. Returns the
/// first failure; valid sets return `Ok(())`.
pub fn validate_curves(set: &AiCurves) -> Result<(), AiCurveError> {
    if set.version == 0 {
        return Err(AiCurveError::ZeroVersion);
    }
    for (index, value) in set.aggression.iter().enumerate() {
        if *value > BIAS_MAX {
            return Err(AiCurveError::AggressionOutOfRange {
                index,
                value: *value,
            });
        }
    }
    for (index, value) in set.caution.iter().enumerate() {
        if *value > BIAS_MAX {
            return Err(AiCurveError::CautionOutOfRange {
                index,
                value: *value,
            });
        }
    }
    for index in 1..CURVE_LANES {
        if set.aggression[index] < set.aggression[index - 1] {
            return Err(AiCurveError::AggressionNotMonotonic {
                index,
                prev: set.aggression[index - 1],
                next: set.aggression[index],
            });
        }
    }
    if set.engage_range_cm < ENGAGE_RANGE_MIN_CM || set.engage_range_cm > ENGAGE_RANGE_MAX_CM {
        return Err(AiCurveError::EngageRangeOutOfRange {
            value: set.engage_range_cm,
        });
    }
    Ok(())
}

/// Validate a full config: curves plus the `0..=1000` morale default.
pub fn validate_config(config: &AiConfig) -> Result<(), AiCurveError> {
    validate_curves(&config.curves)?;
    if config.morale_default < MORALE_MIN || config.morale_default > MORALE_MAX {
        return Err(AiCurveError::BadMoraleDefault {
            value: config.morale_default,
        });
    }
    Ok(())
}

/// Immutable AI configuration snapshot: curve tables plus the morale a new
/// brain starts at. Cloned into the arena at match build; matches never
/// write it back. Invalid snapshots never ship: [`AiConfig::new`] and the
/// brain constructor fall back to [`AiConfig::scripted_defaults`], while
/// [`AiConfig::try_new`] reports the cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiConfig {
    /// Curve tables (validated on construction paths).
    pub curves: AiCurves,
    /// Starting morale, `0..=1000`.
    pub morale_default: i32,
}

impl AiConfig {
    /// Scripted defaults: scripted-brain policy, neutral morale.
    pub const fn scripted_defaults() -> Self {
        Self {
            curves: AiCurves::default_curves(),
            morale_default: DEFAULT_MORALE,
        }
    }

    /// Validated-or-default construction: invalid curves or morale fall back
    /// to scripted defaults. Never panics.
    pub fn new(curves: AiCurves, morale_default: i32) -> Self {
        let candidate = Self {
            curves,
            morale_default,
        };
        match validate_config(&candidate) {
            Ok(()) => candidate,
            Err(_) => Self::scripted_defaults(),
        }
    }

    /// Strict construction: the invalid cause is returned, nothing falls back.
    pub fn try_new(curves: AiCurves, morale_default: i32) -> Result<Self, AiCurveError> {
        let candidate = Self {
            curves,
            morale_default,
        };
        validate_config(&candidate)?;
        Ok(candidate)
    }

    /// True when [`validate_config`] accepts this snapshot.
    pub fn is_valid(&self) -> bool {
        validate_config(self).is_ok()
    }

    /// This snapshot when valid, scripted defaults otherwise. Never panics.
    pub fn effective(&self) -> Self {
        if self.is_valid() {
            *self
        } else {
            Self::scripted_defaults()
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self::scripted_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_defaults_validate() {
        let curves = AiCurves::default_curves();
        validate_curves(&curves).expect("scripted defaults validate");
        let config = AiConfig::scripted_defaults();
        assert!(config.is_valid());
        validate_config(&config).expect("default config validates");
        assert_eq!(config, AiConfig::default());
        assert_eq!(config.morale_default, DEFAULT_MORALE);
    }

    #[test]
    fn shared_dec_scanner_reads_digits() {
        // One ASCII-digits scanner feeds every designer-rows parser; tier
        // rows and ai rows refuse the same shapes with their own error.
        assert_eq!(parse_dec_u32("0"), Some(0));
        assert_eq!(parse_dec_u32("007"), Some(7));
        assert_eq!(parse_dec_u32("4294967295"), Some(u32::MAX));
        assert_eq!(parse_dec_u32(""), None, "empty refuses");
        assert_eq!(parse_dec_u32("4294967296"), None, "overflow refuses");
        assert_eq!(parse_dec_u32("12a"), None, "trailing letter refuses");
        assert_eq!(parse_dec_u32("+12"), None, "sign refuses");
        assert_eq!(parse_dec_u32("1.0"), None, "point refuses");
        assert_eq!(parse_dec_u32(" 12"), None, "space refuses");
    }

    #[test]
    fn zero_version_rejected() {
        let mut curves = AiCurves::default_curves();
        curves.version = 0;
        assert_eq!(validate_curves(&curves), Err(AiCurveError::ZeroVersion));
        assert_eq!(
            AiConfig::new(curves, DEFAULT_MORALE),
            AiConfig::scripted_defaults(),
            "version 0 falls back"
        );
    }

    #[test]
    fn aggression_range_rejected_per_lane() {
        for lane in 0..CURVE_LANES {
            let mut curves = AiCurves::default_curves();
            curves.aggression[lane] = BIAS_MAX + 1;
            assert_eq!(
                validate_curves(&curves),
                Err(AiCurveError::AggressionOutOfRange {
                    index: lane,
                    value: BIAS_MAX + 1,
                }),
                "lane {lane}"
            );
        }
    }

    #[test]
    fn caution_range_rejected_per_lane() {
        for lane in 0..CURVE_LANES {
            let mut curves = AiCurves::default_curves();
            curves.caution[lane] = BIAS_MAX + 1;
            assert_eq!(
                validate_curves(&curves),
                Err(AiCurveError::CautionOutOfRange {
                    index: lane,
                    value: BIAS_MAX + 1,
                }),
                "lane {lane}"
            );
        }
    }

    #[test]
    fn aggression_must_not_decrease() {
        // Flat tables are monotonic; a single drop names its lane.
        let flat = AiCurves {
            version: 2,
            aggression: [400; CURVE_LANES],
            caution: [500; CURVE_LANES],
            engage_range_cm: DEFAULT_ENGAGE_RANGE_CM,
        };
        validate_curves(&flat).expect("flat aggression is monotonic");
        let mut dipped = flat;
        dipped.aggression[5] = 399;
        assert_eq!(
            validate_curves(&dipped),
            Err(AiCurveError::AggressionNotMonotonic {
                index: 5,
                prev: 400,
                next: 399,
            })
        );
    }

    #[test]
    fn engage_range_bounds_rejected() {
        for bad in [
            0,
            ENGAGE_RANGE_MIN_CM - 1,
            ENGAGE_RANGE_MAX_CM + 1,
            u32::MAX,
        ] {
            let mut curves = AiCurves::default_curves();
            curves.engage_range_cm = bad;
            assert_eq!(
                validate_curves(&curves),
                Err(AiCurveError::EngageRangeOutOfRange { value: bad }),
                "range {bad}"
            );
        }
        for good in [
            ENGAGE_RANGE_MIN_CM,
            DEFAULT_ENGAGE_RANGE_CM,
            ENGAGE_RANGE_MAX_CM,
        ] {
            let mut curves = AiCurves::default_curves();
            curves.engage_range_cm = good;
            validate_curves(&curves).expect("range bound validates");
        }
    }

    #[test]
    fn bad_morale_default_rejected_and_falls_back() {
        for bad in [MORALE_MIN - 1, MORALE_MAX + 1, i32::MIN, i32::MAX] {
            let config = AiConfig {
                curves: AiCurves::default_curves(),
                morale_default: bad,
            };
            assert_eq!(
                validate_config(&config),
                Err(AiCurveError::BadMoraleDefault { value: bad })
            );
            assert_eq!(
                AiConfig::try_new(AiCurves::default_curves(), bad),
                Err(AiCurveError::BadMoraleDefault { value: bad })
            );
            assert_eq!(
                AiConfig::new(AiCurves::default_curves(), bad),
                AiConfig::scripted_defaults(),
                "morale {bad} falls back"
            );
        }
    }

    #[test]
    fn morale_bucket_goldens() {
        assert_eq!(AiCurves::morale_bucket(MORALE_MIN - 1), 0);
        assert_eq!(AiCurves::morale_bucket(0), 0);
        assert_eq!(AiCurves::morale_bucket(124), 0);
        assert_eq!(AiCurves::morale_bucket(125), 1);
        assert_eq!(AiCurves::morale_bucket(500), 4);
        assert_eq!(AiCurves::morale_bucket(874), 6);
        assert_eq!(AiCurves::morale_bucket(875), 7);
        assert_eq!(AiCurves::morale_bucket(1_000), 7);
        assert_eq!(AiCurves::morale_bucket(MORALE_MAX + 1), 7);
    }

    #[test]
    fn bias_helpers_read_their_lane() {
        let curves = AiCurves::default_curves();
        assert_eq!(curves.aggression_bias_at(0), -500);
        assert_eq!(curves.aggression_bias_at(500), 0);
        assert_eq!(curves.aggression_bias_at(1_000), 500);
        assert_eq!(curves.caution_bias_at(0), 0);
        assert_eq!(curves.engage_range_mm(), 2_500);
        // Unvalidated tables stay total: out-of-range entries clamp.
        let wild = AiCurves {
            version: 0,
            aggression: [u16::MAX; CURVE_LANES],
            caution: [u16::MAX; CURVE_LANES],
            engage_range_cm: u32::MAX,
        };
        assert_eq!(wild.aggression_bias_at(500), 500);
        assert_eq!(wild.caution_bias_at(500), 500);
        assert_eq!(wild.engage_range_mm(), 60_000);
    }
}
