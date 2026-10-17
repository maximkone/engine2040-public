//! Std-only lib wiring vectors: the endpoint crate root without `mcp`.
//!
//! Consumer: a headless caller linking `engine-endpoint` with default
//! features (no stdio server) who reaches the door through the crate root
//! (`engine_endpoint::tools`, `::transport`, `::guards`). Data owner stays
//! each module; this file owns only the root re-export contract. Units:
//! registry entries, frames, guard verdicts. Determinism: integer compares
//! only, no clock, no IO, no network. Errors are panics via asserts.
//!
//! Why this file exists: `crates/endpoint/src/lib.rs` long held zero
//! `#[test]`, so a dropped `pub mod` compiled the crate but broke every
//! caller at their `use` line. This file pins the `tools`, `transport` and
//! `guards` roots; `tests/it/lib_std_wiring.rs` pins the remaining std-only
//! modules. Each vector below names its root path, so removing or gating a
//! module fails here first.

use engine_endpoint::{
    guards::{self, VerbClass},
    tools,
    transport::{self, Frame},
};

/// The frozen replay registry stays pinned through the crate root.
#[test]
fn std_registry_wiring_pins_frozen_26() {
    assert_eq!(tools::TOOLS.len(), 26, "frozen registry size via lib root");
    assert_eq!(
        tools::REGISTRY_VERSION,
        1,
        "registry version pin via lib root"
    );
    let read = tools::lookup("scene.read").expect("scene.read resolves via root");
    assert!(read.readonly, "scene.read stays read-only");
    let apply = tools::lookup("tx.apply").expect("tx.apply resolves via root");
    assert!(!apply.readonly, "tx.apply stays a writer");
    assert!(
        tools::lookup("shell.exec").is_none(),
        "unknown verb fails closed via root"
    );
    let digest = tools::registry_digest();
    assert_eq!(
        tools::check_pinned_registry(tools::REGISTRY_VERSION, digest),
        Ok(tools::REGISTRY_VERSION),
        "live pin re-verifies via root"
    );
    let calls = ["scene.read", "tx.preview", "log.digest"];
    let first = tools::replay_pinned_calls(tools::REGISTRY_VERSION, digest, &calls)
        .expect("v1 tape replays via root");
    let second = tools::replay_pinned_calls(tools::REGISTRY_VERSION, digest, &calls)
        .expect("v1 tape replays twice via root");
    assert_eq!(first, second, "same tape replays identically via root");
}

/// The loopback transport shape stays reachable through the crate root.
#[test]
fn std_transport_wiring_admits_loopback_calls() {
    let good = Frame::new("tools.call", "scene.read", 7);
    assert!(transport::validate(&good), "shaped call validates via root");
    let bad_method = Frame::new("http.get", "scene.read", 7);
    assert!(
        !transport::validate(&bad_method),
        "unknown method refuses via root"
    );
    let empty_name = Frame::new("tools.call", "", 7);
    assert!(
        !transport::validate(&empty_name),
        "empty name refuses via root"
    );
    assert!(
        transport::is_loopback("127.0.0.1"),
        "loopback admits via root"
    );
    assert!(
        !transport::is_loopback("192.168.1.2"),
        "remote refuses via root"
    );
    assert!(
        transport::token_ok("s3cret", "s3cret"),
        "matching token admits via root"
    );
    assert!(
        !transport::token_ok("s3cret", ""),
        "empty expected token refuses via root"
    );
    assert!(
        transport::check_caps(transport::MAX_BYTES, transport::MAX_CHARS),
        "caps hold at the edge via root"
    );
    assert!(
        !transport::check_caps(transport::MAX_BYTES + 1, 0),
        "byte over-cap refuses via root"
    );
    assert_eq!(transport::timeout_for("search"), 5, "search tier via root");
    assert_eq!(transport::timeout_for("proof"), 30, "proof tier via root");
}

/// The shared guard block stays fail-closed through the crate root.
#[test]
fn std_guards_wiring_fails_closed() {
    assert_eq!(
        guards::verb_class("scene.place"),
        Some(VerbClass::Mutating),
        "scene.place class via root"
    );
    assert_eq!(
        guards::verb_class("tx.apply"),
        Some(VerbClass::Destructive),
        "tx.apply class via root"
    );
    assert_eq!(
        guards::verb_class("shell.exec"),
        None,
        "unknown verb fails closed via root"
    );
    assert!(
        guards::check_intent("").is_err(),
        "empty intent refuses via root"
    );
    assert!(
        guards::check_intent("move the brazier north").is_ok(),
        "named intent admits via root"
    );
    assert!(
        guards::check_dry_run_id("rcpt_7f3a").is_ok(),
        "minted receipt admits via root"
    );
    assert!(
        guards::check_dry_run_id("7f3a").is_err(),
        "unminted receipt refuses via root"
    );
    assert!(guards::check_args_ops(1).is_ok(), "one op admits via root");
    assert!(
        guards::check_args_ops(0).is_err(),
        "empty batch refuses via root"
    );
    assert!(
        guards::check_args_ops(guards::ARGS_MAX_OPS + 1).is_err(),
        "oversized batch refuses via root"
    );
    assert!(
        guards::check_policy(VerbClass::Destructive, false, 0).is_err(),
        "destructive at low tier refuses via root"
    );
    assert!(
        guards::check_policy(VerbClass::Mutating, false, 0).is_ok(),
        "mutating always admits via root"
    );
}
