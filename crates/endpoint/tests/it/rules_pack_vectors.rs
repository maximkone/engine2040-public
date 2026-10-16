//! Rules pack vectors: the `rules` pack as the door registry sees it.
//!
//! Consumer: the MCP door registry (`PackRegistry::shipped`) loads the
//! `rules` pack on demand via `packs::rules::pack()` and dispatches its
//! verbs by exact name. Data owner stays `packs::rules`; this file owns
//! only the pack composition contract. Units: pack and verb names plus
//! capability cards. Determinism: registration order only, no clock, no
//! IO, no network. Errors are panics via asserts; `pack()` itself must be
//! `Ok`.
//!
//! Why this file exists: `crates/endpoint/src/packs/rules.rs` changed 3
//! times in 30 days yet held zero `#[test]`, and no `tests/it` module
//! pinned its `pack()` composition (`rules_door` drives the verbs through
//! the door but never calls `pack()`), so dropping `plan_goals` or
//! `play.step` from the pack would compile yet starve the registry at the
//! `api.load("rules")` line. Each vector below names the pack path, so a
//! dropped verb or renamed pack fails here first.
//!
//! Run: `cargo test -p engine-endpoint --features mcp --test it rules_pack_vectors::`
//! and with rules verbs:
//! `cargo test -p engine-endpoint --features mcp,rules --test it rules_pack_vectors::`.

#![cfg(feature = "mcp")]

use engine_endpoint::packs::{
    rules::{self, PACK},
    PackRegistry,
};
use engine_endpoint::tool_capabilities::CapabilityCard;

#[cfg(feature = "rules")]
use engine_endpoint::packs::rules::{TOOL_RULES_EXPLAIN, TOOL_RULES_SET, TOOL_RULES_TEST};
use engine_endpoint::plan_goals::MCP_TOOL_NAME as TOOL_SIM_PLAN_GOALS;
use engine_endpoint::play::play_session::TOOL_PLAY_STEP;

/// The `rules` pack advertises its contract name with the planner plus the
/// live play step always, and the three scripted-rule verbs only when the
/// `rules` feature is on. Removing one verb fails the exact list below.
#[test]
fn rules_pack_pins_name_and_verb_order() {
    assert_eq!(PACK, "rules", "pack const via pack path");
    assert_eq!(
        TOOL_SIM_PLAN_GOALS, "sim.plan_goals",
        "planner verb const via pack path"
    );
    assert_eq!(
        TOOL_PLAY_STEP, "play.step",
        "live step verb const via pack path"
    );
    #[cfg(feature = "rules")]
    {
        assert_eq!(TOOL_RULES_SET, "rules.set", "set verb const via pack path");
        assert_eq!(
            TOOL_RULES_TEST, "rules.test",
            "test verb const via pack path"
        );
        assert_eq!(
            TOOL_RULES_EXPLAIN, "rules.explain",
            "explain verb const via pack path"
        );
    }

    let pack = rules::pack().expect("rules pack registers via pack path");
    assert_eq!(pack.name(), "rules", "pack name via pack path");
    assert!(!pack.summary().is_empty(), "pack summary via pack path");
    assert!(
        !pack.default_loaded(),
        "rules pack loads on demand via pack path"
    );

    #[cfg(not(feature = "rules"))]
    let expected: Vec<&str> = vec!["sim.plan_goals", "play.step"];
    #[cfg(feature = "rules")]
    let expected: Vec<&str> = vec![
        "sim.plan_goals",
        "play.step",
        "rules.set",
        "rules.test",
        "rules.explain",
    ];
    assert_eq!(pack.verb_names(), expected, "verb order via pack path");

    for verb in pack.verbs() {
        assert_eq!(verb.pack(), "rules", "verb pack via pack path");
        assert_eq!(
            verb.card(),
            CapabilityCard::Observe,
            "verb card via pack path"
        );
        assert!(
            !verb.description().is_empty(),
            "verb description via pack path"
        );
        let model = verb.model();
        assert_eq!(model.name, verb.name(), "model name via pack path");
        assert!(model.output_schema.is_some(), "output schema via pack path");
    }

    let again = rules::pack().expect("rules pack repeats via pack path");
    assert_eq!(
        again.verb_names(),
        pack.verb_names(),
        "same pack repeats identically via pack path"
    );
}

/// The `rules` pack registers clean and resolves through the registry the
/// way `api.load("rules")` sees it: one pack lookup plus one verb lookup
/// per advertised name, with the shipped registry agreeing.
#[test]
fn rules_pack_registers_clean_and_resolves_through_registry() {
    let pack = rules::pack().expect("rules pack registers via pack path");
    let registry = PackRegistry::build(vec![rules::pack()]);
    assert!(
        registry.errors().is_empty(),
        "rules pack builds without error via pack path: {:?}",
        registry.errors()
    );
    let stored = registry
        .pack("rules")
        .expect("rules pack lists via pack path");
    assert_eq!(
        stored.verb_names(),
        pack.verb_names(),
        "stored verbs via pack path"
    );
    for name in pack.verb_names() {
        let verb = registry
            .verb(name)
            .unwrap_or_else(|| panic!("verb {name} resolves via pack path"));
        assert_eq!(verb.pack(), "rules", "verb owner via pack path");
    }
    assert!(
        registry.verb("shell.exec").is_none(),
        "unknown verb misses via pack path"
    );

    let shipped = PackRegistry::shipped();
    let shipped_rules = shipped
        .pack("rules")
        .expect("shipped registry carries rules via pack path");
    assert_eq!(
        shipped_rules.verb_names(),
        pack.verb_names(),
        "shipped verbs via pack path"
    );
}
