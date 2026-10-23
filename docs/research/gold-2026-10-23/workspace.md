# Workspace, tooling and delivery gold, 2026-10-23

Scope: one scout pass over workspace, tooling, delivery and a few engine techniques.
Goal: find work the plans do not own, not work the plans already rank.
This page does not repeat the finish audit (`docs/plan/finish-line.md:48-62`).

How absence was checked: a term scan over `docs/plan/*.md` and `docs/SPRINT*.md` (zero hits means no row owns it), then a read of the code that would have to change.
A live URL was opened on 2026-10-23 unless marked otherwise.
Order is value x cheapness x how much the plan already thinks the gap is covered.

## 1. Fuzz the bytes a stranger sends

**Gap.** The session packet pair already has a checked-in fuzz corpus (`crates/net/tests/it/packet_corpus.rs`: valid, truncated and garbage seeds, no-panic, determinism, a Bolero byte-input shape), but the newer hand-written decoders do not: cells, cut sets, relay ICE, the handshake and delta parse bytes from another machine with no fuzz harness.

**Repo evidence.**
- Decoders: `crates/net/src/cells.rs:142` `decode_packet`, `crates/net/src/cuts.rs:120` and `:380`, `crates/net/src/relay_ice.rs:44`, `:106` and `:607`, `crates/net/src/handshake.rs:718`, `crates/net/src/delta.rs:327`.
- Existing fuzz: `crates/net/tests/it/packet_corpus.rs:1` (the session `decode_input`/`decode_state` pair: nine seeds, no-panic, determinism, Bolero shape at `:177`), `crates/proof/src/tape_fuzz.rs:1` (Bolero `check!` shape, repro tapes only), `crates/proof/src/fuzz.rs:1` (arena spine), `crates/spatial/src/fuzz.rs:1` (relation ops). The two proof modules sit behind a `fuzz` feature (`crates/proof/Cargo.toml:37-38`); the spatial harness is always compiled. Correction 2026-10-23: the card first read the session pair as unfuzzed; it is fuzzed, so the seat below starts from the five newer decoders.
- No `fuzz/` folder and no `fuzz_targets` anywhere. `libfuzzer-sys` sits in the lock only under `rav1e` (`Cargo.lock:7544-7561`), not as a repo target.
- `docs/plan/quality.md:61` records fuzz as present. The plan's other fuzz rows are door verbs and accept lines (`docs/plan/forge-adoption.md:617`, `:619`), not wire bytes.

**Live donor.** `rust-fuzz/cargo-fuzz@bf2fc668dafda5295aa6fd01825ee67b885f0f2b:README.md` (read 2026-10-23). libFuzzer, nightly, Unix-like only, so it runs on the Ubuntu mirror, not this PC. `camshaft/bolero@7642943b6c1f8a9b5e824dd1cd29b7c8e56068b8:README.md` (read 2026-10-23): `bolero::check!()` runs as a normal `#[test]` on stable, which the repo already ports.

**Win.** The duel host accepts packets from another PC (`docs/DEMO.md:86`, `:122`), and the relay and browser net path do too. A panic there ends the host. This proves that no 40-byte cell, cut set, handshake, ICE binding or delta can panic, hang, or allocate a wild length. Add save, patch and bundle readers to the same corpus.

**Effort.** S for the harness (copy the shape of `tape_fuzz.rs`), M for corpus, shrink and a nightly job.

**Prove it.** `cargo test -p engine-proof --features fuzz --lib decoder_fuzz` twice, same seed. Then `cargo fuzz run cell_decode` on the Ubuntu mirror to 1e8 execs in 24 h with zero panics and zero integer overflows. Plant one bug by deleting a length cap in `decode_cuts` and confirm the harness names the input in under 60 s.

**Kill if.** 1e8 execs across all targets find only OOM from a declared length field and no panic, overflow or wild allocation. Fix the caps, keep the seed corpus as a cheap regression, and stop.

**Landed 2026-10-23** (`75629a723`). The bounded suite lives at `crates/net/tests/it/packet_boundary_fuzz.rs`: the five newer decoders (cells, cut set, relay ICE request/response/candidate, handshake token/sealed/index/server, delta via the state frame) take 28,052 seeded sweep cases twice (56,104 decodes) plus 962 strict-prefix cases, all returning a typed `NetError` or a value, in 0.01 s per test. Runs 40018879101 and 40019186297 (x2), clippy 40018723612, golden 40019482337 unchanged. No panic, missing cap or wrong error was found; the "plant a bug" check stays a one-line experiment (delete the `count > MAX_CUTS_PER_PACKET` check and the count-33 assert names the 13-byte header). Still open: the nightly coverage-guided `cargo fuzz` job to 1e8 execs on the Ubuntu mirror, and adding the save, patch and bundle readers to the corpus.

**Why we missed it.** The repo already says it fuzzes (FZ1, HS23), and every decoder returns `Result`, so the code reads as safe. `Result` does not stop slice indexing or an `as` truncation. cargo-fuzz not working on Windows meant nobody reached for it.

## 2. Prove the length the demo actually runs, and make a save carry the match

**Gap.** The demo command runs 100,000 ticks. The hash gate stops at 1,200. A save carries counters, not the match.

**Repo evidence.**
- `docs/DEMO.md:13` and `:86` ship `--ticks 100000`. At 60 Hz (`crates/core/src/tick.rs:6`) that is 27.8 minutes.
- `crates/game/src/lib.rs:268` `MAX_TICKS = 100_000`.
- `docs/HASH-LAW.md:3` fixes the golden at 600 ticks. `crates/proof/src/arena_matrix.rs:31` runs 1,200. `.github/workflows/golden-matrix.yml:15` records the 1,200-tick arena check.
- `crates/proof/src/online_soak.rs:37` runs 216,000 ticks, but that is network convergence, not tape replay and not save.
- `crates/game/src/lib.rs:1912-1931` `save_progress` writes the progression counters plus seed and cursor. `load_progress` (`:1934-1949`) decodes the same.
- `crates/arena/src/live.rs:817-821` `MatchSnapshot` refuses a world-armed, v2, spear or height match. `:1380` repeats the reason: no world.

**Live donor.** `gschup/ggrs@25bd9983dee3ecb0677be03b9e2e23d2bd6d29cb:src/sessions/sync_test_session.rs` (read 2026-10-23): save one frame, advance, roll back `check_distance`, resim, compare checksums, return `MismatchedChecksum`. Gaffer, "Deterministic Lockstep" (read 2026-10-23): after the smallest divergence the two sims drift further and further apart, so a short gate cannot bound a long run.

**Win.** A recorded 100,000-tick demo replays bit-equal, and a crash at tick 90,000 resumes to the same final hash. That closes the "clean at 600, drifting at 50,000" class at the one length the demo ships. It also makes the league resume story real.

**Effort.** M. The long gate is S; the resume leg is M because `MatchSnapshot` has to grow a world and a v2 balance, or the save has to store the tape plus the match spec.

**Prove it.** Run `arena.exe --ticks 100000 --seed 2040 --exit` twice and print the same `arena sim:` hash on three native targets plus wasm. Replay the recorded tape and match it. Save at 90,000, resume, and match the unbroken final hash. Plant one dropped field in the snapshot encoder and confirm the first bad tick is named.

**Kill if.** A 100k replay diverges before tick 2,000 for a known float or presentation reason. Fix that first; do not build resume on a broken base. Or, if 20 seeds at 10,000 ticks show zero drift and the 100k run adds only CI minutes, gate 10k and stop.

**Why we missed it.** The golden is 600 ticks by law, and the matrix grew to 1,200. The 100,000 number lives in the demo page, not in a gate, so nobody joined them. The save is rich enough to look finished until you read that it stores counters.

## 3. Give the `strobe` gate a time term

**Gap.** `red.strobe` measures how much of the screen is tinted. It never measures how often the tint changes.

**Repo evidence.**
- `crates/game/src/fighter_look.rs:634` `FLASH_TICKS = 15` at 60 Hz gives a 4 flashes/s ceiling per fighter. `:4411` pins the 15.
- `tools/product-check.mjs:215` fails `red.strobe` on `tinted_share` alone. `tools/product-bar.json:5` sets the floor at 0.25; `:24` sets the finish target under 0.10 in an 8-fighter brawl.
- `crates/web/src/exchange.rs:43-52` paints five flash kinds; the page holds each for a few ticks.
- 87,296 px is 9.47% of a 1280x720 frame. The 0.10 target is the WCAG 10-degree area. There is no time term.
- The plan scan finds zero photosensitivity, seizure or flash-rate rows. The only earlier mention is a 2026-10-17 research note on Bevy recovery flicker (`docs/research/finish-line-2026-10-17/02-github.md:40`).

**Live donor.** W3C, "Understanding SC 2.3.1: Three Flashes or Below Threshold" (read 2026-10-23): no more than three flashes in any one second, or below the general and red flash thresholds. A 341 x 256 CSS px block is 87,296 px, a 10 degree view. A general flash is a luminance change of 20 cd/m2 or more when the darker state is below 160 cd/m2. The page also names PEAT.

**Win.** The public page and every capture pass a named safety bar, and a 4 Hz exchange cannot ship unseen. Store and platform review ask this question. The fix can be art or a gate, but the number must exist.

**Effort.** S.

**Prove it.** On a 300-frame 8-fighter capture, count qualifying luminance transitions inside the 341 x 256 block and print `flashes_s=` and `area_px=`. Bar: at most 3 per second and at most 87,296 px. A synthetic 4 Hz flicker must fail. Add the two numbers to `tools/product-bar.json` beside `strobe`.

**Kill if.** Three independent brawl captures each read at most 3 flashes/s and at most 9.5% area. Then the art already passes: ship the one-line bar and stop. Do not build a per-cue flash budget.

**Why we missed it.** The failure key is literally `red.strobe` and the target is called `strobe`, so flashing looked owned. It measures a still frame. The WCAG area number was copied; the time half was not.

## 4. Pin the page to one build

**Gap.** The wasm is content-addressed and checked. `engine.js`, `index.html` and the two `.e2bundle` files are fixed names with no version check, so a deploy can mix builds while the page still prints one commit.

**Repo evidence.**
- `crates/web/index.html:154` imports `./engine.js` by fixed name. `:89` and `:125` bake `{{COMMIT}}` into the page and the build line.
- `crates/web/src/content.rs:22` and `:26` fix the arena and rig bundle URLs.
- `crates/web/loader.js:13-17` and `:62-71` verify only `engine_bg.wasm` against the SHA-256 in `build.json`, then cache it under that hash.
- `tools/web-build.mjs:292-299` writes a SHA-256 for every shipped file. `:321-322` writes `build.json`, which cannot hold its own hash.
- `crates/assets/src/asset_source/bundle.rs:495-498` refuses a bundle whose internal version is wrong. That is the only bundle check.

**Live donor.** Vite, "Static Asset Handling" (read 2026-10-23): referenced assets "get hashed file names" in a production build. RFC 9111 `immutable` is the cache rule that makes those names safe to keep for a year.

**Win.** A player never runs stale glue against a new wasm or a stale bundle against a new engine, and the `build:` line tells the truth. A bad deploy is caught by the page with a named mismatch, not by a wasm trap or a silent art swap.

**Effort.** S.

**Prove it.** Serve v1, then v2 with `Cache-Control: max-age=31536000, immutable` on the hashed files and a long cache on the old fixed names. Reload with only some files cached. The page prints `build= wasm= kit= rig=` and refuses a mixed pair. Twenty reloads, zero mixed pairs. Tamper one byte of `engine.js` and get a named refusal.

**Kill if.** Two consecutive deploys under a one-hour cache always fetch a matched pair because the host revalidates HTML and the HTML names hashed files. Then the platform already solves it; stop.

**Why we missed it.** `build.json` already holds every SHA-256, so the output looks content-addressed. Only the wasm reader uses it. The page prints a commit, which reads like provenance but is just a template value.

## 5. One local support bundle, and a cap on the run logs

**Gap.** Crash data is rich and split across two writers. No single command collects it, and per-run logs are never pruned.

**Repo evidence.**
- `crates/game/src/crash.rs:648-731` writes a view-warning bundle: `report.toml`, `tape.bin`, `README.txt`. It is view only.
- `crates/game/src/crash.rs:843-852` installs the run hook: one `arena-<UTC>.log` per run and one `crash-<UTC>.toml`. `default_log_dir` is at `:875-886`.
- `crates/perf/src/crash_receipt.rs:183` writes a second receipt, `:243` picks its own directory (`target/reports/crash`).
- No support, zip or rotation tool exists in `tools/`. The scan finds zero rows for a support bundle.
- `docs/plan/render-look.md:185-187` cuts crash-upload services and keeps a local file only. A local collector stays inside that call.

**Live donor.** Unity, "Log files reference" (read 2026-10-23): one known log location per platform, plus `Application.consoleLogPath`. Godot, "Troubleshooting" (read 2026-10-23): a named recovery path and one page that tells a user where to look.

**Win.** A stranger on chat sends one file. It holds the version, adapter, settings, tape tail, receipt and log. No service, no telemetry, no account. Logs stop growing without a bound.

**Effort.** S.

**Prove it.** Force a panic with `E2040_TEST_PANIC_AT_TICK=5` (`crates/game/src/bin/arena.rs:629`), then run `node tools/support-bundle.mjs`. It writes a 2 MB or smaller zip with six named files plus `sha256.txt`. A second run lists the same six names. A fresh clone with no logs gets a clear "no logs" refusal.

**Kill if.** The existing `crash-*.toml` plus `tape.bin` folder already fits one attachment and the README names the path. Then add only the README line and the log cap; build no tool.

**Why we missed it.** The crash report is already better than most engines, so it reads as solved. The missing part is the last mile, not the data. A public alpha still asks a player to find three folders.

## Top-2 gold

1. **Fuzz the wire decoders (#1).** Highest value per hour. The host is public, the parsers are hand-written, and the harness pattern already exists. One planted bug proves the gate in a minute.
2. **The 100k replay and resume (#2).** The demo already runs this length and the save already looks finished. This is the largest honesty gap between what ships and what is proven.

#3 is the cheapest and the most likely to be asked by a store front. Do it in the same week if a seat is free.

## Do not do

Do not build a crash-upload or telemetry service. It is already a documented cut (`docs/plan/render-look.md:185-187`). A bundle host adds a network surface, a privacy duty and a backend for zero engine value. Keep the bundle a local file the player attaches.

## Sources

Opened 2026-10-23:

- https://www.w3.org/WAI/WCAG22/Understanding/three-flashes-or-below-threshold.html
- https://vite.dev/guide/assets.html
- https://gafferongames.com/post/deterministic_lockstep/
- https://docs.unity3d.com/Manual/LogFiles.html
- https://docs.godotengine.org/en/stable/tutorials/troubleshooting.html

Pinned commits (read 2026-10-23):

- cargo-fuzz `bf2fc668dafda5295aa6fd01825ee67b885f0f2b`
- bolero `7642943b6c1f8a9b5e824dd1cd29b7c8e56068b8`
- GGRS `25bd9983dee3ecb0677be03b9e2e23d2bd6d29cb`
