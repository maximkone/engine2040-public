# PROPOSED-ROWS — fat-row splits (proposal only)

Date: 2026-10-06.
Status: proposal only. No board edits. No commit.
Source: `node tools/board-slice.mjs` for BV1, BV2, BV5, BV6, GM3, NS32, NS32a, NS32b, NS32c, NS32d, NS32e, NS32f, GEO2, GEO12.
E-cards read: E01 triage (L rows split before resend), E03 stuck (VS1.a/b/c chain, S never splits), E05 ladder (free ID check).
Rule used: plan README Sizes (S = 1, M = 2-4, L = 5-10, split anything bigger); MP-SPLIT (Size S with one file never splits; Size M with paths can split; chain a to b to c; only first undone piece is READY).
Rule kept: no text deleted. Each split keeps all accept steps of its parent. Original rows stay as they are.

## BV1 (done, Size L, needs: none)

Why fat: crate plus placement plus hero plus capture plus docs plus golden plus deny plus workspace in one row. L must split.
Follow row already on board: BV5 (release capture plus golden, needs: BV1).

Proposal:

- [ ] BV1.a viewer crate plus placement: the viewer crate builds (crate dropped 2026-10-08, rot 1296f27), placement parse gives 11 actors over 8 kit files, hero skin has 43 joints. Owns viewer-crate new files only. Accept: `cargo test --locked -p engine-viewer --lib` for parse plus joints, x2 equal. needs: none.
- [ ] BV1.b sim place plus draw: hero transform equals sim root-table spot at ticks 0, 60, 599, x2 equal. Owns the viewer draw path only. Accept: transform test green plus pilot `frame_000060.png` with kit and hero, log `bv1 tris=<n> joints=43`, n over 5000, judge opens PNG. needs: BV1.a.
- [ ] BV1.c docs plus gates: one `docs/DEPENDENCIES.md` row, one `docs/ARCHITECTURE.md` edge, not in root `default-members`. Accept: `cargo tree -p engine-game -e normal` shows no bevy, `node tools/workspace-check.mjs` PASS, `cargo deny check` green, golden `d57b5c8ae3096614` unchanged, diff only Owns paths. needs: BV1.b.

## BV2 (done, Size M, needs: BV1)

Why fat: clip choice plus bone bind plus two captures plus side-by-side plus golden in one row.
Follow row already on board: BV6 (per-body clips, needs: BV2).

Proposal:

- [ ] BV2.a clips plus bind: hero plays CT7 idle, run over 0.5 m/s, cast on hit cue, bound by joint name. Accept: `cargo test --locked -p engine-viewer --lib` clip choice matches sim state for 120 ticks x2, sword-hand joint moves over 1 cm frame 30 to 90, 0 unresolved names. needs: BV1.
- [ ] BV2.b captures: pilot writes `target/bv1/frame_000030.png` and `frame_000090.png` with hero in two poses. Accept: both PNGs exist and judge opens them. needs: BV2.a.
- [ ] BV2.c side-by-side plus hold: cubes capture `target/pilot-gx13/owner-arena/frame_000015.png` beside `target/bv1/frame_000090.png`, golden unchanged, workspace PASS, BV1 tests green. Accept: judge opens both, `node tools/workspace-check.mjs` PASS. needs: BV2.b.

## GM3 (open, Size M, needs: GM2, WIN1, AS1 — all ticked)

Why fat: clean copy plus stripped PATH plus 5-minute run plus loud fail plus long DECISION notes in one row. Blocked until stock 5-minute proof exits 0.
Rows GM2, WIN1, AS1 are ticked, so GM3.a is ready.

Proposal:

- [ ] GM3.a short clean proof (record only): packaged folder to `C:/e2040-clean-gm3/`, dev tools stripped from PATH, no source, no Rust or Node. Command: 30-frame clean run. Accept: log under `target/gm3/` with ticks 30 frames 30 exit 0. needs: GM2, WIN1, AS1.
- [ ] GM3.b full 5-minute proof (quiet box only): one exclusive run, no shared `C:/e2040-clean/` replace, scratch target. Command: `node tools/ship.mjs run --dist target/as7-dist --target C:/e2040-clean-gm3/ --frames 18000`. Accept: 5-minute play exits 0 with log, or abort and file BLOCKERS fingerprint if wall projects past keeper; retry once on PLAY_TIMEOUT only on quiet box. needs: GM3.a.
- [ ] GM3.c receipt plus tick: file log plus `clean-run-receipt.json` under `target/gm3/`. Accept: 5-minute accept met, missing dep fails loudly, then tick GM3. Unblocks PQ3 and A8. needs: GM3.b.

## NS32 (done, Size L, needs: none)

Why fat: sandbox plus limits plus registry plus tape hash plus door verbs plus last-good in one L. Already split on board. No new IDs. Keep the split.

Chain as boarded:

- NS32 sandbox (done, needs: none).
- NS32a arena hook plus adapter (done, needs: NS32).
- NS32b door verbs `rules.set`, `rules.test`, `rules.explain` (done, Size M, needs: NS32a).
- NS32c quiet-box p99 (open, Size S, needs: NS32a).
- NS32d second-PC replay (open, Size S, needs: NS32a).
- NS32e walled scene through door (open, Size S, needs: NS32b).
- NS32f effect actions in real arena (open, Size S, needs: NS32a).

Only first undone piece in each chain is READY. S pieces never split further.

## GEO2 (done, Size M, needs: none)

Why fat: GeoMesh plus primitives plus boolean plus 20 pairs plus volume plus typed errors plus kit plus deny plus golden in one row.
Second half already on board: GEO12 weld, mirror, array, flip, normals (open, Size S, needs: none per board).

Proposal (keeps GEO12 as boarded):

- [ ] GEO2.a mesh plus primitives: shared `GeoMesh` (weld, edge census, volume, digest) plus seeded box, sphere, cylinder, torus, L prism. Accept: closed census plus digest x2. needs: none.
- [ ] GEO2.b boolean: `geo::boolean` union, difference, intersection on manifold-rust. Accept: 20 seeded pairs give union plus difference with 0 open edges, digests x2; 6 box pairs match analytic volume within 1e-4; open or bad input refused typed. needs: GEO2.a.
- [ ] GEO2.c kit plus gates: closed kit meshes of `content/arena/src` survive boolean with cutter. Accept: `cargo tree -p engine-game -e normal` shows no geo, `cargo deny` list unchanged, golden unchanged. needs: GEO2.b.

GEO12 stays open as the second half, needs: none per board. No change proposed to it.
