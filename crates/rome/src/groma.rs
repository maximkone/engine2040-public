//! groma: the surveyor's cross and the compass that lay a marching camp out.
//!
//! Pseudo-Hyginus does not draw a camp, he surveys one. The groma is set at the
//! praetorian gate (Hyginus 12), the circinus is opened to the gate's hinge and
//! the *clavicula* is struck across the camp to find its centre (Hyginus 55),
//! the corners are rounded from the corners of the cohort lines (Hyginus 54),
//! and the tenth gate stands on the high point (Hyginus 56).
//!
//! The plan this module returns is checkable, not a vibe:
//!
//! - the *clavicula* bisects the camp: it runs from the gate's middle point
//!   along the camp's axis, and the centre is its midpoint;
//! - the gates stand a whole number of *actae* apart, the survey square of
//!   120 ft a side, so the frontage and the depth are whole actae;
//! - the frontage is a whole number of cohort frontages (Hyginus 2: 30 ft of
//!   frontage for 720 ft of depth).
//!
//! The ground decides the size: the survey takes the widest camp the ground
//! holds, in whole actae. Whole millimetres only, no float, no IO and no clock,
//! so the same ground and the same table always give the same plan.

use crate::camp::{Camp, Ditch, Threat};
use crate::Measure;
use engine_core::Fnv64;

/// The survey square the camp is laid out on: the actus quadratus, 120 ft a side.
const ACTA_FT: u32 = 120;

/// One cohort's frontage: 30 ft (Hyginus 2).
const COHORT_FRONTAGE_FT: u32 = 30;

/// One cohort's depth: 720 ft (Hyginus 2).
const COHORT_DEPTH_FT: u32 = 720;

/// The width of the work the cohort corners make, and so the radius the compass
/// rounds the camp's corners with: 60 ft (Hyginus 54).
const CORNER_RADIUS_FT: u32 = 60;

/// A point on the ground, in whole millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Across the ground, from its left edge.
    pub x: u32,
    /// Into the ground, from its front edge.
    pub y: u32,
}

/// A rectangle of ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rectangle {
    /// The rectangle's front-left corner.
    pub at: Point,
    /// The rectangle's width, across the ground.
    pub width: u32,
    /// The rectangle's depth, into the ground.
    pub depth: u32,
}

/// A straight piece of wall or street.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Where the piece starts.
    pub from: Point,
    /// Where the piece ends.
    pub to: Point,
}

impl Segment {
    /// The piece's length in millimetres. A piece runs across the ground or
    /// into it, never both, so its length is the one whole span it covers.
    #[must_use]
    pub fn length(&self) -> u32 {
        let across = u64::from(self.to.x).abs_diff(u64::from(self.from.x));
        let into = u64::from(self.to.y).abs_diff(u64::from(self.from.y));
        u32::try_from(across.max(into)).unwrap_or(u32::MAX)
    }
}

/// Which gate of the camp a gate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateKind {
    /// The praetorian gate, in the front wall, facing the enemy.
    Praetorian,
    /// The decumana gate, in the rear wall, on the high point.
    Decumana,
    /// The principalis dextra, the right principalis gate.
    PrincipalisDextra,
    /// The principalis sinistra, the left principalis gate.
    PrincipalisSinistra,
}

/// One of the camp's four gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate<'a> {
    /// Which gate this is.
    pub kind: GateKind,
    /// The gate's middle, on the inner line.
    pub at: Point,
    /// How wide the gate's mouth is.
    pub width: Measure<'a>,
    /// The ground's height at the gate, in millimetres.
    pub height: u32,
}

/// Which side of the camp a run lies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The front wall, where the praetorian gate stands.
    Front,
    /// The rear wall, where the decumana gate stands.
    Rear,
    /// The right wall.
    Right,
    /// The left wall.
    Left,
}

/// One straight run of the ditch along a side of the camp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DitchRun<'a> {
    /// Which side of the camp the run lies on.
    pub side: Side,
    /// The run's near end, at the rampart's toe.
    pub from: Point,
    /// The run's far end, at the rampart's toe.
    pub to: Point,
    /// How wide the ditch opens.
    pub width: Measure<'a>,
    /// How deep the century digs it.
    pub depth: Measure<'a>,
}

/// Which street of the camp a street is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreetKind {
    /// The via sagularis, down the middle of the camp.
    Sagularis,
    /// The via principalis, across the front of the camp.
    Principalis,
    /// The via quintana, across the camp behind the praetorium.
    Quintana,
}

/// One street the camp keeps clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Street<'a> {
    /// Which street this is.
    pub kind: StreetKind,
    /// The street's centre line.
    pub line: Segment,
    /// How wide the street is.
    pub width: Measure<'a>,
}

/// The clavicula: the chord struck from the gate hinge that finds the centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clavicula {
    /// Where the compass was set: the praetorian gate's middle point.
    pub pivot: Point,
    /// The compass's radius: the distance from the pivot to the gate's hinge.
    pub radius: u32,
    /// The hinge the compass was opened to.
    pub hinge: Point,
    /// The chord: from the pivot, across the camp, to the rear wall.
    pub chord: Segment,
}

/// The survey grid the camp was laid out on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    /// The survey square, the actus quadratus: 120 ft a side.
    pub acta: u32,
    /// One cohort's frontage: 30 ft.
    pub cohort_frontage: u32,
    /// One cohort's depth: 720 ft. A camp that holds a whole cohort line is
    /// this deep; narrower ground holds a part of one.
    pub cohort_depth: u32,
    /// The radius the compass rounds the camp's corners with: 60 ft.
    pub corner: u32,
    /// How many cohort frontages the camp's frontage holds.
    pub cohorts_across: u32,
    /// How many actae the camp's frontage holds.
    pub actae_across: u32,
    /// How many actae the camp's depth holds.
    pub actae_deep: u32,
    /// The distance from the praetorian gate to a principalis gate.
    pub gate_spacing: u32,
}

/// A camp the groma lays out on the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan<'a> {
    /// The camp's outer front-left corner, on the ground.
    pub origin: Point,
    /// The camp's inner line: the rectangle the tents stand on.
    pub inner: Rectangle,
    /// The works on each side of the inner line: the rampart and the ditch.
    pub works: u32,
    /// The survey grid the camp was laid out on.
    pub grid: Grid,
    /// The four gates: praetorian, decumana, principalis dextra, principalis sinistra.
    pub gates: [Gate<'a>; 4],
    /// The four ditch runs: front, rear, right, left.
    pub ditches: [DitchRun<'a>; 4],
    /// The three streets: sagularis, principalis, quintana.
    pub streets: [Street<'a>; 3],
    /// The clavicula, the chord that finds the centre.
    pub clavicula: Clavicula,
    /// The centre the clavicula finds.
    pub centre: Point,
}

impl Plan<'_> {
    /// A digest of the plan's numbers: two surveys of the same ground and the
    /// same table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/groma/1");
        hasher.write_u32(self.origin.x);
        hasher.write_u32(self.origin.y);
        hasher.write_u32(self.inner.at.x);
        hasher.write_u32(self.inner.at.y);
        hasher.write_u32(self.inner.width);
        hasher.write_u32(self.inner.depth);
        hasher.write_u32(self.works);
        hasher.write_u32(self.centre.x);
        hasher.write_u32(self.centre.y);
        hasher.write_u32(self.clavicula.radius);
        hasher.write_u32(self.clavicula.hinge.x);
        hasher.write_u32(self.clavicula.hinge.y);
        hasher.write_u32(self.clavicula.chord.to.y);
        for gate in &self.gates {
            hasher.write_u32(gate.at.x);
            hasher.write_u32(gate.at.y);
            hasher.write_u32(gate.width.mm);
            hasher.write_u32(gate.height);
        }
        for run in &self.ditches {
            hasher.write_u32(run.from.x);
            hasher.write_u32(run.from.y);
            hasher.write_u32(run.to.x);
            hasher.write_u32(run.to.y);
            hasher.write_u32(run.width.mm);
            hasher.write_u32(run.depth.mm);
        }
        for street in &self.streets {
            hasher.write_u32(street.line.from.x);
            hasher.write_u32(street.line.from.y);
            hasher.write_u32(street.line.to.x);
            hasher.write_u32(street.line.to.y);
            hasher.write_u32(street.width.mm);
        }
        hasher.finish()
    }
}

/// The ground the survey measured: a rectangle of whole millimetres with a
/// height at every sample of the grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    width: u32,
    depth: u32,
    cell: u32,
    heights: Vec<u32>,
}

impl Site {
    /// Level ground: the same height at every sample of the grid.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError::ZeroCell`] when the sample spacing is zero, for a
    /// grid has no shape then.
    pub fn flat(width: u32, depth: u32, cell: u32, height: u32) -> Result<Self, SiteError> {
        if cell == 0 {
            return Err(SiteError::ZeroCell);
        }
        let samples = samples(width, cell) * samples(depth, cell);
        Ok(Self {
            width,
            depth,
            cell,
            heights: vec![height; samples],
        })
    }

    /// Ground with a height at every sample of the grid.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError::ZeroCell`] when the sample spacing is zero and
    /// [`SiteError::SampleCount`] when the heights do not fill the grid the
    /// extent and the spacing ask for.
    pub fn with_heights(
        width: u32,
        depth: u32,
        cell: u32,
        heights: Vec<u32>,
    ) -> Result<Self, SiteError> {
        if cell == 0 {
            return Err(SiteError::ZeroCell);
        }
        let expected = samples(width, cell) * samples(depth, cell);
        if heights.len() != expected {
            return Err(SiteError::SampleCount {
                expected,
                found: heights.len(),
            });
        }
        Ok(Self {
            width,
            depth,
            cell,
            heights,
        })
    }

    /// The ground's width, in millimetres.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The ground's depth, in millimetres.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The spacing of the height samples, in millimetres.
    #[must_use]
    pub fn cell(&self) -> u32 {
        self.cell
    }

    /// The height of the ground at a point, in millimetres, read from the
    /// nearest sample.
    #[must_use]
    pub fn height_at(&self, x: u32, y: u32) -> u32 {
        let column = sample_index(x, self.cell, self.width);
        let row = sample_index(y, self.cell, self.depth);
        self.heights[row * samples(self.width, self.cell) + column]
    }

    /// The x of every sample column of the grid, in order.
    pub fn sample_columns(&self) -> impl Iterator<Item = u32> + '_ {
        let cell = self.cell;
        (0..samples(self.width, cell)).map(move |index| {
            let x = u64::try_from(index).unwrap_or(u64::MAX) * u64::from(cell);
            u32::try_from(x).unwrap_or(u32::MAX)
        })
    }

    /// Raise one sample of the ground, by the point it stands at.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError::NoSample`] when the grid holds no sample at that
    /// point.
    pub fn raise(&mut self, x: u32, y: u32, by: u32) -> Result<(), SiteError> {
        let column = sample_index(x, self.cell, self.width);
        let row = sample_index(y, self.cell, self.depth);
        let index = row * samples(self.width, self.cell) + column;
        let Some(height) = self.heights.get_mut(index) else {
            return Err(SiteError::NoSample { x, y });
        };
        *height = height.saturating_add(by);
        Ok(())
    }
}

/// Why a site could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteError {
    /// The sample spacing is zero, so the grid has no shape.
    ZeroCell,
    /// The heights do not fill the grid the extent and the spacing ask for.
    SampleCount {
        /// How many samples the grid holds.
        expected: usize,
        /// How many samples were given.
        found: usize,
    },
    /// The grid holds no sample at that point.
    NoSample {
        /// The x of the point.
        x: u32,
        /// The y of the point.
        y: u32,
    },
}

impl core::fmt::Display for SiteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroCell => write!(f, "the sample spacing is zero"),
            Self::SampleCount { expected, found } => {
                write!(f, "the grid holds {expected} samples, {found} were given")
            }
            Self::NoSample { x, y } => write!(f, "the grid holds no sample at {x}, {y}"),
        }
    }
}

impl std::error::Error for SiteError {}

/// Why the survey could not lay the camp out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurveyError {
    /// The ground is too small for the works and one whole survey square.
    GroundTooSmall {
        /// The ground's width, in millimetres.
        width: u32,
        /// The ground's depth, in millimetres.
        depth: u32,
    },
    /// A length of the table does not fit a millimetre.
    LengthOverflow,
}

impl core::fmt::Display for SurveyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::GroundTooSmall { width, depth } => {
                write!(f, "the ground of {width} by {depth} mm holds no camp")
            }
            Self::LengthOverflow => write!(f, "a length of the table does not fit a millimetre"),
        }
    }
}

impl std::error::Error for SurveyError {}

/// Lay the camp out on the ground with the groma.
///
/// The ground decides the size: the frontage is the widest run of survey
/// squares the ground allows and the depth is the deepest, so both are whole
/// actae and the gates stand a whole number of actae apart. The frontage is
/// also a whole number of cohort frontages, because the straight front wall
/// holds an even number of actae.
///
/// # Errors
///
/// Returns [`SurveyError::GroundTooSmall`] when the ground cannot hold the
/// works and a whole survey square, and [`SurveyError::LengthOverflow`] when a
/// length of the table does not fit a millimetre.
pub fn survey<'a>(site: &Site, works: &Camp<'a>, threat: Threat) -> Result<Plan<'a>, SurveyError> {
    let units = Units::new(works.foot())?;
    let ditch = works.ditch(threat);
    let size = Size::fit(site, &units, works.rampart.width.mm, ditch.width.mm)?;
    let inner = Rectangle {
        at: Point {
            x: size.origin.x + size.band,
            y: size.origin.y + size.band,
        },
        width: size.frontage,
        depth: size.depth,
    };
    let grid = Grid {
        acta: units.acta,
        cohort_frontage: units.cohort_frontage,
        cohort_depth: units.cohort_depth,
        corner: units.corner,
        cohorts_across: size.frontage / units.cohort_frontage,
        actae_across: size.frontage / units.acta,
        actae_deep: size.depth / units.acta,
        gate_spacing: (size.runs - 1) * units.acta,
    };
    Ok(Plan {
        origin: size.origin,
        inner,
        works: size.band,
        grid,
        gates: gates(&inner, &units, works, site),
        ditches: ditches(&inner, &units, works.rampart.width.mm, ditch),
        streets: streets(&inner, works),
        clavicula: clavicula(&inner, works),
        centre: Point {
            x: inner.at.x + inner.width / 2,
            y: inner.at.y + inner.depth / 2,
        },
    })
}

/// The survey's own lengths, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Units {
    acta: u32,
    cohort_frontage: u32,
    cohort_depth: u32,
    corner: u32,
}

impl Units {
    /// The survey's lengths, turned through the table's own Roman foot.
    fn new(foot: u32) -> Result<Self, SurveyError> {
        Ok(Self {
            acta: feet_to_mm(ACTA_FT, foot)?,
            cohort_frontage: feet_to_mm(COHORT_FRONTAGE_FT, foot)?,
            cohort_depth: feet_to_mm(COHORT_DEPTH_FT, foot)?,
            corner: feet_to_mm(CORNER_RADIUS_FT, foot)?,
        })
    }
}

/// The camp's fitted size: the widest camp the ground holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    frontage: u32,
    depth: u32,
    band: u32,
    runs: u32,
    origin: Point,
}

impl Size {
    /// Fit the camp to the ground.
    ///
    /// The straight front wall holds an even number of actae, so the frontage is
    /// twice a whole number of actae plus the corner radius at each end. The
    /// depth is a whole number of actae.
    fn fit(site: &Site, units: &Units, rampart: u32, ditch: u32) -> Result<Self, SurveyError> {
        let too_small = || SurveyError::GroundTooSmall {
            width: site.width(),
            depth: site.depth(),
        };
        let band = u64::from(rampart) + u64::from(ditch);
        let usable_width = u64::from(site.width()).saturating_sub(2 * band);
        let usable_depth = u64::from(site.depth()).saturating_sub(2 * band);
        let straight =
            usable_width.saturating_sub(2 * u64::from(units.corner)) / (2 * u64::from(units.acta));
        if straight < 2 {
            return Err(too_small());
        }
        let frontage = 2 * (straight * u64::from(units.acta) + u64::from(units.corner));
        let acts = usable_depth / u64::from(units.acta);
        if acts == 0 {
            return Err(too_small());
        }
        let depth = acts * u64::from(units.acta);
        let outer_width = frontage + 2 * band;
        let outer_depth = depth + 2 * band;
        Ok(Self {
            frontage: to_u32(frontage)?,
            depth: to_u32(depth)?,
            band: to_u32(band)?,
            runs: to_u32(straight)?,
            origin: Point {
                x: to_u32((u64::from(site.width()) - outer_width) / 2)?,
                y: to_u32((u64::from(site.depth()) - outer_depth) / 2)?,
            },
        })
    }
}

/// The four gates: the praetorian in the front wall, the decumana on the high
/// point of the rear wall, and the two principalis gates on the front wall.
fn gates<'a>(inner: &Rectangle, units: &Units, works: &Camp<'a>, site: &Site) -> [Gate<'a>; 4] {
    let mouth = works.gate_clear;
    let front = inner.at.y;
    let rear = inner.at.y + inner.depth;
    let middle = inner.at.x + inner.width / 2;
    let left = inner.at.x + units.corner + units.acta;
    let right = inner.at.x + inner.width - units.corner - units.acta;
    let (high, height) = high_point(
        site,
        inner.at.x + units.corner,
        inner.at.x + inner.width - units.corner,
        rear,
    );
    [
        Gate {
            kind: GateKind::Praetorian,
            at: Point {
                x: middle,
                y: front,
            },
            width: mouth,
            height: site.height_at(middle, front),
        },
        Gate {
            kind: GateKind::Decumana,
            at: Point { x: high, y: rear },
            width: mouth,
            height,
        },
        Gate {
            kind: GateKind::PrincipalisDextra,
            at: Point { x: right, y: front },
            width: mouth,
            height: site.height_at(right, front),
        },
        Gate {
            kind: GateKind::PrincipalisSinistra,
            at: Point { x: left, y: front },
            width: mouth,
            height: site.height_at(left, front),
        },
    ]
}

/// The highest ground on the camp's rear wall, and where the tenth gate stands.
///
/// The highest sample wins. On level ground every sample is as high as the
/// next, so the gate keeps the middle of the wall and the camp stays square.
fn high_point(site: &Site, left: u32, right: u32, rear: u32) -> (u32, u32) {
    let middle = left + (right - left) / 2;
    let mut best: Option<(u32, u32)> = None;
    for x in site.sample_columns() {
        if x < left || x > right {
            continue;
        }
        let height = site.height_at(x, rear);
        let better = match best {
            None => true,
            Some((best_x, best_height)) => {
                height > best_height || (height == best_height && nearer(x, middle, best_x))
            }
        };
        if better {
            best = Some((x, height));
        }
    }
    best.unwrap_or_else(|| (middle, site.height_at(middle, rear)))
}

/// Whether a point lies nearer the middle of a wall than another point does.
fn nearer(x: u32, middle: u32, other: u32) -> bool {
    let distance = |point: u32| u64::from(point).abs_diff(u64::from(middle));
    distance(x) < distance(other)
}

/// The four ditch runs, one along each side of the camp, at the rampart's toe.
fn ditches<'a>(
    inner: &Rectangle,
    units: &Units,
    rampart: u32,
    ditch: Ditch<'a>,
) -> [DitchRun<'a>; 4] {
    let toe = units.corner + rampart;
    let near_x = inner.at.x + toe;
    let far_x = inner.at.x + inner.width - toe;
    let near_y = inner.at.y + toe;
    let far_y = inner.at.y + inner.depth - toe;
    let front_y = inner.at.y - rampart;
    let rear_y = inner.at.y + inner.depth + rampart;
    let right_x = inner.at.x + inner.width + rampart;
    let left_x = inner.at.x - rampart;
    [
        DitchRun {
            side: Side::Front,
            from: Point {
                x: near_x,
                y: front_y,
            },
            to: Point {
                x: far_x,
                y: front_y,
            },
            width: ditch.width,
            depth: ditch.depth,
        },
        DitchRun {
            side: Side::Rear,
            from: Point {
                x: near_x,
                y: rear_y,
            },
            to: Point {
                x: far_x,
                y: rear_y,
            },
            width: ditch.width,
            depth: ditch.depth,
        },
        DitchRun {
            side: Side::Right,
            from: Point {
                x: right_x,
                y: near_y,
            },
            to: Point {
                x: right_x,
                y: far_y,
            },
            width: ditch.width,
            depth: ditch.depth,
        },
        DitchRun {
            side: Side::Left,
            from: Point {
                x: left_x,
                y: near_y,
            },
            to: Point {
                x: left_x,
                y: far_y,
            },
            width: ditch.width,
            depth: ditch.depth,
        },
    ]
}

/// The three streets the camp keeps clear.
fn streets<'a>(inner: &Rectangle, works: &Camp<'a>) -> [Street<'a>; 3] {
    let sagularis = works.streets.via_sagularis;
    let principalis = works.streets.via_principalis;
    let quintana = works.streets.via_quintana;
    let middle = inner.at.x + inner.width / 2;
    let front_street = inner.at.y + principalis.mm / 2;
    let middle_street = inner.at.y + inner.depth / 2;
    [
        Street {
            kind: StreetKind::Sagularis,
            line: Segment {
                from: Point {
                    x: middle,
                    y: inner.at.y,
                },
                to: Point {
                    x: middle,
                    y: inner.at.y + inner.depth,
                },
            },
            width: sagularis,
        },
        Street {
            kind: StreetKind::Principalis,
            line: Segment {
                from: Point {
                    x: inner.at.x,
                    y: front_street,
                },
                to: Point {
                    x: inner.at.x + inner.width,
                    y: front_street,
                },
            },
            width: principalis,
        },
        Street {
            kind: StreetKind::Quintana,
            line: Segment {
                from: Point {
                    x: inner.at.x,
                    y: middle_street,
                },
                to: Point {
                    x: inner.at.x + inner.width,
                    y: middle_street,
                },
            },
            width: quintana,
        },
    ]
}

/// The clavicula: the chord struck from the gate hinge that finds the centre.
///
/// The groma is set at the praetorian gate's middle point and the circinus is
/// opened to the gate's hinge; struck across the road, the chord runs along
/// the camp's axis. The centre of the camp is the chord's midpoint.
fn clavicula<'a>(inner: &Rectangle, works: &Camp<'a>) -> Clavicula {
    let pivot = Point {
        x: inner.at.x + inner.width / 2,
        y: inner.at.y,
    };
    let radius = works.gate_clear.mm / 2;
    Clavicula {
        pivot,
        radius,
        hinge: Point {
            x: pivot.x - radius,
            y: pivot.y,
        },
        chord: Segment {
            from: pivot,
            to: Point {
                x: pivot.x,
                y: inner.at.y + inner.depth,
            },
        },
    }
}

/// A length in Roman feet turned into millimetres.
fn feet_to_mm(feet: u32, foot: u32) -> Result<u32, SurveyError> {
    u32::try_from(u64::from(feet) * u64::from(foot)).map_err(|_| SurveyError::LengthOverflow)
}

/// A millimetre length that fits a `u32`.
fn to_u32(value: u64) -> Result<u32, SurveyError> {
    u32::try_from(value).map_err(|_| SurveyError::LengthOverflow)
}

/// How many samples a ground of this extent holds, both edges counted.
fn samples(extent: u32, cell: u32) -> usize {
    if cell == 0 {
        return 1;
    }
    usize::try_from(extent / cell).unwrap_or(usize::MAX) + 1
}

/// The index of the sample nearest a point.
fn sample_index(value: u32, cell: u32, extent: u32) -> usize {
    if cell == 0 {
        return 0;
    }
    let index = (u64::from(value) + u64::from(cell) / 2) / u64::from(cell);
    let last = u64::from(extent) / u64::from(cell);
    usize::try_from(index.min(last)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat site two hundred metres across, sampled every ten metres.
    fn flat_site() -> Site {
        Site::flat(200_000, 200_000, 10_000, 0).expect("flat site")
    }

    /// The camp table the game ships.
    fn works() -> Camp<'static> {
        Camp::shipped().expect("shipped camp table")
    }

    /// One gate of a plan.
    fn gate<'plan, 'text>(plan: &'plan Plan<'text>, kind: GateKind) -> &'plan Gate<'text> {
        plan.gates
            .iter()
            .find(|gate| gate.kind == kind)
            .expect("the camp has that gate")
    }

    /// One street of a plan.
    fn street<'plan, 'text>(plan: &'plan Plan<'text>, kind: StreetKind) -> &'plan Street<'text> {
        plan.streets
            .iter()
            .find(|street| street.kind == kind)
            .expect("the camp has that street")
    }

    #[test]
    fn camp_ditch_runs_are_the_table_sizes() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        assert_eq!(plan.ditches.len(), 4, "the camp has four ditch runs");
        let ditch = works.ditch(Threat::Light);
        for run in plan.ditches {
            assert_eq!(
                run.width.mm, ditch.width.mm,
                "the ditch opens as the table says"
            );
            assert_eq!(
                run.depth.mm, ditch.depth.mm,
                "the ditch is dug as the table says"
            );
        }
        assert_eq!((ditch.width.mm, ditch.depth.mm), (2664, 2072), "9 x 7 ft");
        let one_night = works.ditch(Threat::OneNight);
        assert_eq!(
            (one_night.width.mm, one_night.depth.mm),
            (1480, 888),
            "5 x 3 ft"
        );
        let march = survey(&flat_site(), &works, Threat::OneNight).expect("survey");
        for run in march.ditches {
            assert_eq!(run.width.mm, 1480, "one night: 5 ft wide");
            assert_eq!(run.depth.mm, 888, "one night: 3 ft deep");
        }
    }

    #[test]
    fn camp_clavicula_bisects_the_camp() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        let chord = plan.clavicula.chord;
        assert_eq!(chord.from.x, chord.to.x, "the chord runs straight across");
        assert_eq!(
            chord.from.x, plan.centre.x,
            "the chord runs through the centre"
        );
        let left = chord.from.x - plan.inner.at.x;
        let right = plan.inner.at.x + plan.inner.width - chord.to.x;
        assert_eq!(left, right, "half the camp lies on each side of the chord");
        assert_eq!(
            plan.clavicula.pivot, chord.from,
            "the compass sat at the gate"
        );
        assert_eq!(
            plan.clavicula.hinge.x,
            plan.clavicula.pivot.x - plan.clavicula.radius,
            "the compass was open to the gate hinge"
        );
        assert_eq!(
            plan.clavicula.radius * 2,
            works.gate_clear.mm,
            "the hinge is as far off the gate's middle as the gate is wide"
        );
    }

    #[test]
    fn camp_tenth_gate_stands_on_the_high_point() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        let decumana = gate(&plan, GateKind::Decumana);
        assert_eq!(
            decumana.at.x, plan.centre.x,
            "on level ground the tenth gate keeps the middle of the rear wall"
        );

        let mut site = flat_site();
        site.raise(150_000, 190_000, 5_000).expect("the sample");
        let plan = survey(&site, &works, Threat::Light).expect("survey");
        let decumana = gate(&plan, GateKind::Decumana);
        assert_eq!(
            decumana.at.x, 150_000,
            "the tenth gate stands on the high point"
        );
        assert_eq!(decumana.height, 5_000, "and it stands on the high ground");
    }

    #[test]
    fn camp_two_surveys_give_the_same_plan() {
        let works = works();
        let site = flat_site();
        let first = survey(&site, &works, Threat::Light).expect("survey");
        let second = survey(&site, &works, Threat::Light).expect("survey");
        assert_eq!(first, second, "the same ground gives the same plan");
        assert_eq!(first.hash(), second.hash(), "and the same digest");
    }

    #[test]
    fn camp_gates_stand_a_whole_number_of_actae_apart() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        let acta = plan.grid.acta;
        assert_eq!(acta, 35_520, "the actus quadratus is 120 ft a side");
        assert_eq!(plan.grid.actae_across * acta, plan.inner.width);
        assert_eq!(plan.grid.actae_deep * acta, plan.inner.depth);
        assert_eq!(
            plan.grid.gate_spacing % acta,
            0,
            "the gates stand whole actae apart"
        );
        let praetorian = gate(&plan, GateKind::Praetorian);
        let dextra = gate(&plan, GateKind::PrincipalisDextra);
        let sinistra = gate(&plan, GateKind::PrincipalisSinistra);
        assert_eq!(
            praetorian.at.x.abs_diff(dextra.at.x),
            plan.grid.gate_spacing
        );
        assert_eq!(
            praetorian.at.x.abs_diff(sinistra.at.x),
            plan.grid.gate_spacing
        );
    }

    #[test]
    fn camp_frontage_is_a_whole_number_of_cohort_frontages() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        assert_eq!(plan.grid.cohort_frontage, 8_880, "30 ft of frontage");
        assert_eq!(plan.grid.cohort_depth, 213_120, "720 ft of depth");
        assert_eq!(
            plan.grid.cohorts_across * plan.grid.cohort_frontage,
            plan.inner.width,
            "the frontage is a whole number of cohort frontages"
        );
        assert_eq!(
            plan.grid.cohorts_across, 20,
            "twenty cohorts across the camp"
        );
    }

    #[test]
    fn camp_the_works_stay_on_the_ground() {
        let site = flat_site();
        let works = works();
        let plan = survey(&site, &works, Threat::Light).expect("survey");
        let right = plan.inner.at.x + plan.inner.width + plan.works;
        let rear = plan.inner.at.y + plan.inner.depth + plan.works;
        assert!(right <= site.width(), "the works stay inside the ground");
        assert!(rear <= site.depth(), "the works stay inside the ground");
    }

    #[test]
    fn camp_the_streets_are_the_table_widths() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        assert_eq!(
            street(&plan, StreetKind::Sagularis).width.mm,
            works.streets.via_sagularis.mm
        );
        assert_eq!(
            street(&plan, StreetKind::Principalis).width.mm,
            works.streets.via_principalis.mm
        );
        assert_eq!(
            street(&plan, StreetKind::Quintana).width.mm,
            works.streets.via_quintana.mm
        );
        assert_eq!(
            street(&plan, StreetKind::Sagularis).line.from.x,
            plan.clavicula.chord.from.x,
            "the sagularis runs down the line the clavicula struck"
        );
    }

    #[test]
    fn camp_the_plan_has_the_numbers_the_survey_works_to() {
        let works = works();
        let plan = survey(&flat_site(), &works, Threat::Light).expect("survey");
        assert_eq!(plan.inner.width, 177_600, "600 ft of frontage");
        assert_eq!(plan.inner.depth, 177_600, "600 ft of depth");
        assert_eq!(plan.works, 5_032, "the rampart and the ditch on each side");
        assert_eq!(
            plan.grid.corner, 17_760,
            "the compass rounds the corners 60 ft"
        );
        assert_eq!(plan.grid.gate_spacing, 35_520, "one acta between the gates");
        assert_eq!(
            gate(&plan, GateKind::Praetorian).at,
            Point {
                x: 100_000,
                y: 11_200
            }
        );
        assert_eq!(
            gate(&plan, GateKind::Decumana).at,
            Point {
                x: 100_000,
                y: 188_800
            }
        );
    }

    #[test]
    fn camp_ground_too_small_for_a_camp_is_refused() {
        let works = works();
        let site = Site::flat(10_000, 10_000, 1_000, 0).expect("site");
        assert_eq!(
            survey(&site, &works, Threat::Light),
            Err(SurveyError::GroundTooSmall {
                width: 10_000,
                depth: 10_000
            })
        );
    }
}
