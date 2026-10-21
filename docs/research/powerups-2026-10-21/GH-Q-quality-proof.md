# GH-Q quality and proof: donors for visual and behavioural regression testing

The visual gap is in the diff, not in the hash. The tree already has whole-frame numbers
(mean byte similarity and a golden-capture workflow in `crates/proof/tests/capture_compare.rs`,
windowed SSIM in `crates/proof/tests/capture_ssim_windows.rs`, and integer SSIM, deltaE,
histogram and FLIP in `crates/playtest/src/look/metric.rs`) and it has no per-pixel diff, no diff
picture, no perceptual hash and no golden PNG store, so the art lane's 30 m silhouette PNGs are
still read by a person (`tools/faction-war-report.mjs` prints `silhouette_big.png: ok`, and
`tools/asset-gen/character.mjs` gates only on one scalar, `bbox_fill`) (OBSERVED, read in
`C:\engine2040` on 2026-10-21). The behavioural side is mostly built: a seeded fuzzer with repro
bytes (`crates/proof/src/fuzz.rs`), a 10,000-case corpus with a pinned digest
(`crates/proof/src/break_corpus.rs`), a two-build differential compare
(`crates/proof/src/differential.rs`), a chaos suite and a proptest-shaped property matrix
(`crates/core/tests/it/property_matrix.rs`) (OBSERVED, same day). What is missing there is
coverage-guided fuzzing and a corpus kept in the repo as a golden set. The donors that close the
visual half are `mapbox/pixelmatch` (ISC) for a per-pixel OKLab HyAB diff with a threshold, an
anti-alias rule, a sliding-window rule and an ignore mask, `bokuweb/image-diff-rs` (MIT) for the
Rust shape where the count is computed first and the diff picture is written only when the pair is
rejected, `jaehl/blockhash` (MIT) for a 16x16 block-average shape fingerprint with a Hamming
distance (the right fingerprint for a flat black-on-white silhouette), and `garris/backstopjs`
(MIT) plus `microsoft/playwright` (Apache-2.0) for the golden store, the approve-once workflow and
the four files a failing run leaves behind. The behavioural donors are `rust-fuzz/cargo-fuzz` and
`rust-fuzz/arbitrary` (MIT and Apache-2.0), with one hard fact: the cargo-fuzz README says
libFuzzer works only on x86-64 and aarch64 and only on Unix-like operating systems, not Windows, so
the Windows leg must stay seeded and the coverage leg belongs in the Linux CI (OBSERVED). We port
the idea and write our own code; no donor bytes are vendored.

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Per-pixel picture compare with a tolerance: a colour distance, an anti-alias rule, a sliding-window rule, an ignore mask, and a diff picture. | `mapbox/pixelmatch` — https://github.com/mapbox/pixelmatch (file `index.js`) — read 2026-10-21 (OBSERVED: the doc comment names `threshold` as the maximum OKLab HyAB distance, plus `includeAA`, `diffColorAlt` for added vs removed, `diffMask`, `checkerboard`, `windowSize` = most differing pixels in any N x N window, and `ignoreMask` = pixels to skip) | ISC (root `LICENSE` read live 2026-10-21) | B8 (finish-line gap 8, board row PW14) and no row, propose a new row: **AT1 art: the 30 m silhouette gate compares against a golden PNG and fails the build** | `index.js`: the `colorDelta` / `oklabHyabDelta` pair (the HyAB test is folded so no square root is needed), the `antialiased` rule, the `windowSize` mask loop, the `ignoreMask` skip | S | If the integer port of the folded HyAB test disagrees with the float reference by more than 1e-6 on 100 seeded pixel pairs, or if 20 runs of one unchanged character produce any false fail | PORT (the idea, our own integer code; no donor bytes) |
| 2 | The same diff in Rust, staged: count the differing pixels first, write the diff picture only when the pair is rejected. | `bokuweb/image-diff-rs` — https://github.com/bokuweb/image-diff-rs (`core/src/lib.rs`, `core/src/compare.rs`) — read 2026-10-21 (OBSERVED: `DiffOption{threshold, include_anti_alias, encode_format}`, `diff_count` with no allocation, `diff_rgba` unencoded, `encode_diff`, and `CompareInput{actual_filename, expected_filename, diff_filename, threshold, include_anti_alias}`) | MIT (root `LICENSE` read live 2026-10-21). Risk: the crate builds libwebp with `cc` (build-dependency in `core/Cargo.toml`, `libwebp` submodule in the root listing) so we take the pixel part only | B8 / PW14 and the new row AT1 | `core/src/lib.rs` (the count-then-encode split) and `core/src/compare.rs` (the actual, expected, diff path triple plus the threshold) | S | If the webp and `cc` build cannot be dropped from the port, or if the count over a 2048 x 2048 pair takes more than 50 ms on the potato box | PORT (the shape; the crate is not a dependency) |
| 3 | A shape fingerprint: average each of 16 x 16 blocks, set one bit per block against the median, compare two fingerprints with a Hamming distance. | `jaehl/blockhash` — https://github.com/jaehl/blockhash (`src/hash.rs`, `src/lib.rs`) — read 2026-10-21 (OBSERVED: `blockhash`, `get_values_aligned`, `convert_to_bits`, `distance` as a popcount over XOR, 16 to 256 bit sizes, `#![forbid(unsafe_code)]`, a `no_std` option) | MIT (root `LICENSE` read live 2026-10-21) | The new row AT1 and B8 | `src/hash.rs` (`blockhash`, the block averages, `convert_to_bits`) and `src/lib.rs` (`distance`) | S | If the 256-bit distance between two renders of one mesh is not identical on two machines, or if it cannot tell the existing character silhouettes apart (distance 0 between two different characters) | PORT |
| 4 | Golden picture management in CI: expected, actual, diff and previous files written on every run, a five-value update policy, and a hard cap on how many pixels may differ. | `microsoft/playwright` — https://github.com/microsoft/playwright (`packages/playwright/src/matchers/toMatchSnapshot.ts`) — read 2026-10-21 (OBSERVED: the update policy `updateSnapshots` takes the words `all`, `changed`, `missing`, `none`, `default`; `maxDiffPixels` must be a non-negative integer, `maxDiffPixelRatio` must be in 0..1, and the four output paths `-expected`, `-actual`, `-diff`, `-previous` are written) | Apache-2.0 (root `LICENSE` read live 2026-10-21) | B8 / PW14 and E22 (the CI ship check compares to the reference captures) | `packages/playwright/src/matchers/toMatchSnapshot.ts`: the four output paths and the update policy. The dependency itself is not taken (the repo already has its own headless-Chrome skill) | S | If the four-file layout is more than the art lane needs (keep expected, actual and diff) or if the policy cannot be expressed with the two env words the tree already uses (`CAPTURE_UPDATE`, `INSTA_UPDATE`) | PORT (the contract) / RESEARCH-ONLY (the dependency) |
| 5 | A reference picture store with one human approve step, a percent-of-pixels tolerance per scenario, and a size change treated as a failure. | `garris/backstopjs` — https://github.com/garris/backstopjs (`README.md` config table) — read 2026-10-21 (OBSERVED: `misMatchThreshold` as a percentage of differing pixels, `requireSameDimensions` default true, the `bitmaps_reference` store, `backstop test` then `backstop approve` promotes the test pictures into the reference set) | MIT (root `LICENSE` read live 2026-10-21) | The new row AT1 (the approve-once store for the silhouette goldens) and B8 | `README.md`: the store layout beside the test pictures, the approve-and-promote step, the percent-of-pixels tolerance, the size rule | S | If the approve step needs a browser app the repo does not run, keep only the store and the threshold and drop the approve command | PORT |
| 6 | Coverage-guided fuzzing of the game loop with a corpus directory kept in the repo, plus two commands that shrink one failing case and the whole corpus. | `rust-fuzz/cargo-fuzz` — https://github.com/rust-fuzz/cargo-fuzz (`README.md`) — read 2026-10-21 (OBSERVED: `cargo fuzz tmin <target> <input>` minifies one failing input, `cargo fuzz cmin <target>` minifies the corpus, and the README says libFuzzer "only works on x86-64 and Aarch64, and only on Unix-like operating systems (not Windows)") | MIT and Apache-2.0 (root `LICENSE-MIT` read live 2026-10-21, the README names both) | OPT22 (the property and fuzz board row) and G6 (the fuzzing corpus runs clean) | `README.md`: the corpus layout and the two shrink commands as a runbook. The coverage leg lives in the Linux CI only | M | If the Linux CI leg holds more than N corpus files, or if the coverage fuzzer finds no new case in the first N hours (then the seeded fuzzer already in `crates/proof/src/fuzz.rs` is enough) | PORT (the Linux CI leg and the runbook) / RESEARCH-ONLY (a Windows dev box) |
| 7 | Turn raw fuzzer bytes into a valid game tape: a type can derive the trait, single fields can be pinned. | `rust-fuzz/arbitrary` — https://github.com/rust-fuzz/arbitrary (`README.md`) — read 2026-10-21 (OBSERVED: `Unstructured`, `#[derive(Arbitrary)]`, `#[arbitrary(default)]`, `#[arbitrary(value = 255)]`; the crate works with or without libFuzzer) | MIT (root `LICENSE-MIT` read live 2026-10-21) and Apache-2.0 (root `LICENSE-APACHE` listed) | OPT22 and the fuzz leg of G6 | `README.md`: the derive semantics, so the fighter tape type derives the trait and one corpus file is one valid bout | S | If the derived tape cannot be replayed through `engine_arena::run` without a new engine API, or if the existing input validation refuses a derived tape | PORT |
| 8 | Two numbers for one picture pair (a pixel threshold and an SSIM), the share of pixels that differ as the failure number, and the received picture stored on failure. | `americanexpress/jest-image-snapshot` — https://github.com/americanexpress/jest-image-snapshot (`src/diff-snapshot.js`, `src/index.js`) — read 2026-10-21 (OBSERVED: pixelmatch with `threshold: 0.01` plus `ssim.js` Bezkrovny, the result carries `diffRatio` and `diffSize`, and the counters matched, unmatched, added, updated) | Apache-2.0 (the header in `src/index.js` and `src/diff-snapshot.js` read live; the root `LICENSE` path 404s today) | B8 / PW14 (the second metric on the look score) and the new row AT1 | `src/diff-snapshot.js` (the two-metric compare and the ratio) and `src/index.js` (the counters) | S | If the second metric does not change the verdict on the reference frames, or if the two metrics disagree on more than 1 in 20 seeded pairs | PORT |
| 9 | A perceptual hash with a size knob and a distance number, next to a content hash for the exact bytes. | `kaminocorp/photon` — https://github.com/kaminocorp/photon (`crates/photon-core/src/pipeline/hash.rs`) — read 2026-10-21 (OBSERVED: `HasherConfig::new().hash_alg(HashAlg::DoubleGradient).hash_size(16, 16)`, `perceptual_hash`, `perceptual_distance` as a Hamming distance, and the file's own note that "Distances < 10 typically indicate very similar images" (the donor's claim, not ours)) | MIT or Apache-2.0 (workspace `Cargo.toml` license field and root `LICENSE-MIT` read live 2026-10-21) | The new row AT1 and B8 | `crates/photon-core/src/pipeline/hash.rs`: the config and the distance contract only. The crate is heavy (image, blake3, tokio), so we write our own | S | If the distance threshold cannot separate a good character from a broken one over the existing silhouettes, or if row 3 already gives the same verdict | PORT (the contract) / RESEARCH-ONLY (the crate) |
| 10 | A menu of perceptual hash algorithms in Rust (mean, gradient, DCT, wavelet) behind one config, with a Hamming distance. | `abonander/img_hash` — https://github.com/abonander/img_hash (`src/lib.rs`, `src/alg/mod.rs`) — read 2026-10-21 (OBSERVED: `HashAlg{Mean, Gradient, ...}`, `HasherConfig::hash_size`, `hash_image`, `dist`) | MIT (root `LICENSE-MIT` read live 2026-10-21) and Apache-2.0 (root `LICENSE-APACHE` listed) | The new row AT1 (a second fingerprint if the block hash is too coarse) | `src/alg/mod.rs` (the algorithm list) and `src/lib.rs` (the config and the distance) | S | If the DCT variant needs a float transform and our metric must stay integer (`crates/playtest/src/look/metric.rs` is integer-only), drop the DCT and keep the gradient variant | RESEARCH-ONLY (second choice after row 3) |
| 11 | A line diff for the golden text, so a failing gate prints what moved instead of two blobs. | `mitsuhiko/similar` — https://github.com/mitsuhiko/similar (`Cargo.toml`, `src/udiff.rs`, `src/algorithms/`) — read 2026-10-21 (OBSERVED: `license = "Apache-2.0"`, the source tree holds `udiff.rs` and the algorithms capture, myers, patience, lcs, histogram, compact, hunt, hook) | Apache-2.0 (`Cargo.toml` license field and root `LICENSE` read live 2026-10-21) | The new row AT1 (the `silhouette.json` golden) and B8 | `src/udiff.rs` and `src/algorithms/patience.rs`: a line diff over the JSON lines of the golden | S | If the JSON golden diff is never read in a failing run, then print the numbers only and drop the diff | PORT |
| 12 | Property-based testing with shrinking, and a failure file kept next to the test that is replayed on the next run. Already ported in our tree. | `proptest-rs/proptest` — https://github.com/proptest-rs/proptest (`proptest/src/test_runner/failure_persistence/file.rs`, `proptest/Cargo.toml`) — read 2026-10-21 (OBSERVED: `license = "MIT OR Apache-2.0"`, `FileFailurePersistence::SourceParallel("proptest-regressions")` writes a `.txt` file beside the test source) | MIT or Apache-2.0 (`Cargo.toml` license field and root `LICENSE-MIT` read live 2026-10-21) | OPT22 (the board row that wants the real dependency and ten game invariants) | Nothing new. `crates/core/tests/it/property_matrix.rs` already ports the config, the case count and the failure-persistence file shape (OBSERVED in the tree 2026-10-21). The open part is the dependency and the game-loop invariants | S | If the proptest dependency cannot pass `cargo deny check` against the repo allow-list, the hand-rolled shape stays and OPT22 closes as shape only | PORT as a CORRECTION, not a new find: the donor is already mined in our tree |

## Rejections

- `o3de/o3de` `screenshot_utils.py::compare_golden_image` — already mined: it is named in
  `crates/proof/tests/capture_compare.rs` (read 2026-10-21). Not repeated.
- `mitsuhiko/insta` — already in the tree (`crates/proof/tests/capture_compare.rs` cites
  `insta/src/env.rs` and `insta/src/runtime.rs`). License checked live today: `insta/Cargo.toml`
  says `license = "Apache-2.0"` and the root `LICENSE` is Apache-2.0, so the label in our file is
  right and no correction is needed (OBSERVED 2026-10-21). REJECT as a new row.
- `bokuweb/pixelmatch-rs` (the standalone crate) — REJECT: no `LICENSE` file at the root and no
  `license` key in `pixelmatch/Cargo.toml`, so the bytes are all-rights-reserved (OBSERVED
  2026-10-21). Port the algorithm from the ISC original instead. It also still uses the old YIQ
  colour metric while upstream moved to OKLab HyAB, so it is behind (OBSERVED, both files read).
- `regclient/regclient` — REJECT: no picture diff in it today. A code search for
  `pixelmatch OR "image diff" OR diff.png` in that repo returned 0 hits and `cmd/regctl/image.go`
  holds copy, create, delete, digest, export, import, inspect, mod and check-base but no diff
  command (OBSERVED 2026-10-21).
- `google/honggfuzz` — REJECT for a Windows-first engine: the README lists the platforms as Linux,
  macOS, Android, NetBSD, FreeBSD and "Windows (Cygwin)" (OBSERVED 2026-10-21). RESEARCH-ONLY for
  the Linux CI.
- `backstopjs/backstopjs` — that path 404s today (GitHub API answer 404, 2026-10-21); the live repo
  is `garris/backstopjs` (row 5).
- `BurntSushi/quickcheck` — REJECT: proptest is the Rust norm and its shape is already ported in
  our tree (row 12).
- `facebookarchive/propfuzz` — REJECT: the repo is archived (repo search field `archived: true`,
  2026-10-21) and it is a libFuzzer bridge, which row 6's kill line already rules out on Windows.
- `rust-av/ssimulacra2` and `ggrs` — already mined, not repeated. The deterministic replay side is
  covered by our own `crates/proof/src/break_corpus.rs`, `tape.rs` and `differential.rs`
  (OBSERVED 2026-10-21), so no new donor is needed there.
- `microsoft/playwright` as a dependency — REJECT the dependency (the repo has its own
  headless-Chrome skill and a PNG diff needs no browser). The golden-file contract is still
  ported (row 4).
- `opencv/opencv` `matchShapes` (Hu moments) for silhouette shape distance — not read today;
  skipped because the block hash already gives a shape fingerprint in integer code (INFERRED).

## Top 3

1. `mapbox/pixelmatch` (ISC) — https://github.com/mapbox/pixelmatch (file `index.js`), read
   2026-10-21. Per-pixel diff with a tolerance, the anti-alias rule, the sliding-window rule and
   the ignore mask, in a form that folds the square root away so an integer port is possible.
   Closes the diff half of **B8** (board row PW14) and the new row **AT1 art: the 30 m silhouette
   gate compares against a golden PNG and fails the build**.
2. `jaehl/blockhash` (MIT) — https://github.com/jaehl/blockhash (`src/hash.rs`, `src/lib.rs`), read
   2026-10-21. A 16 x 16 block-average shape fingerprint with a Hamming distance: the one number
   the art gate can print for "the silhouette still reads" without a human opening the PNG.
   Closes **AT1** and the shape half of **B8**.
3. `rust-fuzz/arbitrary` and `rust-fuzz/cargo-fuzz` (MIT and Apache-2.0) —
   https://github.com/rust-fuzz/arbitrary and https://github.com/rust-fuzz/cargo-fuzz, read
   2026-10-21. Structure-aware game tapes plus a corpus kept in the repo and two shrink commands.
   Closes **OPT22** and the fuzz leg of **G6**, with the Windows caveat written into the kill line:
   the coverage fuzzer runs in the Linux CI, the Windows leg stays seeded.
