//! motion_style: style and terrain dials for the motion-matching search (NS7).
//!
//! Consumer: arena locomotion asks for a style (sneak, proud, ...) or a grade
//! when it picks a clip; [`search_styled`] answers with the nearest clip of
//! that style and grade. Data owner: `engine-anim`; the tags come from the LOC2
//! bake ([`LocoSet`] keeps each clip's tag list), the search is the LM1
//! integer scan of [`crate::motion_match`]. Tags: `style_<name>` names a style
//! (the caller lists the names it knows), `slope_p10` and `slope_n10` mean a
//! plus or minus 10 percent grade; a clip without them is neutral and flat.
//! Units: percent grade, integer cost units (squared Q10 meters, see
//! [`crate::motion_match::cost_units`]). Determinism: integers only, ascending
//! index order and strict `<` replacement, so equal inputs pick the same index
//! twice; with [`Dial::OFF`] the pick and its cost equal
//! [`crate::motion_match::search`] exactly. Errors: [`StyleError`] at table
//! build, [`MatchError`] at search. Budget: one scan over the DB, no heap.

use super::loco_bake::LocoSet;
use super::motion_match::{
    search_penalized, MatchError, MatchResult, MotionDb, Query, SearchReceipt, PENALTY_MAX,
};

/// Tag prefix that names a style: `style_sneak`.
pub const STYLE_PREFIX: &str = "style_";
/// Tag prefix that gives a grade: `slope_p10` is +10 percent, `slope_n10` -10.
pub const SLOPE_PREFIX: &str = "slope_";
/// Steepest grade a tag or a dial may name, in percent.
pub const MAX_SLOPE_PCT: i16 = 100;
/// Most styles one table names; the neutral style is 0.
pub const MAX_STYLES: usize = 254;

/// Why a style table could not be built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StyleError {
    /// A `style_` tag names a style the caller did not list; the tag is carried.
    UnknownStyle(String),
    /// A `slope_` tag is not `p` or `n` plus whole percent within
    /// [`MAX_SLOPE_PCT`]; the tag is carried.
    BadSlope(String),
    /// A clip carries two style tags or two slope tags; its name is carried.
    Ambiguous(&'static str),
    /// More styles were listed than [`MAX_STYLES`].
    TooManyStyles,
}

impl core::fmt::Display for StyleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StyleError::UnknownStyle(tag) => write!(f, "style: unknown style tag {tag}"),
            StyleError::BadSlope(tag) => write!(f, "style: bad slope tag {tag}"),
            StyleError::Ambiguous(clip) => {
                write!(f, "style: clip {clip} has two style or slope tags")
            }
            StyleError::TooManyStyles => write!(f, "style: more than {MAX_STYLES} styles"),
        }
    }
}

impl std::error::Error for StyleError {}

/// What the bake says a clip is: its style (0 neutral) and its grade.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ClipMeta {
    /// Style index: 0 neutral, else 1 plus the position in the caller's list.
    pub style: u8,
    /// Grade in percent, positive uphill.
    pub slope_pct: i16,
}

/// The style and grade of every clip of one bake, by clip id.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StyleTable {
    metas: Vec<ClipMeta>,
}

/// A grade tag's percent: `p10` is 10, `n10` is -10.
fn parse_slope(rest: &str) -> Option<i16> {
    let (sign, digits) = match rest.as_bytes().first()? {
        b'p' => (1, &rest[1..]),
        b'n' => (-1, &rest[1..]),
        _ => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let pct: i16 = digits.parse().ok()?;
    (pct <= MAX_SLOPE_PCT).then_some(sign * pct)
}

impl StyleTable {
    /// Reads every clip's tags. `styles` lists the style names the caller knows;
    /// the first is style 1. Cold path: once per bake.
    pub fn from_set(set: &LocoSet, styles: &[&str]) -> Result<Self, StyleError> {
        if styles.len() > MAX_STYLES {
            return Err(StyleError::TooManyStyles);
        }
        let mut metas = Vec::with_capacity(set.clips.len());
        for clip in &set.clips {
            let mut meta = ClipMeta::default();
            let (mut styled, mut sloped) = (false, false);
            for tag in clip.tags {
                if let Some(name) = tag.strip_prefix(STYLE_PREFIX) {
                    let at = styles
                        .iter()
                        .position(|s| *s == name)
                        .ok_or_else(|| StyleError::UnknownStyle((*tag).to_string()))?;
                    if std::mem::replace(&mut styled, true) {
                        return Err(StyleError::Ambiguous(clip.name));
                    }
                    meta.style = u8::try_from(at + 1).map_err(|_| StyleError::TooManyStyles)?;
                } else if let Some(rest) = tag.strip_prefix(SLOPE_PREFIX) {
                    let pct = parse_slope(rest)
                        .ok_or_else(|| StyleError::BadSlope((*tag).to_string()))?;
                    if std::mem::replace(&mut sloped, true) {
                        return Err(StyleError::Ambiguous(clip.name));
                    }
                    meta.slope_pct = pct;
                }
            }
            metas.push(meta);
        }
        Ok(Self { metas })
    }

    /// The meta of `clip`; a clip the table does not know is neutral and flat.
    #[must_use]
    pub fn meta(&self, clip: u16) -> ClipMeta {
        self.metas
            .get(usize::from(clip))
            .copied()
            .unwrap_or_default()
    }
}

/// What the caller wants on top of the pose and trajectory: a style and a
/// grade, each with the cost of missing it. Costs are in the squared cost
/// units of the search, so a style miss can be traded against a pose miss.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dial {
    /// Wanted style (0 neutral).
    pub style: u8,
    /// Cost of a clip of another style.
    pub style_cost: i64,
    /// Wanted grade in percent.
    pub slope_pct: i16,
    /// Cost per squared percent of grade missed.
    pub slope_cost: i64,
}

impl Dial {
    /// No preference: the styled search equals the plain one.
    pub const OFF: Self = Self {
        style: 0,
        style_cost: 0,
        slope_pct: 0,
        slope_cost: 0,
    };

    fn validate(&self) -> Result<(), MatchError> {
        let costs = 0..=PENALTY_MAX;
        if !costs.contains(&self.style_cost) || !costs.contains(&self.slope_cost) {
            return Err(MatchError::BadQuery("dial cost"));
        }
        if self.slope_pct.abs() > MAX_SLOPE_PCT {
            return Err(MatchError::BadQuery("dial slope"));
        }
        Ok(())
    }

    /// What `meta` costs against this dial, capped below [`PENALTY_MAX`] so a
    /// penalty can never hide the first entry.
    fn penalty(&self, meta: ClipMeta) -> i64 {
        let style = if meta.style == self.style {
            0
        } else {
            self.style_cost
        };
        let miss = i64::from((meta.slope_pct - self.slope_pct).abs());
        style
            .saturating_add(miss.saturating_mul(miss).saturating_mul(self.slope_cost))
            .min(PENALTY_MAX)
    }
}

/// [`crate::motion_match::search`] with the dial's penalty added to every
/// entry by its clip's style and grade: the same scan, bound and ties, so
/// [`Dial::OFF`] picks what the plain search picks.
pub fn search_styled(
    db: &MotionDb,
    table: &StyleTable,
    query: &Query,
    dial: &Dial,
) -> Result<(MatchResult, SearchReceipt), MatchError> {
    dial.validate()?;
    search_penalized(db, query, |entry| dial.penalty(table.meta(entry.clip)))
}

#[cfg(test)]
mod tests {
    use super::super::loco_bake::{bake_loco_set, ClipSpec};
    use super::super::motion_match::{cost_units, default_weights, search, FEAT_N, NO_HIST};
    use super::*;

    const STYLES: [&str; 2] = ["sneak", "proud"];
    const SLOPES: [i16; 3] = [-10, 0, 10];
    /// Gaits of the grid: name, hip speed (m/s), foot swing (m).
    const GAITS: [(&str, f32, f32); 2] = [("walk", 1.5, 0.25), ("run", 3.5, 0.5)];
    /// Variants of one gait: every style at every grade.
    const VARIANTS: usize = 3 * SLOPES.len();

    fn leak<T: ?Sized>(value: Box<T>) -> &'static T {
        Box::leak(value)
    }

    /// One clip per gait x style x grade: all variants of a gait share their
    /// pose and trajectory, so only the dial can tell them apart.
    fn grid() -> Vec<ClipSpec> {
        let mut specs = Vec::new();
        for (gait, speed, swing) in GAITS {
            for style in [None, Some(STYLES[0]), Some(STYLES[1])] {
                for slope in SLOPES {
                    let mut tags = vec![gait.to_string()];
                    tags.extend(style.map(|s| format!("{STYLE_PREFIX}{s}")));
                    match slope {
                        0 => {}
                        pct => tags.push(format!(
                            "{SLOPE_PREFIX}{}{}",
                            if pct > 0 { 'p' } else { 'n' },
                            pct.abs()
                        )),
                    }
                    let name =
                        format!("{gait}_{}_{slope}", style.unwrap_or("plain")).replace('-', "n");
                    let tags: Vec<&'static str> =
                        tags.into_iter().map(|t| leak(t.into_boxed_str())).collect();
                    specs.push(ClipSpec {
                        name: leak(name.into_boxed_str()),
                        source: "quaternius",
                        license: "CC0-1.0",
                        tags: leak(tags.into_boxed_slice()),
                        frames: 60,
                        speed0: speed,
                        speed1: speed,
                        swing,
                        lateral: false,
                        looped: true,
                        root_motion: false,
                        synthetic: true,
                    });
                }
            }
        }
        specs
    }

    fn baked() -> (LocoSet, StyleTable) {
        let set = bake_loco_set(&grid()).expect("the style grid bakes");
        let table = StyleTable::from_set(&set, &STYLES).expect("tags read");
        (set, table)
    }

    fn next(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed >> 8
    }

    /// A query on a random baked frame with a little noise, and what to ask for.
    struct Ask {
        query: Query,
        gait: u16,
        dial: Dial,
    }

    fn asks(db: &MotionDb, n: usize) -> Vec<Ask> {
        let mut seed = 0x2040_u32;
        (0..n)
            .map(|_| {
                let entry = db.entries[next(&mut seed) as usize % db.entries.len()];
                let mut feat = entry.feat;
                for lane in feat.iter_mut().take(FEAT_N) {
                    *lane += (next(&mut seed) % 33) as i32 - 16;
                }
                let style = (next(&mut seed) % 3) as u8;
                let slope_pct = SLOPES[next(&mut seed) as usize % SLOPES.len()];
                Ask {
                    query: Query {
                        feat,
                        contact: entry.contact,
                        curr: NO_HIST,
                        weights: default_weights(),
                        jump_cost: 0,
                        contact_cost: 0,
                    },
                    gait: entry.clip / VARIANTS as u16,
                    dial: Dial {
                        style,
                        style_cost: cost_units(0.25),
                        slope_pct,
                        slope_cost: cost_units(0.01),
                    },
                }
            })
            .collect()
    }

    #[test]
    fn the_table_reads_the_bake_tags() {
        let (set, table) = baked();
        // Gait 0, style 1 (sneak), grade +10 is clip 3 + 2 within the gait.
        let sneak_uphill = set
            .clips
            .iter()
            .position(|c| c.name == "walk_sneak_10")
            .expect("clip");
        assert_eq!(
            table.meta(sneak_uphill as u16),
            ClipMeta {
                style: 1,
                slope_pct: 10
            }
        );
        let proud_downhill = set
            .clips
            .iter()
            .position(|c| c.name == "run_proud_n10")
            .expect("clip");
        assert_eq!(
            table.meta(proud_downhill as u16),
            ClipMeta {
                style: 2,
                slope_pct: -10
            }
        );
        assert_eq!(
            table.meta(0),
            ClipMeta {
                style: 0,
                slope_pct: -10
            }
        );
        assert_eq!(table.meta(999), ClipMeta::default());
    }

    #[test]
    fn a_style_query_returns_style_clips_in_99_percent_of_1000_searches() {
        let (set, table) = baked();
        let asks = asks(&set.db, 1000);
        let (mut styled, mut plain, mut both) = (0, 0, 0);
        for ask in &asks {
            let (hit, _) = search_styled(&set.db, &table, &ask.query, &ask.dial).expect("searches");
            let meta = table.meta(hit.clip);
            let right_gait = hit.clip / VARIANTS as u16 == ask.gait;
            styled += usize::from(meta.style == ask.dial.style && right_gait);
            both += usize::from(
                meta.style == ask.dial.style && meta.slope_pct == ask.dial.slope_pct && right_gait,
            );
            // The same query with the dial off keeps the first variant of the gait.
            let (flat, _) = search_styled(&set.db, &table, &ask.query, &Dial::OFF).expect("flat");
            plain += usize::from(table.meta(flat.clip).style == ask.dial.style);
        }
        assert!(styled >= 990, "style honoured in {styled} of 1000");
        assert!(both >= 990, "style and grade honoured in {both} of 1000");
        assert!(
            plain < 500,
            "without the dial {plain} of 1000 land on the style"
        );
    }

    #[test]
    fn a_grade_between_two_clips_takes_the_nearer_and_ties_the_lower_index() {
        let (set, table) = baked();
        let ask = &asks(&set.db, 1)[0];
        let pick = |slope_pct: i16| {
            let dial = Dial {
                style: 0,
                style_cost: 0,
                slope_pct,
                slope_cost: cost_units(0.01),
            };
            let (hit, _) = search_styled(&set.db, &table, &ask.query, &dial).expect("searches");
            table.meta(hit.clip).slope_pct
        };
        assert_eq!((pick(4), pick(6)), (0, 10));
        // 5 percent is 5 from flat and 5 from +10: the clip that comes first wins.
        assert_eq!(pick(5), 0);
        assert_eq!(pick(-7), -10);
    }

    #[test]
    fn an_off_dial_is_the_plain_search_bit_for_bit() {
        let (set, table) = baked();
        for ask in asks(&set.db, 1000) {
            let (plain, _) = search(&set.db, &ask.query).expect("plain");
            let (styled, _) = search_styled(&set.db, &table, &ask.query, &Dial::OFF).expect("off");
            assert_eq!(styled, plain);
        }
    }

    #[test]
    fn the_same_asks_give_the_same_bytes_twice() {
        let run = || {
            let (set, table) = baked();
            asks(&set.db, 1000)
                .iter()
                .map(|ask| search_styled(&set.db, &table, &ask.query, &ask.dial).expect("ok"))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn bad_tags_and_bad_dials_are_refused() {
        let (set, _) = baked();
        // A style the caller did not list.
        assert!(matches!(
            StyleTable::from_set(&set, &["sneak"]),
            Err(StyleError::UnknownStyle(tag)) if tag == "style_proud"
        ));
        for bad in ["", "x10", "p", "p101", "p1x", "p-5"] {
            assert_eq!(parse_slope(bad), None, "{bad:?}");
        }
        assert_eq!(parse_slope("n100"), Some(-100));
        let ask = &asks(&set.db, 1)[0];
        let table = StyleTable::default();
        for dial in [
            Dial {
                style_cost: -1,
                ..Dial::OFF
            },
            Dial {
                slope_cost: PENALTY_MAX + 1,
                ..Dial::OFF
            },
            Dial {
                slope_pct: 101,
                ..Dial::OFF
            },
        ] {
            assert!(matches!(
                search_styled(&set.db, &table, &ask.query, &dial),
                Err(MatchError::BadQuery(_))
            ));
        }
        assert_eq!(
            search_styled(&MotionDb::default(), &table, &ask.query, &Dial::OFF),
            Err(MatchError::EmptyDatabase)
        );
    }
}
