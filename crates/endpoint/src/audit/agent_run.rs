//! Agent-loop consumer: one scripted task drives the door and the log keeps
//! the args.
//!
//! Consumer: the EV1 harness and any test that wants a real plan-act run over
//! the in-process door instead of a canned table. [`AuditDoor`] implements
//! `engine_provider::agent_loop::Door` over a live [`McpServer`]: `scene.read`
//! and `tx.preview` take the shared read path, `tx.apply` the exclusive write
//! path, and every call appends one [`LoopCall`] to the [`DoorAudit`] chain
//! with its args, args digest and the document revision on both sides.
//! [`replay`] runs the recorded `(tool, args)` pairs in order on a fresh door,
//! so it reproduces the same writes at the same final revision.
//! Units: revisions and byte lengths. Determinism: no clock, no thread order;
//! the same script on the same document seals the same chain and the same
//! final revision. Errors: [`LoopError`] from the loop, with `args_too_large`
//! reported as a refusal of the tool that asked.

use engine_provider::agent_loop::{estimate_tokens, Door, LoopError};

use super::{AuditOutcome, DoorAudit, LoopCall};
use crate::mcp::{
    McpServer, SceneReadArgs, TxApplyArgs, TxPreviewArgs, TxUndoArgs, TOOL_SCENE_READ,
    TOOL_TX_APPLY, TOOL_TX_PREVIEW, TOOL_TX_UNDO,
};

/// The door verbs this adapter serves, in a stable order.
pub const AUDIT_DOOR_TOOLS: [&str; 3] = [TOOL_SCENE_READ, TOOL_TX_PREVIEW, TOOL_TX_APPLY];

/// The writer stamp a scripted run puts on its writes: `script:` is the origin
/// form the door accepts for non-model writers.
pub const SCRIPT_ORIGIN: &str = "script:agent-loop";

/// One live door behind the loop: runs the verb, then logs the call.
pub struct AuditDoor<'a> {
    server: &'a McpServer,
    session: u64,
    audit: DoorAudit,
    /// `(key, observation)` for calls already served, so a repeated key
    /// replays its answer instead of running again.
    served: Vec<(u64, String)>,
}

impl<'a> AuditDoor<'a> {
    /// An empty log over `server` for `session`.
    pub fn new(server: &'a McpServer, session: u64) -> Self {
        Self {
            server,
            session,
            audit: DoorAudit::new(),
            served: Vec::new(),
        }
    }

    /// The chain this run wrote.
    pub fn audit(&self) -> &DoorAudit {
        &self.audit
    }

    /// The chain, taken by value when the run is over.
    pub fn into_audit(self) -> DoorAudit {
        self.audit
    }

    /// Run one verb and log it; the observation is the verb's own JSON.
    fn call(&mut self, tool: &str, input: &str, key: u64) -> Result<String, LoopError> {
        if let Some((_, observation)) = self.served.iter().find(|(served, _)| *served == key) {
            return Ok(observation.clone());
        }
        // The args bound is checked before the verb runs: an over-bound call
        // is refused by name and never logged, so the log can never disagree
        // with what ran.
        if let Err(error) = crate::audit::check_args(input) {
            return Err(LoopError::ToolRefused {
                tool: tool.to_owned(),
                reason: error.code().to_owned(),
            });
        }
        let before = self.revision()?;
        let answer = match tool {
            TOOL_SCENE_READ => self.run_read(input),
            TOOL_TX_PREVIEW => self.run_preview(input),
            TOOL_TX_APPLY => self.run_apply(input),
            _ => {
                return Err(LoopError::UnknownTool {
                    tool: tool.to_owned(),
                })
            }
        };
        let after = self.revision()?;
        let outcome = match &answer {
            Ok(_) => AuditOutcome::Ok,
            Err(_) => AuditOutcome::Refused,
        };
        self.audit
            .append_call(self.session, tool, input, before, after, outcome)
            .map_err(|error| LoopError::ToolRefused {
                tool: tool.to_owned(),
                reason: error.code().to_owned(),
            })?;
        match answer {
            Ok(observation) => {
                self.served.push((key, observation.clone()));
                Ok(observation)
            }
            Err(refusal) => Err(LoopError::ToolRefused {
                tool: tool.to_owned(),
                reason: refusal.code().to_owned(),
            }),
        }
    }

    /// Current document revision.
    fn revision(&self) -> Result<u64, LoopError> {
        let state = self
            .server
            .lock_read()
            .map_err(|error| LoopError::ToolRefused {
                tool: String::from("scene.read"),
                reason: error.code.to_owned(),
            })?;
        Ok(state.doc.revision)
    }

    fn run_read(&self, input: &str) -> Result<String, Refusal> {
        let args: SceneReadArgs = parse_args(input)?;
        let answer = self
            .server
            .dispatch_read(TOOL_SCENE_READ, &args, |state| state.scene_read(&args))
            .map_err(|error| Refusal::Door(error.code))?;
        to_json(&answer)
    }

    fn run_preview(&self, input: &str) -> Result<String, Refusal> {
        let args: TxPreviewArgs = parse_args(input)?;
        let answer = self
            .server
            .dispatch_read(TOOL_TX_PREVIEW, &args, |state| state.tx_preview(&args))
            .map_err(|error| Refusal::Door(error.code))?;
        to_json(&answer)
    }

    fn run_apply(&self, input: &str) -> Result<String, Refusal> {
        let args: TxApplyArgs = parse_args(input)?;
        let answer = self
            .server
            .dispatch_screened(TOOL_TX_APPLY, &args, |state| state.tx_apply(&args))
            .map_err(|error| Refusal::Door(error.code))?;
        to_json(&answer)
    }
}

impl Door for AuditDoor<'_> {
    fn tool_names(&self) -> Vec<String> {
        AUDIT_DOOR_TOOLS
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect()
    }

    fn cost_of(&self, tool: &str) -> u64 {
        u64::from(estimate_tokens(tool))
    }

    fn act(&mut self, tool: &str, input: &str, key: u64) -> Result<String, LoopError> {
        self.call(tool, input, key)
    }

    fn undo(&mut self, key: u64) -> bool {
        let args = TxUndoArgs {
            undo_token: Some(key),
        };
        self.server
            .dispatch_screened(TOOL_TX_UNDO, &args, |state| state.tx_undo(&args))
            .is_ok()
    }
}

/// Replay a recorded run: the same tools with the same args, in order, on
/// `server`. Keys are fresh (`seq`), because the tool-level idempotency rides
/// inside the args: a replay is a pure log replay of what the run did.
/// Errors: [`LoopError`] when a recorded call no longer runs the same way.
pub fn replay(server: &McpServer, log: &DoorAudit, session: u64) -> Result<DoorAudit, LoopError> {
    let mut door = AuditDoor::new(server, session);
    let calls: Vec<LoopCall> = log.loop_calls().cloned().collect();
    for call in calls {
        door.act(&call.record.tool, &call.args, call.record.seq)?;
    }
    Ok(door.into_audit())
}

fn parse_args<T: rmcp::serde::de::DeserializeOwned>(input: &str) -> Result<T, Refusal> {
    rmcp::serde_json::from_str(input).map_err(|_| Refusal::BadArgs)
}

/// Why one door verb did not answer: its own refusal code, or an input or an
/// answer that does not parse. Typed so a caller branches on the case, never
/// on prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// The input is not that verb's args JSON.
    BadArgs,
    /// The verb answered something that does not serialize.
    BadAnswer,
    /// The door refused the call; carries its code.
    Door(&'static str),
}

impl Refusal {
    /// Stable machine code for the loop's refusal reason.
    fn code(self) -> &'static str {
        match self {
            Self::BadArgs => "bad_args",
            Self::BadAnswer => "bad_answer",
            Self::Door(code) => code,
        }
    }
}

fn to_json<T: rmcp::serde::Serialize>(value: &T) -> Result<String, Refusal> {
    rmcp::serde_json::to_string(value).map_err(|_| Refusal::BadAnswer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{args_digest, MAX_ARGS_BYTES};
    use crate::door_dispatch::witness_digest_for;
    use engine_editor::capability::Capability;
    use engine_provider::agent_loop::{AgentLoop, LoopBudget, Plan, StepRecord};
    use rmcp::serde_json::{self, Value};

    /// One fresh insert: a root object at the origin, identity pose.
    const OPS: &str = concat!(
        r#"[{"op":"insert","id":"8f3a2b1c-0000-4000-8000-0000000000aa","#,
        r#""parent":"00000000-0000-0000-0000-000000000000","#,
        r#""local":{"pos":[0.0,0.0,0.0],"rot":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]}}]"#
    );

    fn door() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// The scripted task: read the scene, preview one insert, apply it with the
    /// approval the preview minted, then report done.
    fn script(transcript: &[StepRecord]) -> Result<Plan, LoopError> {
        Ok(match transcript.len() {
            0 => Plan::ToolCall {
                tool: TOOL_SCENE_READ.to_owned(),
                input: String::from("{}"),
                idempotency_key: 1,
            },
            1 => Plan::ToolCall {
                tool: TOOL_TX_PREVIEW.to_owned(),
                input: format!(r#"{{"expected_revision":0,"idempotency_key":"2","ops":{OPS}}}"#),
                idempotency_key: 2,
            },
            2 => {
                let preview: Value =
                    serde_json::from_str(&transcript[1].observation).map_err(|_| {
                        LoopError::ToolRefused {
                            tool: TOOL_TX_PREVIEW.to_owned(),
                            reason: String::from("bad_answer"),
                        }
                    })?;
                let approval =
                    preview
                        .get("approval")
                        .cloned()
                        .ok_or_else(|| LoopError::ToolRefused {
                            tool: TOOL_TX_PREVIEW.to_owned(),
                            reason: String::from("no_approval"),
                        })?;
                Plan::ToolCall {
                    tool: TOOL_TX_APPLY.to_owned(),
                    input: format!(
                        r#"{{"expected_revision":0,"idempotency_key":"2","ops":{OPS},"approval":{approval},"origin":"{SCRIPT_ORIGIN}"}}"#
                    ),
                    idempotency_key: 3,
                }
            }
            _ => Plan::Done {
                answer: String::from("one prop placed"),
            },
        })
    }

    #[test]
    fn a_scripted_task_lists_previews_applies_and_replays_from_the_log() {
        let server = door();
        let mut run = AuditDoor::new(&server, 7);
        let outcome = AgentLoop::new(LoopBudget::default_budget())
            .run("place one prop", &mut script, &mut run)
            .expect("the scripted task finishes");
        assert_eq!(outcome.steps.len(), 3, "read, preview, apply");
        assert_eq!(server.lock().expect("lock").doc.revision, 1, "one write");

        // The log keeps every call with its args and both revisions.
        let log = run.into_audit();
        let calls: Vec<LoopCall> = log.loop_calls().cloned().collect();
        assert_eq!(calls.len(), 3, "one record per call");
        assert_eq!(calls[0].record.tool, TOOL_SCENE_READ);
        assert_eq!(calls[0].args, "{}");
        assert_eq!(calls[1].record.tool, TOOL_TX_PREVIEW);
        assert!(calls[1].args.contains("\"ops\""), "preview args kept");
        assert_eq!(calls[2].record.tool, TOOL_TX_APPLY);
        assert!(calls[2].args.contains(SCRIPT_ORIGIN), "apply args kept");
        assert_eq!(
            (calls[2].revision_before, calls[2].revision_after),
            (0, 1),
            "the write is bracketed by its revisions"
        );
        assert_eq!(
            (calls[0].revision_before, calls[0].revision_after),
            (0, 0),
            "a read moves nothing"
        );
        for call in &calls {
            assert_eq!(call.args_digest, args_digest(&call.args));
            assert_eq!(call.record.detail, call.args_digest, "the chain seals them");
        }
        log.verify().expect("the chain verifies");

        // A replay of the log on a fresh door reproduces the same write.
        let fresh = door();
        let replayed = super::replay(&fresh, &log, 7).expect("the log replays");
        assert_eq!(
            fresh.lock().expect("lock").doc.revision,
            1,
            "the replay reaches the recorded revision"
        );
        let fresh_digest = {
            let state = fresh.lock().expect("lock");
            witness_digest_for(&state.doc)
        };
        let run_digest = {
            let state = server.lock().expect("lock");
            witness_digest_for(&state.doc)
        };
        assert_eq!(
            fresh_digest, run_digest,
            "the replay lands the same document"
        );
        assert_eq!(
            replayed.loop_calls().count(),
            3,
            "the replay logs its own calls"
        );
    }

    #[test]
    fn args_over_the_bound_are_refused_by_name_before_anything_runs() {
        let big = "x".repeat(MAX_ARGS_BYTES + 1);
        let mut log = DoorAudit::new();
        let error = log
            .append_call(7, TOOL_SCENE_READ, &big, 0, 0, AuditOutcome::Ok)
            .expect_err("over-bound args are refused");
        assert_eq!(error.code(), "args_too_large");
        assert_eq!(log.total_appended(), 0, "a refused call never logs");

        let server = door();
        let mut run = AuditDoor::new(&server, 7);
        let refused = run
            .act(TOOL_SCENE_READ, &big, 1)
            .expect_err("the loop refuses the over-bound call");
        match refused {
            LoopError::ToolRefused { reason, .. } => assert_eq!(reason, "args_too_large"),
            other => panic!("expected a typed refusal, got {other:?}"),
        }
        assert_eq!(run.audit().total_appended(), 0, "nothing logged");
        assert_eq!(server.lock().expect("lock").doc.revision, 0, "nothing ran");
    }
}
