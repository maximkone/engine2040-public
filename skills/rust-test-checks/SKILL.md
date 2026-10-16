---
name: rust-test-checks
description: "Use when a builder writes or reviews Rust unit or integration tests in engine2040 crates, or when a judge checks that a test proves behavior before the diff lands."
version: 0.1.0
author: skillworks
---

# Rust test checks

Nine checks for Rust test shape. Name the crate in each finding (core, ecs, physics, combat, net, arena, assets, perf, save). PASS needs a run line (cargo test -p <crate>) never a promise.

1. **Unit placement.** Ask: does the unit test live in src in a cfg(test) module that brings the parent into scope?
   Flag: helper compiled into the shipped build; test reaching across modules instead of its own unit (crates: core, ecs).
2. **Boolean asserts.** Ask: does a true-or-false check use assert with a clear name and the negated form for the false case?
   Flag: bare panic or a name like it_works that says nothing on failure (crates: physics, combat).
3. **Equality asserts.** Ask: does an equality check use assert_eq or assert_ne so failure prints both sides, with PartialEq plus Debug in place?
   Flag: manual == inside assert that hides values; custom type missing derived traits (crates: combat, save).
4. **Failure message.** Ask: does each assert carry a custom message that prints the actual value seen?
   Flag: bare assertion line with no value to debug from (crates: arena, assets).
5. **Panic paths.** Ask: does an error case use should_panic with an expected substring that pins the right guard?
   Flag: bare should_panic that passes on any panic, even the wrong branch (crates: core, input).
6. **Fallible tests.** Ask: does a test that can fail with Err return Result and use ? for setup, with is_err asserts for error cases?
   Flag: should_panic mixed with Result; unwrap hiding the failure value (crates: net, save).
7. **Parallel safety.** Ask: do tests run parallel by default with no shared file, dir, or env unless split or pinned to one thread?
   Flag: two tests writing the same test-output file; env mutated mid-suite (crates: perf, physics).
8. **Run control.** Ask: can the run filter by name, run one file, show captured output only on demand, and park slow tests as ignored?
   Flag: full suite always run; println relied on without show-output; slow test blocking the gate (crates: perf, arena).
9. **Integration layout.** Ask: do integration tests live in tests as separate crates on the public API, with shared setup in tests/common/mod and thin binaries over a lib?
   Flag: integration test calling private items; tests/common file run as an empty test crate; binary logic untestable from tests (crates: combat, game).

## Do / Don't
- Do: keep unit tests beside the code under cfg(test) and integration tests in tests on the public API. Don't: compile test helpers into the build or test private detail from tests.
- Do: pin panic tests with an expected substring and print actual values in messages. Don't: accept bare should_panic or value-free asserts.

source: Klabnik et al., The Rust Programming Language (3rd ed.), ch. 11
