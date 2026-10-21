# Hunter 2: sound, UI and input

Area: `crates/audio`, `crates/ui`, `crates/game`, `crates/web`, `crates/input`.
Wave 2 of the 2026-10-21 hunt. Every star count below was read from `gh api repos/<r>` on
2026-10-21; every file size from `gh api repos/<r>/git/trees/<branch>?recursive=1`; every
licence from the repo's own `LICENSE` / `NOTICE` file, read live. Our file:line anchors are
lines I read in this tree on 2026-10-21.

What wave 1 already mined and I did **not** re-mine: `emilk/egui`, `tesselode/kira`,
`RustAudio/cpal`, `projectfluent/fluent-rs`, `gilrs-project/gilrs`,
`Leafwing-Studios/leafwing-input-manager`, `simgine/bevy_enhanced_input`,
`ValveSoftware/steam-audio`, `mrDIMAS/hrtf`, `grovesNL/glyphon`, `rustwasm/gloo`,
`piutranq/wasm-audioworklet-example`, `DioxusLabs/taffy`, `deepnight/gamefeel`,
`facebookresearch/audiocraft`, `suno/bark`, `DanielSWolf/rhubarb-lip-sync`,
`nickovic/rtamt`, `StuckiSimon/strahl`, `nicbarker/clay` (the C original — see #8, the
Rust binding is the new part), `Bevy`-family UI, `godotshaders.com`.

---

## Ranked donors (1..12)

| # | donor | license | stars | what to steal (one sentence) | our file:line | our row | measurable accept |
|---|---|---|---|---|---|---|---|
| 1 | [GoogleChrome/omnitone](https://github.com/GoogleChrome/omnitone) | Apache-2.0 (code, root `LICENSE`); **assets also Apache-2.0** (`src/resources/NOTICE`: University of York SADIE, 2017) | 914 | The SH-HRIR dataset plus the FOA/SOA/TOA binaural convolution renderer: 3 tiny WAVs (order 1 = 2.0K / order 2 = 4.5K / order 3 = 8.0K, 48 kHz, 16-bit, 4/9/16 ch, 256 samples) replace our octave-sphere fake with real measured ears. | `crates/audio/src/hrtf.rs:16` (`pub const HRTF_TAPS: usize = 16;` — a 16-tap, 8-octant analytic sphere, not a measured HRIR); `crates/web/src/audio.rs:1` | SN4, PW38 | `cargo test -p engine-audio --lib hrir` prints `front_db=-2.1 back_db=-11.4` (front/back separation >= 6 dB at 48 kHz from the order-1 SH set) and the speakers path stays bit-identical (byte-equal to the pre-change mix). |
| 2 | [Chlumsky/msdfgen](https://github.com/Chlumsky/msdfgen) | MIT (`LICENSE.txt`: "Copyright (c) 2014 - 2025 Viktor Chlumsky") | 4958 | Multi-channel signed distance field glyph generation so HUD text is crisp at any size instead of a 5x7 ASCII bitmap: `core/edge-coloring.cpp` 21.4K, `core/MSDFErrorCorrection.cpp` 25.7K, `core/edge-segments.cpp` 17.4K. | `crates/ui/src/text.rs:212` (`pub struct Atlas;` — the O-023 5x7 bitmap, 95 glyphs, 96x48 atlas); `crates/ui/src/text.rs:41` (`hud_text_px`) | UIX13, NB4 | `cargo test -p engine-ui --lib hud_text`: every glyph in the objective line is >= 28 px tall at 1080p with no padding seam, and the same string rendered at 28 px and 64 px differs by 0 broken pixels (edge-error count printed). |
| 3 | [linebender/vello](https://github.com/linebender/vello) | Apache-2.0 **and** MIT (dual; `vello_common/LICENSE-APACHE` 9.9K + `LICENSE-MIT` 1.0K) | 4431 | A GPU compute 2D renderer with a flat encoded command list (`vello_common/src/encode.rs` 52.7K) and a text path (`vello_cpu/src/text.rs` 16.7K, `vello_gpu/src/text.rs` 10.5K) — the missing body of the HUD pass our graph declares. | `crates/render/src/graph.rs:1517` (`PassDesc::new("hud")` — declared, nothing runs it); `crates/render/src/overlay.rs:64` (`push_quad`) | UIX13, LEG9 | `cargo test -p engine-render --lib hud_pass` (the row's own Accept): draws <= 3, `text_ms <= 0.3` at 1080p, stamina/poise/pip pixels > 0, text contrast >= 4.5. F2P today: the arena capture has 0 HUD pixels. |
| 4 | [pop-os/cosmic-text](https://github.com/pop-os/cosmic-text) | Apache-2.0 **and** MIT (dual; `LICENSE-APACHE` 11.1K + `LICENSE-MIT` 1.0K). Fonts inside the repo carry their own licences (`fonts/Inter-LICENSE`, `fonts/NotoSans-LICENSE`, `fonts/FiraMono-LICENSE`) — separate asset licence. | 2174 | Shaping + font fallback + bidi in Rust: `src/shape.rs` 116.6K, `src/font/fallback/mod.rs` 17.6K, `src/bidi_para.rs` 2.6K, `src/buffer.rs` 65.4K, and a perf gate to copy (`benches/text_shaping_benchmarks.rs` 4.7K). | `crates/ui/src/locale.rs:1` ("static en/de menu labels with fallback plus integer plurals" — no RTL, no font fallback); `crates/ui/src/text.rs:202` ("Printable ASCII glyph (`0x20..=0x7E`; anything else maps to `?`)") | PLF7, UIX13 | `cargo test -p engine-ui --lib shaping`: an Arabic sample lays out right-to-left with 0 overlapping glyph boxes and a Hebrew sample mirrors; `cargo bench -p engine-ui -- shaping` prints <= 0.05 ms per glyph. |
| 5 | [SamiPerttu/fundsp](https://github.com/SamiPerttu/fundsp) | Apache-2.0 **and** MIT (dual; `LICENSE-APACHE` 9.5K + `LICENSE-MIT` 1.0K) | 1227 | The DSP we simply do not have: a look-ahead `Limiter` and peak/RMS `MeterState` (`src/dynamics.rs` 13.4K), a Schroeder/FDN reverb `Combinator` (`src/combinator.rs` 12.5K), panning laws (`src/pan.rs` 4.2K), `fir`/`convolve` (`src/convolve.rs` 1.5K). | `crates/audio/src/mix.rs:31` (`pub struct CombatMix` — voice groups and caps, **no bus, no limiter, no meter**; `grep -i "limiter\|compressor\|meter" crates/audio/src` returns 0 hits); `crates/audio/src/reverb.rs:1` | SN12, UW23, SN13-left | `cargo test -p engine-audio --lib limiter`: a 16-voice simultaneous burst leaves the limiter with output <= -0.1 dBFS and 0 samples over; `meter_rms` prints a peak within 0.5 dB of the reference. |
| 6 | [linebender/parley](https://github.com/linebender/parley) | Apache-2.0 | 764 | Rich text layout so the HUD objective line wraps and scales instead of hard-wrapping in a fixed cell grid: `parley/src/layout/line_break.rs` 83.8K, `layout/cluster.rs` 21.0K, `layout/line.rs` 20.6K, `layout/spacing.rs` 7.8K. | `crates/ui/src/text.rs:442` ("Lay `text` out in a `max_width_cells` grid with hard wrap"); `crates/ui/src/lib.rs:866` (`pub struct DuelOverlay`) | UIX13, NB4, LEG9 | `cargo test -p engine-ui --lib hud_text`: the 120-character objective line wraps to 2 rows at 1280x720 and 1 row at 2560x1080 with 0 clipped glyphs; the row-1 baseline sits >= 4 px above row-2. |
| 7 | [mgth/Omniphony](https://github.com/mgth/Omniphony) | GPL-3.0 — **port the algorithm into our std-only crate, do not vendor the crate or link it** | 104 | Object-based spatial audio in Rust: VBAP speaker-pair weights plus a binaural path and ambisonic order bookkeeping, so our single pan law becomes a real object panner. | `crates/audio/src/spatial.rs:76` (our own rational `1/r` falloff and pan); `crates/audio/src/hrtf.rs:16` | SN4, UW23 | `cargo test -p engine-audio --lib spatial_vbap`: a source on a 5.1 ring resolves to the 2 nearest speakers with VBAP weights that sum to 1.0 +-1e-6 and the 3 others are 0; the HRIR path is byte-identical to before. |
| 8 | [clay-ui-rs/clay](https://github.com/clay-ui-rs/clay) | Zlib (`LICENSE.md` 0.9K) | 144 | The Rust binding shape and the wgpu renderer that UIX13 already names as "the Clay shape": `examples/wgpu/ui_renderer.rs` 29.1K, `examples/wgpu/ui_layout.rs` 18.2K, `examples/wgpu/ui_shader.wgsl` 0.6K, `clay.h` 263.6K — a flat command list of rect, border, text, scissor. | `crates/ui/src/lib.rs:866` (`DuelOverlay` snapshots); `crates/render/src/overlay.rs:64` (`push_quad`, fixed 128-quad budget) | UIX13 | `cargo test -p engine-render --lib hud_pass`: the whole HUD is <= 3 draw calls, the command list is <= 256 entries, and a capture at 1920x1080 shows the stamina strip under the HP bar and the poise ring segments. |
| 9 | [britzl/blip](https://github.com/britzl/blip) | MIT (`LICENSE.md` 1.0K). The repo's `sounds/*.wav` are example assets with their own terms — not needed if we synthesize. | 24 | The seeded SFXR parameter set and 7 ready generators (`blip/src/sfxr.h` 1.6K, `blip/src/sfxr.cpp` 14.4K): Blip, Jump, Hurt, Powerup, Explosion, Laser, Pickup — one seeded generator gives every impact kind a clip with **no WAV asset and no provenance row**. | `crates/audio/src/cue.rs:202` (`pub fn impact_cue(kind, surface)` — 8 kinds x 6 surfaces, currently only 4 distinct families); `crates/audio/src/combat_cues.rs:1` | SN12, BX11, NS18, CT63 | `cargo test -p engine-game --lib hit_matrix` (the row's own Accept): distinct clips >= 12, the same clip never twice in a row, 8 of 8 event kinds have a cue, audio-thread allocations 0. F2P today: distinct clips = 1. |
| 10 | [AccessKit/accesskit](https://github.com/AccessKit/accesskit) | BSD-3-Clause | 1544 | The UI accessibility tree (Role, Label, Live, focus order) so the HUD is a real tree and not only contrast numbers: `accesskit/src/lib.rs` 126.2K, `accesskit/src/geometry.rs` 21.7K. | `crates/ui/src/access.rs:24` (`pub fn hud_palettes()` — 4 palettes + contrast + captions, no tree); `crates/ui/src/access.rs:155` (caption strip) | UIX13, PLF7, LEG9 | `cargo test -p engine-ui --lib a11y_tree`: the `DuelOverlay` emits one tree whose root role is `StatusBar`, one named label per fighter bar, `live=polite` on the objective line, and every contrast pair still >= 4.5:1. |
| 11 | [MoAlyousef/soloud-rs](https://github.com/MoAlyousef/soloud-rs) | MIT (`LICENSE` 1.0K) | 61 | The bus shape we lack: `soloud/src/audio/bus.rs` 4.6K gives a bus an active-voice count, its own volume, `play_clocked` (scheduled, jitter-free) and `play_3d`, plus `soloud/src/audio/sfxr.rs` 1.5K (SoLoud's own SFXR voice). | `crates/audio/src/mix.rs:31` (`CombatMix` — one flat group list); `crates/audio/src/director.rs:269` (intensity ramps); `crates/audio/src/crowd.rs:28` (`HIT_DUCK_TICKS = 20`, a hand-rolled duck) | SN12, UW23, LEG9 | `cargo test -p engine-audio --lib bus`: 3 buses (music, sfx, crowd) each cap their own voices independently; a music cue scheduled at bar N starts within 1 sample of `bar_samples(sample_rate, bpm)`. |
| 12 | [mdqinc/SDL_GameControllerDB](https://github.com/mdqinc/SDL_GameControllerDB) | Zlib (`LICENSE` 0.9K) | 1782 | The community controller mapping table (`gamecontrollerdb.txt` 595.5K) so a prompt says the button the player's actual pad has, and the same pad keeps its identity across a driver change. | `crates/game/src/pad.rs:22` (`pub struct PadFrame` — "no device handles, no threads"); `crates/input/src/bindings.rs:43` ("Gamepad button index (data only; live gamepad polling is a later lane)") | UIX14 | `cargo test -p engine-game --lib rumble_merge`: 7 cues mapped, motors off <= 10 ms after focus loss, a pad disconnect pauses a local duel within 1 tick, and the prompt glyph for the DB entry `a:b0` is "A", not a hardcoded letter. |

### Notes on the licence risk in this table

- #7 (GPL-3.0) and #5's Apache/MIT dual are the only two that need a note. Per `AGENTS.md`
  rule 4 a donor may be ported or adapted from any source, so #7 is legal as an **algorithm**
  donor, but the crate itself must not be vendored or linked into our tree. Every other donor
  is permissive (MIT, Apache-2.0, BSD-3-Clause, Zlib) and can be ported line by line.
- Asset licences are separate from code licences and are called out in the table: omnitone's
  HRIR WAVs are Apache-2.0 (SADIE / University of York, `src/resources/NOTICE`), cosmic-text's
  bundled Inter/NotoSans/FiraMono fonts each carry their own `fonts/*-LICENSE` file, blip's
  `sounds/*.wav` are example assets.

---

## Wild ideas (10)

Not repos yet; each with the measurement that would prove it.

1. **One feel bank for every sound.** A single seeded generator (blip-style) plus one
   parameter vocabulary produces every cue — hit, block, parry, footstep, crowd, sting — so a
   single "material weight" knob moves the whole game at once.
   *Proof:* `cargo test -p engine-audio --lib feel_bank` prints one coefficient set and every
   clip's spectral centroid lands within 10% of its target; changing the weight shifts all
   centroids by the same ratio.
2. **A bus tree instead of a crowd hack.** music -> sfx -> crowd -> master, each bus with an
   envelope follower and a sidechain send, so ducking is a property of the graph and not of
   one voice.
   *Proof:* `cargo test -p engine-audio --lib bus_duck` — the crowd bed dips exactly
   `HIT_DUCK_DEPTH` (0.5, `crates/audio/src/crowd.rs:32`) on the hit tick and recovers over
   `HIT_DUCK_TICKS` (20, `crates/audio/src/crowd.rs:28`); the music bus dips 0.15.
3. **A limiter and a meter as a hard gate.** Nothing may leave the mixer over -0.1 dBFS, and
   every bus reports peak and RMS.
   *Proof:* `cargo test -p engine-audio --lib limiter` — a 16-voice burst has 0 samples above
   -0.1 dBFS; `mixer_peak_db` prints peak and p95 for a 600-tick bout.
4. **Zero audio-thread allocations as a gate, not a note.** Every allocation happens on the
   feed; the mix block allocates 0 bytes.
   *Proof:* `cargo test -p engine-audio --lib audio_thread_allocates_nothing` counts 0 heap
   calls in a 600-tick, 2-fighter bout.
5. **Voice-cap stealing without clicks.** When a group is over its cap, steal the oldest
   voice with a 3 ms fade instead of cutting it, so the mix never has a sample-to-sample
   jump.
   *Proof:* `cargo test -p engine-audio --lib steal_is_silent` — 12 voices over a cap of 8
   produce 0 samples where |x[n] - x[n-1]| > 0.05.
6. **The camera is the listener, and the test says so.** Turning the camera 90 degrees must
   swap the ears exactly, in the pan path and in the HRIR path alike.
   *Proof:* `cargo test -p engine-audio --lib listener_yaw_swaps_ears` — L and R channel
   energies swap within 1 dB after a 90-degree yaw.
7. **Localization as build-time data.** Every player-facing string is a key in a Fluent file;
   a build gate fails on a hardcoded label; the pseudo-locale stays as the length test.
   *Proof:* `node tools/workspace-check.mjs` exits non-zero when a new hardcoded label lands
   in `crates/ui`; the German table has 100% key coverage and the pseudo-locale is >= 40%
   longer than English.
8. **The HUD as a mini scene graph.** Per-element anchors (top-left, bottom-right), a fixed
   safe margin, and a whole-pixel DPI scale, so the HUD is authored once and survives any
   aspect ratio.
   *Proof:* captures at 1280x720, 1024x768 and 2560x1080 keep the HP strip inside a 5%
   margin and the text >= 28 px at 1080p (the NB4 Accept), and a test asserts the strip keeps
   its pixel size +-1.
9. **One goldensheet for the whole HUD.** Render the HUD to a PNG sheet and diff it against a
   stored sheet, so a palette, font or safe-area change is a visible diff, not a silent one.
   *Proof:* `cargo test -p engine-render --lib hud_sheet` writes a 3-panel PNG and the gate
   fails when the changed-pixel count exceeds 0; `tools/product-check.mjs` counts a PNG under
   60,000 bytes as blank.
10. **Gamepad-first input.** The binding table stores the physical button as
    (vendor, product, button index), the prompt follows the last device, and unplugging
    pauses the duel inside one tick.
    *Proof:* `cargo test -p engine-game --lib rumble_merge` — 7 cues mapped, motors off <=
    10 ms after focus loss, a pad disconnect pauses a local duel within 1 tick, and a
    `--features pad` test lists >= 1 fake pad.

---

## Rejected (10)

Looked at, dropped, with the measured reason.

1. [grame-cncm/faust](https://github.com/grame-cncm/faust) — 3192 stars, licence
   **NOASSERTION** (GitHub found no LICENSE file). Dropped: there is no licence to name, and
   it is a whole DSP-to-C/WASM compiler, not a crate we could take without adding a build
   step to a tree under a build hold.
2. [ooesili/sorceress](https://github.com/ooesili/sorceress) — 149 stars, GPL-3.0. Dropped
   as a vendor candidate: fundsp (Apache-2.0/MIT, 1227 stars) covers the same synthesis
   ground permissively.
3. [google/flatui](https://github.com/google/flatui) — 651 stars, Apache-2.0, but
   **archived** (last push 2019-05-11, `archived: true`). Dropped: dead for 7 years, and
   micro-ui/Clay plus vello are the maintained shapes.
4. [MeadowlarkDAW/Dropseed](https://github.com/MeadowlarkDAW/Dropseed) — 69 stars, AGPL-3.0.
   Dropped: AGPL-3.0 and it is a DAW graph, not a game mix bus; soLoud's `bus.rs` (4.6K) is
   a better fit for our 3-bus shape.
5. [cococry/leif](https://github.com/cococry/leif) — 1040 stars, licence **NOASSERTION**.
   Dropped: no LICENSE file to name, and it targets OpenGL while our renderer is
   wgpu/WebGPU.
6. [xjmusic/xjmusic](https://github.com/xjmusic/xjmusic) — 96 stars, licence **NOASSERTION**,
   Java. Dropped: no machine-readable licence and a JVM toolchain; our music crate already
   has layers plus tension hysteresis (`crates/audio/src/music.rs:259`, `TENSION_HYSTERESIS`
   = 0.05).
7. [leomccormack/Spatial_Audio_Framework](https://github.com/leomccormack/Spatial_Audio_Framework)
   — 757 stars, licence **NOASSERTION** (LGPL text + JUCE). Dropped: it needs JUCE, which is
   not free for every use, and the licence is not machine-readable.
8. [videolan/libspatialaudio](https://github.com/videolan/libspatialaudio) — 321 stars,
   licence **NOASSERTION** (LGPL-2.1). Dropped: a C++ library with a C++ toolchain in a
   tree we must not build; Omniphony (#7) gives VBAP plus binaural in Rust.
9. [unicode-org/icu4x](https://github.com/unicode-org/icu4x) — 1874 stars, licence
   **NOASSERTION** (Unicode-3.0). Dropped as overkill: cosmic-text gives us fallback and
   bidi in two files (17.6K + 2.6K) and we need one locale pair, not a full ICU.
10. [harfbuzz/rustybuzz](https://github.com/harfbuzz/rustybuzz) — 666 stars, MIT, but
    **archived** (`archived: true`, last push 2026-07-26). Dropped: dead, and shaping
    without layout is not what we need — parley plus cosmic-text give both, both maintained.

Also dropped without a full read because the licence or the shape was wrong on the first
look: `cimgui/cimgui` (C++ imgui binding, no Rust), `artemtsitronov/glacex` (129 stars, MIT,
a from-scratch GPU UI — overlaps vello and has no text path worth stealing),
`ExtraBinoss/argui` (34 stars, Apache-2.0, a React/QuickJS gallery, not a game HUD),
`longbridge/rust-i18n` (682 stars, MIT, but loads YAML at runtime; our PLF7 wants build-time
Fluent tables — the typed-Fluent idea was taken from `human-solutions/fluent-typed` instead,
14 stars, MIT).

---

## Donors seen but skipped (wave 1, deliberately not re-mined)

| donor | why skipped |
|---|---|
| `emilk/egui` | the immediate-mode toolkit; wave 1 already read it, and our HUD is a retained quad+text pass, not an imgui context |
| `tesselode/kira` | the Rust audio timeline; wave 1 read it, and our mix is a fixed-step presentation layer, not a scheduler |
| `RustAudio/cpal` | the device layer; PW38 already names `cpal`'s AudioWorklet host as its donor |
| `projectfluent/fluent-rs` | PLF7 already cites it in `crates/ui/src/locale.rs:8-10` as a PORT |
| `gilrs-project/gilrs` | the pad backend; UIX14 already names gilrs polling as its donor |
| `Leafwing-Studios/leafwing-input-manager`, `simgine/bevy_enhanced_input` | the input-action shapes; GP46 already landed tap/hold/chord from that pattern |
| `ValveSoftware/steam-audio` | the HRTF reference; SN4 already names it, and omnitone (#1) gives a licence-clean HRIR dataset instead |
| `mrDIMAS/hrtf` | the HRTF generator; the wave-1 entry; omnitone (#1) is the licence-clean, measured-dataset alternative |
| `grovesNL/glyphon` | the text shaping library; UIX13 already cites it and rejects it for pinning wgpu 30 |
| `rustwasm/gloo`, `piutranq/wasm-audioworklet-example` | the web audio half; PW38 already names the cpal worklet as its donor |
| `DioxusLabs/taffy` | the layout engine; already cited by our own layout work |
| `deepnight/gamefeel` | the game-feel reference; wave 1 read it, and its numbers are in GP6 |
| `facebookresearch/audiocraft`, `suno/bark`, `DanielSWolf/rhubarb-lip-sync`, `nickovic/rtamt` | the NS18 offline sound intake; wave 1 read them and their licence terms (CC-BY-NC weights) are why NS18 says "weights never in repo" |
| `StuckiSimon/strahl` | the shared Rust audio mixer; wave 1 read it, and our `CombatMix` is already a smaller, stricter thing |
| `nicbarker/clay` | the C original of the Clay layout library, 18286 stars, Zlib — wave 1 already mined it; the **Rust binding** `clay-ui-rs/clay` (#8) is the new, stealable part |
