# Fix patterns per failure text

Each pattern: symptom, the rule, the sanctioned fix, and how to confirm.
All outputs below were measured with cargo 1.97.1; the replay script
`scripts/cargo_book.py` reproduces every one of them.

## `did not match any packages`

Symptom: `cargo update <name>` fails naming a package no lock entry
carries. Fix: update with no name (every entry) or with the exact locked
name. Confirm: exit 0 and the lockfile keeps its pins.

## `does not contain this feature`

Symptom: `--features <name>` fails because the manifest has no such
`[features]` entry. Fix: add the feature to `[features]` or drop the flag.
Confirm: plain run prints `basic`, flagged run prints `extra on`.

## ``profile `nosuch` is not defined``

Symptom: `--profile <name>` fails for a profile neither built in nor
custom-defined. Fix: use `dev`, `release`, `test`, `bench`, or define
`[profile.<name>]` with `inherits`. Confirm: the build prints the
`Finished` line of the chosen profile.

## `failed to read` a member manifest

Symptom: `cargo build --workspace` fails naming a member directory with no
`Cargo.toml`. Fix: create the member package or drop it from `members`.
Confirm: the build prints `Compiling a` and `Compiling b` and finishes.

## `no targets specified in the manifest`

Symptom: `autobins = false` (or a sibling key) hides `src/main.rs` while no
explicit `[[bin]]` names a target. Fix: declare the target explicitly or
remove the `auto*` key. Confirm: the binary builds again.

## `could not find `Cargo.toml``

Symptom: a cargo command runs in a directory with no manifest above it.
Fix: move into the package or workspace directory first. Confirm: the build
starts with a `Compiling` line.

## `test result: FAILED. 0 passed; 1 failed`

Symptom: a unit test asserts the wrong value (`assert_eq!(add(2, 2), 5)`).
Fix: correct the expected value (`5` to `4`). Confirm: `cargo test` prints
a passing result with exit 0.

## TOML parse failure from `--config`

Symptom: a `--config` string that is not valid TOML fails before any unit
compiles. Fix: pass valid `key = value` TOML or put it in
`.cargo/config.toml`. Confirm: the build proceeds to `Compiling`.
