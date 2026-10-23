# Tick audit 2026-10-23: 30 suspects, 29 verified, 1 unticked

Scope: the finish-line row 9 sentence "`finish.mjs` reads 10 met / 13 partial / 3 open, but it scores ticks; 30 of 1,257 ticked rows are suspect" (`docs/plan/finish-line.md`). This receipt checks that sentence against the tools and the rows' own evidence.

## What was run

| Command | Result |
|---|---|
| `node tools/finish.mjs` | `Summary: 10 met, 13 partial, 3 open`. The scoreboard also flags 3 gate rows: `RT7` and `PQ4` (open rows) and `PF8` (archived) as `path-missing`. Those 3 are not part of the ticked-row count. |
| `node tools/ticked-rows-audit.mjs --no-history` | `SUMMARY 30 suspect row(s) of 1258 ticked [crate-missing 5, filter-missing 5, path-missing 22, path-moved 3]` (827 cargo test commands with `-p`, 1317 source paths checked, 343 rows name neither, 364 rows read from `docs/archive`). This is the doc's count. The doc wrote 1,257; the board is live. A second run minutes later read 1,259 ticked and the same 30 suspects. |
| `node tools/ticked-rows-audit.mjs` (default) | `SUMMARY 37 suspect row(s) of 1502 ticked`. The 7 extra rows come from the two older moved-row snapshots the default run also reads (`docs/archive/SPRINT-2026-10-05.md`, `docs/SPRINT-ARCHIVE.md`). They are outside the doc's 30; see "Default-run extras". |

After the PX21 untick the same command reads `SUMMARY 29 suspect row(s) of 1259 ticked`; the 29 are the verified static flags below.

The audit is static. It reads the row text and checks crate names, test targets, test filters and `crates/...` paths against `git ls-files` and git history. It never runs cargo, and it does not read in-row closure notes. That last point is why most of the 30 still flag today.

## How each row was checked

For each suspect: the cited run id with `gh run view <id> --repo maximkone/engine2040-public --json conclusion`; the cited file, test or test filter at HEAD; the cited commit with `git show --stat <sha>`. The row's own text was the bar. No cargo was run. All 30 suspect files were clean at audit time, so none was deferred.

## Result: 30 suspect, 29 verified, 1 unticked, 0 deferred

Short names: `SPRINT-NN` is `docs/SPRINT-NN-*.md`; `archive-done` is `docs/archive/SPRINT-done-2026-10-10.md`; "owner queue" is `docs/plan/2026-10-19-owner-queue.md`.

| Row | Where | Static flag | Verdict | Evidence checked |
|---|---|---|---|---|
| NS31e2 | SPRINT-01 | crate-missing `engine-viewer` | verified | `c7082f36` adds `crates/material/src/coverage.rs` with its 4 tests; run 39207000455 success; `crates/viewer` was dropped on purpose in `1296f273`; the owner queue's 2026-10-20 note records the close as landed. |
| GP8 | SPRINT-07 | path-missing `camera_feel.rs` | verified | `033aaca3` adds `crates/game/src/fighter_look_cam.rs`; the four named tests exist in `camera.rs`, `camera_rig.rs` and `fighter_look_cam.rs`; the owner queue lists GP8 as replaced. |
| PX21 | SPRINT-08 | path-missing `physics/kcc.rs` | **unticked** | `ee98c096` put `platform_ride_matrix` in `kcc.rs`; `87463faa` (LEG6) deleted `kcc.rs`; no step-off or ballistic test replaced it; `controller2::tests::platform_ride_keeps_station` covers the station only; run 39101688073 passed before the deletion. |
| NV10 | SPRINT-08 | path-missing `nav/crowd.rs` | verified | `f32f2e25`; the five named tests are in `crates/nav/src/orca.rs`; runs 39105710580, 39107075357, 39106089664 success; the owner queue lists NV10 as replaced. |
| SL29 | SPRINT-09 | path-missing `net/hash.rs` | verified | `3ba9bfd`; run 39057462548 success; `crates/net/src` imports `engine_core::Fnv64`; no `net/hash.rs` (the row says the second hash owner was reverted). |
| E28 | SPRINT-10 | path-missing `physics/solver.rs` | verified (closed obsolete) | `3d6ee4a1` deleted `solver.rs`; the row's own call is "Landed obsolete"; the owner queue lists the 32 obsolete rows. |
| E38 | SPRINT-10 | path-missing `render/gi_dyn.rs` | verified | `eb440e64` moves the post WGSL into `crates/render/src/shaders/post/*.wgsl`; `gi_dyn.rs` was a dead spike and the owner dropped that leg (owner queue). |
| NS4 | SPRINT-11 | path-missing `physics/verify.rs` | verified | `7c89af3` and `crates/physics/tests/it/oracle.rs`; `fifty_pose_oracle_report_stays_inside_its_tolerances` exists; runs 39473474246, 39471098511, 39471097667, 39476434413 success. |
| BX9 | SPRINT-17 | path-moved `net/server.rs` | verified | `1d9e19f76` adds `cuts.rs` and `cut_soak.rs`; `bounded_soak_holds_64_clients_at_8_cuts_per_second` exists; run 39926267175 success; the server moved to `dedicated.rs` and `session.rs` (the row's own rot note). |
| EG5 | SPRINT-18 | path-missing `physics/solver.rs` | verified (caveat) | `solver.rs` is gone; the remaining wall-clock asserts in `lod.rs` and `islands.rs` are `#[ignore]`; the rewritten cloth's 500 us budget runs only under `--release`, so the default debug suite is clean. See second pass. |
| EG31 | SPRINT-18 | filter-missing `fsync_order`, `try_from_ids` | verified | Replacement tests exist: `save_generation_reloads_identical_bytes` and `failed_write_preserves_prior_valid_generation` (`save/src/lib.rs`, `editor/src/durable.rs`), `tile_ids_stay_exact_at_the_registry_bound` (`nav/src/tiles.rs`), `nan_inputs_never_round_to_zero` (`nav/src/mesh.rs`). |
| AD14 | SPRINT-18 | path-moved `observe/tests/circuit_vectors.rs` | verified | `42b025c00` adds `observe/src/breaker.rs` and `supervisor_proof.rs`; runs 39857706182, 39857985693, 39858202368, 39858369096, 39858539730, 39858714068, 39858901387 success; the test moved to `tests/it/`. |
| AX23 | SPRINT-18 | path-moved `endpoint/tests/mcp_stdio.rs` | verified | `14fd8625c` adds `scene_query/diff.rs` and `diff_tests.rs`; runs 39922552777, 39923402873, 39922552719, 39934122291 success; the test moved to `tests/it/`. |
| PX41 | SPRINT-20 | filter-missing `projectile_exact`/`ballistics`; path-missing `ballistics.rs` | verified (closed obsolete) | `ballistics.rs` is gone; `git grep step_round -- crates/game` is empty; the owner queue closes PX41 as obsolete. |
| AIN3 | SPRINT-25 | crate-missing `-p endpoint` | verified | `d7d9914` wires the view host; `view_` tests exist in `crates/endpoint/src/packs/view.rs`; `endpoint` is a typo for the package `engine-endpoint` (the row's proof uses the right name). |
| GS43 | SPRINT-26 | path-missing `crates/bevy_pbr/...` | verified | That path is the donor path, not ours; `crates/render/src/contact_shadow.rs` has 5 tests and `contact_shadows_drop_under_both_feet_on_a_live_frame` is in `gtao_live.rs`; `baf9c07`; the capture pair is under `target/clips/gs43-final-on/`. |
| PW47 | SPRINT-28 | path-missing `assets/source.rs` | verified | `f8eb3c9eb` adds `asset_source.rs`, `asset_source/fetch.rs`, `load_queue.rs`, `handle.rs`, `kit_assets.rs`; the four named `load_queue` tests exist; runs 39596950970, 39598950063, 39599440658, 39598332625, 39664985931, 39664975796, 39664962318, 39698918656, 39712748080 success. |
| LEG6 | SPRINT-28-rome | path-missing `physics/kcc.rs` | verified | `87463faa` adds `world/src/dig.rs` and deletes `kcc.rs`, which is the row's own target; runs 39562315663, 39562484902, 39562686678, 39562892763 success. |
| BV1 | archive-done | crate-missing `engine-viewer` | verified (closed dead) | `1296f273` drops `crates/viewer` on purpose; the row's 2026-10-19 note records it; no live viewer work. |
| BV2 | archive-done | crate-missing `engine-viewer` | verified (closed dead) | Same crate drop; the row's note records it. |
| BV4 | archive-done | crate-missing `engine-viewer`; path-missing `viewer/material.rs` | verified (closed dead) | Same crate drop; the material path went with it. |
| RL20 | archive-done | path-missing `render/tunnel_mouth.rs` | verified (closed dead) | `f737386e` render wip sweep removed it; the owner queue closes RL20 as dead. |
| NV2 | archive-done | path-missing `nav/crowd.rs` | verified (replaced) | `crates/nav/src/orca.rs` holds the crowd work; the owner queue lists NV2 as replaced. |
| PX3 | archive-done | path-missing `physics/grapple.rs` | verified (replaced) | `grapple.rs` is gone; the owner queue lists PX3 as replaced. |
| PX28 | archive-done | filter-missing `cloth_couple` | verified | `a_capsule_keeps_the_cape_off_the_body` and `colliders_push_to_the_surface_and_leave_outside_points` exist in `crates/physics/src/cloth/tests.rs`. |
| PX29 | archive-done | filter-missing `cloth_self` | verified | `cloth_gap_a_folded_curtain_never_self_penetrates` exists in the rewritten cloth tests; the intercollision set is in `cloth/intercollision.rs`. |
| PX31 | archive-done | path-missing `physics/substep.rs` | verified (replaced) | `3d6ee4a1` deleted `substep.rs`; the owner queue lists PX31 as replaced (Rapier). |
| SL4 | archive-done | path-missing `ecs/systems.rs` | verified | `da6b7922` deletes `systems.rs`, which is the row's own target; `ecs/src/schedule.rs` stays. |
| SHOW1 | archive-done | path-missing `playtest/showcase.rs` | verified | `tools/showcase.mjs` exists and stitches and validates the manifest; `showcase_manifest` is in `crates/game/src/camera_feed.rs`; the playtest file never landed (the row's note). |
| E52 | archive-done | path-missing `assets/schema_compile.rs` | verified | Removing that file was the row's purpose; the workspace module scan is clean as the row says. |

## The one untick

`PX21` is the only row whose evidence does not support its tick anymore. Commit `ee98c096` added `kcc::tests::platform_ride_matrix`, which checked the step-off landing tick and x against an independent f64 ballistic prediction within 1 cm. Commit `87463faa` (LEG6) deleted `crates/physics/src/kcc.rs`, and no step-off or ballistic test replaced it. `crates/physics/src/controller2.rs:1237` still proves the station (`platform_ride_keeps_station`), but the Accept's "step-off lands within 1 cm" leg has no test. Run 39101688073 passed at the time; that is history, not current evidence. The behavior (`ride_platform`) still exists; the test is what is gone.

The row is unticked in `docs/SPRINT-08-hard-stuff.md`. Its old text, run id and proof line are kept, and one audit line says why.

## Default-run extras (outside the 30)

| Row | Where | Disposition |
|---|---|---|
| CT29a | `docs/archive/SPRINT-2026-10-05.md` | The owner queue lists it as replaced. |
| AM7 | `docs/archive/SPRINT-2026-10-05.md` | The owner queue lists it as LATER, unticked and parked; the old snapshot is the record. |
| GA4 | `docs/archive/SPRINT-2026-10-05.md` | No subsumption code in `crates/ai`; the row's own note says the path never landed. No disposition yet; second pass. |
| PLF3 | `docs/archive/SPRINT-2026-10-05.md` | No Steam code in `crates/net`; the row's note says the code stays deleted. No disposition yet; second pass. |
| PF8 | `docs/SPRINT-ARCHIVE.md` | The owner queue lists it as replaced. |
| SL39 | `docs/SPRINT-ARCHIVE.md` | The owner queue lists it as replaced. |
| WD10 | `docs/SPRINT-ARCHIVE.md` | The owner queue lists it as LATER; the archive is left as the record. |

## Paths touched

- `docs/SPRINT-08-hard-stuff.md`: PX21 unticked and one audit line added.
- `docs/plan/finish-line.md`: row 9 evidence and next-proof cells now quote this audit.
- `docs/receipts/2026-10-23-tick-audit/README.md`: this receipt.

No `crates/**` file and no tool changed. No cargo, no golden run; this is a docs and evidence audit. `node tools/workspace-check.mjs` passed and read 467 of 847 public modules with no caller outside their crate, not 468; row 9's cell now carries the measured number.

## What a second pass still needs

1. Teach `ticked-rows-audit` the in-row closure notes (or a machine-readable disposition). The 29 verified flags are deliberate closures: donor paths, paths deleted as the row's own target, and rows closed by replacement. Without a reader, the audit can never reach 0 and every run repeats this work.
2. Give `GA4` and `PLF3` an owner disposition, or untick them in the old snapshot. Their own notes admit the named path never landed, and no replacement code exists.
3. Decide whether the release-only cloth budget in `eight_capes_step_under_half_a_millisecond_in_release` should be `#[ignore]` like the other timing tests. It does not assert in the default debug suite, so EG5's claim holds there; a release `cargo test` can still flake on it.
4. Restore a step-off test for PX21, or record that the leg is accepted as untested and re-tick with that reason.
5. Refresh the finish-line cells that lag (G5, G10, B8) and then quote `finish.mjs`; the scoreboard still reads 10 met / 13 partial / 3 open.

## Second pass (2026-10-23)

Items 1, 2 and 4 of the list above are done (item 4 by the physics lane, not this pass). The audit now reads closure notes and has tests, `GA4` keeps its tick with a written negative, `PLF3` is unticked, and `PX21` is back ticked with a new step-off commit. Items 3 and 5 are still open.

### What the tool reads

`tools/ticked-rows-audit.mjs` reads a row's closure note only when that row already has a static finding. A note starts at a closure marker and ends at the next marker:

`Tick audit ...:`, `Owner chat ...:`, `Correction ...:`, `Done <date> ...:`, `Landed <sha>`, `Landed obsolete`, `closed as|by`, `replaced by`, `superseded by`, `REPLACED:`, `rot:`.

Running prose is not a note: `Progress 2026-10-19:` is the row's record, so a path named there cannot excuse a stale one.

The note clears the finding only when it names evidence this tree can check:

| Evidence | Check |
|---|---|
| commit | `git cat-file -e <sha>^{commit}`; a digest, a golden hash or a typo fails |
| run id | written as `run <id>` / `runs <ids>`, 9-12 digits; shape only, GitHub is not reachable offline |
| file or path | in `git ls-files`: exact, crate-relative (`nav/orca.rs`), or inside the crate that lost the row's path; `<stem>/mod.rs` too |
| replacement row | `replaced by <ID>` / `superseded by <ID>`, and that row must be ticked |

A note that says `LATER` never clears: a park is not a close. A note with no such evidence keeps the row a suspect. The first note with verified evidence wins and is printed in the `--json` `closures` array (id, source, reasons, evidence, note), so a cleared row keeps its receipt. Human output and `SUMMARY` are unchanged; `closures` is additive.

`tools/finish.mjs` imports the same `auditRows`, so the scoreboard gets the same reader. It passes the whole ticked board as `tickedIds`, because the gate set alone cannot tell whether a replacement row is ticked. That is why the scoreboard's suspect count (2) and this CLI's (6) differ: the CLI audits every ticked row, the scoreboard audits gate rows.

### Tests

`node --test tools/ticked-rows-audit.test.mjs` reads `28 pass, 0 fail`.

- `closureNotes`: a marker starts a note, the next marker ends it, and `Progress ...` is not a note.
- `closureEvidence`: a real commit clears; a bogus sha does not; a note with no evidence does not; a `LATER` note does not; `replaced by` clears only for a ticked row (not an open or absent one); an existing path clears; a gone path does not; a bare number is not a run id, `run <12 digits>` is.
- `auditRows`: a note with a commit drops the finding and lands in `closures`; a note with nothing keeps the row a suspect.
- CLI fixture repo (a file committed then deleted): real sha exits 0 and the JSON names `[{kind: commit, value: sha}]`; the same note with `deadbee` exits 1; no evidence exits 1; `LATER` exits 1; `replaced by <ticked>` exits 0; `<open>` and `<absent>` exit 1; a path in the row body, outside a note, exits 1.
- `tools/finish.test.mjs`: `loadSuspects drops a gate row only when its closure note names evidence` (real HEAD clears, `deadbee` does not, an existing path clears).

### Counts

| Run | Before the pass | After |
|---|---|---|
| `node tools/ticked-rows-audit.mjs --no-history` | `29 suspect of 1259` | `4 suspect of 1268` |
| `node tools/ticked-rows-audit.mjs` (default) | `37 suspect of 1502` | `6 suspect of 1511` |

Before is the first pass's post-untick run; after is the run at 2026-10-23T20:20Z. The ticked count grew between them from other lanes' rows, so the suspect set is the measure: 29 to 4, and 37 to 6.

`node tools/finish.mjs` still reads `10 met, 13 partial, 3 open`; its only suspect gate rows are `RT7` and `PQ4` (open rows), down from three because `PF8` cleared. `--check` reads `0 met gates with a suspect row`, and its one failure is `SL3` (below).

### Rows the reader clears (30)

Each line is the row, its static flag, and the strongest evidence the note named. The full list, including every run id and repeated file, is in the JSON `closures` array.

| Row | Flag | Evidence |
|---|---|---|
| NS31e2 | crate-missing, path-missing | `c7082f36` |
| GP8 | path-missing | `033aaca3`, `crates/game/src/camera_rig.rs` |
| PX21 | path-missing | `64d431e87`, `ee98c096` (re-ticked by the physics lane today, not this pass) |
| NV10 | path-missing | `f32f2e25`, 3 runs, `crates/nav/src/orca.rs` |
| SL29 | path-missing | `3ba9bfd` |
| E28 | path-missing | `3d6ee4a1` |
| E38 | path-missing | `eb440e64` |
| NS4 | path-missing | `7c89af3`, 4 runs |
| BX9 | path-moved | `1d9e19f76`, run 39926267175 |
| AD14 | path-moved | `42b025c00`, 2 runs |
| AX23 | path-moved | `14fd8625c`, 2 runs |
| AIN3 | crate-missing | `d7d9914` |
| GS43 | path-missing | `baf9c07` and 6 more commits |
| PW47 | path-missing | `f8eb3c9eb`, 8 runs |
| LEG6 | path-missing | `87463faa`, 4 runs |
| BV1 | crate-missing | `1296f273` |
| BV2 | crate-missing, path-missing | `1296f273` |
| BV4 | crate-missing, path-missing | `tools/rust-quality.mjs`, `tools/workspace-check.mjs` |
| RL20 | path-missing | `f737386e` |
| NV2 | path-missing | `crates/nav/src/orca.rs` |
| PX3 | path-missing | owner queue doc |
| PX28 | filter-missing | `crates/physics/src/cloth/mod.rs` |
| PX29 | filter-missing | `331dca1b` |
| PX31 | filter-missing, path-missing | `3d6ee4a1` |
| SL4 | path-missing | `da6b7922` |
| SHOW1 | path-missing | `tools/showcase.mjs` |
| CT29a | filter-missing, path-missing | owner queue doc |
| GA4 | path-missing | `docs/plan/gameplay.md`, `docs/archive/2026-10-10/plans/gameplay-cut.md` |
| PF8 | path-missing | owner queue doc |
| SL39 | path-missing | owner queue doc |

"owner queue doc" is `docs/plan/2026-10-19-owner-queue.md`: the note names the file, the file exists, and its row says the close.

The 30 are the first pass's 25 cleared rows, plus the four default-run extras the reader also clears (`CT29a`, `PF8`, `SL39` by the owner queue doc, `GA4` by this pass), plus `PX21`, which is a different action by another lane.

### Refusals (4)

These four rows still flag. Their notes are prose, not evidence: none names a commit git knows, a run id, an existing path, or a ticked replacement row.

| Row | Note says | Why it does not clear |
|---|---|---|
| EG5 | "physics now runs on Rapier" | `crates/physics/src/solver.rs` is gone; no commit, run or live path is named. The 500 us cloth budget is release-only and the wall-clock asserts are `#[ignore]`, so the claim holds only in the default suite. |
| EG31 | names the replacement test fns | A bare fn name is not a path; the note names no commit or run. |
| PX41 | "closed obsolete" and `step_round` | `crates/game/src/ballistics.rs` is gone and `step_round` is only a symbol; the note names no commit, run or live path. |
| E52 | "the workspace module scan is clean" | `crates/assets/src/schema_compile.rs` is gone; no commit, run or live path is named. |

In the default run, `AM7` and `WD10` also still flag: both notes say `LATER`, and the reader treats a park as a park.

To clear any of these, the note has to name the evidence (the removal sha, the run, or the path that now holds the work); to close them for good, untick them the way `PLF3` was. Neither was done here: this pass changed only the rows below.

### GA4 and PLF3

- **GA4** stays ticked in `docs/archive/SPRINT-2026-10-05.md`. Its Accept has an "or dies with a recorded negative" leg, and the negative is recorded: `docs/plan/gameplay.md:68` reads "Subsumption melee stack (GA4) | experiment killed with numbers | spike cost 2.00x against a 3x bar, win-rate neutral", and `docs/archive/2026-10-10/plans/gameplay-cut.md` records `crates/ai/src/subsume.rs` absent by design, the killed shape kept as a negative receipt, never a second brain. The row note now says that instead of "the path was never landed"; the `subsume.rs` path never landed and no code is owed.
- **PLF3** is unticked in the same snapshot. `3151bd46` deleted `crates/net/src/steam_relay.rs`, `crates/net` holds no Steam code, no replacement row covers the Steam-join Accept, and the QUIC path alone is not the proof. The old note and the new audit line are both kept.

### Open red, not this pass

- `node tools/finish.mjs --check` exits 1 on `SL3`: `1 gate row(s) on neither the board nor the archive`. `node --test tools/finish.test.mjs` has three live-tree failures with the same cause. `SL3` is on no live SPRINT file, no archive file and not in `docs/plan/parked-2026-10-22.md`, and the warning was there before this pass edited anything. It is a board gap for another lane.
- `node tools/workspace-check.mjs` fails on `engine2040 -> engine-arena: undocumented local dependency edge`. That edge is in an uncommitted `Cargo.toml` edited by another lane; no file of this pass touches it.

### Paths touched by the second pass

- `tools/ticked-rows-audit.mjs`: closure-note reader, `commitExists`, `closures` in `--json`; header and `--help` text.
- `tools/ticked-rows-audit.test.mjs`: marker, evidence and `auditRows` tests plus the CLI fixture repo.
- `tools/finish.mjs`: passes the ticked board to `loadSuspects`.
- `tools/finish.test.mjs`: the `loadSuspects` closure test.
- `docs/archive/SPRINT-2026-10-05.md`: `GA4` note rewritten, `PLF3` unticked with an audit line.
- `docs/receipts/2026-10-23-tick-audit/README.md`: this section.

No `crates/**` file changed. No cargo and no golden run: the tool is Node, the rows are text.

## Follow-up: PX21 came back

PX21 was re-ticked at `64d431e87` (board `736d769d5`). A PX21 seat found the audit line was half right: `87463faa` (LEG6) deleted the step-off carry itself, not only its test. The old `apply_deck` saved the deck travel and added it back as velocity when the rider left; the replacement moved grounded riders only, so a rider walking off a moving deck landed short. `controller2::ride_platform` now returns the delta and `step_walker` hands it to a rider that leaves that tick (`inherit_ride`, delta x 60 mm/s, saturating). The restored `platform_ride_matrix` proves 0 mm station drift over 600 moving + 60 stopped ticks, a 4200 mm/s launch (wish 3000 + carry 1200), landing tick 21 with 0.7 mm error against the 10 mm bar, 26 mm past a no-carry flight, and hash x2; a negative control with the carry zeroed fails 3000 vs 4200, so the test exercises live behavior. Full physics lib 231 passed; golden unchanged (run 40017447717). The 30-row table below keeps its dated finding; this line is the closure.
