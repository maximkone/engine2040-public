//! loco_bake: locomotion set baked into the motion-matching database.
//!
//! Consumer: [`motion_match::search`] reads the baked [`MotionDb`]; arena
//! locomotion and capture tooling consume the picks. Data owner:
//! `engine-anim`. Units: clips, frames (one DB entry per frame at 60 Hz,
//! every clip on one hero skeleton). Determinism: synthesis is index
//! arithmetic plus the integer motion-matching path, so the same spec table
//! bakes the same database twice. Errors are [`BakeError`].
//!
//! Sources (no external bytes fetched here): Quaternius CC0 clips and 100STYLE
//! CC BY 4.0 motions, authored synthetically in-code in Quaternius style and
//! marked synthetic. Non-commercial licenses are refused, never baked.

use super::locomotion::{contacts_for_phase, loco_skeleton};
use super::motion_bake::{bake_foot_weights, FootWeights, FOOT_TICK_HZ};
use super::motion_match::{
    bake as bake_db, db_hash, make_feature, predict_linear, Entry, MatchError, MotionDb, PoseSample,
};
use engine_core::Fnv64;

/// Max chars per clip name, source id or tag; longer input is refused.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest single clip baked (60 s at 60 Hz); longer specs are refused.
pub const MAX_CLIP_FRAMES: u16 = 3600;
/// Fastest hip speed baked (m/s); faster specs are refused, never clamped.
pub const MAX_BAKE_SPEED: f32 = 12.0;
/// Widest foot swing baked (meters); wider specs are refused.
pub const MAX_SWING: f32 = 1.0;
/// Every baked set must carry all eight locomotion tags.
pub const REQUIRED_TAGS: [&str; 8] = [
    "idle", "walk", "run", "start", "stop", "pivot", "strafe", "turn",
];

/// A licensable clip source: registry id, canonical URL and ship license.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SourceInfo {
    /// Registry id named by [`ClipSpec::source`].
    pub id: &'static str,
    /// Canonical source URL for the provenance row.
    pub url: &'static str,
    /// The only license this source ships under.
    pub license: &'static str,
    /// Attribution and exclusion notes.
    pub note: &'static str,
}

/// Quaternius Universal Animation Library plus base characters (CC0).
pub const QUATERNIUS: SourceInfo = SourceInfo {
    id: "quaternius",
    url: "https://quaternius.com/",
    license: "CC0-1.0",
    note: "Universal Animation Library humanoid clips; synthetic stand-ins here, no donor bytes.",
};

/// 100STYLE styled locomotion (Mason, Starke, Komura 2022), motions only.
pub const STYLE100: SourceInfo = SourceInfo {
    id: "100style",
    url: "https://ianxmason.github.io/100style/",
    license: "CC-BY-4.0",
    note: "Motions only with credit; the Geno.fbx mesh is non-commercial and never ships.",
};

/// OGA "Modular RPG Characters" by System G6 (CC0): the CT7 fighter clips.
pub const SYSTEM_G6: SourceInfo = SourceInfo {
    id: "system_g6",
    url: "https://opengameart.org/content/modular-rpg-characters",
    license: "CC0-1.0",
    note: "CT7 fighter body and clips; synthetic attack stand-ins here, no donor bytes.",
};

/// Look up a source by registry id; unknown ids are refused, never guessed.
pub fn source_by_id(id: &str) -> Result<&'static SourceInfo, BakeError> {
    match id {
        "quaternius" => Ok(&QUATERNIUS),
        "100style" => Ok(&STYLE100),
        "system_g6" => Ok(&SYSTEM_G6),
        _ => Err(BakeError::UnknownSource { name: bound(id) }),
    }
}

/// Why a locomotion bake failed; unlicensed and unknown sources are refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BakeError {
    /// A spec names a source outside the registry; the name is carried.
    UnknownSource {
        /// Truncated source id.
        name: String,
    },
    /// A spec carries a non-shippable or mismatched license; clip carried.
    Unlicensed {
        /// Truncated clip name.
        name: String,
    },
    /// A spec field is out of bounds; clip plus field carried.
    BadSpec {
        /// Truncated clip name, empty for whole-set failures.
        clip: String,
        /// Offending field.
        reason: &'static str,
    },
    /// Feature synthesis hit non-finite math; the lane names the problem.
    BadFeature(&'static str),
    /// The database bake rejected the entries; the lane names the problem.
    BadDatabase(&'static str),
}

impl core::fmt::Display for BakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BakeError::UnknownSource { name } => {
                write!(f, "locomotion bake: unknown source ({name})")
            }
            BakeError::Unlicensed { name } => {
                write!(f, "locomotion bake: unlicensed clip ({name})")
            }
            BakeError::BadSpec { clip, reason } => {
                write!(f, "locomotion bake: bad spec ({clip}: {reason})")
            }
            BakeError::BadFeature(lane) => {
                write!(f, "locomotion bake: bad feature ({lane})")
            }
            BakeError::BadDatabase(lane) => {
                write!(f, "locomotion bake: bad database ({lane})")
            }
        }
    }
}

impl std::error::Error for BakeError {}

impl From<MatchError> for BakeError {
    fn from(cause: MatchError) -> Self {
        match cause {
            MatchError::BadQuery(lane) => BakeError::BadFeature(lane),
            MatchError::BadDatabase(lane) => BakeError::BadDatabase(lane),
            MatchError::EmptyDatabase => BakeError::BadDatabase("empty"),
            MatchError::BadNetDims { .. } | MatchError::Net(_) => {
                BakeError::BadDatabase("trajectory net")
            }
        }
    }
}

/// Truncate a diagnostic to [`MAX_NAME_CHARS`] chars.
pub(crate) fn bound(text: &str) -> String {
    if text.chars().count() <= MAX_NAME_CHARS {
        text.to_string()
    } else {
        text.chars().take(MAX_NAME_CHARS).collect()
    }
}

/// Token charset shared with the clip sidecar schema: ASCII alphanumerics
/// plus `_`, no leading digit, within [`MAX_NAME_CHARS`].
pub(crate) fn valid_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME_CHARS
        && !text.as_bytes()[0].is_ascii_digit()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// One clip to bake: provenance plus a deterministic gait recipe. Speeds ramp
/// linearly from `speed0` to `speed1` across the frames; `lateral` steps along
/// `z` (strafe) instead of `x` (forward travel).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClipSpec {
    /// Clip name, sidecar token charset.
    pub name: &'static str,
    /// Registry source id (`quaternius` or `100style`).
    pub source: &'static str,
    /// Ship license; must equal the source registry license.
    pub license: &'static str,
    /// Locomotion tags (`idle`, `walk`, ...), sidecar token charset.
    pub tags: &'static [&'static str],
    /// Frame count at 60 Hz, one DB entry per frame.
    pub frames: u16,
    /// Hip speed (m/s) at the first frame.
    pub speed0: f32,
    /// Hip speed (m/s) at the last frame.
    pub speed1: f32,
    /// Foot swing amplitude (meters).
    pub swing: f32,
    /// True for lateral side-steps, false for forward travel.
    pub lateral: bool,
    /// True when the clip tiles seamlessly.
    pub looped: bool,
    /// True when the clip drives root motion.
    pub root_motion: bool,
    /// True for in-code synthetic stand-ins (no donor bytes).
    pub synthetic: bool,
}

/// The hero locomotion set: eight clips covering every [`REQUIRED_TAGS`] entry,
/// all synthetic Quaternius-style stand-ins on the one hero skeleton.
pub const HERO_SET: [ClipSpec; 8] = [
    ClipSpec {
        name: "idle_stand",
        source: "quaternius",
        license: "CC0-1.0",
        tags: &["idle"],
        frames: 60,
        speed0: 0.0,
        speed1: 0.0,
        swing: 0.02,
        lateral: false,
        looped: true,
        root_motion: false,
        synthetic: true,
    },
    ClipSpec {
        name: "walk_forward",
        source: "quaternius",
        license: "CC0-1.0",
        tags: &["walk"],
        frames: 60,
        speed0: 1.5,
        speed1: 1.5,
        swing: 0.25,
        lateral: false,
        looped: true,
        root_motion: true,
        synthetic: true,
    },
    ClipSpec {
        name: "run_forward",
        source: "quaternius",
        license: "CC0-1.0",
        tags: &["run"],
        frames: 60,
        speed0: 3.5,
        speed1: 3.5,
        swing: 0.35,
        lateral: false,
        looped: true,
        root_motion: true,
        synthetic: true,
    },
    ClipSpec {
        name: "start_walk",
        source: "100style",
        license: "CC-BY-4.0",
        tags: &["start"],
        frames: 30,
        speed0: 0.0,
        speed1: 1.5,
        swing: 0.25,
        lateral: false,
        looped: false,
        root_motion: true,
        synthetic: true,
    },
    ClipSpec {
        name: "stop_walk",
        source: "100style",
        license: "CC-BY-4.0",
        tags: &["stop"],
        frames: 30,
        speed0: 1.5,
        speed1: 0.0,
        swing: 0.25,
        lateral: false,
        looped: false,
        root_motion: true,
        synthetic: true,
    },
    ClipSpec {
        name: "pivot_turn",
        source: "100style",
        license: "CC-BY-4.0",
        tags: &["pivot"],
        frames: 30,
        speed0: 1.2,
        speed1: 1.2,
        swing: 0.2,
        lateral: false,
        looped: false,
        root_motion: true,
        synthetic: true,
    },
    ClipSpec {
        name: "strafe_left",
        source: "quaternius",
        license: "CC0-1.0",
        tags: &["strafe"],
        frames: 60,
        speed0: 0.0,
        speed1: 1.2,
        swing: 0.2,
        lateral: true,
        looped: true,
        root_motion: true,
        synthetic: true,
    },
    ClipSpec {
        name: "turn_in_place",
        source: "quaternius",
        license: "CC0-1.0",
        tags: &["turn"],
        frames: 30,
        speed0: 0.4,
        speed1: 0.4,
        swing: 0.1,
        lateral: false,
        looped: false,
        root_motion: false,
        synthetic: true,
    },
];

/// One baked clip: provenance plus its stable digest over its entries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BakedClip {
    /// Clip name.
    pub name: &'static str,
    /// Registry source (URL plus ship license).
    pub source: &'static SourceInfo,
    /// Locomotion tags.
    pub tags: &'static [&'static str],
    /// Frame count; the clip owns entries `[base, base + frames)`.
    pub frames: u16,
    /// True when the clip tiles seamlessly.
    pub looped: bool,
    /// True when the clip drives root motion.
    pub root_motion: bool,
    /// True for in-code synthetic stand-ins.
    pub synthetic: bool,
    /// The clip's planted-foot lane at 30 Hz (SW25): the permille of each window each foot
    /// spent down, so the IK starts from a per-clip plant instead of a live contact solve.
    pub ik_weight: FootWeights,
    /// Stable digest over the clip name plus its entry features.
    pub digest: u64,
}

/// A baked locomotion set: the motion database plus per-clip provenance.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocoSet {
    /// Entries in canonical `(clip, tick)` order, one per frame.
    pub db: MotionDb,
    /// Per-clip provenance in bake order.
    pub clips: Vec<BakedClip>,
    /// Joint count of the one hero skeleton every clip shares.
    pub hero_joints: usize,
    /// Total frames baked (equals `db.entries.len()`).
    pub frames: usize,
    /// Stable digest over the whole database.
    pub digest: u64,
}

/// Validate one spec: names, registry source, ship license, tags and ranges.
fn validate_spec(spec: &ClipSpec) -> Result<&'static SourceInfo, BakeError> {
    let clip = bound(spec.name);
    if !valid_token(spec.name) {
        return Err(BakeError::BadSpec {
            clip,
            reason: "name",
        });
    }
    let source = source_by_id(spec.source)?;
    if spec.license != source.license {
        return Err(BakeError::Unlicensed { name: clip });
    }
    if spec.tags.is_empty() {
        return Err(BakeError::BadSpec {
            clip,
            reason: "tags",
        });
    }
    for (index, tag) in spec.tags.iter().enumerate() {
        if !valid_token(tag) || spec.tags[..index].contains(tag) {
            return Err(BakeError::BadSpec {
                clip,
                reason: "tags",
            });
        }
    }
    if spec.frames == 0 || spec.frames > MAX_CLIP_FRAMES {
        return Err(BakeError::BadSpec {
            clip,
            reason: "frames",
        });
    }
    for speed in [spec.speed0, spec.speed1] {
        if !speed.is_finite() || speed.abs() > MAX_BAKE_SPEED {
            return Err(BakeError::BadSpec {
                clip,
                reason: "speed",
            });
        }
    }
    if !spec.swing.is_finite() || spec.swing < 0.0 || spec.swing > MAX_SWING {
        return Err(BakeError::BadSpec {
            clip,
            reason: "swing",
        });
    }
    Ok(source)
}

/// Contact mask for one baked frame: the baseline gait rule at this phase.
fn contact_mask(phase: f32, frame: u16) -> u8 {
    let table = contacts_for_phase(phase, u32::from(frame));
    let mut mask = 0u8;
    for contact in &table.contacts {
        if contact.down {
            mask |= 1u8 << contact.foot;
        }
    }
    mask
}

/// Synthesize one frame entry: ramped hip velocity, triangle-wave feet, gait
/// contacts and a linear trajectory rollout, all finite index arithmetic.
fn frame_entry(spec: &ClipSpec, clip: u16, frame: u16) -> Result<Entry, BakeError> {
    let span = f32::from(spec.frames);
    let phase = f32::from(frame) / span;
    let speed = spec.speed0 + (spec.speed1 - spec.speed0) * phase;
    let hip = if spec.lateral {
        [0.0, speed]
    } else {
        [speed, 0.0]
    };
    let tri = if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    };
    let amp = spec.swing;
    let pose = PoseSample {
        hip_vel: hip,
        foot_l: [tri * amp - amp * 0.5, 0.05, 0.0],
        foot_r: [(1.0 - tri) * amp - amp * 0.5, 0.05, 0.0],
        contact: contact_mask(phase, frame),
    };
    let traj = predict_linear([0.0, 0.0], hip)?;
    Ok(Entry {
        feat: make_feature(&pose, &traj)?,
        clip,
        tick: frame,
        contact: pose.contact,
    })
}

/// Stable digest over one clip: name, tags and every entry feature.
fn clip_digest(spec: &ClipSpec, entries: &[Entry]) -> u64 {
    let mut state = Fnv64::new();
    state.write_str(spec.name);
    state.write_u8(0xFF);
    for tag in spec.tags {
        state.write_str(tag);
        state.write_u8(0);
    }
    for entry in entries {
        state.write_u16(entry.tick);
        state.write_u8(entry.contact);
        for lane in entry.feat {
            state.write_i32(lane);
        }
    }
    state.finish()
}

/// Bake specs into the motion database: validate provenance, synthesize one entry
/// per frame, sort canonically. Refuses unknown sources and unlicensed clips.
pub fn bake_loco_set(specs: &[ClipSpec]) -> Result<LocoSet, BakeError> {
    if specs.is_empty() {
        return Err(BakeError::BadSpec {
            clip: String::new(),
            reason: "empty set",
        });
    }
    let mut seen: Vec<&str> = Vec::with_capacity(specs.len());
    let mut sources: Vec<&SourceInfo> = Vec::with_capacity(specs.len());
    for spec in specs {
        if seen.contains(&spec.name) {
            return Err(BakeError::BadSpec {
                clip: bound(spec.name),
                reason: "duplicate",
            });
        }
        seen.push(spec.name);
        sources.push(validate_spec(spec)?);
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut clips: Vec<BakedClip> = Vec::with_capacity(specs.len());
    for (index, spec) in specs.iter().enumerate() {
        let base = entries.len();
        let mut planted: Vec<[bool; 2]> = Vec::with_capacity(usize::from(spec.frames));
        for frame in 0..spec.frames {
            entries.push(frame_entry(spec, index as u16, frame)?);
            let phase = f32::from(frame) / f32::from(spec.frames);
            let mask = contact_mask(phase, frame);
            planted.push([mask & 1 != 0, mask & 2 != 0]);
        }
        // SW25: the clip's own plant lane, baked beside its entries.
        let ik_weight =
            bake_foot_weights(&planted, FOOT_TICK_HZ).map_err(|err| BakeError::BadSpec {
                clip: bound(spec.name),
                reason: match err {
                    super::motion_bake::FootWeightError::Empty => "no frames to bake a plant",
                    super::motion_bake::FootWeightError::BadTickHz(_) => {
                        "the bake rate misses 30 Hz"
                    }
                    super::motion_bake::FootWeightError::TooLong(_) => "clip passes the bake cap",
                },
            })?;
        clips.push(BakedClip {
            name: spec.name,
            source: sources[index],
            tags: spec.tags,
            frames: spec.frames,
            looped: spec.looped,
            root_motion: spec.root_motion,
            synthetic: spec.synthetic,
            ik_weight,
            digest: clip_digest(spec, &entries[base..]),
        });
    }
    let db = bake_db(entries)?;
    Ok(LocoSet {
        frames: db.entries.len(),
        digest: db_hash(&db),
        hero_joints: loco_skeleton().len(),
        db,
        clips,
    })
}

/// Bake the hero set and require every [`REQUIRED_TAGS`] entry exactly once.
pub fn bake_hero_set() -> Result<LocoSet, BakeError> {
    let set = bake_loco_set(&HERO_SET)?;
    for tag in REQUIRED_TAGS {
        let hits = set
            .clips
            .iter()
            .filter(|clip| clip.tags.contains(&tag))
            .count();
        if hits != 1 {
            return Err(BakeError::BadSpec {
                clip: tag.to_string(),
                reason: "tags",
            });
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::super::motion_match::{build_query, cost_units, default_weights, search, NO_HIST};
    use super::*;

    #[test]
    fn every_baked_clip_carries_its_foot_plant_lane() {
        // SW25: the caller fills the lane from the clip's own contact masks, one sample per two
        // frames, and a walk cycle plants a foot somewhere in it.
        let set = bake_hero_set().expect("hero set bakes");
        for clip in &set.clips {
            assert_eq!(clip.ik_weight.hz, 30, "{} samples at 30 Hz", clip.name);
            assert_eq!(clip.ik_weight.tick_hz, FOOT_TICK_HZ, "{}", clip.name);
            let want = usize::from(clip.frames).div_ceil(2);
            assert_eq!(clip.ik_weight.ik_weight.len(), want, "{}", clip.name);
            let planted = clip
                .ik_weight
                .ik_weight
                .iter()
                .filter(|pair| pair.iter().any(|w| *w >= 900))
                .count();
            assert!(planted > 0, "{} plants a foot in {want} samples", clip.name);
            println!(
                "loco bake: {} frames {} samples {} whole plants {planted}",
                clip.name, clip.frames, want
            );
        }
    }

    #[test]
    fn hero_set_bakes_with_counts_and_tags() {
        let set = bake_hero_set().expect("hero set bakes");
        assert_eq!(set.clips.len(), 8, "clips={}", set.clips.len());
        assert_eq!(set.frames, 360, "frames={}", set.frames);
        assert_eq!(set.db.entries.len(), 360);
        assert_eq!(set.hero_joints, loco_skeleton().len());
        assert_eq!(set.hero_joints, 7);
        // Canonical (clip, tick) order with one entry per frame.
        for (index, entry) in set.db.entries.iter().enumerate() {
            let (clip, tick) = (entry.clip as usize, entry.tick as usize);
            assert!(clip < 8, "entry {index}: clip {}", entry.clip);
            assert!(
                tick < usize::from(set.clips[clip].frames),
                "entry {index}: tick {tick} past {}",
                set.clips[clip].frames
            );
            assert!(entry.contact <= 3, "entry {index}: mask {}", entry.contact);
        }
        // Every clip carries its source license and the synthetic mark.
        for clip in &set.clips {
            assert_eq!(clip.source.license, clip_license(clip.name));
            assert!(clip.synthetic, "{} must stay synthetic", clip.name);
        }
        // Search over the baked set is the motion-matching consumer path.
        let query = build_query(
            &PoseSample {
                hip_vel: [1.5, 0.0],
                foot_l: [0.125, 0.05, 0.0],
                foot_r: [-0.125, 0.05, 0.0],
                contact: contact_mask(0.5, 30),
            },
            &predict_linear([0.0, 0.0], [1.5, 0.0]).expect("finite wish"),
            NO_HIST,
            &default_weights(),
            cost_units(0.01),
            cost_units(0.04),
        )
        .expect("query valid");
        let (picked, _) = search(&set.db, &query).expect("search hits");
        assert_eq!((picked.clip, picked.tick), (1, 30));
    }

    /// Expected ship license per hero clip name.
    fn clip_license(name: &str) -> &'static str {
        match name {
            "start_walk" | "stop_walk" | "pivot_turn" => "CC-BY-4.0",
            _ => "CC0-1.0",
        }
    }

    #[test]
    fn bake_identical_twice() {
        let first = bake_hero_set().expect("first bake");
        let second = bake_hero_set().expect("second bake");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.db.entries, second.db.entries);
        assert_eq!(first.clips, second.clips);
        assert_eq!(first, second);
        println!(
            "CT3_BAKE clips={} frames={} entries={} digest=0x{:016x}",
            first.clips.len(),
            first.frames,
            first.db.entries.len(),
            first.digest
        );
        for clip in &first.clips {
            println!(
                "CT3_CLIP {} frames={} digest=0x{:016x}",
                clip.name, clip.frames, clip.digest
            );
        }
    }

    #[test]
    fn search_repeats_on_baked_db() {
        let set = bake_hero_set().expect("hero set bakes");
        // Rebuild the exact frame recipe mid-walk and query twice.
        let entry = set
            .db
            .entries
            .iter()
            .find(|e| e.clip == 1 && e.tick == 30)
            .copied();
        let _ = entry.expect("walk frame baked");
        let spec = &HERO_SET[1];
        let query = build_query(
            &PoseSample {
                hip_vel: [spec.speed0, 0.0],
                foot_l: [
                    rebuilt_tick_tri(30, 60) * spec.swing - spec.swing * 0.5,
                    0.05,
                    0.0,
                ],
                foot_r: [
                    (1.0 - rebuilt_tick_tri(30, 60)) * spec.swing - spec.swing * 0.5,
                    0.05,
                    0.0,
                ],
                contact: contact_mask(30.0 / 60.0, 30),
            },
            &predict_linear([0.0, 0.0], [spec.speed0, 0.0]).expect("finite wish"),
            NO_HIST,
            &default_weights(),
            cost_units(0.01),
            cost_units(0.04),
        )
        .expect("query valid");
        let first = search(&set.db, &query).expect("search hits");
        let second = search(&set.db, &query).expect("search hits");
        assert_eq!(first, second);
        assert_eq!((first.0.clip, first.0.tick), (1, 30));
        assert_eq!(first.0.cost, 0);
    }

    /// Triangle wave shared with [`frame_entry`]; the test restates the wave
    /// shape (not the code) so a phase regression fails here.
    fn rebuilt_tick_tri(frame: u16, frames: u16) -> f32 {
        let phase = f32::from(frame) / f32::from(frames);
        if phase < 0.5 {
            phase * 2.0
        } else {
            2.0 - phase * 2.0
        }
    }

    #[test]
    fn unlicensed_source_refused() {
        // Non-commercial licenses never bake, even on known sources.
        let nc = ClipSpec {
            name: "geno_mesh",
            source: "100style",
            license: "CC-BY-NC-4.0",
            tags: &["walk"],
            frames: 30,
            speed0: 1.0,
            speed1: 1.0,
            swing: 0.2,
            lateral: false,
            looped: false,
            root_motion: false,
            synthetic: false,
        };
        assert_eq!(
            bake_loco_set(&[nc]),
            Err(BakeError::Unlicensed {
                name: "geno_mesh".to_string()
            })
        );
        // A mismatched license on a known source is refused too.
        let mismatch = ClipSpec {
            license: "CC-BY-4.0",
            ..HERO_SET[0]
        };
        assert_eq!(
            bake_loco_set(&[mismatch]),
            Err(BakeError::Unlicensed {
                name: "idle_stand".to_string()
            })
        );
    }

    #[test]
    fn unknown_source_refused() {
        let foreign = ClipSpec {
            name: "foreign_clip",
            source: "mixamo",
            license: "CC0-1.0",
            tags: &["walk"],
            frames: 30,
            speed0: 1.0,
            speed1: 1.0,
            swing: 0.2,
            lateral: false,
            looped: false,
            root_motion: false,
            synthetic: false,
        };
        assert_eq!(
            bake_loco_set(&[foreign]),
            Err(BakeError::UnknownSource {
                name: "mixamo".to_string()
            })
        );
        assert_eq!(
            source_by_id("lafan1"),
            Err(BakeError::UnknownSource {
                name: "lafan1".to_string()
            })
        );
    }

    #[test]
    fn bad_specs_refused() {
        assert_eq!(
            bake_loco_set(&[]),
            Err(BakeError::BadSpec {
                clip: String::new(),
                reason: "empty set"
            })
        );
        let zero = ClipSpec {
            frames: 0,
            ..HERO_SET[0]
        };
        assert_eq!(
            bake_loco_set(&[zero]),
            Err(BakeError::BadSpec {
                clip: "idle_stand".to_string(),
                reason: "frames"
            })
        );
        let dup = [HERO_SET[0], HERO_SET[0]];
        assert_eq!(
            bake_loco_set(&dup),
            Err(BakeError::BadSpec {
                clip: "idle_stand".to_string(),
                reason: "duplicate"
            })
        );
        let spaced = ClipSpec {
            name: "bad name!",
            ..HERO_SET[0]
        };
        assert_eq!(
            bake_loco_set(&[spaced]),
            Err(BakeError::BadSpec {
                clip: "bad name!".to_string(),
                reason: "name"
            })
        );
        let bad_tag = ClipSpec {
            name: "tagged",
            tags: &["walk", "walk"],
            ..HERO_SET[1]
        };
        assert_eq!(
            bake_loco_set(&[bad_tag]),
            Err(BakeError::BadSpec {
                clip: "tagged".to_string(),
                reason: "tags"
            })
        );
        let fast = ClipSpec {
            name: "fast",
            speed0: f32::NAN,
            ..HERO_SET[1]
        };
        assert_eq!(
            bake_loco_set(&[fast]),
            Err(BakeError::BadSpec {
                clip: "fast".to_string(),
                reason: "speed"
            })
        );
    }
}
