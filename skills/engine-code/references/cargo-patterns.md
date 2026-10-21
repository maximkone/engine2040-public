# Cargo fix patterns per failure text

Symptom, rule, fix, how to confirm.

## `did not match any packages`

Symptom: `cargo update <name>` names a package no lock entry carries.
Fix: update with no name (every entry) or with the exact locked name.
Confirm: exit 0 and the lockfile keeps its pins.

## `does not contain this feature`

Symptom: `--features <name>` fails because the manifest has no such `[features]` entry.
Fix: add the feature to `[features]` or drop the flag.
Confirm: the plain run and the flagged run differ as expected.

## ``profile `nosuch` is not defined``

Symptom: `--profile <name>` names a profile that is neither built in nor defined.
Fix: use `dev`, `release`, `test`, `bench`, or define `[profile.<name>]` with `inherits`.
Confirm: the build prints the `Finished` line of the chosen profile.

## `failed to read` a member manifest

Symptom: `cargo build --workspace` names a member directory with no `Cargo.toml`.
Fix: create the member package or drop it from `members`.
Confirm: the build compiles every member.

## `no targets specified in the manifest`

Symptom: `autobins = false` (or a sibling key) hides `src/main.rs` and no `[[bin]]` names a target.
Fix: declare the target explicitly or remove the `auto*` key.
Confirm: the binary builds again.

## `could not find Cargo.toml`

Symptom: a cargo command runs in a directory with no manifest above it.
Fix: move into the package or workspace directory first.
Confirm: the build starts with a `Compiling` line.

## `test result: FAILED. 0 passed; 1 failed`

Symptom: a unit test asserts the wrong value.
Fix: correct the expected value only if the code is right. Otherwise fix the code.
Confirm: `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>` prints a passing result, exit 0.

## TOML parse failure from `--config`

Symptom: a `--config` string that is not valid TOML fails before any unit compiles.
Fix: pass valid `key = value` TOML or put it in `.cargo/config.toml`.
Confirm: the build proceeds to `Compiling`.
