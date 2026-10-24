//! `e2040-ladder`: print the DOOR7 six-rung ladder report once and stop.
//!
//! Consumer: `tools/agent-ladder.mjs`, which runs this binary twice and
//! compares the tape blocks, and the operator who reads one table by hand.
//! The session lives in memory: one empty document, full capability, no
//! socket and no project folder. Units follow `engine_endpoint::e2e_ladder`.
//! Determinism scope: the tape block is reproducible across runs; wall clock
//! prints beside it. Exit codes: 0 every rung passed, 1 a rung failed,
//! 2 usage.

use engine_editor::capability::Capability;
use engine_endpoint::e2e_ladder::{run_ladder, LADDER_MAGIC, LADDER_VERSION};
use engine_endpoint::mcp::McpServer;

/// Session id the ladder runs under; `door.status` reports it back.
const LADDER_SESSION: u64 = 7;

const USAGE: &str = "usage: e2040-ladder [--table | --tape]\n\
     run the six fixed prompts over the door verbs and print the report;\n\
     --tape prints only the compared artifact (no wall clock).\n\
     exit: 0 all rungs pass, 1 a rung failed, 2 usage.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut tape_only = false;
    for arg in &args {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            "--table" => {}
            "--tape" => tape_only = true,
            other => {
                eprintln!("e2040-ladder: unknown flag `{other}`");
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let server = McpServer::new(LADDER_SESSION, Capability::unrestricted(LADDER_SESSION));
    let report = run_ladder(&server);
    if tape_only {
        print!("{tape}", tape = report.tape());
    } else {
        println!("{report}");
        println!("# {LADDER_MAGIC} v{LADDER_VERSION}");
    }
    if report.failed() == 0 {
        return;
    }
    std::process::exit(1);
}
