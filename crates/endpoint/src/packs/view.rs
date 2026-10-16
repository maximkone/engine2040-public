//! `view` pack: what the door sees about itself and the run.
//!
//! Holds the observe-log digest and pages, the self-describing manifest and
//! `look.judge`, which scores a capture set by machine so a Look row accepts
//! on a number instead of an opened PNG.
//!
//! `look.judge` follows the capture pattern: the endpoint owns no judge. A
//! host that links the judge registers it once with [`register_look_judge`];
//! until then the verb refuses as `unsupported_look_judge`, never a success
//! shape. The door checks the directory arguments before anything runs (a
//! relative path with no `..`) and checks the judge's answer against the
//! contract before it passes it on: schema 1, a 16-hex digest, nine rubric
//! items `L1`..`L9` in order with integer permille scores, no float anywhere.
//! Units: permille for scores, chars for path bounds. Determinism: the door
//! adds nothing to the judge's bytes. Errors: [`McpToolError`] with
//! `bad_args`, `unsupported_look_judge`, `bad_receipt` or the judge's own code.

use std::borrow::Cow;
use std::path::{Component, Path};
use std::sync::OnceLock;

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value};

use super::{PackDef, PackError, VerbSpec};
use crate::discover::{DiscoverTool, TOOL_RPC_DISCOVER};
use crate::log_tools::{LogDigestTool, LogQueryTool, TOOL_LOG_DIGEST, TOOL_LOG_QUERY};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "view";

/// Verb: score a capture set by machine.
pub const TOOL_LOOK_JUDGE: &str = "look.judge";

/// Rubric ids a judge answer must carry, in order.
pub const LOOK_RUBRIC_IDS: [&str; 9] = ["L1", "L2", "L3", "L4", "L5", "L6", "L7", "L8", "L9"];

/// Schema version a judge answer must carry.
pub const LOOK_JUDGE_SCHEMA: u64 = 1;

/// Longest directory argument in chars.
const MAX_DIR_CHARS: usize = 512;

/// The view pack: log digest, log pages, the manifest and the look judge,
/// loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "what the door sees: observe-log digest and pages, the self-describing manifest, \
         look.judge (machine scores for a capture set)",
        false,
    );
    pack.add(VerbSpec::of::<LogDigestTool>(
        TOOL_LOG_DIGEST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<LogQueryTool>(
        TOOL_LOG_QUERY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<DiscoverTool>(
        TOOL_RPC_DISCOVER,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<LookJudgeTool>(
        TOOL_LOOK_JUDGE,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the discovery budget is a gate.
// ---------------------------------------------------------------------------

/// Judge request: a capture directory (PNG frames plus an optional
/// `manifest.json`), optionally a fighter-preview directory (default
/// `content/fighters/previews`) and a Q1 reference directory. Units are
/// relative directory paths. Errors are typed `bad_args` for a path that is
/// absolute, climbs with `..` or is empty. Budget is 512 chars a path.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LookJudgeArgs {
    pub capture_dir: String,
    pub preview_dir: Option<String>,
    pub q1_dir: Option<String>,
}

/// Judge answer: schema, digest, verdict, mean score and the full report
/// (rubric L1..L9, per-frame scores, entity-grounded regions, sequence).
/// Units are permille. No errors once judged. Budget rides on the capture
/// set (at most 64 frames).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LookJudgeOutput {
    pub schema: u64,
    pub digest: String,
    pub pass: bool,
    pub score: u64,
    pub judgement: Value,
}

/// A registered judge: reads the directories named in the request and
/// returns the report as JSON text, or the refusal to pass on.
pub type LookJudgeFn = fn(&LookJudgeArgs) -> Result<String, McpToolError>;

static LOOK_JUDGE: OnceLock<LookJudgeFn> = OnceLock::new();

/// Register the judge for this process. The host that links the judge calls
/// this once at start; a second call refuses, so no later code swaps the
/// judge under a running door.
///
/// # Errors
///
/// `look_judge_registered` when a judge is already registered.
pub fn register_look_judge(judge: LookJudgeFn) -> Result<(), McpToolError> {
    LOOK_JUDGE.set(judge).map_err(|_| {
        McpToolError::new(
            "look_judge_registered",
            "a look judge is already registered for this process; \
             restart the host to register another",
        )
    })
}

fn bad_args(message: String) -> McpToolError {
    McpToolError::new("bad_args", message)
}

/// A directory argument must be a non-empty relative path of plain names.
fn check_dir(field: &str, raw: &str) -> Result<(), McpToolError> {
    let plain = !raw.is_empty()
        && raw.chars().count() <= MAX_DIR_CHARS
        && !raw.contains(['\0', '\\', ':'])
        && Path::new(raw)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if plain {
        return Ok(());
    }
    Err(bad_args(format!(
        "{field} `{}` is not a relative directory of plain names (1..={MAX_DIR_CHARS} chars, \
         `/` separators, no `..`, no drive, colon, backslash or leading slash); re-run \
         with a path such as target/reports/staged-fight/captures",
        raw.chars().take(80).collect::<String>()
    )))
}

fn check_args(args: &LookJudgeArgs) -> Result<(), McpToolError> {
    check_dir("capture_dir", &args.capture_dir)?;
    for (field, value) in [("preview_dir", &args.preview_dir), ("q1_dir", &args.q1_dir)] {
        if let Some(raw) = value {
            check_dir(field, raw)?;
        }
    }
    Ok(())
}

/// Unwired judge: structured unsupported, never a success shape.
fn unsupported() -> McpToolError {
    McpToolError::new(
        "unsupported_look_judge",
        "no look judge is registered: the endpoint owns no judge, so a host must call \
         register_look_judge; absent judge is unsupported, never success. Next: run the \
         judge from a host that links it, or read the frames with the capture verb",
    )
}

fn contract(why: &str) -> McpToolError {
    McpToolError::new(
        "bad_receipt",
        format!("the judge answered outside its contract ({why}); re-run after fixing the judge"),
    )
}

/// No float anywhere: every JSON number is an integer.
fn all_integers(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_i64() || n.is_u64(),
        Value::Array(items) => items.iter().all(all_integers),
        Value::Object(map) => map.values().all(all_integers),
        _ => true,
    }
}

fn permille_of(item: &Value, key: &str) -> Option<u64> {
    item.get(key)?.as_u64().filter(|score| *score <= 1000)
}

/// Check the judge's answer against the contract and lift the headline.
fn validate(text: &str) -> Result<LookJudgeOutput, McpToolError> {
    let judgement: Value =
        serde_json::from_str(text).map_err(|_| contract("the answer is not JSON"))?;
    if !all_integers(&judgement) {
        return Err(contract("a number is not an integer"));
    }
    let schema = judgement.get("schema").and_then(Value::as_u64);
    if schema != Some(LOOK_JUDGE_SCHEMA) {
        return Err(contract("schema is not 1"));
    }
    let digest = judgement
        .get("digest")
        .and_then(Value::as_str)
        .filter(|d| d.len() == 16 && d.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .ok_or_else(|| contract("digest is not 16 lowercase hex"))?
        .to_owned();
    let pass = judgement
        .get("pass")
        .and_then(Value::as_bool)
        .ok_or_else(|| contract("pass is missing"))?;
    let score =
        permille_of(&judgement, "score").ok_or_else(|| contract("score is not 0..=1000"))?;
    let rubric = judgement.get("rubric").and_then(Value::as_array);
    let ordered = rubric.is_some_and(|items| {
        items.len() == LOOK_RUBRIC_IDS.len()
            && items.iter().zip(LOOK_RUBRIC_IDS).all(|(item, id)| {
                item.get("id").and_then(Value::as_str) == Some(id)
                    && permille_of(item, "score").is_some()
                    && item.get("pass").is_some_and(Value::is_boolean)
            })
    });
    if !ordered {
        return Err(contract(
            "rubric is not L1..L9 in order with permille scores",
        ));
    }
    if !judgement
        .get("frames")
        .and_then(Value::as_array)
        .is_some_and(|frames| !frames.is_empty())
    {
        return Err(contract("frames is empty"));
    }
    Ok(LookJudgeOutput {
        schema: LOOK_JUDGE_SCHEMA,
        digest,
        pass,
        score,
        judgement,
    })
}

/// Run one request against an optional judge: arguments first, then the
/// refusal when no judge is wired, then the contract check on its answer.
fn run_look_judge(
    args: &LookJudgeArgs,
    judge: Option<LookJudgeFn>,
) -> Result<LookJudgeOutput, McpToolError> {
    check_args(args)?;
    let judge = judge.ok_or_else(unsupported)?;
    validate(&judge(args)?)
}

/// `look.judge` tool: machine rubric scores for a capture set, read-only.
pub struct LookJudgeTool;

impl ToolBase for LookJudgeTool {
    type Parameter = LookJudgeArgs;
    type Output = LookJudgeOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_LOOK_JUDGE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Machine look score for a capture directory: rubric L1-L9 from pixels, \
             frame checks and references, as integer permille JSON. Read-only; \
             unsupported until a judge is registered."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for LookJudgeTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_LOOK_JUDGE, |_state| {
            run_look_judge(&param, LOOK_JUDGE.get().copied())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn args(dir: &str) -> LookJudgeArgs {
        LookJudgeArgs {
            capture_dir: dir.to_owned(),
            preview_dir: None,
            q1_dir: None,
        }
    }

    /// A contract-clean answer: nine items, one frame, integers only.
    fn answer(edit: impl Fn(String) -> String) -> Result<String, McpToolError> {
        let items: Vec<String> = LOOK_RUBRIC_IDS
            .iter()
            .map(|id| format!("{{\"id\":\"{id}\",\"score\":500,\"pass\":false}}"))
            .collect();
        Ok(edit(format!(
            "{{\"schema\":1,\"digest\":\"00ff00ff00ff00ff\",\"pass\":false,\"score\":500,\
             \"rubric\":[{}],\"frames\":[{{\"name\":\"frame_000000.png\"}}]}}",
            items.join(",")
        )))
    }

    fn good(_: &LookJudgeArgs) -> Result<String, McpToolError> {
        answer(|text| text)
    }

    #[test]
    fn look_judge_view_pack_lists_the_verb_and_api_describe_reads_it() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_LOOK_JUDGE)
            .expect("look.judge is a registered verb");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        assert!(PackRegistry::shipped()
            .pack(PACK)
            .expect("view pack")
            .verb_names()
            .contains(&TOOL_LOOK_JUDGE));
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_LOOK_JUDGE.to_owned(),
            },
        )
        .expect("api.describe lists look.judge");
        assert_eq!(described.path, TOOL_LOOK_JUDGE);
        assert!(described.docs.contains("rubric L1-L9"));
        assert!(described.signature.contains("view"));
    }

    #[test]
    fn look_judge_refuses_by_default_until_a_judge_is_registered() {
        let error = LookJudgeTool::invoke(&server(), args("target/captures"))
            .expect_err("no judge is wired in the library tests");
        assert_eq!(error.code, "unsupported_look_judge");
        assert!(error.message.contains("never success"));
        let via_registry = PackRegistry::shipped()
            .verb(TOOL_LOOK_JUDGE)
            .expect("verb")
            .invoke(
                &server(),
                serde_json::json!({"capture_dir": "target/captures"}),
            )
            .expect_err("the registry path refuses the same way");
        assert_eq!(via_registry.code, "unsupported_look_judge");
    }

    #[test]
    fn look_judge_checks_paths_before_anything_runs() {
        let long = "a".repeat(MAX_DIR_CHARS + 1);
        for bad in [
            "",
            "..",
            "../up",
            "a/../../b",
            "/abs",
            "\\abs",
            "C:\\x",
            "C:x",
            "a\0b",
            &long,
        ] {
            let error = run_look_judge(&args(bad), Some(good)).expect_err(bad);
            assert_eq!(error.code, "bad_args", "{bad:?}");
            assert!(error.message.contains("re-run"));
        }
        for ok in [
            "target/captures",
            "./captures",
            "a",
            "content/fighters/previews",
        ] {
            assert!(run_look_judge(&args(ok), Some(good)).is_ok(), "{ok}");
        }
        let mut with_bad_q1 = args("target/captures");
        with_bad_q1.q1_dir = Some("../q1".to_owned());
        assert_eq!(
            run_look_judge(&with_bad_q1, Some(good))
                .expect_err("q1 escapes")
                .code,
            "bad_args"
        );
        let mut with_bad_previews = args("target/captures");
        with_bad_previews.preview_dir = Some("/previews".to_owned());
        assert!(run_look_judge(&with_bad_previews, None)
            .expect_err("previews")
            .message
            .contains("preview_dir"));
        // Bad args win over the unwired refusal.
        assert_eq!(
            run_look_judge(&args(".."), None).expect_err("bad").code,
            "bad_args"
        );
    }

    #[test]
    fn look_judge_wired_judge_answers_typed_json() {
        let out = run_look_judge(&args("target/captures"), Some(good)).expect("answers");
        assert_eq!((out.schema, out.pass, out.score), (1, false, 500));
        assert_eq!(out.digest, "00ff00ff00ff00ff");
        assert_eq!(out.judgement["rubric"].as_array().map(Vec::len), Some(9));
        assert_eq!(out.judgement["rubric"][8]["id"], "L9");
        let wire = serde_json::to_string(&out).expect("serializes");
        assert!(wire.contains("\"judgement\""));
    }

    #[test]
    fn look_judge_refuses_answers_outside_the_contract() {
        let bad = |edit: fn(String) -> String, why: &str| {
            let error = validate(&answer(edit).expect("fixture builds")).expect_err(why);
            assert_eq!(error.code, "bad_receipt", "{why}");
            assert!(error.message.contains(why), "{why}: {}", error.message);
        };
        bad(
            |t| t.replace("\"schema\":1", "\"schema\":2"),
            "schema is not 1",
        );
        bad(
            |t| t.replace("00ff00ff00ff00ff", "00FF00ff00ff00ff"),
            "digest is not 16 lowercase hex",
        );
        bad(
            |t| t.replace("00ff00ff00ff00ff", "00ff"),
            "digest is not 16 lowercase hex",
        );
        bad(
            |t| t.replace("\"score\":500,\"rubric\"", "\"score\":500.5,\"rubric\""),
            "a number is not an integer",
        );
        bad(
            |t| t.replace("\"score\":500,\"rubric\"", "\"score\":1001,\"rubric\""),
            "score is not 0..=1000",
        );
        bad(
            |t| t.replace("\"pass\":false,\"score\"", "\"score\""),
            "pass is missing",
        );
        bad(
            |t| t.replace("\"id\":\"L9\"", "\"id\":\"L8\""),
            "rubric is not L1..L9 in order",
        );
        bad(
            |t| {
                t.replace(
                    "{\"id\":\"L9\",\"score\":500,\"pass\":false}",
                    "{\"id\":\"L9\",\"score\":9000,\"pass\":false}",
                )
            },
            "rubric is not L1..L9 in order",
        );
        bad(
            |t| t.replace(",{\"id\":\"L9\",\"score\":500,\"pass\":false}", ""),
            "rubric is not L1..L9 in order",
        );
        bad(
            |t| t.replace("[{\"name\":\"frame_000000.png\"}]", "[]"),
            "frames is empty",
        );
        bad(|_| "not json".to_owned(), "the answer is not JSON");
        // A judge's own refusal passes through untouched.
        fn refusing(_: &LookJudgeArgs) -> Result<String, McpToolError> {
            Err(McpToolError::new(
                "no_frames",
                "capture set holds no frame; re-run",
            ))
        }
        let passed =
            run_look_judge(&args("target/empty"), Some(refusing)).expect_err("judge refuses");
        assert_eq!(
            (passed.code, passed.message.as_str()),
            ("no_frames", "capture set holds no frame; re-run")
        );
    }
}
