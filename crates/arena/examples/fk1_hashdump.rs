//! fk1_hashdump: dump one scripted headless arena hash chain to stdout.
//!
//! Helper behind `scripts/fk1_determinism.mjs` (FK1 determinism gate). Runs a
//! scripted match through the gym headless path (`GymLive`, one step per tick)
//! and prints one `tick hash` hex row per tick, so two independent processes
//! give byte-comparable chains. Reads no files, writes no files; IO is stdout
//! rows plus one stderr receipt line, never in the tick. Errors: exit 2 for
//! bad flags/config, exit 1 for a rejected step or a gym/run parity break.

use engine_arena::{gym::GymLive, run, ArenaConfig, FighterTape};
use engine_input::{fighter::ATTACK, Buttons, FighterId, FighterInput};

/// Session seed (the staged-fight default).
const DEFAULT_SEED: u64 = 2040;
/// FK1 gate length in fixed ticks.
const DEFAULT_TICKS: u32 = 6000;
/// Staged-fight roster: hero plus three opponents.
const DEFAULT_FIGHTERS: u8 = 4;
/// Duel half-extent in meters.
const DEFAULT_EXTENT: f32 = 20.0;

/// Scripted tape: slot 0 circle-strafes and attacks every 5 ticks (the staged
/// script, so the match lands real blows), slot 1 drifts on its own cadence
/// and the brains fill the rest. Ticks ascend, fighters ascend: always sorted.
fn script(ticks: u32) -> FighterTape {
    let mut inputs = Vec::new();
    for tick in 0..ticks {
        let (mx, mz) = match (tick / 15) % 4 {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        inputs.push(FighterInput {
            tick,
            fighter: FighterId(0),
            mx,
            mz,
            facing: 0,
            buttons: if tick.is_multiple_of(5) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        });
        if tick.is_multiple_of(7) {
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(1),
                mx: -50,
                mz: 25,
                facing: 9_000,
                buttons: Buttons::NONE,
            });
        }
    }
    FighterTape(inputs)
}

/// This tick's slice of the sorted tape (empty when the script rests).
fn tick_inputs(tape: &FighterTape, tick: u32) -> &[FighterInput] {
    let start = tape.0.partition_point(|input| input.tick < tick);
    let end = tape.0.partition_point(|input| input.tick <= tick);
    &tape.0[start..end]
}

fn usage() -> String {
    "usage: fk1_hashdump [--seed <u64>] [--ticks <1..100000>] [--fighters <2..8>] [--extent <m>]"
        .to_string()
}

fn take_value(raw: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    match raw.get(*index) {
        Some(value) if !value.starts_with("--") => Ok(value.clone()),
        _ => Err(format!("{flag} needs a value")),
    }
}

fn parse_args(raw: &[String]) -> Result<(u64, u32, u8, f32), String> {
    let (mut seed, mut ticks, mut fighters, mut extent) = (
        DEFAULT_SEED,
        DEFAULT_TICKS,
        DEFAULT_FIGHTERS,
        DEFAULT_EXTENT,
    );
    let mut index = 1usize;
    while index < raw.len() {
        match raw[index].as_str() {
            "--seed" => {
                let value = take_value(raw, &mut index, "--seed")?;
                seed = value
                    .parse()
                    .map_err(|_| format!("--seed needs a u64, got {value}"))?;
            }
            "--ticks" => {
                let value = take_value(raw, &mut index, "--ticks")?;
                ticks = value
                    .parse()
                    .map_err(|_| format!("--ticks needs a u32, got {value}"))?;
            }
            "--fighters" => {
                let value = take_value(raw, &mut index, "--fighters")?;
                fighters = value
                    .parse()
                    .map_err(|_| format!("--fighters needs a u8, got {value}"))?;
            }
            "--extent" => {
                let value = take_value(raw, &mut index, "--extent")?;
                extent = value
                    .parse()
                    .map_err(|_| format!("--extent needs meters, got {value}"))?;
            }
            other => return Err(format!("unknown flag {other}")),
        }
        index += 1;
    }
    Ok((seed, ticks, fighters, extent))
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let (seed, ticks, fighters, extent) = match parse_args(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("fk1_hashdump: {err}; {}", usage());
            std::process::exit(2);
        }
    };
    let config = match ArenaConfig::try_new(seed, ticks, fighters, extent) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("fk1_hashdump: bad config: {err:?}");
            std::process::exit(2);
        }
    };
    let tape = script(ticks);
    let mut live = match GymLive::new(&config) {
        Some(live) => live,
        None => {
            eprintln!("fk1_hashdump: live env refused the config");
            std::process::exit(2);
        }
    };
    for tick in 0..ticks {
        if live.step(tick_inputs(&tape, tick)).is_none() {
            eprintln!("fk1_hashdump: step {tick} rejected");
            std::process::exit(1);
        }
        let snapshot = live.snapshot();
        if snapshot.tick != tick + 1 {
            eprintln!(
                "fk1_hashdump: cursor drifted to {} at tick {tick}",
                snapshot.tick
            );
            std::process::exit(1);
        }
        println!("{tick:06} {:016x}", snapshot.digest);
    }
    // Gym/run parity: the live env's final digest must equal `run`'s final
    // checkpoint hash over the same config plus tape.
    let points = run(&config, &tape);
    if points.len() != ticks as usize {
        eprintln!(
            "fk1_hashdump: run gave {}/{} checkpoints",
            points.len(),
            ticks
        );
        std::process::exit(1);
    }
    let final_hash = points.last().map_or(0, |point| point.hash);
    if live.snapshot().digest != final_hash {
        eprintln!("fk1_hashdump: gym/run parity broke at final_hash={final_hash:016x}");
        std::process::exit(1);
    }
    eprintln!(
        "fk1_hashdump: ticks={ticks} seed={seed} fighters={fighters} final_hash={final_hash:016x}"
    );
}
