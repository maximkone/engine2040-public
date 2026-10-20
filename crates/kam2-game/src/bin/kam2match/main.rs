//! KAM6 headless match: the AI mayor in the side-1 seat for 3000 ticks.
//!
//! No window and no GPU: the default build runs the same [`AiMatch`] loop the
//! window plays and prints one receipt line. A refused AI order fails the run,
//! so a green line proves the mayor only asked for what the sim takes.
//!
//! The two-purse receipt runs the field twice. Each run also builds a small
//! side-0 chain on the west half from side 0's own stock, so both sides bake
//! bread with their own wares and the `KAM2 SIDES OK` line prints twice with one
//! hash. Before the side split this field printed
//! `KAM2 AI OK ... side1_houses=10 hash=ff4ce6d9be42e1a4`; the split moved it to
//! `hash=7d537395e4b4ac70` (both purses and every house owner are in the digest).

use std::process::ExitCode;

use kam2_game::{AiMatch, RECEIPT_TICKS};
use kam2_sim::{Command, HouseKind, Refusal};

/// Row of the side-0 road on the receipt field, north of the AI's row.
const PLAYER_ROAD_ROW: i32 = 3;
/// The side-0 chain: kind and top-left tile, paid from side 0's stock.
const PLAYER_TOWN: [(HouseKind, i32, i32); 4] = [
    (HouseKind::Warehouse, 0, 4),
    (HouseKind::Farm, 6, 1),
    (HouseKind::Mill, 8, 1),
    (HouseKind::Bakery, 10, 1),
];

/// Lay the side-0 road and place the side-0 chain through the player's own orders.
fn player_town(game: &mut AiMatch) -> Result<(), Refusal> {
    for x in 0..=12 {
        game.order(Command::PlaceRoad {
            x,
            y: PLAYER_ROAD_ROW,
        })?;
    }
    for (kind, x, y) in PLAYER_TOWN {
        game.order(Command::PlaceHouse { kind, x, y })?;
    }
    Ok(())
}

/// One full run: the KAM6 field, the side-0 chain, then the receipt ticks.
fn play() -> Result<AiMatch, Refusal> {
    let mut game = AiMatch::new()?;
    player_town(&mut game)?;
    game.advance(RECEIPT_TICKS);
    Ok(game)
}

fn main() -> ExitCode {
    let first = match play() {
        Ok(game) => game,
        Err(refusal) => {
            eprintln!("kam2match: the match does not start: {refusal:?}");
            return ExitCode::FAILURE;
        }
    };
    let second = match play() {
        Ok(game) => game,
        Err(refusal) => {
            eprintln!("kam2match: the second match does not start: {refusal:?}");
            return ExitCode::FAILURE;
        }
    };
    if first.refusals() > 0 || second.refusals() > 0 {
        eprintln!(
            "kam2match: the sim refused {} AI orders",
            first.refusals().saturating_add(second.refusals())
        );
        return ExitCode::FAILURE;
    }
    println!("{}", first.receipt());
    println!("{}", first.sides_receipt());
    println!("{}", second.sides_receipt());
    if first.sim().hash() != second.sim().hash() {
        eprintln!("kam2match: the two runs drifted apart");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
