---
name: rust-quality-ratchet
description: "Use when a commit is refused by the pre-commit hook (rust-quality ratchet regresses), node tools/rust-quality.mjs prints FAIL, or you add code that may raise casts, long fns, big files, dup fns or hot allocs: read the number, fix it, or accept it with a reason."
---

# Rust quality ratchet

`tools/rust-quality.mjs` counts debt per crate in production code (tests excluded) and compares with `tools/rust-quality.baseline.json`. Counts only go down. What each metric means and the bar: `docs/RUST-QUALITY.md`. Rule list for your diff: skill `review`.

## 1. Run it

- `node tools/rust-quality.mjs --crate game --no-cache` (name or `engine-game`; repeat `--crate`). Always `--no-cache`: the per-file cache once hid a regression for hours. It takes about a second per crate.
- Last lines: `[FAIL] ratchet <crate> <metric> <was> -> <now>` and `RESULT FAIL: n ratchet regression(s)`, exit 1. `RESULT PASS` exit 0. Exit 2 is bad flags.
- `--list <metric> --crate <c> [--limit 40]` prints the hits (file and line), so you see which function or file raised it. Metrics: `unwrap`, `panic_sites`, `casts`, `allow_no_reason`, `dead_code_allows`, `print_in_lib`, `string_errors`, `board_ids`, `hash_reimpl`, `dup_fns`, `passthrough`, `long_fns`, `big_files`, `hot_alloc`, `nondet`, `waivers`, `undoc_pub`, `unsafe`.
- `--owned <path>` limits the verdict to regressions that touch your paths (foreign debt becomes `[WARN]`). The hook uses it.
- Sim crates also get a config check (`tools/sim-clippy.toml` must be in each): a `sim config` FAIL names the crate.

## 2. Fix, in this order

| metric | what raises it | the fix |
|---|---|---|
| `casts` | `as` between numbers | `From`, `TryFrom`, or a named helper that already exists in `engine-core` |
| `long_fns` | a fn over 100 lines | split into helpers; do not paste |
| `big_files` | a file over 1500 production lines | put new code in a NEW module (as `ragdoll_look.rs` and `break_window.rs` did) instead of growing `fighter_look.rs`; a file already big does not add a count, but it keeps the crate at the limit |
| `dup_fns` | a body copied elsewhere in the workspace | call the owner (reuse ladder in skill `engine-code`) |
| `hash_reimpl` | FNV or SplitMix constants outside `engine-core` | use the core hash |
| `hot_alloc` | `Vec::new`, `format!`, `clone` in a step, tick, update, render or mix fn of a hot crate | reuse a buffer held by the owner |
| `board_ids` | row ids or history in a comment | delete the comment text; history goes to git |
| `allow_no_reason` | `#[allow]` or `#[expect]` with no `reason =` | `#[expect(lint, reason = "...")]` |
| `nondet` | HashMap, Instant, SystemTime in a sim crate | BTreeMap, ticks |
| `undoc_pub` | pub item with no doc comment | one doc line |

Run again after each fix. Do not move code to a test module to hide it.

## 3. The pre-commit hook

`.git/hooks/pre-commit` is a copy of `tools/hooks/pre-commit`. Install by copying it there; the two must stay equal.

- It looks at your STAGED `.rs` files and crate `Cargo.toml` files only, in a snapshot of the index (not your working files, not other sessions' edits). Docs, shaders and root files pass at once.
- It refuses when those crates regress and the baseline is not staged. The message starts `refused: rust-quality ratchet regresses (staged files of: ...)` and prints the command to see why.
- Repeat what it ran: `node tools/rust-quality.mjs --crate <c> --owned crates/<c>/src/<file>.rs --no-cache`. Fix the number (section 2). Then commit again.
- Never use `--no-verify`.

## 4. Tighten and accept

- You paid debt down: `[INFO] n metric(s) improved below baseline`. Run `node tools/rust-quality.mjs --tighten` and include `tools/rust-quality.baseline.json` in the same commit. That locks the gain.
- A raise is allowed only when the design needs it and no smaller way exists (a new crate with a real cast table, a generated file). Then: `node tools/rust-quality.mjs --accept <crate> --reason "<a real reason, 10+ characters>"`. It rewrites that crate's baseline entry and logs date, old and new numbers and the reason in `accepted`. Put the baseline in the same commit as the code: `git commit -F msg -- <your code paths> tools/rust-quality.baseline.json`.
- `--accept` stores the whole crate's current numbers, including debt other sessions have in the working tree. Run it only when `git status --short crates/<c>` shows nothing foreign. Otherwise tell the owner.
- Do not run `--init` (it refuses when a baseline exists).

## 5. Traps

- On the shared tree your index may hold other sessions' staged files; commit by path (skill `commit-and-board`). With a temp index (Case B there), the baseline file must be in that index too, or the hook sees no baseline.
- `rustfmt --edition 2021 --check <files>` is a separate gate. Never `cargo fmt` (it rewrites other sessions' files).
- The mirror CI runs the ratchet on committed HEAD. A hunk another session left in HEAD can make it red; say whose file, do not fix it.
- `cargo clippy --fix` can delete a `return` that is only needless on Linux: check `cfg(target_os)` code by hand.
