//! API4 tool-collapse measurement: 20 task fixtures against the flat door.
//!
//! The flat door advertises every reachable tool (13 core plus every pack
//! verb). Door v2 advertises the 13 core tools and reaches a pack verb as
//! `api.search` -> `api.describe` -> `api.call` (the `api.load` step is
//! optional). This file measures that trade on 20 fixtures: tool-definition
//! size, success (the door arm must return exactly the flat arm's answer, with
//! the verb found in the top 5 hits and its args listed by `api.describe`), and
//! in-process latency. Tokens are estimated as chars / 4. Latency is printed,
//! not asserted: in-process the search step is the whole overhead, and the
//! real cost is two extra model turns per pack verb.
#![cfg(feature = "mcp")]

use std::time::Instant;

use engine_editor::capability::Capability;
use engine_endpoint::insight::mcp_tools::{
    ApiDescribeArgs, ApiDescribeTool, ApiSearchArgs, ApiSearchTool,
};
use engine_endpoint::mcp::{McpServer, SceneReadTool, SimBuildEncounterTool};
use engine_endpoint::packs::{ApiCallArgs, ApiCallTool, PackRegistry};
use engine_endpoint::tool_macro::TestRunTool;
use rmcp::handler::server::router::tool::SyncTool;
use rmcp::serde_json::{self, json, Value};

/// One task: what the agent wants, the verb that answers it, its args.
struct Fixture {
    task: &'static str,
    verb: &'static str,
    args: Value,
}

fn fixtures() -> Vec<Fixture> {
    let f = |task, verb, args| Fixture { task, verb, args };
    vec![
        f("snapshot of the scene rows with stable ids", "scene.read", json!({})),
        f("run the structured harvest checks", "test.run", json!({})),
        f(
            "plan an encounter from a short brief",
            "sim.build_encounter",
            json!({"brief": "two bandits ambush a courier at a bridge", "seed": 7}),
        ),
        f("list the shipped material records", "material.list", json!({})),
        f("json schema of a material record", "material.schema", json!({})),
        f("short brief of the staged scene", "scene.brief", json!({})),
        f("spatial findings such as overlaps in the scene", "scene.spatial", json!({})),
        f("digest of the observed event ring", "log.digest", json!({})),
        f("page through the observed events", "log.query", json!({})),
        f("self describing manifest of every tool", "rpc.discover", json!({})),
        f("read the animation state machine", "anim.graph.get", json!({})),
        f("analyze the weakest bay of a frame building", "structure.analyze", json!({})),
        f("fracture a wall into voronoi cells", "break.fracture", json!({})),
        f("simulate blade hits on worn armor", "hit.simulate", json!({})),
        f("reset the headless spar to tick zero", "sim.reset", json!({})),
        f("step the headless spar through a tape", "sim.step", json!({})),
        f("hash a bout prefix into a state digest", "state.digest", json!({})),
        f("how to make a prop from a picture", "asset.generate", json!({})),
        f("what one blow does to a fighter's balance", "physics.read", json!({})),
        f("run a bout until a fighter's hp falls low", "play.step_until", json!({})),
    ]
}

fn server() -> McpServer {
    McpServer::new(7, Capability::unrestricted(7))
}

/// Run a typed core tool from JSON args and back to JSON.
fn typed<T>(server: &McpServer, args: Value) -> Result<Value, String>
where
    T: SyncTool<McpServer>,
    T::Parameter: rmcp::serde::de::DeserializeOwned,
    T::Output: rmcp::serde::Serialize,
    T::Error: std::fmt::Debug,
{
    let param = serde_json::from_value(args).map_err(|e| e.to_string())?;
    let out = T::invoke(server, param).map_err(|e| format!("{e:?}"))?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

/// The flat door's call: the tool by exact name, nothing in between.
fn flat(server: &McpServer, fixture: &Fixture) -> Result<Value, String> {
    let args = fixture.args.clone();
    match fixture.verb {
        "scene.read" => typed::<SceneReadTool>(server, args),
        "test.run" => typed::<TestRunTool>(server, args),
        "sim.build_encounter" => typed::<SimBuildEncounterTool>(server, args),
        verb => PackRegistry::shipped()
            .verb(verb)
            .expect("fixture verb is registered")
            .invoke(server, args)
            .map_err(|e| format!("{e:?}")),
    }
}

/// The door v2 call for a pack verb: search, describe, call. Err names the
/// step that failed. Core tools are advertised, so they go straight through.
fn door(server: &McpServer, fixture: &Fixture) -> Result<Value, String> {
    if PackRegistry::shipped().verb(fixture.verb).is_none() {
        return flat(server, fixture);
    }
    let found = ApiSearchTool::invoke(
        server,
        ApiSearchArgs {
            query: fixture.task.to_owned(),
            limit: Some(5),
        },
    )
    .map_err(|e| format!("search: {e:?}"))?;
    if !found.hits.iter().any(|hit| hit.path == fixture.verb) {
        let top: Vec<&str> = found.hits.iter().map(|h| h.path.as_str()).collect();
        return Err(format!("search: `{}` not in top 5 {top:?}", fixture.verb));
    }
    let entry = ApiDescribeTool::invoke(
        server,
        ApiDescribeArgs {
            path: fixture.verb.to_owned(),
        },
    )
    .map_err(|e| format!("describe: {e:?}"))?;
    for key in fixture.args.as_object().into_iter().flat_map(|o| o.keys()) {
        if !entry.docs.contains(&format!("{key}:")) {
            return Err(format!("describe: `{key}` missing from `{}`", entry.docs));
        }
    }
    let called = ApiCallTool::invoke(
        server,
        ApiCallArgs {
            verb: fixture.verb.to_owned(),
            args: Some(fixture.args.clone()),
        },
    )
    .map_err(|e| format!("call: {e:?}"))?;
    Ok(called.result)
}

fn chars(tools: &[rmcp::model::Tool]) -> usize {
    tools
        .iter()
        .map(|tool| serde_json::to_string(tool).expect("tool serializes").len())
        .sum()
}

fn p95(samples: &mut [u128]) -> u128 {
    samples.sort_unstable();
    samples[samples.len() * 95 / 100]
}

#[test]
fn door_v2_cuts_tool_definitions_by_half_with_no_success_loss() {
    let probe = server();
    let all = chars(&probe.all_tools());
    let core = chars(&probe.core_tools());
    let listed = chars(&probe.tools());
    let saved = 100 - core * 100 / all;
    eprintln!(
        "API4 tokens: flat {} tools {all} chars (~{} tokens), door v2 core {} tools {core} chars \
         (~{} tokens, {saved}% fewer), default list {listed} chars",
        probe.all_tools().len(),
        all / 4,
        probe.core_tools().len(),
        core / 4,
    );
    assert!(saved >= 50, "door v2 advertises {core} of {all} chars: {saved}% fewer, bar is 50%");

    let all_fixtures = fixtures();
    assert_eq!(all_fixtures.len(), 20, "the spike runs 20 fixtures");
    let (mut flat_us, mut door_us) = (Vec::new(), Vec::new());
    let mut lost = Vec::new();
    for fixture in &all_fixtures {
        let expected = match flat(&server(), fixture) {
            Ok(answer) => answer,
            Err(e) => {
                lost.push(format!("{}: flat arm must answer: {e}", fixture.verb));
                continue;
            }
        };
        for rep in 0..5 {
            let (flat_server, door_server) = (server(), server());
            let start = Instant::now();
            let flat_answer = flat(&flat_server, fixture);
            flat_us.push(start.elapsed().as_micros());
            let start = Instant::now();
            let door_answer = door(&door_server, fixture);
            door_us.push(start.elapsed().as_micros());
            assert_eq!(flat_answer.as_ref(), Ok(&expected), "flat arm is deterministic");
            if rep == 0 && door_answer.as_ref() != Ok(&expected) {
                lost.push(format!("{}: {:?}", fixture.verb, door_answer.err()));
            }
        }
    }
    eprintln!(
        "API4 success: {}/20, p95 in-process flat {} us, door {} us",
        20 - lost.len(),
        p95(&mut flat_us),
        p95(&mut door_us),
    );
    assert!(lost.is_empty(), "success lost on: {lost:#?}");
}
