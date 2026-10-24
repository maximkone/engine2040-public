//! Door tests of `view.scene` and `view.steer` on fake hosts: a canned report in
//! the shape `arena --scene-report` prints, a fake seat and a fake window. Each
//! test changes one thing, so a break in the contract check, the line builder
//! or the answer mapping turns exactly its own test red.

use engine_editor::capability::Capability;
use rmcp::handler::server::router::tool::SyncTool;
use rmcp::serde_json::{json, Value};

use super::super::{check_frame, observe_core, ViewFrame};
use super::*;
use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
use crate::packs::gym::{SimResetArgs, SimResetTool, SimStepArgs, SimStepTool};
use crate::packs::PackRegistry;
use crate::tool_capabilities::CapabilityCard;

const SEED: u64 = 2;

fn server() -> McpServer {
    McpServer::new(7, Capability::unrestricted(7))
}

fn step(server: &McpServer, ticks: u32) {
    SimStepTool::invoke(
        server,
        SimStepArgs {
            seed: Some(SEED),
            participants: Some(2),
            half_extent_m: Some(20.0),
            ticks: Some(ticks),
            inputs: None,
            events: Some(8),
        },
    )
    .expect("step serves");
}

/// One fighter thing in the shape the arena prints.
fn fighter(slot: u8, ndc: Option<[f64; 4]>) -> Value {
    json!({
        "id": format!("fighter_{slot}"), "kind": "fighter", "label": format!("slot {slot}"),
        "solid": true, "drawn": true,
        "min": [-0.3, 0.0, -0.2], "max": [0.3, 1.8, 0.2], "depth_m": 9.0,
        "screen": ndc.map_or(Value::Null, |r| json!({"ndc": r, "px": [0, 0, 1, 1], "cover": 0.04})),
        "vis": {"visible": 1.0, "hidden": 0.0, "outside": 0.0, "hidden_by": []},
        "sun": {"lit": 0.5, "shaded": 0.0, "shaded_by": [], "map": "yes", "shadow_on_sand": Value::Null},
        "touch": [],
        "slot": slot, "act": "idle", "clip": 3, "tint": slot == 1, "ragdoll": false, "hp": 1000,
        "armed": "gladius", "root": [f64::from(slot) * 2.0, 0.0, 0.0], "yaw_deg": 0.0,
        "size": [0.6, 1.8, 0.4], "joints": {}, "floor": Value::Null, "hands": [],
        "slide_cm": {"l": 0.3, "r": Value::Null}, "error_parts": 0, "nonfinite": 0
    })
}

/// A contract-clean report for `fighters` fighters at `ticks`.
fn report(seed: u64, ticks: u32, fighters: u8, findings: &[&str]) -> Value {
    let things: Vec<Value> = (0..fighters)
        .filter(|_| ticks > 0)
        .map(|slot| fighter(slot, (slot == 0).then_some([-0.2, -0.4, 0.2, 0.4])))
        .collect();
    json!({
        "schema": 1, "source": "headless", "tick": ticks, "seed": seed.to_string(),
        "viewport": [1280, 720],
        "camera": {"eye": [0.0, 5.0, 12.0], "target": [0.0, 1.0, 0.0], "up": [0.0, 1.0, 0.0],
                   "fov_deg": 50.0, "aspect": 1.778, "near": 0.1, "far": 200.0,
                   "forward": [0.0, -0.3, -0.95], "heading_deg": 180.0, "pitch_deg": -18.0,
                   "shot": "action", "mode": "director"},
        "light": {"sun_dir": [-0.4, 0.7, 0.5], "sun_elev_deg": 45.0},
        "things": things, "pixels": Value::Null,
        "laws": {"line": if ticks == 0 { "" } else { "arena view: frames=120 bodies=240" },
                 "verdict": "ok", "warnings": []},
        "findings": findings,
        "text": ["scene tick line", "camera line"]
    })
}

fn bout_host(
    seed: u64,
    ticks: u32,
    fighters: u8,
    _showcase: bool,
    _shot: Option<&str>,
) -> Result<String, McpToolError> {
    Ok(report(seed, ticks, fighters, &["fighter_1 is outside the picture"]).to_string())
}

fn seat_host(_pixels: bool) -> Result<String, McpToolError> {
    let mut live = report(7, 55, 2, &[]);
    live["source"] = json!("window");
    Ok(live.to_string())
}

fn no_seat_host(_pixels: bool) -> Result<String, McpToolError> {
    Err(McpToolError::new(
        "no_seat",
        "no live seat; call seat.open first",
    ))
}

fn scene(server: &McpServer, args: &ViewSceneArgs) -> Result<ViewSceneOutput, McpToolError> {
    let state = server.lock().expect("test lock holds");
    scene_core(&state.play, args, false, no_seat_host, bout_host)
}

#[test]
fn view_scene_names_every_fighter_after_120_ticks_and_repeats_its_bytes() {
    let first = {
        let session = server();
        step(&session, 120);
        scene(&session, &ViewSceneArgs::default()).expect("view.scene serves")
    };
    assert_eq!(first.source, "bout");
    assert_eq!((first.tick, first.seed.as_str()), (120, "2"));
    assert_eq!(first.check, CheckKind::ReadOnly.name());
    assert_eq!(first.findings, ["fighter_1 is outside the picture"]);
    assert_eq!(first.text.as_deref(), Some("scene tick line\ncamera line"));
    let body = first.report.as_ref().expect("the report rides along");
    assert!(
        body.get("text").is_none(),
        "the text lines are lifted out of the report"
    );
    let ids: Vec<&str> = body["things"]
        .as_array()
        .expect("things")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        ["fighter_0", "fighter_1"],
        "every live fighter, once, in slot order"
    );
    let second = {
        let session = server();
        step(&session, 120);
        scene(&session, &ViewSceneArgs::default()).expect("view.scene serves")
    };
    assert_eq!(
        rmcp::serde_json::to_string(&first).expect("serializes"),
        rmcp::serde_json::to_string(&second).expect("serializes"),
        "one tape twice gives the same bytes"
    );
}

#[test]
fn format_picks_what_the_answer_carries() {
    let session = server();
    step(&session, 30);
    let json_only = scene(
        &session,
        &ViewSceneArgs {
            format: Some("json".to_owned()),
            ..Default::default()
        },
    )
    .expect("json");
    assert!(json_only.report.is_some() && json_only.text.is_none());
    let text_only = scene(
        &session,
        &ViewSceneArgs {
            format: Some("text".to_owned()),
            ..Default::default()
        },
    )
    .expect("text");
    assert!(text_only.report.is_none() && text_only.text.is_some());
    assert_eq!(
        text_only.findings, json_only.findings,
        "findings ride every format"
    );
    let both = scene(
        &session,
        &ViewSceneArgs {
            format: Some("both".to_owned()),
            ..Default::default()
        },
    )
    .expect("both");
    assert!(both.report.is_some() && both.text.is_some());
}

#[test]
fn view_scene_refuses_without_a_bout_and_on_bad_arguments() {
    let fresh = server();
    assert_eq!(
        scene(&fresh, &ViewSceneArgs::default())
            .expect_err("no bout")
            .code,
        "no_live_bout"
    );
    SimResetTool::invoke(
        &fresh,
        SimResetArgs {
            seed: Some(SEED),
            participants: Some(2),
            half_extent_m: Some(20.0),
        },
    )
    .expect("reset serves");
    let tick0 = scene(&fresh, &ViewSceneArgs::default()).expect("a reset bout reports tick 0");
    assert_eq!(tick0.tick, 0);
    assert_eq!(
        tick0.report.expect("report")["things"],
        json!([]),
        "nobody is drawn before tick 1"
    );
    for (args, why) in [
        (
            ViewSceneArgs {
                source: Some("camera".to_owned()),
                ..Default::default()
            },
            "source `camera`",
        ),
        (
            ViewSceneArgs {
                format: Some("xml".to_owned()),
                ..Default::default()
            },
            "format `xml`",
        ),
        (
            ViewSceneArgs {
                shot: Some("top".to_owned()),
                ..Default::default()
            },
            "shot `top`",
        ),
        (
            ViewSceneArgs {
                source: Some("seat".to_owned()),
                showcase: Some(true),
                ..Default::default()
            },
            "windowless bout",
        ),
        (
            ViewSceneArgs {
                source: Some("bout".to_owned()),
                pixels: Some(true),
                ..Default::default()
            },
            "no id buffer",
        ),
    ] {
        let err = scene(&fresh, &args).expect_err(why);
        assert_eq!(err.code, "bad_args", "{why}");
        assert!(
            err.message.contains(why.split(' ').next().unwrap_or(why)),
            "{why}: {}",
            err.message
        );
    }
}

#[test]
fn a_live_seat_wins_auto_and_a_missing_seat_refuses_no_seat() {
    let session = server();
    let state = session.lock().expect("test lock holds");
    // No bout is needed for the seat: the window is the scene.
    let live = scene_core(
        &state.play,
        &ViewSceneArgs::default(),
        true,
        seat_host,
        bout_host,
    )
    .expect("the seat answers");
    assert_eq!((live.source.as_str(), live.tick), ("seat", 55));
    assert_eq!(live.report.expect("report")["source"], "window");
    let chosen = ViewSceneArgs {
        source: Some("seat".to_owned()),
        ..Default::default()
    };
    let err =
        scene_core(&state.play, &chosen, false, no_seat_host, bout_host).expect_err("no seat");
    assert_eq!(err.code, "no_seat");
    let bout = ViewSceneArgs {
        source: Some("bout".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        scene_core(&state.play, &bout, true, seat_host, bout_host)
            .expect_err("no bout yet")
            .code,
        "no_live_bout",
        "source bout ignores an open seat"
    );
}

#[test]
fn a_host_report_outside_the_contract_is_refused_bad_view_never_passed_on() {
    fn not_json(_: u64, _: u32, _: u8, _: bool, _: Option<&str>) -> Result<String, McpToolError> {
        Ok("arena says hi".to_owned())
    }
    fn wrong_schema(
        s: u64,
        t: u32,
        f: u8,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["schema"] = json!(2);
        Ok(r.to_string())
    }
    fn missing_fighter(
        s: u64,
        t: u32,
        f: u8,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["things"].as_array_mut().expect("things").pop();
        Ok(r.to_string())
    }
    fn twin_fighter(
        s: u64,
        t: u32,
        f: u8,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["things"][1]["slot"] = json!(0);
        Ok(r.to_string())
    }
    fn rect_outside(
        s: u64,
        t: u32,
        f: u8,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["things"][0]["screen"]["ndc"] = json!([-0.2, -0.4, 1.5, 0.4]);
        Ok(r.to_string())
    }
    fn rect_flipped(
        s: u64,
        t: u32,
        f: u8,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["things"][0]["screen"]["ndc"] = json!([0.2, -0.4, -0.2, 0.4]);
        Ok(r.to_string())
    }
    fn share_over(s: u64, t: u32, f: u8, _: bool, _: Option<&str>) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["things"][0]["vis"]["hidden"] = json!(1.4);
        Ok(r.to_string())
    }
    fn stale_tick(s: u64, t: u32, f: u8, _: bool, _: Option<&str>) -> Result<String, McpToolError> {
        Ok(report(s, t + 1, f, &[]).to_string())
    }
    fn no_camera(s: u64, t: u32, f: u8, _: bool, _: Option<&str>) -> Result<String, McpToolError> {
        let mut r = report(s, t, f, &[]);
        r["camera"]["eye"] = json!([0.0, 1.0]);
        Ok(r.to_string())
    }
    let session = server();
    step(&session, 120);
    let state = session.lock().expect("test lock holds");
    let hosts: [(&str, SceneBoutFn); 9] = [
        ("not JSON", not_json),
        ("schema", wrong_schema),
        ("slot 1 is missing", missing_fighter),
        ("appears twice", twin_fighter),
        ("leaves -1..=1", rect_outside),
        ("not ordered", rect_flipped),
        ("not in 0..=1", share_over),
        ("is not the live bout's", stale_tick),
        ("camera.eye", no_camera),
    ];
    for (why, host) in hosts {
        let err = scene_core(
            &state.play,
            &ViewSceneArgs::default(),
            false,
            no_seat_host,
            host,
        )
        .expect_err(why);
        assert_eq!(err.code, "bad_view", "{why}");
        assert!(err.message.contains(why), "{why}: {}", err.message);
    }
}

#[test]
fn the_arena_stdout_parser_takes_the_prefixed_line_and_names_a_missing_binary() {
    let stdout =
        "arena notice: boot\r\narena scene-report: {\"schema\":1}\r\narena scene-text| tick 1\r\n";
    assert_eq!(
        report_from_stdout(stdout).as_deref(),
        Some("{\"schema\":1}")
    );
    assert_eq!(report_from_stdout("arena notice: nothing"), None);
    assert_eq!(error_tail(b"one\n\ntwo \n\n"), "two");
    assert_eq!(error_tail(b""), "no stderr");
    let err = arena_scene_report_from(Path::new("target/none/arena.exe"), 2, 10, 2, false, None)
        .expect_err("no binary");
    assert_eq!(err.code, "arena_exe_missing");
    assert!(err.message.contains("target/none/arena.exe") && err.message.contains(ARENA_EXE_ENV));
}

fn view_host(seed: u64, ticks: u32, fighters: u8) -> Result<ViewFrame, McpToolError> {
    scene_view_frame_with(bout_host, seed, ticks, fighters)
}

#[test]
fn view_observe_reads_the_scene_report_through_the_shipped_view_host() {
    let session = server();
    step(&session, 120);
    let state = session.lock().expect("test lock holds");
    let seen =
        observe_core(&state.play, Some(view_host)).expect("view.observe serves from the report");
    assert_eq!(seen.tick, 120);
    let slots: Vec<u8> = seen.bodies.iter().map(|b| b.slot).collect();
    assert_eq!(slots, [0, 1], "every live fighter, once");
    assert!(seen.bodies[0].screen.is_some() && seen.bodies[1].screen.is_none());
    assert_eq!(
        seen.laws, "arena view: frames=120 bodies=240",
        "the laws line passes unchanged"
    );
    assert!((seen.camera.fov_deg - 50.0).abs() < 1e-4);
    let frame = view_host(SEED, 120, 2).expect("host frame");
    check_frame(&frame, 120, 2).expect("the mapped frame meets the observe contract");
    assert_eq!(
        view_host(SEED, 120, 2).expect("again"),
        frame,
        "one bout, one frame"
    );
}

#[test]
fn view_scene_and_view_steer_register_in_the_pack_and_api_describe_reads_them() {
    let registry = PackRegistry::shipped();
    let pack = registry.pack(super::super::PACK).expect("view pack");
    for (name, needle, readonly) in [
        (TOOL_VIEW_SCENE, "Spatial scene report", true),
        (TOOL_VIEW_STEER, "Steer the live window", false),
    ] {
        let verb = registry.verb(name).expect("the scene verb registers");
        assert_eq!(verb.pack(), super::super::PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert_eq!(verb.readonly(), readonly, "{name}");
        assert!(
            pack.verb_names().contains(&name),
            "{name} lists in the pack"
        );
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: name.to_owned(),
            },
        )
        .expect("api.describe reads the verb");
        assert_eq!(described.path, name);
        assert!(described.docs.contains(needle), "{}", described.docs);
    }
}

// ---- view.steer -------------------------------------------------------------

fn args(op: &str) -> ViewSteerArgs {
    ViewSteerArgs {
        op: op.to_owned(),
        ..Default::default()
    }
}

#[test]
fn every_steer_op_builds_its_seat_line() {
    let cases: [(ViewSteerArgs, &str); 12] = [
        (
            ViewSteerArgs {
                shot: Some("top".to_owned()),
                ..args("shot")
            },
            "steer shot top",
        ),
        (
            ViewSteerArgs {
                shot: Some("close:1".to_owned()),
                ..args("shot")
            },
            "steer shot close:1",
        ),
        (
            ViewSteerArgs {
                eye: Some([0.0, 3.0, 8.5]),
                target: Some([0.0, 1.0, 0.0]),
                fov_deg: Some(35.0),
                ..args("pose")
            },
            "steer pose 0 3 8.5 0 1 0 35",
        ),
        (
            ViewSteerArgs {
                eye: Some([1.0, 2.0, 3.0]),
                target: Some([4.0, 5.0, 6.0]),
                ..args("pose")
            },
            "steer pose 1 2 3 4 5 6",
        ),
        (
            ViewSteerArgs {
                name: Some("sun_shadow".to_owned()),
                on: Some(false),
                ..args("pass")
            },
            "steer pass sun_shadow off",
        ),
        (
            ViewSteerArgs {
                name: Some("fog".to_owned()),
                on: Some(true),
                ..args("pass")
            },
            "steer pass fog on",
        ),
        (args("capture"), "steer capture"),
        (
            ViewSteerArgs {
                tag: Some("sag-1".to_owned()),
                ..args("capture")
            },
            "steer capture sag-1",
        ),
        (args("hold"), "steer hold"),
        (args("resume"), "steer resume"),
        (args("step"), "steer step"),
        (args("undo"), "steer undo"),
    ];
    for (given, line) in cases {
        assert_eq!(steer_line(&given).as_deref(), Ok(line), "{given:?}");
    }
    assert_eq!(steer_line(&args("state")).as_deref(), Ok("steer state"));
}

#[test]
fn a_bad_steer_request_is_refused_before_anything_is_sent() {
    for (given, why) in [
        (args("jump"), "op `jump`"),
        (args("shot"), "shot is required"),
        (
            ViewSteerArgs {
                shot: Some("a b".to_owned()),
                ..args("shot")
            },
            "letters, digits",
        ),
        (
            ViewSteerArgs {
                eye: Some([0.0; 3]),
                ..args("pose")
            },
            "target is required",
        ),
        (
            ViewSteerArgs {
                eye: Some([f32::NAN, 0.0, 0.0]),
                target: Some([0.0; 3]),
                ..args("pose")
            },
            "eye is not finite",
        ),
        (
            ViewSteerArgs {
                name: Some("fog".to_owned()),
                ..args("pass")
            },
            "on is required",
        ),
        (
            ViewSteerArgs {
                on: Some(true),
                ..args("pass")
            },
            "name is required",
        ),
        (
            ViewSteerArgs {
                tag: Some("../escape".to_owned()),
                ..args("capture")
            },
            "letters, digits",
        ),
        (
            ViewSteerArgs {
                tag: Some("t".repeat(40)),
                ..args("capture")
            },
            "1..=32",
        ),
    ] {
        let err = steer_line(&given).expect_err(why);
        assert_eq!(err.code, "bad_args", "{why}");
        assert!(err.message.contains(why), "{why}: {}", err.message);
    }
}

fn window_ok(line: &str, prefix: &str) -> Result<String, McpToolError> {
    assert_eq!(prefix, "steer ");
    let op = line.split_whitespace().nth(1).unwrap_or("?");
    Ok(json!({"ok": true, "op": op, "tick": 77, "pin": "top", "camera": {"eye": [0.0, 12.0, 0.0]}}).to_string())
}

fn window_refuses(_line: &str, _prefix: &str) -> Result<String, McpToolError> {
    Ok(
        json!({"ok": false, "op": "pass", "code": "bad_pass", "why": "no render pass `shadows`"})
            .to_string(),
    )
}

fn window_garbage(_line: &str, _prefix: &str) -> Result<String, McpToolError> {
    Ok("steer is fine".to_owned())
}

fn window_gone(_line: &str, _prefix: &str) -> Result<String, McpToolError> {
    Err(McpToolError::new(
        "no_seat",
        "no live seat; call seat.open first",
    ))
}

#[test]
fn the_window_answer_becomes_the_result_and_its_refusal_a_typed_error() {
    let given = ViewSteerArgs {
        shot: Some("top".to_owned()),
        ..args("shot")
    };
    let out = steer_core(&given, window_ok).expect("steer serves");
    assert_eq!((out.op.as_str(), out.tick), ("shot", 77));
    assert_eq!(
        out.result,
        json!({"pin": "top", "camera": {"eye": [0.0, 12.0, 0.0]}}),
        "ok/op/tick are lifted out"
    );
    let pass = ViewSteerArgs {
        name: Some("shadows".to_owned()),
        on: Some(false),
        ..args("pass")
    };
    let refused = steer_core(&pass, window_refuses).expect_err("the window refused");
    assert_eq!(refused.code, "bad_args");
    assert!(
        refused.message.contains("bad_pass")
            && refused.message.contains("no render pass `shadows`"),
        "{}",
        refused.message
    );
    assert_eq!(
        steer_core(&args("state"), window_garbage)
            .expect_err("not json")
            .code,
        "bad_view"
    );
    assert_eq!(
        steer_core(&args("state"), window_gone)
            .expect_err("no seat")
            .code,
        "no_seat"
    );
    assert_eq!(
        steer_core(&args("fly"), window_ok)
            .expect_err("bad op")
            .code,
        "bad_args"
    );
}
