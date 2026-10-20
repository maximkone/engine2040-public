//! CMB1 headless skirmish: two swordsmen eight tiles apart, both attack-move.
//!
//! One run plays the fight through [`kam2_game::orders`], then the run repeats and
//! the two hashes must agree. A refused command or a stall fails the run, so a green
//! line proves the order layer only asked for what the sim takes.
//!
//! The line: `KAM2 ORDERS OK tick=<end> approached=<tiles> kills=<n> refusals=0 hash=<hex>`.

use std::process::ExitCode;

use kam2_game::{skirmish, SKIRMISH_GAP, SKIRMISH_TICKS};
use kam2_sim::{Refusal, Sim};

/// One finished skirmish, as the receipt needs it.
struct Run {
    tick: u32,
    approached: i32,
    kills: u32,
    refusals: u32,
    hash: u64,
}

/// Taxicab tiles between the two swordsmen, while both live.
fn gap(sim: &Sim) -> Option<i32> {
    let first = sim.unit(0)?;
    let second = sim.unit(1)?;
    Some((first.x - second.x).abs() + (first.y - second.y).abs())
}

/// One full run: both attack-move orders, played until the last one ends.
fn play() -> Result<Run, String> {
    let (mut sim, mut orders) = skirmish().map_err(|refusal: Refusal| format!("{refusal:?}"))?;
    let mut closest = SKIRMISH_GAP;
    while sim.ticks() < SKIRMISH_TICKS {
        orders.tick(&mut sim);
        if orders.is_empty() {
            break;
        }
        sim.tick();
        if let Some(distance) = gap(&sim) {
            closest = closest.min(distance);
        }
        orders.retire(&sim);
        if orders.is_empty() {
            break;
        }
    }
    if !orders.is_empty() {
        return Err(format!("the orders still run after {SKIRMISH_TICKS} ticks"));
    }
    Ok(Run {
        tick: sim.ticks(),
        approached: SKIRMISH_GAP - closest,
        kills: 2u32.saturating_sub(u32::try_from(sim.units().len()).unwrap_or(2)),
        refusals: orders.refusals(),
        hash: sim.hash(),
    })
}

/// One receipt line; both runs print the same line when the hash holds.
fn line(run: &Run) -> String {
    format!(
        "KAM2 ORDERS OK tick={} approached={} kills={} refusals={} hash={:016x}",
        run.tick, run.approached, run.kills, run.refusals, run.hash
    )
}

fn main() -> ExitCode {
    let first = match play() {
        Ok(run) => run,
        Err(message) => {
            eprintln!("kam2orders: the skirmish does not finish: {message}");
            return ExitCode::FAILURE;
        }
    };
    let second = match play() {
        Ok(run) => run,
        Err(message) => {
            eprintln!("kam2orders: the second skirmish does not finish: {message}");
            return ExitCode::FAILURE;
        }
    };
    if first.refusals > 0 || second.refusals > 0 {
        eprintln!(
            "kam2orders: the sim refused {} order commands",
            first.refusals.saturating_add(second.refusals)
        );
        return ExitCode::FAILURE;
    }
    if first.hash != second.hash || first.tick != second.tick || first.kills != second.kills {
        eprintln!("kam2orders: the two runs drifted apart");
        return ExitCode::FAILURE;
    }
    println!("{}", line(&first));
    println!("{}", line(&second));
    ExitCode::SUCCESS
}
