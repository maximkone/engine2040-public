//! cue: sound cues over shared clips.
//!
//! A [`Cue`] is a set of clip variants plus how to play them: flat or
//! positional with a falloff range, a [`Group`] for instance limits, loop,
//! priority and [`Bus`]. The game plays cues by name; a [`CueBank`] turns the
//! name into a [`CueId`] once on the game thread, and the audio thread only
//! sees ids. Clip data is an `Arc<[f32]>`, so starting a voice copies a
//! pointer, never samples. Variant picks are seeded and never repeat the
//! previous pick, so a sequence of cues replays identically. Gregory, Game
//! Engine Architecture 13.5.7.2 (cues, not clips) and 13.5.8.4 (instance
//! limiting).

use std::sync::Arc;

use engine_core::{splitmix64, Rng};

use crate::bank::{Bank, Synth};
use crate::dsp::Bus;
use crate::playlist::MusicBank;
use crate::Pcm;

/// Most clip variants one cue holds.
pub const MAX_VARIANTS: usize = 4;
/// Longest resampled clip kept at load (16 Mi frames, 64 MiB mono); longer
/// sources keep their own rate instead of growing without bound.
pub const MAX_CLIP_FRAMES: usize = 1 << 24;
/// Falloff min for combat cues: full volume inside 8 m, so a fight in front
/// of an orbiting camera is not attenuated.
pub const COMBAT_NEAR_CM: u32 = 800;
/// Falloff max for combat cues: silent beyond 60 m.
pub const COMBAT_FAR_CM: u32 = 6_000;
/// Pack-vs-stream frame threshold: 4 s at 48 kHz. Longer clips stream.
pub const STREAM_FRAMES: usize = 192_000;
/// Pack-vs-stream byte threshold: 1 MiB decoded (f32 bytes). Larger clips
/// stream.
pub const STREAM_BYTES: usize = 1_048_576;

/// Pure pack-vs-stream decision: true when `frames > STREAM_FRAMES` or
/// `bytes * instances > STREAM_BYTES`, else false. Integer only, no clock,
/// no alloc; same inputs give the same bool. Short one-shots pack, long or
/// large beds stream (gems2 21.3 event-table rules).
pub fn should_stream(frames: usize, instances: usize, bytes: usize) -> bool {
    if frames > STREAM_FRAMES {
        return true;
    }
    bytes.saturating_mul(instances.max(1)) > STREAM_BYTES
}

/// Cue names the arena plays. The bank resolves them to ids once.
pub mod names {
    /// Blade or fist lands on flesh.
    pub const HIT: &str = "hit";
    /// Blow lands on a shield.
    pub const BLOCK: &str = "block";
    /// Blade meets blade.
    pub const PARRY: &str = "parry";
    /// Fighter goes down: heavy impact.
    pub const KNOCKDOWN: &str = "knockdown";
    /// Weapon whoosh.
    pub const SWING: &str = "swing";
    /// Footstep on arena sand (gravel recordings).
    pub const FOOTSTEP: &str = "footstep";
    /// Footstep on stone (tunnel).
    pub const FOOTSTEP_STONE: &str = "footstep_stone";
    /// Footstep on wood (platform).
    pub const FOOTSTEP_WOOD: &str = "footstep_wood";
    /// Footstep on grass.
    pub const FOOTSTEP_GRASS: &str = "footstep_grass";
    /// Footstep on metal (grate).
    pub const FOOTSTEP_METAL: &str = "footstep_metal";
    /// Footstep through water.
    pub const FOOTSTEP_WATER: &str = "footstep_water";
    /// Looping crowd bed.
    pub const CROWD: &str = "crowd";
    /// Stinger: a kill lands (low hit and a minor chord stab).
    pub const STINGER_KILL: &str = "stinger_kill";
    /// Stinger: the crowd roars (swelling noise).
    pub const STINGER_ROAR: &str = "stinger_roar";
    /// Weapon or body against the stone wall.
    pub const IMPACT_STONE: &str = "impact_stone";
    /// Blade on bone: a hard knock with a crack.
    pub const IMPACT_BONE: &str = "impact_bone";
    /// Blade on armor: a metal ring over a body thud.
    pub const IMPACT_ARMOR: &str = "impact_armor";
    /// Two blades bind on the same tick: a hard steel clash.
    pub const CLASH: &str = "clash";
    /// A counter blow off a parry: a fast cutting strike.
    pub const RIPOSTE: &str = "riposte";
    /// A blow severs a joint: the blade tears through.
    pub const SEVER: &str = "sever";
    /// A wet body blow: blood and tissue leave the body.
    pub const GORE: &str = "gore";
    /// A severed joint opens: wet spurt and bone crack.
    pub const STUMP: &str = "stump";
    /// Loaded structure creaks before it fails.
    pub const STRUCTURE_GROAN: &str = "structure_groan";
}

/// What happened on a blow, for the impact table below. The sim events carry
/// no surface or kind field; the presentation side names the cue from what it
/// already knows (the event, the move, the victim's kit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImpactKind {
    /// Blade or fist lands on a body.
    Hit,
    /// A held block soaks the blow.
    Block,
    /// A block inside the parry window: blade on blade.
    Parry,
    /// A fighter drops to the floor.
    Knockdown,
    /// Two attackers bind on the same tick.
    Clash,
    /// A counter blow off a parry.
    Riposte,
    /// A blow severs a joint.
    Sever,
    /// Blood and tissue leave the body.
    Gore,
}

impl ImpactKind {
    /// Number of kinds.
    pub const COUNT: usize = 8;
    /// Every kind, in [`Self::index`] order.
    pub const ALL: [ImpactKind; ImpactKind::COUNT] = [
        Self::Hit,
        Self::Block,
        Self::Parry,
        Self::Knockdown,
        Self::Clash,
        Self::Riposte,
        Self::Sever,
        Self::Gore,
    ];

    /// Dense index `0..COUNT` for per-kind tables.
    pub fn index(self) -> usize {
        match self {
            Self::Hit => 0,
            Self::Block => 1,
            Self::Parry => 2,
            Self::Knockdown => 3,
            Self::Clash => 4,
            Self::Riposte => 5,
            Self::Sever => 6,
            Self::Gore => 7,
        }
    }
}

/// What the blow landed on: picks the material family of the take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImpactSurface {
    /// Bare or cloth-covered body.
    Flesh,
    /// Leather or mail: a body thud with a soft ring.
    Mail,
    /// Plate: a metal ring over the body.
    Plate,
    /// Wooden shield.
    ShieldWood,
    /// Iron shield or helmet.
    ShieldIron,
    /// Stone wall or floor.
    Stone,
}

impl ImpactSurface {
    /// Number of surfaces.
    pub const COUNT: usize = 6;
    /// Every surface, in [`Self::index`] order.
    pub const ALL: [ImpactSurface; ImpactSurface::COUNT] = [
        Self::Flesh,
        Self::Mail,
        Self::Plate,
        Self::ShieldWood,
        Self::ShieldIron,
        Self::Stone,
    ];

    /// Dense index `0..COUNT` for per-surface tables.
    pub fn index(self) -> usize {
        match self {
            Self::Flesh => 0,
            Self::Mail => 1,
            Self::Plate => 2,
            Self::ShieldWood => 3,
            Self::ShieldIron => 4,
            Self::Stone => 5,
        }
    }
}

/// The impact table: one row per [`ImpactKind`], one column per
/// [`ImpactSurface`], each cell the arena cue that blow plays.
///
/// Rows are the kind of blow, columns the material it met. The Godot
/// `AudioStreamRandomizer` shape: the kind and the surface live on the
/// presentation side, so the sim event carries no extra field. Every kind
/// owns a family, so a clash never sounds like a parry and a riposte never
/// sounds like a hit; the surface then picks the material take inside that
/// family. Every cell names a cue of [`ARENA_CUES`].
///
/// Row order is [`ImpactKind::index`]; column order is
/// [`ImpactSurface::index`].
pub const IMPACT_CUES: [[&str; ImpactSurface::COUNT]; ImpactKind::COUNT] = [
    // Hit: a body blow takes the body sound; metal, wood and stone take
    // their own.
    [
        names::HIT,
        names::HIT,
        names::IMPACT_ARMOR,
        names::BLOCK,
        names::IMPACT_ARMOR,
        names::IMPACT_STONE,
    ],
    // Block: a held shield soaks the blow; iron and plate ring.
    [
        names::BLOCK,
        names::BLOCK,
        names::IMPACT_ARMOR,
        names::BLOCK,
        names::IMPACT_ARMOR,
        names::BLOCK,
    ],
    // Parry: blade on blade, whatever the kit.
    [names::PARRY; ImpactSurface::COUNT],
    // Knockdown: the body hits the floor; a wall takes the stone sound.
    [
        names::KNOCKDOWN,
        names::KNOCKDOWN,
        names::KNOCKDOWN,
        names::KNOCKDOWN,
        names::KNOCKDOWN,
        names::IMPACT_STONE,
    ],
    // Clash: two blades bind on the same tick.
    [names::CLASH; ImpactSurface::COUNT],
    // Riposte: a counter blow off a parry.
    [names::RIPOSTE; ImpactSurface::COUNT],
    // Sever: a joint leaves the body.
    [names::SEVER; ImpactSurface::COUNT],
    // Gore: blood and tissue leave the body.
    [names::GORE; ImpactSurface::COUNT],
];

/// The arena cue for one impact: the kind of blow and the material it hit.
///
/// A lookup in [`IMPACT_CUES`], so the table is the one place the mapping
/// lives. Never panics.
pub fn impact_cue(kind: ImpactKind, surface: ImpactSurface) -> &'static str {
    IMPACT_CUES[kind.index()][surface.index()]
}

/// Index of a cue inside its [`CueBank`]; stable because the bank sorts by
/// name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CueId(pub u16);

/// Instance-limit category: voices of one group compete for the group's
/// slots before they compete for the whole pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Hits, blocks, parries, knockdowns, wall impacts.
    Impact,
    /// Weapon whooshes.
    Swing,
    /// Footsteps.
    Footstep,
    /// Crowd beds and cheers.
    Crowd,
    /// Environment beds.
    Ambient,
    /// Music tracks.
    Music,
    /// Spoken lines.
    Dialogue,
    /// One-shot music stingers.
    Stinger,
}

impl Group {
    /// Number of groups.
    pub const COUNT: usize = 8;
    /// Every group, in [`Self::index`] order.
    pub const ALL: [Group; Group::COUNT] = [
        Self::Impact,
        Self::Swing,
        Self::Footstep,
        Self::Crowd,
        Self::Ambient,
        Self::Music,
        Self::Dialogue,
        Self::Stinger,
    ];

    /// Dense index `0..COUNT` for per-group tables.
    pub fn index(self) -> usize {
        match self {
            Self::Impact => 0,
            Self::Swing => 1,
            Self::Footstep => 2,
            Self::Crowd => 3,
            Self::Ambient => 4,
            Self::Music => 5,
            Self::Dialogue => 6,
            Self::Stinger => 7,
        }
    }

    /// Default simultaneous-instance limit: up to three speakers, a handful
    /// of effects and two overlapping music tracks (the book's example mix).
    pub fn default_limit(self) -> u8 {
        match self {
            Self::Impact => 8,
            Self::Swing => 4,
            Self::Footstep => 6,
            Self::Crowd => 2,
            Self::Ambient => 4,
            Self::Music => 2,
            Self::Dialogue => 3,
            Self::Stinger => 2,
        }
    }
}

/// How a cue is placed in the mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spatial {
    /// Same level in both ears (music, crowd bed, dialogue).
    Flat,
    /// Placed at a position: gain is 1 inside `falloff_min_cm`, falls as
    /// `min / distance` out to `falloff_max_cm`, then 0.
    Positional {
        /// Radius inside which the sound is not attenuated.
        falloff_min_cm: u32,
        /// Radius beyond which the sound is silent.
        falloff_max_cm: u32,
    },
}

/// One decoded clip with shared sample storage.
#[derive(Clone, Debug)]
pub struct Clip {
    /// Sample rate in Hz of `samples`.
    pub sample_rate: u32,
    /// Interleaved channel count, 1 or 2.
    pub channels: u16,
    /// Interleaved samples in `[-1, 1]`; cloning the clip clones the pointer.
    pub samples: Arc<[f32]>,
}

impl Clip {
    /// Copy `pcm` into shared storage once (load time).
    pub fn from_pcm(pcm: &Pcm) -> Self {
        Self {
            sample_rate: pcm.sample_rate,
            channels: pcm.channels,
            samples: Arc::from(pcm.samples.as_slice()),
        }
    }

    /// Like [`Self::from_pcm`] but resampled (linear interpolation) to
    /// `rate` Hz, because the mixer plays one sample per output frame. A
    /// matching rate, a zero rate, an empty source or a result past
    /// [`MAX_CLIP_FRAMES`] keeps the source rate.
    pub fn from_pcm_at_rate(pcm: &Pcm, rate: u32) -> Self {
        let from = pcm.sample_rate;
        if rate == 0 || from == 0 || from == rate || pcm.is_empty() {
            return Self::from_pcm(pcm);
        }
        let src_frames = pcm.frames();
        let scaled = u64::try_from(src_frames)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(rate))
            / u64::from(from);
        let out_frames = usize::try_from(scaled).unwrap_or(usize::MAX).max(1);
        if out_frames > MAX_CLIP_FRAMES {
            return Self::from_pcm(pcm);
        }
        let ch = usize::from(pcm.channels);
        let mut out = Vec::with_capacity(out_frames.saturating_mul(ch));
        for o in 0..out_frames {
            let pos = u64::try_from(o)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(from));
            let i0 = usize::try_from(pos / u64::from(rate))
                .unwrap_or(usize::MAX)
                .min(src_frames - 1);
            let i1 = (i0 + 1).min(src_frames - 1);
            let frac_q16 = ((pos % u64::from(rate)) << 16) / u64::from(rate);
            let frac = f32::from(u16::try_from(frac_q16).unwrap_or(u16::MAX)) / 65_536.0;
            for c in 0..ch {
                let a = pcm.samples[i0 * ch + c];
                let b = pcm.samples[i1 * ch + c];
                out.push(a + (b - a) * frac);
            }
        }
        Self {
            sample_rate: rate,
            channels: pcm.channels,
            samples: Arc::from(out),
        }
    }

    /// Whole frames in the clip.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
}

/// A playable sound: variants plus placement and mixing metadata.
#[derive(Clone, Debug)]
pub struct Cue {
    /// Unique name the game plays it by.
    pub name: String,
    /// 1 to [`MAX_VARIANTS`] alternative clips; one is picked per play.
    pub variants: Vec<Clip>,
    /// Flat or positional.
    pub spatial: Spatial,
    /// Instance-limit group.
    pub group: Group,
    /// Wrap around at the end of the clip until stopped.
    pub looped: bool,
    /// Higher survives voice stealing.
    pub priority: u8,
    /// Mix bus.
    pub bus: Bus,
    /// Base trim multiplied into every play's gain.
    pub gain: f32,
    /// While a voice of this cue sounds, the music bus ducks (dialogue,
    /// stingers).
    pub ducks_music: bool,
}

/// Why a [`Cue`] or [`CueBank`] was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CueError {
    /// The cue name is empty.
    EmptyName,
    /// A cue has no clip variants.
    NoVariants {
        /// Offending cue.
        name: String,
    },
    /// A cue has more than [`MAX_VARIANTS`] variants.
    TooManyVariants {
        /// Offending cue.
        name: String,
        /// Offered variant count.
        count: usize,
    },
    /// A variant has no frames or an unsupported channel count.
    BadClip {
        /// Offending cue.
        name: String,
        /// Variant index.
        variant: usize,
    },
    /// Falloff range is not `0 < min < max`.
    BadFalloff {
        /// Offending cue.
        name: String,
    },
    /// The base gain is negative or not finite.
    BadGain {
        /// Offending cue.
        name: String,
    },
    /// Two cues share a name.
    DuplicateName {
        /// Repeated name.
        name: String,
    },
    /// More cues than a [`CueId`] can index.
    TooManyCues {
        /// Offered cue count.
        count: usize,
    },
}

impl std::fmt::Display for CueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => write!(f, "cue: empty name"),
            Self::NoVariants { name } => write!(f, "cue {name}: no variants"),
            Self::TooManyVariants { name, count } => {
                write!(f, "cue {name}: {count} variants (max {MAX_VARIANTS})")
            }
            Self::BadClip { name, variant } => write!(f, "cue {name}: variant {variant} unusable"),
            Self::BadFalloff { name } => write!(f, "cue {name}: falloff needs 0 < min < max"),
            Self::BadGain { name } => write!(f, "cue {name}: gain must be finite and >= 0"),
            Self::DuplicateName { name } => write!(f, "cue {name}: duplicate name"),
            Self::TooManyCues { count } => write!(f, "cue bank: {count} cues exceed the id range"),
        }
    }
}

impl std::error::Error for CueError {}

impl Cue {
    /// Check the invariants the mixer relies on.
    fn validate(&self) -> Result<(), CueError> {
        let name = || self.name.clone();
        if self.name.is_empty() {
            return Err(CueError::EmptyName);
        }
        match self.variants.len() {
            0 => return Err(CueError::NoVariants { name: name() }),
            n if n > MAX_VARIANTS => {
                return Err(CueError::TooManyVariants {
                    name: name(),
                    count: n,
                })
            }
            _ => {}
        }
        for (variant, clip) in self.variants.iter().enumerate() {
            if clip.frames() == 0 || clip.channels == 0 || clip.channels > 2 {
                return Err(CueError::BadClip {
                    name: name(),
                    variant,
                });
            }
        }
        if let Spatial::Positional {
            falloff_min_cm,
            falloff_max_cm,
        } = self.spatial
        {
            if falloff_min_cm == 0 || falloff_min_cm >= falloff_max_cm {
                return Err(CueError::BadFalloff { name: name() });
            }
        }
        if !self.gain.is_finite() || self.gain < 0.0 {
            return Err(CueError::BadGain { name: name() });
        }
        Ok(())
    }
}

/// Recipe for one arena cue: which recorded files to load, and which
/// procedural voice stands in when there are none.
#[derive(Clone, Copy, Debug)]
pub struct CueSpec {
    /// Cue name, one of [`names`].
    pub name: &'static str,
    /// Clip paths under the audio content root; empty means synthesized.
    pub files: &'static [&'static str],
    /// Voice used when no listed file loads.
    pub synth: Synth,
    /// Placement.
    pub spatial: Spatial,
    /// Instance-limit group.
    pub group: Group,
    /// Loop until stopped.
    pub looped: bool,
    /// Stealing priority.
    pub priority: u8,
    /// Mix bus.
    pub bus: Bus,
    /// Base trim.
    pub gain: f32,
    /// Ducks the music bus while it sounds.
    pub ducks_music: bool,
}

/// Combat placement: [`COMBAT_NEAR_CM`] to [`COMBAT_FAR_CM`].
const COMBAT: Spatial = Spatial::Positional {
    falloff_min_cm: COMBAT_NEAR_CM,
    falloff_max_cm: COMBAT_FAR_CM,
};
/// Footstep placement: quieter sources, shorter range.
const STEPS: Spatial = Spatial::Positional {
    falloff_min_cm: 500,
    falloff_max_cm: 3_000,
};

/// Shorthand for one footstep cue over four recorded takes.
const fn step(name: &'static str, files: &'static [&'static str]) -> CueSpec {
    CueSpec {
        name,
        files,
        synth: Synth::Footstep,
        spatial: STEPS,
        group: Group::Footstep,
        looped: false,
        priority: 40,
        bus: Bus::Sfx,
        gain: 0.8,
        ducks_music: false,
    }
}

/// Shorthand for one stinger: a flat one-shot that ducks the music bed and
/// outranks everything else.
const fn stinger(name: &'static str, synth: Synth) -> CueSpec {
    CueSpec {
        name,
        files: &[],
        synth,
        spatial: Spatial::Flat,
        group: Group::Stinger,
        looped: false,
        priority: 230,
        bus: Bus::Sfx,
        gain: 0.9,
        ducks_music: true,
    }
}

/// Shorthand for one impact cue over three recorded sizes.
const fn impact(
    name: &'static str,
    files: &'static [&'static str],
    priority: u8,
    gain: f32,
) -> CueSpec {
    CueSpec {
        name,
        files,
        synth: Synth::Hit,
        spatial: COMBAT,
        group: Group::Impact,
        looped: false,
        priority,
        bus: Bus::Sfx,
        gain,
        ducks_music: false,
    }
}

/// Shorthand for one synthesized combat cue: no recordings yet, so the
/// procedural voice stands in.
const fn synthesized(name: &'static str, synth: Synth, priority: u8, gain: f32) -> CueSpec {
    CueSpec {
        name,
        files: &[],
        synth,
        spatial: COMBAT,
        group: Group::Impact,
        looped: false,
        priority,
        bus: Bus::Sfx,
        gain,
        ducks_music: false,
    }
}

/// Every cue the arena plays, mapped onto the CC0 clips in `content/audio`.
/// `swing`, `crowd`, the two stingers, the BX11 material/tissue takes
/// (`impact_bone`, `impact_armor`, `stump`, `structure_groan`) and the four
/// SN12 kind cues (`clash`, `riposte`, `sever`, `gore`) have no recording yet:
/// they are synthesized.
pub const ARENA_CUES: [CueSpec; 23] = [
    impact(
        names::HIT,
        &[
            "impacts/flesh-small.wav",
            "impacts/flesh-medium.wav",
            "impacts/flesh-large.wav",
        ],
        180,
        1.0,
    ),
    impact(
        names::BLOCK,
        &[
            "impacts/wood-small.wav",
            "impacts/wood-medium.wav",
            "impacts/wood-large.wav",
        ],
        160,
        0.9,
    ),
    impact(
        names::PARRY,
        &[
            "impacts/metal-small.wav",
            "impacts/metal-medium.wav",
            "impacts/metal-large.wav",
        ],
        170,
        0.9,
    ),
    impact(
        names::KNOCKDOWN,
        &[
            "impacts/flesh-large.wav",
            "impacts/wood-large.wav",
            "impacts/stone-large.wav",
            "impacts/metal-large.wav",
        ],
        200,
        1.0,
    ),
    impact(
        names::IMPACT_STONE,
        &[
            "impacts/stone-small.wav",
            "impacts/stone-medium.wav",
            "impacts/stone-large.wav",
        ],
        120,
        0.9,
    ),
    synthesized(names::IMPACT_BONE, Synth::Bone, 175, 1.0),
    synthesized(names::IMPACT_ARMOR, Synth::Armor, 165, 0.9),
    synthesized(names::STUMP, Synth::Stump, 185, 0.9),
    synthesized(names::STRUCTURE_GROAN, Synth::Groan, 110, 0.8),
    synthesized(names::CLASH, Synth::Clash, 172, 0.9),
    synthesized(names::RIPOSTE, Synth::Riposte, 168, 0.9),
    synthesized(names::SEVER, Synth::Sever, 178, 1.0),
    synthesized(names::GORE, Synth::Gore, 150, 0.8),
    CueSpec {
        name: names::SWING,
        files: &[],
        synth: Synth::Swing,
        spatial: Spatial::Positional {
            falloff_min_cm: 600,
            falloff_max_cm: 4_000,
        },
        group: Group::Swing,
        looped: false,
        priority: 100,
        bus: Bus::Sfx,
        gain: 0.7,
        ducks_music: false,
    },
    step(
        names::FOOTSTEP,
        &[
            "footsteps/gravel-01.wav",
            "footsteps/gravel-02.wav",
            "footsteps/gravel-03.wav",
            "footsteps/gravel-04.wav",
        ],
    ),
    step(
        names::FOOTSTEP_STONE,
        &[
            "footsteps/stone-01.wav",
            "footsteps/stone-02.wav",
            "footsteps/stone-03.wav",
            "footsteps/stone-04.wav",
        ],
    ),
    step(
        names::FOOTSTEP_WOOD,
        &[
            "footsteps/wood-01.wav",
            "footsteps/wood-02.wav",
            "footsteps/wood-03.wav",
            "footsteps/wood-04.wav",
        ],
    ),
    step(
        names::FOOTSTEP_GRASS,
        &[
            "footsteps/grass-01.wav",
            "footsteps/grass-02.wav",
            "footsteps/grass-03.wav",
            "footsteps/grass-04.wav",
        ],
    ),
    step(
        names::FOOTSTEP_METAL,
        &[
            "footsteps/metal-01.wav",
            "footsteps/metal-02.wav",
            "footsteps/metal-03.wav",
            "footsteps/metal-04.wav",
        ],
    ),
    step(
        names::FOOTSTEP_WATER,
        &[
            "footsteps/water-01.wav",
            "footsteps/water-02.wav",
            "footsteps/water-03.wav",
            "footsteps/water-04.wav",
        ],
    ),
    CueSpec {
        name: names::CROWD,
        files: &[],
        synth: Synth::Crowd,
        spatial: Spatial::Flat,
        group: Group::Crowd,
        looped: true,
        // A bed that stealing silences stays silent until someone replays
        // it, so it outranks every one-shot.
        priority: 220,
        bus: Bus::Sfx,
        gain: 0.6,
        ducks_music: false,
    },
    stinger(names::STINGER_KILL, Synth::Kill),
    stinger(names::STINGER_ROAR, Synth::Roar),
];

/// What a bank build found: how many files loaded, which listed files were
/// missing or undecodable, and which cues fell back to synthesis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Files decoded and kept.
    pub loaded_files: usize,
    /// Listed files that could not be read or decoded.
    pub missing_files: Vec<&'static str>,
    /// Cues built from synthesized voices (no file loaded for them).
    pub synthesized: Vec<&'static str>,
}

/// Immutable cue table shared by the game thread (names to ids) and the
/// audio thread (ids to clips).
#[derive(Clone, Debug)]
pub struct CueBank {
    cues: Vec<Cue>,
    music: Option<Arc<MusicBank>>,
}

impl CueBank {
    /// Validate and sort `cues` by name; ids follow the sorted order.
    ///
    /// # Errors
    ///
    /// The first [`CueError`] found: bad cue, duplicate name or too many cues.
    pub fn new(mut cues: Vec<Cue>) -> Result<Self, CueError> {
        if cues.len() > usize::from(u16::MAX) {
            return Err(CueError::TooManyCues { count: cues.len() });
        }
        for cue in &cues {
            cue.validate()?;
        }
        cues.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(pair) = cues.windows(2).find(|w| w[0].name == w[1].name) {
            return Err(CueError::DuplicateName {
                name: pair[0].name.clone(),
            });
        }
        Ok(Self { cues, music: None })
    }

    /// Build the arena table at `sample_rate`: each recorded file comes from
    /// `load` (relative path to a decoded clip, `None` when missing) and is
    /// resampled to the output rate; a cue none of whose files load is
    /// synthesized instead and listed in the report.
    ///
    /// # Errors
    ///
    /// A [`CueError`] when the assembled table is invalid.
    pub fn build(
        sample_rate: u32,
        mut load: impl FnMut(&'static str) -> Option<Pcm>,
    ) -> Result<(Self, LoadReport), CueError> {
        let mut report = LoadReport::default();
        let mut cues = Vec::with_capacity(ARENA_CUES.len());
        for spec in &ARENA_CUES {
            let mut variants = Vec::with_capacity(MAX_VARIANTS);
            for &file in spec.files {
                match load(file) {
                    Some(pcm) => {
                        #[cfg(feature = "device")]
                        let stream = crate::decode::pcm_should_stream(&pcm, 1);
                        #[cfg(not(feature = "device"))]
                        let stream =
                            should_stream(pcm.frames(), 1, pcm.samples.len().saturating_mul(4));
                        if stream {
                            // Streaming bed: keep the source rate so a long
                            // bed never grows through resampling.
                            variants.push(Clip::from_pcm(&pcm));
                        } else {
                            variants.push(Clip::from_pcm_at_rate(&pcm, sample_rate));
                        }
                        report.loaded_files += 1;
                    }
                    None => report.missing_files.push(file),
                }
            }
            if variants.is_empty() {
                for v in 0..spec.synth.variants() {
                    let pcm = Bank::synth(spec.synth, sample_rate, u8::try_from(v).unwrap_or(0));
                    variants.push(Clip::from_pcm(&pcm));
                }
                report.synthesized.push(spec.name);
            }
            cues.push(Cue {
                name: spec.name.to_string(),
                variants,
                spatial: spec.spatial,
                group: spec.group,
                looped: spec.looped,
                priority: spec.priority,
                bus: spec.bus,
                gain: spec.gain,
                ducks_music: spec.ducks_music,
            });
        }
        Ok((Self::new(cues)?, report))
    }

    /// The arena table with every cue synthesized: headless runs and the
    /// fallback when no content directory exists.
    ///
    /// # Errors
    ///
    /// A [`CueError`] when the table is invalid (never for the shipped table).
    pub fn placeholder(sample_rate: u32) -> Result<Self, CueError> {
        Self::build(sample_rate, |_| None).map(|(bank, _)| bank)
    }

    /// Attach the tension playlists the engine's music bed plays; without
    /// them tension commands are ignored.
    #[must_use]
    pub fn with_music(mut self, music: MusicBank) -> Self {
        self.music = Some(Arc::new(music));
        self
    }

    /// The tension playlists, when attached.
    pub fn music(&self) -> Option<&Arc<MusicBank>> {
        self.music.as_ref()
    }

    /// Id of the cue called `name`, `None` when unknown.
    pub fn id(&self, name: &str) -> Option<CueId> {
        let idx = self
            .cues
            .binary_search_by(|cue| cue.name.as_str().cmp(name))
            .ok()?;
        u16::try_from(idx).ok().map(CueId)
    }

    /// The cue behind `id`.
    pub fn cue(&self, id: CueId) -> Option<&Cue> {
        self.cues.get(usize::from(id.0))
    }

    /// Cue count.
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    /// True when the bank holds no cues.
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Cue names in id order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.cues.iter().map(|cue| cue.name.as_str())
    }
}

/// One routed impact: the cue that plays, its id in the bank, and the take
/// of that cue to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoutedImpact {
    /// Cue name in the bank.
    pub name: &'static str,
    /// Cue id in the bank.
    pub id: CueId,
    /// Take of that cue to play, in `0..variants`.
    pub take: usize,
}

/// The impact table on a live bank: the kind of blow and the material it
/// met name a cue, and a round robin picks the take.
///
/// The presentation side owns this whole choice (the Godot
/// `AudioStreamRandomizer` shape), so the sim event carries no kind and no
/// surface field. The router holds one round-robin cycle per bank cue, so
/// two blows of the same kind walk the takes in turn and never play the
/// same take twice in a row.
///
/// One router per session: it owns the cycles, so it allocates once at
/// build time and never on a play. A play is O(1) and touches no heap.
///
/// Cloneable and debuggable so the window can hold one beside its other
/// presentation state (see `engine_game::window_audio_space::AudioSpace`):
/// a clone of a mid-cycle router continues the same cycle.
#[derive(Clone, Debug)]
pub struct ImpactRouter {
    bank: Arc<CueBank>,
    cycles: Vec<VariantCycle>,
}

impl ImpactRouter {
    /// Router over `bank`: one round-robin cycle per cue, started from
    /// `seed`, so the same seed replays the same takes.
    pub fn new(bank: Arc<CueBank>, seed: u64) -> Self {
        let cycles = (0..bank.len())
            .map(|idx| VariantCycle::new(seed, CueId(u16::try_from(idx).unwrap_or(0))))
            .collect();
        Self { bank, cycles }
    }

    /// The bank the router names cues against.
    pub fn bank(&self) -> &Arc<CueBank> {
        &self.bank
    }

    /// Route one blow: the cue the (kind, surface) table names, its id and
    /// the next take of that cue. `None` when the bank lacks the cue.
    pub fn route(&mut self, kind: ImpactKind, surface: ImpactSurface) -> Option<RoutedImpact> {
        let name = impact_cue(kind, surface);
        let id = self.bank.id(name)?;
        let cue = self.bank.cue(id)?;
        let take = self
            .cycles
            .get_mut(usize::from(id.0))?
            .next_take(cue.variants.len());
        Some(RoutedImpact { name, id, take })
    }

    /// Route one blow the caller already named: the id of the cue called
    /// `name` in the bank and the next take of it. `None` when the bank
    /// lacks the cue.
    ///
    /// The window knows the cue name (the table named it) but not the
    /// (kind, surface) pair that produced it, so it routes by name. The
    /// round robin is the same one [`Self::route`] walks: one cycle per cue,
    /// so two blows of the same cue take different takes in turn.
    pub fn route_name(&mut self, name: &str) -> Option<(CueId, usize)> {
        let id = self.bank.id(name)?;
        let cue = self.bank.cue(id)?;
        let take = self
            .cycles
            .get_mut(usize::from(id.0))?
            .next_take(cue.variants.len());
        Some((id, take))
    }
}

/// Seeded variant choice for one cue: uniform over the variants except the
/// one picked last, so a cue never plays the same take twice in a row.
#[derive(Clone, Copy, Debug)]
pub struct VariantPicker {
    rng: Rng,
    last: Option<usize>,
}

impl VariantPicker {
    /// Picker for `cue`, its own stream forked from `seed` so other cues'
    /// plays never shift this cue's picks.
    pub fn new(seed: u64, cue: CueId) -> Self {
        Self::for_stream(seed, u64::from(cue.0))
    }

    /// Picker on its own numbered stream of `seed`, for choices that are
    /// not cues (one stream per music level).
    pub fn for_stream(seed: u64, stream: u64) -> Self {
        Self {
            rng: Rng::from_state(splitmix64(seed)).fork(stream),
            last: None,
        }
    }

    /// Next variant index in `0..variants` (0 when there is no choice).
    pub fn pick(&mut self, variants: usize) -> usize {
        if variants < 2 {
            self.last = Some(0);
            return 0;
        }
        // The first pick is uniform over every variant; later picks are
        // uniform over the others, skipping the one played last.
        let others = if self.last.is_some() {
            variants - 1
        } else {
            variants
        };
        let span = u64::try_from(others).unwrap_or(1);
        let mut idx = usize::try_from(self.rng.next_range(0, span)).unwrap_or(0);
        if self.last.is_some_and(|last| idx >= last) {
            idx += 1;
        }
        self.last = Some(idx);
        idx
    }
}

/// Round-robin variant choice: the next take in a fixed cycle, so a cue
/// walks its variants in order and never plays the same take twice in a row.
///
/// The seeded [`VariantPicker`] spreads takes at random; the round robin
/// spreads them evenly. The impact table uses this one, so a busy fight
/// hears every take of a cue in turn instead of clumping on one. The
/// starting point is a seed, so a replay starts at the same take. One
/// counter per cue, no allocation, no clock: the same call sequence gives
/// the same takes twice.
#[derive(Clone, Copy, Debug)]
pub struct VariantCycle {
    start: usize,
    played: usize,
}

impl VariantCycle {
    /// Cycle for one cue, starting at a seed-derived take. The start is
    /// `seed` scrambled, so two seeds start at different takes.
    pub fn new(seed: u64, cue: CueId) -> Self {
        let scramble = splitmix64(seed ^ u64::from(cue.0));
        Self {
            start: usize::try_from(scramble % u64::try_from(MAX_VARIANTS).unwrap_or(1))
                .unwrap_or(0),
            played: 0,
        }
    }

    /// Next variant index in `0..variants` (0 when there is no choice). The
    /// index walks the cycle, so a cue with three takes plays its takes in
    /// order and wraps, never the same take twice in a row.
    pub fn next_take(&mut self, variants: usize) -> usize {
        if variants < 2 {
            self.played = 0;
            return 0;
        }
        let idx = (self.start + self.played) % variants;
        self.played = self.played.wrapping_add(1);
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(value: f32, frames: usize) -> Clip {
        Clip::from_pcm(&Pcm::new(48_000, 1, vec![value; frames]))
    }

    fn cue(name: &str, variants: Vec<Clip>) -> Cue {
        Cue {
            name: name.to_string(),
            variants,
            spatial: Spatial::Flat,
            group: Group::Impact,
            looped: false,
            priority: 10,
            bus: Bus::Sfx,
            gain: 1.0,
            ducks_music: false,
        }
    }

    #[test]
    fn bank_sorts_by_name_and_resolves_ids() {
        let bank = CueBank::new(vec![
            cue("zeta", vec![clip(0.1, 4)]),
            cue("alpha", vec![clip(0.2, 4)]),
        ])
        .expect("two valid cues");
        assert_eq!(bank.id("alpha"), Some(CueId(0)));
        assert_eq!(bank.id("zeta"), Some(CueId(1)));
        assert_eq!(bank.id("missing"), None);
        assert_eq!(bank.names().collect::<Vec<_>>(), vec!["alpha", "zeta"]);
        assert_eq!(bank.cue(CueId(1)).map(|c| c.name.as_str()), Some("zeta"));
        assert!(bank.cue(CueId(9)).is_none());
    }

    #[test]
    fn invalid_cues_are_refused_with_typed_errors() {
        let ok = vec![clip(0.1, 4)];
        let five = vec![clip(0.1, 4); MAX_VARIANTS + 1];
        let mut inverted = cue("far", ok.clone());
        inverted.spatial = Spatial::Positional {
            falloff_min_cm: 500,
            falloff_max_cm: 500,
        };
        let mut zero_min = inverted.clone();
        zero_min.spatial = Spatial::Positional {
            falloff_min_cm: 0,
            falloff_max_cm: 500,
        };
        let mut loud = cue("loud", ok.clone());
        loud.gain = f32::NAN;
        let silent = Clip {
            sample_rate: 48_000,
            channels: 1,
            samples: Arc::from(Vec::new()),
        };
        let cases = [
            (cue("", ok.clone()), CueError::EmptyName),
            (
                cue("none", vec![]),
                CueError::NoVariants {
                    name: "none".into(),
                },
            ),
            (
                cue("many", five),
                CueError::TooManyVariants {
                    name: "many".into(),
                    count: MAX_VARIANTS + 1,
                },
            ),
            (
                cue("empty", vec![silent]),
                CueError::BadClip {
                    name: "empty".into(),
                    variant: 0,
                },
            ),
            (inverted, CueError::BadFalloff { name: "far".into() }),
            (zero_min, CueError::BadFalloff { name: "far".into() }),
            (
                loud,
                CueError::BadGain {
                    name: "loud".into(),
                },
            ),
        ];
        for (bad, want) in cases {
            let got = CueBank::new(vec![bad]).expect_err("cue must be refused");
            assert_eq!(got, want);
            assert!(!got.to_string().is_empty());
        }
        let dup = CueBank::new(vec![cue("a", ok.clone()), cue("a", ok)]);
        assert_eq!(
            dup.unwrap_err(),
            CueError::DuplicateName { name: "a".into() }
        );
    }

    #[test]
    fn picker_replays_per_seed_and_never_repeats() {
        let run = |seed: u64| {
            let mut p = VariantPicker::new(seed, CueId(3));
            (0..64).map(|_| p.pick(4)).collect::<Vec<_>>()
        };
        let a = run(2040);
        assert_eq!(a, run(2040), "same seed, same picks");
        assert_ne!(a, run(2041), "adjacent seeds give different streams");
        assert!(
            a.windows(2).all(|w| w[0] != w[1]),
            "no immediate repeat: {a:?}"
        );
        for v in 0..4 {
            assert!(a.contains(&v), "variant {v} never picked in 64 draws");
        }
        let b: Vec<usize> = {
            let mut p = VariantPicker::new(2040, CueId(4));
            (0..64).map(|_| p.pick(4)).collect()
        };
        assert_ne!(a, b, "each cue draws from its own stream");
        let mut single = VariantPicker::new(1, CueId(0));
        assert!((0..8).all(|_| single.pick(1) == 0));
        assert_eq!(single.pick(0), 0);
    }

    #[test]
    fn picker_two_variants_alternate() {
        let mut p = VariantPicker::new(7, CueId(0));
        let picks: Vec<usize> = (0..8).map(|_| p.pick(2)).collect();
        assert!(picks.windows(2).all(|w| w[0] != w[1]), "{picks:?}");
    }

    #[test]
    fn resample_changes_length_and_interpolates() {
        let ramp = Pcm::new(8_000, 1, vec![0.0, 0.5, 1.0, 0.5]);
        let up = Clip::from_pcm_at_rate(&ramp, 16_000);
        assert_eq!((up.sample_rate, up.frames()), (16_000, 8));
        assert_eq!(&up.samples[..5], &[0.0, 0.25, 0.5, 0.75, 1.0]);
        let same = Clip::from_pcm_at_rate(&ramp, 8_000);
        assert_eq!(&*same.samples, ramp.samples.as_slice());
        let down = Clip::from_pcm_at_rate(&ramp, 4_000);
        assert_eq!(down.frames(), 2);
        let stereo = Pcm::new(8_000, 2, vec![0.0, 1.0, 1.0, 0.0]);
        let wide = Clip::from_pcm_at_rate(&stereo, 16_000);
        assert_eq!((wide.channels, wide.frames()), (2, 4));
        assert_eq!(&wide.samples[..4], &[0.0, 1.0, 0.5, 0.5]);
        let empty = Clip::from_pcm_at_rate(&Pcm::new(8_000, 1, vec![]), 48_000);
        assert_eq!((empty.sample_rate, empty.frames()), (8_000, 0));
        assert_eq!(Clip::from_pcm_at_rate(&ramp, 0).sample_rate, 8_000);
    }

    #[test]
    fn placeholder_bank_has_every_arena_cue_with_valid_variants() {
        let bank = CueBank::placeholder(48_000).expect("shipped table is valid");
        assert_eq!(bank.len(), ARENA_CUES.len());
        for spec in &ARENA_CUES {
            let id = bank
                .id(spec.name)
                .unwrap_or_else(|| panic!("{} missing", spec.name));
            let cue = bank.cue(id).expect("id resolves");
            assert!(
                (1..=MAX_VARIANTS).contains(&cue.variants.len()),
                "{}",
                spec.name
            );
            assert!(cue
                .variants
                .iter()
                .all(|c| c.sample_rate == 48_000 && c.frames() > 0));
            assert_eq!(cue.looped, spec.looped);
        }
        let hit = bank
            .cue(bank.id(names::HIT).expect("hit"))
            .expect("hit cue");
        assert_ne!(
            hit.variants[0].samples, hit.variants[1].samples,
            "synth variants differ"
        );
    }

    #[test]
    fn build_reports_missing_files_and_keeps_partial_loads() {
        let (bank, report) = CueBank::build(48_000, |file| {
            file.ends_with("-01.wav")
                .then(|| Pcm::new(24_000, 1, vec![0.5; 10]))
        })
        .expect("table builds");
        let step = bank
            .cue(bank.id(names::FOOTSTEP).expect("footstep"))
            .expect("cue");
        assert_eq!(step.variants.len(), 1, "only the -01 take loaded");
        assert_eq!(step.variants[0].sample_rate, 48_000, "resampled to output");
        assert_eq!(step.variants[0].frames(), 20);
        assert!(report.missing_files.contains(&"footsteps/gravel-02.wav"));
        assert_eq!(report.loaded_files, 6, "six footstep cues list a -01 file");
        assert!(
            report.synthesized.contains(&names::HIT),
            "no hit file loaded"
        );
        assert!(!report.synthesized.contains(&names::FOOTSTEP));
    }

    #[test]
    fn first_pick_is_uniform_over_every_variant() {
        let firsts: std::collections::BTreeSet<usize> = (0..200u64)
            .map(|seed| VariantPicker::new(seed, CueId(0)).pick(4))
            .collect();
        assert_eq!(firsts.into_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        let two: std::collections::BTreeSet<usize> = (0..50u64)
            .map(|seed| VariantPicker::for_stream(seed, 9).pick(2))
            .collect();
        assert_eq!(
            two.len(),
            2,
            "with two takes the seed still picks the first"
        );
    }

    #[test]
    fn stingers_are_flat_music_ducking_one_shots_in_their_own_group() {
        let bank = CueBank::placeholder(8_000).expect("shipped table");
        for name in [names::STINGER_KILL, names::STINGER_ROAR] {
            let cue = bank.cue(bank.id(name).expect("cue")).expect("id");
            assert_eq!(cue.group, Group::Stinger);
            assert!(cue.ducks_music && !cue.looped);
            assert_eq!(cue.spatial, Spatial::Flat);
            assert!(cue.priority > 200, "stingers outrank effects");
        }
        for spec in &ARENA_CUES {
            assert_eq!(
                spec.ducks_music,
                spec.group == Group::Stinger,
                "{} ducks only if it is a stinger",
                spec.name
            );
        }
        assert_eq!(Group::ALL.len(), Group::COUNT);
        for (i, group) in Group::ALL.into_iter().enumerate() {
            assert_eq!(group.index(), i);
        }
    }

    #[test]
    fn eg21_pack_short_oneshot_packs() {
        assert!(!should_stream(12_000, 3, 48_000), "0.25 s hit packs");
    }

    #[test]
    fn eg21_pack_single_small_packs_large_streams() {
        assert!(!should_stream(4_000, 1, 16_000), "single small clip packs");
        assert!(
            should_stream(4_000, 1, 5_000_000),
            "large byte size streams"
        );
    }

    #[test]
    fn eg21_pack_streamed_bed_keeps_source_rate() {
        let src_rate = 8_000;
        let out_rate = 48_000;
        let big = STREAM_FRAMES + 1_000;
        let (bank, report) = CueBank::build(out_rate, |file| {
            (file == "impacts/flesh-small.wav").then(|| Pcm::new(src_rate, 1, vec![0.5; big]))
        })
        .expect("table builds");
        let hit = bank.cue(bank.id(names::HIT).expect("hit")).expect("cue");
        assert_eq!(hit.variants.len(), 1, "only the big take loads");
        assert_eq!(
            hit.variants[0].sample_rate, src_rate,
            "streamed bed keeps source rate, no resample growth"
        );
        assert_eq!(report.loaded_files, 1);
    }

    #[test]
    fn impact_matrix_covers_every_kind_on_the_shipped_bank() {
        let bank = CueBank::placeholder(48_000).expect("shipped table is valid");
        assert_eq!(ImpactKind::ALL.len(), ImpactKind::COUNT);
        assert_eq!(ImpactSurface::ALL.len(), ImpactSurface::COUNT);
        for (index, kind) in ImpactKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.index(), index);
            for (at, surface) in ImpactSurface::ALL.into_iter().enumerate() {
                assert_eq!(surface.index(), at);
                let name = impact_cue(kind, surface);
                assert!(
                    bank.id(name).is_some(),
                    "{kind:?} on {surface:?} names {name} outside the bank"
                );
            }
        }
    }

    #[test]
    fn impact_matrix_reads_both_kind_and_surface() {
        let flesh = impact_cue(ImpactKind::Hit, ImpactSurface::Flesh);
        let wood = impact_cue(ImpactKind::Hit, ImpactSurface::ShieldWood);
        let iron = impact_cue(ImpactKind::Hit, ImpactSurface::ShieldIron);
        assert_eq!(
            (flesh, wood, iron),
            (names::HIT, names::BLOCK, names::IMPACT_ARMOR),
            "the surface picks the material take"
        );
        assert_eq!(
            impact_cue(ImpactKind::Block, ImpactSurface::ShieldIron),
            names::IMPACT_ARMOR,
            "a block on iron rings like metal"
        );
        assert_eq!(
            impact_cue(ImpactKind::Block, ImpactSurface::ShieldWood),
            names::BLOCK
        );
        assert_eq!(
            impact_cue(ImpactKind::Clash, ImpactSurface::Flesh),
            names::CLASH,
            "a clash is blade on blade whatever the kit"
        );
        assert_eq!(
            impact_cue(ImpactKind::Knockdown, ImpactSurface::Stone),
            names::IMPACT_STONE,
            "a body on a wall takes the stone sound"
        );
        assert_eq!(
            impact_cue(ImpactKind::Sever, ImpactSurface::Flesh),
            names::SEVER,
            "sever has its own cue"
        );
        assert_eq!(
            impact_cue(ImpactKind::Parry, ImpactSurface::Plate),
            impact_cue(ImpactKind::Parry, ImpactSurface::Plate),
            "same inputs, same play"
        );
    }

    /// SN12: every kind owns a family, so a clash never sounds like a
    /// parry and a riposte never sounds like a hit. The eight kinds name
    /// eight different cues on the shipped bank.
    #[test]
    fn every_impact_kind_owns_its_own_cue() {
        let bank = CueBank::placeholder(48_000).expect("shipped table is valid");
        let mut families = std::collections::BTreeSet::new();
        for kind in ImpactKind::ALL {
            let name = impact_cue(kind, ImpactSurface::Flesh);
            assert!(
                bank.id(name).is_some(),
                "{kind:?} names {name} outside the bank"
            );
            families.insert(name);
        }
        assert_eq!(
            families.len(),
            ImpactKind::COUNT,
            "one family per kind: {families:?}"
        );
        for (kind, want) in [
            (ImpactKind::Hit, names::HIT),
            (ImpactKind::Block, names::BLOCK),
            (ImpactKind::Parry, names::PARRY),
            (ImpactKind::Knockdown, names::KNOCKDOWN),
            (ImpactKind::Clash, names::CLASH),
            (ImpactKind::Riposte, names::RIPOSTE),
            (ImpactKind::Sever, names::SEVER),
            (ImpactKind::Gore, names::GORE),
        ] {
            assert_eq!(
                impact_cue(kind, ImpactSurface::Flesh),
                want,
                "{kind:?} plays {want}"
            );
        }
    }

    /// The round robin walks the takes of a cue in order and never plays
    /// the same take twice in a row. Same seed, same takes.
    #[test]
    fn the_round_robin_walks_the_takes_and_never_repeats() {
        let mut cycle = VariantCycle::new(2040, CueId(3));
        let picks: Vec<usize> = (0..12).map(|_| cycle.next_take(3)).collect();
        assert!(
            picks.windows(2).all(|w| w[0] != w[1]),
            "no immediate repeat: {picks:?}"
        );
        let mut seen = std::collections::BTreeSet::new();
        for take in &picks {
            seen.insert(*take);
        }
        assert_eq!(seen.len(), 3, "every take is heard: {picks:?}");
        let mut again = VariantCycle::new(2040, CueId(3));
        let replay: Vec<usize> = (0..12).map(|_| again.next_take(3)).collect();
        assert_eq!(picks, replay, "same seed, same takes");
        let mut other = VariantCycle::new(2041, CueId(3));
        let differs: Vec<usize> = (0..12).map(|_| other.next_take(3)).collect();
        assert_ne!(picks, differs, "another seed starts elsewhere");
        let mut single = VariantCycle::new(1, CueId(0));
        assert!((0..8).all(|_| single.next_take(1) == 0));
        assert_eq!(single.next_take(0), 0);
    }

    /// The router walks the whole (kind, surface) table over the shipped
    /// bank: every cell names a cue, every kind owns a family, and the
    /// round robin never plays the same take twice in a row.
    #[test]
    fn the_router_walks_the_table_over_the_shipped_bank() {
        let bank = Arc::new(CueBank::placeholder(48_000).expect("shipped table is valid"));
        let mut router = ImpactRouter::new(Arc::clone(&bank), 2040);
        let mut last: Option<(CueId, usize)> = None;
        let mut kinds = std::collections::BTreeSet::new();
        let mut takes = std::collections::BTreeSet::new();
        for _round in 0..2 {
            for kind in ImpactKind::ALL {
                for surface in ImpactSurface::ALL {
                    let routed = router
                        .route(kind, surface)
                        .expect("the table names a bank cue");
                    assert_eq!(routed.name, impact_cue(kind, surface));
                    let cue = bank.cue(routed.id).expect("id resolves");
                    assert!(routed.take < cue.variants.len());
                    kinds.insert(kind.index());
                    takes.insert((routed.id, routed.take));
                    if let Some((id, take)) = last {
                        assert!(
                            id != routed.id || take != routed.take,
                            "the same take twice in a row: {kind:?} on {surface:?}"
                        );
                    }
                    last = Some((routed.id, routed.take));
                }
            }
        }
        assert_eq!(kinds.len(), ImpactKind::COUNT, "8 of 8 kinds routed");
        assert!(
            takes.len() >= 12,
            "the table reaches 12 or more distinct takes: {}",
            takes.len()
        );
    }

    /// The window knows the cue name, not the (kind, surface) pair, so it
    /// routes by name. The round robin is the same one the table route
    /// walks: two blows of the same cue take different takes in turn, and
    /// the same seed replays the same takes.
    #[test]
    fn routing_by_name_walks_the_same_round_robin() {
        let bank = Arc::new(CueBank::placeholder(48_000).expect("shipped table is valid"));
        let mut router = ImpactRouter::new(Arc::clone(&bank), 2040);
        let name = impact_cue(ImpactKind::Hit, ImpactSurface::Flesh);
        let mut takes = Vec::new();
        for _ in 0..6 {
            let (id, take) = router.route_name(name).expect("the bank carries the cue");
            let cue = bank.cue(id).expect("id resolves");
            assert!(take < cue.variants.len());
            takes.push((id, take));
        }
        assert!(
            takes.windows(2).all(|w| w[0] != w[1]),
            "the same take twice in a row: {takes:?}"
        );
        let mut again = ImpactRouter::new(Arc::clone(&bank), 2040);
        let replay: Vec<(CueId, usize)> = (0..6)
            .map(|_| again.route_name(name).expect("the bank carries the cue"))
            .collect();
        assert_eq!(takes, replay, "same seed, same takes");
        assert!(
            router.route_name("no_such_cue").is_none(),
            "a name the bank lacks routes nothing"
        );
    }
}
