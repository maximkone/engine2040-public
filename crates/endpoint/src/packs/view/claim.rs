//! The view pack's claim lane (row AD21): `claim.check` takes an agent's own
//! claim about its work plus the receipt key the claim rests on and answers
//! whether the door's own records back it.
//!
//! Consumer: any agent's closing step and the AD25 scene-IQ harness, so a
//! report is checked against a receipt instead of trusted. The pure check is
//! [`crate::receipts::claim_check`]; this module only gathers its two inputs:
//! the key record from the session journal (an applied entry carries the
//! staged and read-back digests AD2 landed) and the read records the caller
//! names, each kept only when the door's own audit chain holds a dispatched
//! call of that tool at that revision. A read the door never ran backs
//! nothing, so an agent cannot mint its own evidence. A key the journal no
//! longer holds answers `unknown`, never `backed`.
//!
//! Units: revision numbers, digests as bytes, counts of checks and findings.
//! Determinism: the same journal, audit chain and args answer the same
//! verdict twice; no clock, no model, no allocation beyond one answer.
//! Errors: [`McpToolError`] `bad_args` for a claim word outside
//! `landed|confirmed|viewed`; every refusal names the missing proof plus one
//! live verb that produces it, so a weak model retries correctly.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use crate::audit::AuditOutcome;
use crate::mcp::{McpServer, McpToolError, ServerState};
use crate::receipts::{claim_check, Claim, ClaimVerdict, KeyRecord, ReadRecord, CLAIM_NOT_BACKED};

/// Verb: check one agent claim against the receipt it rests on.
pub const TOOL_CLAIM_CHECK: &str = "claim.check";

/// The claim words the verb accepts, in the order the description lists them.
pub const CLAIM_WORDS: [&str; 3] = ["landed", "confirmed", "viewed"];

/// Read verbs whose dispatched call at a revision proves a checked read: they
/// run the spatial checks and report their findings (AD3).
pub const CHECK_READ_TOOLS: [&str; 1] = ["scene.spatial"];

/// Read verbs whose dispatched call at a revision proves a capture or view
/// read at it.
pub const VIEW_READ_TOOLS: [&str; 3] = ["view.observe", "view.scene", "browser.capture"];

/// Verdict word for a claim the records back.
pub const BACKED: &str = "backed";

/// Verdict word for a key the journal no longer holds.
pub const UNKNOWN: &str = "unknown";

// ---------------------------------------------------------------------------
// claim.check
// ---------------------------------------------------------------------------

/// Args for `claim.check`: the claim word, the receipt key, and the reads the
/// claim rests on (the harness reports the counts it saw; the door checks each
/// one against its own audit chain).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ClaimCheckArgs {
    pub claim: String,
    pub key: u64,
    pub reads: Option<Vec<ClaimRead>>,
}

/// One read the claim rests on: the verb that read, the revision it read at,
/// how many checks ran, how many findings they raised, and whether it was a
/// capture or view read rather than a checked one.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ClaimRead {
    pub tool: String,
    pub revision: u64,
    pub checks_run: u32,
    pub findings: u32,
    pub view: bool,
}

/// `claim.check` answer: the verdict word, whether it is `backed`, the
/// missing proof plus the verb that produces it on a refusal, and the
/// revision the key landed at when the journal still holds it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ClaimCheckOutput {
    pub claim: String,
    pub key: u64,
    pub verdict: String,
    pub backed: bool,
    pub missing: Option<String>,
    pub next: Option<String>,
    pub revision: Option<u64>,
}

/// Parse one claim word; anything else refuses `bad_args` with the legal three
/// and one recovery.
fn parse_claim(word: &str) -> Result<Claim, McpToolError> {
    match word {
        "landed" => Ok(Claim::Landed),
        "confirmed" => Ok(Claim::Confirmed),
        "viewed" => Ok(Claim::Viewed),
        other => Err(McpToolError::new(
            "bad_args",
            format!(
                "claim `{other}` is not one of {}; re-run with one of those words",
                CLAIM_WORDS.join(", ")
            ),
        )),
    }
}

/// What the session journal holds for one receipt key: an applied entry's
/// result revision and its two digests, or `Unknown` when no entry carries
/// the key any more.
fn key_record(state: &ServerState, key: u64) -> KeyRecord {
    match state
        .journal
        .past
        .iter()
        .rev()
        .find(|entry| entry.key == key)
    {
        None => KeyRecord::Unknown,
        Some(entry) => KeyRecord::Applied {
            revision: entry.result_revision,
            staged: entry.staged_digest,
            after: entry.after_digest,
        },
    }
}

/// The read verbs that ground a claim of this kind. `landed` rests on the
/// apply receipt alone, so it grounds nothing here.
fn read_tools_for(claim: Claim) -> &'static [&'static str] {
    match claim {
        Claim::Landed => &[],
        Claim::Confirmed => &CHECK_READ_TOOLS,
        Claim::Viewed => &VIEW_READ_TOOLS,
    }
}

/// True when the door's audit chain holds a dispatched, successful call of
/// this tool at this revision: the read really happened there.
fn grounded(state: &ServerState, read: &ClaimRead) -> bool {
    state.audit.iter().any(|record| {
        record.outcome == AuditOutcome::Ok
            && record.revision == read.revision
            && record.tool == read.tool
    })
}

/// The reads that count: those naming a verb this claim kind accepts, a read
/// the door's audit chain grounds at that revision, and (for a view read) a
/// view flag that agrees with the verb it names. Everything else backs
/// nothing.
fn kept_reads(state: &ServerState, claim: Claim, reads: &[ClaimRead]) -> Vec<ReadRecord> {
    let allowed = read_tools_for(claim);
    reads
        .iter()
        .filter(|read| {
            let view_tool = VIEW_READ_TOOLS.contains(&read.tool.as_str());
            allowed.contains(&read.tool.as_str()) && read.view == view_tool && grounded(state, read)
        })
        .map(|read| ReadRecord {
            revision: read.revision,
            checks_run: read.checks_run,
            findings: read.findings,
            view: read.view,
        })
        .collect()
}

/// Serialize one verdict into the answer shape, naming no missing proof when
/// the claim is backed and no revision when the key is gone.
fn answer(
    claim: &str,
    key: u64,
    verdict: ClaimVerdict,
    landed_at: Option<u64>,
) -> ClaimCheckOutput {
    let (word, missing, next) = match verdict {
        ClaimVerdict::Backed => (BACKED, None, None),
        ClaimVerdict::Unknown => (UNKNOWN, None, None),
        ClaimVerdict::Refused { missing, next } => (
            CLAIM_NOT_BACKED,
            Some(missing.to_string()),
            Some(next.to_string()),
        ),
    };
    ClaimCheckOutput {
        claim: claim.to_owned(),
        key,
        backed: word == BACKED,
        verdict: word.to_owned(),
        missing,
        next,
        revision: landed_at,
    }
}

/// Gather the record and the grounded reads, then run the pure check. Read
/// only: nothing in the session moves.
pub(crate) fn claim_core(
    state: &ServerState,
    args: &ClaimCheckArgs,
) -> Result<ClaimCheckOutput, McpToolError> {
    let claim = parse_claim(&args.claim)?;
    let record = key_record(state, args.key);
    let landed_at = match record {
        KeyRecord::Applied { revision, .. } => Some(revision),
        KeyRecord::Unknown | KeyRecord::Preview => None,
    };
    let reads = match args.reads.as_deref() {
        None => Vec::new(),
        Some(read_args) => kept_reads(state, claim, read_args),
    };
    let verdict = claim_check(claim, record, &reads);
    Ok(answer(&args.claim, args.key, verdict, landed_at))
}

fn claim_check_verb(
    service: &McpServer,
    args: &ClaimCheckArgs,
) -> Result<ClaimCheckOutput, McpToolError> {
    service.dispatch(TOOL_CLAIM_CHECK, |state| claim_core(state, args))
}

crate::e2_tool!(
    /// `claim.check` tool: check one agent claim against its receipt. Read-only.
    ClaimCheckTool,
    name = TOOL_CLAIM_CHECK,
    description = "Check an agent's own claim about its work against the receipt it rests on. claim is landed, confirmed or viewed; key is the tx.apply key (its undo_token). landed needs an apply receipt whose read-back digest equals the staged one (a preview key never lands); confirmed needs a later checked read at that revision with checks run and no findings; viewed needs a capture or view read at it. A key the journal no longer holds answers unknown, never backed. Each read you name must be a call the door dispatched at that revision. Writes nothing.",
    readonly = true,
    args = ClaimCheckArgs,
    output = ClaimCheckOutput,
    invoke = claim_check_verb
);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use engine_editor::commands::{Batch, Document, Journal, Op};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;
    use rmcp::handler::server::router::tool::SyncTool;
    use rmcp::serde_json::json;

    use crate::packs::PackRegistry;

    /// A door session holding one applied insert under key 41 at revision 1.
    fn session() -> McpServer {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: ObjectId::new(),
                parent: ObjectId::NIL,
                local: Transform::IDENTITY,
            }],
            base_revision: doc.revision,
            idempotency_key: 41,
            provenance: None,
        };
        engine_editor::commands::apply(&mut doc, &mut journal, &batch).expect("the seed applies");
        McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
    }

    fn args(claim: &str, key: u64, reads: Vec<ClaimRead>) -> ClaimCheckArgs {
        ClaimCheckArgs {
            claim: claim.to_owned(),
            key,
            reads: Some(reads),
        }
    }

    fn read(tool: &str, revision: u64, checks_run: u32, findings: u32) -> ClaimRead {
        ClaimRead {
            tool: tool.to_owned(),
            revision,
            checks_run,
            findings,
            view: VIEW_READ_TOOLS.contains(&tool),
        }
    }

    /// Seal one successful audited call of `tool` at the current revision, so
    /// the door's own chain grounds a read without running the host verb. A
    /// manifest-scanned tool only admits through the screened seam; every
    /// other verb admits through the plain one, exactly as its own `invoke`
    /// reaches the door.
    fn seal_read(server: &McpServer, tool: &'static str) {
        match server.dispatch(tool, |_| Ok(())) {
            Ok(()) => {}
            Err(refusal) if refusal.code == "unscreened_call" => {
                server
                    .dispatch_screened(tool, &json!({}), |_| Ok(()))
                    .expect("the screened seam seals one Ok record");
            }
            Err(refusal) => panic!("the door seals one Ok record: {refusal:?}"),
        }
    }

    #[test]
    fn claim_check_verb_is_a_registered_read_verb() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_CLAIM_CHECK)
            .expect("claim.check is a registered verb");
        assert_eq!(verb.name(), "claim.check");
        assert!(verb.readonly());
        assert!(!PackRegistry::shipped().verb_names().is_empty());
        assert!(crate::tools::lookup("claim.check").is_none(), "not frozen");
        // The verb name carries no forbidden word and rides the view pack.
        assert_eq!(verb.pack(), super::super::PACK);
    }

    #[test]
    fn claim_check_verb_backs_a_landed_key_and_one_digest_byte_flips_it() {
        let server = session();
        let backed = ClaimCheckTool::invoke(&server, args("landed", 41, Vec::new()))
            .expect("the verb answers");
        assert_eq!(backed.verdict, BACKED, "{backed:?}");
        assert!(backed.backed);
        assert_eq!(backed.revision, Some(1));
        assert!(backed.missing.is_none() && backed.next.is_none());

        // One flipped byte in the read-back digest flips the answer.
        {
            let mut state = server.lock().expect("test lock holds");
            state.journal.past[0].after_digest[31] ^= 1;
        }
        let flipped = ClaimCheckTool::invoke(&server, args("landed", 41, Vec::new()))
            .expect("the verb answers");
        assert_eq!(flipped.verdict, CLAIM_NOT_BACKED, "{flipped:?}");
        assert!(!flipped.backed);
        assert!(flipped
            .missing
            .as_deref()
            .expect("the proof is named")
            .contains("read-back"));
        let next = flipped.next.as_deref().expect("a recovery verb");
        assert!(
            crate::tools::lookup(next).is_some(),
            "next `{next}` is a live verb"
        );
    }

    #[test]
    fn claim_check_verb_answers_unknown_for_a_key_the_journal_dropped() {
        let server = session();
        let out = ClaimCheckTool::invoke(&server, args("landed", 99, Vec::new()))
            .expect("the verb answers");
        assert_eq!(out.verdict, UNKNOWN, "{out:?}");
        assert!(!out.backed, "an unknown key is never backed");
        assert_eq!(out.revision, None);
    }

    #[test]
    fn claim_check_verb_confirmed_needs_a_grounded_checked_read() {
        let server = session();
        // No read: refused, and the missing proof is a checked read.
        let none = ClaimCheckTool::invoke(&server, args("confirmed", 41, Vec::new()))
            .expect("the verb answers");
        assert_eq!(none.verdict, CLAIM_NOT_BACKED, "{none:?}");

        // A read the door never dispatched at that revision backs nothing.
        let forged = ClaimCheckTool::invoke(
            &server,
            args("confirmed", 41, vec![read("scene.spatial", 1, 4, 0)]),
        )
        .expect("the verb answers");
        assert_eq!(forged.verdict, CLAIM_NOT_BACKED, "{forged:?}");

        // Once the door itself ran the checked read at that revision, backed.
        seal_read(&server, "scene.spatial");
        let backed = ClaimCheckTool::invoke(
            &server,
            args("confirmed", 41, vec![read("scene.spatial", 1, 4, 0)]),
        )
        .expect("the verb answers");
        assert_eq!(backed.verdict, BACKED, "{backed:?}");
    }

    #[test]
    fn claim_check_verb_viewed_needs_a_grounded_view_read() {
        let server = session();
        // A checked read at a view claim is refused: the claim needs a view.
        seal_read(&server, "scene.spatial");
        let checked = ClaimCheckTool::invoke(
            &server,
            args("viewed", 41, vec![read("scene.spatial", 1, 4, 0)]),
        )
        .expect("the verb answers");
        assert_eq!(checked.verdict, CLAIM_NOT_BACKED, "{checked:?}");
        assert!(checked.missing.as_deref().expect("named").contains("view"));

        seal_read(&server, "view.scene");
        let backed = ClaimCheckTool::invoke(
            &server,
            args("viewed", 41, vec![read("view.scene", 1, 0, 0)]),
        )
        .expect("the verb answers");
        assert_eq!(backed.verdict, BACKED, "{backed:?}");
    }

    #[test]
    fn claim_check_verb_refuses_an_unknown_claim_word() {
        let server = session();
        let error = ClaimCheckTool::invoke(&server, args("done", 41, Vec::new()))
            .expect_err("a word outside the three refuses");
        assert_eq!(error.code, "bad_args");
        for word in CLAIM_WORDS {
            assert!(error.message.contains(word), "names `{word}`");
        }
        assert!(error.message.contains("re-run"));
    }

    #[test]
    fn claim_check_verb_answers_the_same_verdict_twice() {
        let server = session();
        seal_read(&server, "scene.spatial");
        let call = || {
            ClaimCheckTool::invoke(
                &server,
                args("confirmed", 41, vec![read("scene.spatial", 1, 4, 0)]),
            )
            .expect("the verb answers")
        };
        assert_eq!(call(), call(), "the same records answer the same verdict");
    }

    #[test]
    fn claim_check_verb_reads_through_the_registry_dispatch_thunk() {
        let server = session();
        let out = PackRegistry::shipped()
            .verb(TOOL_CLAIM_CHECK)
            .expect("verb")
            .invoke(&server, json!({"claim": "landed", "key": 41, "reads": []}))
            .expect("the registry path answers");
        assert_eq!(
            out.get("verdict").and_then(|v| v.as_str()),
            Some(BACKED),
            "{out}"
        );
    }
}
