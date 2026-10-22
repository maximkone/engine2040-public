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

## W1 — The picture is not lit, and its score does not see it (pillar 1)

- `content/reference/` is not empty and I was wrong to read it that way. It holds a real reference
  set: `index.json` with three rows (`native` 1280x720, `web` 640x360, `potato` 320x180), each with a
  `pixel_fnv` digest, no PNG bytes in git by design, painted by a recipe in
  `crates/playtest/src/look/reference.rs`. **The problem is the recipe**: its own `source` string
  says "the stands band over textured sand and two staged fighters". That is a hand-painted stand-in,
  not a frame any renderer produced. So `view.compare` (`crates/endpoint/src/packs/view.rs:77`) and
  SSIMULACRA2 (`crates/playtest/src/look/ssim2.rs`) score real captures against a made-up picture,
  and the look gate can go green without the renderer ever running. Same family as W3, and the fix is
  the same shape: the reference must be the cooked frame (WEB17 already aims at that).
- Still open: LF2 (the window lights with IBL), EG12 (ambient, torches, normal and ORM maps),
  FC1 (frame and look), FEEL5 (sparks, blood and dust on a hit).
- The lit-frame build (gap B7, size L) has no depth and normal prepass, no GTAO, no real post
  chain (bloom, grade, TAA), no particles or decals in the scene.

Cost of fixing: the verb and the metric are done; the reference content is the missing half, and it
is WEB17's job (the page and the window drawing the real cooked fight, not stand-ins). The denoise
and post chain is the big one — see `01-big-steals.md` S1 and S2 for a donor that already solved it
in WGSL.

## W2 — Impact and weight are unimplemented (pillar 1, feel)

FEEL2 (a blow moves a body by momentum), FEEL3, FEEL5, FEEL1, FEEL7 (landing has weight),
FEEL6 (a clash binds and recoils), FEEL4 (armour deflects), FEEL9, FEEL10 (close contact pushes
back), FEEL11 (pad rumble) — all open. AM24 still owns the 258-264 pops residual
(attack 62 / idle 68 / run 128 per 300 frames). The duel reads as two boxes exchanging numbers.

## W3 — A gate can pass by doing nothing (trust) — CLOSED 2026-10-22

- `Select-String 'not_vacuous'` over `crates/`: no hit (2026-10-22 morning). **Closed the same day**:
  `crates/proof/src/not_vacuous.rs` (commit `39267b928`) refuses zero steps, a digest at 0, a digest
  at the FNV-1a offset basis, and a digest that does not move when one velocity bit (1 ULP) changes
  or the run takes one more tick; proven falsifiable against four fake producers, and the 600-tick
  golden tape clears the guard. `proof --lib` 193 passed on GitHub.
- `crates/arena/src/sync.rs:46` names the first diverging *system* out of five fixed ones
  (`world`, `space`, `combat`, `rng`, `ai`). It cannot name the fighter or the component.
- Was: the golden is the engine's whole claim, and a golden that stopped reading the sim would stay
  green. It no longer can; what remains is the entity-level name.

What is left here: the per-entity slices (the labelled-digest steal from `balaur`, booked today by
the survey) and a consumer that prints the new verdict. The gate itself now fails if it stops
reading the sim.

## W4 — The net layer cannot say whose fault (pillar 4)

- `check_desync` in `crates/net/src/rollback.rs` compares one peer hash and returns a `Desync`
  error. No input digest on the wire: `Select-String 'input_digest' crates/net` gives no hit
  (the only `input_digest` in the tree is the door's brief-stage record,
  `crates/endpoint/src/build_game/replay.rs:92`).
- ON30 (internet play) and the browser client (gap A3) are open. `crates/net` is the third
  biggest crate (2.1 MB, 570 tests), so the code is there; the *report* was missing. **Closed the
  same day**: `crates/net/src/desync.rs` (commit `8369761c4`) carries tick, local peer, per-peer
  state and input digests, `last_agreed_tick`, `inputs_agree()`, `witnessed()` and a `DesyncVerdict`
  (sim side / inputs differ / inputs unsettled / unwitnessed), folded locally from the confirmed
  rows — no wire, codec, snapshot or golden change. Proofs: `net --lib desync` 14 passed,
  `net --test it` 44 passed. Still open: a consumer that prints it, and the browser transport.

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

## Order of attack, and what actually happened (updated the same day)

1. **W3 anti-vacuity control — LANDED** `39267b928` (`crates/proof/src/not_vacuous.rs`, 7 tests,
   `proof --lib` 193 passed on the runner).
2. **W4 input digest + `inputs_agree()` — LANDED** `8369761c4` (`crates/net/src/desync.rs`, 6 tests,
   `net --lib desync` 14 passed, `net --test it` 44 passed). No wire, codec or golden change.
3. **W1 — the reference must be the cooked frame, not a painted stand-in** (WEB17's job). Size M.
   Blocked today on `crates/render`, which another session has dirty across the post chain
   (`post_chain.wgsl`, `composite_compute.wgsl`, `tonemap.wgsl`, `forward.wgsl`, `sky.rs`, `fog_fwd.rs`).
4. **W8 — re-verify the 23 gap rows against the tree** before anyone is sent to build a shipped
   thing. Size S (a read pass, no code).
5. **S1 slice one** (hit buffer + à-trous in `crates/render`), **S2** (the wgpu denoiser), **S3/S4**
   (the door's checks and its generated guide), **S5** (a replay corpus per system), **S6** (the
   wasm transport): all real, all M or L, and none of them is a day's work. They are not landed, and
   writing them badly today would cost more than waiting.
