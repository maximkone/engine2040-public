//! Std-only root wiring beyond `tools`/`transport`/`guards`.
//!
//! Consumer: a headless caller linking `engine-endpoint` with default
//! features who reaches every std-only module through the crate root
//! (`engine_endpoint::audit`, `::brief`, `::canon`, ...). Data owner stays
//! each module; this file owns only the root re-export contract. Units:
//! chars, bytes, ticks, permille, digests. Determinism: integer and string
//! compares only, no clock, no IO, no network. Errors are panics via asserts.
//!
//! Why this file exists: `crates/endpoint/src/lib.rs` changed 32 times in
//! 30 days yet held zero `#[test]`, and `tests/it/lib_wiring_vectors.rs`
//! pins only `tools`, `transport` and `guards`, so dropping `pub mod audit`
//! or any other std-only line compiled the crate but broke every caller at
//! their `use` line. Each vector below names the root path, so a dropped
//! module or drifted budget fails here first.

use engine_endpoint::{
    assert_run, audit, brief, build_character, build_encounter, build_game, canon, injection,
    insight, receipts, sandbox, secret_launch, witness,
};

/// Every std-only `pub mod` in the crate root stays reachable with its
/// headline budget intact. Removing one module line fails the `use` above.
#[test]
fn std_root_pins_budgets_and_names() {
    assert_eq!(brief::BRIEF_MAX_CHARS, 4096, "brief bound via root");
    assert_eq!(brief::REPAIR_BUDGET_DEFAULT, 3, "level budget via root");
    assert_eq!(
        brief::REPAIR_BUDGET_ENCOUNTER,
        5,
        "encounter budget via root"
    );
    assert_eq!(canon::MAX_VARIANTS, 1024, "canon cap via root");
    assert_eq!(canon::RULE_COUNT, canon::RULES.len(), "rule count via root");
    assert_eq!(
        audit::DOOR_AUDIT_BUDGET_NS,
        500_000,
        "audit budget via root"
    );
    assert_eq!(audit::MAX_TOOL_NAME_LEN, 64, "tool name cap via root");
    assert_eq!(
        witness::WITNESS_DOMAIN,
        0xd02f_1177_1ce0_0029,
        "witness domain via root"
    );
    assert_eq!(witness::GOAL_MAX_BYTES, 512, "goal cap via root");
    assert_eq!(assert_run::MAX_ASSERTS_PER_RUN, 64, "assert cap via root");
    assert_eq!(assert_run::MAX_TAPE_ROWS, 4096, "tape cap via root");
    assert_eq!(
        secret_launch::MAX_VAULT_BYTES,
        64 * 1024,
        "vault cap via root"
    );
    assert_eq!(secret_launch::MAX_SECRETS, 64, "secret cap via root");
    assert_eq!(
        build_character::TOOL_BUILD_CHARACTER,
        "character.build",
        "character tool via root"
    );
    assert_eq!(build_character::EXPECTED_JOINTS, 45, "joint count via root");
    assert_eq!(build_character::MAX_PARTS, 16, "part cap via root");
    assert_eq!(
        build_encounter::MCP_TOOL_NAME,
        "sim.build_encounter",
        "encounter tool via root"
    );
    assert_eq!(build_encounter::WIN_MIN_PERMILLE, 400, "win floor via root");
    assert_eq!(
        build_game::TOOL_BUILD_GAME,
        "game.build",
        "game tool via root"
    );
    assert_eq!(build_game::TICKS_PER_SECOND, 60, "tick rate via root");
    assert_eq!(build_game::PLAY_TICKS, 18_000, "slice length via root");
    assert_eq!(injection::TOTAL_ATTEMPTS, 50, "injection corpus via root");
    assert_eq!(injection::SECRET_PREFIX, "sk-", "secret prefix via root");
    assert_eq!(
        insight::search::SEARCH_DEFAULT_LIMIT,
        8,
        "search default via root"
    );
    assert_eq!(insight::search::SEARCH_MAX_LIMIT, 32, "search cap via root");
    assert!(build_character::readonly(), "character stays read-only");
    assert_eq!(
        build_character::tool_name(),
        "character.build",
        "character name via root"
    );
}

/// Receipts carry proof and the sandbox gate fails closed, both through
/// the crate root. A changed code or flipped gate fails the asserts below.
#[test]
fn std_root_receipts_and_sandbox_fail_closed() {
    let landed = receipts::Receipt::landed(7, 3);
    assert!(landed.ok, "landed ok via root");
    assert_eq!(landed.effect, receipts::Effect::Landed, "landed via root");
    assert_eq!(landed.hash, 7, "hash threads via root");
    let answered = receipts::Receipt::answered();
    assert!(answered.ok, "answered ok via root");
    assert_eq!(
        answered.effect,
        receipts::Effect::Answered,
        "answered via root"
    );
    let refused = receipts::Receipt::refused_bind();
    assert!(!refused.ok, "refused fails via root");
    assert!(refused.retryable, "bind refusal retries via root");
    assert_eq!(refused.code, "arg_bind_failed", "bind code via root");
    assert_eq!(
        sandbox::admit(true, true),
        receipts::Effect::Landed,
        "readonly admits via root"
    );
    assert_eq!(
        sandbox::admit(false, true),
        receipts::Effect::Answered,
        "preview holds writes via root"
    );
    assert!(
        sandbox::tier_allows(0, true, false),
        "tier 0 reads via root"
    );
    assert!(
        !sandbox::tier_allows(0, false, false),
        "tier 0 never writes via root"
    );
    assert!(
        !sandbox::tier_allows(0, false, true),
        "tier 0 never deletes via root"
    );
    let mut log = audit::DoorAudit::new();
    assert!(log.is_empty(), "fresh audit empty via root");
    log.append(7, "scene.read", 0, audit::AuditOutcome::Ok, 11)
        .expect("audit appends via root");
    log.verify().expect("audit verifies via root");
    assert!(
        secret_launch::is_valid_key("API_TOKEN"),
        "good key via root"
    );
    assert!(
        !secret_launch::is_valid_key("lower"),
        "bad key refuses via root"
    );
}

/// Brief, canon, witness, injection and insight replay identically through
/// the root. Same inputs give the same ids, keys and receipts twice.
#[test]
fn std_root_brief_canon_witness_insight_replay() {
    assert_eq!(brief::BriefStage::Level.as_str(), "level", "stage via root");
    assert_eq!(
        brief::BriefStage::parse("level").expect("tag parses via root"),
        brief::BriefStage::Level,
        "tag roundtrips via root"
    );
    assert_eq!(
        brief::brief_id_of("prop crate probe_a", 7),
        0xf919fb008dbe0d32,
        "brief id vector via root"
    );
    let raw = "prop crate alpha\nwant alpha next_to wall:north\nspawn alpha";
    let first = canon::canon_key(raw).expect("canon lands via root");
    let again = canon::canon_key(raw).expect("canon replays via root");
    assert_eq!(first, again, "same raw gives same key via root");
    let seen = witness::Witness::new(0x1111_2222_3333_4444, 7);
    let proposal = witness::prepare(
        seen,
        "move the brazier north",
        "brazier by the north wall",
        0xaaaa_bbbb_cccc_dddd,
        0,
    )
    .expect("proposal prepares via root");
    let mut gate = witness::WitnessGate::new(0x1111_2222_3333_4444, 7);
    let receipt = gate
        .commit(&proposal, 0, &[])
        .expect("commit lands via root");
    let retry = gate
        .commit(&proposal, 0, &[])
        .expect("retry answers via root");
    assert_eq!(receipt, retry, "double commit answers once via root");
    assert_eq!(gate.apply_count(), 1, "one state change via root");
    assert_eq!(
        injection::Category::ALL.len(),
        5,
        "five categories via root"
    );
    assert_eq!(
        injection::redact("token sk-abc-123 done"),
        "token [redacted] done",
        "redact via root"
    );
    let tokens = insight::search::tokenize("ObjectId");
    assert!(tokens.contains(&"objectid".to_owned()), "full via root");
    assert!(tokens.contains(&"object".to_owned()), "hump via root");
    assert!(tokens.contains(&"id".to_owned()), "tail via root");
    assert!(
        insight::search::tokenize("a").is_empty(),
        "noise drops via root"
    );
    let tape = vec![
        assert_run::TapeRow {
            tick: 0,
            digest: 11,
            used_network: false,
        },
        assert_run::TapeRow {
            tick: 1,
            digest: 22,
            used_network: false,
        },
    ];
    let asserts = vec![
        assert_run::TickAssert {
            tick: 0,
            want_digest: 11,
        },
        assert_run::TickAssert {
            tick: 1,
            want_digest: 22,
        },
    ];
    let cfg = assert_run::SealedConfig {
        max_ticks: 600,
        confinement: assert_run::Confinement::Strict,
    };
    let start = assert_run::RunProgress {
        next_index: 0,
        checked: 0,
    };
    let done =
        assert_run::run_sealed(&tape, &asserts, &cfg, &start).expect("sealed run passes via root");
    assert!(done.completed, "run completes via root");
    assert_eq!(done.checked, 2, "two rows via root");
}
