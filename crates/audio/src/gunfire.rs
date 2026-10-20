//! gunfire: gunshot, supersonic-crack and shout acoustics for a city block.
//!
//! Consumer: the urban presentation layer reads one [`AcousticReport`] per
//! source per tick and applies it to a [`Mixer`](crate::Mixer) voice: gain,
//! lowpass cutoff, start delay and the room or street reverb send. The report
//! is presentation data only; no simulation code reads it and no hash covers
//! it. Walls and doors are caller input (the game-side spatial index query),
//! the report itself is a pure function of scene, listener and source.
//!
//! What the report carries:
//!
//! - **Distance filter and delay**: [`spatial::falloff_gain`](crate::spatial::falloff_gain)
//!   plus the air cutoff already owned by [`occlude`], and the travel time at
//!   343 m/s in whole microseconds, so replays are bit-identical.
//! - **Supersonic crack**: a second onset at the bullet's closest approach,
//!   ahead of the muzzle blast for every shot that passes the listener; a
//!   subsonic round has no crack.
//! - **Room and street reverb**: the source's SN1 zone picks a
//!   [`ReverbParams`] tail; a source outside every zone falls back to the
//!   listener's zone, then to an open street.
//! - **SN2 occlusion through walls and doors**: closed doors are wood
//!   occluders, open doors pass; the shout bus keeps the SN2 voice floor.
//!
//! Units: centimeters for positions, Hz for cutoffs, ms for delays and
//! tails, linear `0.0..=1.0` for gains. Budget: 32 reports per batch under
//! [`BUDGET_MS_PER_BATCH`] on the game thread; the timing probe is ignored by
//! default and runs under `--perf` in release.

use crate::occlude::{self, OccludeError, Wall, VOICE_FLOOR_Q16};
use crate::reverb::{ReverbError, ReverbParams};

/// Speed of sound in air: 343 m/s in integer centimeters per second.
pub const SPEED_OF_SOUND_CM_PER_S: u32 = 34_300;
/// Most sources one [`Acoustics::report_batch`] accepts per tick.
pub const MAX_BATCH: usize = 32;
/// Most static walls one scene holds (the SN2 query cap).
pub const MAX_WALLS: usize = occlude::MAX_WALLS_PER_QUERY;
/// Most doors one scene holds.
pub const MAX_DOORS: usize = 128;
/// Most reverb zones one scene holds.
pub const MAX_ZONES: usize = 32;
/// Per-batch budget in ms at 60 Hz, game thread, release profile.
pub const BUDGET_MS_PER_BATCH: f32 = 0.2;
/// Gunshot distance at which attenuation starts, cm (3 m).
pub const GUNSHOT_NEAR_CM: u32 = 300;
/// Gunshot cull distance, cm (200 m).
pub const GUNSHOT_MAX_CM: u32 = 20_000;
/// Shout distance at which attenuation starts, cm (1 m).
pub const SHOUT_NEAR_CM: u32 = 100;
/// Shout cull distance, cm (60 m).
pub const SHOUT_MAX_CM: u32 = 6_000;
/// Closest-approach distance that still gives a full-level crack, cm.
pub const CRACK_NEAR_CM: u32 = 25;
/// A crack keeps some extra brightness over the muffled blast, Hz.
pub const CRACK_BRIGHTEN_HZ: u32 = 4_000;

/// Small-room tail length in ms.
pub const ROOM_RT60_MS: u16 = 600;
/// Small-room pre-delay in ms.
pub const ROOM_PRE_DELAY_MS: f32 = 12.0;
/// Small-room damping.
pub const ROOM_DAMPING: f32 = 0.58;
/// Small-room wet level.
pub const ROOM_WET: f32 = 0.30;
/// Street-canyon tail length in ms.
pub const STREET_RT60_MS: u16 = 1_200;
/// Street-canyon pre-delay in ms.
pub const STREET_PRE_DELAY_MS: f32 = 28.0;
/// Street-canyon damping.
pub const STREET_DAMPING: f32 = 0.40;
/// Street-canyon wet level.
pub const STREET_WET: f32 = 0.20;

/// What made the sound: a gunshot, a supersonic round or a human shout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// Subsonic gunshot: muzzle blast only.
    Gunshot,
    /// Supersonic round: muzzle blast plus a shockwave at closest approach.
    Supersonic,
    /// Human shout: the SN2 voice floor applies to the occlusion gain.
    Shout,
}

/// One sound source for the tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    /// Ground position: east, south, in cm (the reverb zone uses height 0).
    pub pos_cm: [i32; 2],
    /// What made the sound.
    pub kind: SourceKind,
    /// Perpendicular miss distance in cm; read only for [`SourceKind::Supersonic`].
    pub miss_cm: u32,
}

impl Source {
    /// A subsonic gunshot at `pos_cm`.
    #[must_use]
    pub fn gunshot(pos_cm: [i32; 2]) -> Self {
        Self {
            pos_cm,
            kind: SourceKind::Gunshot,
            miss_cm: 0,
        }
    }

    /// A supersonic round whose path passes within `miss_cm` of the listener.
    #[must_use]
    pub fn supersonic(pos_cm: [i32; 2], miss_cm: u32) -> Self {
        Self {
            pos_cm,
            kind: SourceKind::Supersonic,
            miss_cm,
        }
    }

    /// A shout at `pos_cm`.
    #[must_use]
    pub fn shout(pos_cm: [i32; 2]) -> Self {
        Self {
            pos_cm,
            kind: SourceKind::Shout,
            miss_cm: 0,
        }
    }
}

/// One reverb zone: a circle on the ground with a tail length and preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    /// Room or street preset.
    pub kind: ZoneKind,
    /// Zone center: east, south, in cm.
    pub center_cm: [i32; 2],
    /// Zone radius in cm.
    pub radius_cm: u32,
    /// Tail length in ms.
    pub tail_ms: u16,
}

/// The two reverb families this module places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    /// Small room: short, dense, wetter tail.
    Room,
    /// Street canyon: longer slap, drier tail.
    Street,
}

impl Zone {
    /// A small room centered at `center_cm` (9 m radius, 600 ms tail).
    #[must_use]
    pub fn room(center_cm: [i32; 2]) -> Self {
        Self {
            kind: ZoneKind::Room,
            center_cm,
            radius_cm: 900,
            tail_ms: ROOM_RT60_MS,
        }
    }

    /// A street canyon centered at `center_cm` (25 m radius, 1200 ms tail).
    #[must_use]
    pub fn street(center_cm: [i32; 2]) -> Self {
        Self {
            kind: ZoneKind::Street,
            center_cm,
            radius_cm: 2_500,
            tail_ms: STREET_RT60_MS,
        }
    }

    /// True when `pos_cm` lies inside the circle, edge included.
    #[must_use]
    pub fn contains(&self, pos_cm: [i32; 2]) -> bool {
        let dx = i64::from(pos_cm[0]) - i64::from(self.center_cm[0]);
        let dz = i64::from(pos_cm[1]) - i64::from(self.center_cm[1]);
        let radius = i64::from(self.radius_cm);
        dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz)) <= radius.saturating_mul(radius)
    }

    /// SN1 params for this zone at `sample_rate_hz`.
    ///
    /// Errors: [`ReverbError`] when the rate or the tail is out of range.
    pub fn params(&self, sample_rate_hz: u32) -> Result<ReverbParams, ReverbError> {
        let (pre_delay_ms, damping, wet) = match self.kind {
            ZoneKind::Room => (ROOM_PRE_DELAY_MS, ROOM_DAMPING, ROOM_WET),
            ZoneKind::Street => (STREET_PRE_DELAY_MS, STREET_DAMPING, STREET_WET),
        };
        let params = ReverbParams {
            sample_rate_hz,
            pre_delay_ms,
            rt60_ms: f32::from(self.tail_ms),
            damping,
            wet,
        };
        params.validate()?;
        Ok(params)
    }
}

/// One door on the path: a wall segment that only occludes when closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Door {
    /// The door leaf as a wall segment and material.
    pub wall: Wall,
    /// Open doors pass sound; closed doors muffle like their material.
    pub open: bool,
}

impl Door {
    /// A door leaf from integer-cm endpoints and a material.
    #[must_use]
    pub fn new(
        ax_cm: i32,
        az_cm: i32,
        bx_cm: i32,
        bz_cm: i32,
        material: occlude::Material,
    ) -> Self {
        Self {
            wall: Wall::new(ax_cm, az_cm, bx_cm, bz_cm, material),
            open: false,
        }
    }

    /// A closed wood door from integer-cm endpoints.
    #[must_use]
    pub fn closed(ax_cm: i32, az_cm: i32, bx_cm: i32, bz_cm: i32) -> Self {
        Self::new(ax_cm, az_cm, bx_cm, bz_cm, occlude::Material::Wood)
    }

    /// An open door: set [`Door::open`] on a closed leaf.
    #[must_use]
    pub fn opened(mut self) -> Self {
        self.open = true;
        self
    }
}

/// The shockwave of a supersonic round: a bright onset at closest approach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crack {
    /// Travel time from the closest approach to the ears, ms.
    pub delay_ms: f32,
    /// The same delay in whole samples at the scene rate.
    pub delay_samples: u32,
    /// How far the crack leads the muzzle blast, ms (never negative).
    pub lead_ms: f32,
    /// Linear level of the crack.
    pub gain: f32,
    /// Lowpass cutoff for the crack in Hz.
    pub cutoff_hz: u32,
}

/// One source's presentation parameters for this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcousticReport {
    /// What made the sound.
    pub kind: SourceKind,
    /// Path length from source to listener, cm.
    pub distance_cm: u32,
    /// Distance falloff alone, `0.0..=1.0`.
    pub distance_gain: f32,
    /// Wall and closed-door gain alone, `0.0..=1.0` (after the shout floor).
    pub occlusion_gain: f32,
    /// Direct level the mixer applies: distance times occlusion.
    pub gain: f32,
    /// Lowpass cutoff for the direct sound and tail, Hz.
    pub cutoff_hz: u32,
    /// Travel time of the direct sound, ms.
    pub delay_ms: f32,
    /// The same delay in whole samples at the scene rate.
    pub delay_samples: u32,
    /// True when the SN2 voice floor raised the occlusion gain.
    pub floor_applied: bool,
    /// Reverb zone family the tail belongs to.
    pub zone: ZoneKind,
    /// SN1 params for the tail.
    pub reverb: ReverbParams,
    /// Level of the reverb send: direct gain times the preset wet.
    pub reverb_send: f32,
    /// Shockwave onset for [`SourceKind::Supersonic`].
    pub crack: Option<Crack>,
}

/// Typed refusal from the scene or one report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GunfireError {
    /// More walls than [`MAX_WALLS`].
    TooManyWalls {
        /// Rejected count.
        got: usize,
    },
    /// More doors than [`MAX_DOORS`].
    TooManyDoors {
        /// Rejected count.
        got: usize,
    },
    /// More zones than [`MAX_ZONES`].
    TooManyZones {
        /// Rejected count.
        got: usize,
    },
    /// More sources than [`MAX_BATCH`] in one batch.
    BatchTooLarge {
        /// Rejected count.
        got: usize,
    },
    /// The output slice is shorter than the source slice.
    OutputTooSmall {
        /// Provided report slots.
        got: usize,
        /// Required report slots.
        want: usize,
    },
    /// The SN1 params refused the scene rate or a zone tail.
    Reverb(ReverbError),
    /// The SN2 query refused the wall set.
    Occlude(OccludeError),
}

impl std::fmt::Display for GunfireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyWalls { got } => write!(f, "gunfire: too many walls {got}"),
            Self::TooManyDoors { got } => write!(f, "gunfire: too many doors {got}"),
            Self::TooManyZones { got } => write!(f, "gunfire: too many zones {got}"),
            Self::BatchTooLarge { got } => write!(f, "gunfire: batch of {got} over 32"),
            Self::OutputTooSmall { got, want } => {
                write!(f, "gunfire: output has {got} slots, needs {want}")
            }
            Self::Reverb(error) => write!(f, "gunfire reverb: {error}"),
            Self::Occlude(error) => write!(f, "gunfire occlusion: {error}"),
        }
    }
}

impl std::error::Error for GunfireError {}

impl From<ReverbError> for GunfireError {
    fn from(error: ReverbError) -> Self {
        Self::Reverb(error)
    }
}

impl From<OccludeError> for GunfireError {
    fn from(error: OccludeError) -> Self {
        Self::Occlude(error)
    }
}

/// Travel time split into the report's two units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Travel {
    ms: u32,
    samples: u32,
}

impl Travel {
    /// Full microseconds from `distance_cm` at [`SPEED_OF_SOUND_CM_PER_S`].
    fn micros(distance_cm: u32) -> u32 {
        u32::try_from(u64::from(distance_cm) * 1_000_000 / u64::from(SPEED_OF_SOUND_CM_PER_S))
            .unwrap_or(u32::MAX)
    }

    fn new(distance_cm: u32, sample_rate_hz: u32) -> Self {
        let micros = Self::micros(distance_cm);
        let samples = u32::try_from(u64::from(micros) * u64::from(sample_rate_hz) / 1_000_000)
            .unwrap_or(u32::MAX);
        Self {
            ms: micros,
            samples,
        }
    }

    /// Travel time in whole milliseconds as a float for the report.
    fn ms_f32(self) -> f32 {
        crate::f32_of(self.ms) / 1_000.0
    }
}

/// A city scene: static walls, doors and reverb zones, fixed at build time.
#[derive(Clone, Debug)]
pub struct Acoustics {
    walls: Vec<Wall>,
    doors: Vec<Door>,
    zones: Vec<Zone>,
    sample_rate_hz: u32,
}

impl Acoustics {
    /// Empty scene at `sample_rate_hz`.
    ///
    /// Errors: [`GunfireError::Reverb`] when the rate is unsupported.
    pub fn new(sample_rate_hz: u32) -> Result<Self, GunfireError> {
        Zone::room([0, 0]).params(sample_rate_hz)?;
        Ok(Self {
            walls: Vec::new(),
            doors: Vec::new(),
            zones: Vec::new(),
            sample_rate_hz,
        })
    }

    /// Add one occluder wall. Build-time only; the tick borrows the slice.
    ///
    /// Errors: [`GunfireError::TooManyWalls`] past [`MAX_WALLS`].
    pub fn add_wall(&mut self, wall: Wall) -> Result<(), GunfireError> {
        if self.walls.len() >= MAX_WALLS {
            return Err(GunfireError::TooManyWalls {
                got: self.walls.len().saturating_add(1),
            });
        }
        self.walls.push(wall);
        Ok(())
    }

    /// Add one door. Build-time only.
    ///
    /// Errors: [`GunfireError::TooManyDoors`] past [`MAX_DOORS`].
    pub fn add_door(&mut self, door: Door) -> Result<(), GunfireError> {
        if self.doors.len() >= MAX_DOORS {
            return Err(GunfireError::TooManyDoors {
                got: self.doors.len().saturating_add(1),
            });
        }
        self.doors.push(door);
        Ok(())
    }

    /// Add one reverb zone. Build-time only.
    ///
    /// Errors: [`GunfireError::TooManyZones`] past [`MAX_ZONES`].
    pub fn add_zone(&mut self, zone: Zone) -> Result<(), GunfireError> {
        if self.zones.len() >= MAX_ZONES {
            return Err(GunfireError::TooManyZones {
                got: self.zones.len().saturating_add(1),
            });
        }
        self.zones.push(zone);
        Ok(())
    }

    /// Fixed scene rate in Hz.
    #[must_use]
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// One source against one listener: direct gain, filter, delay, crack
    /// and tail. Borrows only; allocates nothing.
    ///
    /// Errors: [`GunfireError::Occlude`] from the wall query,
    /// [`GunfireError::Reverb`] from the zone params.
    pub fn report(
        &self,
        listener_cm: [i32; 2],
        source: Source,
    ) -> Result<AcousticReport, GunfireError> {
        let distance_cm = path_cm(source.pos_cm, listener_cm);
        let (near_cm, max_cm) = match source.kind {
            SourceKind::Gunshot | SourceKind::Supersonic => (GUNSHOT_NEAR_CM, GUNSHOT_MAX_CM),
            SourceKind::Shout => (SHOUT_NEAR_CM, SHOUT_MAX_CM),
        };
        let in_range = distance_cm <= max_cm;
        let distance_gain = if in_range {
            crate::spatial::falloff_gain(distance_cm, near_cm, max_cm)
        } else {
            0.0
        };

        let mut muffle = occlude::target_muffle(source.pos_cm, listener_cm, &self.walls)?;
        for door in &self.doors {
            if door.open
                || !occlude::segments_cross(
                    source.pos_cm,
                    listener_cm,
                    [door.wall.ax_cm, door.wall.az_cm],
                    [door.wall.bx_cm, door.wall.bz_cm],
                )
            {
                continue;
            }
            muffle.gain_q16 = occlude::apply_wall(muffle.gain_q16, door.wall.material);
            muffle.cutoff_hz = muffle.cutoff_hz.min(occlude::cutoff_hz(door.wall.material));
        }

        let mut occlusion_gain = occlude::q16_to_gain(muffle.gain_q16);
        let mut floor_applied = false;
        if source.kind == SourceKind::Shout {
            let floor = occlude::q16_to_gain(VOICE_FLOOR_Q16);
            if occlusion_gain < floor {
                occlusion_gain = floor;
                floor_applied = true;
            }
        }
        let gain = if in_range {
            distance_gain * occlusion_gain
        } else {
            0.0
        };

        let zone = self.zone_for(source.pos_cm, listener_cm);
        let reverb = zone.params(self.sample_rate_hz)?;
        let travel = Travel::new(distance_cm, self.sample_rate_hz);
        let crack = if source.kind == SourceKind::Supersonic && in_range {
            Some(self.crack(source, distance_cm, occlusion_gain, muffle.cutoff_hz))
        } else {
            None
        };

        Ok(AcousticReport {
            kind: source.kind,
            distance_cm,
            distance_gain,
            occlusion_gain,
            gain,
            cutoff_hz: muffle.cutoff_hz,
            delay_ms: travel.ms_f32(),
            delay_samples: travel.samples,
            floor_applied,
            zone: zone.kind,
            reverb,
            reverb_send: gain * reverb.wet,
            crack,
        })
    }

    /// Fill `out[..sources.len()]` with one report each, in source order.
    /// No heap: the caller owns the report buffer across ticks.
    ///
    /// Errors: [`GunfireError::BatchTooLarge`] past [`MAX_BATCH`],
    /// [`GunfireError::OutputTooSmall`], or a per-report refusal.
    pub fn report_batch(
        &self,
        listener_cm: [i32; 2],
        sources: &[Source],
        out: &mut [AcousticReport],
    ) -> Result<usize, GunfireError> {
        if sources.len() > MAX_BATCH {
            return Err(GunfireError::BatchTooLarge { got: sources.len() });
        }
        if out.len() < sources.len() {
            return Err(GunfireError::OutputTooSmall {
                got: out.len(),
                want: sources.len(),
            });
        }
        for (slot, source) in out.iter_mut().zip(sources) {
            *slot = self.report(listener_cm, *source)?;
        }
        Ok(sources.len())
    }

    /// Source zone, else listener zone, else the open street.
    fn zone_for(&self, source_cm: [i32; 2], listener_cm: [i32; 2]) -> Zone {
        self.zones
            .iter()
            .find(|zone| zone.contains(source_cm))
            .or_else(|| self.zones.iter().find(|zone| zone.contains(listener_cm)))
            .copied()
            .unwrap_or_else(|| Zone::street([0, 0]))
    }

    /// Shockwave at the round's closest approach.
    fn crack(
        &self,
        source: Source,
        distance_cm: u32,
        occlusion_gain: f32,
        cutoff_hz: u32,
    ) -> Crack {
        let miss_cm = source.miss_cm.min(distance_cm);
        let gain =
            crate::spatial::falloff_gain(miss_cm, CRACK_NEAR_CM, GUNSHOT_MAX_CM) * occlusion_gain;
        let travel = Travel::new(miss_cm, self.sample_rate_hz);
        let blast_ms = Travel::new(distance_cm, self.sample_rate_hz).ms_f32();
        Crack {
            delay_ms: travel.ms_f32(),
            delay_samples: travel.samples,
            lead_ms: (blast_ms - travel.ms_f32()).max(0.0),
            gain,
            cutoff_hz: cutoff_hz
                .saturating_add(CRACK_BRIGHTEN_HZ)
                .min(occlude::OPEN_CUTOFF_HZ),
        }
    }
}

/// Ground distance in cm; saturating, deterministic.
fn path_cm(a: [i32; 2], b: [i32; 2]) -> u32 {
    let dx = i64::from(a[0]) - i64::from(b[0]);
    let dz = i64::from(a[1]) - i64::from(b[1]);
    let squared = dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz));
    u32::try_from(crate::isqrt(squared.unsigned_abs())).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::OnePole;
    use crate::reverb::Reverb;
    use crate::spatial::falloff_gain;

    const RATE: u32 = 48_000;

    fn assert_ms(got: f32, want: f32) {
        assert!((got - want).abs() < 0.01, "delay {got} ms, want {want} ms");
    }

    fn scene() -> Acoustics {
        let mut scene = Acoustics::new(RATE).expect("48 kHz is supported");
        scene.add_zone(Zone::room([0, 0])).expect("zone fits");
        scene
            .add_wall(Wall::new(
                2_000,
                -1_000,
                2_000,
                1_000,
                occlude::Material::Stone,
            ))
            .expect("wall fits");
        scene
            .add_zone(Zone::street([10_000, 0]))
            .expect("zone fits");
        scene
    }

    fn open_scene() -> Acoustics {
        let mut scene = Acoustics::new(RATE).expect("48 kHz is supported");
        scene.add_zone(Zone::room([0, 0])).expect("zone fits");
        scene
            .add_zone(Zone::street([10_000, 0]))
            .expect("zone fits");
        scene
    }

    #[test]
    fn distance_filter_lowers_and_darkens_with_range() {
        let scene = open_scene();
        let near = scene
            .report([0, 0], Source::gunshot([200, 0]))
            .expect("near report");
        let far = scene
            .report([0, 0], Source::gunshot([5_000, 0]))
            .expect("far report");
        assert!(near.gain > far.gain, "near shot is louder");
        assert!(near.cutoff_hz > far.cutoff_hz, "near shot is brighter");
        assert!(
            near.delay_samples < far.delay_samples,
            "near shot arrives first"
        );
        assert_ms(far.delay_ms, 145.77);
    }

    #[test]
    fn supersonic_crack_leads_the_muzzle_blast() {
        let scene = open_scene();
        let round = scene
            .report([0, 0], Source::supersonic([10_000, 0], 200))
            .expect("supersonic report");
        let crack = round.crack.expect("supersonic round cracks");
        assert!(crack.lead_ms > 0.0, "the crack reaches the ears first");
        assert!(
            crack.delay_samples < round.delay_samples,
            "crack starts earlier"
        );
        assert!(crack.gain > round.distance_gain, "crack is close and loud");
        assert_ms(round.delay_ms, 291.55);
        assert_ms(crack.delay_ms, 5.83);

        let subsonic = scene
            .report([0, 0], Source::gunshot([10_000, 0]))
            .expect("subsonic report");
        assert!(subsonic.crack.is_none(), "no shockwave without supersonic");
    }

    #[test]
    fn room_and_street_tails_are_distinct() {
        let scene = open_scene();
        let room = scene
            .report([0, 0], Source::gunshot([300, 0]))
            .expect("room report");
        let street = scene
            .report([10_000, 0], Source::gunshot([10_300, 0]))
            .expect("street report");
        assert_eq!(room.zone, ZoneKind::Room);
        assert_eq!(street.zone, ZoneKind::Street);
        assert!(
            room.reverb.rt60_ms < street.reverb.rt60_ms,
            "street rings longer"
        );
        assert!(room.reverb.wet > street.reverb.wet, "room tail is wetter");
        assert!(room.reverb_send > 0.0 && street.reverb_send > 0.0);
    }

    #[test]
    fn closed_door_muffles_and_open_door_passes() {
        let mut scene = open_scene();
        let door = Door::closed(500, -300, 500, 300);
        scene.add_door(door).expect("door fits");
        let closed = scene
            .report([0, 0], Source::gunshot([2_000, 0]))
            .expect("closed door report");
        let mut open = open_scene();
        open.add_door(door.opened()).expect("door fits");
        let passed = open
            .report([0, 0], Source::gunshot([2_000, 0]))
            .expect("open door report");
        assert!(closed.gain < passed.gain, "a closed door ducks the shot");
        assert!(
            closed.cutoff_hz < passed.cutoff_hz,
            "a closed door darkens it"
        );
        assert_eq!(passed.occlusion_gain, 1.0, "an open door passes");
    }

    #[test]
    fn shout_stays_at_the_voice_floor_through_two_stone_walls() {
        let mut scene = open_scene();
        scene
            .add_wall(Wall::new(500, -1_000, 500, 1_000, occlude::Material::Stone))
            .expect("wall fits");
        scene
            .add_wall(Wall::new(
                1_500,
                -1_000,
                1_500,
                1_000,
                occlude::Material::Stone,
            ))
            .expect("wall fits");
        let walled = scene
            .report([0, 0], Source::shout([2_000, 0]))
            .expect("walled shout");
        let open = open_scene()
            .report([0, 0], Source::shout([2_000, 0]))
            .expect("open shout");
        let floor = occlude::q16_to_gain(VOICE_FLOOR_Q16);
        assert!(walled.floor_applied, "two stone walls would sink a shout");
        assert_eq!(walled.occlusion_gain, floor, "the voice floor holds");
        assert!(walled.gain < open.gain, "walled shout is quieter than open");
        assert!(walled.gain > 0.0, "but it is still heard through the rooms");
        assert!((walled.gain - open.distance_gain * floor).abs() < 1e-6);
    }

    #[test]
    fn batch_of_32_is_zero_alloc_and_replay_safe() {
        let scene = scene();
        let listener = [0, 0];
        let sources: Vec<Source> = (0..MAX_BATCH)
            .map(|i| {
                let x = 300 + i32::try_from(i).unwrap_or(0) * 200;
                if i % 3 == 0 {
                    Source::shout([x, 0])
                } else if i % 3 == 1 {
                    Source::supersonic([x, 0], 40 + u32::try_from(i).unwrap_or(0))
                } else {
                    Source::gunshot([x, 0])
                }
            })
            .collect();
        let first = scene
            .report(listener, sources[MAX_BATCH - 1])
            .expect("warm report");
        let mut out = [first; MAX_BATCH];
        let count = scene
            .report_batch(listener, &sources, &mut out)
            .expect("batch fits");
        assert_eq!(count, MAX_BATCH);
        let snapshot = out;

        let ((), heap) = engine_core::alloc_probe::measure(|| {
            let filled = scene
                .report_batch(listener, &sources, &mut out)
                .expect("batch fits");
            std::hint::black_box(filled);
        });
        assert_eq!(heap.calls, 0, "the tick batch must not allocate");
        let again = scene
            .report_batch(listener, &sources, &mut out)
            .expect("batch fits");
        assert_eq!(again, MAX_BATCH);
        assert_eq!(out, snapshot, "same inputs give the same reports");
    }

    #[test]
    fn batch_refusals_are_typed() {
        let scene = open_scene();
        let sources = [Source::gunshot([0, 0]); MAX_BATCH + 1];
        let mut out = [scene
            .report([0, 0], Source::gunshot([0, 0]))
            .expect("report"); MAX_BATCH];
        assert!(matches!(
            scene.report_batch([0, 0], &sources, &mut out),
            Err(GunfireError::BatchTooLarge { got: 33 })
        ));
        let short = [Source::gunshot([0, 0]); 2];
        let mut one = [out[0]; 1];
        assert!(matches!(
            scene.report_batch([0, 0], &short, &mut one),
            Err(GunfireError::OutputTooSmall { got: 1, want: 2 })
        ));
    }

    #[test]
    fn bad_sample_rate_is_refused() {
        assert!(matches!(
            Acoustics::new(1_000),
            Err(GunfireError::Reverb(ReverbError::BadSampleRate { .. }))
        ));
    }

    #[test]
    #[ignore = "timing: 32 reports/batch 0.2 ms budget probe, run under --perf in release"]
    fn budget_probe_32_reports() {
        let mut scene = open_scene();
        for i in 0..32 {
            let x = 300 + i * 40;
            scene
                .add_wall(Wall::new(x, -2_000, x, 2_000, occlude::Material::Stone))
                .expect("wall fits");
        }
        for i in 0..8 {
            let x = 900 + i * 300;
            scene
                .add_door(Door::closed(x, -100, x, 100))
                .expect("door fits");
        }
        let listener = [0, 0];
        let sources: Vec<Source> = (0..MAX_BATCH)
            .map(|i| {
                let x = 300 + i32::try_from(i).unwrap_or(0) * 200;
                if i % 3 == 0 {
                    Source::shout([x, 0])
                } else if i % 3 == 1 {
                    Source::supersonic([x, 0], 60)
                } else {
                    Source::gunshot([x, 0])
                }
            })
            .collect();
        let first = scene.report(listener, sources[0]).expect("report");
        let mut out = [first; MAX_BATCH];
        for _ in 0..64 {
            scene
                .report_batch(listener, &sources, &mut out)
                .expect("batch fits");
        }
        let rounds = 4_000u32;
        let start = std::time::Instant::now();
        let mut folded = 0i32;
        for _ in 0..rounds {
            scene
                .report_batch(listener, &sources, &mut out)
                .expect("batch fits");
            folded ^= out[0].cutoff_hz.to_le_bytes()[0] as i32;
        }
        std::hint::black_box(folded);
        let mean_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        eprintln!("gunfire: 32 reports mean {mean_ms:.4} ms (bar {BUDGET_MS_PER_BATCH} ms)");
        assert!(
            mean_ms < BUDGET_MS_PER_BATCH as f64,
            "mean {mean_ms} ms per 32-report batch over budget"
        );
    }

    // ---- blind listening fixture ------------------------------------------
    //
    // The judge below never sees a label: it reads rendered audio features
    // only and returns which of two unnamed buffers wins a property. The
    // test then checks that the winner is the scene the audio design meant.

    fn ms_samples(ms: u32, rate: u32) -> usize {
        usize::try_from(u64::from(ms) * u64::from(rate) / 1_000).unwrap_or(0)
    }

    fn noise_sample(i: usize) -> f32 {
        let mut x = u64::try_from(i)
            .unwrap_or(0)
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        x ^= x >> 33;
        crate::f32_of(u32::try_from((x >> 16) & 0xFFFF).unwrap_or(0)) / 32_768.0 - 1.0
    }

    fn one_pole(cutoff_hz: u32, rate: u32) -> OnePole {
        if cutoff_hz >= occlude::OPEN_CUTOFF_HZ {
            return OnePole::new(1.0);
        }
        let x = std::f32::consts::TAU * crate::f32_of(cutoff_hz) / crate::f32_of(rate);
        OnePole::new(1.0 - (-x).exp())
    }

    fn burst(out: &mut [f32], start: usize, len: usize, level: f32, cutoff_hz: u32, rate: u32) {
        let mut filter = one_pole(cutoff_hz, rate);
        let total = crate::f32_of(u32::try_from(len).unwrap_or(1));
        for i in 0..len {
            let Some(sample) = out.get_mut(start.saturating_add(i)) else {
                break;
            };
            let env = 1.0 - crate::f32_of(u32::try_from(i).unwrap_or(0)) / total;
            *sample += filter.step(noise_sample(start.saturating_add(i))) * level * env;
        }
    }

    fn shout(out: &mut [f32], start: usize, len: usize, level: f32, cutoff_hz: u32, rate: u32) {
        let mut filter = one_pole(cutoff_hz, rate);
        let total = crate::f32_of(u32::try_from(len).unwrap_or(1));
        let rate_f = crate::f32_of(rate);
        for i in 0..len {
            let Some(sample) = out.get_mut(start.saturating_add(i)) else {
                break;
            };
            let t = crate::f32_of(u32::try_from(i).unwrap_or(0)) / rate_f;
            let env = 1.0 - crate::f32_of(u32::try_from(i).unwrap_or(0)) / total;
            let voice = (std::f32::consts::TAU * 220.0 * t).sin() * 0.6
                + (std::f32::consts::TAU * 440.0 * t).sin() * 0.3;
            *sample += filter.step(voice) * level * env;
        }
    }

    fn render(scene: &Acoustics, listener: [i32; 2], source: Source, frames: usize) -> Vec<f32> {
        let report = scene.report(listener, source).expect("valid report");
        let mut out = vec![0.0f32; frames];
        let start = usize::try_from(report.delay_samples).unwrap_or(usize::MAX);
        let len = ms_samples(40, RATE);
        if source.kind == SourceKind::Shout {
            shout(&mut out, start, len, report.gain, report.cutoff_hz, RATE);
        } else {
            burst(&mut out, start, len, report.gain, report.cutoff_hz, RATE);
        }
        if let Some(crack) = report.crack {
            burst(
                &mut out,
                usize::try_from(crack.delay_samples).unwrap_or(usize::MAX),
                ms_samples(6, RATE),
                crack.gain,
                crack.cutoff_hz,
                RATE,
            );
        }
        let mut wet = vec![0.0f32; frames];
        let mut reverb = Reverb::new(&report.reverb).expect("params validate");
        reverb.process(&out, &mut wet).expect("lengths match");
        for (dry, tail) in out.iter_mut().zip(wet) {
            *dry = (*dry + tail).clamp(-1.0, 1.0);
        }
        out
    }

    #[derive(Clone, Copy, Debug)]
    struct Features {
        onset: usize,
        rms: f64,
        zero_crossings: u32,
        tail_ratio: f64,
    }

    fn features(buf: &[f32]) -> Features {
        let peak = buf.iter().fold(0.0f32, |top, x| top.max(x.abs()));
        let onset = buf.iter().position(|x| x.abs() > peak * 0.1).unwrap_or(0);
        let mut energy = 0.0f64;
        let mut late = 0.0f64;
        let split = onset.saturating_add(ms_samples(250, RATE));
        let mut crossings = 0u32;
        let mut last = 0.0f32;
        for (i, x) in buf.iter().enumerate().skip(onset) {
            let power = f64::from(*x) * f64::from(*x);
            energy += power;
            if i >= split {
                late += power;
            }
            if i > onset && ((last >= 0.0) != (*x >= 0.0)) && last.abs() + x.abs() > 1e-6 {
                crossings = crossings.saturating_add(1);
            }
            last = *x;
        }
        let tail_ratio = if energy > 0.0 { late / energy } else { 0.0 };
        Features {
            onset,
            rms: energy.sqrt(),
            zero_crossings: crossings,
            tail_ratio,
        }
    }

    /// Index of the farther source: later arrival, then quieter, then duller.
    fn judge_farther(a: &Features, b: &Features) -> usize {
        if a.onset != b.onset {
            return usize::from(a.onset <= b.onset);
        }
        if (a.rms - b.rms).abs() > 1e-9 {
            return usize::from(a.rms >= b.rms);
        }
        usize::from(a.zero_crossings >= b.zero_crossings)
    }

    /// Index of the buffer with more late energy.
    fn judge_longer_tail(a: &Features, b: &Features) -> usize {
        usize::from(a.tail_ratio <= b.tail_ratio)
    }

    /// Index of the quieter direct sound, then the duller one.
    fn judge_muffled(a: &Features, b: &Features) -> usize {
        if (a.rms - b.rms).abs() > 1e-9 {
            return usize::from(a.rms >= b.rms);
        }
        usize::from(a.zero_crossings >= b.zero_crossings)
    }

    fn energy_before(buf: &[f32], end: usize) -> f64 {
        buf.iter()
            .take(end)
            .map(|x| f64::from(*x) * f64::from(*x))
            .sum()
    }

    #[test]
    fn blind_listening_judge_names_every_pair() {
        let scene = open_scene();
        let listener = [0, 0];
        let frames = ms_samples(1_200, RATE);

        let near = features(&render(&scene, listener, Source::gunshot([500, 0]), frames));
        let far = features(&render(
            &scene,
            listener,
            Source::gunshot([5_000, 0]),
            frames,
        ));
        assert_eq!(judge_farther(&near, &far), 1, "judge hears the far shot");
        assert_eq!(
            judge_muffled(&near, &far),
            1,
            "judge hears the duller far shot"
        );

        let room = features(&render(&scene, listener, Source::gunshot([500, 0]), frames));
        let street = features(&render(
            &scene,
            [10_000, 0],
            Source::gunshot([10_500, 0]),
            frames,
        ));
        assert_eq!(
            judge_longer_tail(&room, &street),
            1,
            "judge hears the street slap"
        );

        let mut door_scene = open_scene();
        door_scene
            .add_door(Door::closed(800, -300, 800, 300))
            .expect("door fits");
        let closed = features(&render(
            &door_scene,
            listener,
            Source::gunshot([1_600, 0]),
            frames,
        ));
        let mut open_door_scene = open_scene();
        open_door_scene
            .add_door(Door::closed(800, -300, 800, 300).opened())
            .expect("door fits");
        let passed = features(&render(
            &open_door_scene,
            listener,
            Source::gunshot([1_600, 0]),
            frames,
        ));
        assert_eq!(
            judge_muffled(&closed, &passed),
            0,
            "judge hears the closed door"
        );

        let plain = scene
            .report(listener, Source::gunshot([4_000, 0]))
            .expect("plain report");
        let supersonic = scene
            .report(listener, Source::supersonic([4_000, 0], 80))
            .expect("crack report");
        let plain_audio = render(&scene, listener, Source::gunshot([4_000, 0]), frames);
        let crack_audio = render(&scene, listener, Source::supersonic([4_000, 0], 80), frames);
        let blast = usize::try_from(plain.delay_samples).unwrap_or(usize::MAX);
        let before_plain = energy_before(&plain_audio, blast);
        let before_crack = energy_before(&crack_audio, blast);
        assert!(
            before_crack > before_plain * 100.0,
            "judge hears the crack before the blast"
        );
        assert!(supersonic.crack.expect("crack").lead_ms > 0.0);

        let mut room_scene = open_scene();
        room_scene
            .add_wall(Wall::new(400, -1_000, 400, 1_000, occlude::Material::Stone))
            .expect("wall fits");
        room_scene
            .add_wall(Wall::new(
                1_200,
                -1_000,
                1_200,
                1_000,
                occlude::Material::Stone,
            ))
            .expect("wall fits");
        let open_shout = render(&open_scene(), listener, Source::shout([1_600, 0]), frames);
        let walled_shout = render(&room_scene, listener, Source::shout([1_600, 0]), frames);
        let open_rms = features(&open_shout).rms;
        let walled_rms = features(&walled_shout).rms;
        assert!(
            walled_rms > 0.001,
            "judge hears the shout through two rooms"
        );
        assert!(
            walled_rms < open_rms,
            "judge hears the rooms duck the shout"
        );
    }

    #[test]
    fn path_and_crack_stay_finite_at_extremes() {
        let scene = open_scene();
        let report = scene
            .report(
                [i32::MIN, i32::MIN],
                Source::supersonic([i32::MAX, i32::MAX], u32::MAX),
            )
            .expect("extreme path still answers");
        assert!(report.gain == 0.0, "past cull range is silent");
        assert!(report.crack.is_none(), "out of range has no crack");
        assert!(report.delay_ms.is_finite());
    }

    #[test]
    fn falloff_matches_the_shared_distance_owner() {
        assert_eq!(
            falloff_gain(GUNSHOT_NEAR_CM, GUNSHOT_NEAR_CM, GUNSHOT_MAX_CM),
            1.0
        );
        assert_eq!(
            falloff_gain(GUNSHOT_MAX_CM, GUNSHOT_NEAR_CM, GUNSHOT_MAX_CM),
            0.015
        );
    }
}
