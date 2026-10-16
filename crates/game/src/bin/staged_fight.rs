//! staged_fight: headless staged fight — hero + 3 opponents.
//!
//! Consumer: the operator or CI runs `staged_fight` after the content and window ticks; the
//! receipt plus PNG captures under `target/ct15-staged-fight` are the evidence
//! the board row reads. Owner: this binary owns the script, the headless run
//! and the receipt; [`fighter_bind`] owns manifest parsing plus body checks
//! and `engine-arena` (through [`App`]) owns match authority. Units: fixed
//! ticks at 60 Hz, pixels, bytes; SHAs as lowercase hex. Determinism: the same
//! manifest plus seed plus script gives the same fight, the same PNGs and the
//! same receipt (no wall clock, no RNG, fixed JSON field order). Errors: exit
//! 2 for bad flags/config, exit 1 for bind/fight/capture failures; every error
//! names its culprit on stderr, never panics.
//!
//! Frames: the PNGs are the staged fight raster from `windowed` (sand plus one
//! body per fighter, hits flashing), not the synth noise frames of
//! `capture::run_headless_capture`. That raster sits in the `app` leg, so this
//! binary needs `--features app` (see `required-features` in Cargo.toml).
//!
//! Budget: one 600-tick headless session plus the 600-tick frame run and four
//! 128x96 CPU frames (seconds on the shared build pool); all IO lands after
//! the fight, never in the tick.

use std::path::{Path, PathBuf};

use engine_audio::FightCue;
use engine_game::bench::{BenchConfig, Policy};
use engine_game::capture::fnv_hex;
use engine_game::fighter_bind::{self, FighterBinding, VARIANTS};
use engine_game::windowed::{run_staged_fight_frames, StagedFightFrame};
use engine_game::{App, GameConfig, HUMAN_SLOT, MAX_TICKS};
use engine_input::{
    fighter::{ATTACK, BLOCK},
    Buttons, FighterId, FighterInput,
};

/// Session seed for the staged fight (matches the headless default).
const SEED: u64 = 2040;
/// Full fight length in fixed ticks: the staged fight is always this long.
const DEFAULT_TICKS: u32 = 600;
/// Arena half-extent in meters (the duel default, fits 4 fighters).
const HALF_EXTENT_M: f32 = 20.0;
/// Golden simulation hash the receipt is filed under.
const GOLDEN: &str = "d57b5c8ae3096614";
/// Default receipt root (relative to the lane checkout at runtime).
const DEFAULT_OUT_DIR: &str = "target/ct15-staged-fight";
/// Capture subdirectory under the receipt root.
const CAPTURES_DIR: &str = "captures";
/// Frame manifest name beside the frame PNGs (a JSON array, one row per frame).
const FRAMES_MANIFEST_NAME: &str = "manifest.json";
/// Receipt file name beside the captures.
const RECEIPT_NAME: &str = "receipt.json";
/// Renderer key filed in staged captures plus receipt: the CPU raster that
/// drew these frames, not the twin or GPU (honest evidence, CODELAW 10).
const STAGED_RENDERER: &str = "staged-cpu";

/// Scripted human input: circle-strafe base with timed blocks plus a
/// riposte-window attack, so the bout shows blocks and fires Parried cues
/// instead of idling. The spawn spread parks the duel out of blade reach
/// (the human starts by the east wall, the pack brawls mid-arena), so the
/// script treks west to find the pack and holds fresh blocks 4 of every 5
/// ticks: each re-press resets the parry age inside the 4-tick parry
/// window. Cycle 50: regular attack at 0, bait blocks through the middle,
/// then riposte attack at 35 inside the parry stagger. Pure.
fn script(tick: u32, human: FighterId) -> FighterInput {
    let weave = match (tick / 10) % 4 {
        1 => 60,
        3 => -60,
        _ => 0,
    };
    let cycle = tick % 50;
    let buttons = if cycle == 0 || cycle == 35 {
        Buttons(ATTACK)
    } else if tick % 5 == 4 {
        Buttons::NONE
    } else {
        Buttons(BLOCK)
    };
    FighterInput {
        tick,
        fighter: human,
        mx: -60,
        mz: weave,
        facing: 0,
        buttons,
    }
}

/// Fight totals folded from the tick outcomes (presentation of sim state).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct FightStats {
    cues: usize,
    hits: usize,
    knockdowns: usize,
    blocked: usize,
    parried: usize,
}

fn fold_cues(stats: &mut FightStats, cues: &[FightCue]) {
    stats.cues += cues.len();
    for cue in cues {
        match cue {
            FightCue::Hit { .. } => stats.hits += 1,
            FightCue::Knockdown { .. } => stats.knockdowns += 1,
            FightCue::Blocked { .. } => stats.blocked += 1,
            FightCue::Parried { .. } => stats.parried += 1,
            FightCue::Dodged { .. } | FightCue::StaminaOut { .. } => {}
        }
    }
}

/// Bare flag that runs the offline PQ2 balance screen and exits before any
/// staged fight ticks. Never enters the fixed tick; fork-based only.
const BALANCE_SCREEN_FLAG: &str = "--balance-screen";

/// Parsed CLI: content dir, receipt root and fight length.
struct Args {
    content_dir: PathBuf,
    out_dir: PathBuf,
    ticks: u32,
    balance_screen: bool,
}

/// Resolve the fighters content dir: explicit flag first, then
/// `content/fighters` under the current directory (lane root under
/// `cargo run`), then the compile-time lane path.
fn resolve_content_dir(flag: Option<&str>) -> PathBuf {
    if let Some(dir) = flag {
        return PathBuf::from(dir);
    }
    let cwd = PathBuf::from("content/fighters");
    if cwd.join(fighter_bind::MANIFEST_NAME).is_file() {
        return cwd;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
}

fn take_value(raw: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    match raw.get(*index) {
        Some(value) if !value.starts_with("--") => Ok(value.clone()),
        _ => Err(format!("{flag} needs a value")),
    }
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut content: Option<String> = None;
    let mut out: Option<String> = None;
    let mut ticks: Option<String> = None;
    let mut balance_screen = false;
    let mut index = 1usize;
    while index < raw.len() {
        match raw[index].as_str() {
            "--content-dir" => content = Some(take_value(raw, &mut index, "--content-dir")?),
            "--out-dir" => out = Some(take_value(raw, &mut index, "--out-dir")?),
            "--ticks" => ticks = Some(take_value(raw, &mut index, "--ticks")?),
            flag if flag == BALANCE_SCREEN_FLAG => balance_screen = true,
            other => return Err(format!("unknown flag {other}")),
        }
        index += 1;
    }
    let ticks = match ticks {
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| format!("--ticks needs 1..={MAX_TICKS}, got {raw}"))?,
        None => DEFAULT_TICKS,
    };
    if ticks != DEFAULT_TICKS {
        return Err(format!(
            "--ticks needs {DEFAULT_TICKS} (the staged fight; frames cover all of it), got {ticks}"
        ));
    }
    Ok(Args {
        content_dir: resolve_content_dir(content.as_deref()),
        out_dir: out
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_OUT_DIR)),
        ticks,
        balance_screen,
    })
}

/// Fight result filed in the receipt: folded cue totals, final hp per bound
/// fighter, the last checkpoint hash and the capture manifest rows. Bundled
/// so the receipt constructor stays under the argument-count lint.
struct FightReport {
    stats: FightStats,
    hp: Vec<u16>,
    final_hash: u64,
    capture_frames: u32,
    capture_manifest: String,
    sequence_fnv: String,
}

/// Receipt JSON: fixed field order, file names only (no absolute paths), so
/// the same fight gives byte-identical receipts twice.
fn receipt_json(bindings: &[FighterBinding], config: GameConfig, report: &FightReport) -> String {
    let mut out = String::from("{\"schema\":1,\"task\":\"CT15\"");
    out.push_str(&format!(
        ",\"seed\":{},\"ticks\":{},\"fighters\":{}",
        config.seed, config.ticks, config.fighters
    ));
    out.push_str(",\"fighters_bound\":[");
    for (index, binding) in bindings.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"variant\":\"{}\",\"bundle_sha256\":\"{}\",\"body_glb_sha256\":\"{}\",\"body\":\"{}\",\"joints\":{},\"tris_lod0\":{},\"clips\":{}}}",
            binding.variant,
            binding.bundle_sha256,
            binding.body_glb_sha256,
            body_file_name(&binding.variant),
            binding.joints,
            binding.tris_lod0,
            binding.clips
        ));
    }
    out.push_str(&format!(
        "],\"ticks_run\":{},\"cues\":{},\"hits\":{},\"knockdowns\":{},\"blocked\":{},\"parried\":{}",
        config.ticks,
        report.stats.cues,
        report.stats.hits,
        report.stats.knockdowns,
        report.stats.blocked,
        report.stats.parried
    ));
    out.push_str(",\"final_hp\":[");
    for (index, hp) in report.hp.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&hp.to_string());
    }
    out.push_str(&format!("],\"final_hash\":\"{:016x}\"", report.final_hash));
    out.push_str(&format!(
        ",\"captures\":{{\"dir\":\"{CAPTURES_DIR}\",\"frames\":{},\"manifest\":\"{}\",\"sequence_fnv\":\"{}\",\"renderer\":\"{STAGED_RENDERER}\"}}",
        report.capture_frames, report.capture_manifest, report.sequence_fnv
    ));
    out.push_str(&format!(",\"golden\":\"{GOLDEN}\"}}"));
    out
}

/// Body file name without touching the checked path (receipts file names
/// only, so two machines write the same bytes).
fn body_file_name(variant: &str) -> String {
    fighter_bind::body_file_name(variant)
}

fn run(args: &Args) -> Result<i32, String> {
    let bindings =
        fighter_bind::bind_all(&args.content_dir).map_err(|err| format!("bind: {err}"))?;
    for binding in &bindings {
        println!(
            "staged_fight bind: variant={} bundle_sha256={} body_sha256={} body={} joints={} tris_lod0={} clips={}",
            binding.variant,
            binding.bundle_sha256,
            binding.body_glb_sha256,
            body_file_name(&binding.variant),
            binding.joints,
            binding.tris_lod0,
            binding.clips
        );
    }
    let config = GameConfig {
        seed: SEED,
        ticks: args.ticks,
        fighters: VARIANTS.len() as u8,
        half_extent_m: HALF_EXTENT_M,
    };
    config.validate().map_err(|err| format!("config: {err}"))?;
    let human = FighterId::new(HUMAN_SLOT).ok_or_else(|| "config: no human slot".to_string())?;
    let mut app = App::new(config).map_err(|err| format!("session: {err}"))?;
    let mut stats = FightStats::default();
    for tick in 0..args.ticks {
        let outcome = app.tick(script(tick, human));
        if !outcome.input_accepted {
            return Err(format!("fight: tick {tick} rejected"));
        }
        fold_cues(&mut stats, &outcome.cues);
    }
    if app.cursor() != args.ticks || app.checkpoints().len() != args.ticks as usize {
        return Err(format!("fight: ended at {}/{}", app.cursor(), args.ticks));
    }
    let final_hash = app
        .checkpoints()
        .last()
        .map(|point| point.hash)
        .unwrap_or(0);
    let hp: Vec<u16> = app.hp()[..VARIANTS.len()].to_vec();
    // Crowd wire: export the reacting room's 64 levels so the built
    // `export_levels` has a live caller outside its own tests. Presentation
    // only, hash untouched; a short buffer fails loudly.
    let mut crowd_levels = [0u16; engine_game::crowd_gpu::CROWD_BLOCKS];
    let crowd_exported = app
        .crowd_levels(&mut crowd_levels)
        .map_err(|err| format!("crowd: {err}"))?;
    if crowd_exported != engine_game::crowd_gpu::CROWD_BLOCKS {
        return Err(format!(
            "crowd: exported {crowd_exported}, want {}",
            engine_game::crowd_gpu::CROWD_BLOCKS
        ));
    }

    // The frames come from the staged fight raster (bodies drawn, hits
    // flashing). It runs the same script through the arena authority, so its
    // cue totals must match the App run before any frame is filed.
    let fight = run_staged_fight_frames().map_err(|err| format!("captures: {err}"))?;
    if fight.hits != stats.hits || fight.knockdowns != stats.knockdowns {
        return Err(format!(
            "captures: frame run drifted (hits {}/{} knockdowns {}/{})",
            fight.hits, stats.hits, fight.knockdowns, stats.knockdowns
        ));
    }
    let captures = args.out_dir.join(CAPTURES_DIR);
    std::fs::create_dir_all(&captures).map_err(|err| format!("captures: {}", err.kind()))?;
    let (frames, manifest_name, sequence_fnv) = write_frames(&captures, &fight.frames)?;
    if frames == 0 {
        return Err("captures: zero frames written".to_string());
    }
    let receipt = receipt_json(
        &bindings,
        config,
        &FightReport {
            stats,
            hp,
            final_hash,
            capture_frames: frames,
            capture_manifest: manifest_name,
            sequence_fnv,
        },
    );
    std::fs::write(args.out_dir.join(RECEIPT_NAME), receipt.as_bytes())
        .map_err(|err| format!("receipt: {}", err.kind()))?;
    println!(
        "staged_fight done: ticks={} cues={} hits={} knockdowns={} blocked={} parried={} Parried={} final_hash={final_hash:016x} captures={} manifest=\"{}\" receipt=\"{}\"",
        args.ticks,
        stats.cues,
        stats.hits,
        stats.knockdowns,
        stats.blocked,
        stats.parried,
        stats.parried,
        frames,
        captures.join(FRAMES_MANIFEST_NAME).display(),
        args.out_dir.join(RECEIPT_NAME).display()
    );
    Ok(0)
}

/// Stamp one staged frame manifest with the renderer key: insert
/// `"renderer":"staged-cpu"` after `{"schema":1`, so each filed row names
/// its drawer. Torn rows without the prefix pass through unstamped, never
/// corrupted. Pure, IO-free (filing path only, never the tick).
fn with_staged_renderer(manifest: &str) -> String {
    const PREFIX: &str = "{\"schema\":1";
    match manifest.strip_prefix(PREFIX) {
        Some(rest) => format!("{PREFIX},\"renderer\":\"{STAGED_RENDERER}\"{rest}"),
        None => manifest.to_string(),
    }
}

/// Write the staged frames as `frame_NNNNNN.png` plus one manifest JSON array
/// (each row is that frame's own manifest, stamped with the renderer key). Returns the frame count, the
/// manifest name and the FNV-1a identity over the PNG bytes in frame order.
fn write_frames(dir: &Path, frames: &[StagedFightFrame]) -> Result<(u32, String, String), String> {
    let mut manifest = String::from("[");
    let mut sequence: Vec<u8> = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let file = format!("frame_{index:06}.png");
        std::fs::write(dir.join(&file), &frame.png)
            .map_err(|err| format!("captures: {}", err.kind()))?;
        if index > 0 {
            manifest.push(',');
        }
        manifest.push_str(&with_staged_renderer(&frame.manifest));
        sequence.extend_from_slice(&frame.png);
    }
    manifest.push(']');
    std::fs::write(dir.join(FRAMES_MANIFEST_NAME), manifest.as_bytes())
        .map_err(|err| format!("captures: {}", err.kind()))?;
    let count = u32::try_from(frames.len()).map_err(|_| "captures: too many frames".to_string())?;
    Ok((count, FRAMES_MANIFEST_NAME.to_string(), fnv_hex(&sequence)))
}

/// Offline PQ2 balance screen: the bloodless anchor must hold the per-move
/// bar while the poke lever (every kill on the strike byte) must trip it.
/// Fork-based, integer-only, keyboard-out; never enters the fixed tick, so
/// the golden hash cannot move. Reuses the live 350 bar, never a second gate.
fn check_balance_pair(anchor_share: u16, lever_share: u16) -> Result<(), String> {
    use engine_game::bench::{check_pq2_kill_share, PQ2_KILL_SHARE_MAX_PERMILLE};
    if check_pq2_kill_share(anchor_share).is_err() {
        return Err(format!(
            "balance: bloodless anchor tripped bar {anchor_share} over {PQ2_KILL_SHARE_MAX_PERMILLE}"
        ));
    }
    if check_pq2_kill_share(lever_share).is_ok() {
        return Err(format!(
            "balance: lopsided lever held bar {lever_share} at {PQ2_KILL_SHARE_MAX_PERMILLE}"
        ));
    }
    Ok(())
}

/// Agreement gate over the 5% contract bound: anchor (bloodless) and lever (poke kills)
/// must disagree, proving `BenchReport::agrees_with` plus `divergence_permille` live.
/// Returns the distance; refuses an agreeing pair loudly. Pure, offline, never the tick.
/// Caller: `run_balance_screen` behind `BALANCE_SCREEN_FLAG`.
fn agreement_gate(
    anchor: &engine_game::bench::BenchReport,
    lever: &engine_game::bench::BenchReport,
) -> Result<u16, String> {
    let divergence = anchor.divergence_permille(lever);
    if anchor.agrees_with(lever) {
        return Err(format!(
            "balance: anchor agrees with lever (divergence {divergence})"
        ));
    }
    Ok(divergence)
}

/// Run the offline screen: anchor plus lever through `bench::run`, per-move
/// pin plus the 350 bar exercised outside tests. Same configs twice give the
/// same line; anchor drift or a vacuous lever fails loudly.
fn run_balance_screen() -> Result<String, String> {
    use engine_game::bench::{
        check_pq2_kill_share, kill_share_permille, move_kill_share_permille,
        PQ2_KILL_SHARE_MAX_PERMILLE,
    };
    let anchor_cfg = BenchConfig::standard(Policy::Idle, Policy::Idle);
    let lever_cfg = BenchConfig::standard(Policy::Poke, Policy::Idle);
    let anchor =
        engine_game::bench::run(&anchor_cfg).map_err(|err| format!("balance anchor: {err}"))?;
    let lever =
        engine_game::bench::run(&lever_cfg).map_err(|err| format!("balance lever: {err}"))?;
    let anchor_share = kill_share_permille(&anchor);
    let lever_share = kill_share_permille(&lever);
    let lever_move = move_kill_share_permille(&lever, lever.strike_byte);
    if lever_move != lever_share {
        return Err(format!(
            "balance: per-move share drifted (move {lever_move} vs kill {lever_share})"
        ));
    }
    for byte in 0..=7u8 {
        if byte != lever.strike_byte && move_kill_share_permille(&lever, byte) != 0 {
            return Err(format!(
                "balance: byte {byte} owns kills outside the strike"
            ));
        }
    }
    check_balance_pair(anchor_share, lever_share)?;
    let anchor_ok = check_pq2_kill_share(anchor_share).is_ok();
    let lever_trips = check_pq2_kill_share(lever_share).is_err();
    if !anchor_ok || !lever_trips {
        return Err("balance: bar wiring drifted".to_string());
    }
    let divergence = agreement_gate(&anchor, &lever)?;
    Ok(format!(
        "balance-screen anchor_share={} lever_share={} move_byte={} bar={} anchor=PASS lever=TRIP kills_a={}/{} divergence={} agree=false",
        anchor_share,
        lever_share,
        lever.strike_byte,
        PQ2_KILL_SHARE_MAX_PERMILLE,
        lever.kills_a,
        lever.matches,
        divergence,
    ))
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("staged_fight: {err}");
            std::process::exit(2);
        }
    };
    if args.balance_screen {
        match run_balance_screen() {
            Ok(line) => {
                println!("{line}");
                std::process::exit(0);
            }
            Err(err) => {
                eprintln!("staged_fight: {err}");
                std::process::exit(1);
            }
        }
    }
    match run(&args) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("staged_fight: {err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four_fighter_config(ticks: u32) -> GameConfig {
        GameConfig {
            seed: SEED,
            ticks,
            fighters: 4,
            half_extent_m: HALF_EXTENT_M,
        }
    }

    #[test]
    fn staged_script_completes_short_four_fighter_fight() {
        let config = four_fighter_config(64);
        let human = FighterId::new(HUMAN_SLOT).expect("slot 0 is a fighter");
        let mut app = App::new(config).expect("4-fighter session builds");
        let mut stats = FightStats::default();
        for tick in 0..config.ticks {
            let outcome = app.tick(script(tick, human));
            assert!(outcome.input_accepted, "tick {tick} rejected");
            fold_cues(&mut stats, &outcome.cues);
        }
        assert_eq!(app.cursor(), 64);
        assert_eq!(app.checkpoints().len(), 64);
        assert!(
            stats.cues > 0,
            "script must land cues or the bind is vacuous"
        );
    }

    #[test]
    fn staged_fight_rejects_solo_config() {
        let solo = GameConfig {
            fighters: 1,
            ..four_fighter_config(64)
        };
        assert!(App::new(solo).is_err(), "one fighter is no fight");
        assert!(parse_args(&[
            "staged_fight".to_string(),
            "--ticks".to_string(),
            "0".to_string()
        ])
        .is_err());
        assert!(parse_args(&["staged_fight".to_string(), "--nope".to_string()]).is_err());
        // The bundle SHAs the receipt cites are the manifest bytes.
        assert_eq!(VARIANTS.len(), fighter_bind::EXPECTED_BUNDLE_SHA256.len());
        assert_eq!(VARIANTS.len(), fighter_bind::EXPECTED_BODY_SHA256.len());
    }

    /// Run the full staged 600-tick fight once: folded cues, final hp per
    /// bound fighter and the last checkpoint hash. Same script as the
    /// receipt the binary files.
    fn run_full_fight() -> (FightStats, Vec<u16>, u64) {
        let config = four_fighter_config(DEFAULT_TICKS);
        let human = FighterId::new(HUMAN_SLOT).expect("slot 0 is a fighter");
        let mut app = App::new(config).expect("4-fighter session builds");
        let mut stats = FightStats::default();
        for tick in 0..config.ticks {
            let outcome = app.tick(script(tick, human));
            assert!(outcome.input_accepted, "tick {tick} rejected");
            fold_cues(&mut stats, &outcome.cues);
        }
        assert_eq!(app.cursor(), config.ticks);
        assert_eq!(app.checkpoints().len(), config.ticks as usize);
        let hash = app
            .checkpoints()
            .last()
            .map(|point| point.hash)
            .unwrap_or(0);
        (stats, app.hp()[..VARIANTS.len()].to_vec(), hash)
    }

    fn fighters_content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    fn win6a_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("e2040-win6a-{name}-{}", std::process::id()))
    }

    /// Run the staged frame pass once into a fresh temp dir exactly as the
    /// binary files it: frame count, sequence hash and the manifest bytes
    /// filed beside the PNGs.
    fn run_frame_capture(name: &str) -> (u32, String, Vec<u8>) {
        let fight = run_staged_fight_frames().expect("staged frames run");
        let dir = win6a_temp_dir(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir opens");
        let (frames, manifest, sequence) = write_frames(&dir, &fight.frames).expect("frames write");
        let bytes = std::fs::read(dir.join(&manifest)).expect("manifest filed");
        let _ = std::fs::remove_dir_all(&dir);
        (frames, sequence, bytes)
    }

    /// Collect every `"key":"hex"` value filed in a manifest (frame rows).
    fn manifest_hexes(manifest: &[u8], key: &str) -> Vec<String> {
        let text = std::str::from_utf8(manifest).expect("manifest is UTF-8");
        let marker = format!("\"{key}\":\"");
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find(marker.as_str()) {
            rest = &rest[at + marker.len()..];
            if let Some(end) = rest.find('"') {
                out.push(rest[..end].to_string());
                rest = &rest[end + 1..];
            } else {
                break;
            }
        }
        out
    }

    #[test]
    fn staged_600_tick_receipt_replays_byte_identical() {
        // The staged 600-tick fight pins 27 hits and 3 knockdowns, replayed
        // twice byte-identical with the pinned bundle SHAs in the receipt.
        let (first_stats, first_hp, first_hash) = run_full_fight();
        let (second_stats, second_hp, second_hash) = run_full_fight();
        assert_eq!(first_stats.cues, 30, "script drifted: {first_stats:?}");
        assert_eq!(first_stats.hits, 27, "script drifted: {first_stats:?}");
        assert_eq!(first_stats.knockdowns, 3, "script drifted: {first_stats:?}");
        assert_eq!(first_stats, second_stats, "fight is not deterministic");
        assert_eq!(first_hp, second_hp, "hp is not deterministic");
        assert_eq!(
            first_hp,
            vec![1000, 0, 0, 0],
            "final hp drifted: {first_hp:?}"
        );
        assert_eq!(first_hash, second_hash, "final hash is not deterministic");
        // Fighter bodies bind from the manifest before any receipt is filed.
        let bindings = fighter_bind::bind_all(&fighters_content_dir()).expect("CT7 manifest binds");
        assert_eq!(bindings.len(), VARIANTS.len());
        // The receipt files the same bytes twice over the same captures.
        let (first_frames, first_sequence, _) = run_frame_capture("receipt-a");
        let (second_frames, second_sequence, _) = run_frame_capture("receipt-b");
        assert_eq!(first_frames, 4, "staged captures drifted");
        assert_eq!(
            first_sequence, second_sequence,
            "captures are not deterministic"
        );
        let config = four_fighter_config(DEFAULT_TICKS);
        let receipt_for = |frames: u32, sequence: &str| {
            receipt_json(
                &bindings,
                config,
                &FightReport {
                    stats: first_stats,
                    hp: first_hp.clone(),
                    final_hash: first_hash,
                    capture_frames: frames,
                    capture_manifest: FRAMES_MANIFEST_NAME.to_string(),
                    sequence_fnv: sequence.to_string(),
                },
            )
        };
        let first_receipt = receipt_for(first_frames, &first_sequence);
        let second_receipt = receipt_for(second_frames, &second_sequence);
        assert_eq!(
            first_receipt, second_receipt,
            "receipt is not byte-identical"
        );
        for sha in fighter_bind::EXPECTED_BUNDLE_SHA256
            .iter()
            .chain(fighter_bind::EXPECTED_BODY_SHA256.iter())
        {
            assert!(first_receipt.contains(sha), "receipt lost a CT7 SHA");
        }
        assert!(
            first_receipt.contains("\"hits\":27"),
            "hits row:\n{first_receipt}"
        );
        assert!(
            first_receipt.contains("\"knockdowns\":3"),
            "knockdowns row:\n{first_receipt}"
        );
        assert!(
            first_receipt.contains(&format!("\"final_hash\":\"{first_hash:016x}\"")),
            "hash row:\n{first_receipt}"
        );
        assert!(
            first_receipt.contains(&first_sequence),
            "sequence row:\n{first_receipt}"
        );
    }

    #[test]
    fn staged_capture_manifests_differ_across_frames() {
        // Per-frame manifests are recorded and differ across frames, while
        // the same capture replays byte-identical twice.
        let (first_frames, first_sequence, first_bytes) = run_frame_capture("frames-a");
        let (second_frames, second_sequence, second_bytes) = run_frame_capture("frames-b");
        assert_eq!(first_frames, 4, "staged captures drifted");
        assert_eq!(second_frames, 4, "staged captures drifted");
        assert_eq!(
            first_sequence, second_sequence,
            "captures are not deterministic"
        );
        assert_eq!(first_bytes, second_bytes, "manifest is not byte-identical");
        for key in ["png_fnv", "checkpoint"] {
            let hexes = manifest_hexes(&first_bytes, key);
            assert_eq!(
                hexes.len(),
                4,
                "{key} rows:\n{}",
                String::from_utf8_lossy(&first_bytes)
            );
            let mut distinct = hexes.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), hexes.len(), "{key} repeats across frames");
        }
    }

    #[test]
    fn staged_manifest_and_receipt_name_renderer() {
        // Every filed row names the CPU raster that drew it, uniformly, and
        // the receipt carries the same key; a torn row never gains a key.
        let (_, _, bytes) = run_frame_capture("renderer");
        let text = std::str::from_utf8(&bytes).expect("manifest is UTF-8");
        assert_eq!(
            text.matches("\"renderer\":\"staged-cpu\"").count(),
            4,
            "renderer rows:\n{text}"
        );
        let bindings = fighter_bind::bind_all(&fighters_content_dir()).expect("CT7 manifest binds");
        let receipt = receipt_json(
            &bindings,
            four_fighter_config(DEFAULT_TICKS),
            &FightReport {
                stats: FightStats::default(),
                hp: vec![1000, 0, 0, 0],
                final_hash: 0,
                capture_frames: 4,
                capture_manifest: FRAMES_MANIFEST_NAME.to_string(),
                sequence_fnv: "0".repeat(16),
            },
        );
        assert!(
            receipt.contains("\"renderer\":\"staged-cpu\""),
            "receipt renderer:\n{receipt}"
        );
        // Failure: a torn row passes through unstamped, never corrupted.
        assert_eq!(with_staged_renderer("torn"), "torn".to_string());
    }

    #[test]
    fn staged_frames_show_fighters_not_noise() {
        // Each filed frame draws the fighters: non-sand pixels inside the body
        // rects, far fewer colours than the noise frames had (3072 of 3072
        // distinct pixels), and a real PNG signature on disk.
        let fight = run_staged_fight_frames().expect("staged frames run");
        assert_eq!(fight.frames.len(), 4, "staged frames drifted");
        let (width, height) = engine_game::windowed::SMOKE_FRAME_SIZE;
        let sand = [194u8, 160, 104, 255];
        for frame in &fight.frames {
            assert_eq!(&frame.png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
            let pixels = engine_game::windowed::staged_fight_frame_rgba(&frame.dots, width, height)
                .expect("raster builds");
            let body = pixels
                .chunks_exact(4)
                .filter(|pixel| *pixel != sand)
                .count();
            assert!(
                body >= 4 * 20,
                "tick {} hides its fighters ({body} px)",
                frame.tick
            );
            let mut colours: Vec<&[u8]> = pixels.chunks_exact(4).collect();
            colours.sort_unstable();
            colours.dedup();
            assert!(
                colours.len() < 16,
                "tick {} reads as noise ({} colours)",
                frame.tick,
                colours.len()
            );
        }
    }

    #[test]
    fn balance_screen_exercises_pq2_bar_offline() {
        // Behavior: the offline screen runs anchor plus lever through
        // `bench::run`, pins the per-move share and reports the 350 bar with
        // anchor PASS and lever TRIP. Same configs twice give the same line.
        let first = run_balance_screen().expect("balance screen runs");
        let second = run_balance_screen().expect("balance screen replays");
        assert_eq!(first, second, "screen is not deterministic");
        assert!(
            first.contains("anchor_share=0"),
            "bloodless anchor drifted: {first}"
        );
        assert!(first.contains("anchor=PASS"), "anchor must hold: {first}");
        assert!(first.contains("lever=TRIP"), "lever must trip: {first}");
        assert!(
            first.contains("bar=350"),
            "screen must reuse the 350 bar: {first}"
        );
        // The flag reaches the screen without touching the staged fight.
        let flagged = parse_args(&["staged_fight".to_string(), BALANCE_SCREEN_FLAG.to_string()])
            .expect("balance flag parses");
        assert!(flagged.balance_screen, "flag must arm the screen");
        let plain = parse_args(&["staged_fight".to_string()]).expect("defaults parse");
        assert!(!plain.balance_screen, "screen stays off by default");
    }

    #[test]
    fn balance_screen_pair_fails_loudly_on_drift() {
        // Failure: a tripped anchor or a held lever fails instead of filing a
        // vacuous PASS, so a drifted harness cannot report green.
        assert!(
            check_balance_pair(0, 1000).is_ok(),
            "lopsided must pass the pair"
        );
        assert!(
            check_balance_pair(400, 1000).is_err(),
            "tripped anchor must fail"
        );
        assert!(check_balance_pair(0, 0).is_err(), "held lever must fail");
        assert!(
            parse_args(&["staged_fight".to_string(), "--nope".to_string()]).is_err(),
            "unknown flag must fail"
        );
    }

    #[test]
    fn balance_screen_reports_agreement() {
        // Behavior: the screen wires `BenchReport::agrees_with` plus
        // `divergence_permille` behind `--balance-screen`, so the filed line
        // proves the anchor and lever disagree on the 5% contract bound.
        let line = run_balance_screen().expect("balance screen runs");
        assert!(
            line.contains("divergence="),
            "screen must report the contract distance: {line}"
        );
        assert!(
            line.contains("agree=false"),
            "anchor and lever must disagree: {line}"
        );
    }

    #[test]
    fn balance_screen_agreement_gate_rejects_identical() {
        // Failure: the agreement gate refuses an agreeing pair, so a vacuous
        // lever that mirrors the anchor cannot file green.
        let anchor_cfg = BenchConfig::standard(Policy::Idle, Policy::Idle);
        let anchor = engine_game::bench::run(&anchor_cfg).expect("anchor runs");
        let again = engine_game::bench::run(&anchor_cfg).expect("anchor replays");
        assert!(
            agreement_gate(&anchor, &again).is_err(),
            "identical reports must trip the agreement gate"
        );
        let lever_cfg = BenchConfig::standard(Policy::Poke, Policy::Idle);
        let lever = engine_game::bench::run(&lever_cfg).expect("lever runs");
        let divergence = agreement_gate(&anchor, &lever).expect("anchor vs lever disagrees");
        assert!(
            divergence > engine_game::bench::AGREE_TOL_PERMILLE,
            "lever must stand outside the 5% bound: {divergence}"
        );
    }
}
