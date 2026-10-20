//! motion_bake: the offline teacher bake (AM27). Not a tick crate, not a
//! workspace member: it runs on a desk, reads a teacher file and writes a
//! committed fixture plus its receipt.
//!
//!   motion_bake synth --seed 7 --out fixtures/clips/stagger.npz
//!   motion_bake bake --source fixtures/clips/stagger.npz --name stagger \
//!       --out fixtures/clips/stagger.am5p --receipt fixtures/clips/stagger.receipt.json
//!
//! `synth` writes the seed's synthetic SOMA-77 stagger in Kimodo's NPZ layout
//! (the committed source the fixture bakes from); `bake` reads any clip in that
//! layout, retargets it through `soma_pairs()`, applies the PartiGen endpoint
//! rule and packs the two foot lanes to `.am5p`. Both print one line with the
//! numbers the gate reads, and the same bytes every run.

use std::process::ExitCode;

mod bake;
mod npz;
mod rig;
mod synth;

/// Default seed of the committed stagger source.
const DEFAULT_SEED: i64 = 7;
/// Default source path, relative to the repository root.
const DEFAULT_SOURCE: &str = "fixtures/clips/stagger.npz";
/// Default pack path, relative to the repository root.
const DEFAULT_PACK: &str = "fixtures/clips/stagger.am5p";
/// Default receipt path, relative to the repository root.
const DEFAULT_RECEIPT: &str = "fixtures/clips/stagger.receipt.json";

fn usage() -> &'static str {
    "usage:\n  motion_bake synth [--seed N] [--out PATH]\n  motion_bake bake [--source PATH] [--name NAME] [--out PATH] [--receipt PATH]"
}

/// One `--flag value` pair by name, `None` when the flag was not passed.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let at = args.iter().position(|arg| arg == name)?;
    args.get(at + 1).map(String::as_str)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("{}", usage());
        return ExitCode::from(2);
    };
    match command {
        "synth" => match run_synth(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("motion_bake synth: {why}");
                ExitCode::FAILURE
            }
        },
        "bake" => match run_bake(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("motion_bake bake: {why}");
                ExitCode::FAILURE
            }
        },
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("motion_bake: unknown command {other}\n{}", usage());
            ExitCode::from(2)
        }
    }
}

/// `synth`: write the seed's teacher clip as an `.npz`.
fn run_synth(args: &[String]) -> Result<(), String> {
    let seed = match flag(args, "--seed") {
        Some(text) => text
            .parse::<i64>()
            .map_err(|_| format!("--seed {text} is not an integer"))?,
        None => DEFAULT_SEED,
    };
    let out = flag(args, "--out").unwrap_or(DEFAULT_SOURCE);
    let clip = synth::stagger(seed);
    let bytes = npz::write_npz(&clip.arrays());
    write_file(out, &bytes)?;
    println!(
        "motion_bake: synth seed={seed} frames={} fps={} bytes={} sha256={} out={out}",
        clip.frames(),
        clip.fps,
        bytes.len(),
        bake::sha256_hex(&bytes)
    );
    Ok(())
}

/// `bake`: read a teacher clip, pack it and write the receipt beside it.
fn run_bake(args: &[String]) -> Result<(), String> {
    let source = flag(args, "--source").unwrap_or(DEFAULT_SOURCE);
    let out = flag(args, "--out").unwrap_or(DEFAULT_PACK);
    let receipt_path = flag(args, "--receipt").unwrap_or(DEFAULT_RECEIPT);
    let name = flag(args, "--name").unwrap_or("stagger");
    let bytes = std::fs::read(source).map_err(|why| format!("read {source}: {why}"))?;
    let baked = bake::bake(&bytes, name).map_err(|why| why.to_string())?;
    write_file(out, &baked.bytes)?;
    write_file(receipt_path, baked.receipt.to_json().as_bytes())?;
    let seed = match baked.receipt.seed {
        Some(seed) => seed.to_string(),
        None => "none".to_string(),
    };
    println!(
        "motion_bake: clip={name} seed={seed} source={source} source_sha256={} ticks={} qclip_hash=0x{:016x} avg_slip={:.6}/{:.6} packed={} retarget_digest=0x{:016x} out={out} receipt={receipt_path}",
        bake::sha256_hex(&bytes),
        baked.receipt.ticks,
        baked.receipt.qclip_hash,
        baked.receipt.avg_slip[0],
        baked.receipt.avg_slip[1],
        baked.bytes.len(),
        baked.receipt.retarget_digest
    );
    Ok(())
}

/// Write bytes, creating the parent directory when the path names one.
fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|why| format!("mkdir {parent:?}: {why}"))?;
        }
    }
    std::fs::write(path, bytes).map_err(|why| format!("write {path}: {why}"))
}
