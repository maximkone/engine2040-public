//! lod: one distance policy for a scene's detail levels.
//!
//! Owner: `engine-spatial`. Consumer: any scene report or renderer row that has
//! to answer "how much detail does this actor get at this distance" the same
//! way every frame; until a door verb names it the policy is wired in-test
//! below against its own bands and the static table a scene would ship.
//! Units: distances in integer centimetres, a level is a small integer with 0
//! the closest (most detailed) band.
//! Determinism: pure integer functions, no floats, no clock, no IO; the same
//! policy and the same distance always answer the same level.
//! Failure: a policy text refuses as a typed [`LodError`] that names the row or
//! the rule it broke; a query over a built policy never panics.
//! Budget: O(bands) per query over a handful of entries ([`MAX_LEVEL`] caps the
//! chain), never an allocation after the policy is built.

use std::fmt;

/// Coarsest level a band may name; level 0 is the nearest band, so a chain
/// holds at most eight bands.
pub const MAX_LEVEL: u8 = 7;

/// One band: an actor no farther than `max_distance_cm` draws at `level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodBand {
    /// Band edge in integer centimetres, measured from the camera.
    pub max_distance_cm: u32,
    /// Detail level inside the band; 0 is the closest, most detailed one.
    pub level: u8,
}

/// A scene's whole distance policy: near-to-far bands, the radius past which
/// nothing draws, and the hysteresis that keeps an actor at a band edge from
/// flickering between two levels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LodPolicy {
    bands: Vec<LodBand>,
    cull_distance_cm: u32,
    hysteresis_cm: u32,
}

/// Why a policy was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LodError {
    /// A row is not `band <level> <cm>`, `cull <cm>` or `hysteresis <cm>`.
    BadRow {
        /// 1-based line of the row.
        line: usize,
        /// What the row should have been.
        reason: &'static str,
    },
    /// A row names a key the policy does not know.
    UnknownKey {
        /// 1-based line of the row.
        line: usize,
        /// The unknown key.
        key: String,
    },
    /// A row's number does not parse, or is out of the type's range.
    BadNumber {
        /// 1-based line of the row.
        line: usize,
        /// The text that did not parse.
        value: String,
    },
    /// A policy with no band draws nothing it could ever show.
    NoBands,
    /// The first band is not level 0, so "closest" would have no meaning.
    FirstLevelNotZero {
        /// The level the first band named.
        level: u8,
    },
    /// A band repeats or lowers the level before it.
    LevelNotIncreasing {
        /// The level that did not grow.
        level: u8,
    },
    /// A band edge is at or before the edge before it.
    DistanceNotIncreasing {
        /// The edge before.
        from_cm: u32,
        /// The edge that did not grow.
        to_cm: u32,
    },
    /// A level past [`MAX_LEVEL`].
    LevelTooHigh {
        /// The level that was too high.
        level: u8,
    },
    /// The cull radius is not past the last band, so the farthest band could
    /// never be selected.
    CullNotBeyondLastBand {
        /// The last band edge.
        band_cm: u32,
        /// The cull radius.
        cull_cm: u32,
    },
    /// The hysteresis is as wide as the narrowest gap, so a kept level could
    /// reach past the band after it.
    HysteresisTooWide {
        /// The hysteresis asked for.
        hysteresis_cm: u32,
        /// The narrowest band gap.
        gap_cm: u32,
    },
    /// No `cull` row: a policy with no far limit draws the whole world.
    NoCull,
}

impl fmt::Display for LodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRow { line, reason } => write!(f, "line {line}: {reason}"),
            Self::UnknownKey { line, key } => write!(
                f,
                "line {line}: `{key}` is not a policy key; write band, cull or hysteresis"
            ),
            Self::BadNumber { line, value } => {
                write!(f, "line {line}: `{value}` is not a whole number")
            }
            Self::NoBands => write!(f, "no band row: a policy needs at least one band"),
            Self::FirstLevelNotZero { level } => {
                write!(
                    f,
                    "the first band is level {level}; level 0 is the closest band"
                )
            }
            Self::LevelNotIncreasing { level } => {
                write!(f, "level {level} does not grow on the band before it")
            }
            Self::DistanceNotIncreasing { from_cm, to_cm } => {
                write!(f, "band edge {to_cm} cm does not grow past {from_cm} cm")
            }
            Self::LevelTooHigh { level } => {
                write!(f, "level {level}; at most {MAX_LEVEL}")
            }
            Self::CullNotBeyondLastBand { band_cm, cull_cm } => write!(
                f,
                "cull {cull_cm} cm does not pass the last band edge {band_cm} cm"
            ),
            Self::HysteresisTooWide {
                hysteresis_cm,
                gap_cm,
            } => write!(
                f,
                "hysteresis {hysteresis_cm} cm reaches the narrowest band gap {gap_cm} cm"
            ),
            Self::NoCull => write!(f, "no cull row: name the distance past which nothing draws"),
        }
    }
}

impl std::error::Error for LodError {}

/// A whole number from one row, refused with the row's line.
fn number(line: usize, value: &str) -> Result<u32, LodError> {
    value.parse().map_err(|_| LodError::BadNumber {
        line,
        value: value.to_owned(),
    })
}

/// A level from one row, refused with the row's line.
fn level(line: usize, value: &str) -> Result<u8, LodError> {
    value.parse().map_err(|_| LodError::BadNumber {
        line,
        value: value.to_owned(),
    })
}

impl LodPolicy {
    /// Build a policy from bands, a cull radius and a hysteresis, checking
    /// every rule a scene depends on.
    pub fn new(
        bands: Vec<LodBand>,
        cull_distance_cm: u32,
        hysteresis_cm: u32,
    ) -> Result<Self, LodError> {
        if bands.is_empty() {
            return Err(LodError::NoBands);
        }
        let mut previous: Option<LodBand> = None;
        for band in &bands {
            if band.level > MAX_LEVEL {
                return Err(LodError::LevelTooHigh { level: band.level });
            }
            match previous {
                None if band.level != 0 => {
                    return Err(LodError::FirstLevelNotZero { level: band.level });
                }
                None => {}
                Some(previous) => {
                    if band.level <= previous.level {
                        return Err(LodError::LevelNotIncreasing { level: band.level });
                    }
                    if band.max_distance_cm <= previous.max_distance_cm {
                        return Err(LodError::DistanceNotIncreasing {
                            from_cm: previous.max_distance_cm,
                            to_cm: band.max_distance_cm,
                        });
                    }
                }
            }
            previous = Some(*band);
        }
        let last = bands.last().copied().ok_or(LodError::NoBands)?;
        if cull_distance_cm <= last.max_distance_cm {
            return Err(LodError::CullNotBeyondLastBand {
                band_cm: last.max_distance_cm,
                cull_cm: cull_distance_cm,
            });
        }
        let mut narrowest = cull_distance_cm - last.max_distance_cm;
        for pair in bands.windows(2) {
            if let [near, far] = pair {
                narrowest = narrowest.min(far.max_distance_cm - near.max_distance_cm);
            }
        }
        if hysteresis_cm >= narrowest {
            return Err(LodError::HysteresisTooWide {
                hysteresis_cm,
                gap_cm: narrowest,
            });
        }
        Ok(Self {
            bands,
            cull_distance_cm,
            hysteresis_cm,
        })
    }

    /// Parse a policy text: `band <level> <cm>`, `cull <cm>` and an optional
    /// `hysteresis <cm>`, one per line, `#` starting a comment.
    pub fn parse(text: &str) -> Result<Self, LodError> {
        let mut bands: Vec<LodBand> = Vec::new();
        let mut cull_distance_cm: Option<u32> = None;
        let mut hysteresis_cm = 0;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = raw.split('#').next().unwrap_or(raw).trim();
            if body.is_empty() {
                continue;
            }
            let mut words = body.split_whitespace();
            let Some(key) = words.next() else {
                continue;
            };
            match key {
                "band" => {
                    let band_level = words.next().ok_or(LodError::BadRow {
                        line,
                        reason: "band needs a level and a distance",
                    })?;
                    let distance = words.next().ok_or(LodError::BadRow {
                        line,
                        reason: "band needs a level and a distance",
                    })?;
                    if words.next().is_some() {
                        return Err(LodError::BadRow {
                            line,
                            reason: "band takes a level and one distance, nothing else",
                        });
                    }
                    bands.push(LodBand {
                        max_distance_cm: number(line, distance)?,
                        level: level(line, band_level)?,
                    });
                }
                "cull" => {
                    let distance = words.next().ok_or(LodError::BadRow {
                        line,
                        reason: "cull needs one distance",
                    })?;
                    if words.next().is_some() {
                        return Err(LodError::BadRow {
                            line,
                            reason: "cull takes one distance, nothing else",
                        });
                    }
                    cull_distance_cm = Some(number(line, distance)?);
                }
                "hysteresis" => {
                    let distance = words.next().ok_or(LodError::BadRow {
                        line,
                        reason: "hysteresis needs one distance",
                    })?;
                    if words.next().is_some() {
                        return Err(LodError::BadRow {
                            line,
                            reason: "hysteresis takes one distance, nothing else",
                        });
                    }
                    hysteresis_cm = number(line, distance)?;
                }
                other => {
                    return Err(LodError::UnknownKey {
                        line,
                        key: other.to_owned(),
                    });
                }
            }
        }
        let cull_distance_cm = cull_distance_cm.ok_or(LodError::NoCull)?;
        Self::new(bands, cull_distance_cm, hysteresis_cm)
    }

    /// The bands, nearest first.
    #[must_use]
    pub fn bands(&self) -> &[LodBand] {
        &self.bands
    }

    /// The distance past which nothing draws, in integer centimetres.
    #[must_use]
    pub fn cull_distance_cm(&self) -> u32 {
        self.cull_distance_cm
    }

    /// The hysteresis a kept level may linger, in integer centimetres.
    #[must_use]
    pub fn hysteresis_cm(&self) -> u32 {
        self.hysteresis_cm
    }

    /// The level at one distance, or `None` past the cull radius. A distance
    /// exactly on a band edge belongs to the nearer band.
    #[must_use]
    pub fn level_at(&self, distance_cm: u32) -> Option<u8> {
        if distance_cm >= self.cull_distance_cm {
            return None;
        }
        self.bands
            .iter()
            .find(|band| distance_cm <= band.max_distance_cm)
            .or_else(|| self.bands.last())
            .map(|band| band.level)
    }

    /// The level at one distance, keeping `previous` while the hysteresis
    /// allows it, so a camera sliding along a band edge does not flip levels.
    /// Culling is immediate: past the radius the answer is `None` whatever the
    /// previous level. A nearer band is taken at once.
    #[must_use]
    pub fn level_kept(&self, distance_cm: u32, previous: Option<u8>) -> Option<u8> {
        let fresh = self.level_at(distance_cm);
        let Some(kept) = previous else {
            return fresh;
        };
        let fresh_level = fresh?;
        // Only a coarser level may linger; a nearer band is taken at once.
        if fresh_level <= kept {
            return fresh;
        }
        let Some(band) = self.bands.iter().find(|band| band.level == kept) else {
            return fresh;
        };
        if distance_cm <= band.max_distance_cm.saturating_add(self.hysteresis_cm) {
            return Some(kept);
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = "\
# three bands and a far limit
band 0 2500
band 1 6000   # mid
band 2 9000
cull 12000
hysteresis 250
";

    #[test]
    fn parse_reads_bands_cull_and_hysteresis() {
        let policy = LodPolicy::parse(SCENE).expect("the scene policy parses");
        assert_eq!(
            policy.bands(),
            &[
                LodBand {
                    max_distance_cm: 2500,
                    level: 0,
                },
                LodBand {
                    max_distance_cm: 6000,
                    level: 1,
                },
                LodBand {
                    max_distance_cm: 9000,
                    level: 2,
                },
            ][..]
        );
        assert_eq!(policy.cull_distance_cm(), 12_000);
        assert_eq!(policy.hysteresis_cm(), 250);
        // The hysteresis is optional and defaults to none.
        let bare = LodPolicy::parse("band 0 1000\ncull 2000\n").expect("parses");
        assert_eq!(bare.hysteresis_cm(), 0);
        // A policy is data: the same text builds the same policy twice.
        assert_eq!(LodPolicy::parse(SCENE), Ok(policy));
    }

    #[test]
    fn level_switches_on_the_band_edge_and_culls_past_the_radius() {
        let policy = LodPolicy::parse(SCENE).expect("parses");
        let cases: [(u32, Option<u8>); 9] = [
            (0, Some(0)),
            (2500, Some(0)),
            (2501, Some(1)),
            (6000, Some(1)),
            (6001, Some(2)),
            (9000, Some(2)),
            (11_999, Some(2)),
            (12_000, None),
            (40_000, None),
        ];
        for (distance_cm, expected) in cases {
            assert_eq!(policy.level_at(distance_cm), expected, "{distance_cm} cm");
        }
    }

    #[test]
    fn hysteresis_keeps_a_level_until_the_band_gap_and_never_culls_late() {
        let policy = LodPolicy::parse(SCENE).expect("parses");
        // Inside the hysteresis the old level stays; past it the fresh one wins.
        assert_eq!(policy.level_kept(2600, Some(0)), Some(0));
        assert_eq!(policy.level_kept(2750, Some(0)), Some(0));
        assert_eq!(policy.level_kept(2751, Some(0)), Some(1));
        // A nearer band is taken at once: detail never waits for hysteresis.
        assert_eq!(policy.level_kept(300, Some(2)), Some(0));
        // Culling is immediate.
        assert_eq!(policy.level_kept(12_500, Some(2)), None);
        // With no previous level the hysteresis has nothing to keep.
        assert_eq!(policy.level_kept(2600, None), Some(1));
    }

    #[test]
    fn policy_refusals_name_the_row_or_the_rule() {
        let cases: [(&str, LodError); 9] = [
            ("cull 2000\n", LodError::NoBands),
            ("band 0 1000\n", LodError::NoCull),
            (
                "band 0 1000\ncull 2000\ndistance 4\n",
                LodError::UnknownKey {
                    line: 3,
                    key: "distance".to_owned(),
                },
            ),
            (
                "band 0 far\ncull 2000\n",
                LodError::BadNumber {
                    line: 1,
                    value: "far".to_owned(),
                },
            ),
            (
                "band 1 1000\ncull 2000\n",
                LodError::FirstLevelNotZero { level: 1 },
            ),
            (
                "band 0 1000\nband 0 2000\ncull 3000\n",
                LodError::LevelNotIncreasing { level: 0 },
            ),
            (
                "band 0 2000\nband 1 2000\ncull 3000\n",
                LodError::DistanceNotIncreasing {
                    from_cm: 2000,
                    to_cm: 2000,
                },
            ),
            (
                "band 0 1000\ncull 1000\n",
                LodError::CullNotBeyondLastBand {
                    band_cm: 1000,
                    cull_cm: 1000,
                },
            ),
            (
                "band 0 1000\nband 1 2000\ncull 3000\nhysteresis 1000\n",
                LodError::HysteresisTooWide {
                    hysteresis_cm: 1000,
                    gap_cm: 1000,
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(LodPolicy::parse(text), Err(expected), "{text}");
        }
        // A band edge that does not grow is refused as a distance rule.
        assert_eq!(
            LodPolicy::parse("band 0 1000\nband 1 900\ncull 2000\n"),
            Err(LodError::DistanceNotIncreasing {
                from_cm: 1000,
                to_cm: 900,
            })
        );
        // A level past the cap is refused by the level.
        assert_eq!(
            LodPolicy::parse("band 8 1000\ncull 2000\n"),
            Err(LodError::LevelTooHigh { level: 8 })
        );
        // Every refusal says what to write instead.
        for error in [
            LodError::NoBands,
            LodError::NoCull,
            LodError::BadRow {
                line: 2,
                reason: "cull needs one distance",
            },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
