# Diagnosis 2026-10-22: the core systems that are weak, ranked

Measured this session, not copied from a plan. Every claim has a path or a count.

## How I measured

- Crate size and `#[test]` count per crate (`Get-ChildItem crates -Recurse -Filter *.rs | Select-String '#\[test\]'`).
- Open and landed rows per board file (`- [ ]` / `- [x]`).
- Targeted greps in `crates/` for the mechanisms the gap list says are missing.
- `node tools/board-slice.mjs --focus` and `--ready`.

Board load: 180 ready rows. The open pools: `SPRINT-18-gaps.md` 54, `SPRINT-21-look-feel.md` 50,
`SPRINT-22-web-realism.md` 45, `SPRINT-08-hard-stuff.md` 28, `SPRINT-01-rome-first.md` 27,
`SPRINT-03-finish-line.md` 18, `SPRINT-31-research.md` 15, `SPRINT-27-final-picture.md` 15,
`SPRINT-30-feel.md` 12, `SPRINT-28-rome.md` 12.

Test depth is not the weak point: every crate above 50 KB has at least 31 unit tests
(`render` 1403, `net` 570, `assets` 562, `anim` 507, `playtest` 497, `spatial` 445).

## W1 — The picture is not lit, and nothing scores it (pillar 1)

- `content/reference/` holds 2 files: `index.json` and `README.md`. Zero reference captures.
- The machine score exists and has nothing to compare to: `crates/endpoint/src/packs/view.rs:77`
  (`view.compare`) and `crates/playtest/src/look/ssim2.rs` (SSIMULACRA2, whose header names the verb
  as its consumer).
- Still open: LF2 (the window lights with IBL), EG12 (ambient, torches, normal and ORM maps),
  FC1 (frame and look), FEEL5 (sparks, blood and dust on a hit).
- The lit-frame build (gap B7, size L) has no depth and normal prepass, no GTAO, no real post
  chain (bloom, grade, TAA), no particles or decals in the scene.

Cost of fixing: the verb and the metric are done; captures are the missing half. Reference
captures per tier are hours, not weeks. The denoise and post chain is the big one — see
`01-big-steals.md` S1 and S2 for a donor that already solved it in WGSL.

## W2 — Impact and weight are unimplemented (pillar 1, feel)

FEEL2 (a blow moves a body by momentum), FEEL3, FEEL5, FEEL1, FEEL7 (landing has weight),
FEEL6 (a clash binds and recoils), FEEL4 (armour deflects), FEEL9, FEEL10 (close contact pushes
back), FEEL11 (pad rumble) — all open. AM24 still owns the 258-264 pops residual
(attack 62 / idle 68 / run 128 per 300 frames). The duel reads as two boxes exchanging numbers.

## W3 — A gate can pass by doing nothing (trust)

- `Select-String 'not_vacuous'` over `crates/`: no hit. `crates/proof` has 269 unit tests and no
  anti-vacuity helper, and no perturbation control (nudge one input, require the digest to move).
- `crates/arena/src/sync.rs:46` names the first diverging *system* out of five fixed ones
  (`world`, `space`, `combat`, `rng`, `ai`). It cannot name the fighter or the component.
- The golden is the engine's whole claim. A golden that stopped reading the sim would stay green.

This is the cheapest high-value fix in the list (size S) and it is a donor pattern, not a new
idea: see S4 and the 2026-10-22 survey (`renew`'s `assert_not_vacuous`).

## W4 — The net layer cannot say whose fault (pillar 4)

- `check_desync` in `crates/net/src/rollback.rs` compares one peer hash and returns a `Desync`
  error. No input digest on the wire: `Select-String 'input_digest' crates/net` gives no hit
  (the only `input_digest` in the tree is the door's brief-stage record,
  `crates/endpoint/src/build_game/replay.rs:92`).
- ON30 (internet play) and the browser client (gap A3) are open. `crates/net` is the third
  biggest crate (2.1 MB, 570 tests), so the code is there; the *report* is missing.

## W5 — The door budgets in characters, not tokens (pillar 3)

`crates/endpoint/src/mcp.rs:150` and its neighbors size responses in characters. The 2026-10-22
survey part 2 found a donor (`modoki-engine` `docs/mcp-response-budget.md`) that measured the
`chars/4` estimate against a real BPE tokenizer and found it under-reports JSON payloads by 25 to
38 percent, with hex GUIDs costing 1.80 chars per token. Our door's whole bet is what a weak model
can hold in one call, so this number is load-bearing.

## W6 — Content loads by file, not by fetch (pillar 9)

- WEB6 open: no `AssetSource` fetch path, no `LoadQueue` (priority, byte budget, cancel).
- `crates/assets/src/asset_source/bundle.rs` sorts blobs by a static `(priority, path)` order.
- The generational `AssetHandle` (`crates/assets/src/handle.rs`) still has no caller.
- Consequence: the browser tab and a weak PC both get the same un-budgeted load.

## W7 — The tick is single-threaded where it shows (pillar 3)

Gap E21 open: the fighter step and the islands do not run through `fork_join_on`. The fork path
itself already moved to the shared pool (`crates/arena/src/fork.rs:410` "queue the job on the
shared job pool"), so the remaining work is the step loop, not the plumbing.

## W8 — The plan drifts from the tree (process)

The gap list in `docs/plan/finish-line.md` calls things missing that the tree has:

- A4 "a CI leg that creates the instance with `STRICT_WEBGPU_COMPLIANCE`" is **shipped**, in all
  three parts: the flag (`crates/render/src/backend.rs:437` `STRICT_WEBGPU_ENV`, added at line 461),
  the runner (`crates/render/src/bin/webgpu_cts.rs`), and the legs — `.github/workflows/ci.yml:113`
  and `.github/workflows/webgpu-cts.yml:39` both set `E2040_STRICT_WEBGPU: "1"`. The row still reads
  as open work in `docs/plan/finish-line.md`.
- C13 debug kit: `crates/render/src/debug_draw.rs` exists with five tests and (verified) no
  caller. The row is right about the caller, wrong about the module.
- D16 shipped 2026-10-21 (resources, scenario prompts, progress notes).

A stale gap list sends the next agent to build what already exists. Fix: re-verify the 23 items
against the tree in one pass, and mark each one built / partial / missing with a path.

## W9 — No once-per-session state seam, no hot reload (pillar 4, 7)

The tape records per-tick inputs and state, but there is no seam for state decided once before
tick 0 (bindings, locale, audio mix, pipeline warm-up). A replay derived from the replaying
machine's settings is how a rebound key silently changes what a recording reproduces.

## W10 — Parked systems that the pillars still lean on

`crates/audio` (363 tests) and the GPU crowd are parked by owner decision, so they are not
weaknesses; they are deferred. Listed so a reader does not count them as gaps.

## What I would close first

1. Reference captures per tier into `content/reference/` (W1): the verb already scores them. Hours.
2. Anti-vacuity helper plus a one-input perturbation control in `crates/proof` (W3). Size S.
3. `input_digest` beside the state digest in `crates/net`, one bit `inputs_agree()` (W4). Size S.
4. The hit-buffer-plus-denoise post chain (W1) from the donors in S1 and S2. Size M-L, in slices.
5. Re-verify the 23 gap rows against the tree (W8) so the next wave does not re-build them.
