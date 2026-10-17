//! Brief to playable slice: one slice text chains AB1, AB2, AB3 plus the
//! WD6 mode framework into a packaged build that boots, plays five minutes
//! and replays exactly. Owner is `engine-endpoint`; consumer is the endpoint
//! library helpers (`build_game`, not an MCP tool) plus the AB8/AB9 follow-ups.
//! Units: chars, seeds, fixed ticks (60 Hz), integer cm, permille hp. Budget:
//! ten fixtures, 18_000 ticks each, O(1) integer ops per tick, packages under
//! `PACKAGE_MAX_BYTES`. Errors are typed `GameError`; refusals name the stage
//! plus the legal values and write nothing. Determinism: FNV-1a plus SplitMix
//! lanes only, no wall clock; same text plus seed gives the same package.

use super::brief::{BriefEnvelope, BriefStage};
use super::build_character::{
    build_character, BuiltCharacter, CharacterBrief, LicenseRef, VariantId,
};
use super::build_encounter::{
    build_encounter, BuiltEncounter, DeterministicOracle, EncounterBrief,
};
use engine_core::{
    hash::{FNV64_OFFSET, FNV64_PRIME, GOLDEN_GAMMA},
    splitmix64, Fnv64,
};

/// Reserved door name for this stage; not registered in `mcp.rs`/`tool_names` today.
pub const TOOL_BUILD_GAME: &str = "game.build";
/// One-line summary for this stage (not advertised by the MCP door today).
pub const TOOL_SUMMARY: &str =
    "brief text to packaged playable slice: level, character, encounter, five-minute play proof";
/// Ticks per second of the slice session; five minutes is `PLAY_TICKS`.
/// Single owner of the rate is `engine_core::TICK_HZ`.
pub const TICKS_PER_SECOND: u32 = engine_core::TICK_HZ as u32;
/// Fixed play length: five minutes at 60 Hz. Fits the WD6 lobby tick bound
/// the AB8 door bind passes verbatim to `LobbyBoot`.
pub const PLAY_TICKS: u32 = 300 * TICKS_PER_SECOND;
/// Ticks between stored checkpoints; 300 tips per five-minute run.
pub const CHECKPOINT_EVERY: u32 = 60;
/// Stored checkpoints per run (`PLAY_TICKS / CHECKPOINT_EVERY`).
pub const CHECKPOINT_COUNT: usize = 300;
/// Combatants on the slice session; the AB8 door bind passes this verbatim to
/// `LobbyBoot` (arena/district take small symmetric lobbies).
pub const SLICE_FIGHTERS: u8 = 2;
/// Arena half-extent in integer cm; the AB8 door bind passes 20.0 m verbatim
/// to `LobbyBoot` (well inside the lobby extent range).
pub const SLICE_HALF_EXTENT_CM: i32 = 2000;
/// Grid step per tick in integer cm.
pub const STEP_CM: i32 = 25;
/// Slice hp starts at full health in permille; encounter damage debits it.
pub const SLICE_HP_PERMILLE: u32 = 1000;
/// Ticks between damage applications; 30 hits per run, deterministic.
pub const HIT_EVERY_TICKS: u32 = 600;
/// Package magic for `package_bytes`.
pub const PACKAGE_MAGIC: [u8; 4] = *b"ABG1";
/// Package version for `package_bytes`.
pub const PACKAGE_VERSION: u8 = 1;
/// Largest package in bytes; real packages are near 2.6 KB.
pub const PACKAGE_MAX_BYTES: usize = 65536;
/// Slice-level repair budget, carried by the shared envelope (AB1 pattern).
pub const MAX_REPAIRS: u8 = 3;
/// Seed domain for the slice play stream (kept apart from brief ids).
pub const PLAY_DOMAIN: u64 = 0x0ab4_9a1c_e004_0001;
/// Seed domain for the offline level line (never mistaken for an AB1 digest).
pub const OFFLINE_LEVEL_DOMAIN: u64 = 0x0ab4_0ff1_1e00_0001;

/// Reserved door name; the router does not register it today.
pub fn tool_name() -> &'static str {
    TOOL_BUILD_GAME
}

/// True: this stage previews and packages only, never mutates the document.
pub fn readonly() -> bool {
    true
}

/// What stopped a slice build. Every message names the stage plus the legal
/// values; failures before packaging write nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameError {
    /// Machine code (`empty_brief`, `encounter_refused`, ...).
    pub code: &'static str,
    /// Human detail with the stage and the fix embedded.
    pub message: String,
}

impl GameError {
    /// New error with a static code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Replay-divergence refusal: the two digests that disagreed.
    pub fn replay_mismatch(play: u64, replay: u64) -> Self {
        Self::new(
            "replay_mismatch",
            format!(
                "slice replay diverged: play digest {play:016x} != replay digest {replay:016x}; \
                 the recorded inputs must rerun byte-identically with providers off"
            ),
        )
    }
}

impl core::fmt::Display for GameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for GameError {}

/// Mix one little-endian word into a running hash.
fn mix_word(hash: &mut u64, word: u64) {
    *hash ^= word;
    *hash = hash.wrapping_mul(FNV64_PRIME);
}

/// Which level path fed this slice. Receipts say what ran: the AB1 chain on
/// the MCP door, or the deterministic offline digest when the door is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelPath {
    /// Real AB1 `BuildReport` line, consumed verbatim through the door.
    Ab1,
    /// Deterministic offline digest; namespaced so it never poses as AB1.
    Offline,
}

impl LevelPath {
    /// Stable tag for receipts and digests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ab1 => "ab1",
            Self::Offline => "offline",
        }
    }
}

/// Level input line feeding the slice: digest, seed, route span and repairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelLine {
    /// FNV over the placement table (AB1) or the offline digest.
    pub level_digest: u64,
    /// FNV over the cell bytes (AB1) or the offline cell digest.
    pub cell_digest: u64,
    /// AB1 `next_ab4` verbatim, or the namespaced offline line.
    pub line: String,
    /// Which path produced this line.
    pub path: LevelPath,
}

/// AB1 bind, live only behind the `mcp` door: envelope, plan, solve, gate,
/// then consume the passing `BuildReport` verbatim. Refusals refuse the whole
/// slice with the AB1 conflicts named. Compiled out without the door, where
/// the offline digest below keeps the chain testable.
#[cfg(feature = "mcp")]
fn level_line(text: &str, seed: u64) -> Result<LevelLine, GameError> {
    use super::build_level::{build, envelope, BuildOutcome};
    let env = envelope(text, seed).map_err(|error| {
        GameError::new("level_refused", format!("AB1 envelope refused: {error}"))
    })?;
    match build(&env)
        .map_err(|error| GameError::new("level_refused", format!("AB1 plan refused: {error}")))?
    {
        BuildOutcome::Placed(level) => Ok(LevelLine {
            level_digest: level.report.level_digest,
            cell_digest: level.report.cell_digest,
            line: level.report.next_ab4.clone(),
            path: LevelPath::Ab1,
        }),
        BuildOutcome::Refused(refusal) => Err(GameError::new(
            "level_refused",
            format!(
                "AB1 refused after {} repair(s): {}; conflicts: [{}]",
                refusal.repairs_used,
                refusal.reason,
                refusal.conflicts.join(", ")
            ),
        )),
    }
}

/// Offline level line: deterministic digest over the shared envelope id, the
/// level text and the seed. Same inputs give the same line on every machine;
/// the `offline` path tag keeps it apart from real AB1 lines.
#[cfg(not(feature = "mcp"))]
fn level_line(text: &str, seed: u64) -> Result<LevelLine, GameError> {
    let brief_id = BriefEnvelope::new(text, seed, BriefStage::Slice)
        .map_err(|error| GameError::new(error.code(), error.to_string()))?
        .brief_id();
    let level_digest = {
        let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ OFFLINE_LEVEL_DOMAIN);
        hasher.write(format!("level|{brief_id:016x}|{seed}|{text}").as_bytes());
        hasher.finish()
    };
    let cell_digest = {
        let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ (OFFLINE_LEVEL_DOMAIN ^ 0xce11));
        hasher.write(format!("cell|{brief_id:016x}|{seed}|{text}").as_bytes());
        hasher.finish()
    };
    Ok(LevelLine {
        level_digest,
        cell_digest,
        line: format!(
            "{} input (offline): brief {brief_id:016x} seed {seed} digest {level_digest:016x}",
            BriefStage::Slice.as_str()
        ),
        path: LevelPath::Offline,
    })
}

/// Plain lobby params the AB8 door bind passes verbatim to
/// `engine_game::modes_world::LobbyBoot::new` with `WorldMode::Arena`, then
/// drives with `ModeSession::boot`, the scripted ticks, `digest` and
/// `replay`. Plain primitives only: no WD6 type is mirrored here; the
/// `engine-game` dependency edge lands with AB8 (reported to the
/// orchestrator), which turns this struct into the direct call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LobbyBootParams {
    /// Master seed; expanded per stream, never aliased.
    pub seed: u64,
    /// Fixed steps: five minutes at 60 Hz.
    pub ticks: u32,
    /// Combatants requested.
    pub fighters: u8,
    /// Arena half-extent in meters.
    pub half_extent_m: f32,
}

impl LobbyBootParams {
    /// Params for one slice seed: the exact values the door passes through.
    pub const fn for_slice(seed: u64) -> Self {
        Self {
            seed,
            ticks: PLAY_TICKS,
            fighters: SLICE_FIGHTERS,
            half_extent_m: 20.0,
        }
    }
}

/// Slice play session: a deterministic fixed-tick session over the encounter
/// plan and the character digest. Integer grid movement, permille hp debited
/// by the encounter damage rows, checkpoint folds every minute-tip; same boot
/// params give the same digest on every machine, no wall clock, no floats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceSession {
    /// Input stream state (seeded at boot).
    stream: u64,
    /// Position in integer cm, clamped to the half-extent.
    x_cm: i32,
    /// Position in integer cm, clamped to the half-extent.
    z_cm: i32,
    /// Hp in permille.
    hp: u32,
    /// Ticks simulated so far.
    tick: u32,
    /// Damage per hit in permille, from the encounter rows.
    damage: u16,
    /// Running FNV fold of every tick.
    hash: u64,
    /// Checkpoint folds, one per `CHECKPOINT_EVERY` ticks, preallocated.
    checkpoints: Vec<u64>,
}

impl SliceSession {
    /// Boot from the slice seed, the encounter plan and the character digest.
    /// Streams fork without aliasing: each lane mixes its own domain.
    pub fn boot(seed: u64, plan_id: u64, character_digest: u64, damage: u16) -> Self {
        let mut stream = PLAY_DOMAIN ^ seed;
        let mut stream_hasher = Fnv64::with_state(FNV64_OFFSET ^ stream);
        stream_hasher.write(&plan_id.to_le_bytes());
        stream = stream_hasher.finish();
        stream_hasher = Fnv64::with_state(FNV64_OFFSET ^ stream);
        stream_hasher.write(&character_digest.to_le_bytes());
        stream = stream_hasher.finish();
        let mut checkpoints = Vec::new();
        checkpoints.reserve_exact(CHECKPOINT_COUNT);
        Self {
            stream,
            x_cm: 0,
            z_cm: 0,
            hp: SLICE_HP_PERMILLE,
            tick: 0,
            damage,
            hash: FNV64_OFFSET,
            checkpoints,
        }
    }

    /// Ticks simulated so far.
    pub const fn cursor(&self) -> u32 {
        self.tick
    }

    /// Stored checkpoint folds.
    pub fn checkpoints(&self) -> &[u64] {
        &self.checkpoints
    }

    /// Converged digest of the recorded run. Integer folds only.
    pub const fn digest(&self) -> u64 {
        self.hash
    }

    /// Advance one fixed tick: scripted input from the seeded stream, clamped
    /// grid step, periodic damage, fold. O(1) integer ops, no allocation.
    pub fn tick(&mut self) {
        let draw = splitmix64(self.stream);
        self.stream = self.stream.wrapping_add(GOLDEN_GAMMA);
        let dx = (draw % 3) as i32 - 1;
        let dz = ((draw >> 2) % 3) as i32 - 1;
        self.x_cm = (self.x_cm + dx * STEP_CM).clamp(-SLICE_HALF_EXTENT_CM, SLICE_HALF_EXTENT_CM);
        self.z_cm = (self.z_cm + dz * STEP_CM).clamp(-SLICE_HALF_EXTENT_CM, SLICE_HALF_EXTENT_CM);
        self.tick += 1;
        if self.tick.is_multiple_of(HIT_EVERY_TICKS) {
            self.hp = self.hp.saturating_sub(u32::from(self.damage));
        }
        mix_word(&mut self.hash, self.x_cm as u64);
        mix_word(&mut self.hash, self.z_cm as u64);
        mix_word(&mut self.hash, u64::from(self.hp));
        mix_word(&mut self.hash, draw & 0xff);
        mix_word(&mut self.hash, u64::from(self.tick));
        if self.tick.is_multiple_of(CHECKPOINT_EVERY) {
            self.checkpoints.push(self.hash);
        }
    }

    /// Play the full five minutes from boot.
    pub fn play_full(&mut self) {
        for _ in 0..PLAY_TICKS {
            self.tick();
        }
    }

    /// Replay from the same boot params and fail loudly on any divergence.
    /// Returns the replay digest, which must equal `digest`.
    pub fn replay(&self, seed: u64, plan_id: u64, character_digest: u64) -> Result<u64, GameError> {
        let mut again = Self::boot(seed, plan_id, character_digest, self.damage);
        again.play_full();
        if again.checkpoints != self.checkpoints || again.hash != self.hash {
            return Err(GameError::replay_mismatch(self.hash, again.hash));
        }
        Ok(again.hash)
    }

    /// Cross-check another session: equal digests and tips pass, anything
    /// else is a loud replay mismatch (corruption and seed swaps fail here).
    pub fn verify_against(&self, other: &Self) -> Result<(), GameError> {
        if other.checkpoints == self.checkpoints && other.hash == self.hash {
            return Ok(());
        }
        Err(GameError::replay_mismatch(self.hash, other.hash))
    }
}

/// Eval report for one brief: every stage digest plus the boot, play and
/// replay proof. One report per brief; the ten-fixture test asserts each one
/// passes with 18_000 ticks played and an exact replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalReport {
    /// Shared envelope brief id.
    pub brief_id: u64,
    /// Build seed.
    pub seed: u64,
    /// Encounter plan id.
    pub plan_id: u64,
    /// Encounter fork id of the passing window.
    pub fork_id: u64,
    /// Typed encounter repairs consumed.
    pub encounter_repairs: u8,
    /// Character receipt digest.
    pub character_digest: u64,
    /// Level digest (AB1 or offline).
    pub level_digest: u64,
    /// Cell digest (AB1 or offline).
    pub cell_digest: u64,
    /// Which level path fed this slice.
    pub level_path: LevelPath,
    /// True once the package boots (header plus digest verify).
    pub boot_ok: bool,
    /// Ticks played; passing slices play the full five minutes.
    pub ticks_played: u32,
    /// Stored checkpoints; passing slices hold `CHECKPOINT_COUNT`.
    pub checkpoints: u32,
    /// Play digest after five minutes.
    pub play_digest: u64,
    /// Replay digest; must equal `play_digest`.
    pub replay_digest: u64,
    /// True when replay matched exactly.
    pub replay_ok: bool,
    /// Package digest.
    pub package_digest: u64,
    /// Package length in bytes.
    pub package_bytes: u32,
    /// Always `"pass"` here; refusals ride `GameError`.
    pub verdict: String,
}

impl EvalReport {
    /// True when the slice booted, played five minutes and replayed exactly.
    pub fn passed(&self) -> bool {
        self.boot_ok
            && self.replay_ok
            && self.verdict == "pass"
            && self.ticks_played == PLAY_TICKS
            && self.checkpoints == CHECKPOINT_COUNT as u32
            && self.play_digest == self.replay_digest
    }
}

/// Packaged playable build: the chained stage outputs, the five-minute play
/// proof, the eval report and the canonical package bytes. This is the AB4
/// input the AB8 door bind and the AB9 offline replay consume.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayableSlice {
    /// Shared slice envelope record line (AB9 replays this verbatim).
    pub record_line: String,
    /// Landed encounter plan (AB3 public output, stored verbatim).
    pub encounter: BuiltEncounter,
    /// Assembled character (AB2 public output, stored verbatim).
    pub character: BuiltCharacter,
    /// Level line plus digests (AB1 verbatim, or the offline digest).
    pub level: LevelLine,
    /// Play digest after five minutes.
    pub play_digest: u64,
    /// Stored checkpoints (300 tips).
    pub checkpoints: Vec<u64>,
    /// Eval report for this brief.
    pub eval: EvalReport,
    /// Canonical package bytes; `boot_package` verifies them.
    pub package: Vec<u8>,
    /// FNV over the package bytes.
    pub package_digest: u64,
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], at: &mut usize) -> Result<u32, GameError> {
    let end = at.saturating_add(4);
    if end > bytes.len() {
        return Err(GameError::new(
            "bad_package",
            "package is truncated; rebuild the slice via `build_game`",
        ));
    }
    let mut lane = [0u8; 4];
    lane.copy_from_slice(&bytes[*at..end]);
    *at = end;
    Ok(u32::from_le_bytes(lane))
}

fn read_u64(bytes: &[u8], at: &mut usize) -> Result<u64, GameError> {
    let end = at.saturating_add(8);
    if end > bytes.len() {
        return Err(GameError::new(
            "bad_package",
            "package is truncated; rebuild the slice via `build_game`",
        ));
    }
    let mut lane = [0u8; 8];
    lane.copy_from_slice(&bytes[*at..end]);
    *at = end;
    Ok(u64::from_le_bytes(lane))
}

impl PlayableSlice {
    /// Canonical package bytes: magic, version, digests, counts, checkpoint
    /// tips, trailer digest. Same slice gives the same bytes twice.
    pub fn package_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(128 + self.checkpoints.len() * 8);
        out.extend_from_slice(&PACKAGE_MAGIC);
        out.push(PACKAGE_VERSION);
        write_u64(&mut out, self.eval.brief_id);
        write_u64(&mut out, self.eval.seed);
        write_u64(&mut out, self.eval.plan_id);
        write_u64(&mut out, self.eval.fork_id);
        write_u64(&mut out, self.eval.character_digest);
        write_u64(&mut out, self.eval.level_digest);
        write_u64(&mut out, self.eval.cell_digest);
        write_u64(&mut out, self.play_digest);
        write_u32(&mut out, self.eval.ticks_played);
        write_u32(&mut out, self.checkpoints.len() as u32);
        for tip in &self.checkpoints {
            write_u64(&mut out, *tip);
        }
        let mut digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ PLAY_DOMAIN);
        digest_hasher.write(&out);
        let digest = digest_hasher.finish();
        write_u64(&mut out, digest);
        out
    }

    /// Boot a package: verify magic, version, length, tip count and the
    /// trailer digest. Anything else is a loud `bad_package`, never a guess.
    pub fn boot_package(bytes: &[u8]) -> Result<(u64, u64), GameError> {
        let bad = |detail: &str| {
            GameError::new(
                "bad_package",
                format!("{detail}; rebuild the slice via `build_game`"),
            )
        };
        if bytes.len() < 4 + 1 + 8 * 8 + 4 + 4 + 8 || bytes.len() > PACKAGE_MAX_BYTES {
            return Err(bad("package has a bad length"));
        }
        if bytes[0..4] != PACKAGE_MAGIC {
            return Err(bad("package magic is not ABG1"));
        }
        if bytes[4] != PACKAGE_VERSION {
            return Err(bad("package version is not 1"));
        }
        let mut at = 5usize;
        let brief_id = read_u64(bytes, &mut at)?;
        let _seed = read_u64(bytes, &mut at)?;
        let _plan = read_u64(bytes, &mut at)?;
        let _fork = read_u64(bytes, &mut at)?;
        let _character = read_u64(bytes, &mut at)?;
        let _level = read_u64(bytes, &mut at)?;
        let _cell = read_u64(bytes, &mut at)?;
        let play_digest = read_u64(bytes, &mut at)?;
        let ticks = read_u32(bytes, &mut at)?;
        let tips = read_u32(bytes, &mut at)?;
        if ticks != PLAY_TICKS || tips != CHECKPOINT_COUNT as u32 {
            return Err(bad(
                "package ticks or tip count are not the five-minute shape",
            ));
        }
        let tips_end = at.saturating_add(CHECKPOINT_COUNT.saturating_mul(8));
        let trailer_at = tips_end;
        if bytes.len() != trailer_at.saturating_add(8) {
            return Err(bad("package has trailing bytes past the tips"));
        }
        let mut cursor = at;
        for _ in 0..CHECKPOINT_COUNT {
            read_u64(bytes, &mut cursor)?;
        }
        let mut digest_at = trailer_at;
        let trailer = read_u64(bytes, &mut digest_at)?;
        let mut trailer_hasher = Fnv64::with_state(FNV64_OFFSET ^ PLAY_DOMAIN);
        trailer_hasher.write(&bytes[..trailer_at]);
        if trailer_hasher.finish() != trailer {
            return Err(bad("package digest does not verify"));
        }
        Ok((brief_id, play_digest))
    }
}

/// One slice fixture: level directives the AB1 chain solves, a `#` prose
/// comment the AB3 translator reads (the AB1 grammar ignores `#` comments),
/// and the character body. The prose lines are the ten landing encounter
/// briefs, so every fixture passes every gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceFixture {
    /// Level lines plus one `#` prose comment.
    pub text: &'static str,
    /// Character body assembled for this slice.
    pub variant: VariantId,
    /// True adds the wet-blood veteran finish.
    pub bloodied: bool,
    /// Build seed.
    pub seed: u64,
}

/// Ten briefs to ten packaged builds: each boots, plays five minutes and
/// replays exactly, with one eval report per brief. Each text holds level
/// directives plus one `#` prose comment carrying a landing encounter brief.
pub fn fixture_briefs() -> [SliceFixture; 10] {
    [
        SliceFixture {
            text: "prop brazier slice_01\nwant slice_01 next_to wall:north [0,80]\nspawn slice_01\n# a quiet duel at dawn in the training yard",
            variant: VariantId::Hero,
            bloodied: false,
            seed: 5000,
        },
        SliceFixture {
            text: "prop crate slice_02\nwant slice_02 next_to wall:south [0,80]\nspawn slice_02\n# a rivalry grudge match under the noon sun",
            variant: VariantId::OppMaleLight,
            bloodied: false,
            seed: 5001,
        },
        SliceFixture {
            text: "prop pillar slice_03\nwant slice_03 next_to wall:east [0,80]\nspawn slice_03\n# a brutal cauldron final before the crowd",
            variant: VariantId::OppFemaleMedium,
            bloodied: false,
            seed: 5002,
        },
        SliceFixture {
            text: "prop torch slice_04\nwant slice_04 next_to wall:west [0,80]\nspawn slice_04\n# an advanced duel with footwork drills",
            variant: VariantId::OppFemaleLight,
            bloodied: false,
            seed: 5003,
        },
        SliceFixture {
            text: "prop altar slice_05\nwant slice_05 next_to wall:north [0,80]\nspawn slice_05\n# a training duel for the new recruits",
            variant: VariantId::Hero,
            bloodied: true,
            seed: 5004,
        },
        SliceFixture {
            text: "prop stand slice_06\nwant slice_06 next_to wall:south [0,80]\nspawn slice_06\n# a pressure rivalry between rival houses",
            variant: VariantId::OppMaleLight,
            bloodied: false,
            seed: 5005,
        },
        SliceFixture {
            text: "prop post slice_07\nwant slice_07 next_to wall:east [0,80]\nspawn slice_07\n# an easy sparring circle for beginners",
            variant: VariantId::OppFemaleMedium,
            bloodied: false,
            seed: 5006,
        },
        SliceFixture {
            text: "prop bench slice_08\nwant slice_08 next_to wall:west [0,80]\nspawn slice_08\n# a hard final between champions",
            variant: VariantId::OppFemaleLight,
            bloodied: false,
            seed: 5007,
        },
        SliceFixture {
            text: "prop barrel slice_09\nwant slice_09 next_to wall:north [0,80]\nspawn slice_09\n# a marathon duel of endurance",
            variant: VariantId::Hero,
            bloodied: false,
            seed: 5008,
        },
        SliceFixture {
            text: "prop banner slice_10\nwant slice_10 next_to wall:south [0,80]\nspawn slice_10\n# an exhausting rivalry across the sand",
            variant: VariantId::OppMaleLight,
            bloodied: false,
            seed: 5009,
        },
    ]
}

/// Character clips every slice assembles: all four sit in the CT7 kit table,
/// so every variant lands rigged and animated past the import gates.
pub const SLICE_CLIPS: [&str; 4] = ["idle", "run", "attack_1h_1", "block_shield"];

/// Build one packaged slice: shared envelope, AB3 encounter, AB2 character,
/// AB1 level line, five-minute play, exact replay, package plus eval report.
/// Stage refusals refuse the whole slice with the stage named; nothing is
/// written by construction.
pub fn build_slice(
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
) -> Result<PlayableSlice, GameError> {
    let envelope = BriefEnvelope::new(text, seed, BriefStage::Slice)
        .map_err(|error| GameError::new(error.code(), error.to_string()))?;
    if u64::from(envelope.repair_budget()) < u64::from(MAX_REPAIRS) {
        return Err(GameError::new(
            "over_budget",
            format!(
                "slice budget is {} repairs; at most {MAX_REPAIRS} repair rounds, \
                 then the stage refuses with its findings",
                envelope.repair_budget()
            ),
        ));
    }

    let encounter_brief = EncounterBrief::new(text, seed).map_err(|error| {
        let message = error.to_string();
        GameError::new(error.code(), message)
    })?;
    let encounter = build_encounter(&encounter_brief, &DeterministicOracle).map_err(|refusal| {
        GameError::new("encounter_refused", format!("AB3 refused: {refusal}"))
    })?;

    let character_brief = CharacterBrief {
        brief_text: text.to_owned(),
        variant,
        want_clips: SLICE_CLIPS.iter().map(|slot| (*slot).to_owned()).collect(),
        bloodied,
        license: LicenseRef::ct7(),
        repairs_used: 0,
        seed,
    };
    let character = build_character(&character_brief).map_err(|error| {
        GameError::new(
            "character_refused",
            format!("AB2 refused with {}: {error}", error.code),
        )
    })?;

    let level = level_line(text, seed)?;

    let mut session = SliceSession::boot(
        seed,
        encounter.plan_id,
        character.digest,
        encounter.rows.attack_damage,
    );
    session.play_full();
    let play_digest = session.digest();
    let replay_digest = session.replay(seed, encounter.plan_id, character.digest)?;
    let checkpoints = session.checkpoints.clone();

    let record_line = envelope.record_line();
    let mut slice = PlayableSlice {
        record_line,
        encounter,
        character,
        level,
        play_digest,
        checkpoints,
        eval: EvalReport {
            brief_id: envelope.brief_id(),
            seed,
            plan_id: 0,
            fork_id: 0,
            encounter_repairs: 0,
            character_digest: 0,
            level_digest: 0,
            cell_digest: 0,
            level_path: LevelPath::Offline,
            boot_ok: false,
            ticks_played: session.cursor(),
            checkpoints: session.checkpoints.len() as u32,
            play_digest,
            replay_digest,
            replay_ok: play_digest == replay_digest,
            package_digest: 0,
            package_bytes: 0,
            verdict: "pass".to_owned(),
        },
        package: Vec::new(),
        package_digest: 0,
    };
    slice.eval.plan_id = slice.encounter.plan_id;
    slice.eval.fork_id = slice.encounter.fork_id;
    slice.eval.encounter_repairs = slice.encounter.repairs_used.len() as u8;
    slice.eval.character_digest = slice.character.digest;
    slice.eval.level_digest = slice.level.level_digest;
    slice.eval.cell_digest = slice.level.cell_digest;
    slice.eval.level_path = slice.level.path;
    let package = slice.package_bytes();
    if package.len() > PACKAGE_MAX_BYTES {
        return Err(GameError::new(
            "over_budget",
            format!(
                "package holds {} bytes; at most {PACKAGE_MAX_BYTES}",
                package.len()
            ),
        ));
    }
    let mut package_hasher = Fnv64::with_state(FNV64_OFFSET ^ PLAY_DOMAIN);
    package_hasher.write(&package);
    let package_digest = package_hasher.finish();
    slice.eval.package_digest = package_digest;
    slice.eval.package_bytes = package.len() as u32;
    let (boot_id, boot_digest) = PlayableSlice::boot_package(&package)
        .map_err(|error| GameError::new(error.code, error.message))?;
    slice.eval.boot_ok = boot_id == envelope.brief_id() && boot_digest == play_digest;
    if !slice.eval.boot_ok {
        return Err(GameError::new(
            "replay_mismatch",
            "fresh package failed to boot its own play digest; rebuild the slice",
        ));
    }
    slice.package = package;
    slice.package_digest = package_digest;
    Ok(slice)
}

/// Build one fixture slice by index.
pub fn build_fixture(index: usize) -> Result<PlayableSlice, GameError> {
    let fixtures = fixture_briefs();
    let fixture = fixtures.get(index).ok_or_else(|| {
        GameError::new(
            "bad_brief",
            format!(
                "fixture {index} is unknown; submit 0..={} for the ten packaged builds",
                fixtures.len() - 1
            ),
        )
    })?;
    build_slice(
        fixture.text,
        fixture.seed,
        fixture.variant,
        fixture.bloodied,
    )
}

mod replay;
pub use replay::*;

// WD6 lobby verbs play two scripted minutes, LW3 voices staff one offline
// visit over WD2 data, everything replays twice providers-off. Owner is
// `engine-endpoint`; consumer is the endpoint MCP service plus the playtest
// eval. Units: fixed ticks (60 Hz), meters (WD2 zones), integer checkpoints.
// Budget: five fixtures, 7_200 ticks each, O(1) integer ops per tick; visits
// run off-tick and may allocate. Errors are typed `GameError`; refusals name
// the stage plus the fix and write nothing — the only writes land in the owned
// `LudusNpcs`, which drops. Determinism: same brief plus seed visits identically.

/// Two scripted minutes at the slice tick rate (the AB8 door play length).
#[cfg(feature = "mcp")]
pub const VISIT_TICKS: u32 = 2 * 60 * TICKS_PER_SECOND;
/// Visit briefs per slice run: the first five packaged fixtures.
#[cfg(feature = "mcp")]
pub const VISIT_COUNT: usize = 5;
/// Fixed tick one offline visit speaks at (the last play tick).
#[cfg(feature = "mcp")]
pub const VISIT_TICK: u64 = VISIT_TICKS as u64;
/// Ticks the WD6 door session drives directly: the lobby-test smoke shape.
/// The full two minutes play through the same authority in one pass below
/// (`run`), because `ModeSession::tick` reruns the prefix per tick (O(n^2);
/// the game crate keeps sessions small), which no test budget fits at 7_200.
#[cfg(feature = "mcp")]
pub const LOBBY_SMOKE_TICKS: u32 = 64;

/// Five practice-corner briefs: the first five packaged fixtures, whose level
/// directives the AB1 chain solves and whose prose the encounter translator
/// reads. Same array twice is the same visit set.
#[cfg(feature = "mcp")]
pub fn visit_briefs() -> [SliceFixture; VISIT_COUNT] {
    let all = fixture_briefs();
    [all[0], all[1], all[2], all[3], all[4]]
}

/// Scripted wish for one visit tick: a slow square that stays inside the
/// arena, mirroring the WD6 lobby test script. Pure.
#[cfg(feature = "mcp")]
fn visit_script(tick: u32) -> engine_arena::FighterInput {
    let (mx, mz) = match (tick / 15) % 4 {
        0 => (100, 0),
        1 => (0, 100),
        2 => (-100, 0),
        _ => (0, -100),
    };
    engine_arena::FighterInput {
        tick,
        fighter: engine_arena::FighterId(engine_game::HUMAN_SLOT),
        mx,
        mz,
        facing: 0,
        buttons: Default::default(),
    }
}

/// FNV-1a fold over one arena checkpoint stream: tick then hash per tip.
/// The visit-play receipt only; equality across runs is the proof, the
/// absolute value is never compared across shapes.
#[cfg(feature = "mcp")]
fn fold_play_digest(checkpoints: &[engine_arena::Checkpoint]) -> u64 {
    let mut hash: u64 = FNV64_OFFSET;
    for checkpoint in checkpoints {
        hash ^= checkpoint.tick as u64;
        hash = hash.wrapping_mul(FNV64_PRIME);
        hash ^= checkpoint.hash;
        hash = hash.wrapping_mul(FNV64_PRIME);
    }
    hash
}

/// One ludus-visit slice: the AB4 stage outputs plus the WD6 two-minute play
/// proof and the LW3 offline visit over WD2 stations. This is the AB8 door
/// bind the playtest eval consumes.
#[cfg(feature = "mcp")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitSlice {
    /// Shared slice envelope record line (replayed verbatim).
    pub record_line: String,
    /// Shared envelope brief id (AB7).
    pub brief_id: u64,
    /// Build seed.
    pub seed: u64,
    /// Encounter plan id (AB3).
    pub plan_id: u64,
    /// Encounter fork id (AB3).
    pub fork_id: u64,
    /// Character receipt digest (AB2).
    pub character_digest: u64,
    /// Level digest (AB1 verbatim).
    pub level_digest: u64,
    /// Cell digest (AB1 verbatim).
    pub cell_digest: u64,
    /// Which level path fed this slice.
    pub level_path: LevelPath,
    /// Ticks played; passing visits play the full two minutes.
    pub ticks_played: u32,
    /// Play digest after two minutes.
    pub play_digest: u64,
    /// First providers-off replay digest; must equal `play_digest`.
    pub replay_digest: u64,
    /// Second providers-off replay digest; must equal `play_digest`.
    pub replay_digest_again: u64,
    /// WD6 door smoke digest (`LOBBY_SMOKE_TICKS` ticks through the verbs).
    pub door_digest: u64,
    /// WD6 door smoke replay digest; must equal `door_digest`.
    pub door_replay: u64,
    /// Fixed tick the offline visit spoke at.
    pub visit_tick: u64,
    /// Voiced display lines in role order (`Trainer @practice_yard: ...`).
    pub visit_lines: Vec<String>,
    /// Session checkpoint after the visit.
    pub visit_checkpoint: u64,
    /// Lines that fell back to the scripted table (all of them offline).
    pub visit_scripted: usize,
    /// Providers-off log checkpoint; both log replays must match it.
    pub log_checkpoint: u64,
    /// True once a fresh rebuild from seed plus bout reproduced the visit.
    pub rebuild_ok: bool,
    /// True once the lobby booted and the full two minutes played.
    pub boot_ok: bool,
    /// Always `"pass"` here; refusals ride `GameError`.
    pub verdict: String,
}

#[cfg(feature = "mcp")]
impl VisitSlice {
    /// True when the visit booted, played two minutes, replayed twice
    /// providers-off, drove the door verbs cleanly and voiced one stationed
    /// line per role.
    pub fn passed(&self) -> bool {
        self.boot_ok
            && self.rebuild_ok
            && self.verdict == "pass"
            && self.ticks_played == VISIT_TICKS
            && self.play_digest == self.replay_digest
            && self.play_digest == self.replay_digest_again
            && self.door_digest == self.door_replay
            && self.visit_lines.len() == engine_game::ludus_npcs::NpcRole::ALL.len()
    }
}

/// Build one ludus-visit slice: shared envelope (AB7), AB3 encounter, AB2
/// character, AB1 level line, two scripted minutes of arena play replayed
/// twice providers-off, the WD6 lobby verbs driven directly on a smoke
/// session, one offline LW3 visit over WD2 stations, then the visit log
/// replayed twice providers-off. Stage refusals refuse the whole visit with
/// the stage named; nothing is written by construction.
#[cfg(feature = "mcp")]
pub fn build_visit_slice(
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
) -> Result<VisitSlice, GameError> {
    use engine_game::ludus_npcs::{
        ludus_contains, ludus_zone, FightReport, LudusNpcs, LUDUS_SPAWNS,
    };
    use engine_game::modes_world::{LobbyBoot, ModeSession, WorldMode};

    let envelope = BriefEnvelope::new(text, seed, BriefStage::Slice)
        .map_err(|error| GameError::new(error.code(), error.to_string()))?;
    if u64::from(envelope.repair_budget()) < u64::from(MAX_REPAIRS) {
        return Err(GameError::new(
            "over_budget",
            format!(
                "slice budget is {} repairs; at most {MAX_REPAIRS} repair rounds, \
                 then the stage refuses with its findings",
                envelope.repair_budget()
            ),
        ));
    }

    let encounter_brief = EncounterBrief::new(text, seed).map_err(|error| {
        let message = error.to_string();
        GameError::new(error.code(), message)
    })?;
    let encounter = build_encounter(&encounter_brief, &DeterministicOracle).map_err(|refusal| {
        GameError::new("encounter_refused", format!("AB3 refused: {refusal}"))
    })?;

    let character_brief = CharacterBrief {
        brief_text: text.to_owned(),
        variant,
        want_clips: SLICE_CLIPS.iter().map(|slot| (*slot).to_owned()).collect(),
        bloodied,
        license: LicenseRef::ct7(),
        repairs_used: 0,
        seed,
    };
    let character = build_character(&character_brief).map_err(|error| {
        GameError::new(
            "character_refused",
            format!("AB2 refused with {}: {error}", error.code),
        )
    })?;

    let level = level_line(text, seed)?;

    // WD6 lobby verbs, called directly: no WD6 type is mirrored here. The boot
    // validates the full two-minute shape (fighters plus extent ride the AB4
    // door values verbatim); the smoke drives the tick, digest and replay
    // verbs through the real session.
    let lobby = LobbyBoot::new(WorldMode::Arena, seed, VISIT_TICKS, SLICE_FIGHTERS, 20.0);
    let mut session = ModeSession::boot(lobby).map_err(|error| {
        GameError::new(
            "lobby_refused",
            format!("WD6 lobby refused: {error}; submit 2 fighters on a 5.0..=100.0 m arena"),
        )
    })?;
    for tick in 0..LOBBY_SMOKE_TICKS {
        let accepted = session.tick(visit_script(tick)).map_err(|error| {
            GameError::new(
                "session_refused",
                format!("WD6 session refused scripted tick {tick}: {error}"),
            )
        })?;
        if !accepted {
            return Err(GameError::new(
                "session_refused",
                format!(
                    "WD6 session dropped scripted tick {tick}; the script names slot 0 wishes only"
                ),
            ));
        }
    }
    let door_digest = session.digest();
    let door_replay = session.replay().map_err(|error| {
        GameError::new(
            "replay_mismatch",
            format!(
                "WD6 door smoke diverged on its providers-off replay: {error}; \
                 the recorded inputs must rerun byte-identically with providers off"
            ),
        )
    })?;
    if door_replay != door_digest {
        return Err(GameError::replay_mismatch(door_digest, door_replay));
    }

    // Two scripted minutes through the same arena authority in one O(n) pass
    // per run: the scripted tape drives `run` to the last tip, then two more
    // passes replay it providers-off. A short tape here refuses loudly with
    // the legal shape instead of a partial play.
    let mut tape = Vec::with_capacity(VISIT_TICKS as usize);
    for tick in 0..VISIT_TICKS {
        tape.push(visit_script(tick));
    }
    let arena_config = engine_arena::ArenaConfig {
        seed,
        ticks: VISIT_TICKS,
        fighters: SLICE_FIGHTERS,
        half_extent_m: 20.0,
        table_version: 1,
    };
    let play_tape = engine_arena::FighterTape(tape);
    let play_tips = engine_arena::run(&arena_config, &play_tape);
    if play_tips.len() != VISIT_TICKS as usize {
        return Err(GameError::new(
            "session_refused",
            format!(
                "arena played {} of {VISIT_TICKS} ticks; the visit plays the full two minutes",
                play_tips.len()
            ),
        ));
    }
    let play_digest = fold_play_digest(&play_tips);
    let replay_digest = fold_play_digest(&engine_arena::run(&arena_config, &play_tape));
    if replay_digest != play_digest {
        return Err(GameError::replay_mismatch(play_digest, replay_digest));
    }
    let replay_digest_again = fold_play_digest(&engine_arena::run(&arena_config, &play_tape));
    if replay_digest_again != play_digest {
        return Err(GameError::replay_mismatch(play_digest, replay_digest_again));
    }

    // LW3 voices over WD2 data, providers off: every drill stands on the
    // practice yard, or the map content reloads instead of a guess.
    let yard = ludus_zone("practice_yard").ok_or_else(|| {
        GameError::new(
            "zone_unknown",
            "WD2 ludus zone `practice_yard` is missing; reload the ludus map content",
        )
    })?;
    for spawn in LUDUS_SPAWNS.iter() {
        if !ludus_contains(yard, spawn.x, spawn.z) {
            return Err(GameError::new(
                "zone_unknown",
                format!(
                    "WD2 spawn `{}` stands off the practice yard; reload the ludus map content",
                    spawn.name
                ),
            ));
        }
    }
    let mut ludus = LudusNpcs::new(seed);
    let bout = FightReport::new(VISIT_TICK, 0, 1, 5 + (play_digest % 96) as u32)
        .map_err(|error| GameError::new("visit_refused", format!("LW3 bout refused: {error}")))?;
    ludus.observe_fight(&bout).map_err(|error| {
        GameError::new(
            "visit_refused",
            format!("LW3 observe refused: {error}; fighters must differ below the cap"),
        )
    })?;
    let outcome = ludus.visit_offline(VISIT_TICK);
    for line in outcome.lines.iter() {
        if ludus_zone(line.role.station()).is_none() {
            return Err(GameError::new(
                "zone_unknown",
                format!(
                    "LW3 voice `{}` staffs unknown station `{}`; reload the ludus map content",
                    line.role.label(),
                    line.role.station()
                ),
            ));
        }
    }
    let entries = ludus.recorded_entries();
    let log_checkpoint = ludus.log_checkpoint();
    let replay_log = LudusNpcs::replay_checkpoint(&entries).map_err(|error| {
        GameError::new(
            "replay_mismatch",
            format!("visit log refused the first providers-off replay: {error}"),
        )
    })?;
    if replay_log != log_checkpoint {
        return Err(GameError::replay_mismatch(log_checkpoint, replay_log));
    }
    let replay_log_again = LudusNpcs::replay_checkpoint(&entries).map_err(|error| {
        GameError::new(
            "replay_mismatch",
            format!("visit log refused the second providers-off replay: {error}"),
        )
    })?;
    if replay_log_again != log_checkpoint {
        return Err(GameError::replay_mismatch(log_checkpoint, replay_log_again));
    }
    // Undo-cleanly: the only writes sit in the owned ludus, so rebuilding
    // from the same seed plus the same bout must reproduce the visit exactly;
    // dropping the value then undoes everything (no global, no disk).
    let mut twin = LudusNpcs::new(seed);
    twin.observe_fight(&bout).map_err(|error| {
        GameError::new(
            "visit_refused",
            format!("LW3 observe refused on rebuild: {error}"),
        )
    })?;
    let twin_outcome = twin.visit_offline(VISIT_TICK);
    let rebuild_ok = twin.checkpoint() == ludus.checkpoint()
        && twin.memory_bytes() == ludus.memory_bytes()
        && twin_outcome.lines == outcome.lines;

    Ok(VisitSlice {
        record_line: envelope.record_line(),
        brief_id: envelope.brief_id(),
        seed,
        plan_id: encounter.plan_id,
        fork_id: encounter.fork_id,
        character_digest: character.digest,
        level_digest: level.level_digest,
        cell_digest: level.cell_digest,
        level_path: level.path,
        ticks_played: play_tips.len() as u32,
        play_digest,
        replay_digest,
        replay_digest_again,
        door_digest,
        door_replay,
        visit_tick: outcome.tick,
        visit_lines: outcome.lines.iter().map(|line| line.display()).collect(),
        visit_checkpoint: outcome.checkpoint,
        visit_scripted: outcome.scripted,
        log_checkpoint,
        rebuild_ok,
        boot_ok: true,
        verdict: "pass".to_owned(),
    })
}

/// Build one visit slice by index over the five visit briefs.
#[cfg(feature = "mcp")]
pub fn build_visit_fixture(index: usize) -> Result<VisitSlice, GameError> {
    let briefs = visit_briefs();
    let fixture = briefs.get(index).ok_or_else(|| {
        GameError::new(
            "bad_brief",
            format!(
                "visit fixture {index} is unknown; submit 0..={} for the five visit briefs",
                briefs.len() - 1
            ),
        )
    })?;
    build_visit_slice(
        fixture.text,
        fixture.seed,
        fixture.variant,
        fixture.bloodied,
    )
}

#[cfg(test)]
mod tests {
    use super::super::build_encounter::{
        build_with_budget, MAX_REPAIRS as ENCOUNTER_REPAIRS, WIN_MAX_PERMILLE, WIN_MIN_PERMILLE,
    };
    use super::*;

    #[test]
    fn ten_briefs_boot_play_five_minutes_and_replay_exactly() {
        let fixtures = fixture_briefs();
        assert_eq!(fixtures.len(), 10);
        for (index, fixture) in fixtures.iter().enumerate() {
            let slice = build_slice(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .unwrap_or_else(|error| panic!("brief {index} must package: {error}"));
            let eval = &slice.eval;
            assert!(eval.boot_ok, "brief {index} boots");
            assert_eq!(
                eval.ticks_played, PLAY_TICKS,
                "brief {index} plays five minutes"
            );
            assert_eq!(
                eval.checkpoints, CHECKPOINT_COUNT as u32,
                "brief {index} stores every tip"
            );
            assert!(eval.replay_ok, "brief {index} replays");
            assert_eq!(eval.play_digest, eval.replay_digest, "brief {index}");
            assert!(eval.passed(), "brief {index} eval passes");
            assert_eq!(eval.verdict, "pass");
            assert!(
                eval.package_bytes as usize <= PACKAGE_MAX_BYTES,
                "brief {index} package fits"
            );
            // Same brief twice is the same build: the chain is deterministic.
            let again = build_slice(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect("rebuild lands");
            assert_eq!(slice, again, "brief {index} rebuilds identically");
            assert_eq!(slice.package_digest, again.package_digest);
        }
    }

    #[test]
    fn eval_report_names_every_stage_and_the_play_proof() {
        let slice = build_fixture(0).expect("fixture 0 packages");
        let eval = &slice.eval;
        let record = &slice.record_line;
        assert!(record.contains("playable-slice"), "record: {record}");
        assert!(record.contains("seed 5000"), "record: {record}");
        assert_ne!(eval.plan_id, 0, "encounter plan feeds the report");
        assert_ne!(eval.fork_id, 0, "encounter fork feeds the report");
        assert_ne!(
            eval.character_digest, 0,
            "character digest feeds the report"
        );
        assert_ne!(eval.level_digest, 0, "level digest feeds the report");
        assert_ne!(eval.cell_digest, 0, "cell digest feeds the report");
        assert_ne!(eval.play_digest, 0, "play digest is real");
        assert!(eval.encounter_repairs <= ENCOUNTER_REPAIRS as u8);
        assert_eq!(slice.checkpoints.len(), CHECKPOINT_COUNT);
        assert_eq!(slice.package.len(), eval.package_bytes as usize);
    }

    #[test]
    fn chained_stages_pass_their_own_gates() {
        for index in 0..10 {
            let slice = build_fixture(index).expect("fixture packages");
            assert!(
                slice.character.is_rigged(),
                "brief {index} character rigged"
            );
            assert!(
                slice.character.is_animated(),
                "brief {index} character animated"
            );
            slice.character.check_gates().expect("character gates pass");
            assert!(
                (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE)
                    .contains(&slice.encounter.sample.win_rate_permille),
                "brief {index} encounter inside the P3 band"
            );
            assert!(
                slice.encounter.repairs_used.len() <= ENCOUNTER_REPAIRS,
                "brief {index} encounter repairs bounded"
            );
        }
    }

    #[test]
    fn package_boots_and_detects_tamper() {
        let slice = build_fixture(3).expect("fixture 3 packages");
        let (brief_id, play_digest) =
            PlayableSlice::boot_package(&slice.package).expect("package boots");
        assert_eq!(brief_id, slice.eval.brief_id);
        assert_eq!(play_digest, slice.play_digest);
        // One flipped byte anywhere breaks the trailer: boot refuses loudly.
        for at in [5usize, 20, slice.package.len() / 2, slice.package.len() - 1] {
            let mut tampered = slice.package.clone();
            tampered[at] ^= 0x01;
            let error = PlayableSlice::boot_package(&tampered).expect_err("tamper must refuse");
            assert_eq!(error.code, "bad_package", "byte {at}");
        }
        let short = &slice.package[..slice.package.len() - 9];
        assert_eq!(
            PlayableSlice::boot_package(short)
                .expect_err("truncation must refuse")
                .code,
            "bad_package"
        );
        let mut wrong_magic = slice.package.clone();
        wrong_magic[0] = b'X';
        assert_eq!(
            PlayableSlice::boot_package(&wrong_magic)
                .expect_err("magic must refuse")
                .code,
            "bad_package"
        );
    }

    #[test]
    fn replay_detects_divergence_between_seeds() {
        let first = build_fixture(1).expect("fixture 1 packages");
        let second = build_fixture(2).expect("fixture 2 packages");
        assert_ne!(first.play_digest, second.play_digest, "seeds diverge");
        let a = SliceSession::boot(11, 22, 33, 120);
        let mut run_a = a.clone();
        run_a.play_full();
        let mut run_b = SliceSession::boot(11, 22, 33, 120);
        run_b.play_full();
        run_a.verify_against(&run_b).expect("identical runs verify");
        let mut run_c = SliceSession::boot(12, 22, 33, 120);
        run_c.play_full();
        let error = run_a
            .verify_against(&run_c)
            .expect_err("seed swap must fail");
        assert_eq!(error.code, "replay_mismatch");
        assert_ne!(first.eval.play_digest, 0);
    }

    #[test]
    fn empty_and_overlong_briefs_refuse_with_legal_values() {
        let error = build_slice("", 1, VariantId::Hero, false).expect_err("empty refuses");
        assert_eq!(error.code, "empty_brief");
        assert!(error.to_string().contains("1..="), "legal: {error}");
        let long = "prop crate x\n".repeat(2000);
        let error = build_slice(&long, 1, VariantId::Hero, false).expect_err("overlong refuses");
        assert_eq!(error.code, "brief_too_long");
        let error = build_fixture(10).expect_err("fixture 10 is unknown");
        assert_eq!(error.code, "bad_brief");
        assert!(error.to_string().contains("0..=9"), "legal: {error}");
    }

    #[test]
    fn encounter_refusal_fails_the_slice_with_the_stage_named() {
        let brief = EncounterBrief::new("an overpowered duel with oneshot blades", 77)
            .expect("brief valid");
        let refusal =
            build_with_budget(&brief, &DeterministicOracle, 0).expect_err("no budget refuses");
        let error = GameError::new("encounter_refused", format!("AB3 refused: {refusal}"));
        assert_eq!(error.code, "encounter_refused");
        assert!(
            error.to_string().contains("encounter_refused"),
            "stage named: {error}"
        );
    }

    #[test]
    fn slice_envelope_uses_the_shared_type_with_slice_budget() {
        let env = BriefEnvelope::new("prop crate probe_a\nspawn probe_a", 7, BriefStage::Slice)
            .expect("slice envelopes");
        assert_eq!(env.stage(), BriefStage::Slice);
        assert_eq!(env.repair_budget(), MAX_REPAIRS);
        assert!(env.record_line().contains("playable-slice"));
        assert_eq!(
            env.brief_id(),
            super::super::brief::brief_id_of("prop crate probe_a\nspawn probe_a", 7)
        );
    }

    #[test]
    fn lobby_params_carry_the_five_minute_contract() {
        let params = LobbyBootParams::for_slice(4242);
        assert_eq!(params.ticks, PLAY_TICKS);
        assert_eq!(params.ticks, 18_000);
        assert_eq!(params.fighters, SLICE_FIGHTERS);
        assert_eq!(params.seed, 4242);
        assert!(params.half_extent_m.is_finite() && params.half_extent_m > 0.0);
        assert_eq!(params.half_extent_m, 20.0);
        assert_eq!(params, LobbyBootParams::for_slice(4242));
        assert_ne!(params, LobbyBootParams::for_slice(4243));
    }

    #[test]
    fn checkpoints_are_preallocated_with_no_per_tick_growth() {
        let mut session = SliceSession::boot(9, 8, 7, 120);
        session.play_full();
        assert_eq!(session.cursor(), PLAY_TICKS);
        assert_eq!(session.checkpoints.len(), CHECKPOINT_COUNT);
        assert_eq!(
            session.checkpoints.capacity(),
            CHECKPOINT_COUNT,
            "tips preallocate exactly: the tick loop never reallocates"
        );
        assert_ne!(session.digest(), 0);
        let replayed = session.replay(9, 8, 7).expect("session replays");
        assert_eq!(replayed, session.digest());
    }

    #[test]
    fn offline_level_line_is_namespaced_and_deterministic() {
        let fixtures = fixture_briefs();
        let first = level_line(fixtures[0].text, fixtures[0].seed).expect("offline line lands");
        let again = level_line(fixtures[0].text, fixtures[0].seed).expect("offline line repeats");
        assert_eq!(first, again, "same inputs name the same level line");
        assert_ne!(first.level_digest, 0);
        assert_ne!(first.cell_digest, 0);
        assert_ne!(first.level_digest, first.cell_digest);
        #[cfg(not(feature = "mcp"))]
        {
            assert_eq!(first.path, LevelPath::Offline);
            assert!(
                first.line.contains("playable-slice"),
                "line: {}",
                first.line
            );
            assert!(
                first.line.contains("(offline)"),
                "honest path tag: {}",
                first.line
            );
        }
        let other = level_line(fixtures[1].text, fixtures[1].seed).expect("other line lands");
        assert_ne!(first.level_digest, other.level_digest, "briefs diverge");
    }

    #[test]
    fn ab11_brief_chain_provenance_audit_every_ab_write_carries_its_record() {
        // One command proves the AB11 contract: 100% of AB1-AB3 writes
        // carry their LW6 envelope record line end to end, undo restores
        // the digest, and records survive save/load byte-identically.
        let fixtures = fixture_briefs();
        assert_eq!(fixtures.len(), 10);
        for (index, fixture) in fixtures.iter().enumerate() {
            let slice = build_slice(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .unwrap_or_else(|error| panic!("brief {index} must package: {error}"));
            let chain = record_chain(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
                &slice,
            )
            .expect("brief records its chain");
            chain.verify_complete().expect("chain holds every stage");
            assert_eq!(
                chain.stages.len(),
                REPLAY_STAGES.len(),
                "brief {index} records every stage"
            );
            // Every stage carries its own LW6 line, verbatim from the
            // shared envelope, with real digests that survive save/load.
            for stage in REPLAY_STAGES {
                let record = chain.find_stage(stage).expect("stage recorded");
                let envelope = BriefEnvelope::new(fixture.text, fixture.seed, stage)
                    .expect("fixture envelopes");
                assert_eq!(
                    record.record_line,
                    envelope.record_line(),
                    "brief {index} stage `{}` carries its LW6 line",
                    stage.as_str()
                );
                assert_ne!(record.input_digest, 0, "brief {index} input real");
                assert_ne!(record.output_digest, 0, "brief {index} output real");
                assert_eq!(record.brief_id, envelope.brief_id());
                assert_eq!(record.seed, fixture.seed);
                let saved = record.encode();
                let loaded = StageRecord::decode(&saved).expect("record survives save/load");
                assert_eq!(*record, loaded, "brief {index} record round-trips");
                assert_eq!(record.digest(), loaded.digest());
            }
            // The per-stage AB2/AB3 records stamp the same bytes as the
            // chain: one format end to end, never a second.
            let character_stage = record_character_stage(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect("AB2 records");
            assert_eq!(
                character_stage.encode(),
                chain
                    .find_stage(BriefStage::Character)
                    .expect("character recorded")
                    .encode(),
                "brief {index} AB2 record matches the chain"
            );
            let replayed = replay_character_stage_offline(
                &character_stage,
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect("AB2 replays providers-off");
            assert_eq!(replayed.encode(), character_stage.encode());
            let stored = replay_stored_character(
                &chain.stages,
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect("stored AB2 replays");
            assert_eq!(stored.encode(), character_stage.encode());
            let encounter_stage =
                record_encounter_stage(fixture.text, fixture.seed).expect("AB3 records");
            assert_eq!(
                encounter_stage.encode(),
                chain
                    .find_stage(BriefStage::Encounter)
                    .expect("encounter recorded")
                    .encode(),
                "brief {index} AB3 record matches the chain"
            );
            let replayed =
                replay_encounter_stage_offline(&encounter_stage, fixture.text, fixture.seed)
                    .expect("AB3 replays providers-off");
            assert_eq!(replayed.encode(), encounter_stage.encode());
            let stored = replay_stored_encounter(&chain.stages, fixture.text, fixture.seed)
                .expect("stored AB3 replays");
            assert_eq!(stored.encode(), encounter_stage.encode());
            // The stage briefs stamp the same LW6 lines the chain holds:
            // the AB2/AB3 write paths carry them, not just the chain.
            let character_brief = CharacterBrief {
                brief_text: fixture.text.to_owned(),
                variant: fixture.variant,
                want_clips: SLICE_CLIPS.iter().map(|slot| (*slot).to_owned()).collect(),
                bloodied: fixture.bloodied,
                license: LicenseRef::ct7(),
                repairs_used: 0,
                seed: fixture.seed,
            };
            assert_eq!(
                character_brief
                    .record_line()
                    .expect("AB2 brief stamps its line"),
                chain
                    .find_stage(BriefStage::Character)
                    .expect("character recorded")
                    .record_line,
                "brief {index} AB2 brief carries its record",
            );
            let encounter_brief =
                EncounterBrief::new(fixture.text, fixture.seed).expect("AB3 brief valid");
            assert_eq!(
                encounter_brief.record_line(),
                chain
                    .find_stage(BriefStage::Encounter)
                    .expect("encounter recorded")
                    .record_line,
                "brief {index} AB3 brief carries its record",
            );
            // The full chain survives save/load and replays byte-identically.
            let saved = chain.encode();
            let loaded = ChainRecord::decode(&saved).expect("chain survives save/load");
            assert_eq!(chain, loaded, "brief {index} chain round-trips");
            let fresh = replay_chain_offline(
                &chain,
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect("chain replays providers-off");
            assert_eq!(fresh.encode(), chain.encode());
            // A missing AB2 record fails loudly instead of calling a model.
            let mut short = chain.clone();
            short
                .stages
                .retain(|record| record.stage != BriefStage::Character);
            let error = replay_stored_character(
                &short.stages,
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect_err("a missing AB2 record must refuse");
            assert_eq!(error.code, "missing_record");
            assert!(
                error.to_string().contains(BriefStage::Character.as_str()),
                "names the stage: {error}"
            );
            // A tampered AB3 output diverges loudly on replay.
            let mut tampered = encounter_stage.clone();
            tampered.output_digest ^= 0x01;
            assert_eq!(
                replay_encounter_stage_offline(&tampered, fixture.text, fixture.seed)
                    .expect_err("tampered AB3 must diverge")
                    .code,
                "replay_mismatch"
            );
            // Corrupt chain bytes refuse loudly, never as a guess.
            assert_eq!(
                ChainRecord::decode(&saved[..saved.len() - 1])
                    .expect_err("truncation must refuse")
                    .code,
                "bad_record"
            );
        }
        // AB1 write/undo proof rides the live session behind the door:
        // preview and apply carry the LW6 line and undo restores the
        // exact digest. Runs only with the door compiled in.
        #[cfg(feature = "mcp")]
        {
            use super::super::build_level::{
                envelope as level_envelope, record_level, LevelSession,
            };
            let text = fixtures[0].text;
            let seed = fixtures[0].seed;
            let env = level_envelope(text, seed).expect("AB1 envelopes");
            let mut session = LevelSession::new();
            let before = session.current_digest();
            let preview = session.preview(text, seed).expect("AB1 previews");
            assert_eq!(preview.record_line, env.record_line());
            assert!(
                preview.record_line.contains("level"),
                "line: {}",
                preview.record_line
            );
            let key = 0xab11_0001u64;
            let applied = match session
                .apply(text, seed, session.revision(), key)
                .expect("AB1 applies")
            {
                super::super::build_level::ApplyOutcome::Placed(applied) => applied,
                super::super::build_level::ApplyOutcome::Refused { refusal, .. } => {
                    panic!("audit brief must place, refused: {}", refusal.reason)
                }
            };
            assert_eq!(applied.record_line, env.record_line());
            assert_ne!(session.current_digest(), before, "the write moved state");
            // The per-stage AB1 record matches the chain stage byte-identically.
            let slice = build_slice(
                fixtures[0].text,
                fixtures[0].seed,
                fixtures[0].variant,
                fixtures[0].bloodied,
            )
            .expect("audit slice packages");
            let chain = record_chain(
                fixtures[0].text,
                fixtures[0].seed,
                fixtures[0].variant,
                fixtures[0].bloodied,
                &slice,
            )
            .expect("audit chain records");
            let level_stage = record_level(text, seed).expect("AB1 records");
            assert_eq!(
                level_stage.encode(),
                chain
                    .find_stage(BriefStage::Level)
                    .expect("level recorded")
                    .encode(),
                "AB1 record matches the chain"
            );
            let undone = session.undo(None).expect("AB1 undoes");
            assert_eq!(undone.revision, 2);
            assert_eq!(session.current_digest(), before, "undo restores the digest");
        }
    }

    #[test]
    fn hook_names_the_door_registration() {
        assert_eq!(tool_name(), TOOL_BUILD_GAME);
        assert_eq!(TOOL_BUILD_GAME, "game.build");
        assert!(readonly());
        assert!(!TOOL_SUMMARY.is_empty());
        assert_eq!(LevelPath::Ab1.as_str(), "ab1");
        assert_eq!(LevelPath::Offline.as_str(), "offline");
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn ab1_adapter_returns_the_real_next_line() {
        use super::super::build_level::{build, envelope, BuildOutcome};
        let fixtures = fixture_briefs();
        for (index, fixture) in fixtures.iter().enumerate() {
            let line = level_line(fixture.text, fixture.seed).expect("AB1 line lands");
            assert_eq!(
                line.path,
                LevelPath::Ab1,
                "brief {index} rides the real chain"
            );
            let env = envelope(fixture.text, fixture.seed).expect("brief envelopes");
            match build(&env).expect("brief builds") {
                BuildOutcome::Placed(level) => {
                    assert_eq!(line.line, level.report.next_ab4, "brief {index} verbatim");
                    assert_eq!(line.level_digest, level.report.level_digest);
                    assert_eq!(line.cell_digest, level.report.cell_digest);
                }
                BuildOutcome::Refused(refusal) => {
                    panic!(
                        "slice fixture {index} must place on AB1: {}",
                        refusal.reason
                    )
                }
            }
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn five_visit_briefs_boot_play_two_minutes_and_replay_twice_offline() {
        let briefs = visit_briefs();
        assert_eq!(briefs.len(), VISIT_COUNT);
        for (index, fixture) in briefs.iter().enumerate() {
            let visit = build_visit_slice(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .unwrap_or_else(|error| panic!("visit {index} must land: {error}"));
            assert!(visit.boot_ok, "visit {index} boots");
            assert_eq!(
                visit.ticks_played, VISIT_TICKS,
                "visit {index} plays two minutes"
            );
            assert_eq!(visit.ticks_played, 7_200, "visit {index} is two minutes");
            assert_eq!(
                visit.play_digest, visit.replay_digest,
                "visit {index} replays once providers-off"
            );
            assert_eq!(
                visit.play_digest, visit.replay_digest_again,
                "visit {index} replays twice providers-off"
            );
            assert!(visit.passed(), "visit {index} eval passes");
            assert_eq!(visit.verdict, "pass");
            assert_eq!(
                visit.visit_lines.len(),
                3,
                "visit {index} voices all three roles"
            );
            assert_eq!(
                visit.visit_scripted,
                visit.visit_lines.len(),
                "visit {index} is fully scripted with providers off"
            );
            assert_ne!(
                visit.visit_checkpoint, 0,
                "visit {index} checkpoint is real"
            );
            assert!(
                visit.rebuild_ok,
                "visit {index} rebuilds from seed plus bout"
            );
            // Same brief twice is the same visit: the chain is deterministic.
            let again = build_visit_slice(
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect("rebuild lands");
            assert_eq!(visit, again, "visit {index} rebuilds identically");
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn visit_eval_line_names_boot_play_both_replays_and_the_visit() {
        let visit = build_visit_fixture(0).expect("visit fixture 0 lands");
        // The AB4 door mirror and the direct WD6 call agree on the lobby shape.
        let params = LobbyBootParams::for_slice(visit.seed);
        assert_eq!(params.fighters, SLICE_FIGHTERS);
        assert_eq!(params.half_extent_m, 20.0);
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn visit_writes_replay_cleanly_and_undo_to_a_drop() {
        use engine_game::ludus_npcs::{
            ludus_contains, ludus_zone, LudusNpcs, LUDUS_MAP_NAME, LUDUS_SPAWNS,
        };
        let visit = build_visit_fixture(1).expect("visit fixture 1 lands");
        assert!(
            visit.rebuild_ok,
            "a fresh rebuild from seed plus bout reproduces the visit"
        );
        // Every voiced line staffs its WD2 station in role order.
        for (line, station) in
            visit
                .visit_lines
                .iter()
                .zip(["practice_yard", "blacksmith", "party_board"])
        {
            assert!(line.contains(station), "stationed line: {line}");
            assert!(
                ludus_zone(station).is_some(),
                "station `{station}` resolves in WD2 data"
            );
        }
        let yard = ludus_zone("practice_yard").expect("yard resolves");
        for spawn in LUDUS_SPAWNS.iter() {
            assert!(
                ludus_contains(yard, spawn.x, spawn.z),
                "spawn {} stands on the yard",
                spawn.name
            );
        }
        assert_eq!(LUDUS_MAP_NAME, "ludus");
        assert!(
            ludus_zone("no-such-zone").is_none(),
            "unknown zones read None, never a guess"
        );
        // Dropping the owned ludus undoes everything: nothing global moved.
        let fresh = LudusNpcs::new(visit.seed);
        assert_ne!(
            fresh.checkpoint(),
            visit.visit_checkpoint,
            "the visit wrote, and only the owned value holds it"
        );
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn visit_refuses_bad_inputs_with_legal_values() {
        use engine_game::modes_world::{LobbyBoot, ModeSession, WorldMode};
        let briefs = visit_briefs();
        let error = build_visit_slice("", briefs[0].seed, briefs[0].variant, false)
            .expect_err("empty refuses");
        assert_eq!(error.code, "empty_brief");
        let error = build_visit_fixture(VISIT_COUNT).expect_err("visit fixture 5 is unknown");
        assert_eq!(error.code, "bad_brief");
        assert!(error.to_string().contains("0..=4"), "legal: {error}");
        let bad = LobbyBoot::new(WorldMode::Arena, 1, 0, SLICE_FIGHTERS, 20.0);
        assert!(
            ModeSession::boot(bad).is_err(),
            "zero ticks must refuse the lobby"
        );
        let bad = LobbyBoot::new(WorldMode::Arena, 1, VISIT_TICKS, 1, 20.0);
        assert!(
            ModeSession::boot(bad).is_err(),
            "one fighter must refuse the lobby"
        );
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn visit_diverges_between_seeds_and_matches_the_ab4_package() {
        let first = build_visit_fixture(2).expect("visit fixture 2 lands");
        let second = build_visit_fixture(3).expect("visit fixture 3 lands");
        assert_ne!(first.play_digest, second.play_digest, "seeds diverge");
        assert_ne!(
            first.visit_checkpoint, second.visit_checkpoint,
            "visits diverge"
        );
        // The same brief still rides the AB4 package: boot verifies the id.
        let slice = build_fixture(2).expect("AB4 fixture 2 packages");
        let (brief_id, _) = PlayableSlice::boot_package(&slice.package).expect("AB4 package boots");
        assert_eq!(
            brief_id, first.brief_id,
            "visit and package share the AB7 envelope id"
        );
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;

    #[test]
    fn ten_recorded_briefs_replay_byte_identically_twice_offline() {
        for index in 0..10 {
            let (live, first, second) =
                replay_fixture_offline(index).expect("recorded chain replays offline");
            // Byte-identical records twice: the full chain replays providers-off.
            assert_eq!(
                live.encode(),
                first.encode(),
                "brief {index} replays byte-identically once"
            );
            assert_eq!(
                live.encode(),
                second.encode(),
                "brief {index} replays byte-identically twice"
            );
            assert_eq!(live.digest(), first.digest(), "brief {index} digest");
            assert_eq!(live.digest(), second.digest(), "brief {index} digest x2");
            // Live and offline digests match on every stage plus the play proof.
            for stage in REPLAY_STAGES {
                let live_stage = live.find_stage(stage).expect("stage recorded");
                let first_stage = first.find_stage(stage).expect("stage replayed");
                assert_eq!(
                    live_stage.output_digest,
                    first_stage.output_digest,
                    "brief {index} stage `{}` live and offline digests match",
                    stage.as_str()
                );
                assert_eq!(
                    live_stage.record_line,
                    first_stage.record_line,
                    "brief {index} stage `{}` stamps one record line",
                    stage.as_str()
                );
                assert_ne!(live_stage.output_digest, 0, "brief {index} output real");
                assert_ne!(live_stage.input_digest, 0, "brief {index} input real");
            }
            assert_eq!(live.play_digest, first.play_digest, "brief {index} play");
            assert_eq!(
                live.play_digest, second.play_digest,
                "brief {index} play x2"
            );
            assert_eq!(
                live.checkpoint_digest, first.checkpoint_digest,
                "brief {index} tips"
            );
            assert_eq!(
                live.package_digest, first.package_digest,
                "brief {index} package"
            );
            assert_ne!(live.play_digest, 0, "brief {index} play digest real");
            // The replayed chain still decodes to itself: the bytes are canonical.
            let decoded = ChainRecord::decode(&live.encode()).expect("chain bytes decode");
            assert_eq!(live, decoded, "brief {index} round-trips");
            let summary = live.summary();
            assert!(summary.contains("replay ok"), "receipt: {summary}");
            assert!(summary.contains("4 stages"), "receipt: {summary}");
            assert!(
                summary.contains(&format!("{:016x}", live.play_digest)),
                "receipt: {summary}"
            );
        }
    }

    #[test]
    fn replay_round_trips_single_records_and_rejects_reorders() {
        let (live, _, _) = replay_fixture_offline(0).expect("fixture 0 records");
        for stage in REPLAY_STAGES {
            let record = live.find_stage(stage).expect("stage recorded");
            let decoded = StageRecord::decode(&record.encode()).expect("record decodes");
            assert_eq!(*record, decoded, "stage `{}` round-trips", stage.as_str());
            assert_eq!(record.digest(), decoded.digest());
        }
        // A reordered chain is a loud `bad_record`, never a silent accept.
        let mut swapped = live.clone();
        swapped.stages.swap(0, 1);
        let error = ChainRecord::decode(&swapped.encode()).expect_err("reorder must refuse");
        assert_eq!(error.code, "bad_record");
    }

    #[test]
    fn missing_record_fails_loudly_instead_of_calling_a_model() {
        let (live, _, _) = replay_fixture_offline(1).expect("fixture 1 records");
        let fixtures = fixture_briefs();
        let fixture = &fixtures[1];
        // Dropping any one stage fails before any rebuild, naming the stage.
        for stage in REPLAY_STAGES {
            let mut short = live.clone();
            short.stages.retain(|record| record.stage != stage);
            let error = replay_chain_offline(
                &short,
                fixture.text,
                fixture.seed,
                fixture.variant,
                fixture.bloodied,
            )
            .expect_err("a missing stage record must refuse");
            assert_eq!(error.code, "missing_record", "stage `{}`", stage.as_str());
            assert!(
                error.to_string().contains(stage.as_str()),
                "names the stage: {error}"
            );
            assert!(
                error.to_string().contains("instead of calling a model"),
                "never falls back to a model: {error}"
            );
        }
        // An empty record fails the same way, on the first canonical stage.
        let empty = ChainRecord {
            stages: Vec::new(),
            ..live.clone()
        };
        let error = empty
            .verify_complete()
            .expect_err("empty chain must refuse");
        assert_eq!(error.code, "missing_record");
        assert!(
            error.to_string().contains("level"),
            "first gap named: {error}"
        );
        let missing = live
            .find_stage(BriefStage::Replay)
            .expect_err("only chain stages are recorded");
        assert_eq!(missing.code, "missing_record");
    }

    #[test]
    fn corrupt_record_bytes_refuse_with_bad_record() {
        let (live, _, _) = replay_fixture_offline(2).expect("fixture 2 records");
        let bytes = live.encode();
        // Truncation at several cut points refuses loudly.
        for at in [0usize, 1, 5, bytes.len() / 2, bytes.len() - 1] {
            let error = ChainRecord::decode(&bytes[..at]).expect_err("cut must refuse");
            assert_eq!(error.code, "bad_record", "cut at {at}");
        }
        // One flipped id byte still decodes structurally, but the chain no
        // longer replays: the re-recorded bytes diverge loudly.
        let mut tampered = bytes.clone();
        tampered[1] ^= 0x01;
        let decoded_tampered = ChainRecord::decode(&tampered).expect("id flip stays structural");
        assert_ne!(decoded_tampered.brief_id, live.brief_id);
        let fixtures_again = fixture_briefs();
        let fixture_again = &fixtures_again[2];
        assert_eq!(
            replay_chain_offline(
                &decoded_tampered,
                fixture_again.text,
                fixture_again.seed,
                fixture_again.variant,
                fixture_again.bloodied,
            )
            .expect_err("flipped id must diverge")
            .code,
            "replay_mismatch"
        );
        // Trailing bytes past the play proof refuse.
        let mut trailed = bytes.clone();
        trailed.push(0x00);
        assert_eq!(
            ChainRecord::decode(&trailed)
                .expect_err("trailing byte must refuse")
                .code,
            "bad_record"
        );
        // A bad version refuses before any field is read.
        let mut versioned = bytes.clone();
        versioned[0] = 0x7f;
        assert_eq!(
            ChainRecord::decode(&versioned)
                .expect_err("bad version must refuse")
                .code,
            "bad_record"
        );
        // A bare stage record with trailing bytes refuses too.
        let record = live.find_stage(BriefStage::Level).expect("level recorded");
        let mut trailed_stage = record.encode();
        trailed_stage.push(0x00);
        assert_eq!(
            StageRecord::decode(&trailed_stage)
                .expect_err("trailed stage must refuse")
                .code,
            "bad_record"
        );
        assert!(
            StageRecord::decode(&[])
                .expect_err("empty must refuse")
                .code
                == "bad_record"
        );
    }

    #[test]
    fn replay_detects_tampered_outputs_and_inputs() {
        let (live, _, _) = replay_fixture_offline(3).expect("fixture 3 records");
        let fixtures = fixture_briefs();
        let fixture = &fixtures[3];
        // A tampered output digest rebuilds, then diverges loudly.
        let mut tampered = live.clone();
        tampered.stages[0].output_digest ^= 0x01;
        let error = replay_chain_offline(
            &tampered,
            fixture.text,
            fixture.seed,
            fixture.variant,
            fixture.bloodied,
        )
        .expect_err("tampered output must diverge");
        assert_eq!(error.code, "replay_mismatch");
        // A swapped brief text rebuilds a different chain, then diverges loudly.
        let other = &fixture_briefs()[4];
        let error = replay_chain_offline(
            &live,
            other.text,
            fixture.seed,
            fixture.variant,
            fixture.bloodied,
        )
        .expect_err("swapped text must diverge");
        assert_eq!(error.code, "replay_mismatch");
        // An unknown fixture index names the legal range.
        let error = replay_fixture_offline(10).expect_err("fixture 10 is unknown");
        assert_eq!(error.code, "bad_brief");
        assert!(error.to_string().contains("0..=9"), "legal: {error}");
    }
}
