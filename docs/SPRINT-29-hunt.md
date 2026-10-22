# Hunt 2026-10-21: steals from outside the engine set (owner 2026-10-21)

Boarded from the research dive in `docs/research/hunt-2026-10-21/`. Every donor was
verified live on 2026-10-21 with `gh api repos/<owner>/<repo>` and the donor file
path was read, not guessed. Evidence and the full list: `docs/research/hunt-2026-10-21/`.
Rules v1 and the golden `d57b5c8ae3096614` never move for these rows.

Donor license does not matter (owner 2026-10-10: any donor code may be ported), but
the donor path, the commit and the idea are written down, so the next reader does
not re-mine the same repo.

- [x] HX6 rome: the codex pipeline. A tool reads the two public-domain sources  Landed aaca5ad5. `node tools/codex-check.mjs` prints `codex: numbers=25 unmatched=0` and exits 0 (two runs byte-equal); a table value no passage supports prints `unmatched` and exits 1; a passage whose text hash moved prints `changed` and exits 1; the ditch reads 1480 mm wide and 888 mm deep with `ref = "PHG 49"`; `node --test tools/codex-check.test.mjs` 6 tests pass; `cargo test -p engine-rome --lib` 18 passed; the golden d57b5c8ae3096614 x2 (`proof_cli default_and_max_seed` 1 passed, the release binary printed final_hash=d57b5c8ae3096614); `node tools/workspace-check.mjs` PASS.
  (Vegetius *De re militari* I, Pseudo-Hyginus *De munitionibus castrorum*) from
  the machine-readable corpus `cltk/lat_text_latin_library` (Public Domain Mark
  1.0: `hyginus/hyginus6.txt` paragraphs `1.`-`58.`, `vegetius1.txt` chapters
  in Roman numerals) and prints the numbers they carry next to
  `content/rome/camp.toml` and `content/rome/line.toml`, so a changed number in
  the table shows as a diff against the source. **The passage's text hash is
  stored beside the ref**, so a re-numbered edition fails the lint instead of
  silently changing a ditch. The passages and the conversions are already
  written down in
  `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md` (25 numbers with
  chapter refs, 1 Roman foot = 296 mm).
  Tier A. Size S. Owns `tools/codex-check.mjs`, `content/rome/camp.toml`,
  `content/rome/line.toml`, `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md`.
  Accept: `node tools/codex-check.mjs` prints `codex: numbers=25 unmatched=0` and
  exits 0; a table value that no passage supports is printed as `unmatched` and
  exits 1; a passage whose text hash changed is printed as `changed` and exits 1;
  the ditch row reads 1480 mm wide and 888 mm deep with `ref = "PHG 49"`.
  F2P: no codex tool and no `content/rome` tables.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [x] HX10 tools: the ship build is reproducible. A CI job builds the Windows ship  Landed 8303e625d. `node tools/ship-check.mjs --print` prints one SHA-256 content hash per file and a tree hash over `path\0content-hash` pairs; the path rides inside the hash, so a renamed file moves the tree hash. `node --test tools/ship-check.test.mjs` 7 tests pass: the same tree prints the same tree hash, one changed byte changes the tree hash and only that file's line, a renamed file changes it, the lines are sorted by path without the locale, a missing folder exits non-zero and names the folder, an empty folder is refused, and the hash flag prints only the tree hash. `node tools/workspace-check.mjs` PASS. The Windows job in `.github/workflows/ship-check.yml` builds the ship profile twice from a clean folder (`--profile ship -p engine-game --features app`, the same profile `tools/ship.mjs` packs) and compares the two tree hashes; equal hashes are the receipt, a mismatch names the file that moved. The tool reads bytes only: no clock, locale or timezone enters the hash. The mirror job then went red and the red was true: two cold ship builds of one commit differed in twenty bytes per executable, the COFF time stamp and the CodeView debug GUID (the PDB GUID and its age), which the MSVC linker writes fresh in every link. `tools/ship-check.mjs` now reads a PE image and zeroes exactly those fields before it hashes it, and it leaves the PDB out of the tree and names it in the receipt (the PDB is debug data and is not in the shipped bundle). Landed 67dbcb126. The job also no longer lets a newer push cancel it: two cold fat-LTO builds take longer than the gap between two pushes to the mirror, so every run was cancelled before it could compare the two trees. Landed 54dc8ae7f. The mirror job now passes: `SHIP-CHECK PASS: the two ship builds have the same tree hash 4a5eae7f0f78404405cf7b15a2b6d4c89c0757c9445336b53fd5c1159165bede` (run 39642426754, head a3c4b33cc). `node --test tools/ship-check.test.mjs` 12 tests pass.
  profile twice from a clean folder and compares the file trees by content hash, and
  the receipt names the tree hash. Donors: `rust-secure-code/cargo-repro`
  (Apache-2.0, 58 stars) and `omnibor/omnibor-rs` (Apache-2.0, 30 stars) for
  fine-grained content-addressed file naming.
  Tier B. Size S-M. Owns `tools/ship-check.mjs`, `.github/workflows/`.
  Accept: `node tools/ship-check.mjs --print` prints the file tree with one content
  hash per file and a tree hash over them; two builds of the same commit print the
  same tree hash, and a tree hash that changes when one byte changes is the F2P.
  F2P: no ship check.
  P2P: the golden matrix workflow is untouched.
  needs: none.
- [x] HX12 ci: the test report a stranger can read. The CI publishes a structured
  test report (the CTRF JSON shape) for the door and playtest suites, so a run's
  result is read without opening the log. Donor: `ctrf-io/github-test-reporter`
  (MIT, 378 stars, pushed 2026-10-11). Landed c879adfaa; fixed in 2af474f85 (the tool
  finds the receipt itself, the CI log keeps stderr, and the log is checked per status:
  `node --test tools/ctrf-report.test.mjs tools/ci.test.mjs` 23 pass; a real nextest
  0.9.149 run agrees with its receipt).
  Tier C. Size S. Owns `tools/ctrf-report.mjs`, `.github/workflows/`.
  Accept: `node tools/ctrf-report.mjs --out target/reports/ctrf.json` writes a
  report whose `summary.tests` counts equal the number of tests the run printed,
  and the file is byte-equal across two runs of the same tree.
  F2P: no structured report.
  P2P: the golden matrix workflow is untouched.
  needs: none.
- [x] HX15 web: the frame bus. The sim state crosses to the draw thread over a  Landed 8b3cb4431. `crates/web/src/frame_bus.rs` holds the seqlock: the writer bumps a counter to odd, writes the payload, bumps it to even, and a reader that sees the counter change throws its torn copy away and retries, at most `MAX_READ_ATTEMPTS` (8) times, then says so instead of spinning. The payload lives in a fixed array of `AtomicU32` words next to the counter, so the module needs no `unsafe` and no `UnsafeCell`; the writer stores the words relaxed and publishes them with a release store of the counter, and the reader loads the words relaxed and fences around the counter it reads twice. The same reader runs over a plain byte buffer (`PlainFrame`), so the postMessage fallback and the shared path share one code path. `cargo test -p engine-web --lib frame_bus` 10 passed (run 39614971453): a reader never sees a half-written frame, a torn copy is thrown away and the read succeeds, a real writer-thread race never shows a mixed frame, the plain path and the shared path agree byte for byte, no read waits on a write, a read gives up after a bounded number of attempts, the shared path stores the payload as little-endian words, bytes that do not fit one frame are refused, many readers see the same frame, and a frame keeps its bytes exactly. Clippy `-p engine-web --all-targets -- -D warnings` PASS (run 39615496316); rustfmt exit 0; `node tools/workspace-check.mjs` PASS. The module is std-only and needs no hash, so `engine-core` was not pulled in.
  **seqlock**: one writer, many readers, nobody waits. The writer bumps a counter
  to odd, writes the payload, bumps it to even; a reader that sees the counter
  change discards the torn copy and retries. The same reader works over a plain
  `ArrayBuffer`, so the `postMessage` fallback and the shared path share one
  code path. Donor: `nithish101/resonance`
  (`packages/web/src/lib/frames.ts`, MIT, read 2026-10-21).
  Tier B. Size S-M. Owns `crates/web/src/frame_bus.rs`, `crates/web/src/lib.rs`.
  Accept: `cargo test -p engine-web --lib frame_bus` passes: a reader never sees a
  half-written frame (a writer that mutates the payload while a reader copies
  makes the reader discard and retry), the `ArrayBuffer` path and the shared path
  agree byte for byte, and no read waits on a write.
  F2P: no bus between the sim and the draw.
  P2P: golden `d57b5c8ae3096614` x2; the page still plays the same match.
  needs: none.
- [x] HX16 net: the desync locator. A Merkle tree over the entity state finds a Landed 39e2fef8f. `cargo test -p engine-net --lib merkle` 9 passed (local lane, CARGO_INCREMENTAL=0); rustfmt exit 0; `node tools/rust-quality.mjs --owned crates/net/src/merkle.rs,crates/net/src/lib.rs` PASS (score 97, grade A); `node tools/workspace-check.mjs` PASS. The locator lives in `crates/net/src/merkle.rs` on top of `engine_core::digest::MerkleDigest`; `crates/net/src/guard.rs` was left alone because it is the input admission screen, not a desync check.
  desync in O(log n) instead of a full compare: the root hash is compared first
  and only the differing branch is walked, down to the entity. It is a
  **diagnostic only** and never a gate, so it stays off the golden. Donor:
  `braedonsaunders/voidstrike` (`src/engine/network/MerkleTree.ts`, MIT, 26 stars,
  read 2026-10-21).
  Tier B. Size M. Owns `crates/net/src/merkle.rs`, `crates/net/src/guard.rs`.
  Accept: `cargo test -p engine-net --lib merkle` passes: two states that differ
  in one entity are located at that entity in at most log2(n) comparisons, two
  equal states compare equal at the root, and the tree never enters the tick
  hash.
  F2P: a desync is found by a full compare.
  P2P: golden `d57b5c8ae3096614` x2; the snapshot codec digests do not change.
  needs: none.
- [x] HX24 rome: the groma lays the camp. The marching camp is generated by the Landed 531854bff. `crates/rome/src/groma.rs` surveys the camp out of a height grid: the ground decides the size in whole actae, the *clavicula* is struck from the gate hinge with the circinus open and bisects the camp, the corners are rounded from the corners of the cohort lines, and the tenth gate stands on the high point of the rear wall. `cargo test -p engine-rome --lib camp` passes (run 39593398101): a flat 200 m site with threat `light` returns four ditch runs of 2664 x 2072 mm (one night: 1480 mm wide, 888 mm deep), the clavicula halves the camp, the tenth gate stands on the high point, and two surveys of the same ground give the same plan. Clippy `-p engine-rome --all-targets -- -D warnings` PASS (run 39593403812). Golden `d57b5c8ae3096614` x2 (runs 39593454390, 39593457852).
  **survey procedure** of the source, not by a rectangle: from the praetorian
  gate strike the *clavicula* (a chord from the gate hinge with the circinus
  open) to find the centre, circle the corners of the cohort lines to fix the
  sides, and place the tenth gate on the high point. The plan is **checkable**:
  the *clavicula* must bisect the camp, the gate spacing must be a whole number
  of *actae*, and the frontage must be the cohort frontage (30 ft of frontage per
  720 ft of depth). Sources: Pseudo-Hyginus 54-55 (the groma and the circinus)
  and 49-50 (the ditch and the rampart), read in the machine-readable corpus
  `cltk/lat_text_latin_library` (`hyginus/hyginus6.txt`, 22.6K, Public Domain
  Mark 1.0, 28 stars); the numbers are in
  `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md`.
  Rows **LEG7** (the camp works), **ROM5** (the `rome.camp` verb), **ROM2**.
  Tier A. Size M. Owns `crates/rome/src/camp.rs`, `crates/rome/src/groma.rs`.
  Accept: `cargo test -p engine-rome --lib camp` passes: a flat 200 m site with
  threat `light` returns four ditch runs whose sizes match `camp.toml` (one
  night: 1480 mm wide, 888 mm deep), the *clavicula* bisects the camp, the
  tenth gate stands on the high point, and two runs give the same plan.
  F2P: no `crates/rome` and no camp generator.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [x] HX25 rome: every number carries its tier. A registry holds every number Landed 4ae7d434. `cargo test -p engine-rome --lib certainty` 17 passed 0 failed (gh run 39589887759): a number with no tier is refused with the key `units.roman_foot_mm` named, an unregistered key cannot be rendered (`LoadError::Unregistered`), and the Roman foot prints with the tier `contested` and a note naming 295.7 mm or 297.4 mm; the HUD line of the pilum reads `pila 2.0 m (Vegetius 2.15)`; F2P: the red run 39588679012 failed the first and the third of those before the fix; the whole lib 26 passed 0 failed (gh run 39590640299); `cargo clippy -p engine-rome --all-targets -- -D warnings` PASS (gh run 39590220619); golden d57b5c8ae3096614 x2 (gh runs 39590172706 and 39592466625); `node tools/workspace-check.mjs` PASS; `node tools/rust-quality.mjs --owned crates/rome/src/certainty.rs --owned crates/rome/src/lib.rs` PASS. The registry is a const table of 38 numbers in `crates/rome/src/certainty.rs`, each with its tier, a note and its source; the loader refuses a claim with no tier, the renderer asks the registry by key, so a number that is not registered cannot be drawn at all, and the new `LoadError::Unregistered` lives in `crates/rome/src/lib.rs`.
  the game shows with its tier (attested, reconstructed, contested, invented), a
  note and a source, and the loader **refuses an entry with no tier**; the
  renderer asks the registry for a number by key, so a number that is not
  registered **cannot be drawn at all**. The HUD then reads `pila 2.0 m
  (Vegetius 2.15)` with a tier chip, and the contested numbers (the Roman foot
  is 295.7 mm or 297.4 mm) become visible instead of hidden. Donor:
  `dtm988/colosseum_people_flow` (`src/claims.js`, MIT, read 2026-10-21).
  Rows **KIT1**, **CDX8**, and the codex rows of `docs/SPRINT-28-rome.md`.
  Tier A. Size S-M. Owns `crates/rome/src/certainty.rs`, `crates/rome/src/lib.rs`.
  Accept: `cargo test -p engine-rome --lib certainty` passes: a number with no
  tier is refused with the key named, an unregistered key cannot be rendered,
  and the Roman foot is printed with the tier that says it is contested.
  F2P: no certainty registry.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [x] HX28 rome: the chapter maps are real ground. The Digital Atlas of the Landed 6c433bed1. `cargo test -p engine-rome --lib geo` 9 passed, 0 failed (run 39597501940): the two tables load with every entry naming a `ref`, a point in the Danube region finds Drobeta within 5 km, the Limes segments carry the names the atlas gives them, and two loads hash the same. The frontier is 51 named lines (the Upper German and Raetian Limes, the Main, the Neckar, the frontier road and the Wall mile markers); the gazetteer is 25 settlements of the six chapter boxes with their Pleiades, Barrington, DARE and Wikidata ids. Clippy `-p engine-rome --all-targets -- -D warnings` PASS (run 39598639281); golden `d57b5c8ae3096614` x2 (runs 39599002285, 39599338633); licence and sha256 of both donors in `content/rome/PROVENANCE.md`.
  Roman Empire gives the **frontier as geometry**: the Upper German and
  Raetian Limes as named, segmented lines (with the Main and the Neckar named
  as rivers), Hadrian's Wall mile markers, the AD 117 provinces, and the
  **building footprint and wall polygon layer of Pompeii** with areas and
  lengths. Donor: `johaahlf/dare` (`fortifications.geojson`, `provinces.geojson`,
  `pompeii_architecture.geojson`, `places2.geojson`; **Apache-2.0**, the cleanest
  licence in the hunt, 5 stars, read 2026-10-21). A second source gives the
  settlements: `Diasito/urbesetorbis` (BSD-2) with the cross-reference ids
  (Pleiades, Barrington, DARE, Wikidata) and the founding year, so a chapter
  area is a **box filter** over the gazetteer.
  Rows **LEG6** (the dig layer), **LEG7** (the camp works), **CDX3** (the
  places), **ROM2**.
  Tier B. Size S. Owns `content/rome/places.toml`, `content/rome/fortifications.toml`,
  `crates/rome/src/geo.rs`.
  Accept: `cargo test -p engine-rome --lib geo` passes: the table loads with
  every entry carrying a `ref`, a point in the Danube region resolves to the
  nearest settlement within 5 km, the Limes segments carry their names, and two
  loads hash the same.
  F2P: no `crates/rome` and no `content/rome`.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [x] HX31 proof: the differential ship check. Landed 583ff9bb. `cargo test -p engine-proof --lib differential` 17 passed (run 39575557613); clippy `-p engine-proof --all-targets -- -D warnings` PASS (run 39575534607); golden `d57b5c8ae3096614` x2 (runs 39575583574, 39576754570). The job in `.github/workflows/differential.yml` ran on the public mirror and passed (run 39579398044): the two receipts match step for step, and the golden row is in both. A curated scenario list runs Landed 40c204954. The committed scenario list is now read by a test, not by CI alone: `cargo test -p engine-proof --lib differential` 19 passed, and a typo in `fixtures/differential/scenarios.txt` fails a unit test instead of a build. Landed a4405e051. The deep tier had no automatic slot at all: nothing ever built `spine_parity_line` or `ragdoll_line` on a runner, so the deep tier could be broken and CI would stay green. The workflow now has a weekly leg (Monday 04:17 UTC) that plays the deep tier, and the shape of the workflow is held by `tools/differential.test.mjs` (8 tests pass). Landed 9d32b1fe7. The manual and the pull request legs of the job were dead: the guard read `github.event.before` on every event, and that field only exists on a push (on the others it is null, and GitHub casts null and a string to a number, so null and the zero string are the same number and the guard was false). On the mirror the manual leg had never run once, so the deep tier could not be reached by hand at all. The guard now reads the before commit only where it exists. The deep tier ran on a Windows runner for the first time and passed: `DIFFERENTIAL clean tier=deep steps=2` with `--features arena,spine,ragdoll` (run 39640204582).
  against the **reference** build and the **candidate** build, and every step of
  the receipt is diffed with a normalization list (timestamps, ids, paths),
  where a real behaviour change must still show: a random value is pinned by
  seed rather than normalized away. Exit 0 means every in-scope step matched,
  1 means one divergence. Two tiers: a fast curated set on every change and a
  deep catalog off by default. Donor: `gastownhall/beads`
  (`tests/oracle-a/harness/src/differential.rs`, 30K, `tests/oracle-a/README.md`;
  MIT, 28056 stars, read 2026-10-21). We take the **shape**, because our
  receipts are our own line format, not JSON.
  Row **E22** (the Windows ship check from CI); gate G1.
  Tier A. Size M. Owns `crates/proof/src/differential.rs`,
  `crates/proof/src/bin/differential.rs`, `fixtures/differential/scenarios.txt`,
  `.github/workflows/differential.yml`.
  Accept: `cargo test -p engine-proof --lib differential` passes: two runs of
  the same build give the same normalized receipt, a seeded change shows up as a
  divergence and is not normalized away, and the tool exits 1 on one in-scope
  divergence.
  F2P: no ship check.
  P2P: golden `d57b5c8ae3096614` x2; the golden matrix workflow is untouched.
  needs: none.
- [x] HX35 playtest: the knob sweep comes out of the test module. The knob sweep is **already written** but behind `#[cfg(test)]`, so it never runs in a build and no verb reaches it: `crates/playtest/src/tournament.rs:1490-1700` holds `SWEEP_KNOB`, `SweepConfig`, `SweepReport`, `sweep_difficulty_curve` and `render_sweep_csv`, and the CSV already prints `knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials` with `# skipped` comment rows. The work: promote the block out of the test module, add a `tune.sweep` verb over it (one knob, a range, 5 fixed seeds), return the rows and the CSV bytes, and land the winning value as one `settings.set` plus one `settings.undo` so the tape stays replayable. No donor needed: this is a re-land of our own code. Row **D20** (tune.sweep). Tier A. Size S. Owns `crates/playtest/src/tournament.rs`, `crates/endpoint/src/packs/sweep.rs`, `crates/endpoint/src/packs/mod.rs`. Accept: `cargo test -p engine-playtest --lib sweep` and `cargo test -p engine-endpoint --features mcp --lib packs::sweep` pass: `tune.sweep` over one knob and 5 fixed seeds returns the CSV bytes, the same knob and seeds give the same CSV twice, the knob may be any name in the `settings.get` table, and a bad knob name is a typed refusal. The sweep reads no clock and no unseeded RNG. F2P: the sweep exists only inside the test module and no `tune.sweep` verb answers. P2P: golden `d57b5c8ae3096614` x2; the existing `play.sweep` verb keeps its behaviour. needs: none.  Landed 78cb4f6. The sweep left the test module and lives in `crates/playtest/src/tournament/sweep.rs`: `sweep_points`, `sweep_difficulty_curve` and `render_sweep_csv` are public, the knob name rides in the report, and the CSV keeps its knob column. The door got the verb `tune.sweep` in `crates/endpoint/src/packs/sweep.rs`: one knob (`match_ticks` or any name in the `settings.get` table), a range of values, 5 fixed seeds, answering with the curve rows, the CSV bytes, the winning value and the undoable landing (one `settings.set` plus one `settings.undo`). The same knob and seeds give the same CSV twice; a bad knob name and an out-of-range value are typed `bad_args` refusals naming the legal values. Proofs: `cargo test -p engine-playtest --lib sweep` 6 passed, 0 failed, 1 ignored (run 39687193058); `cargo test -p engine-playtest --lib` 449 passed, 0 failed, 4 ignored (run 39687698753); `cargo test -p engine-endpoint --features mcp --lib packs::play::sweep` 11 passed, 0 failed (run 39698154787) - 4 of them the new `tune.sweep` ones, 7 the `play.sweep` ones that keep their behaviour; `cargo clippy -p engine-playtest --all-targets -- -D warnings` PASS (run 39688232626); `cargo clippy -p engine-endpoint --all-targets --features mcp -- -D warnings` PASS (run 39688245549); `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` 1 passed, 0 failed, the binary printed `final_hash=d57b5c8ae3096614` (run 39690615093); `rustfmt --edition 2021 --check` clean on the four changed Rust files; `node tools/rust-quality.mjs --owned` PASS, 0 ratchet regressions over 44 crates; `node tools/workspace-check.mjs` PASS.
  `render_sweep_csv`, and the CSV already prints
  `knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials`
  with `# skipped` comment rows. The work: promote the block out of the test
  module, add a `tune.sweep` verb over it (one knob, a range, 5 fixed seeds),
  return the rows and the CSV bytes, and land the winning value as one
  `settings.set` plus one `settings.undo` so the tape stays replayable. No
  donor needed: this is a re-land of our own code. Row **D20** (tune.sweep).
  Tier A. Size S. Owns `crates/playtest/src/tournament.rs`,
  `crates/endpoint/src/packs/sweep.rs`, `crates/endpoint/src/packs/mod.rs`.
  Accept: `cargo test -p engine-playtest --lib sweep` and
  `cargo test -p engine-endpoint --features mcp --lib packs::sweep` pass:
  `tune.sweep` over one knob and 5 fixed seeds returns the CSV bytes, the same
  knob and seeds give the same CSV twice, the knob may be any name in the
  `settings.get` table, and a bad knob name is a typed refusal. The sweep reads
  no clock and no unseeded RNG.
  F2P: the sweep exists only inside the test module and no `tune.sweep` verb
  answers.
  P2P: golden `d57b5c8ae3096614` x2; the existing `play.sweep` verb keeps its
  behaviour.
  needs: none.
