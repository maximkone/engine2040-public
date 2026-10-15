//! Arena-knob learned-rules verify vectors.
//!
//! Consumer: packaging and quality review over the arena-knob fixture.
//! Data owner: this test owns its strict knob parser; the fixture
//! `fixtures/games/arena_knobs.json` owns the six canonical values.
//! Units: counts are integers, extents and camera rows are meters.
//! Determinism scope: same rows give the same accept/refuse answer, no IO.
//! Errors: `KnobError` names the culprit row; missing or unknown refuses.
//!
//! Idea port from `ConfigV/ConfigV` (BSD-3-Clause): learned-rules verify
//! refuses unread or unknown rules instead of silently defaulting.
//! Reimplemented fresh here for arena knobs; no bytes copied.

/// The six knobs in `fixtures/games/arena_knobs.json`.
const KNOWN_KNOBS: [&str; 6] = [
    "agents",
    "enemies",
    "half_extent_m",
    "kill_target",
    "camera_height_m",
    "camera_lead_m",
];

/// Verified arena knobs: counts plus meters. Plain data.
#[derive(Clone, Copy, PartialEq, Debug)]
struct ArenaKnobs {
    agents: u32,
    enemies: u32,
    half_extent_m: f32,
    kill_target: u32,
    camera_height_m: f32,
    camera_lead_m: f32,
}

/// Knob verify failure: which row refused and why. Typed, no strings built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KnobError {
    /// Row name not one of the six known knobs.
    Unknown,
    /// Known knob absent: no silent default fills it.
    Missing(&'static str),
    /// Known knob present but outside its range or not a number.
    BadValue(&'static str),
}

impl core::fmt::Display for KnobError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown knob"),
            Self::Missing(row) => write!(f, "missing knob ({row})"),
            Self::BadValue(row) => write!(f, "bad knob value ({row})"),
        }
    }
}

impl std::error::Error for KnobError {}

/// True when the text is non-empty ASCII digits only. Pure.
fn is_digits(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit())
}

/// Look up one named row; `None` when absent. Pure.
fn row_of<'a>(rows: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    rows.iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
}

/// Parse one count row (`1..=16` for agents/enemies). Pure, never panics.
fn parse_count(rows: &[(&str, &str)], key: &'static str) -> Result<u32, KnobError> {
    let raw = row_of(rows, key).ok_or(KnobError::Missing(key))?;
    if !is_digits(raw) {
        return Err(KnobError::BadValue(key));
    }
    let value: u32 = raw.parse().map_err(|_| KnobError::BadValue(key))?;
    if !(1..=16).contains(&value) {
        return Err(KnobError::BadValue(key));
    }
    Ok(value)
}

/// Strict verify of arena-knob rows: unknown keys refuse, every known knob
/// must be present (unread refuses, never defaults), then each value faces
/// its range check. Pure, never panics.
fn verify_knobs(rows: &[(&str, &str)]) -> Result<ArenaKnobs, KnobError> {
    for entry in rows {
        if !KNOWN_KNOBS.contains(&entry.0) {
            return Err(KnobError::Unknown);
        }
    }
    let agents = parse_count(rows, "agents")?;
    let enemies = parse_count(rows, "enemies")?;
    let half_raw = row_of(rows, "half_extent_m").ok_or(KnobError::Missing("half_extent_m"))?;
    let half_extent_m: f32 = half_raw
        .parse()
        .map_err(|_| KnobError::BadValue("half_extent_m"))?;
    if !half_extent_m.is_finite() || !(5.0..=100.0).contains(&half_extent_m) {
        return Err(KnobError::BadValue("half_extent_m"));
    }
    let kill_raw = row_of(rows, "kill_target").ok_or(KnobError::Missing("kill_target"))?;
    if !is_digits(kill_raw) {
        return Err(KnobError::BadValue("kill_target"));
    }
    let kill_target: u32 = kill_raw
        .parse()
        .map_err(|_| KnobError::BadValue("kill_target"))?;
    if !(1..=32).contains(&kill_target) {
        return Err(KnobError::BadValue("kill_target"));
    }
    let height_raw =
        row_of(rows, "camera_height_m").ok_or(KnobError::Missing("camera_height_m"))?;
    let camera_height_m: f32 = height_raw
        .parse()
        .map_err(|_| KnobError::BadValue("camera_height_m"))?;
    if !camera_height_m.is_finite() || !(2.0..=50.0).contains(&camera_height_m) {
        return Err(KnobError::BadValue("camera_height_m"));
    }
    let lead_raw = row_of(rows, "camera_lead_m").ok_or(KnobError::Missing("camera_lead_m"))?;
    let camera_lead_m: f32 = lead_raw
        .parse()
        .map_err(|_| KnobError::BadValue("camera_lead_m"))?;
    if !camera_lead_m.is_finite() || !(0.0..=10.0).contains(&camera_lead_m) {
        return Err(KnobError::BadValue("camera_lead_m"));
    }
    Ok(ArenaKnobs {
        agents,
        enemies,
        half_extent_m,
        kill_target,
        camera_height_m,
        camera_lead_m,
    })
}

/// Canonical fixture rows: the six values in `fixtures/games/arena_knobs.json`.
fn fixture_rows() -> [(&'static str, &'static str); 6] {
    [
        ("agents", "2"),
        ("enemies", "3"),
        ("half_extent_m", "16"),
        ("kill_target", "2"),
        ("camera_height_m", "14"),
        ("camera_lead_m", "1.5"),
    ]
}

#[test]
fn valid_fixture_6_of_6_accepted() {
    let rows = fixture_rows();
    assert_eq!(
        verify_knobs(&rows).expect("fixture accepts"),
        ArenaKnobs {
            agents: 2,
            enemies: 3,
            half_extent_m: 16.0,
            kill_target: 2,
            camera_height_m: 14.0,
            camera_lead_m: 1.5,
        }
    );
}

#[test]
fn unread_4_of_4_refused_no_default() {
    // Each of the four learned knobs refuses when unread; no default fills it.
    let gaps: [&'static str; 4] = ["enemies", "kill_target", "camera_height_m", "camera_lead_m"];
    for missing in gaps {
        let kept: Vec<(&str, &str)> = fixture_rows()
            .into_iter()
            .filter(|entry| entry.0 != missing)
            .collect();
        assert_eq!(
            verify_knobs(&kept),
            Err(KnobError::Missing(missing)),
            "unread {missing} must refuse"
        );
    }
    // Empty input refuses too: nothing silently defaults.
    assert_eq!(
        verify_knobs(&[]),
        Err(KnobError::Missing("agents")),
        "empty rows must refuse"
    );
}

#[test]
fn unknown_knob_refused() {
    let mut rows = fixture_rows().to_vec();
    rows.push(("mode", "arena"));
    assert_eq!(verify_knobs(&rows), Err(KnobError::Unknown));
    // A bad value in a known knob refuses as well.
    let bad = [
        ("agents", "2"),
        ("enemies", "0"),
        ("half_extent_m", "16"),
        ("kill_target", "2"),
        ("camera_height_m", "14"),
        ("camera_lead_m", "1.5"),
    ];
    assert_eq!(
        verify_knobs(&bad),
        Err(KnobError::BadValue("enemies")),
        "out-of-range enemies must refuse"
    );
}
