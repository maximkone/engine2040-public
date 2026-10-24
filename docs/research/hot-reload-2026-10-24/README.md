# Hot reload of gameplay code — design, 2026-10-24

Scope: one question. Can a gameplay edit reach the running arena **without a
restart**, and what exactly does it cost? This page is a plan, not a feature:
no Rust file changed, no flag added, no cargo run made (the box is shared).
Every repo number is cited as `path:line`, read at HEAD on 2026-10-24.

Labels, same as the sibling cards: **measured** = read from a file or printed by
a tool on this PC; **donor** = the donor project's own words; **arithmetic** =
plain division of measured numbers; **estimate** = ours, with the reason named;
**unmeasured** = nobody has the number and this page does not invent one.

Read next to `../build-speed-2026-10-24/README.md` (the per-crate codegen table
this page builds on) and `../rust-speed-2026-10-23/README.md` (workspace
splitting and nextest, already rejected there — not repeated here).

## 0. What this repo already decided, and what that forces

Three decisions exist already. A design that ignores them is dead on arrival.

| Decision | Where | What it forces |
|---|---|---|
| Presentation data swaps at a frame boundary; authoritative rules swap only in practice mode at a recorded tick; a competitive match pins its rules/content version; a broken file keeps last-good and shows a diagnostic | `../../SMOOTH-BUILD.md:17-21` | The reload is gated at a **tick boundary** and has a refusal path, not just a load path |
| No machine-code patch of sim crates; OPT13 is a reload drill (snapshot, relaunch, restore, same hash) | `../../plan/runtime.md:218` | The authority step (`engine-arena`) is **not** patched. A reload that changes sim behaviour is practice-only; anything else restarts |
| Allowed: a dev-only dylib for a presentation crate, copy the file, load the new one, drop the old | `../steals-2026-10-12/arch.md:46`, `:69` | The dylib shape is sanctioned for dev; the shipped binary keeps the static path |

So this design is not "patch the sim". It is: **the code above the authority
becomes a dev-only dynamic library; the fight state never leaves the host
process; the arena step is recompiled and reloaded only in practice mode, and
everywhere else a sim edit is a restart.**

## 1. Which crate is the hot library, and which is the host

| Package | Role | Built as |
|---|---|---|
| `engine-game` (`crates/game`) | **hot library**: the game shell + presentation-math code the host calls into today | `rlib` + `dylib` |
| `engine-play` (`crates/play`, new) | **host**: process, window, fight state, reload gate | binary only |
| `engine-arena`, `engine-combat`, `engine-ai`, `engine-render`, … | static, unchanged | rlib, linked into the host |

`engine-game` is the right first hot crate because it is exactly what the host
already calls: the window binary lives *inside* that package
(`crates/game/Cargo.toml:43-53`) and the per-frame call is
`self.app.tick_into(input, &mut self.tick_scratch)` at
`crates/game/src/bin/arena_frame/mod.rs:442`. Nothing else moves.

Exact manifest blocks:

```toml
# crates/game/Cargo.toml — new block above [features]
[lib]
name = "engine_game"
path = "src/lib.rs"
# rlib so the host can name the types; dylib so the reload can swap the code.
crate-type = ["rlib", "dylib"]
```

```toml
# crates/play/Cargo.toml — new package (workspace members = "crates/*", Cargo.toml:13)
[package]
name = "engine-play"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lints]
workspace = true

[features]
default = []
# The dev hot path. Off in every gate, so CI and ship never load a dll.
hot = ["dep:hot-lib-reloader"]
app = ["engine-game/app"]

[dependencies]
engine-game.workspace = true
engine-input.workspace = true
hot-lib-reloader = { version = "=0.8.2", optional = true }

[[bin]]
name = "play_reload"
path = "src/bin/play_reload.rs"
```

Why `dylib` and not `cdylib` (the task's word). The donor documents
`crate-type = ["rlib", "dylib"]` and exports plain `pub fn` with
`#[unsafe(no_mangle)]` taking Rust types (`&mut State`) — donor docs, "Library"
section. `cdylib` is a C ABI boundary: no Rust types, no unwinding across it,
and every value would need `#[repr(C)]`. That is a different and much larger
design; see section 7.

What the file is called and where the host looks: on Windows a `dylib` crate
`engine-game` writes `target/debug/engine_game.dll`; the donor's default
lookup is `$CARGO_MANIFEST_DIR/target/debug` — wrong here, because the host's
manifest is `crates/play`, so the module must pass `lib_dir` (donor docs,
"Change the name and location of the dylib file"). The donor also copies the
loaded file to a shadow name (`{lib_name}-hot-{load_counter}` by default),
which is why cargo may overwrite `engine_game.dll` while it is loaded — a
Windows lock would otherwise stop every reload (donor docs, "Adjust the dylib
filename").

Version pin: **`=0.8.2`**. It is the latest release, 2026-09-11, MIT
(docs.rs crate page) — MIT is on the allow-list (`deny.toml:12`). Its own deps
are `libloading ^0.8`, `notify ^6`, `notify-debouncer-full ^0.3.1`,
`crc32fast`, `hot-lib-reloader-macro ^0.8.0` (docs.rs "Dependencies").
`libloading 0.8.9` is already in the lock (`Cargo.lock:5351-5358`); `notify` is
not, so this brings the OS file watcher in as a new transitive dependency
(CC0-1.0 per `../powerups-2026-10-21/GH-E-engine-arch.md:24`, also allowed,
`deny.toml:20`). Exact pin, like the house style for dev tools
(`minidump-writer = "=0.13.0"`, `gilrs = "=0.11.2"`, `crates/game/Cargo.toml:127`,
`:130`): the macro and the runtime crate must stay one minor, and a reload
protocol that drifts mid-wave is not debuggable.

Cost to name now: `[lib] crate-type` cannot be feature-gated, so every
`cargo build -p engine-game` (and `--workspace`) also links the dll. That link
is **unmeasured**; step 1 records it.

## 2. The edge that has to be inverted

Today: one package, one static call.

```
engine-game (bin `arena` + lib, one package)
   arena_frame::mod.rs:442 -> self.app.tick_into(..)      // static, linked in
```

After:

```
engine-play (exe)  ── dlopen ──>  engine_game.dll
   holds `App`, `LiveMatch`, tape, cursor      hot_functions_from_file!("../game/src/hot.rs")
   calls hot_game::hot_tick(&mut app, ..)      hot_tick / hot_tick_empty / hot_layout_probe
   types come from the engine-game **rlib**    code comes from the **dll**
```

Host side, one module in `crates/play/src/bin/play_reload.rs`:

```rust
#[hot_lib_reloader::hot_module(
    dylib = "engine_game",
    lib_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug"),
    file_watch_debounce = 50
)]
mod hot_game {
    pub use engine_game::{App, GameConfig, TickOutcome};
    pub use engine_input::FighterInput;
    hot_functions_from_file!("../game/src/hot.rs");

    #[lib_change_subscription]
    pub fn subscribe() -> hot_lib_reloader::LibReloadObserver {}

    #[lib_updated]
    pub fn was_updated() -> bool {}
}
```

Lib side, new `crates/game/src/hot.rs` (the whole hot set, ~40 lines):

```rust
/// Advance one fixed tick (hot).
#[unsafe(no_mangle)]
pub fn hot_tick(app: &mut App, input: FighterInput, out: &mut TickOutcome) {
    app.tick_into(input, out);
}

/// Advance with every slot AI-filled (hot).
#[unsafe(no_mangle)]
pub fn hot_tick_empty(app: &mut App, out: &mut TickOutcome) {
    *out = app.tick_empty();
}

/// Layout handshake: what this build thinks the crossing types look like (hot).
#[unsafe(no_mangle)]
pub fn hot_layout_probe() -> [u64; 6] { /* size/align/offsets of App + TickOutcome */ }
```

`#[unsafe(no_mangle)]` is what the donor requires; edition is 2021 here
(`Cargo.toml:33`), where the plain form still compiles, but the `unsafe(...)`
form is what stays legal when the workspace moves to edition 2024.
`unsafe_code = "deny"` (`Cargo.toml:50`) does not apply to attributes.

The path in `hot_functions_from_file!` is documented as "relative to the
project root"; for a workspace member that is `crates/play`, hence
`../game/src/hot.rs`. If the macro resolves from the workspace root instead,
step 1 switches it to the absolute path — one line, decided by the compiler,
not by a guess here.

**The hot set is exactly the list in `hot.rs`.** Every other call the host makes
into `engine_game::` runs the rlib copy and is stale until a restart. That
single sentence is what keeps this design honest; section 5 makes it a rule.

## 3. The state problem: what must survive, and where it lives

Nothing is serialized on the normal path, because the fight never enters the
dll. The host owns it:

| Value | Type and file | Role across a reload |
|---|---|---|
| running session | `App`, `crates/game/src/lib.rs:794-917` | **host-owned**, passed as `&mut` for one call |
| authority stepper | `LiveMatch`, `crates/arena/src/live.rs:380` (private `Sim` inside) | **host-owned**, never in the dll |
| inputs that ran | `FighterTape` via `LiveMatch::recorded_tape()`, `live.rs:810` | the replay recipe |
| match identity | `MatchSpec`, `crates/arena/src/spec.rs:36` (config, rules, world, spear, height, weapons) | the rebuild recipe |
| tick receipt | `Checkpoint { tick, hash }`, `crates/arena/src/lib.rs:520` (`Copy`) | continuity proof after a swap |
| config row | `ArenaConfig`, `arena/src/lib.rs:209`; `ArenaRules`, `:237` | frozen for the match |
| full restore state | `MatchSnapshot`, `arena/src/lib.rs:571` | **not usable today** |

Fields of `App` that a reader will ask about: `live`, `tape`, `cursor`,
`rules`, `checkpoints`, `hp`, `hitstop_remaining`, `rounds`, `hud`, `crowd`,
`crowd_gpu`, `director`, `cut_director`, `present`, `present_ticks`,
`present_latest`, `present_sim` (`game/src/lib.rs:907`), `world`.

Three rules make this work:

1. **A swap happens between ticks only.** The donor's wrapper performs a
   pending reload at call time, so calling `hot_tick` at the top of a tick *is*
   the tick-boundary gate. No other call site exists.
2. **Continuity is the receipt.** After a swap the next `Checkpoint.hash` must
   equal the uninterrupted run's at the same tick. A break means the "hot" edit
   changed sim behaviour — that is the practice-mode-only path (`../../SMOOTH-BUILD.md:19-20`).
3. **The fallback is a replay, not a serialize.** If a change is not
   reloadable (a signature or a crossing type moved), the host restarts the
   round from `spec() + recorded_tape()`: `App::rebuild_live`,
   `game/src/lib.rs:1493-1504`, which walks `0..cursor` and bills the retired
   steps. `MatchSnapshot` would avoid that replay, but it has private fields,
   no serde, and `LiveMatch::snapshot` refuses non-empty worlds, non-v1 rules,
   spear and height (`live.rs:826-835`) — so it is **not** the reload state
   today, and making it one is a separate row (serde in `engine-arena`, whose
   dependency list has none today, `crates/arena/Cargo.toml:11-31`).

`App::save_progress` (`game/src/lib.rs:1921`) writes progression only; there is
no mid-match save, so nothing in the reload path depends on it.

## 4. What the win actually is, from this repo's numbers

Measured today in `../build-speed-2026-10-24/README.md`:

| Quantity | Value | Source |
|---|---|---|
| `cargo build -p engine-game` after a render edit (the current loop) | **102.7 s** wall | `:138` |
| `engine-game` unit in that run | 59.0 s wall, **51.1 s codegen** | `:167` |
| `engine-arena` unit, same run | 42.3 s wall, **39.6 s codegen** | `:168` |
| `engine-render` alone, warm, near-solo | 48.6 s (18.2 frontend + 30.4 codegen) | `:174` |
| edit to a sim-core crate, rebuild | **15 s** | `Cargo.toml:263` (measured 2026-09-29) |
| touch-to-game `cargo check` | 2.48 s | `Cargo.toml:273` (measured 2026-10-03) |
| fat arena cold link, `rust-lld` | 79 s | `../../SMOOTH-BUILD.md:144` |
| non-test lines, `crates/*/src` | game 57,547 · arena 13,200 · render 76,512 | counted today over `git ls-files` (node, read-only) |

Arithmetic on our own table (codegen seconds per non-test line): render 1.03 ms,
game 0.89 ms, arena 3.00 ms. So a **5k-line** hot crate is ~5 s (game-shaped)
to ~15 s (arena-shaped) of codegen.

What that means per edit target, with the hot library as proposed:

| Edit lands in | Today | With the hot dll | Why |
|---|---|---|---|
| `crates/game/src` (body only) | game unit + link | **15 s** (incremental, repo figure) to **51 s** (cold codegen) + dll link (**unmeasured**) | the hot crate is the only thing recompiled |
| `crates/arena/src`, `combat`, `ai` | arena 39.6 + game 51.1 s codegen | ~the same | they are dependencies of the hot crate, so the dll is rebuilt anyway |
| `crates/render/src` | 102.7 s | **102.7 s, unchanged** | render is not in the hot crate; a render edit rebuilds render (48.6 s warm) then the dll |

Reload latency itself: donor default debounce 500 ms, settable to 50 ms
(donor docs, "Adjust the file watch debounce duration"); the dlopen + symbol
rebind inside that is **unmeasured** on this PC. The number to record is
"mtime of the dll to the first tick that ran new code".

Honest summary, and this is the part to quote:

- Hot reload turns the **game-crate** edit loop from ~100 s into a
  two-digit number of seconds. That is 2x-6x, not 100x.
- It does **nothing** for a render edit and almost nothing for an
  `engine-arena` edit, because both still force a full codegen of the crate
  (51.1 s and 39.6 s measured).
- "About a second" (`../../SMOOTH-BUILD.md:17`) needs a **small** hot crate:
  the arithmetic above says ~5k lines is single-digit seconds. That is a crate
  cut (the presentation half out of `engine-game`), not a flag — the follow-up
  row, not this one.
- CI is unchanged. `hot` is off in every gate; the golden still runs the static
  path and must still print `d57b5c8ae3096614`.

## 5. ABI hazards and the one rule

Reloading Rust code is unsound by default. These are the concrete breaks in
this tree, not a generic list:

| Hazard | Where it bites here |
|---|---|
| **Statics** are per-load: each load gets a fresh copy, and the host has its own | `engine_clock::CLOCK` (`crates/clock/src/lib.rs:46`) with `ORIGIN` (`:71`); `perf` profiler `ENABLED`/`FRAMES_BEGUN` (`crates/perf/src/profiler.rs:52`, `:237`); `crash_receipt::WATCH` (`crates/perf/src/crash_receipt.rs:353`) |
| **TypeId / typed registries** change identity across a load | `engine-ecs` is a hard dependency of `engine-arena` (`crates/arena/Cargo.toml:30`); donor issue rksm/hot-lib-reloader-rs#34 (donor docs, "Global state in reloadable code") |
| **Allocator**: a `Vec`/`Box` made by one side and freed by the other | `TickOutcome` carries `Vec` fields (`game/src/lib.rs:686-703`) that the dll fills and the host drops |
| **`#[repr(Rust)]` layout** is not a contract; a field added in the dll is UB, not a compile error | `App` (`game/src/lib.rs:794`) and `TickOutcome` (`:673`) have no `repr` |
| **Generic monomorphisation**: a generic fn is copied into the host from the rlib and never hot | donor docs, "Hot-reloadable functions cannot be generic"; it fails *silently* (stale code), which is the worst failure mode |
| **Trait objects / fn pointers** point into the unloaded image | any `Box<dyn Trait>` built in the dll and stored by the host; `present_sim: Option<Box<LivePresentation>>` (`game/src/lib.rs:907`) is concrete and safe, a `dyn` twin is not |
| **TLS and threads**: a lib thread that outlives the unload calls unmapped code | `crates/arena/src/leg_pool.rs:29`, `crates/game/src/windowed_dressing.rs:340`, `crates/game/src/pad_rumble.rs:54` |
| **Signatures**: a changed argument list is a crash, not a diagnostic | donor docs, "No signature changes" |

The rule the team follows (put it at the top of `crates/game/src/hot.rs`):

> Hot code is a non-generic `#[unsafe(no_mangle)] fn` that takes `&mut` to
> host-owned state and plain-data arguments. Nothing created in the dll is
> stored by the host or outlives the call. No statics, no TLS, no threads, no
> `dyn` values, no registry/TypeId registration inside the hot crate. Every
> type that crosses the boundary has a frozen layout and a handshake probe. A
> change to a signature or a crossing type is a restart, not a reload.

The handshake (step 3) is what makes the rule enforceable: the dll reports
size/align/offsets plus its feature list and build id; the host compares them
with its own and **refuses** the swap with a diagnostic, keeping the last good
library — the same shape the data gate already uses for a bad file
(`crates/game/src/reload.rs:74-100`, `:151-194`).

## 6. Landing plan, seven commits

Each step is one commit, one proof command, and names its files. Step 1 needs
no GPU and no window, which is the point: this PC shares its GPU.

| # | Step | Files | Proof |
|---|---|---|---|
| 1 | Dll shape + the hot set + a headless host that swaps a library at runtime | `crates/game/Cargo.toml`, `crates/game/src/hot.rs` (new), `crates/game/src/lib.rs` (one `pub mod hot;`), `crates/play/Cargo.toml` (new), `crates/play/src/bin/play_reload.rs` (new), `tools/hot-reload-proof.mjs` (new) | `node tools/gh-check.mjs cargo test -p engine-game --lib hot_` (headless; the hot fn's bytes equal `App::tick_into` driven directly), then the live receipt `node tools/hot-reload-proof.mjs` → `loaded=1 ticks=300 swapped loaded=2 ticks=600 state_kept=true` |
| 2 | Swap only at a tick boundary; continuity receipt | `crates/play/src/bin/play_reload.rs`, `crates/play/tests/it/main.rs`, `crates/play/tests/it/reload.rs` (new) | `node tools/gh-check.mjs cargo test -p engine-play --test it reload_keeps_the_fight` (300 ticks, swap, 300 more: `sim_steps == 600`, no replay, final hash equals the uninterrupted run) |
| 3 | Refusal path: layout/build handshake, keep last good | `crates/game/src/hot.rs` (probe), `crates/play/src/bin/play_reload.rs`, `crates/play/tests/it/reload.rs` | `node tools/gh-check.mjs cargo test -p engine-play --test it reload_refuses_a_mismatched_lib` |
| 4 | The developer loop: watch sources, build the dll, print mtime→tick, pick the mode from the changed paths (sim crate ⇒ practice only) | `tools/hot.mjs` (new), `tools/hot.test.mjs` (new) | `node --test tools/hot.test.mjs` |
| 5 | The window's tick call goes through the hot module behind `hot` | `crates/game/src/bin/arena_frame/mod.rs` (the call at `:442`), `crates/game/Cargo.toml` (feature `hot`) | `node tools/gh-check.mjs cargo check -p engine-game --features app,hot` (compiles on a runner, no GPU) — the windowed run is an owner gate, not this row |
| 6 | Move the two bin targets and their module trees into the host, so the exe no longer links the hot rlib | `crates/game/Cargo.toml` (drop `[[bin]]`), `crates/play/Cargo.toml` (`[[bin]] arena`, `staged_fight`, `required-features = ["app"]`), `git mv crates/game/src/bin crates/play/src/bin` | `node tools/gh-check.mjs cargo check -p engine-play --features app,hot` + `cargo check -p engine-game` |
| 7 | Record the measured loop and close OPT13's open note | `docs/SPRINT-13-engine-edge.md`, `docs/plan/quality.md:396`, `../../SMOOTH-BUILD.md:154` | `node tools/workspace-check.mjs` (red today on another agent's `crates/endpoint/src/packs/project.rs`: run it when that lands) + the numbers in the row |

Kill criterion for step 1, stated before it starts: if rustc 1.97.1 cannot link
a `dylib` on this MSVC toolchain without a second std DLL that the exe also
needs, stop and re-plan with `cdylib` + a `#[repr(C)]`-only boundary (section 7,
first row). That is a bigger design and must not be smuggled in as a fallback
inside step 1.

## 7. Rejected, and what reopens it

| Idea | Why it is not the first move |
|---|---|
| `cdylib` + `extern "C"` | C ABI only: no Rust types, no unwinding, `#[repr(C)]` on everything (`App` is 124 lines of `repr(Rust)`, `game/src/lib.rs:794-917`). Larger boundary, not smaller |
| Serialize the whole session at every reload (the donor's reload-events pattern) | Needs serde on game+arena types that have none today (`crates/arena/Cargo.toml:11-31`), and `MatchSnapshot` cannot carry v2/world/spear/height (`live.rs:826-835`). Slower and bigger than a layout handshake, and it buys only the restart case |
| Make `engine-render` the hot library | It is the 48.6-102.7 s pain, but the dll would carry wgpu + winit (a second copy) and GPU handles cannot cross a reload; a render crash would cost the window |
| `subsecond` / the Dioxus jump-table detour | The repo already has the PORT note (`crates/game/src/dev_hotpatch.rs:31-33`) and the plan's verdict (`../steals-2026-10-12/arch.md:46`): it patches function pointers, it does not recompile Rust, so the cargo cost stays |
| `cargo-watch` alone | Saves keystrokes, not the 102.7 s (`:138`) |
| Two chained dlls (arena + game) in phase 1 | Two boundaries, two handshakes, a dll calling into another dll through its own symbol table. Defer until a measurement says arena edits are the common case |
| A Mun-style second runtime | Already rejected: the sim leaves the process, and with it one tick and one hash (`../powerups-2026-10-21/GH-E-engine-arch.md:31`) |
| Splitting `engine-render` to speed the hot library | Complementary, not a substitute: a split shortens a **static** rebuild (`../build-speed-2026-10-24/README.md:109`), and the hot crate still recompiles on every edit |
| Data hot reload (`crates/game/src/reload.rs`, practice rows) | Already shipped and stays the first tool for number tuning: HR2/TU1 is the path for values, this page is only for logic |

## 8. Open, unmeasured, and what step 1 must print

- **Dll link time** on this box: unmeasured. The 79 s figure
  (`../../SMOOTH-BUILD.md:144`) is the fat release arena exe, not a debug dll.
- **dlopen + rebind latency**: unmeasured; donor debounce floor 50 ms.
- **Incremental rebuild of a 57.5k-line crate after a one-function edit**:
  the repo's only figure is 15 s (`Cargo.toml:263`), path unstated. Step 4
  prints the real number per edit.
- **Whether `dylib` links on 1.97.1 MSVC without a second std**: the step 1
  kill criterion above.
- **A second copy of `engine-clock`/`engine-perf` statics** inside the dll:
  expected by the ABI table, never observed here. Step 1 prints the clock
  reading from both sides once to confirm the split.
- **Feature parity** between the dll and the host (`app` on or off) is a
  handshake field, not a promise.

## Sources read live 2026-10-24

- https://docs.rs/hot-lib-reloader/latest/hot_lib_reloader/ (0.8.2, 2026-09-11, MIT; usage, `lib_dir`, debounce, limitations)
- https://raw.githubusercontent.com/rksm/hot-lib-reloader-rs/master/README.md (same text, `crate-type = ["rlib", "dylib"]`)
- https://github.com/rksm/hot-lib-reloader-rs/issues/34 (TypeId after reload, cited by the donor docs)
