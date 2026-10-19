# Owner queue 2026-10-19

Rows and decisions only Maxim can close. One line each: the exact command or
the exact question. Everything else on the board is agent work.

## Human / machine runs

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
- API2 leftovers: `missing_docs` in 38 crates, a zero-warning `cargo doc` and
  the doctest leg. Which of the three is the bar? (Since faa2c442 the coverage
  ratchet itself is green again: `node tools/docs-coverage.mjs` RESULT PASS, 0
  regressions over 39 crates — the tool read a doc comment above a multi-line
  `#[expect(` group as missing. The three legs above are still open and are
  mechanical agent work, not a blocked question.)
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
- GX14 (docs/SPRINT-03-finish-line.md): the pilot is measured and the bar is
  posted as FAIL (receipt `docs/receipts/2026-10-19-gx14-gtao-pilot.md`,
  evidence `target/evidence/gx5/`). The Accept's p95-at-1080p stamp cannot be
  produced: the PF2 ring holds p50/p99 only, the window prints the gtao mean,
  and `WINDOW_SIZE` is a const with no `--size` flag — all three outside this
  row's owned paths (`gtao.rs` untouched). Name the row that lands the p95
  stamp/flag, or reword the bar to p50/p99.

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

