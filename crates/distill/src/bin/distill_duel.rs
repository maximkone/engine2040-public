//! distill_duel: distill the arena depth brain into an int8 `.e2nn` student
//! and measure it in real arena bouts.
//!
//! Chain (gogame 6.2 teacher data, 14.2 small net in the tick, 10.3 strength):
//! 1. record `(lanes, intent)` rows from actuated brain-vs-brain duels on
//!    fixed seeds (see the `duel` module docs for the actuator);
//! 2. adopt them as a digested replay buffer;
//! 3. fit a two-layer student (std-only softmax cross-entropy, seeded);
//! 4. fold it to int8 `.e2nn` bytes and write them with their FNV-1a digest;
//! 5. play the strength gate twice: student vs the actuated brain (the
//!    like-for-like check) and vs the raw arena brain, sides swapped every
//!    bout, Wilson 95% interval and binomial p-value.
//!
//! Usage: `distill_duel [--episodes N] [--ticks T] [--seed S] [--hidden H]
//! [--epochs E] [--bouts B] [--bout-ticks T] [--gate-seed S] [--out PATH]
//! [--check] [--repeat]`. `--check` regenerates and compares against the file
//! at `--out` instead of writing it; `--repeat` plays the gate twice and fails
//! unless both tables match. Exit 1 on any refusal or mismatch.

use std::process::ExitCode;

use engine_distill::{
    check_fps, export_mlp, fit_mlp, strength_gate, teacher_episode, teacher_rows_per_episode,
    DatasetManifest, Episode, MlpSpec, Opponent, ReplayBuffer, ReplayConfig, StudentSpec,
    DEFAULT_BOUTS, DEFAULT_BOUT_TICKS, DUEL_CLASSES, DUEL_LANES, INPUT_QUANT_SCALE,
};

/// Generator settings; defaults produce the committed fixture.
struct Args {
    episodes: usize,
    ticks: u32,
    seed: u64,
    hidden: usize,
    epochs: u32,
    bouts: u32,
    bout_ticks: u32,
    gate_seed: u64,
    out: String,
    check: bool,
    repeat: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            episodes: 50,
            ticks: 3_600,
            seed: 0xD15_7111,
            hidden: 24,
            epochs: 40,
            bouts: DEFAULT_BOUTS,
            bout_ticks: DEFAULT_BOUT_TICKS,
            gate_seed: 0x6A7E_0000,
            out: "fixtures/nets/depth_student.e2nn".to_string(),
            check: false,
            repeat: false,
        }
    }
}

fn parse_num<T: std::str::FromStr>(flag: &str, value: Option<String>) -> Result<T, String> {
    let raw = value.ok_or_else(|| format!("{flag} needs a value"))?;
    let digits = raw.trim_start_matches("0x");
    let parsed = if raw.starts_with("0x") {
        u64::from_str_radix(digits, 16)
            .map_err(|e| e.to_string())
            .and_then(|v| {
                v.to_string()
                    .parse::<T>()
                    .map_err(|_| "out of range".to_string())
            })
    } else {
        raw.parse::<T>().map_err(|_| "not a number".to_string())
    };
    parsed.map_err(|e| format!("{flag} {raw}: {e}"))
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--episodes" => args.episodes = parse_num(&flag, it.next())?,
            "--ticks" => args.ticks = parse_num(&flag, it.next())?,
            "--seed" => args.seed = parse_num(&flag, it.next())?,
            "--hidden" => args.hidden = parse_num(&flag, it.next())?,
            "--epochs" => args.epochs = parse_num(&flag, it.next())?,
            "--bouts" => args.bouts = parse_num(&flag, it.next())?,
            "--bout-ticks" => args.bout_ticks = parse_num(&flag, it.next())?,
            "--gate-seed" => args.gate_seed = parse_num(&flag, it.next())?,
            "--out" => args.out = it.next().ok_or("--out needs a path")?,
            "--check" => args.check = true,
            "--repeat" => args.repeat = true,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(args)
}

/// Steps 1-4: teacher rows, buffer, fit, export. Returns the `.e2nn` bytes.
fn distill(args: &Args) -> Result<Vec<u8>, String> {
    let rows_per = teacher_rows_per_episode(args.ticks);
    let config = ReplayConfig::new(DUEL_LANES, DUEL_CLASSES, args.episodes, rows_per)
        .map_err(|e| format!("replay shape: {e}"))?;
    let mut episodes = Vec::with_capacity(args.episodes);
    let mut class_counts = [0usize; DUEL_CLASSES];
    let mut seed = args.seed;
    for _ in 0..args.episodes {
        let rows = teacher_episode(seed, args.ticks).map_err(|e| format!("teacher: {e}"))?;
        let mut features = Vec::with_capacity(rows.len());
        let mut actions = Vec::with_capacity(rows.len());
        for (lanes, class) in rows {
            class_counts[usize::from(class)] += 1;
            features.push(
                lanes
                    .iter()
                    .map(|q| f32::from(*q) / INPUT_QUANT_SCALE)
                    .collect(),
            );
            actions.push(class);
        }
        episodes.push(Episode {
            seed,
            features,
            actions,
        });
        seed = seed.wrapping_add(1);
    }
    let buffer =
        ReplayBuffer::from_episodes(config, episodes).map_err(|e| format!("buffer: {e}"))?;
    println!(
        "teacher: {} duels x {} ticks, {} rows, classes [attack block dodge advance retreat] = {class_counts:?}, buffer digest {:016x}",
        args.episodes,
        args.ticks,
        buffer.len_steps(),
        buffer.digest
    );
    let manifest = DatasetManifest::new(
        "arena-depth-brain-duels",
        "MIT",
        buffer.digest,
        "duel-intent-policy",
    )
    .map_err(|e| format!("manifest: {e}"))?;
    let student =
        StudentSpec::new(DUEL_LANES, DUEL_CLASSES, args.seed).map_err(|e| format!("spec: {e}"))?;
    let samples = buffer
        .to_samples(&student)
        .map_err(|e| format!("samples: {e}"))?;
    let spec = MlpSpec {
        student,
        hidden: args.hidden,
        epochs: args.epochs,
        batch: 256,
        learning_rate: 0.01,
    };
    let mlp = fit_mlp(&samples, &spec, Some(&manifest)).map_err(|e| format!("fit: {e}"))?;
    let (agree, total) = mlp.agreement(&samples);
    let bytes = export_mlp(&mlp, &samples, &[manifest]).map_err(|e| format!("export: {e}"))?;
    let model = engine_nn::load(&bytes).map_err(|e| format!("reload: {e}"))?;
    let mut int_agree = 0usize;
    for row in &samples {
        let logits = engine_nn::infer(&model, &engine_distill::quantize_input(&row.features));
        if engine_distill::argmax(&logits) == Some(usize::from(row.target)) {
            int_agree += 1;
        }
    }
    println!(
        "student: {DUEL_LANES}-{}-{DUEL_CLASSES} int8, {} B, digest {:016x}; teacher agreement float {agree}/{total}, int8 {int_agree}/{total}",
        args.hidden,
        bytes.len(),
        model.digest()
    );
    let fps = check_fps(&bytes).map_err(|e| format!("fps budget: {e}"))?;
    println!("{fps}");
    Ok(bytes)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let bytes = distill(&args)?;
    if args.check {
        let on_disk = std::fs::read(&args.out).map_err(|e| format!("read {}: {e}", args.out))?;
        if on_disk != bytes {
            return Err(format!(
                "{} differs from the regenerated net ({:016x} vs {:016x})",
                args.out,
                engine_core::hash::hash_bytes(&on_disk),
                engine_core::hash::hash_bytes(&bytes)
            ));
        }
        println!("check: {} matches the regenerated net", args.out);
    } else {
        if let Some(dir) = std::path::Path::new(&args.out).parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
        }
        std::fs::write(&args.out, &bytes).map_err(|e| format!("write {}: {e}", args.out))?;
        println!("wrote {}", args.out);
    }
    for opponent in [Opponent::BrainIntents, Opponent::ArenaBrain] {
        let report = strength_gate(
            &bytes,
            opponent,
            args.gate_seed,
            args.bouts,
            args.bout_ticks,
        )
        .map_err(|e| format!("gate: {e}"))?;
        println!("{report}");
        if args.repeat {
            let again = strength_gate(
                &bytes,
                opponent,
                args.gate_seed,
                args.bouts,
                args.bout_ticks,
            )
            .map_err(|e| format!("gate: {e}"))?;
            if again != report {
                return Err(format!(
                    "strength gate vs {opponent:?} differs on the second run"
                ));
            }
            println!("repeat: second table vs {opponent:?} identical");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("distill_duel: {e}");
            ExitCode::FAILURE
        }
    }
}
