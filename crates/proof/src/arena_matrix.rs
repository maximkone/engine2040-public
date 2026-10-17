//! Arena tape of the golden matrix (DSN2): one fixed 8-fighter rules-v2 duel
//! on the arena kit walls, 1200 fixed ticks, the checkpoint hash of every
//! tick digested into one number.
//!
//! The legacy golden tape never runs arena code, so it cannot say whether
//! the match players and the browser actually run (rules v2: balance,
//! collision, stagger memory, limb pool, the bonded crate prop) is equal on
//! x86-64, aarch64 and wasm32. This tape runs exactly that match, through the
//! same [`MatchSpec`] and [`engine_arena::LiveMatch`] the game window and the
//! net server build, with a scripted tape for two human slots and the arena
//! brains on the other six.
//!
//! Consumer: the `arena_line` bin prints [`ArenaTape::rows`]; the
//! golden-matrix workflow runs it on every runner and
//! `tools/golden-matrix.mjs --arena` compares the files, naming the first
//! tick that differs. Data owner: this module owns the fixture (seed, script,
//! row layout); `engine-arena` owns the match. Units: fixed ticks at 60 Hz.
//! The digest is [`engine_core::Fnv64`] over the spec identity and every tick
//! hash, so one different hash anywhere moves it. Nothing here touches the
//! simulation: the tape only calls the public arena API.

use std::fmt;
use std::fmt::Write as _;

use engine_arena::world::kit::KitError;
use engine_arena::{ArenaConfig, BodyId, BodyInput, MatchSpec};
use engine_combat::FightEvent;
use engine_core::Fnv64;

/// Fixed steps of the match (20 s at 60 Hz).
pub const TICKS: u32 = 1200;
/// Fighters on the kit: two scripted humans and six brains.
pub const FIGHTERS: u8 = 8;
/// Slots driven by the script; the arena brains fill the rest.
const HUMANS: u8 = 2;
/// Match seed of the standard tape.
pub const SEED: u64 = 0x00A2_E4A1_2026_1035;
/// Arena half-extent in meters: the kit walls stand inside it.
pub const HALF_EXTENT_M: f32 = 18.0;
/// Facing turn per tick, centidegrees.
const FACING_STEP: u32 = 5;
/// Button bits of the fighter contract.
const ATTACK: u8 = 0x01;
const BLOCK: u8 = 0x02;
const DODGE: u8 = 0x04;

/// Why the tape could not be played.
#[derive(Debug, PartialEq)]
pub enum ArenaTapeError {
    /// The embedded kit placement did not build a world.
    Kit(KitError),
    /// The spec built no live match (a spawn blocked by the world).
    Spec,
    /// The match rejected the inputs of this tick.
    Step(u32),
}

impl fmt::Display for ArenaTapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kit(error) => write!(f, "kit world: {error}"),
            Self::Spec => write!(f, "the match spec builds no live match"),
            Self::Step(tick) => write!(f, "the match rejected tick {tick}"),
        }
    }
}

impl std::error::Error for ArenaTapeError {}

/// The knobs of the match. [`Fixture::STANDARD`] is what the matrix runs; the
/// tests move one knob at a time to show the digest follows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fixture {
    /// Match seed.
    pub seed: u64,
    /// Tick on which slot 0's attack bit is flipped, or `None` for the
    /// scripted tape as written.
    pub flip_tick: Option<u32>,
}

impl Fixture {
    /// The match the golden matrix compares across machines.
    pub const STANDARD: Self = Self {
        seed: SEED,
        flip_tick: None,
    };

    /// The spec every peer plays: rules v2 on the kit walls.
    pub fn spec(&self) -> Result<MatchSpec, ArenaTapeError> {
        let config = ArenaConfig {
            seed: self.seed,
            ticks: TICKS,
            fighters: FIGHTERS,
            half_extent_m: HALF_EXTENT_M,
            table_version: 2,
        };
        MatchSpec::kit_duel(config).map_err(ArenaTapeError::Kit)
    }

    /// The scripted inputs of one tick, slot order. Slot 0 swings every 30
    /// ticks, guards for a window and dodges once per 120. Slot 1 re-presses
    /// block every 8 ticks, so a blow that reaches it from the front lands
    /// inside a fresh parry window, and swings once per 48. Facing turns
    /// slowly. The seed was picked by scanning seeds for a tape in which
    /// every kind of outcome below happens.
    fn inputs(&self, tick: u32) -> Vec<BodyInput> {
        let mut out = Vec::with_capacity(usize::from(HUMANS));
        for slot in 0..HUMANS {
            let phase = tick % 120;
            let mut bits = 0u8;
            if slot == 0 {
                if phase.is_multiple_of(30) {
                    bits |= ATTACK;
                }
                if (84..96).contains(&phase) {
                    bits |= BLOCK;
                }
                if phase == 108 {
                    bits |= DODGE;
                }
                if self.flip_tick == Some(tick) {
                    bits ^= ATTACK;
                }
            } else if tick % 8 != 7 {
                bits |= BLOCK;
            } else if tick % 48 == 47 {
                bits |= ATTACK;
            }
            let sweep = i32::try_from((tick * FACING_STEP) % 36_000).unwrap_or(0) - 18_000;
            let mut input = BodyInput {
                tick,
                fighter: BodyId(slot),
                mx: if (phase / 30).is_multiple_of(2) {
                    100
                } else {
                    -60
                },
                mz: if slot == 0 { 30 } else { -30 },
                facing: i16::try_from(sweep).unwrap_or(0),
                buttons: Default::default(),
            };
            // The type of `buttons` is not re-exported by the arena; every
            // u8 under 8 is a valid bitset of the contract.
            input.buttons.0 = bits;
            out.push(input);
        }
        out
    }
}

/// What happened in the match, by kind: evidence that the tape reaches the
/// rules it is meant to pin. Never hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Blows that landed for damage.
    pub hits: u32,
    /// Blows soaked by a held block.
    pub blocks: u32,
    /// Blocks inside the parry window.
    pub parries: u32,
    /// Blows negated by dodge frames.
    pub dodges: u32,
    /// Fighters dropped to zero hp.
    pub knockdowns: u32,
    /// Destruction events of the crate prop: bonds snapped and chunks freed.
    pub breaks: u32,
}

/// One machine's arena tape: the hash of every tick and their digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaTape {
    /// [`MatchSpec::identity`] of the match played.
    pub identity: u64,
    /// [`MatchSpec::world_digest`] of the kit walls.
    pub world: u64,
    /// Checkpoint hash of every tick, tick order.
    pub hashes: Vec<u64>,
    /// Digest over the identity and every hash.
    pub digest: u64,
    /// Event counts, to show what the tape exercised.
    pub tally: Tally,
}

impl ArenaTape {
    /// The text one runner stores: a summary line, then one
    /// `<host> tick=<t> hash=<16 hex>` line per tick. The host label leads
    /// every line. `tools/golden-matrix.mjs --arena` parses exactly this.
    #[must_use]
    pub fn rows(&self, host: &str) -> String {
        let tally = &self.tally;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{host} arena={:016x} ticks={} fighters={FIGHTERS} spec={:016x} world={:016x} \
             hits={} blocks={} parries={} dodges={} knockdowns={} breaks={}",
            self.digest,
            self.hashes.len(),
            self.identity,
            self.world,
            tally.hits,
            tally.blocks,
            tally.parries,
            tally.dodges,
            tally.knockdowns,
            tally.breaks,
        );
        for (tick, hash) in self.hashes.iter().enumerate() {
            let _ = writeln!(out, "{host} tick={tick} hash={hash:016x}");
        }
        out
    }
}

/// Play the fixture and collect its tape.
///
/// Determinism: same binary and fixture give the same tape; a second
/// machine's tape must equal it byte for byte.
pub fn run(fixture: &Fixture) -> Result<ArenaTape, ArenaTapeError> {
    let spec = fixture.spec()?;
    let mut live = spec.live().ok_or(ArenaTapeError::Spec)?;
    let mut digest = Fnv64::new();
    digest.write_u64(spec.identity());
    let mut hashes = Vec::with_capacity(TICKS as usize);
    let mut tally = Tally::default();
    for tick in 0..TICKS {
        let stepped = live
            .step(&fixture.inputs(tick))
            .ok_or(ArenaTapeError::Step(tick))?;
        digest.write_u64(stepped.checkpoint.hash);
        hashes.push(stepped.checkpoint.hash);
        for event in &stepped.events.events {
            match event {
                FightEvent::Hit { .. } => tally.hits += 1,
                FightEvent::Blocked { .. } => tally.blocks += 1,
                FightEvent::Parried { .. } => tally.parries += 1,
                FightEvent::Dodged { .. } => tally.dodges += 1,
                FightEvent::Knockdown { .. } => tally.knockdowns += 1,
                FightEvent::StaminaOut { .. } => {}
            }
        }
        tally.breaks += u32::try_from(stepped.breaks.events.len()).unwrap_or(u32::MAX);
    }
    Ok(ArenaTape {
        identity: spec.identity(),
        world: spec.world_digest(),
        hashes,
        digest: digest.finish(),
        tally,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_tape_repeats_on_one_machine() {
        let first = run(&Fixture::STANDARD).expect("first match");
        let second = run(&Fixture::STANDARD).expect("second match");
        assert_eq!(first, second);
        assert_eq!(first.hashes.len(), TICKS as usize);
    }

    #[test]
    fn arena_tape_follows_one_flipped_input() {
        let base = run(&Fixture::STANDARD).expect("base");
        let flipped = run(&Fixture {
            flip_tick: Some(300),
            ..Fixture::STANDARD
        })
        .expect("flipped");
        assert_ne!(base.digest, flipped.digest);
        let first_bad = base
            .hashes
            .iter()
            .zip(&flipped.hashes)
            .position(|(a, b)| a != b);
        assert_eq!(first_bad, Some(300), "the match moves on the flipped tick");
    }

    #[test]
    fn arena_tape_follows_the_seed() {
        let base = run(&Fixture::STANDARD).expect("base");
        let other = run(&Fixture {
            seed: SEED ^ 1,
            ..Fixture::STANDARD
        })
        .expect("other");
        assert_ne!(base.digest, other.digest);
        assert_ne!(base.identity, other.identity);
    }

    #[test]
    fn arena_tape_plays_rules_v2_on_the_kit_walls() {
        let tape = run(&Fixture::STANDARD).expect("tape");
        let spec = Fixture::STANDARD.spec().expect("spec");
        assert_eq!(tape.identity, spec.identity());
        assert_ne!(tape.world, 0, "the kit walls are in the match");
        assert_eq!(spec.rules, spec.config.rules());
    }

    /// The tape must reach the rules it pins; a script that only ever swings
    /// at air would pin nothing.
    #[test]
    fn arena_tape_exercises_block_parry_knockdown_and_the_crate() {
        let t = run(&Fixture::STANDARD).expect("tape").tally;
        assert!(t.hits > 0, "{t:?}");
        assert!(t.blocks > 0, "{t:?}");
        assert!(t.parries > 0, "{t:?}");
        assert!(t.knockdowns > 0, "{t:?}");
        assert!(t.breaks > 0, "{t:?}");
    }

    #[test]
    fn arena_rows_are_one_summary_then_one_line_per_tick() {
        let tape = run(&Fixture::STANDARD).expect("tape");
        let text = tape.rows("test-host");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1 + TICKS as usize);
        assert!(lines[0].starts_with(&format!("test-host arena={:016x} ticks=1200", tape.digest)));
        assert_eq!(
            lines[301],
            format!("test-host tick=300 hash={:016x}", tape.hashes[300])
        );
    }
}
