# STEAL Dive 11 — Game feel, camera and combat design (2026-09-26)

**Dive**: 11 (Game feel, camera and combat design). Home: `crates/combat`, `crates/game`, `crates/anim`, `docs/plan/gameplay.md`, `docs/FOCUS.md`.
**Baseline**: hit-stop + reactions + permille camera trauma + HR3 move tables + PQ2 tournament. GP1/GP2/GP3/GP4/GP5/GP11/GP12 ticked; GP3 director (`crates/game/src/camera.rs`, 3283 lines, trauma recorded in-sim, shake presentation-side via GP16); GP11 feel schemas (`crates/combat/src/feel.rs`, MAX_BUFFER_TICKS=12); GP12 balance harness (`crates/game/src/bench.rs`).
**Fixture**: PQ1 latency (p95 ≤ 50 ms), PQ2 balance (40–60% win-rate, no move > 35% of kills), playtester session with captures.
**Kill**: feel measured by numbers only with no playtest, or any change that raises input latency.
**Method**: GitHub repo search (single-term seeds: `bevy camera`, `dolly camera`, `trauma shake`, `fighting game rust`, `MUGEN fighting engine`) + `github_get_file_contents` dir listings + `webfetch` LICENSE reads live. No donor repeat: Pichlmair/FootsiesGym/Gajardo/Halina-Guzdal/Parr (Dive 11 prior), flax-mcp GP8/GP9, dolly-adjacent `BlackPhlox/bevy_dolly` not separately taken.
**deny.toml allow-list**: MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0.
**Access date**: 2026-09-26 (all URLs + LICENSE reads).

## Ranked cards (best-first, 5) + 1 moonshot BET

### Card 1 — TAKE — `h3r2tic/dolly` — composable camera rigs (GP19)

- **Question**: Does a chain-of-drivers camera rig (each driver maps parent transform → child transform in order, final transform cached) transfer to `crates/game/src/camera.rs` as the structure under the landed GP3 director, replacing ad-hoc lock-on/trauma layering with named, ordered, testable stages?
- **Engine home**: `crates/game/src/camera.rs` (GP3 director, GP16 presentation feed). **Simpler baseline**: GP3's monolithic director with inline lock-on + trauma.
- **URL + rev + access**: https://github.com/h3r2tic/dolly @ `34931eee224a3227d277f2feec81e95d7287d017` (default `main`, 481★, updated 2026-09-15), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` + `LICENSE-APACHE` read live via webfetch/api — both on allow-list). Data/weights: none. `PORT(h3r2tic/dolly@34931eee:src/rig.rs)` + EVIDENCE row proposed below.
- **Exact claim**: a `CameraRig` holding `Vec<Box<dyn RigDriverTraits>>` plus a builder (`with(driver)` then `build()`), where `update(dt)` walks drivers in insertion order threading each stage's output as the next stage's parent, is approximately framerate-independent and yields one final camera transform. Drivers are looked up by type (`driver_mut::<T>()`).
- **Assumptions**: engine camera stages (orbit/follow → lock-on → trauma offset → framing clamp) factor as ordered pure maps over a transform; `dt` stays presentation-side (never in the tick); fixed-point/integer trauma track from GP3 feeds one stage.
- **Counterevidence**: donor is `f32` + heap (`Vec<Box<dyn>>`) with panicking lookups — both banned in the tick; donor has no determinism, no replay, no trauma semantics; Bevy binding (`BlackPhlox/bevy_dolly`) adds no new pattern.
- **What does NOT transfer**: the `f32` math, the boxed trait objects, the panics, any Bevy/ECS wiring. Only the *ordering idea* (named stages + builder + single final transform) transfers, reimplemented std-only with enum-dispatched fixed stages.
- **Cost / fixture / metric / budget**: S (1 session). Fixture: scripted 2-fighter orbit 600 ticks through GP16 feed; metric: both fighters framed every tick + trauma replay identical x2 + feed ≤ 0.05 ms. Budget: 0 per-tick heap allocs; zero float in sim.
- **Ablation/comparator**: current GP3 monolith (baseline) vs rig-staged director (candidate) vs per-stage bypass (each stage removable, framing still holds). Success threshold: stage order change needs only builder reorder; bypassing trauma stage changes ≤ 1 framed-tick.
- **Kill criterion**: any sim-float, any per-tick alloc, any framed-tick loss vs GP3 baseline, or feed > 0.05 ms.
- **Board row**: `[ ] GP19 game: staged camera rig under the GP3 director. Tier B. Size S. Owns stage hunks beside crates/game/src/camera.rs (director API frozen). Accept: 600-tick 2-fighter orbit keeps both framed every tick; bypass-one-stage still frames; trauma x2 identical; 0 allocs; feed ≤ 0.05 ms. needs: GP3, GP16.`
- **Falsifiable experiment + consumer**: reimplement 3 stages (follow → lock-on → trauma) as an enum-dispatched chain beside `camera.rs`; run the GP16 600-tick orbit twice; consumer is the WIN1 presentation feed. Fails (killed) if any tick drops framing vs baseline or bytes diverge.
- **EMPIRE outcome 4** (AI makes whole games): a staged rig is what the brief→game camera (AB1–AB4, GP16) drives — named stages are generatable and verifiable, unlike a monolith.

### Card 2 — TAKE — `bonsairobo/smooth-bevy-cameras` — exponential smoothing + look-transform split (GP20)

- **Question**: Does separating *where the camera wants to look* (`LookTransform { eye, target, up }`) from *the smoothed scene transform* (`Smoother(t)` applied only at sync) transfer as the damping discipline for GP3/GP16, killing frame-rate-dependent lerp bugs?
- **Engine home**: `crates/game/src/camera.rs` + `camera_feed.rs`. **Simpler baseline**: direct per-frame lerp of the scene transform.
- **URL + rev + access**: https://github.com/bonsairobo/smooth-bevy-cameras @ `1d0da095c73d9706bb86b79ea2ff20b2011b019b` (default `main`, 315★, updated 2026-08-21), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` read live, bonsairobo 2021 — on allow-list). Data/weights: none. `PORT(bonsairobo/smooth-bevy-cameras@1d0da09:src/lib.rs)` + EVIDENCE row below.
- **Exact claim**: keep an unsmoothed intent (`LookTransform`) that gameplay writes freely; a `Smoother(0..1)` component exponentially eases only the rendered `Transform` toward intent at sync time; FPS/orbit/unreal controllers all write intent through pitch/yaw helpers (`LookAngles`), never the rendered transform.
- **Assumptions**: intent can be integer/permille sim-side while smoothing stays presentation-side; smoothing factor is a presentation constant, never a sim input.
- **Counterevidence**: donor is Bevy-ECS + `f32`, with no tick/replay split — writing smoothing into sim would break HASH-LAW; donor controllers (FPS/orbit/unreal) are editor idioms, not arena framing.
- **What does NOT transfer**: Bevy plugins/bundles/systems, `f32` easing inside sim, editor controllers. Only the *split* (intent vs smoothed output) + exponential-sync rule transfers.
- **Cost / fixture / metric / budget**: S. Fixture: 20-step duel at 30/60/120 Hz presentation; metric: framing identical in sim bytes x2 while rendered path converges without overshoot; budget 0 sim allocs.
- **Ablation**: direct-lerp (baseline) vs intent+smoothed-sync (candidate) vs smoother bypassed (must equal raw intent). Threshold: bypass ≡ intent exactly; 3 refresh rates agree on sim bytes.
- **Kill criterion**: any sim-byte divergence across refresh rates, any overshoot that loses a fighter for ≥ 1 tick, any float in sim.
- **Board row**: `[ ] GP20 game: intent/smoothed-split camera damping. Tier B. Size S. Owns sync hunks beside crates/game/src/camera.rs + camera_feed.rs. Accept: intent bypass ≡ raw; sim bytes x2 identical at 30/60/120 Hz; no framed-tick loss vs GP3. needs: GP3, GP16.`
- **Falsifiable experiment + consumer**: add intent struct + presentation-only smoother; run duel at 3 refresh rates; consumer WIN1 feed. Killed if sim bytes differ by rate.
- **EMPIRE outcome 4**: rate-independent framing is required for generated games to look right on any machine — the brief→game pipeline (AB4 demo) cannot ship rate-coupled cameras.

### Card 3 — TAKE — `johanhelsing/bevy_trauma_shake` — trauma² + decay + seeded noise shake (GP21)

- **Question**: Does the donor's shake recipe — clamped trauma accumulator with linear decay, `trauma^power` response curve, amplitude × noise offset applied in `PostUpdate` and restored in `PreUpdate` — transfer as the GP3 trauma presentation stage, with the accumulator fed by the existing permille sim track?
- **Engine home**: `crates/game/src/camera.rs` trauma stage (sim track) + presentation shake. **Simpler baseline**: fixed-amplitude random-offset shake.
- **URL + rev + access**: https://github.com/johanhelsing/bevy_trauma_shake @ `e6eac1b10723964051d0eb606674786eb9116484` (default `main`, 28★, updated 2026-09-17), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` + `LICENSE-APACHE` read live — on allow-list). Data/weights: none. `PORT(johanhelsing/bevy_trauma_shake@e6eac1b:src/lib.rs)` + EVIDENCE row below.
- **Exact claim**: `Shake { trauma: 0..1 }` with `add_trauma` stacking-clamped, per-second linear decay, `powf(trauma, power≈2)` response so small hits whisper and big hits punch, simplex-noise offset × amplitude × response, applied after gameplay and undone before next gameplay so gameplay logic never sees the offset.
- **Assumptions**: sim posts integer trauma events only; all `f32`/noise lives presentation-side; restore-before-gameplay invariant holds (GP16 already separates this).
- **Counterevidence**: donor is 2D-only (`x/y` translation), Bevy `Time`-coupled, `f32 powf` + `noisy_bevy` simplex — none of that enters the tick; 28★, thin community proof; frequency/octave tuning is taste, not science.
- **What does NOT transfer**: 2D offsets, `powf`/simplex in sim, Bevy schedule, tuning constants. Only the *shape* (clamp → decay → power curve → apply/restore split) transfers, with engine-side integer curve + seeded noise.
- **Cost / fixture / metric / budget**: S. Fixture: scripted hit ladder (jab → launcher → super) 600 ticks; metric: shake energy monotonic in trauma, zero gameplay-path divergence x2; budget: presentation ≤ 0.05 ms share.
- **Ablation**: fixed-amplitude (baseline) vs trauma²-decay (candidate) vs power=1 (must feel linearly harsher on jabs — fails the subtlety check). Threshold: jab offset ≤ ¼ super offset at equal amplitude.
- **Kill criterion**: any gameplay-byte influence from shake, any missing restore (gameplay sees offset), PQ1 regression, or playtester "jabs punch like supers".
- **Board row**: `[ ] GP21 game: trauma-curve shake stage. Tier B. Size S. Owns shake hunks beside crates/game/src/camera.rs + camera_feed.rs. Accept: jab ≤ 1/4 super displacement; restore holds every tick; sim bytes x2; playtester confirms small/big separation. needs: GP3, GP16, GP11.`
- **Falsifiable experiment + consumer**: implement clamp/decay/power/integer-noise shake presentation-side; run hit ladder x2; consumer WIN1 feed + playtester captures. Killed if restore ever leaks into sim bytes.
- **EMPIRE outcome 4**: impact readability is what makes generated fights watchable — the AB4 demo and PQ2 tournament need small/big separation, not uniform jitter.

### Card 4 — TAKE — `saltmire/saltmire-juice` — one-call unified juice (shake + hitstop + flash + pop + pooled numbers) (GP22)

- **Question**: Does a single `impact()`-style call that fans out to five independent layers (trauma shake, freeze-frame hitstop, hit flash, elastic scale pop, pooled damage numbers) transfer as the GP11 feel-schema presentation wiring, so gameplay posts one event and presentation orchestrates?
- **Engine home**: `crates/combat/src/feel.rs` (GP11 schemas) + presentation wiring. **Simpler baseline**: each effect hand-wired per move.
- **URL + rev + access**: https://github.com/saltmire/saltmire-juice @ `258dbf25f3a131ae0960374b3a7c6c0b4d417ee9` (default `master`, 5★, updated 2026-09-11, Godot 4.6+), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE.txt` Saltmire 2026 read live — on allow-list). Data/weights: demo media unneeded, not taken. `PORT(saltmire/saltmire-juice@258dbf2:README.md#What's-inside)` (pattern port; GDScript → Rust reimplementation) + EVIDENCE row below.
- **Exact claim**: five one-liners (`shake`, `hitstop`, `flash`, `pop`, `damage_number`) behind one autoload; shake stacks trauma; hitstop freezes via time-scale with latest-wins nesting; flash fades back; pop is an elastic scale punch; damage numbers are pooled so spam is cheap — demo scene proves the combo.
- **Assumptions**: sim posts a single typed impact event with strength; every layer reads it presentation-side; hitstop freezes sim-clock delivery (fixed-tick pause), never sim math; pooling keeps 0 per-tick allocs.
- **Counterevidence**: GDScript/Godot autoload, `Engine.time_scale` freeze, `camera.offset` writes — none ports literally; 5★, no statistical proof; damage-number pooling in Godot ≠ engine text path.
- **What does NOT transfer**: Godot nodes/autoload/time-scale, sprite/flash specifics, font/number rendering. Only the *fan-out + layer-independence + pooling* discipline transfers.
- **Cost / fixture / metric / budget**: M (2 sessions). Fixture: PQ2 200-bout harness + playtester captures; metric: one impact event drives all 5 layers; p95 input ≤ 50 ms; 0 per-tick allocs. Budget: hitstop ≤ 6 ticks (MAX_BUFFER_TICKS=12 headroom).
- **Ablation**: hand-wired (baseline) vs fan-out (candidate) vs layers-toggled-off one-by-one (each toggle must degrade feel without breaking sim). Threshold: toggling any layer changes captures but never sim bytes.
- **Kill criterion**: any sim-byte change from layer toggles, any p95 rise, any per-tick alloc, or playtester cannot tell layers apart.
- **Board row**: `[ ] GP22 game: unified impact fan-out. Tier B. Size M. Owns wiring beside crates/combat/src/feel.rs + presentation. Accept: one event → 5 layers; per-layer toggles keep sim bytes x2; p95 ≤ 50 ms; 0 allocs; captures show all layers. needs: GP11, GP16, GP21.`
- **Falsifiable experiment + consumer**: wire one impact event to 5 presentation layers with kill-switches; run PQ2 harness x2 + captures; consumer arena match + WIN1. Killed if any toggle changes sim bytes.
- **EMPIRE outcome 4**: generated encounters (AB3) need one validated event → full presentation; hand-wiring per move does not scale to brief→game output.

### Card 5 — TAKE — `ValorZard/Quad-Fighter-2` — ECS move-state machine + rollback tutorial shape (GP23)

- **Question**: Does the donor's data-driven fighter shape — moves as shared ECS data with per-move startup/active/endlag phases, counter-hit/whiff-punish windows, hitstun + knockback, gatling cancels, meter-spend cancels — transfer as the HR3 move-table schema discipline the engine's `exchange.rs`/`sweep.rs` already resolve?
- **Engine home**: HR3 move tables → `crates/combat/src/exchange.rs` (GP1) + `sweep.rs` (GP2) + GP12 bench. **Simpler baseline**: one-off attack code per fighter.
- **URL + rev + access**: https://github.com/ValorZard/Quad-Fighter-2 @ `c96e98cd6a4ab40a389c0d95165c15062669cc22` (default `master`, 10★, updated 2026-08-25, "Open Source Rust Fighting Game with Rollback"), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` + `LICENSE-APACHE` read live — on allow-list). Data: KFM sprites CC-BY-NC per GAME-DESIGN — **not taken** (content rule). `PORT(ValorZard/Quad-Fighter-2@c96e98c:GAME-DESIGN.md)` (design pattern only) + EVIDENCE row below.
- **Exact claim**: fighters as data collections sharing whole move systems (a mimic fighter reuses another's systems); each normal has startup (counter-hittable) → active (hitboxes out, hitstun + knockback on contact) → endlag (whiff-punishable, cancellable into stronger moves); specials add projectiles/meter; rollback tutorial structure keeps logic deterministic-friendly.
- **Assumptions**: phases/cancels expressible as integer tick tables + resolver calls; art stays engine-owned (no CC-BY-NC sprites); rollback stays the engine's own net layer (MP rows), donor is schema witness only.
- **Counterevidence**: 10★ tutorial, unfinished ("MAKE SOMETHING WORK FIRST"), Bevy-era ECS idioms; no balance numbers; sprite source disqualified for git; rollback implementation not audited here.
- **What does NOT transfer**: sprites/art, Bevy code, any netcode bytes, gatling/cancel tuning. Only the *phase + cancel + shared-system* schema discipline transfers.
- **Cost / fixture / metric / budget**: M (2 sessions). Fixture: HR3 table for 2 fighters (semi-clone second, per donor) through PQ2 200 bouts; metric: all moves have all three phases; cancels only on listed windows; win-rate 40–60%, no move > 35% kills. Budget: 0 float in tables.
- **Ablation**: one-off code (baseline, rejected by construction) vs phased tables (candidate) vs phase-stripped tables (must produce whiff-punish/counter-hit collapse — proves phases matter). Threshold: stripped tables drop punish rates to ~0.
- **Kill criterion**: any phase-less move, any cancel outside windows, any float in table path, PQ2 outside band, or per-tick alloc.
- **Board row**: `[ ] GP23 game: phased move-table schema (startup/active/endlag + cancels). Tier B. Size M. Owns table hunks beside crates/combat/src/exchange.rs + sweep.rs (resolvers frozen). Accept: 2 fighters fully phased; cancels window-gated; PQ2 band holds x2; 0 float. needs: GP1, GP2, GP12.`
- **Falsifiable experiment + consumer**: author 2 phased fighters, run PQ2 harness x2 + phase-stripped control; consumer arena match + PQ2 tournament. Killed if stripped control still punishes (phases vacuous) or band breaks.
- **EMPIRE outcome 4**: brief→game fighters (AB3 encounters) must be data, not bespoke code — shared move systems are how generated rosters scale.

### Moonshot BET — `m2-md/game-feel-hitstop-juice` — deterministic layered impact engine (BET-GP24)

- **Bet**: a fully deterministic, seeded, unit-tested impact core (hitstop with partial-frame split + sim-time conservation; trauma² shake from seeded value-noise; volume-preserving squash `sx·sy=1`; brightest-flash-wins; single `impact()` merge: trauma adds, hitstop takes max) can be reimplemented integer-side as the GP11/GP18 feel kernel — making feel *replay-exact* instead of presentation-only.
- **Baseline**: feel lives presentation-side only (GP16 rule); sim posts trauma, presentation shakes.
- **Metric**: (a) `simDt=0` while frozen with zero sim-time loss; (b) seeded shake reproducible bit-exact x2; (c) jab ≤ ¼ super displacement; (d) `Feel.update` cost invisible (donor bench: ~7 ns/op logic, 720k shake-offsets/frame on Apple Silicon — engine must hold ≤ 0.05 ms camera share).
- **URL + rev + access**: https://github.com/m2-md/game-feel-hitstop-juice @ `64b82da02bda576307991660d4fa2e7479b71158` (0★, updated 2026-09-03, TypeScript + 19 logic tests + bench), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` Mustafa Morbel 2026 read live — on allow-list). Data/weights: none. `PORT(m2-md/game-feel-hitstop-juice@64b82da:src/feel.ts)` (reimplementation spec) + EVIDENCE row below.
- **Claim / assumptions / counterevidence / no-transfer**: claim — the four layers compose under one `impact()` with per-layer merge rules and a `timeScale` slow-mo; 19 pure tests (no DOM/time-injection) + bench prove logic-level determinism. Assumptions: engine reimplements noise as integer hash-noise and hitstop as tick-freeze accounting; squash maps to scale integers. Counterevidence: TypeScript/Canvas, float easing curves (31 Penner entries), 0★, taste-level tuning; float-in-tick would violate HASH-LAW — hence moonshot, not Tier A. No-transfer: easing library, Canvas demo, float noise — only merge rules + split-frame accounting + test list transfer.
- **Kill rule**: any float in tick path; any sim-time loss across a freeze; any seeded-shake divergence x2; PQ1 p95 rise > 2 ms; or playtester blind A/B cannot distinguish layered vs shake-only (feel vacuous).
- **Board row (BET)**: `[ ] BET-GP24 game (MOONSHOT): deterministic impact kernel. Tier C. Size M. Owns kernel beside crates/combat/src/feel.rs (behind GP18 gate). Accept: freeze conserves ticks x2; seeded shake bit-exact x2; jab ≤ 1/4 super; camera share ≤ 0.05 ms; blind A/B prefers layered. Kill: float in tick, time loss, divergence, p95 +2 ms, or A/B null. needs: GP11, GP18, PQ1.`
- **Falsifiable experiment + consumer**: port merge rules + freeze accounting to integer kernel; run freeze-conservation + seeded-shake x2 + blind A/B; consumer PQ2 tournament + captures. Killed by the rule above.
- **EMPIRE outcome 4**: if feel itself becomes deterministic, generated games (brief→game) ship feel that replays — the bold version of "AI makes whole games".

## Rejects (one-line reasons)

- `ikemen-engine/Ikemen-GO` (Go, 1564★, engine MIT verified live at `LICENCE.txt`): REJECT — engine MIT but bundled art CC-BY-3.0 + release screenpack separate licence + Go (not a Rust consumer crate) + MUGEN content risk; ideas only.
- `SMITTY-94/bevy_third_person_camera` (MIT+Apache-2.0 verified live): REJECT — thin Bevy wrapper over the dolly/intent pattern with no new falsifiable gap beyond Cards 1–2.
- `reu/rugen` (Rust MUGEN libs), `haihala/whoops-all-grapplers` (Rust 2.5D fighter, 9★), `flmmkch/nugem-rs` (Rust MUGEN, stale 2024): REJECT — no new schema beyond Card 5's phased tables; revisit only with played-bout numbers.
- `BlackPhlox/bevy_dolly`: REJECT — binding of Card 1, not a second donor.

## EVIDENCE rows (proposed)

| Idea | Source | Verdict |
|---|---|---|
| Ordered camera-rig stages (builder + final transform) | `PORT(h3r2tic/dolly@34931eee:src/rig.rs)` MIT+Apache-2.0, https://raw.githubusercontent.com/h3r2tic/dolly/main/LICENSE-MIT, verified 2026-09-26 | IN — GP19 |
| Intent vs smoothed-output damping split | `PORT(bonsairobo/smooth-bevy-cameras@1d0da09:src/lib.rs)` MIT, https://raw.githubusercontent.com/bonsairobo/smooth-bevy-cameras/main/LICENSE, verified 2026-09-26 | IN — GP20 |
| Trauma clamp/decay/power + apply/restore shake | `PORT(johanhelsing/bevy_trauma_shake@e6eac1b:src/lib.rs)` MIT+Apache-2.0, https://raw.githubusercontent.com/johanhelsing/bevy_trauma_shake/main/LICENSE-MIT, verified 2026-09-26 | IN — GP21 |
| One-event five-layer juice fan-out + pooling | `PORT(saltmire/saltmire-juice@258dbf2:README.md)` MIT, https://raw.githubusercontent.com/saltmire/saltmire-juice/master/LICENSE.txt, verified 2026-09-26 | IN — GP22 |
| Phased move tables (startup/active/endlag + gatling/meter cancels) | `PORT(ValorZard/Quad-Fighter-2@c96e98c:GAME-DESIGN.md)` MIT+Apache-2.0 code only (sprites CC-BY-NC excluded), LICENSE-MIT read live, verified 2026-09-26 | IN — GP23 |
| Deterministic impact merge (trauma-add/max-freeze/split-frame/seeded noise) | `PORT(m2-md/game-feel-hitstop-juice@64b82da:src/feel.ts)` MIT, https://raw.githubusercontent.com/m2-md/game-feel-hitstop-juice/main/LICENSE, verified 2026-09-26 | BET — GP24 moonshot |

## Unverified

- No code cloned or run; all takes are pattern reimplementations (Rust, integer-sim + presentation-float split) — REPRODUCE stamps require the listed experiments.
- Tuning constants (power≈2, decay 0.8/s, amplitude, freeze ticks) are donor taste; engine values come from PQ1/PQ2 + playtester A/B, not donor numbers.
- `SMITTY-94/bevy_third_person_camera` license files read via API listing only (not webfetch body); rejected regardless, so no take depends on it.
