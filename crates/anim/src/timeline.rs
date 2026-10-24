//! Cutscene keyframe document: declared tracks, keys with a tick, a pose value
//! and an easing word, compiled onto the same SoA clip the playback samples.
//!
//! [`Timeline::compile`] validates the document and bakes it with
//! [`bake_clip`], so playing a scripted timeline is the ordinary clip sampler
//! and nothing new runs at playback time. Ticks are integer ticks at the engine
//! tick rate; a key on a track the document does not declare is refused by
//! name, and a track's keys must be non-decreasing in tick. Std-only: the JSON
//! shape lives with the door, which parses into these plain types.

use crate::clip::{bake_clip, bake_track, qclip_hash, sample_all, Cursor, QClip};
use crate::pose::lerp3;

/// Seconds of one tick, owned by the clip module.
pub const TICK_DT: f32 = crate::clip::TICK_DT;

/// Largest tick a key may name, so a document cannot build a huge clip.
pub const MAX_TICK: u32 = 60 * 60 * 10;

/// Most tracks one timeline may declare.
pub const MAX_TRACKS: usize = 64;

/// Most keys one timeline may hold.
pub const MAX_KEYS: usize = 4096;

/// How a segment reaches the next key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    /// Straight line to the next key.
    #[default]
    Linear,
    /// Hold this key's value until the next key's tick.
    Step,
    /// Slow start, fast finish.
    EaseIn,
    /// Fast start, slow finish.
    EaseOut,
    /// Slow at both ends.
    EaseInOut,
}

impl Easing {
    /// The wire word of this easing.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Step => "step",
            Self::EaseIn => "ease_in",
            Self::EaseOut => "ease_out",
            Self::EaseInOut => "ease_in_out",
        }
    }

    /// Parse a wire word; `None` for an unknown word.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "linear" => Some(Self::Linear),
            "step" => Some(Self::Step),
            "ease_in" => Some(Self::EaseIn),
            "ease_out" => Some(Self::EaseOut),
            "ease_in_out" => Some(Self::EaseInOut),
            _ => None,
        }
    }

    /// Sub-samples baked into a segment so the clip sampler follows the curve.
    /// `Linear` and `Step` bake no curve; they bake their own control points.
    fn steps(self) -> usize {
        match self {
            Self::Linear | Self::Step => 0,
            Self::EaseIn | Self::EaseOut | Self::EaseInOut => 4,
        }
    }

    /// Eased weight in `0..=1` for a linear segment weight `u`.
    fn weight(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Linear | Self::Step => u,
            Self::EaseIn => u * u,
            Self::EaseOut => 1.0 - (1.0 - u) * (1.0 - u),
            Self::EaseInOut => {
                if u <= 0.5 {
                    2.0 * u * u
                } else {
                    1.0 - 2.0 * (1.0 - u) * (1.0 - u)
                }
            }
        }
    }
}

/// One keyframe: which track, on which tick, with which pose value.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    /// Track name, which the document must declare in `tracks`.
    pub track: String,
    /// Tick this key lands on.
    pub tick: u32,
    /// Three-value pose (meters or degrees, per track convention).
    pub value: [f32; 3],
    /// How the segment after this key reaches the next one.
    pub easing: Easing,
}

/// The authoring document: declared tracks plus their keys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimelineDoc {
    /// Track names in sample order; a key may only name one of these.
    pub tracks: Vec<String>,
    /// Keys of every track, in authoring order.
    pub keys: Vec<Key>,
}

/// Why a timeline document was refused. Every case names the culprit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineError {
    /// The document declares no track.
    NoTracks,
    /// The document declares no key.
    NoKeys,
    /// Two tracks share a name.
    DuplicateTrack {
        /// The repeated name.
        name: String,
    },
    /// A key names a track the document does not declare.
    UnknownTrack {
        /// The name the key carried.
        name: String,
    },
    /// A key's tick is lower than the previous key on the same track.
    NonMonotonic {
        /// Track whose keys went backwards.
        track: String,
        /// Tick of the earlier key.
        previous: u32,
        /// Tick of the offending key.
        tick: u32,
    },
    /// A tick is past [`MAX_TICK`].
    TickTooLarge {
        /// The offending tick.
        tick: u32,
    },
    /// The document holds more than [`MAX_TRACKS`] tracks.
    TooManyTracks {
        /// Tracks found.
        found: usize,
    },
    /// The document holds more than [`MAX_KEYS`] keys.
    TooManyKeys {
        /// Keys found.
        found: usize,
    },
}

impl TimelineError {
    /// Machine code the door reports for this refusal.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoTracks
            | Self::NoKeys
            | Self::TooManyTracks { .. }
            | Self::TooManyKeys { .. } => "bad_timeline",
            Self::DuplicateTrack { .. } => "duplicate_track",
            Self::UnknownTrack { .. } => "unknown_track",
            Self::NonMonotonic { .. } => "non_monotonic",
            Self::TickTooLarge { .. } => "bad_tick",
        }
    }

    /// One sentence naming the culprit and the recovery.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoTracks => "the timeline declares no track; name at least one".to_owned(),
            Self::NoKeys => "the timeline holds no key; add one key per track".to_owned(),
            Self::DuplicateTrack { name } => {
                format!("track `{name}` is declared twice; name each track once")
            }
            Self::UnknownTrack { name } => format!(
                "a key names track `{name}`, which the document does not declare; \
                 list it in `tracks` or drop the key"
            ),
            Self::NonMonotonic {
                track,
                previous,
                tick,
            } => format!(
                "track `{track}` has key tick {tick} after tick {previous}; keys must be \
                 non-decreasing per track"
            ),
            Self::TickTooLarge { tick } => {
                format!("key tick {tick} is past the cap {MAX_TICK}")
            }
            Self::TooManyTracks { found } => {
                format!("{found} tracks is past the cap {MAX_TRACKS}")
            }
            Self::TooManyKeys { found } => format!("{found} keys is past the cap {MAX_KEYS}"),
        }
    }
}

/// A validated, compiled timeline: the clip the playback samples.
#[derive(Debug, Clone)]
pub struct Timeline {
    clip: QClip,
    tracks: Vec<String>,
    keys: usize,
    dur_ticks: u32,
}

/// `tick` as seconds, saturating past the `u16` range (past [`MAX_TICK`]).
fn tick_secs(tick: u32) -> f32 {
    f32::from(u16::try_from(tick).unwrap_or(u16::MAX)) * TICK_DT
}

impl Timeline {
    /// Validate a document and bake it into the clip the playback samples.
    pub fn compile(doc: &TimelineDoc) -> Result<Self, TimelineError> {
        if doc.tracks.is_empty() {
            return Err(TimelineError::NoTracks);
        }
        if doc.keys.is_empty() {
            return Err(TimelineError::NoKeys);
        }
        if doc.tracks.len() > MAX_TRACKS {
            return Err(TimelineError::TooManyTracks {
                found: doc.tracks.len(),
            });
        }
        if doc.keys.len() > MAX_KEYS {
            return Err(TimelineError::TooManyKeys {
                found: doc.keys.len(),
            });
        }
        let mut declared: Vec<&str> = Vec::with_capacity(doc.tracks.len());
        for name in &doc.tracks {
            if declared.contains(&name.as_str()) {
                return Err(TimelineError::DuplicateTrack { name: name.clone() });
            }
            declared.push(name);
        }
        let mut per_track: Vec<Vec<&Key>> = vec![Vec::new(); doc.tracks.len()];
        for key in &doc.keys {
            if key.tick > MAX_TICK {
                return Err(TimelineError::TickTooLarge { tick: key.tick });
            }
            let Some(index) = doc.tracks.iter().position(|name| name == &key.track) else {
                return Err(TimelineError::UnknownTrack {
                    name: key.track.clone(),
                });
            };
            if let Some(previous) = per_track[index].last() {
                if key.tick < previous.tick {
                    return Err(TimelineError::NonMonotonic {
                        track: key.track.clone(),
                        previous: previous.tick,
                        tick: key.tick,
                    });
                }
            }
            per_track[index].push(key);
        }
        let mut baked = Vec::with_capacity(doc.tracks.len());
        let mut dur_ticks = 0u32;
        for keys in &per_track {
            let (times, values) = track_points(keys);
            if let Some(last) = keys.last() {
                dur_ticks = dur_ticks.max(last.tick);
            }
            baked.push(bake_track(times, values));
        }
        Ok(Self {
            clip: bake_clip(baked),
            tracks: doc.tracks.clone(),
            keys: doc.keys.len(),
            dur_ticks,
        })
    }

    /// The compiled clip, as the playback holds it.
    #[must_use]
    pub fn clip(&self) -> &QClip {
        &self.clip
    }

    /// Declared track names in sample order.
    #[must_use]
    pub fn tracks(&self) -> &[String] {
        &self.tracks
    }

    /// Keys the document held.
    #[must_use]
    pub fn keys(&self) -> usize {
        self.keys
    }

    /// Last key tick.
    #[must_use]
    pub fn dur_ticks(&self) -> u32 {
        self.dur_ticks
    }

    /// Stable digest of the compiled clip: equal for equal documents.
    #[must_use]
    pub fn digest(&self) -> u64 {
        qclip_hash(&self.clip)
    }

    /// Sample every track at one tick, through the clip sampler the playback
    /// uses, so a timeline and a hand-built clip of the same keys agree.
    #[must_use]
    pub fn sample(&self, tick: u32) -> Vec<[f32; 3]> {
        let count = self.clip.tracks.len();
        let mut cursor = Cursor::new(count);
        let mut out = vec![[0.0; 3]; count];
        sample_all(&self.clip, tick_secs(tick), &mut cursor, &mut out);
        out
    }
}

/// Clip times and values of one track, with each segment's easing applied.
fn track_points(keys: &[&Key]) -> (Vec<f32>, Vec<[f32; 3]>) {
    let mut times = Vec::with_capacity(keys.len());
    let mut values = Vec::with_capacity(keys.len());
    if let Some(first) = keys.first() {
        times.push(tick_secs(first.tick));
        values.push(first.value);
    }
    for pair in keys.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        for (tick, value) in segment_points(left, right) {
            times.push(tick_secs(tick));
            values.push(value);
        }
    }
    (times, values)
}

/// The points one segment contributes after its left key: its own eased
/// sub-samples, or the single point the next key lands on when the segment is
/// a straight line (or a hold).
fn segment_points(left: &Key, right: &Key) -> Vec<(u32, [f32; 3])> {
    let steps = left.easing.steps();
    if steps == 0 {
        if left.easing == Easing::Step && right.tick > left.tick + 1 {
            return vec![(right.tick - 1, left.value), (right.tick, right.value)];
        }
        return vec![(right.tick, right.value)];
    }
    let span = right.tick.saturating_sub(left.tick);
    let divisor = u32::try_from(steps).unwrap_or(1);
    let mut points = Vec::with_capacity(steps);
    for step in 1..=steps {
        let step_u32 = u32::try_from(step).unwrap_or(1);
        let fraction = f32::from(u16::try_from(step_u32).unwrap_or(1))
            / f32::from(u16::try_from(divisor).unwrap_or(1));
        if step == steps {
            points.push((right.tick, right.value));
            continue;
        }
        let weight = left.easing.weight(fraction);
        points.push((
            left.tick + span * step_u32 / divisor,
            lerp3(left.value, right.value, weight),
        ));
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(track: &str, tick: u32, value: [f32; 3]) -> Key {
        Key {
            track: track.to_owned(),
            tick,
            value,
            easing: Easing::Linear,
        }
    }

    fn two_track() -> TimelineDoc {
        TimelineDoc {
            tracks: vec!["hips".to_owned(), "head".to_owned()],
            keys: vec![
                key("hips", 0, [0.0, 1.0, 0.0]),
                key("hips", 30, [0.0, 2.0, 0.0]),
                key("head", 15, [10.0, 0.0, 0.0]),
                key("head", 45, [20.0, 0.0, 0.0]),
            ],
        }
    }

    /// F2P: a two-track timeline samples as the hand-built clip does.
    #[test]
    fn timeline_samples_match_the_hand_built_clip() {
        let timeline = Timeline::compile(&two_track()).expect("timeline compiles");
        assert_eq!(timeline.tracks().len(), 2);
        assert_eq!(timeline.keys(), 4);
        assert_eq!(timeline.dur_ticks(), 45);
        // The same keys baked by hand, from the same tick seconds.
        let hand = bake_clip(vec![
            bake_track(
                vec![tick_secs(0), tick_secs(30)],
                vec![[0.0, 1.0, 0.0], [0.0, 2.0, 0.0]],
            ),
            bake_track(
                vec![tick_secs(15), tick_secs(45)],
                vec![[10.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
            ),
        ]);
        let mut cursor = Cursor::new(2);
        let mut want = [[0.0f32; 3]; 2];
        for tick in [0u32, 7, 15, 22, 30, 44] {
            let got = timeline.sample(tick);
            sample_all(&hand, tick_secs(tick), &mut cursor, &mut want);
            for track in 0..2 {
                for axis in 0..3 {
                    assert!(
                        (got[track][axis] - want[track][axis]).abs() <= 1e-5,
                        "tick {tick} track {track} axis {axis}: {} vs {}",
                        got[track][axis],
                        want[track][axis]
                    );
                }
            }
        }
        assert_eq!(
            timeline.digest(),
            qclip_hash(&hand),
            "digest is the compiled clip's own hash"
        );
    }

    #[test]
    fn unknown_track_is_refused_by_name() {
        let mut doc = two_track();
        doc.keys[2].track = "tail".to_owned();
        let error = Timeline::compile(&doc).expect_err("unknown track refuses");
        assert_eq!(error.code(), "unknown_track");
        assert!(error.message().contains("tail"), "{}", error.message());
    }

    #[test]
    fn keys_must_be_monotonic_per_track() {
        let mut doc = two_track();
        doc.keys[0].tick = 40;
        let error = Timeline::compile(&doc).expect_err("backwards key refuses");
        assert_eq!(error.code(), "non_monotonic");
        assert!(error.message().contains("hips"), "{}", error.message());
        let mut split = two_track();
        split.keys[2].tick = 0;
        assert!(Timeline::compile(&split).is_ok(), "tracks stay independent");
    }

    #[test]
    fn easing_shapes_the_segment_and_step_holds() {
        let mut doc = two_track();
        doc.keys[0].easing = Easing::EaseIn;
        let eased = Timeline::compile(&doc).expect("eased compiles");
        let mid = eased.sample(15)[0][1];
        assert!(mid < 1.5, "ease-in lags the line at the midpoint: {mid}");
        assert!(
            (eased.sample(30)[0][1] - 2.0).abs() <= 1e-5,
            "keys land exact"
        );
        doc.keys[0].easing = Easing::Step;
        let step = Timeline::compile(&doc).expect("step compiles");
        assert!((step.sample(15)[0][1] - 1.0).abs() <= 1e-5, "step holds");
        assert!(
            (step.sample(30)[0][1] - 2.0).abs() <= 1e-5,
            "next key lands"
        );
    }

    #[test]
    fn empty_and_duplicate_documents_are_refused() {
        let mut doc = TimelineDoc::default();
        assert_eq!(
            Timeline::compile(&doc).expect_err("no tracks").code(),
            "bad_timeline"
        );
        doc.tracks.push("hips".to_owned());
        assert_eq!(
            Timeline::compile(&doc).expect_err("no keys").code(),
            "bad_timeline"
        );
        doc.tracks.push("hips".to_owned());
        doc.keys.push(key("hips", 0, [0.0; 3]));
        assert_eq!(
            Timeline::compile(&doc).expect_err("duplicate").code(),
            "duplicate_track"
        );
    }
}
