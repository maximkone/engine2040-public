# Build CPU: one slot pool for every cargo on the PC

Owner 2026-10-21: "optimize our cpu usage af and all rust and cargo too ... 100% CPU, 27 cargo,
20 rustc". Rows OPT37-OPT42 on `docs/SPRINT-27-final-picture.md`. Earlier build work and its
numbers live in `docs/SMOOTH-BUILD.md` "Build speed facts"; this plan adds what that page lacks.

## 1. What the PC was doing (census 2026-10-21, 12 CPUs)

- 36 `cargo.exe` processes were 18 builds: every cargo runs twice, the rustup proxy plus the real
  toolchain cargo. So "27 cargo" means about 13 builds, and most of them were not compiling at all.
  They were queued on the one `target/debug` build-dir lock.
- The builds that did compile ran in four target dirs at once: `target/debug` (tests),
  `target/release` (the golden verify, fat LTO, one codegen unit), `target/product` (the arena
  bin), and an old spike dir (`target/spike-opt8/before2`, compiling `syn` at opt-level 3).
- Each cargo starts its own pool of 12 jobs, and rustc's codegen threads use that same pool. Four
  live builds can ask for 48 jobs on 12 cores. The OS then time-slices them, so every build gets
  slower and the desktop lags.
- The same dependency gets compiled again in each target dir and profile. sccache does not fix
  this here. It was measured on this PC: 0 hits across fresh target dirs
  (`docs/receipts/2026-10-20-opt9-compile-cache-spike.md`). Do not bring it back as a global
  wrapper.

## 2. The fix: one pool of compile slots (OPT37)

Cargo and rustc speak the GNU make jobserver protocol. On Windows the pool is a named semaphore.
A cargo started with `CARGO_MAKEFLAGS=-j --jobserver-auth=<name>` opens that semaphore and takes
one slot for each extra rustc and each codegen thread. It does not start its own pool of 12. So N
sessions share `<Tokens>` slots plus one implicit slot each. No build waits on a lock it did not
wait on before. They just stop fighting over the cores.

- `tools/jobserver.ps1 start [-Tokens 10]` holds the pool, `status` counts the free slots, and
  `env` prints the line. If the pool named in the env is not running, cargo warns and falls back
  to its own `-j`, so a stopped holder never breaks a build.
- Measured on this PC 2026-10-21, with the box already at 100% from other sessions. A scratch
  crate with 7 dependencies, `cargo check --offline`:
  - 1-slot pool: at most 2 rustc at once, 591 s.
  - Its own `-j`: up to 10 rustc at once, 588 s.
  So on a full box the 8 extra jobs bought nothing; they only took CPU from every other build.
  A cargo whose env names a pool that is not running still builds (checked the same day).
- Leaked slots: a build killed mid-job never gives its slot back. When no rustc and no build
  script is running, the holder resets the free count to `<Tokens>` (every 30 s).
- Adoption needs the owner. Setting `CARGO_MAKEFLAGS` in a user env var or in
  `.claude/settings.json` `env`, plus a logon task that runs the holder, is persistent machine
  config. The tools can join without that: `gh-check.mjs`'s local fallback and `proof.mjs` set the
  env only when `status` says the pool is up (OPT37 row).
- Tokens: 10 of 12 leaves two cores for the desktop, the game window and the editor. More slots do
  not speed up a box that is already full.

## 3. Smaller levers, in order

| Row | Lever | Why | Cost |
|---|---|---|---|
| OPT38 | The local golden verify builds with `--profile product` (thin LTO, 16 units), not `--release` (fat LTO, 1 unit) | The fat-LTO relink is the slowest build in the tree, and every session's golden check pays it again after any engine edit. `product` already shares `target/product` with the arena bin. A community report puts fat LTO + 1 unit at +128% build time over plain release. The hash must not change: no fast-math, overflow checks stay on. | CI release-proof keeps `--release`; the row proves both profiles print `d57b5c8ae3096614` |
| OPT39 | Dependencies build without debug info in dev (`[profile.dev.package."*"] debug = false`) | Debug info is codegen and link work for crates nobody steps into. The perf book says 20-40% with full debug info. We already use line tables, so expect less; the row measures it. | Backtraces lose line numbers inside dependencies only |
| OPT40 | Tools that spawn cargo run it at below-normal priority | Windows children inherit below-normal priority, so every rustc under that cargo runs below the desktop. The game window, editor and terminal stay smooth while builds still use the idle cores. It saves no CPU; it makes 100% CPU bearable. | One `os.setPriority(PRIORITY_BELOW_NORMAL)` on the tool's own process before `spawnSync` (Windows hands the class down to the child) |
| OPT41 | Old spike dirs do not build in the background | `target/spike-*` builds compete with live work for the same cores. A spike tool refuses to start when the pool has no free slot (`status`) unless `--force`. | none |
| OPT42 | LATER: nightly `-Zthreads=8` or the Cranelift backend for dev, in their own target dir | Perf book: up to 50% front-end speedup at best. Cranelift is Windows-x86_64 capable. Both are nightly only, so a spike row, never the default. | nightly toolchain |

## 4. Not for this PC

- Dev Drive needs Windows 11 22621.2338 or later. This PC runs Windows 10.
- mold has no Windows support. rust-lld is already the linker (OPT4: 121 s -> 79 s link).
- Dependencies at opt-level 1, to share generics: the dev "*" opt-level 3 line was measured
  (OPT3) and kept for a playable dev window. Do not flip it without a new A/B.
- Coverage-based test selection (Facebook's predictive selection, arXiv 1810.05286: half the
  test cost with over 99.9% of bad changes caught; RustyRTS, ICST 2025: 52-63% of run-all time).
  `tools/affected.mjs` (OPT7) already picks packages. Function-level selection waits for a tool
  that runs on Windows.
- Already on the board: split `engine-render` (PW43, the biggest lever left: 294 s of the 347 s
  dev build), wire `engine-workspace-hack` (OPT8/OPT36), nextest (OPT6).

## 5. For the owner (I cannot do these)

- Windows Defender scans every `.rlib` and `.exe` that rustc writes. Excluding the build dirs is a
  security trade-off, so the owner decides and runs it in an admin PowerShell:
  `Add-MpPreference -ExclusionPath C:\engine2040\target, $env:USERPROFILE\.cargo, $env:USERPROFILE\.rustup`
- Whether to make the pool permanent (user env var or `.claude/settings.json` `env`, plus a logon
  task that runs `tools/jobserver.ps1 start`).
