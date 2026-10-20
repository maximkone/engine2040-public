//! KAM6 headless match: the AI mayor in the side-1 seat for 3000 ticks.
//!
//! No window and no GPU: the default build runs the same [`AiMatch`] loop the
//! window plays, with the player idle, and prints one receipt line. A refused
//! AI order fails the run, so a green line proves the mayor only asked for what
//! the sim takes.

use std::process::ExitCode;

use kam2_game::{AiMatch, RECEIPT_TICKS};

fn main() -> ExitCode {
    let mut game = match AiMatch::new() {
        Ok(game) => game,
        Err(refusal) => {
            eprintln!("kam2match: the match does not start: {refusal:?}");
            return ExitCode::FAILURE;
        }
    };
    game.advance(RECEIPT_TICKS);
    if game.refusals() > 0 {
        eprintln!("kam2match: the sim refused {} AI orders", game.refusals());
        return ExitCode::FAILURE;
    }
    println!("{}", game.receipt());
    ExitCode::SUCCESS
}
