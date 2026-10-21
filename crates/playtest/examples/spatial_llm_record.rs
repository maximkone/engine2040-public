//! SPR4 operator entry: record one live model into the spatial-llm cassette.
//!
//! Run with `OPENROUTER_API_KEY` set:
//! `cargo run -p engine-playtest --features live --example spatial_llm_record`
//! Pass `smoke` to ask one raw and one verb question without writing a file.

use engine_playtest::spatial_eval::llm::{self, Arm, Model};
use engine_playtest::spatial_eval::llm_live::{
    record_and_write_with, OpenRouter, FILE_STEM, MODEL,
};
use std::path::PathBuf;

fn main() {
    let smoke = std::env::args().nth(1).as_deref() == Some("smoke");
    if smoke {
        let agent = OpenRouter::from_env(MODEL).expect("key and endpoint");
        agent.set_trace_path(PathBuf::from("target/spatial-llm-trace.log"));
        let questions = llm::questions();
        for (arm, index) in [(Arm::Raw, 0_usize), (Arm::Verb, 4_usize)] {
            let q = &questions[index];
            let reply = agent.ask(arm, q);
            println!(
                "smoke {} {} calls={} in={} out={} answer={:?}",
                arm.as_str(),
                q.id,
                reply.calls,
                reply.tokens_in,
                reply.tokens_out,
                reply.answer
            );
            for line in agent.take_trace() {
                println!("  {line}");
            }
        }
        if let Some(why) = agent.failure() {
            panic!("{why}");
        }
        return;
    }
    let table = record_and_write_with(
        MODEL,
        FILE_STEM,
        Some(&PathBuf::from("target/spatial-llm-trace.log")),
        &|line| {
            println!("{line}");
        },
    )
    .expect("live recording");
    println!("{table}");
}
