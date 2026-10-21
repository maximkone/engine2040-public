//! geo: the chapter maps as real ground.
//!
//! The game is set in six chapter areas, and every chapter map is a box of
//! ground with a settlement gazetteer and, for the frontier chapters, the
//! frontier line itself. This module reads the two shipped tables that hold
//! that ground:
//!
//! - `content/rome/places.toml` holds the settlements of the six chapter areas:
//!   a slice of the gazetteer of the empire under Hadrian, cut by the box of
//!   each area, with the cross-reference ids (Pleiades, Barrington, DARE,
//!   Wikidata) and the founding year of every place.
//! - `content/rome/fortifications.toml` holds the frontier as geometry: the
//!   Upper German and Raetian Limes as named, segmented lines, the Main and the
//!   Neckar as rivers, and the mile markers of Hadrian's Wall.
//!
//! Every number is an integer. A coordinate is micro-degrees, a distance is
//! millimetres, and both tables are embedded with `include_str!`, so the tick
//! never reads a file and two loads of one table hash the same. There is no
//! float, no clock and no IO here.
//!
//! The rules the loader keeps:
//!
//! - every entry names its source in `ref`, and an entry without one is
//!   refused by name;
//! - a place names an area the table holds, and it lies inside the box of
//!   that area, so a chapter area really is a box filter over the gazetteer;
//! - a frontier segment carries at least two points, so it has a length.

use core::fmt;

use crate::table::{Entry, Table};
use crate::{int, only, LoadError};
use engine_core::fixed::isqrt;
use engine_core::Fnv64;

/// Why a geo table could not be read into its typed form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeoError<'a> {
    /// Another Rome table refused the file first.
    Load(LoadError<'a>),
    /// The file is not a well-formed table.
    Table(crate::TableError<'a>),
    /// A required section is missing.
    MissingEntry {
        /// The section key, for example `area.danube`.
        entry: &'a str,
    },
    /// A required key is missing from a section that exists.
    MissingKey {
        /// The section key.
        entry: &'a str,
        /// The missing key.
        key: &'a str,
    },
    /// A section holds a key the typed form does not know.
    UnknownKey {
        /// The section key.
        entry: &'a str,
        /// The unknown key.
        key: &'a str,
    },
    /// An entry carries no `ref` naming its source.
    MissingRef {
        /// The section key, so the refusal names the entry.
        entry: &'a str,
    },
    /// A value is not the integer the typed form reads.
    BadValue {
        /// The section key.
        entry: &'a str,
        /// The key the bad value sits under.
        key: &'a str,
        /// The value as written.
        value: &'a str,
    },
    /// A number does not fit the type the key holds.
    NumberOverflow {
        /// The section key.
        entry: &'a str,
        /// The overflowing key.
        key: &'a str,
    },
    /// A place names an area the table does not hold.
    NoSuchArea {
        /// The place key.
        place: &'a str,
        /// The area the place names.
        area: &'a str,
    },
    /// The table holds no area with this key.
    UnknownArea {
        /// The area key.
        area: &'a str,
    },
    /// An area holds no place, so nothing can be nearest to a point.
    EmptyArea {
        /// The area key.
        area: &'a str,
    },
    /// A place lies outside the box of its area.
    OutsideBox {
        /// The place key.
        place: &'a str,
        /// The area the place names.
        area: &'a str,
    },
    /// A frontier segment holds fewer than two points, so it has no length.
    TooFewPoints {
        /// The section key.
        entry: &'a str,
    },
}

impl fmt::Display for GeoError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => error.fmt(f),
            Self::Table(error) => error.fmt(f),
            Self::MissingEntry { entry } => write!(f, "the table has no `[{entry}]` entry"),
            Self::MissingKey { entry, key } => write!(f, "`[{entry}]` has no `{key}`"),
            Self::UnknownKey { entry, key } => {
                write!(f, "`[{entry}]` has an unknown key `{key}`")
            }
            Self::MissingRef { entry } => {
                write!(f, "`[{entry}]` has no `ref` naming its source")
            }
            Self::BadValue { entry, key, value } => {
                write!(
                    f,
                    "`[{entry}]` {key} is `{value}`, which this crate does not read"
                )
            }
            Self::NumberOverflow { entry, key } => {
                write!(f, "`[{entry}]` {key} does not fit its number")
            }
            Self::NoSuchArea { place, area } => {
                write!(
                    f,
                    "the place `{place}` names the area `{area}`, which the table does not hold"
                )
            }
            Self::UnknownArea { area } => write!(f, "the table holds no area `{area}`"),
            Self::EmptyArea { area } => write!(f, "the area `{area}` holds no place"),
            Self::OutsideBox { place, area } => {
                write!(
                    f,
                    "the place `{place}` lies outside the box of the area `{area}`"
                )
            }
            Self::TooFewPoints { entry } => {
                write!(
                    f,
                    "`[{entry}]` holds fewer than two points, so it has no length"
                )
            }
        }
    }
}

impl std::error::Error for GeoError<'_> {}

impl<'a> From<LoadError<'a>> for GeoError<'a> {
    fn from(error: LoadError<'a>) -> Self {
        Self::Load(error)
    }
}

impl<'a> From<crate::TableError<'a>> for GeoError<'a> {
    fn from(error: crate::TableError<'a>) -> Self {
        Self::Table(error)
    }
}

/// One point of the map: a signed micro-degree longitude and latitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// The longitude, in micro-degrees east of Greenwich.
    pub lon_micro: i64,
    /// The latitude, in micro-degrees north of the equator.
    pub lat_micro: i64,
}

/// Where a table came from: the donor file, its licence and its sha256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    /// The id of the donor, for example `urbesetorbis`.
    pub id: &'a str,
    /// The path of the donor file inside its repository.
    pub file: &'a str,
    /// Where the donor file is fetched from.
    pub url: &'a str,
    /// The licence the donor states.
    pub licence: &'a str,
    /// The sha256 of the donor file as it was fetched.
    pub sha256: &'a str,
}

/// A chapter area: the box of its ground and the metric frame of its map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area<'a> {
    id: &'a str,
    key: &'a str,
    name: &'a str,
    chapter: &'a str,
    lat_min: i64,
    lat_max: i64,
    lon_min: i64,
    lon_max: i64,
    m_per_deg_lon: u32,
    source: &'a str,
}

impl<'a> Area<'a> {
    /// The id of the area, for example `danube`: the key its places name it by.
    #[must_use]
    pub fn id(&self) -> &'a str {
        self.id
    }

    /// The section key of the area, for example `area.danube`.
    #[must_use]
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// The name the plan gives the area.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// The chapter the area is the ground of, for example `IV`.
    #[must_use]
    pub fn chapter(&self) -> &'a str {
        self.chapter
    }

    /// The source the box and the frame come from.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// The metres of one degree of longitude at the centre of the box.
    #[must_use]
    pub fn metres_per_degree_lon(&self) -> u32 {
        self.m_per_deg_lon
    }

    /// True when the point lies inside the box of the area.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        point.lat_micro >= self.lat_min
            && point.lat_micro <= self.lat_max
            && point.lon_micro >= self.lon_min
            && point.lon_micro <= self.lon_max
    }
}

/// The cross-reference ids of one settlement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ids<'a> {
    /// The Pleiades place id.
    pub pleiades: &'a str,
    /// The Barrington Atlas square.
    pub barrington: &'a str,
    /// The DARE place id.
    pub dare: &'a str,
    /// The Wikidata item id.
    pub wikidata: &'a str,
}

/// One settlement of the gazetteer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place<'a> {
    key: &'a str,
    name: &'a str,
    area: &'a str,
    rank: u32,
    province: &'a str,
    founded: i32,
    point: Point,
    ids: Ids<'a>,
    source: &'a str,
}

impl<'a> Place<'a> {
    /// The section key of the place, for example `place.drobeta`.
    #[must_use]
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// The name of the settlement.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// The key of the area the place lies in.
    #[must_use]
    pub fn area(&self) -> &'a str {
        self.area
    }

    /// The rank the gazetteer gives the settlement; 1 is the largest.
    #[must_use]
    pub fn rank(&self) -> u32 {
        self.rank
    }

    /// The province the gazetteer gives the settlement.
    #[must_use]
    pub fn province(&self) -> &'a str {
        self.province
    }

    /// The year the gazetteer gives as the start of the settlement; a negative
    /// year is before the common era.
    #[must_use]
    pub fn founded(&self) -> i32 {
        self.founded
    }

    /// Where the settlement lies.
    #[must_use]
    pub fn point(&self) -> Point {
        self.point
    }

    /// The ids of the databases the settlement can be checked against.
    #[must_use]
    pub fn ids(&self) -> Ids<'a> {
        self.ids
    }

    /// The source the settlement comes from.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }
}

/// The settlement nearest a point, and how far it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nearest<'a> {
    place: Place<'a>,
    distance_mm: u64,
}

impl<'a> Nearest<'a> {
    /// The settlement nearest the point.
    #[must_use]
    pub fn place(&self) -> &Place<'a> {
        &self.place
    }

    /// The distance to the settlement, in millimetres.
    #[must_use]
    pub fn distance_mm(&self) -> u64 {
        self.distance_mm
    }
}

/// The settlements of the six chapter areas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places<'a> {
    m_per_deg_lat: u32,
    areas: Vec<Area<'a>>,
    places: Vec<Place<'a>>,
    sources: Vec<Source<'a>>,
}

impl Places<'static> {
    /// The places table shipped in `content/rome/places.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/places.toml");

    /// Read the shipped places table.
    ///
    /// # Errors
    ///
    /// Same as [`Places::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, GeoError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Places<'a> {
    /// The metres of one degree of latitude the table measures with.
    #[must_use]
    pub fn metres_per_degree_lat(&self) -> u32 {
        self.m_per_deg_lat
    }

    /// Every chapter area, in table order.
    #[must_use]
    pub fn areas(&self) -> &[Area<'a>] {
        &self.areas
    }

    /// The area with this id, for example `danube`.
    #[must_use]
    pub fn area(&self, id: &str) -> Option<&Area<'a>> {
        self.areas.iter().find(|area| area.id == id)
    }

    /// Every settlement, in table order.
    #[must_use]
    pub fn places(&self) -> &[Place<'a>] {
        &self.places
    }

    /// The settlements of one area: the box filter over the gazetteer.
    pub fn in_area<'s>(&'s self, key: &'s str) -> impl Iterator<Item = &'s Place<'a>> + 's {
        self.places.iter().filter(move |place| place.area == key)
    }

    /// The donors the table was cut from.
    #[must_use]
    pub fn sources(&self) -> &[Source<'a>] {
        &self.sources
    }

    /// The settlement of an area nearest a point.
    ///
    /// # Errors
    ///
    /// Returns [`GeoError::UnknownArea`] when the table holds no such area and
    /// [`GeoError::EmptyArea`] when the area holds no settlement.
    pub fn nearest(&self, area: &'a str, point: Point) -> Result<Nearest<'a>, GeoError<'a>> {
        let frame = self.area(area).ok_or(GeoError::UnknownArea { area })?;
        let mut best: Option<(u64, Place<'a>)> = None;
        for place in &self.places {
            if place.area != area {
                continue;
            }
            let distance = distance_mm(
                place.point,
                point,
                self.m_per_deg_lat,
                frame.metres_per_degree_lon(),
            );
            let closer = best.is_none_or(|(nearest, _)| distance < nearest);
            if closer {
                best = Some((distance, *place));
            }
        }
        let (distance_mm, place) = best.ok_or(GeoError::EmptyArea { area })?;
        Ok(Nearest { place, distance_mm })
    }

    /// A digest of the loaded table: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/places/1");
        hasher.write_u32(self.m_per_deg_lat);
        for area in &self.areas {
            hasher.write_str(area.id);
            hasher.write_str(area.key);
            hasher.write_str(area.name);
            hasher.write_str(area.chapter);
            hasher.write_i64(area.lat_min);
            hasher.write_i64(area.lat_max);
            hasher.write_i64(area.lon_min);
            hasher.write_i64(area.lon_max);
            hasher.write_u32(area.m_per_deg_lon);
            hasher.write_str(area.source);
        }
        for place in &self.places {
            hasher.write_str(place.key);
            hasher.write_str(place.name);
            hasher.write_str(place.area);
            hasher.write_u32(place.rank);
            hasher.write_str(place.province);
            hasher.write_i32(place.founded);
            hasher.write_i64(place.point.lat_micro);
            hasher.write_i64(place.point.lon_micro);
            hasher.write_str(place.ids.pleiades);
            hasher.write_str(place.ids.barrington);
            hasher.write_str(place.ids.dare);
            hasher.write_str(place.ids.wikidata);
            hasher.write_str(place.source);
        }
        for source in &self.sources {
            hasher.write_str(source.id);
            hasher.write_str(source.file);
            hasher.write_str(source.url);
            hasher.write_str(source.licence);
            hasher.write_str(source.sha256);
        }
        hasher.finish()
    }

    /// Read one places table.
    ///
    /// # Errors
    ///
    /// Returns [`GeoError::Load`] or [`GeoError::Table`] when the text is not
    /// a well-formed table, [`GeoError::MissingRef`] when an entry names no
    /// source, [`GeoError::NoSuchArea`] when a place names an area the table
    /// does not hold, and [`GeoError::OutsideBox`] when a place lies outside
    /// the box of its area.
    pub fn parse(text: &'a str) -> Result<Self, GeoError<'a>> {
        let table = Table::parse(text)?;
        let m_per_deg_lat = units_lat(&table)?;
        let mut areas: Vec<Area<'a>> = Vec::new();
        let mut places: Vec<Place<'a>> = Vec::new();
        let mut sources: Vec<Source<'a>> = Vec::new();
        for key in section_keys(text) {
            if let Some(id) = key.strip_prefix("area.") {
                areas.push(area(&table, key, id)?);
            } else if key.starts_with("place.") {
                places.push(place(&table, key)?);
            } else if key.starts_with("provenance.") {
                sources.push(source(&table, key)?);
            }
        }
        let places = Places {
            m_per_deg_lat,
            areas,
            places,
            sources,
        };
        places.check_boxes()?;
        Ok(places)
    }

    /// Every place lies inside the box of the area it names.
    fn check_boxes(&self) -> Result<(), GeoError<'a>> {
        for place in &self.places {
            let frame = self.area(place.area).ok_or(GeoError::NoSuchArea {
                place: place.key,
                area: place.area,
            })?;
            if !frame.contains(place.point) {
                return Err(GeoError::OutsideBox {
                    place: place.key,
                    area: place.area,
                });
            }
        }
        Ok(())
    }
}

/// What kind of frontier line a segment is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// A stretch of the frontier limes.
    Limes,
    /// A river the frontier follows.
    River,
    /// A mile marker of Hadrian's Wall.
    Wall,
    /// A road along the frontier.
    Road,
}

impl SegmentKind {
    /// The number the kind carries in a digest.
    const fn code(self) -> u32 {
        match self {
            Self::Limes => 0,
            Self::River => 1,
            Self::Wall => 2,
            Self::Road => 3,
        }
    }
}

/// One named frontier line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment<'a> {
    key: &'a str,
    name: &'a str,
    kind: SegmentKind,
    m_per_deg_lat: u32,
    m_per_deg_lon: u32,
    points: Vec<Point>,
    source: &'a str,
}

impl<'a> Segment<'a> {
    /// The section key of the segment, for example `segment.wall_mile_46`.
    #[must_use]
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// The name the atlas gives the line.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// What kind of line it is.
    #[must_use]
    pub fn kind(&self) -> SegmentKind {
        self.kind
    }

    /// The source the line comes from.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// The points of the line, in the order the atlas writes them.
    #[must_use]
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// The length of the line in millimetres, along its own points.
    #[must_use]
    pub fn length_mm(&self) -> u64 {
        let mut total = 0u64;
        for pair in self.points.windows(2) {
            let chord = distance_mm(pair[0], pair[1], self.m_per_deg_lat, self.m_per_deg_lon);
            total = total.saturating_add(chord);
        }
        total
    }
}

/// The frontier of the empire as geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fortifications<'a> {
    m_per_deg_lat: u32,
    segments: Vec<Segment<'a>>,
    sources: Vec<Source<'a>>,
}

impl Fortifications<'static> {
    /// The fortifications table shipped in `content/rome/fortifications.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/fortifications.toml");

    /// Read the shipped fortifications table.
    ///
    /// # Errors
    ///
    /// Same as [`Fortifications::parse`]; a test keeps the shipped table valid,
    /// so this doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, GeoError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Fortifications<'a> {
    /// The metres of one degree of latitude the table measures with.
    #[must_use]
    pub fn metres_per_degree_lat(&self) -> u32 {
        self.m_per_deg_lat
    }

    /// Every frontier line, in table order.
    #[must_use]
    pub fn segments(&self) -> &[Segment<'a>] {
        &self.segments
    }

    /// The line with this name, for example `Limes Germania superior - Pars I`.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&Segment<'a>> {
        self.segments.iter().find(|segment| segment.name == name)
    }

    /// The lines of one kind, in table order.
    pub fn of_kind(&self, kind: SegmentKind) -> impl Iterator<Item = &Segment<'a>> + '_ {
        self.segments
            .iter()
            .filter(move |segment| segment.kind == kind)
    }

    /// The donors the table was cut from.
    #[must_use]
    pub fn sources(&self) -> &[Source<'a>] {
        &self.sources
    }

    /// The length of the whole frontier in millimetres.
    #[must_use]
    pub fn length_mm(&self) -> u64 {
        let mut total = 0u64;
        for segment in &self.segments {
            total = total.saturating_add(segment.length_mm());
        }
        total
    }

    /// A digest of the loaded table: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/fortifications/1");
        hasher.write_u32(self.m_per_deg_lat);
        for segment in &self.segments {
            hasher.write_str(segment.key);
            hasher.write_str(segment.name);
            hasher.write_u32(segment.kind.code());
            hasher.write_u32(segment.m_per_deg_lon);
            for point in &segment.points {
                hasher.write_i64(point.lon_micro);
                hasher.write_i64(point.lat_micro);
            }
            hasher.write_str(segment.source);
        }
        for source in &self.sources {
            hasher.write_str(source.id);
            hasher.write_str(source.file);
            hasher.write_str(source.url);
            hasher.write_str(source.licence);
            hasher.write_str(source.sha256);
        }
        hasher.finish()
    }

    /// Read one fortifications table.
    ///
    /// # Errors
    ///
    /// Returns [`GeoError::Load`] or [`GeoError::Table`] when the text is not
    /// a well-formed table, [`GeoError::MissingRef`] when an entry names no
    /// source, and [`GeoError::TooFewPoints`] when a line holds fewer than two
    /// points.
    pub fn parse(text: &'a str) -> Result<Self, GeoError<'a>> {
        let table = Table::parse(text)?;
        let m_per_deg_lat = units_lat(&table)?;
        let mut segments: Vec<Segment<'a>> = Vec::new();
        let mut sources: Vec<Source<'a>> = Vec::new();
        for key in section_keys(text) {
            if key.starts_with("segment.") {
                segments.push(segment(&table, key, m_per_deg_lat)?);
            } else if key.starts_with("provenance.") {
                sources.push(source(&table, key)?);
            }
        }
        Ok(Fortifications {
            m_per_deg_lat,
            segments,
            sources,
        })
    }
}

/// The section keys of a table, in file order.
///
/// The table reader holds its sections but does not hand them out one by one,
/// so the loader reads the header lines itself. A file the reader accepts has
/// one header per section, so the two see the same sections.
fn section_keys(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter_map(|line| {
        let body = line.trim();
        if body.len() > 2 && body.starts_with('[') && body.ends_with(']') {
            Some(&body[1..body.len() - 1])
        } else {
            None
        }
    })
}

/// The metres of one degree of latitude a table measures with.
fn units_lat<'a>(table: &Table<'a>) -> Result<u32, GeoError<'a>> {
    let entry = table
        .entry("units")
        .ok_or(GeoError::MissingEntry { entry: "units" })?;
    only(entry, &["m_per_deg_lat"])?;
    require_ref(entry)?;
    let value = int(entry, "m_per_deg_lat")?;
    u32::try_from(value).map_err(|_| GeoError::NumberOverflow {
        entry: "units",
        key: "m_per_deg_lat",
    })
}

/// One chapter area.
fn area<'a>(table: &Table<'a>, key: &'a str, id: &'a str) -> Result<Area<'a>, GeoError<'a>> {
    let entry = table
        .entry(key)
        .ok_or(GeoError::MissingEntry { entry: key })?;
    only(
        entry,
        &[
            "name",
            "chapter",
            "lat_min",
            "lat_max",
            "lon_min",
            "lon_max",
            "m_per_deg_lon",
        ],
    )?;
    let source = require_ref(entry)?;
    let m_per_deg_lon = int(entry, "m_per_deg_lon")?;
    let m_per_deg_lon = u32::try_from(m_per_deg_lon).map_err(|_| GeoError::NumberOverflow {
        entry: key,
        key: "m_per_deg_lon",
    })?;
    Ok(Area {
        id,
        key,
        name: text(entry, "name")?,
        chapter: text(entry, "chapter")?,
        lat_min: micro(entry, "lat_min")?,
        lat_max: micro(entry, "lat_max")?,
        lon_min: micro(entry, "lon_min")?,
        lon_max: micro(entry, "lon_max")?,
        m_per_deg_lon,
        source,
    })
}

/// One settlement.
fn place<'a>(table: &Table<'a>, key: &'a str) -> Result<Place<'a>, GeoError<'a>> {
    let entry = table
        .entry(key)
        .ok_or(GeoError::MissingEntry { entry: key })?;
    only(
        entry,
        &[
            "name",
            "area",
            "rank",
            "province",
            "founded",
            "lat",
            "lon",
            "pleiades",
            "barrington",
            "dare",
            "wikidata",
        ],
    )?;
    let source = require_ref(entry)?;
    let rank = int(entry, "rank")?;
    let rank = u32::try_from(rank).map_err(|_| GeoError::NumberOverflow {
        entry: key,
        key: "rank",
    })?;
    let founded_raw = text(entry, "founded")?;
    let founded = signed(founded_raw).ok_or(GeoError::BadValue {
        entry: key,
        key: "founded",
        value: founded_raw,
    })?;
    let founded = i32::try_from(founded).map_err(|_| GeoError::NumberOverflow {
        entry: key,
        key: "founded",
    })?;
    Ok(Place {
        key,
        name: text(entry, "name")?,
        area: text(entry, "area")?,
        rank,
        province: text(entry, "province")?,
        founded,
        point: Point {
            lon_micro: micro(entry, "lon")?,
            lat_micro: micro(entry, "lat")?,
        },
        ids: Ids {
            pleiades: text(entry, "pleiades")?,
            barrington: text(entry, "barrington")?,
            dare: text(entry, "dare")?,
            wikidata: text(entry, "wikidata")?,
        },
        source,
    })
}

/// One frontier line.
fn segment<'a>(
    table: &Table<'a>,
    key: &'a str,
    m_per_deg_lat: u32,
) -> Result<Segment<'a>, GeoError<'a>> {
    let entry = table
        .entry(key)
        .ok_or(GeoError::MissingEntry { entry: key })?;
    only(entry, &["name", "kind", "m_per_deg_lon", "points"])?;
    let source = require_ref(entry)?;
    let m_per_deg_lon = int(entry, "m_per_deg_lon")?;
    let m_per_deg_lon = u32::try_from(m_per_deg_lon).map_err(|_| GeoError::NumberOverflow {
        entry: key,
        key: "m_per_deg_lon",
    })?;
    let points = points(entry)?;
    if points.len() < 2 {
        return Err(GeoError::TooFewPoints { entry: key });
    }
    Ok(Segment {
        key,
        name: text(entry, "name")?,
        kind: kind(text(entry, "kind")?)?,
        m_per_deg_lat,
        m_per_deg_lon,
        points,
        source,
    })
}

/// The donor record of a table.
fn source<'a>(table: &Table<'a>, key: &'a str) -> Result<Source<'a>, GeoError<'a>> {
    let entry = table
        .entry(key)
        .ok_or(GeoError::MissingEntry { entry: key })?;
    only(entry, &["id", "file", "url", "licence", "sha256"])?;
    Ok(Source {
        id: text(entry, "id")?,
        file: text(entry, "file")?,
        url: text(entry, "url")?,
        licence: text(entry, "licence")?,
        sha256: text(entry, "sha256")?,
    })
}

/// The kind of a frontier line.
fn kind<'a>(text: &'a str) -> Result<SegmentKind, GeoError<'a>> {
    match text {
        "limes" => Ok(SegmentKind::Limes),
        "river" => Ok(SegmentKind::River),
        "wall" => Ok(SegmentKind::Wall),
        "road" => Ok(SegmentKind::Road),
        other => Err(GeoError::BadValue {
            entry: "segment",
            key: "kind",
            value: other,
        }),
    }
}

/// The points of a frontier line: `lon,lat` pairs separated by spaces.
fn points<'a>(entry: &Entry<'a>) -> Result<Vec<Point>, GeoError<'a>> {
    let raw = text(entry, "points")?;
    let mut out = Vec::new();
    for pair in raw.split(' ') {
        if pair.is_empty() {
            continue;
        }
        let Some((lon, lat)) = pair.split_once(',') else {
            return Err(GeoError::BadValue {
                entry: entry.key(),
                key: "points",
                value: pair,
            });
        };
        let (Some(lon), Some(lat)) = (signed(lon), signed(lat)) else {
            return Err(GeoError::BadValue {
                entry: entry.key(),
                key: "points",
                value: pair,
            });
        };
        out.push(Point {
            lon_micro: lon,
            lat_micro: lat,
        });
    }
    Ok(out)
}

/// One required string from a section.
fn text<'a>(entry: &Entry<'a>, key: &'a str) -> Result<&'a str, GeoError<'a>> {
    entry.text(key).ok_or_else(|| GeoError::MissingKey {
        entry: entry.key(),
        key,
    })
}

/// The `ref` of an entry: where its numbers come from.
fn require_ref<'a>(entry: &Entry<'a>) -> Result<&'a str, GeoError<'a>> {
    let source = entry.source();
    if source.trim().is_empty() {
        return Err(GeoError::MissingRef { entry: entry.key() });
    }
    Ok(source)
}

/// A signed micro-degree count written as text, e.g. `-2549940`.
fn micro<'a>(entry: &Entry<'a>, key: &'a str) -> Result<i64, GeoError<'a>> {
    let raw = text(entry, key)?;
    signed(raw).ok_or_else(|| GeoError::BadValue {
        entry: entry.key(),
        key,
        value: raw,
    })
}

/// A signed whole number written as text, e.g. `-30`.
fn signed(text: &str) -> Option<i64> {
    let body = text.strip_prefix('-').unwrap_or(text);
    if body.is_empty() || !body.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = body.parse::<i64>().ok()?;
    Some(if text.starts_with('-') { -value } else { value })
}

/// The millimetres between two points in a frame.
///
/// A degree is a million micro-degrees and a metre is a thousand millimetres,
/// so the scale of a frame divides by a thousand. The squares are taken in
/// `u128`, so a distance across the whole globe still fits.
fn distance_mm(a: Point, b: Point, m_per_deg_lat: u32, m_per_deg_lon: u32) -> u64 {
    let north = u128::from(a.lat_micro.abs_diff(b.lat_micro)) * u128::from(m_per_deg_lat) / 1000;
    let east = u128::from(a.lon_micro.abs_diff(b.lon_micro)) * u128::from(m_per_deg_lon) / 1000;
    let sum = north * north + east * east;
    u64::try_from(isqrt(sum)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_places_table_loads_and_every_entry_names_its_source() {
        let places = Places::shipped().expect("shipped places table");
        assert_eq!(places.areas().len(), 6, "the six chapter areas");
        assert!(!places.places().is_empty(), "the gazetteer slice");
        for area in places.areas() {
            assert!(!area.source().is_empty(), "the area {}", area.key());
        }
        for place in places.places() {
            assert!(!place.source().is_empty(), "the place {}", place.key());
        }
        for source in places.sources() {
            assert!(!source.licence.is_empty(), "the donor {}", source.id);
            assert_eq!(source.sha256.len(), 64, "the sha256 of {}", source.id);
        }
    }

    #[test]
    fn an_entry_without_a_ref_is_refused_by_name() {
        // The ref line is cut out, not the line ending, so the test reads the
        // same on a tree with LF and on one with CRLF.
        let text = Places::SHIPPED.replace(
            "ref = \"urbesetorbis:301023_AD130.csv row 503, the gazetteer of the settlements under Hadrian (BSD-2-Clause)\"",
            "",
        );
        assert!(text.len() < Places::SHIPPED.len(), "the ref is gone");
        let error = Places::parse(&text).expect_err("a place with no ref");
        assert!(error.to_string().contains("place.drobeta"), "{error}");
    }

    #[test]
    fn a_point_in_the_danube_region_finds_the_nearest_settlement() {
        let places = Places::shipped().expect("shipped places table");
        let point = Point {
            lon_micro: 22_680_000,
            lat_micro: 44_650_000,
        };
        let nearest = places.nearest("danube", point).expect("the danube area");
        assert_eq!(nearest.place().name(), "Drobeta");
        assert!(
            nearest.distance_mm() <= 5_000_000,
            "{} mm from the point",
            nearest.distance_mm()
        );
        assert!(
            nearest.distance_mm() >= 1_000_000,
            "the point is not the settlement itself"
        );
    }

    #[test]
    fn every_place_lies_in_the_box_of_its_area() {
        let places = Places::shipped().expect("shipped places table");
        for place in places.places() {
            let area = places.area(place.area()).expect("the area of the place");
            assert!(
                area.contains(place.point()),
                "{} lies outside {}",
                place.key(),
                area.key()
            );
        }
        let danube = places.in_area("danube").count();
        assert_eq!(danube, 6, "the settlements of the Danube box");
    }

    #[test]
    fn a_place_outside_the_box_of_its_area_is_refused() {
        let text = Places::SHIPPED.replace("lat = \"44629271\"", "lat = \"45000000\"");
        let error = Places::parse(&text).expect_err("a place outside its box");
        assert!(error.to_string().contains("place.drobeta"), "{error}");
    }

    #[test]
    fn a_place_that_names_an_unknown_area_is_refused() {
        let text = Places::SHIPPED.replace("area = \"danube\"", "area = \"nowhere\"");
        let error = Places::parse(&text).expect_err("a place with no area");
        assert!(error.to_string().contains("nowhere"), "{error}");
    }

    #[test]
    fn the_limes_segments_carry_their_names() {
        let frontier = Fortifications::shipped().expect("shipped fortifications table");
        let first = frontier
            .named("Limes Germania superior - Pars I")
            .expect("the first stretch of the limes");
        assert_eq!(first.kind(), SegmentKind::Limes);
        assert!(first.points().len() > 100, "the stretch keeps its points");
        assert!(
            frontier.named("FL. MOENVS (Main)").is_some(),
            "the Main is a named river"
        );
        assert!(
            frontier.named("FL. NICER (Neckar)").is_some(),
            "the Neckar is a named river"
        );
        assert!(
            frontier.named("Wall Mile 46").is_some(),
            "the wall keeps its mile markers"
        );
        for segment in frontier.segments() {
            assert!(!segment.source().is_empty(), "the line {}", segment.key());
        }
        assert_eq!(
            frontier.of_kind(SegmentKind::River).count(),
            2,
            "the Main and the Neckar"
        );
    }

    #[test]
    fn the_frontier_length_is_the_sum_of_its_segments() {
        let frontier = Fortifications::shipped().expect("shipped fortifications table");
        let total: u64 = frontier.segments().iter().map(|s| s.length_mm()).sum();
        assert_eq!(frontier.length_mm(), total);
        assert!(
            frontier.length_mm() > 100_000_000,
            "the frontier is longer than 100 km"
        );
    }

    #[test]
    fn two_loads_hash_the_same() {
        let places = Places::shipped().expect("shipped places table");
        assert_eq!(
            places.hash(),
            Places::shipped().expect("shipped places table").hash()
        );
        let frontier = Fortifications::shipped().expect("shipped fortifications table");
        assert_eq!(
            frontier.hash(),
            Fortifications::shipped()
                .expect("shipped fortifications table")
                .hash()
        );
        assert_ne!(
            places.hash(),
            frontier.hash(),
            "the two tables hash under different tags"
        );
    }
}
