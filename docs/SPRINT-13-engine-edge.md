### Engine edge (owner 2026-09-26, `docs/plan/quality.md`)

Speed, quality, AI-agnostic patterns and workspace. Every row measures
before and after and keeps its change only if the numbers move; misses
revert the same wave. OPT1–OPT13 serve the FOCUS "Now" speed pillar.
Rows marked (orchestrator) land root config or `tools/` edits.

- [x] OPT4 ops: linker A/B and PF6 re-verify — `rust-lld.exe` for Landed 631e7192.
  `x86_64-pc-windows-msvc`; record link time for `arena.exe` and test
  binaries; restate what PF6 actually delivered. (orchestrator) Tier A.
  Size S. Owns `.cargo/config.toml` hunks and a PF6 note. Accept: keep
  only if link ≥ 20% faster with every test green; the PF6 receipt names
  what exists. needs: OPT1.
- [x] OPT5 ops: profiles with one job each — `profiling` (release, line Landed 15ffd5d2.
  tables, thin LTO), `perf-test` (release, thin LTO, 16 codegen units)
  for proof `--release` legs if PF1 budgets hold, `ship` (fat LTO, one
  unit, `panic = "abort"` after an unwind audit, PGO from a recorded
  tape per PF3). (orchestrator) Tier A. Size M. Owns the profile edits.
  Accept: proof wall time and `arena.exe` frame time before/after; PF1
  baselines re-recorded append-only (PF11) when timings shift. needs:
  OPT1, PF3, PF11.
- [ ] OPT6 ops: nextest spike — proof legs under nextest with per-test
  processes; retries reported, never hidden; the `-j4` flake fingerprint
  re-checked. (orchestrator) Tier A. Size S. Owns the proof-leg switch.
  Accept: wall time and flake counts before/after; kept only if faster
   with the same failures visible. needs: OPT1.
   (DO NOT DISPATCH 2026-10-05: PARKED, cargo-nextest absent exit 101. Retry only after owner installs and posts version. Never re-probe whole, per 510 deep-dive.)
- [x] OPT7 ops: affected-package gate — `tools/affected.mjs` maps a diff Landed cd37e7c4.
  to changed packages plus reverse dependencies via `cargo metadata`;
  lane acceptance runs that closure; the full proof stays once per wave.
  (orchestrator) Tier A. Size M. Owns `tools/affected.mjs`. Accept: a
  seeded `combat` edit runs combat and dependents only; a `core` edit
  runs all; lane gate median wall time halved. needs: OPT1.
- [ ] OPT8 ops: feature unification spike — a cargo-hakari workspace-hack
  crate (license read first). (orchestrator) Tier B. Size S. Owns the
  spike report. Accept: dependency rebuild count across five lane builds
  before/after; kept only if it drops by a third. needs: OPT1.
- [ ] OPT9 ops: compile cache spike — sccache across lane pools.
  (orchestrator) Tier B. Size S. Owns the spike report. Accept: cold lane
  build time before/after; kept only if ≥ 30% faster. needs: OPT1.
- [ ] OPT11 ops: crate fan-out cut — reverse-dependency closure times
  compile seconds from OPT1 names the costliest edges; split or
  feature-gate the worst one or two. (orchestrator + owning lanes) Tier C.
  Size M. Owns the report plus the named split. Accept: touch-rebuild of
  the chosen crate down ≥ 30%, or the row closes with its numbers. needs:
  OPT1, OPT7.
- [ ] OPT13 ops: gameplay code hot-patch spike — subsecond or
  hot-lib-reloader (licenses read first) on one gameplay system in the
  window build. Tier C. Size M. Owns the spike report. Accept: an edit
  reaches the running game in ≤ 2 s with state kept, or the row closes
  with the failure. needs: OPT3.
- [ ] OPT14 ecs: spine performance bars — PF1 workloads on the ARC spine:
  10k entities transform and body step ≤ 0.5 ms, 1k agents full tick
  ≤ 2 ms, 0 allocations; the ARC3 parity harness prints them. Tier B.
  Size S. Owns `crates/perf` spine workloads. Accept: numbers x2 in the
  PF1 table. needs: ARC2, PF1.
- [ ] OPT15 app: CPU frame pipelining — sim tick N+1 overlaps render
  extraction and submit of tick N; the extraction view is the only sync
  point. Tier C. Size M. Owns the frame loop in `crates/app`. Accept: CPU
  frame p95 −25% in the window build; replay hash unchanged x2. needs:
  ARC6.
- [ ] OPT16 render: GPU feature ladder — probe bindless texture arrays,
  multi-draw indirect count and ray query at startup beside RT0; draw
  through the fastest supported path with a tested fallback. Tier B.
  Size M. Owns `crates/render/src/ladder.rs`. Accept: 10k instances of 200
  materials in ≤ 50 draw calls on the fast path; fallback matches
  within tolerance on llvmpipe. needs: VG1, RA8.
- [ ] OPT17 render: texture streaming — mip residency from a GPU feedback
  buffer read back asynchronously, a VRAM budget for 6 GB, LRU eviction,
  uploads on the RA6 belts; never stalls. Tier B. Size L. Owns
  `crates/render/src/stream.rs`. Accept: a dense fixture stays under
  budget; no frame hitch > 2 ms from uploads; sharp within 1 s of a cut.
  needs: RA4, RA6.
- [ ] OPT18 render: frame pacing and latency — present mode per setting,
  frame latency 1 in low-latency mode, input sampled late for the camera;
  measured by the PQ1 meter. Tier B. Size M. Owns
  `crates/render/src/pacing.rs`. Accept: input-to-photon p95 −20% at equal
  fps; frame-time variance reported. needs: WIN1, PF2.
- [ ] OPT19 render: zero first-use stutter — warm every pipeline variant
  a scene can use during load through RA5; a camera walk across every
  material shows no frame above 1.5× the median from pipeline creation.
  Tier A. Size S. Owns warm-up hunks in `crates/render/src/cache.rs`.
  Accept: the walk fixture passes x2. needs: RA5, RA7.
- [ ] OPT20 render: dynamic resolution — measured GPU time drives the
  internal resolution between 50% and 100% through the TAAU upscaler to
  hold the frame budget. Tier B. Size M. Owns
  `crates/render/src/dynres.rs`. Accept: under a seeded GPU load spike
  frame time p95 ≤ 16.7 ms; resolution changes logged; golden unchanged.
  needs: GX4, RA8.
- [ ] OPT21 ops: mutation testing ratchet — cargo-mutants on `core`,
  `combat`, `nav`, `spatial` and `net`; surviving mutants listed with
  rows for the worst gaps; the survival rate per crate may never rise.
  Tier A. Size M. Owns the report and the ratchet leg. Accept: seeding a
  deleted assertion fails the ratchet. needs: none.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] OPT22 proof: property tests — proptest (license read first, dev
  dependency) for ten invariants: codec round-trips, replay equality,
  snapshot restore, spatial check symmetry, fixed-point bounds; fixed
  seeds in proof, shrinking reported. Tier B. Size M. Owns
  `tests/properties/`. Accept: 10k cases each x2; a seeded bug is found
  and shrunk. needs: none.
  Wave 57 detail (WILD corner-23 Card 3, no new row): spatial_eval
  to_text output gets a normalized compare (timings/paths redacted);
  0 volatile-field golden churn over 5 waves, seeded volatiles caught
  10/10; kill if >2 seeded volatiles escape or normalization touches
  semantic bytes.
- [ ] OPT23 ops: API-break check — cargo-semver-checks (license read
  first) on every public engine crate against the last tagged release.
  Tier C. Size S. Owns the check leg. Accept: a seeded breaking change
  fails; a doc-only change passes. needs: API2.
- [x] OPT24 provider: response cassettes — a content-addressed store Landed e04071b6.
  keyed by (adapter, model, request digest, schema version); live runs
  record, evals and CI replay offline with no keys; a stale entry is
  refused with its reason, never reused silently. Tier B. Size M. Owns
  `crates/provider/src/cassette.rs`. Accept: the EV1 suite replays from
  cassettes byte-identical x2 with the network off; a changed prompt
  misses and says why. Landed so far (2026-10-12 check): 86d5164a (cassette store, single-call replay). Remaining: the EV1 suite recorded once and replayed twice from the cassette store gives byte-identical `to_text` with no provider calls; a changed prompt misses with its reason. needs: PV5.
- [x] OPT25 provider: structured-output ladder — one engine schema per
  action type maps to each vendor's native structured output, then JSON
  mode, then text with a bounded repair parse, then scripted; the PV5
  matrix records the rung each backend used. Tier B. Size M. Owns
  `crates/provider/src/ladder.rs`. Accept: every backend reports its rung;
  malformed output never applies; rung drops are logged events. needs:
  PV5, AX1.
- [x] OPT26 provider: scorecard router — per task class (bark, dialogue, Landed aa644693. Run 39052718180; router has no caller beyond Ask::for_request yet.
  level brief, encounter, review) pick a model from the eval scorecard
  (quality, p95 latency, cost) under budget caps, falling back local →
  remote → scripted; every decision recorded. Tier B. Size M. Owns
  `crates/provider/src/router.rs`. Accept: a seeded outage of the top
  model reroutes within the deadline; decisions replay from the record;
  a cost cap refuses loudly. needs: OPT24, EV1.
- [ ] OPT27 provider: inference arbiter — local models and the game Progress 2026-10-17: provider side done (arbiter, serving adapter); left: feed live frame p95 and read_vram, window-build budget check on a GPU.
  share one 6 GB GPU: under frame pressure local inference moves to CPU,
  defers or routes remote; VRAM use reported; the tick never waits.
  Tier B. Size M. Owns `crates/provider/src/arbiter.rs`. Accept: with a
  local model loaded the window build holds the P6 frame budget; requests
  degrade with typed notices. needs: PV2, RA4.
- [x] OPT28 provider: prompts as versioned assets — every prompt and Landed aa644693. Run 39052718180.
  brief template is a content-addressed asset with version, provenance
  and an eval set; an edit re-runs its evals and may not lower the score.
  Tier B. Size S. Owns `crates/provider/src/prompt.rs`. Accept: a seeded
  score-dropping edit fails the gate; the prompt digest lands in every
  provider record. needs: OPT24, EV1.
- [ ] OPT29 mcp: agent onboarding budget — tokens an outside agent
  spends from discover to its first applied edit, per vendor, with a
  budget and a ratchet beside the AX3 ledger. Tier B. Size S. Owns the
  measurement test. Accept: counts per vendor x2; the budget holds.
  needs: MCP7, EV1.
- [ ] OPT30 ops: crate cards — every crate root header states owner,
  consumers, invariants, budgets and public entry points in one checked
  format that Engine Insight indexes. Tier B. Size S. Owns the
  `workspace-check` rule (orchestrator) plus header edits. Accept: 30 of
  30 crates pass; a seeded missing field fails. needs: API1.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] OPT31 pilot: dev window arena ≥ 30 fps on the reference GPU over
  the fixed arena camera path after OPT3, x2 within 5%, with the frame
  report under `target/evidence/build/`. Tier A. Size S. Owns the
  report. Accept: numbers x2 name adapter and driver; blocked until the
  BLOCKERS window-path entry closes. needs: OPT3, WIN1. (OPT3 VERDICT
  wave 84.)
- [ ] OPT32 ops: build-baseline promotion on the quiet box — rerun the
  OPT1 matrix back-to-back x2 (clean plus 4-crate touches x 3 profiles),
  single cargo, lanes halted, load <10%, root `target/`.
  (orchestrator) Tier A. Size S. Owns the promotion. Accept: per-crate
  pairs within 10% with the critical path named; on success the OPT1
  table is marked baseline and OPT4–OPT9 plus OPT12 unpark. HELD for
  the OPERATOR quiet-box window; until then OPT1 stays observed-only.
  needs: OPT1. (OPT1 VERDICT wave 84.)
- [ ] OPT33 operator: DX12 vs Vulkan A/B on the reference GPU with the
  OPT2 harness over the fixed 600-frame arena camera path, x2 within 5%;
  the default backend follows the numbers with the other as fallback and
  startup logs why. Tier B. Size S. Owns the report. Accept: report under
  `target/evidence/backend/` names adapter and driver with CPU and GPU
  p50/p95/p99 per pass, VRAM, draws and validation errors; HELD for the
  OPERATOR quiet-box window (shared with OPT32/PF18b/Q1); Q1 reuses the
  harness. needs: OPT2. (OPT2 VERDICT wave 84.)
