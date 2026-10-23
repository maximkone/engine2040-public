# Before code and workflow

Full text behind `SKILL.md` Before code and Workflow.

## Before code

1. `git status`. Preserve changes you did not make.
2. `node tools/board-slice.mjs <ROW>`. A ticked row, or a named function that already
   exists, is a NOOP: say so and stop.
3. Read the crate's public API and tests. Grep its section of `docs/ARCHITECTURE.md`
   (not the whole file) and `docs/CODELAW.md`.
4. Name the consumer, the data owner, the units, the determinism scope and the error
   type.

## Workflow

1. Smallest complete change. Ladder: does it need to exist, then reuse, then std, then
   an allowed installed crate, then new code.
2. Prefer a module. Add a crate only for a real dependency, platform or ownership
   boundary.
3. Typed ids and errors. Bounded buffers. Validate external input (sizes, ranges, NaN,
   versions). No `unwrap` on IO or user data.
4. `engine-core` stays std-only. IO, GPU, model and network work stays outside the
   fixed tick. Job completion order never decides sim state.
5. Test the behavior and one failure case. Then wire the real consumer. A trait that
   compiles is not integration.
6. `git grep` every new `pub` name. A caller outside tests must exist. Zero callers:
   wire one (tick, submit, encoder) with a behavior test on the call path, or board
   its consumer row.
7. Finish the capability in one row. If it cannot land whole, say exactly what
   remains.
8. Proof: `cargo check -p <crate>` while you work. Then
   `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>` and
   `node tools/gh-check.mjs cargo clippy -p <crate> --all-targets -- -D warnings`.
   `rustfmt --edition 2021 --check <files>` (never `cargo fmt`).
9. If sim code changed, run the golden exactly as AGENTS.md "Proof" says:
   `node tools/gh-check.mjs cargo test -p engine2040 --release --test proof_cli
   default_and_max_seed`. The binary's `--verify` must print `d57b5c8ae3096614`
   (GitHub builds the fat-LTO binary). `-p` is the package name in
   `crates/<dir>/Cargo.toml`.

"Blocking waiting for file lock" is another cargo build in `target/`. Wait.
