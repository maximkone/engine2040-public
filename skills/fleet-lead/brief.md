# Coder brief for engine2040 helper agents (read once, follow exactly)

Repo: C:\engine2040, Windows, shell is pwsh (not bash: no `&&`, no heredocs, no `$(...)`). One branch `master`,
one working tree shared by up to 20 AIs editing at the same time. AGENTS.md is the law; this is the short form.

## Your job
One board row (or one named slice of it). 45-minute box. Land each proven piece at once. You send no helpers.
Never pick up a neighbouring row. Never ask questions (nobody answers): decide small things, report them.

## Read
- Row: `node tools/board-slice.mjs <ID>` (row text, plan excerpt, owned paths). Plan: `node tools/plan-atlas.mjs --row <ID>`.
- Search with grep/glob/read, always with a `path` or `include` (a root grep dies on a huge glTF under content/arena/src).
- Skills in C:\engine2040\skills\<name>\SKILL.md if the row's area needs one (engine-code, anim-combat, physics-bodies,
  render-lighting, wgpu-render, ai-door, test-and-proof). Read at most two.

## Edit
- `git pull --no-rebase --no-edit origin master` first. Then `git status --short`: every dirty line you did not make
  belongs to another AI. Never revert, rewrite, stash, reset, checkout or restore it.
- A file another AI has dirty is fine for a small separate hunk (one field, one mod line, one call): edit it, prove
  and commit only your hunk with `--list` / `--pick`. Skip only when your edit would land inside their hunk.
- Re-read a file right before you edit it; change hunks with the edit tool, never rewrite a whole file someone else has dirty.
- Tests first. No stubs, no `todo!()`, no `#[allow]` (use `#[expect(lint, reason = "...")]`). Short English comments
  that state invariants. Match the surrounding code.
- Prefer NEW files for new logic (`foo.rs` + `foo_tests.rs` via `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`),
  so you touch shared big files only for the one `mod` line or one call.
- Golden hash d57b5c8ae3096614 must not move. If your change touches sim code (crates core, combat, physics, rules,
  arena tick), say so in the report; the lead runs the golden.

## Prove (never local cargo test/clippy; never a private CARGO_TARGET_DIR or copied tree)
- ALWAYS put `--remote-only` before the cargo word: without it a run that shows 0 tests or cannot start falls back
  to a full build on this PC (2.5 GB free RAM, shared target lock). Exit 2 or "0 passed" means YOUR line is wrong:
  the test file or its `mod` line hunk (`lib.rs#N`) is missing from `--only`, the run needs a feature
  (`--features rapier` for physics rapier code, `--features app` for crates/game look code, `--features mcp` for
  endpoint verbs), or the filter is misspelled. Fix that and run again.
- `node tools/gh-check.mjs --remote-only --only <your,paths> cargo test -p <crate> --lib <filter>`
  (crate names are `engine-<dir>`, e.g. engine-render, engine-anim; check the crate's Cargo.toml `name`).
  crates/game tests need `--features app`: `cargo test -p engine-game --features app --lib <filter>`.
  Other feature gates: physfx `--features rapier`; endpoint `--features mcp` (intake verb `mcp,intake`); audio
  `--features device`; render GPU tests `--features window` (`gpu` alone fails on window-only create_surface).
- A passing test's println is hidden: to read a receipt number, end the cargo line with `-- --nocapture`
  (with a filter: `--lib <filter> -- --nocapture`), then `node skills/fleet-lead/scripts/runlog.mjs <run id>`.
- New refusal codes in crates/endpoint need a row in the advice table and a kind in mcp_refusal, or the
  whole-lib tests `refusal_advice_*` / `refusal_every_code_*` go red. Run `--lib mcp::advice` and `--lib mcp_refusal`.
- Exit 3 = still running: run the printed `node tools/gh-check.mjs --wait wip-...` line (add `--budget 600`). Never rerun
  the cargo line. Read the LAST line: a pass must show a test count above 0.
- Stay in your turn while a proof runs: run the `--wait` line yourself; never end a reply with "waiting on the runner".
- To iterate on compile errors: `node tools/gh-check.mjs --only <paths> cargo check -p <crate> --tests` (about 2 min).
- In a file another AI also has dirty, prove only your hunks: `node tools/commit-mine.mjs --list <path>` numbers
  hunks, then `--only <path>#2,<path>#5`.
- Before commit: `rustfmt --edition 2021 --check <your .rs files>` (fix with `rustfmt --edition 2021 <files>` only on
  files wholly yours; never `cargo fmt`), and `node tools/rust-quality.mjs --crate <crate-dir-name> --owned <file> --owned <file>` PASS.

## Commit (land each proven piece at once)
- `git diff --check -- <paths>` and read `git diff -- <paths>`: every hunk must be yours.
- `node tools/commit-mine.mjs -m "<area>: <what changed, short> (<ROW>); run <gh run id> green, <N> tests" --whole <new-or-wholly-yours files> --pick <shared/path.rs>#<n> --push`
  `--pick` takes the hunk numbers from `--list`. It refuses if HEAD moved: pull and run it again.
  `--whole` now also takes LFS files (.glb/.png etc., it stages the pointer), binary files (.ogg) and deletions
  (a path gone from disk): no more plain `git add` for those.
- Do NOT edit docs/SPRINT*.md or docs/FOCUS.md; the lead ticks rows from your report.
- Never: `git add -A`, `git commit -a`, stash, reset, rebase, amend, force-push, worktree, `git clean`, deleting under target/,
  starting arena.exe directly (use `node tools/arena-run.mjs` only if the row says a frame, and only if told).

## Report (your final message, max 15 lines)
ROW: <ID>  RESULT: DONE | PARTIAL | BLOCKED
COMMITS: <sha7> <one line each>
PROOF: <exact gh-check line> -> <passed count>, run <id>
ACCEPT LEFT: <what of the row's Accept is not done, e.g. a frame capture>
SIM TOUCHED: yes/no
NOTES: <surprises, other AIs' reds you saw (name the file, do not fix it)>
