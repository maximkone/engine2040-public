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
use engine_endpoint::mcp::{
    McpServer, SceneReadTool, SimBuildEncounterTool, TOOL_LIST_BUDGET_CHARS,
    TOOL_LIST_MIN_HEADROOM_CHARS,
};
use engine_endpoint::packs::{ApiCallArgs, ApiCallTool, ApiLoadTool, PackRegistry};
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
        f(
            "snapshot of the scene rows with stable ids",
            "scene.read",
            json!({}),
        ),
        f("run the structured harvest checks", "test.run", json!({})),
        f(
            "plan an encounter from a short brief",
            "sim.build_encounter",
            json!({"brief": "two bandits ambush a courier at a bridge", "seed": 7}),
        ),
        f(
            "list the shipped material records",
            "material.list",
            json!({}),
        ),
        f(
            "json schema of a material record",
            "material.schema",
            json!({}),
        ),
        f("short brief of the staged scene", "scene.brief", json!({})),
        f(
            "spatial findings such as overlaps in the scene",
            "scene.spatial",
            json!({}),
        ),
        f("digest of the observed event ring", "log.digest", json!({})),
        f("cursor page through observe events", "log.query", json!({})),
        f(
            "self describing manifest of every tool",
            "rpc.discover",
            json!({}),
        ),
        f(
            "read the animation state machine",
            "anim.graph.get",
            json!({}),
        ),
        f(
            "analyze the weakest bay of a frame building",
            "structure.analyze",
            json!({}),
        ),
        f(
            "fracture a wall into voronoi cells",
            "break.fracture",
            json!({}),
        ),
        f(
            "simulate blade hits on worn armor",
            "hit.simulate",
            json!({}),
        ),
        f(
            "reset the headless spar to tick zero",
            "sim.reset",
            json!({}),
        ),
        f(
            "step the headless spar through a tape",
            "sim.step",
            json!({"ticks": 30}),
        ),
        f(
            "hash a bout prefix into a state digest",
            "state.digest",
            json!({"ticks": 30}),
        ),
        f(
            "how to make a prop from a picture",
            "asset.generate",
            json!({}),
        ),
        f(
            "what one blow does to a fighter's balance",
            "physics.read",
            json!({}),
        ),
        f(
            "run a bout until a fighter's hp falls low",
            "play.step_until",
            json!({"tick_cap": 30, "fighter": 0, "hp_below": 500}),
        ),
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

fn pct(samples: &mut [u128], p: usize) -> u128 {
    samples.sort_unstable();
    samples[samples.len() * p / 100]
}

fn p95(samples: &mut [u128]) -> u128 {
    pct(samples, 95)
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
    assert!(
        saved >= 50,
        "door v2 advertises {core} of {all} chars: {saved}% fewer, bar is 50%"
    );

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
            assert_eq!(
                flat_answer.as_ref(),
                Ok(&expected),
                "flat arm is deterministic"
            );
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

/// API6 collapse guardrail on the worst list the door can show: every pack
/// loaded, the current flat surface (76 tools, 164,676 chars). The door
/// advertises the pinned retain spine plus the adaptive pack share, and what
/// it hides stays callable through the advertised `api.call`.
///
/// This test prices the guardrail itself: tokens (serialized chars, the same
/// measure as the API4 report), success (every fixture answers exactly what
/// the flat door answers in the same wide state) and latency (in-process per
/// answer: each arm pairs its calls on the same state, five reps, median, then
/// p95 over the 20 fixture medians). Finding a hidden verb is API4's measured
/// step (search hit, describe args), not re-priced here: the guardrail only
/// decides what is advertised.
#[test]
fn guardrail_holds_the_wide_surface_with_no_answer_loss() {
    let probe = server();
    let flat_tools = probe.all_tools();
    let flat_chars = chars(&flat_tools);

    // Worst case: a session that loads every pack.
    let wide = server();
    load_every_pack(&wide);
    let guarded_tools = wide.tools();
    let guarded_chars = chars(&guarded_tools);
    let guarded_ledger = wide.tool_list_budget_chars();
    let guarded_names: Vec<&str> = guarded_tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect();
    let plan = engine_endpoint::collapse::plan(&flat_tools, &wide.load_order());
    let saved = 100 - guarded_chars * 100 / flat_chars;
    eprintln!(
        "API6 tokens: flat {} tools {flat_chars} chars (~{} tokens), guarded {} tools \
         {guarded_chars} chars (~{} tokens, {saved}% fewer), hidden {} names ({}%), \
         default session {} chars",
        flat_tools.len(),
        flat_chars / 4,
        guarded_tools.len(),
        guarded_chars / 4,
        plan.hidden.len(),
        plan.hidden_share_pct(),
        chars(&probe.tools()),
    );
    assert!(
        guarded_chars * 2 <= flat_chars,
        "guarded list costs {guarded_chars} of {flat_chars} chars: {saved}% fewer, bar is 50%"
    );
    assert!(
        guarded_names.len() < flat_tools.len(),
        "the wide session hides {} tools",
        flat_tools.len() - guarded_names.len()
    );
    for kept in engine_endpoint::collapse::RETAIN_TOOL_NAMES {
        assert!(
            guarded_names.contains(&kept),
            "retain tool `{kept}` stays advertised on the wide session"
        );
    }
    let surface_ledger = wide.surface_chars();
    eprintln!(
        "API6 ledger: surface {surface_ledger} chars, advertised {guarded_ledger} chars, ceiling \
         {} chars (budget {TOOL_LIST_BUDGET_CHARS} minus the {TOOL_LIST_MIN_HEADROOM_CHARS} floor)",
        engine_endpoint::collapse::MAX_ADVERTISED_CHARS,
    );
    assert!(
        surface_ledger > TOOL_LIST_BUDGET_CHARS,
        "the wide session's surface {surface_ledger} stays over budget {TOOL_LIST_BUDGET_CHARS}, \
         so the guardrail has to trim what it advertises"
    );
    assert!(
        guarded_ledger <= engine_endpoint::collapse::MAX_ADVERTISED_CHARS,
        "advertised list costs {guarded_ledger} chars, over the ledger maximum"
    );
    assert!(
        wide.tool_budget_gate_with_bloat(0).is_ok(),
        "the guarded list passes the door's own ledger gate"
    );
    assert!(
        chars(&probe.tools()) < engine_endpoint::collapse::MAX_ADVERTISED_CHARS,
        "a default session sits under the guard threshold, so its list is untouched"
    );

    let all_fixtures = fixtures();
    assert_eq!(all_fixtures.len(), 20, "the report runs 20 fixtures");
    let (mut flat_med, mut guarded_med, mut guard_us) = (Vec::new(), Vec::new(), Vec::new());
    let mut lost = Vec::new();
    let mut direct = 0;
    let mut worst = (0_u128, "", 0_u128, 0_u128);
    for fixture in &all_fixtures {
        // Both arms run the same wide session state: the guardrail's own cost
        // must not be confused with a session that loaded a different surface.
        let expected_server = server();
        load_every_pack(&expected_server);
        let expected = match flat(&expected_server, fixture) {
            Ok(answer) => answer,
            Err(e) => {
                lost.push(format!("{}: flat arm must answer: {e}", fixture.verb));
                continue;
            }
        };
        // One throwaway run of each arm warms the code paths without touching
        // the servers the samples measure (some fixtures advance the spar).
        let warm = server();
        load_every_pack(&warm);
        let _ = guarded_call(&warm, &warm.tools(), fixture);
        let warm_flat = server();
        load_every_pack(&warm_flat);
        let _ = flat(&warm_flat, fixture);
        // Five reps per fixture, then one median per arm: the paired typical
        // cost. A raw p95 over 100 samples is just the slowest fixture's tail,
        // so the corpus percentile is taken over those medians below.
        let (mut flat_us, mut guarded_us) = (Vec::new(), Vec::new());
        for rep in 0..5 {
            let flat_server = server();
            load_every_pack(&flat_server);
            let start = Instant::now();
            let flat_answer = flat(&flat_server, fixture);
            flat_us.push(start.elapsed().as_micros());
            let guarded_server = server();
            load_every_pack(&guarded_server);
            let start = Instant::now();
            let guarded_tools = guarded_server.tools();
            guard_us.push(start.elapsed().as_micros());
            let start = Instant::now();
            let guarded_answer = guarded_call(&guarded_server, &guarded_tools, fixture);
            guarded_us.push(start.elapsed().as_micros());
            assert_eq!(
                flat_answer.as_ref(),
                Ok(&expected),
                "flat arm is deterministic"
            );
            if rep == 0 {
                if guarded_tools.iter().any(|tool| tool.name == fixture.verb) {
                    direct += 1;
                }
                if guarded_answer.as_ref() != Ok(&expected) {
                    lost.push(format!("{}: {:?}", fixture.verb, guarded_answer.err()));
                }
            }
        }
        let (flat_mid, guarded_mid) = (pct(&mut flat_us, 50), pct(&mut guarded_us, 50));
        if flat_mid >= 10 {
            let ratio = guarded_mid * 100 / flat_mid;
            if ratio > worst.0 {
                worst = (ratio, fixture.verb, flat_mid, guarded_mid);
            }
        }
        flat_med.push(flat_mid);
        guarded_med.push(guarded_mid);
    }
    let (flat_p50, flat_p95) = (pct(&mut flat_med, 50), pct(&mut flat_med, 95));
    let (guarded_p50, guarded_p95) = (pct(&mut guarded_med, 50), pct(&mut guarded_med, 95));
    let guard_p95 = p95(&mut guard_us);
    eprintln!(
        "API6 success: {}/20 ({direct} answered by an advertised name), per-fixture median \
         us flat p50 {flat_p50} p95 {flat_p95}, guarded p50 {guarded_p50} p95 {guarded_p95} \
         (worst fixture {} at {}% of flat: {} us vs {} us), guard plan p95 {guard_p95} us \
         once per session",
        20 - lost.len(),
        worst.1,
        worst.0,
        worst.3,
        worst.2
    );
    assert!(lost.is_empty(), "success lost on: {lost:#?}");
    assert!(
        guarded_p95 * 100 <= flat_p95 * 110,
        "guarded p95 {guarded_p95} us against flat {flat_p95} us is over the +10% bar"
    );
}

/// Load every shipped pack into a session, the wide surface's setup.
fn load_every_pack(server: &McpServer) {
    for pack in PackRegistry::shipped().packs() {
        typed::<ApiLoadTool>(server, json!({ "pack": pack.name() }))
            .expect("every shipped pack loads through api.load");
    }
}

/// The guarded arm: an advertised verb goes straight through; a hidden verb
/// goes through the advertised `api.call` by exact name.
fn guarded_call(
    server: &McpServer,
    advertised: &[rmcp::model::Tool],
    fixture: &Fixture,
) -> Result<Value, String> {
    if advertised.iter().any(|tool| tool.name == fixture.verb) {
        return flat(server, fixture);
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
