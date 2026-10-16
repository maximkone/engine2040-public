//! Fork replay buffer: deterministic episodes feeding the offline learner.
//!
//! Consumer: the offline learner maps the buffer to [`Sample`]s with
//! [`ReplayBuffer::to_samples`], trains under the `train` feature, exports
//! with [`export`](crate::export) and scores the bytes with
//! [`tournament`](crate::tournament) — the DT3 tournament shape. Owner: this
//! module owns the episode schema, the seeded collector and the buffer
//! digest. Units: features are unitless `f32` on exact `1/16` steps in
//! `[-8, 8)`; actions are class ids (`u8`); the digest is a `u64` content
//! hash. Determinism scope: the same seed plus the same [`ReplayConfig`]
//! collects the same episodes on every machine — fixed stream order, integer
//! lane math, no wall clock and no threads. Errors: every bad shape or
//! teacher id maps to [`ReplayError`]; [`ReplayBuffer::collect`] never
//! panics.
//!
//! ## Determinism owners
//!
//! Seed expansion and the content hash reuse the engine owners
//! (`engine_core::{Fnv64, Rng, hash::splitmix64}`): episode streams fork
//! like every other subsystem, and the digest is FNV-1a 64 over the same
//! little-endian integers on every machine. The arena fork itself stays
//! behind the teacher callback: any teacher (script, provider, recorded
//! human play) labels plain feature rows, exactly like [`Sample`]
//! collection.

use crate::data::{Sample, INPUT_QUANT_SCALE, MAX_FEATURE_ABS};
use crate::spec::StudentSpec;
use crate::tournament::{MatchScript, TournamentReport};
use engine_core::hash::{splitmix64, GOLDEN_GAMMA};
use engine_core::{Fnv64, Rng};

/// Most episodes one buffer holds: mirrors the tournament's match cap so a
/// full buffer always converts to a playable bracket.
pub const MAX_REPLAY_EPISODES: usize = 100;

/// Most steps one episode holds: one decision per tick for a 60 s bout at
/// 60 Hz, mirroring the tournament's per-match observation cap.
pub const MAX_REPLAY_STEPS_PER_EPISODE: usize = 3_600;

/// Most steps one buffer holds: equals the trainer's dataset cap so a full
/// buffer always converts to a trainable slice.
pub const MAX_REPLAY_STEPS: usize = 65_536;

/// Collector shape: feature width, teacher class count, episode count and
/// steps per episode. Built only by [`ReplayConfig::new`], which refuses
/// empty and oversize shapes before any row is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayConfig {
    /// Features per observation, `1..=MAX_SPEC_DIM`-bounded (see
    /// [`StudentSpec`]).
    pub input_dim: usize,
    /// Teacher action-class count, `1..=256` (actions travel in `u8`).
    pub classes: usize,
    /// Episodes to collect, `1..=MAX_REPLAY_EPISODES`.
    pub episodes: usize,
    /// Steps per episode, `1..=MAX_REPLAY_STEPS_PER_EPISODE`.
    pub steps_per_episode: usize,
}

/// Collector failures. Every variant names its cause; construction and
/// collection never panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// Feature width is zero or above the spec cap; carries the width.
    BadInputDim(usize),
    /// Class count is zero or above 256; carries the count.
    BadClasses(usize),
    /// Episode count is zero or above [`MAX_REPLAY_EPISODES`]; carries it.
    BadEpisodes(usize),
    /// Steps per episode is zero or above [`MAX_REPLAY_STEPS_PER_EPISODE`];
    /// carries the count.
    BadSteps(usize),
    /// The full buffer would exceed [`MAX_REPLAY_STEPS`]; carries
    /// `(steps, cap)`.
    TooManySteps {
        /// Requested total steps.
        steps: usize,
        /// Buffer step cap.
        cap: usize,
    },
    /// The teacher named no class for this shape; carries episode, row,
    /// `(got, cap)`.
    BadTeacherClass {
        /// Episode index.
        episode: usize,
        /// Row index inside the episode.
        row: usize,
        /// Action class the teacher returned.
        got: usize,
        /// Collector class count.
        cap: usize,
    },
    /// The spec shape differs from the collector shape; carries
    /// `(spec_in, want_in, spec_out, want_out)`.
    SpecMismatch {
        /// Spec input dim.
        spec_in: usize,
        /// Collector input dim.
        want_in: usize,
        /// Spec output dim.
        spec_out: usize,
        /// Collector class count.
        want_out: usize,
    },
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReplayError::BadInputDim(d) => write!(f, "input dim {d} outside 1..=1024"),
            ReplayError::BadClasses(c) => write!(f, "class count {c} outside 1..=256"),
            ReplayError::BadEpisodes(n) => {
                write!(f, "episode count {n} outside 1..={MAX_REPLAY_EPISODES}")
            }
            ReplayError::BadSteps(n) => {
                write!(
                    f,
                    "steps per episode {n} outside 1..={MAX_REPLAY_STEPS_PER_EPISODE}"
                )
            }
            ReplayError::TooManySteps { steps, cap } => {
                write!(f, "buffer holds {steps} steps, cap {cap}")
            }
            ReplayError::BadTeacherClass {
                episode,
                row,
                got,
                cap,
            } => write!(
                f,
                "episode {episode} row {row}: teacher class {got} at or above {cap}"
            ),
            ReplayError::SpecMismatch {
                spec_in,
                want_in,
                spec_out,
                want_out,
            } => write!(
                f,
                "spec ({spec_in}, {spec_out}) differs from buffer ({want_in}, {want_out})"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

impl ReplayConfig {
    /// Validate the shape and pack the config. Never panics.
    pub fn new(
        input_dim: usize,
        classes: usize,
        episodes: usize,
        steps_per_episode: usize,
    ) -> Result<Self, ReplayError> {
        if input_dim == 0 || input_dim > 1024 {
            return Err(ReplayError::BadInputDim(input_dim));
        }
        if classes == 0 || classes > 256 {
            return Err(ReplayError::BadClasses(classes));
        }
        if episodes == 0 || episodes > MAX_REPLAY_EPISODES {
            return Err(ReplayError::BadEpisodes(episodes));
        }
        if steps_per_episode == 0 || steps_per_episode > MAX_REPLAY_STEPS_PER_EPISODE {
            return Err(ReplayError::BadSteps(steps_per_episode));
        }
        let steps = episodes.saturating_mul(steps_per_episode);
        if steps > MAX_REPLAY_STEPS {
            return Err(ReplayError::TooManySteps {
                steps,
                cap: MAX_REPLAY_STEPS,
            });
        }
        Ok(Self {
            input_dim,
            classes,
            episodes,
            steps_per_episode,
        })
    }

    /// Total observation rows the collector draws.
    pub fn total_steps(&self) -> usize {
        self.episodes * self.steps_per_episode
    }
}

/// One fork episode: the seeded observation rows plus the teacher action per
/// row, in step order. Episode `i` of a collection streams from
/// `seed.wrapping_add(i)`, mirroring the verifier's seed convention.
#[derive(Clone, Debug, PartialEq)]
pub struct Episode {
    /// Deterministic stream this episode's rows came from.
    pub seed: u64,
    /// Observation rows, one feature vector each; every row is exactly the
    /// collector input dim wide on exact `1/16` steps.
    pub features: Vec<Vec<f32>>,
    /// Teacher action per row, every id below the collector class count.
    pub actions: Vec<u8>,
}

/// Deterministic episode buffer plus its content digest. Built by
/// [`ReplayBuffer::collect`] (seeded draws) or
/// [`ReplayBuffer::from_episodes`] (recorded arena rows); the digest covers the config, every episode
/// seed and every quantized lane plus action, so equal buffers hash equal.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayBuffer {
    /// Shape this buffer was collected with.
    pub config: ReplayConfig,
    /// Episodes in collection order.
    pub episodes: Vec<Episode>,
    /// Content digest over config, seeds, quantized lanes and actions.
    pub digest: u64,
}

/// Episode stream `index` of collection `seed`: SplitMix64 over
/// `seed + index * GOLDEN_GAMMA` into the engine stream, so episode streams
/// fork like every other subsystem and never share state.
fn episode_rng(seed: u64, index: u64) -> Rng {
    Rng::from_state(splitmix64(
        seed.wrapping_add(index.wrapping_mul(GOLDEN_GAMMA)),
    ))
}

/// Widen a buffer length into the digest lane. Lengths always fit `u64` on
/// the supported targets; saturate instead of panicking on exotic widths.
fn len_to_digest_lane(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Widen an episode index into the fork lane. Indices stay below
/// [`MAX_REPLAY_EPISODES`]; saturate instead of panicking.
fn index_to_fork_lane(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX)
}

/// Narrow the `0..255` draw residue to the centered lane in
/// `[-127, 127]`. Residues fit `u8` by construction; saturate instead of
/// panicking.
fn residue_to_centered(residue: u64) -> i16 {
    let narrow = u8::try_from(residue).unwrap_or(u8::MAX);
    i16::from(narrow) - 127
}

/// Quantize one feature lane exactly as inference does: the one
/// quantization edge ([`engine_nn::quantize_lane`]) at 16 steps per unit, so
/// equal lanes hash equal and the digest covers what the net sees. Pure;
/// never panics; a NaN lane maps to zero.
fn quantize_lane(value: f32) -> i8 {
    engine_nn::quantize_lane(value, INPUT_QUANT_SCALE)
}

/// Digest over the config, every episode seed and every quantized lane plus
/// action. Pure; never panics.
fn digest_buffer(config: &ReplayConfig, episodes: &[Episode]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u64(len_to_digest_lane(config.input_dim));
    hasher.write_u64(len_to_digest_lane(config.classes));
    hasher.write_u64(len_to_digest_lane(config.episodes));
    hasher.write_u64(len_to_digest_lane(config.steps_per_episode));
    for episode in episodes {
        hasher.write_u64(episode.seed);
        for (row, action) in episode.features.iter().zip(episode.actions.iter()) {
            for value in row.iter() {
                // Rows sit on exact 1/16 steps, so the runtime quantization
                // is exact: the digest covers what inference sees.
                let q = quantize_lane(*value);
                hasher.write_u8(q.cast_unsigned());
            }
            hasher.write_u8(*action);
        }
    }
    hasher.finish()
}

impl ReplayBuffer {
    /// Collect `config.episodes` episodes of `config.steps_per_episode`
    /// seeded observations, labeled by `teacher`. Episode `i` streams from
    /// `seed.wrapping_add(i)`; every lane draws `(next % 255) - 127` over
    /// 16, so rows land on exact `1/16` steps inside `±MAX_FEATURE_ABS` and
    /// runtime quantization is exact. The teacher sees plain feature slices
    /// and returns the action class; an out-of-range id fails the whole
    /// collection with its episode and row. Pure and deterministic: the same
    /// seed plus config plus teacher collects the same buffer on every
    /// machine.
    pub fn collect(
        seed: u64,
        config: ReplayConfig,
        teacher: fn(&[f32]) -> u8,
    ) -> Result<Self, ReplayError> {
        let mut episodes = Vec::with_capacity(config.episodes);
        for episode_index in 0..config.episodes {
            let episode_seed = seed.wrapping_add(index_to_fork_lane(episode_index));
            let mut rng = episode_rng(seed, index_to_fork_lane(episode_index));
            let mut features = Vec::with_capacity(config.steps_per_episode);
            let mut actions = Vec::with_capacity(config.steps_per_episode);
            for row_index in 0..config.steps_per_episode {
                let mut row = Vec::with_capacity(config.input_dim);
                for _ in 0..config.input_dim {
                    let centered = residue_to_centered(rng.next_u64() % 255);
                    row.push(f32::from(centered) / 16.0);
                }
                debug_assert!(
                    row.iter()
                        .all(|v| v.is_finite() && v.abs() <= MAX_FEATURE_ABS),
                    "drawn lanes stay in range"
                );
                let action = teacher(&row);
                if usize::from(action) >= config.classes {
                    return Err(ReplayError::BadTeacherClass {
                        episode: episode_index,
                        row: row_index,
                        got: usize::from(action),
                        cap: config.classes,
                    });
                }
                actions.push(action);
                features.push(row);
            }
            episodes.push(Episode {
                seed: episode_seed,
                features,
                actions,
            });
        }
        let digest = digest_buffer(&config, &episodes);
        Ok(Self {
            config,
            episodes,
            digest,
        })
    }

    /// Adopt recorded episodes (an arena teacher run, a tape range) as a
    /// buffer: every episode must hold exactly `config.steps_per_episode`
    /// rows of `config.input_dim` finite lanes within `±MAX_FEATURE_ABS`
    /// and actions below `config.classes`, and the episode count must equal
    /// `config.episodes`. The digest is the same content hash
    /// [`collect`](Self::collect) stamps. Pure; never panics.
    pub fn from_episodes(
        config: ReplayConfig,
        episodes: Vec<Episode>,
    ) -> Result<Self, ReplayError> {
        if episodes.len() != config.episodes {
            return Err(ReplayError::BadEpisodes(episodes.len()));
        }
        for (episode_index, episode) in episodes.iter().enumerate() {
            if episode.features.len() != config.steps_per_episode
                || episode.actions.len() != config.steps_per_episode
            {
                return Err(ReplayError::BadSteps(
                    episode.features.len().max(episode.actions.len()),
                ));
            }
            for (row_index, (row, action)) in
                episode.features.iter().zip(&episode.actions).enumerate()
            {
                if row.len() != config.input_dim
                    || !row
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= MAX_FEATURE_ABS)
                {
                    return Err(ReplayError::BadInputDim(row.len()));
                }
                if usize::from(*action) >= config.classes {
                    return Err(ReplayError::BadTeacherClass {
                        episode: episode_index,
                        row: row_index,
                        got: usize::from(*action),
                        cap: config.classes,
                    });
                }
            }
        }
        let digest = digest_buffer(&config, &episodes);
        Ok(Self {
            config,
            episodes,
            digest,
        })
    }

    /// Observation rows collected, across all episodes.
    pub fn len_steps(&self) -> usize {
        self.config.total_steps()
    }

    /// Map every episode row to a validated [`Sample`] in collection order,
    /// ready for [`train`](crate::train). Fails when the spec shape differs
    /// from the collector shape. Never panics.
    pub fn to_samples(&self, spec: &StudentSpec) -> Result<Vec<Sample>, ReplayError> {
        if spec.dims.input_dim != self.config.input_dim
            || spec.dims.output_dim != self.config.classes
        {
            return Err(ReplayError::SpecMismatch {
                spec_in: spec.dims.input_dim,
                want_in: self.config.input_dim,
                spec_out: spec.dims.output_dim,
                want_out: self.config.classes,
            });
        }
        let mut out = Vec::with_capacity(self.len_steps());
        for episode in &self.episodes {
            for (row, action) in episode.features.iter().zip(episode.actions.iter()) {
                // Rows are drawn in range and teacher ids are checked at
                // collection, so this only fails on a hand-built buffer.
                match Sample::new(row.clone(), *action, spec) {
                    Ok(sample) => out.push(sample),
                    Err(_) => {
                        return Err(ReplayError::BadTeacherClass {
                            episode: 0,
                            row: out.len(),
                            got: usize::from(*action),
                            cap: self.config.classes,
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    /// Map every episode to a [`MatchScript`] plus its teacher actions, in
    /// collection order — the exact input shape
    /// [`tournament`](crate::tournament) scores. Never panics.
    pub fn to_scripts(&self) -> (Vec<MatchScript>, Vec<Vec<u8>>) {
        let mut scripts = Vec::with_capacity(self.episodes.len());
        let mut teacher = Vec::with_capacity(self.episodes.len());
        for episode in &self.episodes {
            scripts.push(MatchScript {
                seed: episode.seed,
                features: episode.features.clone(),
            });
            teacher.push(episode.actions.clone());
        }
        (scripts, teacher)
    }
}

/// Kill-rule cause when a replay-fed student fails the ship gate. Every
/// variant carries the bout counts or rate that killed the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillCause {
    /// The replay student banked fewer bouts than the scripted stand-in.
    LostToScripted {
        /// Bouts the replay-fed student banked.
        student_wins: u32,
        /// Bouts the scripted stand-in banked.
        scripted_wins: u32,
    },
    /// The replay student rate left the verifier bounds.
    OutOfBounds {
        /// `student_wins * 1000 / (3 * matches)`, outside `[350, 650]`.
        student_rate_permille: u16,
    },
    /// The replay student failed to beat the static-slice baseline on the
    /// same bracket (ties kill: the buffer must prove it earned its rows).
    NoBetterThanStatic {
        /// Bouts the replay-fed student banked.
        replay_wins: u32,
        /// Bouts the static-slice student banked.
        static_wins: u32,
    },
}

impl core::fmt::Display for KillCause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            KillCause::LostToScripted {
                student_wins,
                scripted_wins,
            } => write!(
                f,
                "replay student banked {student_wins} bouts, scripted {scripted_wins}"
            ),
            KillCause::OutOfBounds {
                student_rate_permille,
            } => write!(
                f,
                "replay student rate {student_rate_permille} permille outside [350, 650]"
            ),
            KillCause::NoBetterThanStatic {
                replay_wins,
                static_wins,
            } => write!(
                f,
                "replay student banked {replay_wins} bouts, static slice {static_wins}"
            ),
        }
    }
}

impl std::error::Error for KillCause {}

/// Ship verdict for one replay-fed student against its static-slice
/// baseline: [`ReplayVerdict::Pass`] feeds the learner,
/// [`ReplayVerdict::Killed`] names the cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayVerdict {
    /// The replay student held the bounds, matched the stand-in and beat
    /// the static slice: the buffer earned its rows.
    Pass,
    /// The buffer is killed for this student pair; the cause is carried.
    Killed(KillCause),
}

impl core::fmt::Display for ReplayVerdict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReplayVerdict::Pass => f.write_str("replay buffer ships"),
            ReplayVerdict::Killed(cause) => write!(f, "replay buffer killed: {cause}"),
        }
    }
}

impl std::error::Error for ReplayVerdict {}

/// Judge one replay-fed student against its static-slice baseline on the
/// same bracket: `replay` is the [`tournament`](crate::tournament) report of
/// the student trained on the full buffer, `baseline` the report of the
/// student trained on the fixed-prefix static slice, both over the same
/// scripts and teacher. Pass needs all three: the replay rate sits inside
/// the verifier bounds, the replay student banks minimum matching bouts
/// versus the scripted stand-in, and strictly more than the static baseline.
/// Pure; never panics.
pub fn judge_buffer_student(
    replay: &TournamentReport,
    baseline: &TournamentReport,
) -> ReplayVerdict {
    if !replay.within_bounds {
        return ReplayVerdict::Killed(KillCause::OutOfBounds {
            student_rate_permille: replay.student_rate_permille,
        });
    }
    if replay.student_wins < replay.scripted_wins {
        return ReplayVerdict::Killed(KillCause::LostToScripted {
            student_wins: replay.student_wins,
            scripted_wins: replay.scripted_wins,
        });
    }
    if replay.student_wins <= baseline.student_wins {
        return ReplayVerdict::Killed(KillCause::NoBetterThanStatic {
            replay_wins: replay.student_wins,
            static_wins: baseline.student_wins,
        });
    }
    ReplayVerdict::Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::student::Student;

    /// Fixed teacher: the same separable 3-class probe over 4 features the
    /// tournament fixture distills, so the replay buffer tells one story
    /// from collection through the tournament. Class = first-max-wins argmax.
    const TEACHER: [[f32; 4]; 3] = [
        [1.5, -1.0, 0.5, 0.25],
        [-1.0, 1.5, -0.5, 0.75],
        [0.25, 0.5, 1.5, -1.5],
    ];

    fn teacher_class(features: &[f32]) -> u8 {
        let mut best = 0u8;
        let mut top = f32::NEG_INFINITY;
        for (o, row) in TEACHER.iter().enumerate() {
            let mut acc = 0.0f32;
            for (w, v) in row.iter().zip(features.iter()) {
                acc += w * v;
            }
            if acc > top {
                top = acc;
                best = u8::try_from(o).unwrap_or(u8::MAX);
            }
        }
        best
    }

    fn config() -> ReplayConfig {
        ReplayConfig::new(4, 3, 10, 24).expect("test shape is legal")
    }

    #[test]
    fn adopted_episodes_digest_like_collected_ones() {
        let collected =
            ReplayBuffer::collect(0x00BE_5500, config(), teacher_class).expect("fixture collects");
        let adopted = ReplayBuffer::from_episodes(config(), collected.episodes.clone())
            .expect("collected episodes adopt");
        assert_eq!(
            adopted, collected,
            "same rows give the same buffer and digest"
        );

        let mut short = collected.episodes.clone();
        short.pop();
        assert_eq!(
            ReplayBuffer::from_episodes(config(), short),
            Err(ReplayError::BadEpisodes(9))
        );
        let mut ragged = collected.episodes.clone();
        ragged[2].actions.pop();
        assert_eq!(
            ReplayBuffer::from_episodes(config(), ragged),
            Err(ReplayError::BadSteps(24))
        );
        let mut wide = collected.episodes.clone();
        wide[1].features[3].push(0.0);
        assert_eq!(
            ReplayBuffer::from_episodes(config(), wide),
            Err(ReplayError::BadInputDim(5))
        );
        let mut nan = collected.episodes.clone();
        nan[1].features[3][0] = f32::NAN;
        assert_eq!(
            ReplayBuffer::from_episodes(config(), nan),
            Err(ReplayError::BadInputDim(4))
        );
        let mut wrong = collected.episodes;
        wrong[4].actions[7] = 3;
        assert_eq!(
            ReplayBuffer::from_episodes(config(), wrong),
            Err(ReplayError::BadTeacherClass {
                episode: 4,
                row: 7,
                got: 3,
                cap: 3
            })
        );
    }

    #[test]
    fn same_seed_collects_the_same_buffer_twice() {
        let first =
            ReplayBuffer::collect(0x00BE_5500, config(), teacher_class).expect("fixture collects");
        let second = ReplayBuffer::collect(0x00BE_5500, config(), teacher_class)
            .expect("same seed collects again");
        assert_eq!(first, second, "same seed must collect the same buffer");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.len_steps(), 240);
        // Seeds open the recorded range episode by episode.
        assert_eq!(first.episodes[0].seed, 0x00BE_5500);
        assert_eq!(first.episodes[9].seed, 0x00BE_5500u64.wrapping_add(9));
        // A different seed collects a different buffer.
        let other = ReplayBuffer::collect(0x00BE_5501, config(), teacher_class)
            .expect("next seed collects");
        assert_ne!(
            first.digest, other.digest,
            "next seed must digest different"
        );
        assert_ne!(first, other);
    }

    #[test]
    fn bad_shapes_and_teachers_refused_with_cause() {
        assert_eq!(
            ReplayConfig::new(0, 3, 2, 4),
            Err(ReplayError::BadInputDim(0))
        );
        assert_eq!(
            ReplayConfig::new(4, 0, 2, 4),
            Err(ReplayError::BadClasses(0))
        );
        assert_eq!(
            ReplayConfig::new(4, 3, 0, 4),
            Err(ReplayError::BadEpisodes(0))
        );
        assert_eq!(ReplayConfig::new(4, 3, 2, 0), Err(ReplayError::BadSteps(0)));
        assert_eq!(
            ReplayConfig::new(4, 3, MAX_REPLAY_EPISODES + 1, 4),
            Err(ReplayError::BadEpisodes(MAX_REPLAY_EPISODES + 1))
        );
        // 100 episodes of 3_600 steps overflows the buffer step cap.
        assert_eq!(
            ReplayConfig::new(4, 3, 100, 3_600),
            Err(ReplayError::TooManySteps {
                steps: 360_000,
                cap: MAX_REPLAY_STEPS,
            })
        );
        // A teacher id outside the class count names its episode and row.
        let shape = ReplayConfig::new(4, 3, 2, 4).expect("legal shape");
        assert_eq!(
            ReplayBuffer::collect(7, shape, |_| 9),
            Err(ReplayError::BadTeacherClass {
                episode: 0,
                row: 0,
                got: 9,
                cap: 3,
            })
        );
        // Samples refuse a spec the buffer was not collected for.
        let buffer = ReplayBuffer::collect(7, config(), teacher_class).expect("collects");
        let wrong = StudentSpec::new(2, 3, 1).expect("legal spec");
        assert_eq!(
            buffer.to_samples(&wrong),
            Err(ReplayError::SpecMismatch {
                spec_in: 2,
                want_in: 4,
                spec_out: 3,
                want_out: 3,
            })
        );
    }

    #[test]
    fn buffer_feeds_samples_and_the_tournament() {
        use crate::tournament::{score_against_teacher, scripted_decide, tournament};

        let buffer =
            ReplayBuffer::collect(0xD7300, config(), teacher_class).expect("fixture collects");
        let spec = StudentSpec::new(4, 3, 0x70u64).expect("fixture spec is legal");
        let samples = buffer.to_samples(&spec).expect("shapes match");
        assert_eq!(samples.len(), 240, "every row maps to one sample");
        // The recorded fixture exercises every teacher action.
        let mut class_counts = [0usize; 3];
        for sample in samples.iter() {
            class_counts[usize::from(sample.target)] += 1;
        }
        assert!(
            class_counts.iter().all(|count| *count >= 5),
            "buffer must exercise every class: {class_counts:?}"
        );
        // Teacher knowledge beats nothing: the exact-teacher student
        // decides every buffer row like the teacher, while the zero
        // student (first-max-wins always picks class 0) only agrees on
        // the class-0 rows. Agreement, not MSE, is the decision metric:
        // raw linear logits are not one-hot, so MSE punishes scale.
        let mut weights = Vec::with_capacity(12);
        for row in TEACHER.iter() {
            weights.extend_from_slice(row);
        }
        let exact = Student::new(spec, weights, vec![0.0; 3], 0).expect("teacher params are legal");
        let baseline = Student::zeros(spec);
        // First-max-wins over float logits, mirroring `argmax` for the
        // reference forward (which returns floats, not runtime ints).
        fn argmax_f32(logits: &[f32]) -> usize {
            let mut best = 0usize;
            let mut top = f32::NEG_INFINITY;
            for (i, v) in logits.iter().enumerate() {
                if *v > top {
                    top = *v;
                    best = i;
                }
            }
            best
        }
        let mut exact_agree = 0usize;
        let mut base_agree = 0usize;
        for sample in samples.iter() {
            let logits = exact.forward_f32(&sample.features).expect("shapes fit");
            if argmax_f32(&logits) == usize::from(sample.target) {
                exact_agree += 1;
            }
            let base_logits = baseline.forward_f32(&sample.features).expect("shapes fit");
            if argmax_f32(&base_logits) == usize::from(sample.target) {
                base_agree += 1;
            }
        }
        assert_eq!(
            exact_agree,
            samples.len(),
            "teacher student agrees everywhere"
        );
        assert!(
            exact_agree > base_agree,
            "teacher knowledge must beat nothing: {exact_agree} vs {base_agree}"
        );
        // The tournament scores the buffer episodes: the near-teacher
        // student (one lane nudged, mirroring the tournament fixture) carries
        // teacher knowledge the scripted stand-in lacks.
        let mut near = exact.weights.clone();
        near[0] += 0.3;
        let near_student =
            Student::new(spec, near, vec![0.0; 3], 0).expect("near params are legal");
        let bytes =
            crate::export::export(&near_student, &[crate::data::DatasetManifest::fixture()])
                .expect("near student exports");
        let (scripts, teacher) = buffer.to_scripts();
        assert_eq!(scripts.len(), 10);
        assert_eq!(scripts[0].seed, 0xD7300, "scripts keep the episode seeds");
        let report = tournament(&bytes, &teacher, &scripts, 10).expect("bracket plays");
        assert_eq!(report.matches, 10);
        assert_eq!(report.observations, 240);
        assert!(
            report.student_wins >= report.scripted_wins,
            "replay-fed student must at least match scripted: {report:?}"
        );
        // Same buffer twice replays bit-exactly through the tournament.
        let again = tournament(&bytes, &teacher, &scripts, 10).expect("replay plays");
        assert_eq!(report, again);
        // The gate is only meaningful when the stand-in stays weak.
        let mut student_agree = 0usize;
        let mut scripted_agree = 0usize;
        let mut total = 0usize;
        for (script, actions) in scripts.iter().zip(teacher.iter()) {
            let decided: Vec<u8> = script
                .features
                .iter()
                .map(|row| teacher_class(row))
                .collect();
            student_agree += score_against_teacher(&decided, actions).agree;
            for row in script.features.iter() {
                total += 1;
                if scripted_decide(&crate::export::quantize_input(row), 3) == teacher_class(row) {
                    scripted_agree += 1;
                }
            }
        }
        assert_eq!(total, 240);
        assert_eq!(student_agree, total, "exact teacher agrees everywhere");
        assert!(
            scripted_agree * 1000 / total <= 600,
            "stand-in must stay weak: {scripted_agree}/{total}"
        );
    }

    #[test]
    fn kill_rule_passes_only_a_buffer_that_beats_static() {
        fn report(
            student_wins: u32,
            scripted_wins: u32,
            teacher_wins: u32,
            matches: u32,
        ) -> TournamentReport {
            let student_rate_permille =
                u16::try_from(student_wins * 1000 / (3 * matches)).unwrap_or(u16::MAX);
            let within_bounds = (350..=650).contains(&student_rate_permille);
            TournamentReport {
                student_wins,
                scripted_wins,
                teacher_wins,
                matches,
                observations: matches * 8,
                student_rate_permille,
                within_bounds,
                seeds: vec![1; usize::try_from(matches).unwrap_or(usize::MAX)],
            }
        }

        // 10 matches play 30 bouts. Replay 15 / scripted 5 / teacher 10
        // reads 500 permille: in bounds, ahead of the stand-in.
        let replay = report(15, 5, 10, 10);
        assert_eq!(replay.student_rate_permille, 500);
        assert!(replay.within_bounds);
        // Beating the static slice ships the buffer.
        assert_eq!(
            judge_buffer_student(&replay, &report(8, 5, 17, 10)),
            ReplayVerdict::Pass
        );
        // Tying the static slice kills it: the buffer proved nothing.
        assert_eq!(
            judge_buffer_student(&replay, &report(15, 5, 10, 10)),
            ReplayVerdict::Killed(KillCause::NoBetterThanStatic {
                replay_wins: 15,
                static_wins: 15,
            })
        );
        // Regressing against the static slice kills it too.
        assert_eq!(
            judge_buffer_student(&replay, &report(18, 2, 10, 10)),
            ReplayVerdict::Killed(KillCause::NoBetterThanStatic {
                replay_wins: 15,
                static_wins: 18,
            })
        );
        // Losing to the stand-in kills it even when ahead of static:
        // 11 / 12 / 7 reads 366 permille (in bounds) but banks fewer.
        let weak = report(11, 12, 7, 10);
        assert!(weak.within_bounds);
        assert_eq!(
            judge_buffer_student(&weak, &report(8, 5, 17, 10)),
            ReplayVerdict::Killed(KillCause::LostToScripted {
                student_wins: 11,
                scripted_wins: 12,
            })
        );
        // Leaving the bounds kills it first: 20 / 5 / 5 reads 666.
        let wild = report(20, 5, 5, 10);
        assert!(!wild.within_bounds);
        assert_eq!(
            judge_buffer_student(&wild, &report(8, 5, 17, 10)),
            ReplayVerdict::Killed(KillCause::OutOfBounds {
                student_rate_permille: 666,
            })
        );
        // Every cause explains itself in one line.
        assert_eq!(
            ReplayVerdict::Killed(KillCause::LostToScripted {
                student_wins: 11,
                scripted_wins: 12,
            })
            .to_string(),
            "replay buffer killed: replay student banked 11 bouts, scripted 12"
        );
        assert_eq!(
            ReplayVerdict::Killed(KillCause::OutOfBounds {
                student_rate_permille: 666,
            })
            .to_string(),
            "replay buffer killed: replay student rate 666 permille outside [350, 650]"
        );
        assert_eq!(
            ReplayVerdict::Killed(KillCause::NoBetterThanStatic {
                replay_wins: 15,
                static_wins: 15,
            })
            .to_string(),
            "replay buffer killed: replay student banked 15 bouts, static slice 15"
        );
        assert_eq!(ReplayVerdict::Pass.to_string(), "replay buffer ships");
    }

    #[test]
    fn buffer_scripts_judge_teacher_over_nothing() {
        use crate::tournament::tournament;

        // End to end through the consumer: the buffer's own episodes form
        // the bracket, the teacher-fed student plays the replay candidate,
        // zeros play the static slice, and the kill rule ships the buffer
        // that carried teacher knowledge.
        let buffer =
            ReplayBuffer::collect(0xD7300, config(), teacher_class).expect("fixture collects");
        let spec = StudentSpec::new(4, 3, 0x70u64).expect("fixture spec is legal");
        let mut weights = Vec::with_capacity(12);
        for row in TEACHER.iter() {
            weights.extend_from_slice(row);
        }
        weights[0] += 0.3;
        let replay =
            Student::new(spec, weights, vec![0.0; 3], 0).expect("near-teacher params are legal");
        let replay_bytes =
            crate::export::export(&replay, &[crate::data::DatasetManifest::fixture()])
                .expect("replay student exports");
        let static_bytes = crate::export::export(
            &Student::zeros(spec),
            &[crate::data::DatasetManifest::fixture()],
        )
        .expect("static student exports");
        let (scripts, teacher) = buffer.to_scripts();
        let replay_report =
            tournament(&replay_bytes, &teacher, &scripts, 10).expect("replay bracket plays");
        let static_report =
            tournament(&static_bytes, &teacher, &scripts, 10).expect("static bracket plays");
        assert_eq!(
            tournament(&replay_bytes, &teacher, &scripts, 10),
            Ok(replay_report.clone()),
            "same buffer twice replays bit-exactly"
        );
        assert_eq!(
            judge_buffer_student(&replay_report, &static_report),
            ReplayVerdict::Pass,
            "teacher-fed buffer must ship over the static slice: {replay_report:?} vs {static_report:?}"
        );
    }
}
