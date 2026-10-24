//! design_wired: the `design` duel-timing module is declared by the crate.
//!
//! Regression pin for the workspace-audit FAIL
//! (`design.rs is not declared by any module, so it is never compiled`):
//! this file names `engine_combat::design`, so it fails to compile while
//! the module is orphaned and passes once `lib.rs` declares it. Std-only,
//! integer asserts only, no heap in the asserts themselves.

use engine_combat::design::{
    chain_allowed, feint_allowed, hitstop_for_strike, latency_ms, validate, SHIPPED,
};

/// The shipped duel-timing set validates and carries its documented values.
#[test]
fn shipped_design_set_is_wired_and_valid() {
    assert_eq!(validate(&SHIPPED), Ok(()));
    assert_eq!(SHIPPED.light_ticks, 3);
    assert_eq!(SHIPPED.heavy_ticks, 6);
    assert_eq!(SHIPPED.parry_ticks, 8);
    assert_eq!(SHIPPED.startup_ticks, 30);
    assert_eq!(SHIPPED.parry_window_ticks, 7);
}

/// Strike freeze answers parry first, then heavy, then light.
#[test]
fn shipped_hitstop_answers_parry_heavy_light() {
    assert_eq!(hitstop_for_strike(&SHIPPED, false, false), 3);
    assert_eq!(hitstop_for_strike(&SHIPPED, true, false), 6);
    assert_eq!(hitstop_for_strike(&SHIPPED, false, true), 8);
}

/// Feint, chain and latency boundaries match the documented readable tell.
#[test]
fn shipped_feint_chain_and_latency_match_the_tell() {
    assert_eq!(latency_ms(SHIPPED.buffer_ticks), 33);
    assert!(feint_allowed(&SHIPPED, 27));
    assert!(!feint_allowed(&SHIPPED, 28));
    assert!(chain_allowed(&SHIPPED, 33));
    assert!(!chain_allowed(&SHIPPED, 32));
}
