# Living world (NPC minds and live AI)

Pre-merge text: `git show a0e13976:docs/plan/living-world.md`. Merged in below: `docs/archive/2026-10-10/plans/platform-live-ai.md`, the live-AI half only (its section 1 table of live-AI rows and its section 3 target design). The platform half stays in `docs/archive/2026-10-10/plans/platform-live-ai.md`; its board rows stay on the board.

Goal: NPC minds and live AI lines run off the tick; only validated, tick-stamped actions enter the tick.

Acceptance:
1. A recorded model reply replays with the provider off and gives the same hash (LW5).
2. A memory soak of 10,000 events stays under the cap, with bytes equal x2 (LW12).
3. Moderation refuses a flagged line and the tick is unchanged (LAI4).

Order: LW5, AB14, LW12, LW13 (checklist, in that order), then the table in ID order.

Owner crates: `ai`, `endpoint`, `provider`, `game`, `playtest`.

Open board IDs: AB10 AB14 AB15 AB16 LAI1 LAI2 LAI3 LAI4 LAI5 LAI6 LAI7 LAI8 LW5 LW12 LW13 LW14 (ticked and never-boarded rows are dropped).

## 1. Where it stands

| Capability | Status |
| --- | --- |
| NPC mind contract (stream, goals, relations, async asks, fallback, recording) | implemented, unit-tested, integrated, ticked |
| Memory store (bounded per NPC, integer salience/recency, deterministic order) | implemented, unit-tested, integrated, ticked |
| Ludus comes alive (trainer, blacksmith, crowd voices with memory + goals) | implemented, unit-tested, integrated, ticked |
| Living-world eval (20 conversation + goal fixtures, replay exactness) | implemented, unit-tested, integrated, ticked |
| Reflection and importance (bounded 0..=1000, cadence, grounded lines) | partially implemented, open |
| Provenance for every LW/AB write (tick, seq, source, request id, undo) | implemented, unit-tested, integrated, ticked |
| Dialogue scripts (lines, branches, conditions, barks, compiled offline) | implemented, unit-tested, integrated, ticked |
| Tick-boundary apply (Say/Gesture presentation-only; movement as intents) | implemented, unit-tested, integrated, ticked |
| Memory save/load + budget audit | implemented, unit-tested, integrated, ticked |
| Brief to level (relations, SP5 place, nav checks, preview/apply/undo) | implemented, unit-tested, integrated, ticked |
| Brief to character (licensed kit, CT1 retarget, RL6 layers, provenance) | implemented, unit-tested, integrated, ticked |
| Brief to encounter (roster, HR3 tables, EA1c fork check) | implemented, unit-tested, integrated, ticked |
| Shared brief envelope + repair budget | implemented, unit-tested, ticked |
| Brief to playable slice (AB1+AB2+AB3 over WD6 modes, boot + replay) | implemented, unit-tested, integrated, ticked |
| Planned encounters (brief to goals, classical planner in plain Rust) | implemented, unit-tested, integrated, ticked |
| Variant dedup experiment (rewrite rules, collapse near-duplicates) | unit-tested, integrated, ticked |
| Ludus-visit vertical slice (AB1 corner + LW3 voices, play + replay) | implemented, unit-tested, integrated, ticked |
| Offline brief replay (recorded chain, providers-off replay) | implemented, unit-tested, integrated, ticked |
| Bad-build brief fails loudly (seeded bad brief lowers its stage score x2) | open |
| Procedural-vs-model content spike | missing, open experiment |
| AB11 | implemented, unit-tested, integrated, ticked |
| AB13 mcp: dedup review-pipeline bind | implemented, unit-tested, integrated, ticked |
| Eval harness (20 authoring + 5 play x backends) | implemented, ticked |
| Deterministic event queue | rowed elsewhere, not ours |

Module notes that constrain the rows below:

- LW1 is the contract and it holds: `Mind` owns one NPC's stream
  with integer-only recall `salience * 1024 / (1 + age)` and
  earlier-wins ties; `MindRuntime::ask` validates, stamps inside
  `[issued, deadline]`, records provider or scripted fallback,
  suppresses duplicates; `replay_offline` reproduces checkpoints
  with providers off. LW2 (`MemoryStore`, 128 NPCs x 128 records)
  ranks the same shape. Rows deepen these files; no parallel
  mind/memory type is ever created.
- The tick never sees provider text: the LW8 drain is the only
  door. Say/Gesture are presentation-only; Approach/Leave/Idle
  become movement intents the existing brains consume. No row
  moves a body from a line of dialogue. LW10 wires the door; no
  row re-specifies it.
- FOCUS slim rule governs: dialogue-thin (scripted lines, barks,
  small branches), arena-ladder quests only, no social sim, no
  economy, no open chat. CUT social-full gets no code, no content,
  no waves; the ludus keeps trainer, blacksmith and crowd voices.
- AB stages run through the command door (preview, spatial checks,
  fork play-test, CAP1 approval, undo, provenance receipt — the
  EA1a–e spine). The model proposes parameters and text; gates
  decide. At most 3 repair rounds per AB stage (5 typed for
  encounters), else refuse with the findings or conflict set.
  AB7's envelope is the one budget type; LW11 retires AB1's
  private twin.

### Live AI (docs/archive/2026-10-10/plans/platform-live-ai.md section 1)

Scope tier: parked with the platform rows, which stay on the board. The live-AI rows run only when a Finish gate needs them. Voice, moderation and quotas do not exist yet.

| Capability | Status |
|---|---|
| NPC minds, memory, dialogue, tick-boundary apply | ticked (LW1, LW2, LW7, LW8) |
| Scripted announcer and taunts with provider fallback | ticked (PV3) |
| Injection scoring on the MCP door | landed |
| Voice out (TTS), voice in (STT), live lip-sync | missing (LAI1-LAI3) |
| Moderation of model lines before players see them | missing (LAI4) |
| Vision judge for captures | missing (LAI5) |
| Embedding retrieval for NPC memory | missing (LAI6) |
| Shipped local model | missing (LAI7) |
| Key safety for shipped games | missing (LAI8) |

## 2. Finish line

Scope tier (owner 2026-09-30, FOCUS.md): living world is PARKED —
rows stay and are never unticked; no new rows and no writers unless
a bar needs them. Highest-value gap (already rowed, never
duplicated): LW13 grounded-line ratchet — the contradiction rate is
UNKNOWN until it posts.

Tier A alpha (the ludus talks through the real door, one budget):
LW10 apply wiring + LW11 envelope adoption + LW6 provenance kept
green on every LW/AB write — all three ticked on the board
2026-09-30, so Tier A is met and kept green by LW12. Bar: 20
scripted asks apply identically twice with providers off through
a real brain and a real presentation sink; a Say action never
changes a body position; 20 briefs across AB1 and AB3 share one
envelope type and a 4th repair is refused with the findings
named; golden unchanged twice.

Tier B v1 (authors trust the chain): LW5 reflection remainder
(LW14 cadence/surprise, LW13 grounded eval) + LW9 save/load audit
kept green + AB9 offline brief replay (ticked) + planned-encounter goals (plan-local AB5; no board row)
beside AB3 (ticked) + AB15 cost/latency receipt + AB16
canon ratchet. Bar: 20 fixtures lower the contradiction rate on
mock/local/remote with same bytes x2; every said line cites its
recall set; 10 briefs replay exactly with providers off; 10
briefs give encounters inside win-rate bounds with at most 5
attempts each; 64-NPC soak holds caps with p95 printed.

Tier C (EMPIRE: living world + AI builds inside): LW4
eval green per build + AB4 ten playable slices (boot, 5-minute
play, exact replay, per-brief report) + AB8 ludus-visit slice
proving LW3 x WD2 x AB1 together + EV2 brief-to-game eval. Bar:
8+ of 10 briefs go brief-to-playable; every AI edit undoes
cleanly; rule-based keypoints gate, vision notes stay advisory.

CUT (never): social sim, open chat, quests beyond the arena
ladder, economy/crafting/progression-full, per-NPC LoRA (LATER
until LW4 shows memory alone falls short), voice synthesis,
server fleet, analytics, UGC. Barks plus thin branches are the
whole dialogue surface.

## 3. Target design

Data flow: observe (tick-stamped events into `Mind`) → distill
(`MemoryStore::remember_mind`) → recall (top 8) → snapshot (4
memories + 2 goals) → ask off-tick (mock/local/remote adapters,
deadline + privacy) → validate + stamp → record (`ActionLog`) →
drain at the tick boundary (LW8 apply, wired by LW10) → present
(Say/Gesture events) or steer (Approach/Leave as `gladiator`
intents). Reflection folds top recall into a `Felt` event on a
fixed cadence (LW5). AB flow: brief text → typed envelope (AB7,
adopted by LW11) → stage chain (AB1 level, AB2 character, AB3
encounter over plan-local AB5 closed goals) → preview + spatial checks + fork
play-test → apply with provenance (LW6) → undoable slice
(AB4/AB8) → eval (LW4/EV2) → replay offline (AB9, staged by AB12).

Ownership: `crates/ai` owns mind, memory, reflect, apply,
provenance and dialogue tables; `crates/game` owns only the ludus
NPC wiring that drains and presents; `crates/endpoint` owns the
brief envelope and the build stages plus goals/canon; `crates/
playtest` owns the living eval and brief-eval fixtures. No engine
crate imports `game` or `endpoint`; no AB stage invents its own
memory, provenance or repair-budget type after LW11.

Threads: the tick is single-truth and std-only. Provider calls
run off-tick with simulated-ms budgets (1/20/150 mock/local/
remote); late, cancelled or over-budget answers fall back to the
scripted table and are recorded as scripted. Bakes (dialogue
compile, brief repair, reflection summaries) run on workers and
publish at frame/tick boundaries through recorded actions only.

Determinism: authoritative code uses integer math, stable orders
(recall ties to earlier events, goal ties to lower ids, queue
flushes in priority/insertion order) and FNV-1a checkpoints; text
is bounded (280 chars) and hashed as bytes. Model text never
enters the hash except as recorded bytes that replay verbatim.
Wall clock, transport timing and credentials stay outside the
hash (HASH-LAW scope OUT).

Budgets: per-NPC caps frozen (512/16/64 stream, 128 stored,
8-hit recall, 4+2 ask context, 4 actions per ask); 10k-event soak
holds caps; recall fills caller buffers with no steady-state
allocation; AB stages cap at 3 repair rounds (5 typed for
encounters) and refuse past them; eval fixtures run under 60 s
per suite side.

API sketch: `apply_at(tick, action) -> Applied { presented |
intended | refused }`; `memory_save(npc) -> bytes v1`,
`memory_load(bytes) -> digest`; `brief_envelope(text, seed) ->
Envelope { id, budget }`; `build_level/character/encounter/game
(envelope) -> preview | refusal { findings }`; `plan_goals
(brief) -> goals`; `canon(encounters) -> variants`.

### Live AI (docs/archive/2026-10-10/plans/platform-live-ai.md section 3)

Plain version: platforms and models are adapters behind engine-owned
contracts, exactly like the C2 provider contract; the game talks to the
contract, and a null backend always works.

```
game ─► Platform trait ─► NullPlatform (default, offline)
                      └─► SteamPlatform (feature "steam", SDK outside the repo)

NPC wants to speak ─► model line (C2) ─► moderation ─pass─► TTS (cassette)
                                            └refuse─► scripted line      │
                                                                 visemes ─► face
player voice ─► STT ─► validated order (UW18) or refusal ─► recorded action
```

- **Ownership:** `app` owns the platform contract (composition layer);
  `game` owns the Steam backend and store scripts' inputs; `ui` owns
  localization; `editor` owns the sequencer panel; `provider` owns voice,
  moderation, quotas and the shipped-model runtime; `playtest` owns the
  vision judge; `ai` owns recall.
- **Determinism:** platform calls and model outputs never enter the tick
  except as recorded actions; cloud saves carry the SV1 envelope; the
  sequencer plays from data with tick-stamped events.
- **Budgets:** platform callbacks pumped once per frame ≤ 0.1 ms; TTS
  first audio ≤ 400 ms or the subtitle shows alone; moderation ≤ 5 ms
  local; shipped model ≤ 2 s per NPC line on the reference CPU.
- **Keys:** a shipped game never contains a provider key; remote models
  go through an owner-run relay with per-player quotas, or the player's
  own key stored by the OS, or the shipped local model.

## 6. Slim

- Delete: nothing (LW1/LW2/LW6/LW7/LW9 stand; LW10 and LW11
  ticked — wire/adopt landed, never rewritten).
- Merge: one brief envelope (AB7) replaces per-stage repair
  budgets — LW11 retires AB1's private twin; one apply contract
  (LW8) replaces per-NPC ad-hoc drains — LW10 puts the real
  callers on it; one provenance record (LW6) serves AB and LW
  writers; AB12 records per-stage I/O so AB9 replays the chain
  instead of re-asking.
- Freeze: LW1 caps (512/16/64/280/4), LW2 q8 salience and
  insertion-order ties, `apply.rs` effect mapping, `brief.rs`
  stage tags and budgets — rows deepen or wire, never re-tune,
  unless LW4 numbers force a named change.
- Will not build: social sim, open chat, per-NPC LoRA (LATER
  until LW4 shows memory alone falls short), full quest graphs,
  voice synthesis, server fleet, analytics, UGC. Barks plus thin
  branches are the whole dialogue surface; the ludus keeps
  trainer, blacksmith and crowd voices only.
- No mirrored types: AB stages never redeclare memory,
  provenance, digest or repair-budget shapes after LW11;
  `workspace-check` fails new `Mind|Memory|Brief|Provenance`
  twins outside their owner crates.

## 7. Board tasks

Ticked rows are dropped; rows never boarded are not listed. Checklist: the Accept line verbatim. Table: the rest (LW, AB, LAI).

- [ ] AB14 bad-build brief fails loudly. Accept: bad brief deltas >0 and x2 identical; empty brief still refused. needs: EV2, AB4.
- [ ] LW5 reflection and importance. Accept: 20 fixtures lower the contradiction rate on mock, local and remote; same bytes x2; a 10k-tick soak holds the memory cap. Deep sweep 2026-09-25: every line an NPC says cites its recall set; a negation or contradiction falls back and is recorded; importance stays integer; reflection on a fixed cadence plus capped surprise. needs: LW2, EV1.
- [ ] LW12 multi-NPC recall/drain budget soak. Accept: 64-NPC soak holds caps with recall/drain p95 printed; `cargo test -p engine-ai --lib memory` plus `cargo test -p engine-game --lib ludus_npcs` green; golden unchanged twice. needs: LW1, LW2, LW8.
- [ ] LW13 grounded-line plus contradiction ratchet. Accept: `cargo test -p engine-playtest --lib living` green with `grounded_count == rows` and `contradiction_pairs == 0` after repair; bytes x2. needs: LW2, LW4.

| ID | Title | Tier | needs |
|---|---|---|---|
| AB10 | procedural-vs-model content spike | - | CT14, HR6 |
| AB15 | brief-chain cost plus latency receipt | B | AB4, AB9, EV2 |
| AB16 | canon rule-cap ratchet plus VK1 gate | B | AB6, VK1 |
| LAI1 | voice out | B | OPT24, PV4 |
| LAI2 | voice in | B | PV4, UW18 |
| LAI3 | live lip-sync | C | AM12, LAI1 |
| LAI4 | moderation | B | PV3, LW8 |
| LAI5 | vision judge | B | OPT24, OPT25 |
| LAI6 | embedding recall | C | LW2, OPT24 |
| LAI7 | shipped local model | C | OPT27, LAI4 |
| LAI8 | quotas and key safety | B | OPT26, AW9 |
| LW14 | reflection cadence plus capped surprise | B | LW1, LW2 |

## 8. Quality gates

- Golden identical twice per row; every replay claim is
  providers-off x2 (LW1 `replay_offline`, LW9 save round-trip,
  AB12 per-stage replay, AB9 offline chain); a seeded divergence
  fails with the first bad tick named.
- Memory gates: same event log gives same bytes x2; 10k-event
  and 10k-tick soaks hold caps (512 stream, 128 stored, 8-hit
  recall); counting-allocator 0-per-tick on recall/drain paths.
- Dialogue thinness gate: every said line cites its recall set
  (LW5); contradiction/negation falls back recorded; no open
  chat, no social-sim state anywhere in `crates/ai`.
- Brief gates: 10 AB1 briefs zero spatial findings + navigable;
  5 AB2 characters pass import gates; 10 AB3 encounters inside
  win-rate bounds; 10 AB4 slices boot + 5-minute play + exact
  replay; 4th repair refused with findings; undo restores digest.
- Conformance + matrix: LW1 suite green on mock/local/remote per
  row touching asks; provider cost unknown stays unknown (never
  zero); red-team (RV1) stays green on endpoint rows.
- Provenance gate: 100% of AB/LW writes carry LW6 records (AB11
  proves it end to end); undo restores the digest; records
  survive save/load; corrupt bytes refuse loudly.
- Perf: PF1 workloads within 5% per row; recall/drain p95 named
  in the receipt (LW12 prints 64-NPC p95); eval suites under 60 s
  per side; canon 30 sets inside 60 s (AB16).

