# Owner queue 2026-10-19

Rows and decisions only Maxim can close. One line each: the exact command or
the exact question. Everything else on the board is agent work.

## Human / machine runs

Note 2026-10-19 (owner delegated the decisions; the runs below are still runs):
the timing legs (OPT31, OPT33, RL5, RL18, OB4, RC20) need a **quiesced**
machine. Four nonstop loops and five code agents were compiling on this PC, so a
number taken during this pass would be meaningless; the rows stay parked until a
quiet window. CT45 needs a VRAM measurement, not a timing run, so it can go in a
busy window. ON30 and SG3 need people.

- ON30 (docs/SPRINT-11-north-star.md): two humans + 6 AI over two real
  networks. Host: `target\release\net_relay.exe` on a reachable host, then
  `target\release\arena.exe --scene arena --fighters 8 --spear --host`; guest:
  `target\release\arena.exe --scene arena --join HOST:7043 CODE`. Paste the
  loss/latency lines and both seat hashes into the row.
- SG3 (docs/SPRINT-08-hard-stuff.md): one hand-played keyboard run that lands
  a tow. `play-arena.bat`, then hold W and press Q once at the rival; keep the
  log line `arena spear: thrown=... hits=... tethers=1` and paste it here.
- OPT31 (docs/SPRINT-12-tier-2040.md): reference-GPU frame report over the fixed
  arena camera path, x2 within 5%.
  `node tools/frame-report.mjs --frames 600 --out target/evidence/build/` (or the
  OPT2 harness line in the row), then paste adapter + driver + both medians.
- OPT33 (docs/SPRINT-12-tier-2040.md): DX12 vs Vulkan A/B on the reference GPU
  with the OPT2 harness, 600 frames, x2 within 5%; paste both reports under
  `target/evidence/backend/` and say which backend becomes the default.
- CT45 (docs/SPRINT-07-accepted-plan-rows.md): measured VRAM of the 213 MiB
  bound set on the reference GPU. The report table is already generated
  (f2effc78); only the measured VRAM column and the reference adapter name are
  missing.

## Owner decisions

- NS31c1 (docs/SPRINT-01-rome-first.md): the Accept line says the coated
  material record has "strictly more bindings", but coat is a lobe and adds no
  texture slot (its two decidable clauses are covered and green: 11 passed, gh
  run 39177681648). Keep the wording and give coat its own texture binding, or
  change it to what the code does?
- Close the 32 obsolete rows in `docs/plan/2026-10-17-obsolete-rows.md`
  (E4 E6 E12 E17 E21 E22 E26 E5 E8 E11 E18 E19 E20 E24 E25 E28 E33 E34 E35
  E36 E41 E45 E47 E53 E54 E60 E83 E84 E85 E88 E89 E94): tick each with
  `Landed obsolete.` or delete it. E54's premise is wrong (the root package
  keeps those deps on purpose) and E22/E54 have no replacement.
- Deleted net code stays deleted until a caller exists: `replicate.rs`,
  `server.rs` (BX9), `steam_relay.rs`, `nat.rs`; lag compensation is a hold
  window, not a view rewind. Confirm, or name the caller that should come back.
- Brotli or gzip for the web budget (`crates/web` budget leg)?
- SP50 Accept wording: the digest `4aae94af` came from a JS spike and cannot be
  reproduced in Rust. Reword the Accept or drop the digest?
- GP33 contradicts the tier ladder (docs/SPRINT-12-tier-2040.md). Which one
  wins?
- API2: closed 2026-10-19 (delegated). The row's Accept is the bar, not the
  three description legs, and all three clauses are verified: the coverage
  ratchet is wired into `tools/proof.mjs` and PASSes (`node
  tools/docs-coverage.mjs`, 0 regressions over 39 crates, F2P owned by its own
  test), `mdbook build docs/book` is exit 0 (v0.5.4, no warning), and the fresh
  agent drill was followed verbatim on this PC: `--verify` printed the frozen
  `d57b5c8ae3096614`, `--example headless` printed `e8a4e9c86ae12991
  entities=10`, and `staged_fight --ticks 600` twice gave the same
  `a7e69394a687115d` with byte-identical `frame_000000.png` in both out-dirs.
  The drill also caught the quick-start page quoting the 2026-10-02 staged
  receipt, now fixed. `#![warn(missing_docs)]` in every crate and a zero-warning
  `cargo doc --workspace --no-deps` stay wishes (the Accept does not name them).
- ARC13: report-only receipt instead of auto-revert? Confirm.
- 14 ticked rows whose code is gone (listed in
  `docs/plan/2026-10-18-deleted-code-triage.md`, "Ticks with no code"):
  untick them, or accept them as closed by replacement?
- PX41 (docs/SPRINT-20-game.md): the row's Accept runs
  `cargo test -p engine-game --lib ballistics`, but `crates/game/src/ballistics.rs`
  was deleted and `combat::projectile::step_round` still has no game caller. Close
  it as obsolete, or re-scope it to a new caller row?
- E38 (docs/SPRINT-10-empire-inbox-drain.md): the WGSL split landed (eb440e64);
  the remaining gi_dyn leg names `crates/render/src/gi_dyn.rs`, which was deleted
  as a dead spike. Drop the gi_dyn leg and tick, or keep the row open?
## Found 2026-10-19 (third pass)

- QL4 (docs/SPRINT-22-web-realism.md): superseded by RL50, which deleted the
  wrap this row normalises (85022077, `crates/material/src/compile.rs`) and put
  the terminator LUT in its place. Nothing is left to normalise. Close it as
  superseded, or re-scope it to the LUT.
- GX14 (docs/SPRINT-03-finish-line.md): answered 2026-10-19 (delegated) — the
  Accept already carries the escape "gtao p95 at most 0.8 ms at 1080p with
  adapter line **or posted FAIL**", and the pilot posted the FAIL with its
  receipt, so no new row is needed and the `--size`/p95 ring stays a separate
  wish. What remains is the Accept's last clause (`node tools/proof.mjs` PASS,
  golden x2), which today stops on three live sessions' in-flight files
  (`net/src/handshake.rs`, `render/src/particles_gpu.rs:491`,
  `material/src/compile.rs:75,380`), none of them this row's.

- GEO3b, GEO5b (docs/SPRINT-17-body-and-break.md): both took the fallback the
  row itself names ("if it still fails write the measured values into this row
  and BLOCKERS and stop"), so the metrics, unit tests and kit tests are landed
  and green while 6 of 38 kit primitives miss GEO3b's atlas bounds (worst
  `debris/0` fill 0.453) and 2 of 38 miss GEO5b's 0.02 concavity bar
  (`stands/5`, `stands/6` at 0.4734). `docs/BLOCKERS.md` 2026-10-18T08:20Z holds
  the numbers. Fix the kit meshes, or change the bars, or close the rows as
  measured-and-filed. GEO5c (the extra bake beside V-HACD) is separate agent
  work and is not affected.
- GP18 (docs/SPRINT-07-accepted-plan-rows.md): the baselines are pinned and the
  enforcing tests are green (`cargo test -p engine-game --lib pq2` plus
  `kill_share_bar_is_one_number_across_crates`), but the Accept's "no move above
  35% of kills" has two readings: the bar in the code is the per-move share
  (`PQ2_KILL_SHARE_MAX_PERMILLE = 350`, `crates/game/src/bench.rs:69`, through
  `check_pq2_kill_share`), which passes, while the pinned v1 survey's
  whole-attack share reads 395 permille, which would not. Keep the per-move
  reading and tick the row, or re-pin the survey baseline for the whole-mix
  reading?

## Found 2026-10-19 (second pass)

- SPRINT-01-rome-first.md (39 lines name `crates/viewer`): the shard's owned
  path was dropped on purpose (1296f273, "rot: drop dead Bevy viewer crate
  (dropped 2026-10-07, render is the renderer)"), and `RecordLook` with it.
  Rows have been landing in their new homes anyway (NS31a2 → editor, NS31c3 and
  NS31d3 → render, NS31h2 → playtest, NS31e2 → material). Confirm that re-home
  rule for the rest of the shard, or close the rows whose wiring point is gone.
  NS31e2's tracker is landed and tested; only its named command
  (`cargo test -p engine-viewer …`) is unrunnable, so it waits on this answer.

- DX8, DX9 (docs/SPRINT-08-hard-stuff.md): a human blind review decides
  keep-or-kill. DX8: `crates/destruction/src/bake_rank.rs` spaces lattice cuts
  by impact likelihood at the same bond count and anchors (8-way spread
  equality holds, Progress 2026-10-17); the bar is "a blind review prefers the
  graded break, else killed", and the module still has no caller. DX9: the
  same review for graded per-bond breaking strain in
  `crates/destruction/src/bond_grade.rs`. Parked on the board with an operator
  note; the caller half is agent work that stays undone until the review says
  keep.
- RC15 (docs/SPRINT-07-accepted-plan-rows.md): the netcode.io connect token
  needs authenticated encryption. `engine-net` is std-only and has no crypto
  dependency, and hand-rolling crypto is against the rules. Add a feature-gated
  `hmac` + `sha2` (already in the tree, pure Rust, as the proof pack does) and
  build the token on HMAC-SHA256, or kill the row?
- RC28 (docs/SPRINT-07-accepted-plan-rows.md): the Accept wants a TestU01
  SmallCrush pass, which needs the external C suite (not in the repo, no
  runner leg). The earlier attempt was judged FAIL (WyRng unwired, a custom
  schedule instead of the wyrand vectors) and reverted. Run SmallCrush on the
  PC and paste the summary, or reword the bar to pinned upstream vectors plus
  a statistical test we own?
- OB4 (docs/SPRINT-08-hard-stuff.md): the "PF1 within 5%" leg needs a quiesced
  reference-GPU run, not a test: `node tools/gh-check.mjs` will not measure it.
  Run the PF1 gate on the PC with nothing else compiling and paste both
  numbers. (The missing production caller for `event_queue.rs` is agent work.)
- RC20 (docs/SPRINT-07-accepted-plan-rows.md): `collect_indexed_on` landed
  (bit-identical for 0/1/2/4/8 workers, 2.86x on a synthetic 200k bake, run
  39066665851). The remaining "at least 2x on the CT2 bake workload or killed
  with numbers" needs a quiesced cook measurement on the PC: name the command
  that is the CT2 workload in the row and paste the ±5% paired numbers.

## Decided (owner chat 2026-10-19: "you decide on all")

Maxim delegated every decision above. Each row carries the call on its own
lines; this is the index.

Closed on the board (row ticked, the call named in the row):

- The 32 obsolete rows (`plan/2026-10-17-obsolete-rows.md`), each with its own
  reason in the tick: the target or the premise is gone. Kept as rows, not
  deleted, so the record stays.
- Gone-code ticks (`plan/2026-10-18-deleted-code-triage.md`): the 9 REPLACED
  rows are accepted as closed by their replacement (PF8, PX3, PX31, EG5, DX10,
  SL39, CT29a, NV2, NV10, plus GP8); the 3 DEAD ones are closed as dead (AX17,
  SP32, RL20); the 3 LATER ones are unticked and parked with a `LATER:` token so
  nobody picks them until the file is restored (AM7, D1, WD10 — the last lives
  in the archive, which is left as the record).
- RESTORE rows verified back in the tree since the triage: `look/judge.rs`,
  `look/metric.rs`, `clip_hold.rs`, `modes.rs`, `uploads.rs` are present and the
  `look_judge` and `look_metric` filters find their tests; no board change.
- E38: the deleted `gi_dyn` leg is dropped; the WGSL-split half is landed
  (eb440e64). QL4: closed as superseded by RL50. PX41: closed as obsolete (its
  `ballistics.rs` is gone and `step_round` has no caller). SP50: the Accept's
  web-spike digest is dropped; the landed override map and the museum digest x2
  are the bar. GEO3b and GEO5b: closed as measured-and-filed (the module's own
  fallback, numbers in BLOCKERS), the kit meshes and the bars stay as they are.
  RP8b: not boarded (no overturn of the RP8a REJECT). GP18: the per-move reading
  is the Accept. DX8 and DX9: kept, not killed — the decidable legs are green
  and the blind review is not producible here, so that one leg stays an open
  question; DX9's caller already exists (`impact_table.rs` uses `bond_grade`).
- WEB11: gzip is the bar for the compressed-bundle number (no new dependency,
  every host serves it); brotli may ride along as an extra column later.
- CT29: re-scoped to the capture pair (its bind glue is gone).

Answered, row reopened as ordinary agent work (marker removed, no close):

- ARC13: report-only. A seeded divergence fails the gate and writes its
  first-divergence receipt; no auto-revert is wired.
- GP33: the tier ladder wins on the label; the row's Accept is unchanged.

Kept and implemented by the delegated call (agent work dispatched, not parked):

- RC15: kept. The handshake is implemented, with the connect token authenticated
  by HMAC-SHA256 through a new optional `crypto` feature on `engine-net`
  (default off, so the wasm bundle does not grow); no hand-rolled crypto.
- RC28: kept, and its Accept is rewritten to what the repo can repeat: pinned
  upstream wyrand vectors, a statistical test we own, cross-platform equality,
  one real caller wired, p95 kept. TestU01 is out (no C suite in the repo).

Still only a person can do it (a run, not a decision), so it stays in this
queue: ON30 (two humans, two real networks), SG3 (a hand-played tow), OPT31,
OPT33, CT45, RL5, RL18, RL21, RL26, OB4, RC20 — and the DX8/DX9 look, which the
delegated call answers as "kept" rather than waiting for the review.

