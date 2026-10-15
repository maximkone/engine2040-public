//! Per-frame extraction snapshots. Consumers (render, audio, UI, the Bevy
//! viewer) read these; they never touch sim state and never feed sim hashes.
//!
//! Every snapshot carries the tick, the build hash and the schema version
//! it was written with, so a stale consumer detects skew and refreshes
//! instead of presenting torn state. The frozen [`PresentationFrame`] is the
//! one-way bytes a picture process reads: one frame per tick, actors with
//! their sim pose, health, root-motion speed and hit flag. Units: ticks,
//! frames, nodes, meters, radians, permille.

use crate::error::AppError;
use engine_core::hash_bytes;
use engine_scene::ids::SCHEMA_VERSION;

/// Schema version every extraction snapshot is written with.
pub const EXTRACTION_SCHEMA: u32 = SCHEMA_VERSION;

/// Schema version of the frozen [`PresentationFrame`] bytes. Bump it when
/// the layout changes; every older frame then refuses with a refresh hint.
pub const PRESENTATION_FRAME_SCHEMA: u32 = 1;

/// Most actors one frame carries (the arena participant cap).
pub const MAX_FRAME_ACTORS: usize = 16;

/// Leading bytes of every frame, so a foreign file refuses before parsing.
const FRAME_MAGIC: [u8; 4] = *b"E2PF";
/// Header: magic, schema, build hash, tick, digest, actor count.
const FRAME_HEADER_BYTES: usize = 4 + 4 + 8 + 4 + 8 + 1;
/// One actor: slot, position, yaw, health, speed, hit flag.
const FRAME_ACTOR_BYTES: usize = 1 + 12 + 4 + 2 + 4 + 1;

/// Deterministic build hash for one seed: crate version plus seed bytes.
/// Stable for a given binary plus config; changes with either, so a stale
/// consumer on old data sees a different hash and refreshes loudly.
pub fn build_hash_for(seed: u64) -> u64 {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    bytes.extend_from_slice(&seed.to_le_bytes());
    hash_bytes(&bytes)
}

/// What one frame publishes to render, audio and UI.
///
/// Published after the Extract lanes and read by the Present lanes each
/// frame, so arena and second-game menus render through this same shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extraction {
    /// Fixed ticks elapsed (accumulator steps, capped per frame).
    pub tick: u64,
    /// Presentation frames since handover.
    pub frame: u64,
    /// Build hash from [`build_hash_for`]; skew detector.
    pub build_hash: u64,
    /// Scene schema version the snapshot was written with.
    pub schema_version: u32,
    /// Scene hierarchy node count at extraction time.
    pub scene_nodes: usize,
    /// Arena checkpoint hash from the startup verify frame.
    pub verify_hash: u64,
}

impl Extraction {
    /// True when this snapshot was written for `tick`.
    pub fn is_for(&self, tick: u64) -> bool {
        self.tick == tick
    }

    /// Refuse a snapshot written for any tick but `expect`: stale data
    /// never presents silently.
    pub fn require_tick(&self, expect: u64) -> Result<(), AppError> {
        if self.tick == expect {
            Ok(())
        } else {
            Err(AppError::SceneLoad {
                reason: format!(
                    "stale extraction: snapshot tick {} but consumer expects {expect}",
                    self.tick
                ),
            })
        }
    }
}

/// One actor as the picture sees it on one tick. Read from the sim's own
/// observers; the picture never writes it back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorFrame {
    /// Participant slot, `0..MAX_FRAME_ACTORS`.
    pub slot: u8,
    /// Feet position `[x, y, z]` in meters.
    pub pos_m: [f32; 3],
    /// Facing yaw in radians about +Y.
    pub yaw_rad: f32,
    /// Health in permille, `0..=1000`.
    pub hp_permille: u16,
    /// Root-motion speed in meters per second (the locomotion intent).
    pub speed_mps: f32,
    /// True when this actor landed damage on this tick.
    pub hit: bool,
}

/// One frozen tick for the picture: schema and build hash in front so a
/// consumer on stale bytes refuses, then the digest and the actors.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationFrame {
    /// Layout version these bytes follow; [`PRESENTATION_FRAME_SCHEMA`] today.
    pub schema_version: u32,
    /// Build hash from [`build_hash_for`]; the skew detector.
    pub build_hash: u64,
    /// Zero-based fixed tick the frame shows (`0` is the spawn).
    pub tick: u32,
    /// Checkpoint hash after the tick before this frame; `0` at the spawn.
    pub digest: u64,
    /// Actors in slot order.
    pub actors: Vec<ActorFrame>,
}

/// Stale or foreign bytes: the one refusal shape, always with a refresh hint.
fn refuse(what: &str) -> AppError {
    AppError::SceneLoad {
        reason: format!(
            "stale presentation frame: {what}; refresh: rebuild the consumer against this \
             build (schema v{PRESENTATION_FRAME_SCHEMA}) and reread the frame"
        ),
    }
}

/// Little-endian reader over frame bytes; every miss is a truncation refusal.
struct FrameReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl FrameReader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], AppError> {
        let end = self
            .at
            .checked_add(N)
            .ok_or_else(|| refuse("length overflow"))?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| refuse("truncated bytes"))?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, AppError> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, AppError> {
        Ok(u16::from_le_bytes(self.take::<2>()?))
    }

    fn u32(&mut self) -> Result<u32, AppError> {
        Ok(u32::from_le_bytes(self.take::<4>()?))
    }

    fn u64(&mut self) -> Result<u64, AppError> {
        Ok(u64::from_le_bytes(self.take::<8>()?))
    }

    fn f32(&mut self) -> Result<f32, AppError> {
        let value = f32::from_bits(self.u32()?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(refuse("non-finite number"))
        }
    }
}

impl PresentationFrame {
    /// A frame at the current schema.
    pub fn new(build_hash: u64, tick: u32, digest: u64, actors: Vec<ActorFrame>) -> Self {
        Self {
            schema_version: PRESENTATION_FRAME_SCHEMA,
            build_hash,
            tick,
            digest,
            actors,
        }
    }

    /// Canonical little-endian bytes: the same frame twice gives the same
    /// bytes. Floats travel as their bit patterns.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            FRAME_HEADER_BYTES.saturating_add(self.actors.len().saturating_mul(FRAME_ACTOR_BYTES)),
        );
        out.extend_from_slice(&FRAME_MAGIC);
        out.extend_from_slice(&self.schema_version.to_le_bytes());
        out.extend_from_slice(&self.build_hash.to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.digest.to_le_bytes());
        out.push(u8::try_from(self.actors.len()).unwrap_or(u8::MAX));
        for actor in &self.actors {
            out.push(actor.slot);
            for axis in actor.pos_m {
                out.extend_from_slice(&axis.to_bits().to_le_bytes());
            }
            out.extend_from_slice(&actor.yaw_rad.to_bits().to_le_bytes());
            out.extend_from_slice(&actor.hp_permille.to_le_bytes());
            out.extend_from_slice(&actor.speed_mps.to_bits().to_le_bytes());
            out.push(u8::from(actor.hit));
        }
        out
    }

    /// Read frame bytes. Refuses loudly, with a refresh hint, on a foreign
    /// magic, any schema but [`PRESENTATION_FRAME_SCHEMA`], truncated or
    /// trailing bytes, too many actors or a non-finite number.
    pub fn decode(bytes: &[u8]) -> Result<Self, AppError> {
        let mut reader = FrameReader { bytes, at: 0 };
        if reader.take::<4>()? != FRAME_MAGIC {
            return Err(refuse("not a presentation frame (bad magic)"));
        }
        let schema_version = reader.u32()?;
        if schema_version != PRESENTATION_FRAME_SCHEMA {
            return Err(refuse(&format!(
                "schema {schema_version}, this build reads schema {PRESENTATION_FRAME_SCHEMA}"
            )));
        }
        let build_hash = reader.u64()?;
        let tick = reader.u32()?;
        let digest = reader.u64()?;
        let count = usize::from(reader.u8()?);
        if count > MAX_FRAME_ACTORS {
            return Err(refuse(&format!(
                "{count} actors, at most {MAX_FRAME_ACTORS}"
            )));
        }
        let mut actors = Vec::with_capacity(count);
        for _ in 0..count {
            let slot = reader.u8()?;
            let pos_m = [reader.f32()?, reader.f32()?, reader.f32()?];
            let yaw_rad = reader.f32()?;
            let hp_permille = reader.u16()?;
            let speed_mps = reader.f32()?;
            let hit = match reader.u8()? {
                0 => false,
                1 => true,
                other => return Err(refuse(&format!("hit flag {other}"))),
            };
            actors.push(ActorFrame {
                slot,
                pos_m,
                yaw_rad,
                hp_permille,
                speed_mps,
                hit,
            });
        }
        if reader.at != bytes.len() {
            return Err(refuse("trailing bytes"));
        }
        Ok(Self {
            schema_version,
            build_hash,
            tick,
            digest,
            actors,
        })
    }

    /// Refuse a frame written by another build: the consumer must refresh
    /// instead of presenting a torn mix of two binaries.
    pub fn require_build(&self, build_hash: u64) -> Result<(), AppError> {
        if self.build_hash == build_hash {
            Ok(())
        } else {
            Err(refuse(&format!(
                "build hash {:016x} but this build is {build_hash:016x}",
                self.build_hash
            )))
        }
    }

    /// Refuse a frame written for any tick but `expect`.
    pub fn require_tick(&self, expect: u32) -> Result<(), AppError> {
        if self.tick == expect {
            Ok(())
        } else {
            Err(refuse(&format!(
                "tick {} but the consumer expects {expect}",
                self.tick
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_hash_is_stable_per_seed_and_sensitive_to_it() {
        assert_eq!(build_hash_for(2040), build_hash_for(2040));
        assert_ne!(build_hash_for(2040), build_hash_for(7));
    }

    #[test]
    fn stale_snapshot_refuses_loudly() {
        let snapshot = Extraction {
            tick: 3,
            frame: 2,
            build_hash: build_hash_for(2040),
            schema_version: EXTRACTION_SCHEMA,
            scene_nodes: 0,
            verify_hash: 0,
        };
        assert!(snapshot.is_for(3));
        assert!(!snapshot.is_for(4));
        assert!(snapshot.require_tick(3).is_ok());
        let error = snapshot.require_tick(4).unwrap_err();
        let report = error.to_string();
        assert!(report.contains('3') && report.contains('4'));
    }

    fn sample_frame() -> PresentationFrame {
        PresentationFrame::new(
            build_hash_for(2040),
            60,
            0x1234_5678_9abc_def0,
            vec![
                ActorFrame {
                    slot: 0,
                    pos_m: [1.5, 0.0, -2.25],
                    yaw_rad: 0.75,
                    hp_permille: 1000,
                    speed_mps: 2.5,
                    hit: false,
                },
                ActorFrame {
                    slot: 1,
                    pos_m: [-3.0, 0.0, 4.0],
                    yaw_rad: -2.0,
                    hp_permille: 640,
                    speed_mps: 0.0,
                    hit: true,
                },
            ],
        )
    }

    /// Bytes an older build wrote: schema 0, no actors. Frozen here as the
    /// museum fixture; the current decoder must refuse them with a hint.
    const MUSEUM_V0: [u8; FRAME_HEADER_BYTES] = [
        b'E', b'2', b'P', b'F', // magic
        0, 0, 0, 0, // schema 0
        1, 2, 3, 4, 5, 6, 7, 8, // build hash
        60, 0, 0, 0, // tick
        9, 9, 9, 9, 9, 9, 9, 9, // digest
        0, // no actors
    ];

    #[test]
    fn current_frame_round_trips_twice() {
        let frame = sample_frame();
        let first = frame.encode();
        let second = frame.encode();
        assert_eq!(first, second);
        assert_eq!(first.len(), FRAME_HEADER_BYTES + 2 * FRAME_ACTOR_BYTES);
        let back = PresentationFrame::decode(&first).expect("current bytes decode");
        assert_eq!(back, frame);
        assert_eq!(back.encode(), first);
    }

    #[test]
    fn museum_v0_bytes_refuse_with_a_refresh_hint() {
        let error = PresentationFrame::decode(&MUSEUM_V0).expect_err("schema 0 must refuse");
        let report = error.to_string();
        assert!(report.contains("schema 0"), "{report}");
        assert!(report.contains("refresh"), "{report}");
        assert!(report.contains("v1"), "{report}");
    }

    #[test]
    fn foreign_truncated_and_trailing_bytes_refuse() {
        let bytes = sample_frame().encode();
        let mut foreign = bytes.clone();
        foreign[0] = b'X';
        assert!(PresentationFrame::decode(&foreign)
            .unwrap_err()
            .to_string()
            .contains("magic"));
        assert!(PresentationFrame::decode(&bytes[..bytes.len() - 1])
            .unwrap_err()
            .to_string()
            .contains("truncated"));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(PresentationFrame::decode(&trailing)
            .unwrap_err()
            .to_string()
            .contains("trailing"));
        assert!(PresentationFrame::decode(&[]).is_err());
    }

    #[test]
    fn build_and_tick_skew_refuse() {
        let frame = sample_frame();
        assert!(frame.require_build(build_hash_for(2040)).is_ok());
        let skew = frame
            .require_build(build_hash_for(7))
            .unwrap_err()
            .to_string();
        assert!(
            skew.contains("build hash") && skew.contains("refresh"),
            "{skew}"
        );
        assert!(frame.require_tick(60).is_ok());
        assert!(frame.require_tick(61).is_err());
    }

    #[test]
    fn bad_counts_flags_and_numbers_refuse() {
        let mut bytes = sample_frame().encode();
        bytes[FRAME_HEADER_BYTES - 1] = 17;
        assert!(PresentationFrame::decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("17 actors"));
        let mut bytes = sample_frame().encode();
        let hit_at = FRAME_HEADER_BYTES + FRAME_ACTOR_BYTES - 1;
        bytes[hit_at] = 2;
        assert!(PresentationFrame::decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("hit flag"));
        let mut frame = sample_frame();
        frame.actors[0].speed_mps = f32::NAN;
        assert!(PresentationFrame::decode(&frame.encode())
            .unwrap_err()
            .to_string()
            .contains("non-finite"));
    }
}
