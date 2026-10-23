//! hit_track: baked per-move hit tracks for the fight.
//!
//! Consumer: `engine-combat` reads one row by `(move, tick)` and feeds the
//! integer blade segment and bone capsules to the rules-v2 sweep and the cut
//! resolver. Data owner: `engine-anim`. Units: integer millimetres for every
//! position and radius, 60 Hz ticks for time. Determinism: key poses are
//! integer arithmetic only, so the same spec bakes equal rows twice.
//! Errors: [`HitTrackError`]; unknown or unlicensed sources are refused,
//! never guessed.
//!
//! The three shipped clips are the CC0 1H attack set (`attack_1h_1..3`) on
//! the CT7 fighter rig. The rows are authored synthetically in-code, in the
//! same style as [`crate::loco_bake`], and read no donor bytes.

use super::loco_bake::{bound, source_by_id, valid_token};
use engine_combat::anatomy::{JointId, JOINT_TOTAL};
use engine_combat::cut::{
    CutError, HitMove, HitTrack, HitTrackRow, JointTarget, CUT_POS_BOUND_MM, CUT_SWING_LEN_MM_V1,
    HIT_TRACK_VERSION_V1, MAX_HIT_TRACK_MOVES, MAX_HIT_TRACK_TICKS,
};

/// Blade half-thickness baked into every row, in millimetres.
pub const HIT_BLADE_HALF_MM: u32 = 10;
/// Shortest blade segment a spec may bake, in millimetres.
const MIN_BLADE_MM: u32 = 100;
/// Widest chest twist a spec may bake, in millimetres.
const MAX_TWIST_MM: u32 = 500;

/// One blade key pose of an attack: sword palm, blade tip and chest twist.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BladeKey {
    /// Tick of the key inside the move.
    pub tick: u16,
    /// Sword palm position in mm; the row's blade `p0`.
    pub palm_mm: [i32; 3],
    /// Blade tip position in mm; the row's blade `p1`.
    pub tip_mm: [i32; 3],
    /// Chest shift toward the shield arm, in mm.
    pub twist_mm: i32,
}

/// One attack clip to bake: provenance plus five blade key poses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AttackClipSpec {
    /// Clip short name, sidecar token charset (`attack_1h_1`).
    pub name: &'static str,
    /// Registry source id (`system_g6`).
    pub source: &'static str,
    /// Ship license; must equal the source registry license.
    pub license: &'static str,
    /// Frame count at 60 Hz; the shipped attack clips are 25.
    pub frames: u16,
    /// Tick where the blade crosses the strike line.
    pub strike_tick: u16,
    /// Blade key poses in tick order, first `0`, last `frames - 1`.
    pub keys: [BladeKey; 5],
}

/// The three shipped 1H attack clips, baked synthetically on the CT7 rig.
///
/// The strike ticks sit on the readable rules-v2 startup (15 ticks); the
/// `attack_1h_*` names match `content/manifest.json`.
pub const HERO_ATTACK_SET: [AttackClipSpec; 3] = [
    AttackClipSpec {
        name: "attack_1h_1",
        source: "system_g6",
        license: "CC0-1.0",
        frames: 25,
        strike_tick: 15,
        keys: [
            BladeKey {
                tick: 0,
                palm_mm: [-650, 220, -100],
                tip_mm: [-650, 220, 900],
                twist_mm: -70,
            },
            BladeKey {
                tick: 8,
                palm_mm: [-420, 240, 250],
                tip_mm: [-420, 240, 1250],
                twist_mm: -40,
            },
            BladeKey {
                tick: 15,
                palm_mm: [0, 200, 600],
                tip_mm: [0, 200, 1600],
                twist_mm: 0,
            },
            BladeKey {
                tick: 20,
                palm_mm: [420, 240, 250],
                tip_mm: [420, 240, 1250],
                twist_mm: 40,
            },
            BladeKey {
                tick: 24,
                palm_mm: [600, 220, -100],
                tip_mm: [600, 220, 900],
                twist_mm: 70,
            },
        ],
    },
    AttackClipSpec {
        name: "attack_1h_2",
        source: "system_g6",
        license: "CC0-1.0",
        frames: 25,
        strike_tick: 15,
        keys: [
            BladeKey {
                tick: 0,
                palm_mm: [0, 780, 150],
                tip_mm: [0, 1680, 550],
                twist_mm: 0,
            },
            BladeKey {
                tick: 8,
                palm_mm: [0, 720, 280],
                tip_mm: [0, 1620, 680],
                twist_mm: 0,
            },
            BladeKey {
                tick: 15,
                palm_mm: [0, 250, 700],
                tip_mm: [0, -200, 1400],
                twist_mm: 0,
            },
            BladeKey {
                tick: 20,
                palm_mm: [0, 80, 620],
                tip_mm: [0, -520, 1220],
                twist_mm: 0,
            },
            BladeKey {
                tick: 24,
                palm_mm: [0, 150, 380],
                tip_mm: [0, -650, 880],
                twist_mm: 0,
            },
        ],
    },
    AttackClipSpec {
        name: "attack_1h_3",
        source: "system_g6",
        license: "CC0-1.0",
        frames: 25,
        strike_tick: 15,
        keys: [
            BladeKey {
                tick: 0,
                palm_mm: [520, -520, 80],
                tip_mm: [920, -920, 560],
                twist_mm: 60,
            },
            BladeKey {
                tick: 8,
                palm_mm: [260, -260, 340],
                tip_mm: [660, -660, 820],
                twist_mm: 30,
            },
            BladeKey {
                tick: 15,
                palm_mm: [0, -120, 600],
                tip_mm: [0, 680, 1200],
                twist_mm: 0,
            },
            BladeKey {
                tick: 20,
                palm_mm: [-260, 140, 420],
                tip_mm: [-260, 940, 1020],
                twist_mm: -30,
            },
            BladeKey {
                tick: 24,
                palm_mm: [-420, 260, 120],
                tip_mm: [-420, 1060, 720],
                twist_mm: -60,
            },
        ],
    },
];

/// Why a hit-track bake failed; unknown and unlicensed sources are refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HitTrackError {
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
    /// The baked wire failed the combat range checks; field carried.
    BadTrack(&'static str),
}

impl core::fmt::Display for HitTrackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownSource { name } => {
                write!(f, "hit track bake: unknown source ({name})")
            }
            Self::Unlicensed { name } => {
                write!(f, "hit track bake: unlicensed clip ({name})")
            }
            Self::BadSpec { clip, reason } => {
                write!(f, "hit track bake: bad spec ({clip}: {reason})")
            }
            Self::BadTrack(reason) => {
                write!(f, "hit track bake: combat track check ({reason})")
            }
        }
    }
}

impl std::error::Error for HitTrackError {}

/// Bake the shipped three-move attack set.
///
/// # Errors
/// [`HitTrackError`] when a spec names an unknown or unlicensed source, a
/// spec field is out of bounds, or the baked wire fails the combat checks.
pub fn bake_hit_tracks() -> Result<HitTrack, HitTrackError> {
    bake_hit_track_set(&HERO_ATTACK_SET)
}

/// Bake attack specs into `(move, tick)` rows; move id is the spec index.
///
/// # Errors
/// [`HitTrackError`] for an empty or oversized set, a bad spec, an unknown
/// or unlicensed source, or a baked wire the combat reader refuses.
pub fn bake_hit_track_set(specs: &[AttackClipSpec]) -> Result<HitTrack, HitTrackError> {
    if specs.is_empty() || specs.len() > MAX_HIT_TRACK_MOVES {
        return Err(HitTrackError::BadSpec {
            clip: String::new(),
            reason: "move count",
        });
    }
    let mut moves = Vec::with_capacity(specs.len());
    for (index, spec) in specs.iter().enumerate() {
        validate_spec(spec)?;
        let source = source_by_id(spec.source).map_err(|_| HitTrackError::UnknownSource {
            name: bound(spec.source),
        })?;
        if source.license != spec.license {
            return Err(HitTrackError::Unlicensed {
                name: bound(spec.name),
            });
        }
        let id = u16::try_from(index).map_err(|_| HitTrackError::BadSpec {
            clip: bound(spec.name),
            reason: "move id",
        })?;
        moves.push(HitMove {
            id,
            name: spec.name,
            rows: bake_rows(spec),
        });
    }
    let track = HitTrack {
        version: HIT_TRACK_VERSION_V1,
        moves,
    };
    track.validate().map_err(track_error)?;
    Ok(track)
}

/// Range-check one spec before any row is built.
fn validate_spec(spec: &AttackClipSpec) -> Result<(), HitTrackError> {
    let bad = |reason| HitTrackError::BadSpec {
        clip: bound(spec.name),
        reason,
    };
    if !valid_token(spec.name) {
        return Err(bad("name"));
    }
    if spec.frames == 0 || usize::from(spec.frames) > MAX_HIT_TRACK_TICKS {
        return Err(bad("frames"));
    }
    if spec.strike_tick >= spec.frames {
        return Err(bad("strike tick"));
    }
    if spec.keys[0].tick != 0 {
        return Err(bad("first key"));
    }
    let last = spec.keys[spec.keys.len() - 1];
    if u32::from(last.tick) + 1 != u32::from(spec.frames) {
        return Err(bad("last key"));
    }
    let mut previous: Option<u16> = None;
    let mut strike_key = false;
    for key in &spec.keys {
        if let Some(tick) = previous {
            if key.tick <= tick {
                return Err(bad("key order"));
            }
        }
        previous = Some(key.tick);
        if key.tick == spec.strike_tick {
            strike_key = true;
        }
        for value in key.palm_mm.iter().chain(key.tip_mm.iter()) {
            if !(-CUT_POS_BOUND_MM..=CUT_POS_BOUND_MM).contains(value) {
                return Err(bad("key position"));
            }
        }
        if key.twist_mm.unsigned_abs() > MAX_TWIST_MM {
            return Err(bad("twist"));
        }
        let length = blade_len_mm(key.palm_mm, key.tip_mm);
        if !(MIN_BLADE_MM..=CUT_SWING_LEN_MM_V1).contains(&length) {
            return Err(bad("blade length"));
        }
    }
    if !strike_key {
        return Err(bad("strike key"));
    }
    Ok(())
}

/// Build one move's integer-mm rows, one per tick.
fn bake_rows(spec: &AttackClipSpec) -> Vec<HitTrackRow> {
    let mut rows = Vec::with_capacity(usize::from(spec.frames));
    for tick in 0..spec.frames {
        let pose = pose_at(spec, tick);
        rows.push(HitTrackRow {
            tick,
            p0_mm: pose.palm_mm,
            p1_mm: pose.tip_mm,
            blade_half_mm: HIT_BLADE_HALF_MM,
            bones: capsules(pose.palm_mm, pose.tip_mm, pose.twist_mm),
        });
    }
    rows
}

/// One interpolated key pose.
#[derive(Clone, Copy)]
struct KeyPose {
    palm_mm: [i32; 3],
    tip_mm: [i32; 3],
    twist_mm: i32,
}

/// The key pose at `tick`, linearly interpolated between adjacent keys.
fn pose_at(spec: &AttackClipSpec, tick: u16) -> KeyPose {
    let mut index = 0;
    while index + 1 < spec.keys.len() && spec.keys[index + 1].tick <= tick {
        index += 1;
    }
    let key = spec.keys[index];
    if index + 1 == spec.keys.len() || tick <= key.tick {
        return KeyPose {
            palm_mm: key.palm_mm,
            tip_mm: key.tip_mm,
            twist_mm: key.twist_mm,
        };
    }
    let next = spec.keys[index + 1];
    let span = i64::from(next.tick - key.tick);
    let step = i64::from(tick - key.tick);
    KeyPose {
        palm_mm: [
            lerp(key.palm_mm[0], next.palm_mm[0], step, span),
            lerp(key.palm_mm[1], next.palm_mm[1], step, span),
            lerp(key.palm_mm[2], next.palm_mm[2], step, span),
        ],
        tip_mm: [
            lerp(key.tip_mm[0], next.tip_mm[0], step, span),
            lerp(key.tip_mm[1], next.tip_mm[1], step, span),
            lerp(key.tip_mm[2], next.tip_mm[2], step, span),
        ],
        twist_mm: lerp(key.twist_mm, next.twist_mm, step, span),
    }
}

/// Integer linear interpolation: `a` at step 0, `b` at step `span`.
fn lerp(a: i32, b: i32, step: i64, span: i64) -> i32 {
    let value = i64::from(a) + (i64::from(b) - i64::from(a)) * step / span;
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// Build the sixteen CT7 bone capsules in joint wire order.
fn capsules(palm: [i32; 3], tip: [i32; 3], twist: i32) -> [JointTarget; JOINT_TOTAL] {
    let shoulder_l = [180 + twist, 430, 0];
    let elbow_l = [250 + twist, 240, 0];
    let wrist_l = [300 + twist, 60, 0];
    let hand_l = [320 + twist, 10, 0];
    let shoulder_r = [-180 + twist, 430, 0];
    let elbow_r = midpoint(shoulder_r, palm);
    let hand_r = hand_tip(palm, tip);
    [
        capsule(JointId::Pelvis, [0, -80, 0], [0, 80, 0], 90),
        capsule(JointId::Waist, [twist, 110, 0], [twist, 380, 0], 115),
        capsule(JointId::Chest, [twist, 110, 0], [twist, 380, 0], 115),
        capsule(JointId::Neck, [twist, 500, 0], [twist, 620, 0], 90),
        capsule(JointId::ShoulderL, shoulder_l, elbow_l, 40),
        capsule(JointId::ShoulderR, shoulder_r, elbow_r, 40),
        capsule(JointId::ElbowL, elbow_l, wrist_l, 35),
        capsule(JointId::ElbowR, elbow_r, palm, 35),
        capsule(JointId::WristL, wrist_l, hand_l, 30),
        capsule(JointId::WristR, palm, hand_r, 30),
        capsule(JointId::HipL, [90, 0, 0], [90, -450, 0], 70),
        capsule(JointId::HipR, [-90, 0, 0], [-90, -450, 0], 70),
        capsule(JointId::KneeL, [90, -450, 0], [90, -880, 0], 55),
        capsule(JointId::KneeR, [-90, -450, 0], [-90, -880, 0], 55),
        capsule(JointId::AnkleL, [90, -880, 0], [90, -880, 140], 40),
        capsule(JointId::AnkleR, [-90, -880, 0], [-90, -880, 140], 40),
    ]
}

/// One capsule row; fields are integer by construction.
const fn capsule(joint: JointId, q0_mm: [i32; 3], q1_mm: [i32; 3], radius_mm: u32) -> JointTarget {
    JointTarget {
        joint,
        q0_mm,
        q1_mm,
        radius_mm,
    }
}

/// Integer midpoint, rounded toward zero.
fn midpoint(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    let mut out = [0; 3];
    let mut axis = 0;
    while axis < 3 {
        out[axis] = (a[axis] + b[axis]) / 2;
        axis += 1;
    }
    out
}

/// A hand point one sixteenth of the blade past the palm.
fn hand_tip(palm: [i32; 3], tip: [i32; 3]) -> [i32; 3] {
    let mut out = palm;
    let mut axis = 0;
    while axis < 3 {
        out[axis] = palm[axis] + (tip[axis] - palm[axis]) / 16;
        axis += 1;
    }
    out
}

/// Whole-millimetre blade length; integer square root, never a float.
fn blade_len_mm(palm: [i32; 3], tip: [i32; 3]) -> u32 {
    let mut length_sq: i64 = 0;
    let mut axis = 0;
    while axis < 3 {
        let delta = i64::from(tip[axis]) - i64::from(palm[axis]);
        length_sq += delta * delta;
        axis += 1;
    }
    u32::try_from(u64::try_from(length_sq).unwrap_or(0).isqrt()).unwrap_or(u32::MAX)
}

/// Map the combat wire check to the bake's error, keeping the field name.
fn track_error(error: CutError) -> HitTrackError {
    match error {
        CutError::BadVersion(_) => HitTrackError::BadTrack("version"),
        CutError::BadValue { field, .. } => HitTrackError::BadTrack(field),
        _ => HitTrackError::BadTrack("shape"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_combat::fight::{StrikePose, TABLES_V2};
    use engine_combat::sweep::{
        sweep_one, validate_sweep, SweepOutcome, SweepParticipant, SWEEP_V1,
    };

    #[test]
    fn hit_track_bake_is_equal_twice() {
        let first = bake_hit_tracks().expect("first bake");
        let second = bake_hit_tracks().expect("second bake");
        assert_eq!(first, second);
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.version, HIT_TRACK_VERSION_V1);
        assert_eq!(first.moves.len(), HERO_ATTACK_SET.len());
        for (index, spec) in HERO_ATTACK_SET.iter().enumerate() {
            let id = u16::try_from(index).expect("move id");
            let mv = &first.moves[index];
            assert_eq!(mv.id, id);
            assert_eq!(mv.name, spec.name);
            assert_eq!(mv.rows.len(), usize::from(spec.frames));
            for (tick, row) in mv.rows.iter().enumerate() {
                assert_eq!(usize::from(row.tick), tick);
                assert_eq!(row.blade_half_mm, HIT_BLADE_HALF_MM);
                assert!(
                    validate_sweep(&row.blade(0), 1, &SWEEP_V1).is_ok(),
                    "{} tick {tick} blade is in the sweep bounds",
                    spec.name
                );
                for (joint, bone) in row.bones.iter().enumerate() {
                    assert_eq!(usize::from(bone.joint.as_u8()), joint);
                    assert!(bone.radius_mm > 0);
                    assert!(bone.radius_mm <= 500);
                }
                let wrist = usize::from(JointId::WristR.as_u8());
                assert_eq!(row.p0_mm, row.bones[wrist].q0_mm);
                if let Some(key) = spec.keys.iter().find(|key| key.tick == row.tick) {
                    assert_eq!(row.p0_mm, key.palm_mm);
                    assert_eq!(row.p1_mm, key.tip_mm);
                }
            }
        }
    }

    #[test]
    fn hit_track_bake_hits_at_the_baked_tick() {
        let track = bake_hit_tracks().expect("bake");
        for (index, spec) in HERO_ATTACK_SET.iter().enumerate() {
            let id = u16::try_from(index).expect("move id");
            assert_eq!(
                u32::from(spec.strike_tick),
                TABLES_V2.attack_startup_ticks,
                "{} lands on the readable v2 startup",
                spec.name
            );
            let row = track.row(id, spec.strike_tick).expect("strike row");
            let pose = StrikePose::from_hit_row(body(0, 0, 0), row, 0);
            assert_eq!(pose.blade, row.blade(0));
            assert!(
                matches!(
                    sweep_one(&pose.body, &body(1, 0, 900), &pose.blade, &SWEEP_V1),
                    SweepOutcome::Hit { .. }
                ),
                "{} blade hits the foe on the strike line",
                spec.name
            );
            assert!(
                matches!(
                    sweep_one(&pose.body, &body(1, 5000, 900), &pose.blade, &SWEEP_V1),
                    SweepOutcome::Miss(_)
                ),
                "{} blade misses a foe off the path",
                spec.name
            );
        }
    }

    /// The standard arena hurt capsule at `(x, z)` mm facing +z.
    fn body(id: u8, x: i32, z: i32) -> SweepParticipant {
        SweepParticipant {
            id,
            pos_mm: [x, 0, z],
            facing_x: 0,
            facing_z: 1000,
            hurt_half_height_mm: 800,
            hurt_radius_mm: 150,
            shield_half_deg: 60,
            shield_up: false,
        }
    }
}
