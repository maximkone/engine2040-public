//! rules: game rules an AI writes as text, run in a sandbox, replay-exact (owner decision 2026-10-04).
//!
//! A rule script is Rhai restricted to 64-bit integers: no float, no clock, no strings, no
//! imports, no host call outside a short list (`host::HOST_FNS`). It defines `fn on_tick(me)`
//! which reads fighters and emits actions; the arena applies the actions. `Rules::set` compiles,
//! vets and smoke-runs a candidate and installs it only when all of that passes, keeping the
//! last good script otherwise. `Rules::test` and `Rules::explain` change nothing. The hash of
//! every installed script goes into the tape header (`tape`), so an old tape replays with its
//! old script. Not a default workspace member and not a dependency of any tick crate.
//!
//! Callers and edges (from `crates/rules/Cargo.toml:21-22`): above `engine-core`
//! digests, plus optional `engine-arena` only behind the off-by-default `arena`
//! feature (`arena::RulesController`, `crates/rules/src/arena.rs:50`); off
//! `default-members` (root `Cargo.toml:18`). The only in-repo consumer is the
//! door `rules` pack (`crates/endpoint/Cargo.toml:27,44`, feature `rules`, off by
//! default); no tick crate depends on it.
//!
//! Invariants: integers only (`only_i64` + `no_float`, no clock, no import, capture
//! or syntax extension, `crates/rules/Cargo.toml:29-38`); Rhai pinned to `=1.26.1`
//! (`crates/rules/src/rules.rs:27`) because op counts are replay contract and a bump
//! is an ABI change (`RULES_ABI_VERSION`, `crates/rules/src/rules.rs:25`); unknown
//! host calls refused at `set` time by the AST walk. Errors: [`RulesError`] at every
//! boundary, never a panic on a bad script.
//!
//! Budgets (`Limits::DEFAULT`, `crates/rules/src/limits.rs:35-44`): 2_000 ops,
//! 8 call levels, 32 expr depth (16 in functions), 16_384 source bytes, 8 actions
//! per call, 32 variables, 16 functions; 64 fighters (`MAX_FIGHTERS`,
//! `crates/rules/src/limits.rs:7`), 8 memory slots each (`MEM_SLOTS`,
//! `crates/rules/src/limits.rs:9`). Entry points: [`Rules::set`]
//! (`crates/rules/src/rules.rs:262`), [`Rules::test`] (`rules.rs:305`),
//! [`Rules::explain`] (`rules.rs:331`), [`Rules::tick`] (`rules.rs:351`),
//! [`script_hash`] (`crates/rules/src/script.rs:33`), [`rules_abi`] (`rules.rs:32`).

#[cfg(feature = "arena")]
pub mod arena;
pub mod error;
pub mod fixtures;
pub mod host;
pub mod limits;
mod rules;
mod script;
pub mod sim;
pub mod tape;

pub use error::{HostFault, LimitKind, RulesError};
pub use host::{Action, ActorView, Emitted, HostCall, World};
pub use limits::{Limits, MAX_FIGHTERS, MEM_SLOTS};
pub use rules::{
    rules_abi, ActorExplain, CaseReport, Explanation, Fault, Installed, MemSnapshot, Rules,
    SetReceipt, TestReport, TickReport, RHAI_VERSION, RULES_ABI_VERSION,
};
pub use script::script_hash;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod wiring_tests {
    //! Root contract: what callers see through `engine_rules::`.
    //!
    //! Why this module exists: `crates/rules/src/lib.rs` changed 4 times in
    //! 30 days yet held zero `#[test]`, and the crate has no `tests/it/`
    //! directory, so a dropped `pub mod` or `pub use` would still compile
    //! the crate but break every caller at their `use` line (the door
    //! `rules` pack, the arena controller hook). Each test below names the
    //! root path, so a removed or gated item fails here first. The behavior
    //! asserts go through the same root path: they fail when the behavior
    //! breaks, not when the internals move.

    use crate::{
        fixtures, host, rules_abi, script_hash, sim, tape, Action, ActorExplain, ActorView,
        CaseReport, Emitted, Explanation, Fault, HostCall, HostFault, Installed, LimitKind, Limits,
        MemSnapshot, Rules, RulesError, SetReceipt, TestReport, TickReport, World, MAX_FIGHTERS,
        MEM_SLOTS, RHAI_VERSION, RULES_ABI_VERSION,
    };

    /// Every re-exported headline number stays reachable with its frozen
    /// value intact. Removing one `pub mod` or `pub use` line fails this
    /// file at its `use` or at the path below.
    #[test]
    fn root_pins_headline_consts_and_modules() {
        assert_eq!(MAX_FIGHTERS, 64, "fighter cap via root");
        assert_eq!(MEM_SLOTS, 8, "memory slots via root");
        assert_eq!(crate::limits::MAX_FIGHTERS, 64, "limits module agrees");
        assert_eq!(crate::limits::MEM_SLOTS, 8, "slots agree");
        assert_eq!(Limits::DEFAULT.max_ops, 2_000, "op budget via root");
        assert_eq!(Limits::DEFAULT.max_call_levels, 8, "call depth via root");
        assert_eq!(Limits::DEFAULT.max_expr_depth, 32, "expr depth via root");
        assert_eq!(Limits::DEFAULT.max_fn_expr_depth, 16, "fn depth via root");
        assert_eq!(
            Limits::DEFAULT.max_source_bytes,
            16_384,
            "source bytes via root"
        );
        assert_eq!(
            Limits::DEFAULT.max_actions_per_call,
            8,
            "actions per call via root"
        );
        assert_eq!(Limits::DEFAULT.max_variables, 32, "variables via root");
        assert_eq!(Limits::DEFAULT.max_functions, 16, "functions via root");
        assert_eq!(RULES_ABI_VERSION, 3, "ABI version via root");
        assert_eq!(RHAI_VERSION, "1.26.1", "Rhai pin via root");
        assert_eq!(host::HOST_FNS.len(), 20, "host fns via module");
        assert_eq!(host::PROPS.len(), 20, "props via module");
        assert_eq!(host::INPUT_FNS.len(), 3, "input fns via module");
        assert_eq!(host::MAX_AMOUNT, 1_000, "amount bound via module");
        assert_eq!(host::MAX_TICKS, 600, "tick bound via module");
        assert_eq!(host::MAX_PUSH_MM, 5_000, "push bound via module");
        assert_eq!(host::MAX_WISH, 100, "wish bound via module");
        assert_eq!(host::MAX_FACING_CDEG, 18_000, "facing bound via module");
        assert_eq!(host::BTN_ATTACK, 0x01, "attack bit via module");
        assert_eq!(host::BTN_BLOCK, 0x02, "block bit via module");
        assert_eq!(host::BTN_DODGE, 0x04, "dodge bit via module");
        assert_eq!(tape::TAPE_MAGIC, b"E2RL", "tape magic via module");
        assert_eq!(tape::TAPE_VERSION, 1, "tape version via module");
        assert_eq!(tape::MAX_TAPE_TICKS, 200_000, "tape ticks via module");
        // Module paths the door pack reads through the root.
        let _ = crate::host::HOST_FNS;
        let _ = crate::host::PROPS;
        let _ = crate::limits::Limits::DEFAULT;
        let _ = crate::tape::TAPE_MAGIC;
        let _ = crate::fixtures::ring_world as fn(usize, u32, u64) -> World;
        let _ = crate::sim::scripted_inputs as fn(usize, u32, u64) -> Vec<crate::sim::ScriptInput>;
        let _ = script_hash as fn(&str) -> u64;
        let _ = rules_abi as fn(&Limits) -> u64;
        let _ = Action::is_input as fn(&Action) -> bool;
        let _: Option<MemSnapshot> = None;
        let _: Option<TestReport> = None;
        let _: Option<TickReport> = None;
        let _: Option<CaseReport> = None;
        let _: Option<Explanation> = None;
        let _: Option<Fault> = None;
        let _: Option<Installed> = None;
        let _: Option<SetReceipt> = None;
        let _: Option<ActorExplain> = None;
        let _: Option<ActorView> = None;
        let _: Option<Emitted> = None;
        let _: Option<HostCall> = None;
        let _: Option<World> = None;
        let _: Option<RulesError> = None;
        let _: Option<HostFault> = None;
        let _: Option<LimitKind> = None;
    }

    /// Hash plus ABI stay deterministic through the root and move only
    /// when their inputs move.
    #[test]
    fn script_hash_and_abi_stable_via_root() {
        let source = "damage(1, 10);\n";
        assert_eq!(script_hash(source), script_hash(source), "hash repeats");
        assert_eq!(
            script_hash("damage(1, 10);\r\n"),
            script_hash(source),
            "CRLF normalizes via root"
        );
        assert_ne!(
            script_hash(source),
            script_hash("heal(1, 10);\n"),
            "edit moves hash via root"
        );
        let limits = Limits::DEFAULT;
        assert_eq!(rules_abi(&limits), rules_abi(&limits), "ABI repeats");
        let mut wider = limits;
        wider.max_ops += 1;
        assert_ne!(
            rules_abi(&limits),
            rules_abi(&wider),
            "budget moves ABI via root"
        );
        assert!(
            !Action::Damage {
                target: 1,
                permille: 10
            }
            .is_input(),
            "damage is state via root"
        );
        assert!(
            Action::Wish { mx: 0, mz: 0 }.is_input(),
            "wish is input via root"
        );
        assert!(
            Action::Face { cdeg: 0 }.is_input(),
            "face is input via root"
        );
        assert!(
            Action::Press { buttons: 0 }.is_input(),
            "press is input via root"
        );
    }

    /// Set, test and tick round-trip through the root: a good edit
    /// installs and acts, bad edits refuse with the last good kept.
    #[test]
    fn set_test_tick_roundtrip_via_root() {
        const GOOD: &str = "damage(1, 10);\n";
        let mut rules = Rules::default();
        let receipt = rules.set(GOOD, 0).expect("good installs via root");
        assert_eq!(receipt.rev, 1, "first rev via root");
        assert_eq!(receipt.hash, script_hash(GOOD), "hash via root");
        assert!(receipt.max_ops > 0, "ops counted via root");
        assert_eq!(rules.script_hash(), Some(receipt.hash), "active via root");
        assert_eq!(rules.history().len(), 1, "history grows via root");
        let worlds = fixtures::ring_world(2, 100, 7);
        let report = rules.test(GOOD, std::slice::from_ref(&worlds));
        assert!(report.ok(), "good tests clean via root");
        assert_eq!(report.hash, script_hash(GOOD), "test hash via root");
        let tick = rules.tick(&worlds).expect("tick runs via root");
        assert!(tick.faults.is_empty(), "no faults via root");
        assert_eq!(tick.actions.len(), 2, "both actors act via root");
        for emitted in &tick.actions {
            assert_eq!(
                emitted.action,
                Action::Damage {
                    target: 1,
                    permille: 10
                },
                "damage shape via root"
            );
        }
        let before = rules.script_hash();
        let float = rules.set("let f = 1.5;\n", 1).expect_err("float refuses");
        assert_eq!(float, RulesError::Float { line: 1 }, "float line via root");
        assert_eq!(rules.script_hash(), before, "last good kept via root");
        assert_eq!(rules.refusals(), 1, "refusal counted via root");
        let unknown = rules
            .set("bogus_call(1);\n", 2)
            .expect_err("unknown host refuses");
        assert!(
            matches!(
                unknown,
                RulesError::UnknownHostCall { ref name, .. } if name == "bogus_call"
            ),
            "unknown names call via root: {unknown}"
        );
        assert_eq!(
            rules.tick(&worlds).unwrap().actions.len(),
            2,
            "good still acts"
        );
        let _ = sim::scripted_inputs as fn(usize, u32, u64) -> Vec<crate::sim::ScriptInput>;
    }
}
