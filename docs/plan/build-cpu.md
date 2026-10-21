# Build CPU: one slot pool for every cargo on the PC

Owner 2026-10-21: "optimize our cpu usage af and all rust and cargo too ... 100% CPU, 27 cargo,
20 rustc". Rows OPT37-OPT42 on `docs/SPRINT-27-final-picture.md`. Earlier build work and its
numbers live in `docs/SMOOTH-BUILD.md` "Build speed facts"; a parallel research card from the same
day (`docs/research/build-cpu-2026-10-21/CARD.md`) ranks the flag levers and holds the thin-LTO
arena spike. This plan adds the shared slot pool and what neither page has.

## 1. What the PC was doing (census 2026-10-21, 12 CPUs)

- 36 `cargo.exe` processes were 18 builds: every cargo runs twice, the rustup proxy plus the real
  toolchain cargo. So "27 cargo" means about 13 builds, and most of them were not compiling at all.
  They were queued on the one `target/debug` build-dir lock.
- The builds that did compile ran in four target dirs at once: `target/debug` (tests),
  `target/release` (the golden verify, fat LTO, one codegen unit), `target/product` (another session's
  thin-LTO arena probe, since reverted), and an old spike dir (`target/spike-opt8/before2`, compiling `syn` at opt-level 3).
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

- `tools/jobserver.ps1 start [-Tokens 8]` holds the pool, `status` counts the free slots, and
  `env` prints the line. If the pool named in the env is not running, cargo falls back to its own
  `-j`, so a stopped holder never breaks a build.
- Measured on this PC 2026-10-21, with the box already at 100% from other sessions. A scratch
  crate with 7 dependencies, `cargo check --offline`:
  - 1-slot pool: at most 2 rustc at once, 591 s.
  - Its own `-j`: up to 10 rustc at once, 588 s.
  So on a full box the 8 extra jobs bought nothing; they only took CPU from every other build.
  A cargo whose env names a pool that is not running still builds (checked the same day).
- Slot balance (every 30 s): free slots never exceed `<Tokens>`. Extra ones come from an adopted
  pool or a slot given back twice, and the holder takes them back. A build killed mid-job never
  gives its slot back; when no rustc and no build script is running, the holder resets the free
  count to `<Tokens>`.
- Permanent since 2026-10-21 (owner: "make the pool permanent so all sessions join it"):
  `tools/jobserver.ps1 install` sets the user env var `CARGO_MAKEFLAGS`, puts a hidden holder in
  the Startup folder and starts it now. Every process started after that joins the pool. Apps
  and shells that were already running keep their old env until they restart. `uninstall`
  undoes all three. `tools/gh-check.mjs` and `tools/proof.mjs` also join on their own while
  the pool is up (OPT37), and run cargo below normal priority (OPT40; timed runs excepted).
- Builds outside the pool (sessions started before the env var, or a stopped holder) are capped
  at 4 jobs each by `[build] jobs = 4` in `.cargo/config.toml`, which every cargo reads fresh, so
  the cap reached the running sessions at once. An explicit `-j` still wins.
- One holder per pool: a named mutex marks the live holder. If a holder dies while builds
  still have the pool open, the next `start` adopts that pool instead of refusing.
- Tokens: 8 of 12. Each live build also keeps its one implicit slot, so 3-4 live builds land near
  12 jobs on 12 cores. More slots do not speed up a box that is already full (the A/B above).

## 2b. Work leaves the PC and is never done twice (OPT43, OPT44)

Owner 2026-10-21: "make sure they check all build perfecto on github on mirror repo and they reuse
the fuk out of it so we save our poor cpu too".

- Feature builds go to a Windows runner. `app`, `window`, `ui` and `device` lines that only need
  Windows to build used to compile on this PC: check, clippy, `--no-run`, and tests that open no
  GPU or window. They now run on the mirror's `windows-latest` runner. Only GPU, window and
  sound-device tests stay here.
- Every result is reused. gh-check keys a result by the line, the runner and the exact inputs it
  builds: its crates and their dependencies, the manifests, the lock, `content/`, `fixtures/`, and
  paths its code names. The same line on unchanged inputs answers at once: 0.4 s, against minutes
  on a runner. A pass is kept 24 h, a fail 30 min. A line another AI is already running is joined,
  and a local twin is waited for.
- The golden check goes through gh-check (`cargo test -p engine2040 --release --test proof_cli`), so
  GitHub builds the fat-LTO binary, not the PC (AGENTS.md "Proof").

## 2c. The GPU and window legs stay on this PC (lead decision 2026-10-21)

RL5, RL18, RL20, RL27 and RL28 need a real GPU adapter and a real window. The mirror's
`windows-latest` runner has neither, so these rows are not a CI check and never become one:
they are a PC-only leg, and they stay OPEN until a dated receipt exists under `docs/research/`.

The recipe, one command per step, no invented flags:

1. Build the window binary in release: `cargo build --release -p engine-game --features app --bin
   arena` (the product check rebuilds it when it is older than the code, one cargo slot).
2. Run the keeper's own eyes: `node tools/product-check.mjs`. It runs arena.exe twice (a look run
   with captures, a timing run), scores them against `tools/product-bar.json` and writes
   `target/reports/product/latest.md` plus the captures under
   `target/reports/product/runs/<sha>-<utc>/`.
3. Pin the stills layout: `node tools/q1-check.mjs` (read-only scan of `target/q1`: 4 synth and 30
   orbit PNGs, 2 manifests).
4. Read the frames: `node tools/frame-triage.mjs`. It counts the fighter-kit pixels in each frame
   and passes only above the floor; the manifest's claimed fighter count is reported and never
   decides.
5. Look at every PNG. The numbers do not replace the eye, and the eye does not replace the numbers.
6. Write the receipt: a dated file under `docs/research/` naming the commit, the GPU, the frame
   count, the triage numbers and what the eye saw.

A capture run is a heavy job, so it goes through center's queue instead of the box:

```
cd C:\empire\center
HEAVY_CMD_B64=<base64 of the command> node heavy.mjs run --repo engine --class priority
```

`priority` is the class the `proof` and `finish` runs take: it goes first in the queue, and
every wait is bounded (20 minutes), then the command runs anyway and the log says `failopen`.
When the frames must be timed, ask for `--class exclusive` instead: it waits for half the slots
to drain and holds that half, so the capture run is not fighting the other lanes for the same
cores. Center's own note from 2026-10-13 says exclusive was meant for fresh-clone runs, so it
is a deliberate choice for a timed capture, not the default.

A row ticks when the receipt exists and names the commit it was measured at. A CI run that never
opened a window proves nothing about these rows and does not tick them.

## 3. Smaller levers, in order

| Row | Lever | Why | Cost |
|---|---|---|---|
| OPT38 | The local golden verify builds with `--profile perf-test` (thin LTO, 16 units, overflow checks from release), not `--release` (fat LTO, 1 unit) | The fat-LTO relink is the slowest build in the tree, and every session's golden check pays it again after any engine edit. A community report puts fat LTO + 1 unit at +128% build time over plain release. The hash must not change: no fast-math, overflow checks stay on. (A `product` profile for the arena bin is the other session's spike: `docs/research/build-cpu-2026-10-21/CARD.md`.) | CI release-proof keeps `--release`; the row proves both profiles pass the golden test |
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
- Done 2026-10-21: the pool is permanent (`tools/jobserver.ps1 install`). Restart the Claude app
  and open terminals once, so the sessions already running pick up `CARGO_MAKEFLAGS`.
