STEAL SCOUT r2 - AUDIO, UI, INPUT + CAPABILITY MATRIX (read 2026-10-12, READ-ONLY; nothing built, run or committed)
Field note: the brief's version list is stale. Found live today: Bevy v0.20.0 (2026-10-08), Fyrox v1.0.0 (2026-03-29), Godot 4.7.2, Wicked v0.72.120,
Flax 1.12 (2026-05-18), Stride 4.4.0 (2026-10-09), O3DE 2605.0, Defold 1.13.2, Blade (no tags), Dagor Engine tag dagor_2026_08_01 (BSD-3-Clause, new find).
Tags in steals: PRESENTATION = float/GPU/audio/UI, never read by the tick. IO = input layer before the tape. Donor files read through the GitHub contents API (path-level sha shown where the API gave it,
else repo HEAD at read time); 7 release-note pages (Bevy 0.19/0.20, Godot 4.6/4.7, Flax, Stride, Fyrox blog) read for Part B. Web search summaries are marked where a page was not opened.

=== 1. WE HAVE (file:line) ===
- Audio core: 64-voice mixer, ducking, fades (audio/src/lib.rs:86,320-352); lock-free feed (feed.rs:1-13); AudioEngine owns the callback (engine.rs:1-8);
  CueBank of 15 CC0-clip cues (cue.rs:462, impacts flesh/wood/metal/stone x3 sizes, bank.rs:58-69). CueBank/AudioEngine have NO user in crates/game: windowed.rs:19 imports only DeviceOutput, Bank, Mixer (grep over crates outside audio: no CueBank/FeedTx/AudioEngine user in game).
- The played mix is the old path: game/src/lib.rs:911-948 maps Hit, Blocked, Parried, Knockdown all to BankSound::Hit (3 placeholder sounds, bank.rs:31); Dodged and
  StaminaOut are silent (:940); windowed.rs:1060 plays that bank. So a parry and a hit sound the same in the window.
- Event coverage: FightCue has 6 variants (audio/crowd.rs:43-88). Combat also emits Parried/Riposte/Clash (combat/exchange.rs:335-361), SweepEvent (sweep.rs:244),
  Cut (cut.rs:271), GoreEvent (gore.rs:67). game/src has 0 uses of those four; audio has 0 cues for clash, riposte, sever, fire.
- Space: integer placement (spatial.rs:1-15). Occlusion is a Q16 GAIN only per material (occlude.rs:49-58, apply_wall :159-163); the voice mixer has no filter stage
  (0 lowpass/biquad hits in lib.rs). Reverb: 4 combs + 2 allpass (reverb.rs:437-447), one zone ctor tunnel() (:150), send is binary inside a 20 m circle (game/audio_mix.rs:30,96-99).
- HRTF: 8 octants x 16 taps (hrtf.rs:16,28), not in device.rs (0 hits); parked.md:59 "module proof, not a played mix"; SN4/SN5 are LATER (parked.md:63-64).
- Crowd and music: reaction cooldowns 20/45/20 ticks + bed duck (crowd.rs:19-36), Director intensity to 4 layers (director.rs:95-215, music.rs:22,419-480). cpal callback drops
  OutputCallbackInfo (device.rs:307): no latency number anywhere.
- UI is a data lane: Hud, FighterHud hp+stamina (ui/lib.rs:92), DuelOverlay v1 rows = name + hp only (:742-795), 0 'poise' hits in crates/ui while combat has PoiseMeter
  (combat/depth.rs:208). 4 colorblind palettes (:410-464), MIN_CONTRAST 3.0 (:41), ASCII-only 5x7 bitmap font (text.rs:12-16), linear menu state machine (menus.rs:345).
  Small bug: lib.rs:8 says integers only but state_hash writes f32 minimap bytes (:608-609); gallery hash only, not the tick.
- HUD is never drawn: windowed.rs:1089-1093 keeps HUD quads on the receipt, "not the draw list"; render/graph.rs:1506 declares a "hud" pass (32 draws, 4,000 tris) that nothing executes;
  crates/render/src has no UI pass. Plan: GP38 (gameplay.md:56,267-275,378); UI park lifts for the duel HUD only.
- Input: 7 actions (bindings.rs:21,99-114), 3 button bits ATTACK/BLOCK/DODGE (input/fighter.rs:24-30), pad is plain data (game/pad.rs:21), "live gamepad polling is a later lane" (bindings.rs:43).
  gilrs/kira/glyphon/taffy/fluent/accesskit: 0 hits in Cargo.toml. Haptics: 0 code. Plan: UIX10 pad-only rebind drill (parked.md:171), GP28 sim-side input buffer (gameplay.md:370).

=== 2. STEALS (ranked by Rome-duel look/feel per hour) ===
1. Draw the duel HUD: command list + quad pass + real font | grovesNL/glyphon@7f43114:Cargo.toml and src/text_render.rs, @49dc8f7 HEAD; nicbarker/clay@139802b:clay.h:759-784 (read 2026-10-12)
   | glyphon MIT OR Apache-2.0 OR Zlib; clay Zlib | PRESENTATION | lands in crates/render (new hud pass for graph.rs:1506) + crates/game/src/windowed.rs:1089 | M
   Idea: ui emits a flat command list (rect, border, text, scissor: the Clay set) from DuelOverlay; the hud pass draws rects as one instanced draw and text with glyphon prepare/render.
   glyphon 0.12.0 pins wgpu 30.0.0 and cosmic-text 0.19, the same wgpu line as ours (30.0.1). Game Accessibility Guidelines ask 28 px minimum at 1080p; our 5x7 atlas needs scale 4 (blocky).
   Number: HUD pixels on the arena capture 0 -> hp pips + stamina bar + poise flash + round wins visible; hud cost <= 0.3 ms at 1080p on the 3050, 3 draws; text contrast 3.0 gate -> 4.5:1 measured on pixels.
   Proof: `cargo test -p engine-render --lib hud_pass` prints draws, text_ms, nonbg_px; capture diff via tools/finish.mjs A4. Risk: CPU shaping on a weak CPU; cache by (string,size). Kill: > 0.5 ms CPU/frame on the UHD -> keep 5x7 quads (still drawn).
2. Hit-kind x surface impact table, layered, no-repeat pick | godotengine/godot@b4ebc1c:scene/resources/audio/audio_stream_randomizer.h + .cpp (pool weights, no-repeat mode, random pitch semitones and dB) | MIT | PRESENTATION
   | lands in audio/src/cue.rs (ARENA_CUES :462) + game/src/lib.rs:911-948 + audio/crowd.rs FightCue | M
   Idea: wire the window mix to CueBank/AudioEngine so the 15 cues play. Cue id = (kind: slash, thrust, shield bash, clash, riposte, sever, limb drop) x (surface: flesh, leather/mail, plate, wood shield, iron shield, stone),
   built from the 12 CC0 impact clips as 2 layers (steel transient + body), size 1-3 from damage. Kind and surface come from the attacker move id and victim loadout by a read-only lookup on the presentation side;
   FightEvent gets no new field. Clash, riposte, cut and gore get cues.
   Number: distinct impact sounds in a seeded 600-tick bout 1 -> >= 12, immediate repeats 0; event kinds with their own cue 4 (hit, block, parry, knockdown) -> 8 (adds clash, riposte, sever, gore); Dodged/StaminaOut stay quiet by design.
   Proof: `cargo test -p engine-game --lib hit_matrix` (distinct clips, x2 equal); operator listen for feel. Risk: 2 layers double voices (cap 64); cap at 2 layers. Kill: any audio-thread alloc (counting allocator).
3. Occlusion that muffles: lowpass per voice + distance air absorption + 2-band wall pass | BillyDM/Firewheel@34b35ca:crates/firewheel-core/src/dsp/distance_attenuation.rs (muffle cutoff 20..20,480 Hz, distance-muffle exponent)
   and ValveSoftware/steam-audio@045a222:core/src/core/direct_simulator.cpp (transmission = product over hit materials per band, square root when > 1 hit; volumetric occlusion by sphere samples) | Apache-2.0 both
   | PRESENTATION | lands in audio/src/occlude.rs (:34-40 one absorb value per material, :159 gain only) + a one-pole state in the Voice (lib.rs:252) + game/audio_mix.rs | S
   Idea: each Material gets (low pass, high pass) instead of one number; target gain stays, plus a cutoff that slews with the existing 120/300 ms attack/release (occlude.rs:18-20). Far stands lose highs by distance.
   Number: stone wall today -7.96 dB flat (occlude.rs:156-158) -> -8 dB and >= 15 dB less at 4 kHz+; zipper clicks 0.
   Proof: `cargo test -p engine-audio --lib muffle_band` (reuse band_rms reverb.rs:241); wall A/B blind 4/4 stays (parked.md:74). Risk: LPF zipper on fast voices. Kill: any audio-thread alloc.
4. Live gamepad (poll, hot-plug), four-motor rumble envelopes, device glyph prompts | Arvamer/gilrs@78581b7 (mirror of gitlab gilrs-project/gilrs):gilrs-core/src/platform/windows_wgi/ff.rs and windows_xinput/ff.rs;
   GaijinEntertainment/DagorEngine@2a79eaa:prog/gameLibs/forceFeedback/rumble.cpp + publicInclude/forceFeedback/forceFeedback.h; Kenney Input Prompts (CC0 art, page read) | gilrs Apache-2.0 OR MIT; Dagor BSD-3-Clause
   | IO + PRESENTATION | lands in game/src/pad.rs (:21 snapshot) + windowed.rs event loop + new game/src/rumble.rs | M
   Idea: poll gilrs once per frame into PadFrame; remember the last-used device so prompts show Xbox/PS/keyboard glyphs. Rumble = events {band lo|hi, strength, ms}, merged by max per band, one worker waking every 5 ms only while active,
   zero on focus loss, global scale 0..1 (Dagor). gilrs WGI backend passes 0.0 to both trigger motors; GamepadVibration has 4 fields (MS docs), so call Windows.Gaming.Input SetVibration for the impulse triggers.
   Table by cue: light hit, heavy hit, block, parry, clash, stagger, knockdown. Wicked ships vibration + LED, Godot 4.6 gamepad LED (reference only).
   Number: live pads 0 -> 1 (UIX10 keyboard-unplugged drill passes); haptic cues 0 -> 7; rumble off <= 10 ms after focus loss.
   Proof: `cargo test -p engine-game --lib rumble_merge` (max-merge, expiry, focus-loss zero, x2); real pad run is Maxim's hands (inbox). Risk: WGI needs window focus. Kill: build fails without the DLL -> feature `pad`, off by default.
5. Audio output latency probe and the hit-sync number | RustAudio/cpal@cf83483:src/host/wasapi/device.rs (shared-mode Initialize, no IAudioClient3; device.rs:975-992) and @79275c2:src/host/wasapi/stream.rs (latency + fill per callback, :221,:316-320); tesselode/kira@d0980a8:crates/kira/src/start_time.rs (Immediate/Delayed/ClockTime) | Apache-2.0 | PRESENTATION
   | lands in audio/src/device.rs:307 + SN10 (parked.md:151) | S
   Idea: read OutputCallbackInfo, keep a running p50/p95 of (playback instant - callback instant + buffer fill). If p95 > budget, schedule the hit cue earlier by a sample offset (kira-style start time) or hold the flash (LF10).
   Number: latency unknown -> printed `audio_latency_ms p50= p95=`; target p95 <= 40 ms hit tick to speaker.
   Proof: `cargo test -p engine-audio --features device --lib latency_probe` (skips loudly with no device). Risk: virtual devices report fake latency. Kill: probe unreadable on this PC -> drop the offset, keep the print.
6. Tick-counted input conditions (Tap, Hold, HoldAndRelease, Chord, Cooldown) + action-set stack | simgine/bevy_enhanced_input@ca63988:src/condition/tap.rs and hold_and_release.rs; Dagor@2a79eaa:prog/gameLibs/daInput/actionData.h (action sets, exclusive_with, long press 300 ms, double click 220 ms, binding columns)
   | Apache-2.0; BSD-3-Clause | IO (new button bits are v2; proof scene keeps v1) | lands in input/src/bindings.rs (:21) + fighter.rs:24-30 | M
   Idea: a condition is a small state machine over (actuated, ticks held) -> None/Ongoing/Fired. Run in integer ticks before FighterInput so the tape holds only the result. Gestures: tap = light, hold = heavy, release before the hold = feint,
   block + attack chord = shield bash, block tap = parry try. Action sets (Duel, Menu, Pause) stack so menu keys never reach the tape. Complements GP28 (buffer at intake).
   Number: attack gestures 1 -> 4; menu inputs reaching the tape 0.
   Proof: `cargo test -p engine-input --lib conditions` (same raw press script gives same tape bytes x2, +-1 tick edges). Risk: window numbers clash with GP28/GP29; keep numbers in rows. Kill: any v1 tape byte changes.
7. Crowd roar as a wave across the ring + synced excitement stems | IDEA-ONLY for the wave (applause ensemble model, Lee and Reiss AES, paper not opened; no code found); Godot@b4ebc1c:modules/interactive_music/audio_stream_synchronized.h (32 synced stems, per-stem dB);
   fundsp@3f867aa:src/granular.rs (offline grain tool, marked WIP) | MIT; Apache-2.0 | PRESENTATION | lands in audio/crowd.rs:94 + director.rs Layers | M
   Idea: on Ooh/Cheer/Gasp start 8 ring emitters, each delayed by distance / 343 m/s (a 40 m span = 0..117 ms) plus 0..60 ms seeded jitter, panned by spatial.rs; murmur/excited/roar run synced and crossfade on Director intensity (thresholds .35/.70).
   Number: onset spread across the crowd 0 -> >= 80 ms; synced stems 1 -> 3. Proof: `cargo test -p engine-audio --lib crowd_wave` (onsets x2 equal). Risk: voice budget; group limits (cue.rs:139). Kill: > 12 simultaneous crowd voices.
8. Reverb zones: open bowl with stand slapback, tunnel, gate arch, weighted send | BillyDM/Firewheel@34b35ca:crates/firewheel-nodes/src/freeverb/mod.rs (room_size, damping, width) and convolution.rs (partitioned FFT, IR up to 4 s);
   godotengine/godot@324512e:servers/audio/effects/audio_effect_reverb.h (predelay, predelay feedback, hpf, room size, damping, spread, dry, wet) | Apache-2.0; MIT | PRESENTATION | lands in audio/reverb.rs:141 + game/audio_mix.rs:96 | M
   Idea: 3 presets, crossfaded send over 2 m instead of in/out at 20 m; add the hpf and spread knobs our ReverbParams lacks (reverb.rs:82-90); slap echo at 2d/343 s from the tiers; tail from Sabine using the Material table.
   Comfort tier may load an IR through a convolution node (OpenAIR CC library has heritage sites; no Colosseum IR found, IDEA-ONLY).
   Number: zones 1 -> 3; send step per tick <= 0.02; cost <= 0.4 ms per 480 frames (reverb.rs:20). Proof: `cargo test -p engine-audio --lib zone_blend` + operator listen. Kill: blind A/B bowl vs flat below 4/4.
9. HRIR sphere table from SADIE II D1, barycentric blend, short crossfade | ValveSoftware/steam-audio@480dd64:core/data/hrtf/sadie_d1.sofa (36.6 MB tree entry; data only) + mrDIMAS/hrtf@e984d42:src/lib.rs (barycentric :169; README admits clicks on fast sources, fix = crossfade)
   | SADIE II Apache-2.0 per York page (search summary, page not opened: verify); code MIT | PRESENTATION | lands in audio/hrtf.rs:16,28 | M
   Number: 8 directions -> 72 az x 3 el, 32-tap min-phase + ITD per ear, ~0.2 MB baked; orbit fixture 7/8 (parked.md:74) -> 8/8; cost 32 MAC/frame (hrtf.rs:11) -> ~128; audio-thread allocs 0 stays.
   Proof: `cargo test -p engine-audio --lib hrtf_sphere`; offline bake x2 bytes. Kill: dataset terms not confirmable. (Steam Audio itself stays out: parked.md:74.)
10. Layout for menus, settings, lobby (the 16-slot list): Clay-style engine in Rust | nicbarker/clay@139802b:clay.h (4.8k LOC, zero deps, ~3.5 MB for 8192 elements per README@66bbb81) and TheRedDeveloper/ply-engine@d4f682f:src/render_commands.rs + layout.rs (Rust port of the Clay model)
   | Zlib; 0BSD | PRESENTATION | lands in ui (new layout.rs) | M. Alternative: DioxusLabs/taffy@7d33901 (MIT, 0.14.0, flex+grid, no text measure). Decide after XX1: the HUD needs anchors only (about 60 lines); menus are the first real layout user.
   Number: layouts assert no overlap at 16:9, 21:9, 4:3 (0 checks today). Proof: `cargo test -p engine-ui --lib layout_aspects`. Kill: if the port passes 800 lines, take the Taffy dependency instead.
11. Unicode text + Fluent strings + pseudo-locale | projectfluent/fluent-rs@0f87afb (Apache-2.0 per repo field); ply-engine@52be132:Cargo.toml feature `locales` (fluent-bundle); Godot 4.6 CSV ?context/?plural (release page read); cosmic-text 0.19 via glyphon
   | PRESENTATION | lands in ui/text.rs:12-16 (ASCII-only) + PLF7 (parked.md:162) | M. Number: glyph coverage ASCII 95 -> Latin-1 + Greek; pseudo-locale test: 0 clipped labels. Proof: `cargo test -p engine-ui --lib pseudo_locale`. Needed before a store page, not before the duel plays.
12. Directional focus for 2-D menus | bevyengine/bevy@550b826:crates/bevy_input_focus/src/directional_navigation.rs (AutoNavigationConfig :94: min_alignment_factor, max_search_distance, prefer_aligned) | Apache-2.0 OR MIT | PRESENTATION | lands in ui/menus.rs:345 + UIX10 | S
   Honest value: low today. Menu already takes Up/Down/Confirm/Back with wrap (menus.rs:101-114), so a pad reaches every row of the 3 screens. It matters once settings or lobby become grids. Proof: `cargo test -p engine-ui --lib focus_nav`.
13. Accessibility roles and live regions (AccessKit) | ply-engine@1eac0e6:src/accessibility.rs (roles incl. ProgressBar; live-region modes); bevyengine/bevy@c9c21c2:crates/bevy_a11y/src/lib.rs (AccessibilityNode over accesskit); godot drivers/accesskit exists (listing) | 0BSD; Apache-2.0 OR MIT; MIT
   | PRESENTATION | lands in ui/lib.rs:3 (accessibility-tree gate input) + UIX4 | M. Number: nodes announced 0 -> hp/stamina bars as ProgressBar, "round won" as a live region. Proof: `cargo test -p engine-ui --lib a11y_tree`. Low value for the duel.
14. Per-pair music transitions (kill sting, round end) | godotengine/godot@b4ebc1c:modules/interactive_music/audio_stream_interactive.h (from: now/next beat/next bar/end; to: same position/start; fade in beats; filler clip; 63 clips) | MIT | PRESENTATION | lands in audio/music.rs:550 (Player) | S
   Number: transition rules 0 -> 6 pairs in data; none lands off-bar. Proof: `cargo test -p engine-audio --lib music_transitions` x2 bytes. Value low: music.rs already holds 4 tension stems.

=== 3. BOARD ROWS (top 6; prefix XX, lead numbers them) ===
- [ ] XX1 ui: duel HUD drawn through a quad+text pass with stamina and poise — graph.rs:1506 declares a "hud" pass nothing draws, windowed.rs:1089-1093 keeps quads on the receipt, DuelOverlay has hp only (ui/lib.rs:742-795). Diff: good. Tier A. Size M.
  Owns crates/render/src/hud_pass.rs crates/ui/src/lib.rs crates/ui/src/text.rs crates/game/src/windowed.rs. Accept: `cargo test -p engine-render --lib hud_pass` prints draws<=3 text_ms<=0.3 at 1080p, stamina+poise+pip pixels>0, text contrast>=4.5.
  F2P: arena capture has 0 HUD pixels. P2P: golden unchanged. needs: none (GP38 consumes it). Plan owner: gameplay.md GP38 (UI park lifted for the duel HUD) with parked.md UIX.
- [ ] XX2 audio: hit-kind x surface cue table, window mix on CueBank — game/lib.rs:911-948 plays one BankSound::Hit for four events, AudioEngine has no game user. Diff: good. Tier A. Size M.
  Owns crates/audio/src/cue.rs crates/audio/src/crowd.rs crates/game/src/lib.rs crates/game/src/windowed.rs. Accept: `cargo test -p engine-game --lib hit_matrix` prints distinct_clips>=12 repeat_run=0 kinds_with_cue=8/8.
  F2P: distinct_clips=1 today. P2P: golden unchanged, audio-thread allocs 0. needs: none. Plan owner: parked.md audio (SN6 follow-up) + gameplay.md cues.rows.
- [ ] XX3 audio: occlusion lowpass and air absorption — occlude.rs:159 returns gain only, the mixer has no filter. Diff: regular. Tier B. Size S.
  Owns crates/audio/src/occlude.rs crates/audio/src/lib.rs crates/game/src/audio_mix.rs. Accept: `cargo test -p engine-audio --lib muffle_band` prints high-band drop>=15 dB and low-band drop<=8 dB behind stone, allocs 0.
  F2P: high-band drop 0 dB today. P2P: golden unchanged, wall A/B 4/4. needs: none. Plan owner: parked.md audio (SN2/SN6).
- [ ] XX4 input: live gamepad, hot-plug, four-motor rumble, last-device prompts — bindings.rs:43 says live polling is a later lane; pad.rs:21 is data; 0 haptic code. Diff: good. Tier A. Size M.
  Owns crates/game/src/pad.rs crates/game/src/rumble.rs crates/game/src/windowed.rs crates/input/src/bindings.rs. Accept: `cargo test -p engine-game --lib rumble_merge` prints cues=7 off_after_focus_loss_ms<=10 and `--features pad` live-poll test lists >=1 fake pad.
  F2P: rumble_merge does not exist. P2P: golden unchanged. needs: none. Plan owner: parked.md UIX10 (pad-only rebind drill) + gameplay.md input.
- [ ] XX5 audio: output-latency probe and hit-sync number — device.rs:307 drops OutputCallbackInfo; SN10 is LATER. Diff: regular. Tier B. Size S.
  Owns crates/audio/src/device.rs crates/audio/src/engine.rs. Accept: `cargo test -p engine-audio --features device --lib latency_probe` prints audio_latency_ms p50 and p95 (skips loudly without a device); p95<=40 or the early-queue offset is set.
  F2P: no latency line exists. P2P: golden unchanged. needs: none. Plan owner: parked.md audio (SN10).
- [ ] XX6 input: tick-counted conditions (tap, hold, hold-and-release, chord) and an action-set stack — 7 actions, 3 button bits (bindings.rs:21, fighter.rs:24-30). Diff: good. Tier A. Size M.
  Owns crates/input/src/bindings.rs crates/input/src/conditions.rs crates/input/src/fighter.rs. Accept: `cargo test -p engine-input --lib conditions` prints gestures=4 menu_leaks=0 tape_bytes_x2=equal.
  F2P: only press exists. P2P: golden unchanged (new bits are v2, proof scene keeps v1). needs: GP28. Plan owner: gameplay.md (GP28 neighbor, input rows).

=== 4. PLAN TEXT (for parked.md section 3 and 4, and gameplay.md HUD section) ===
- audio: The played mix goes only through CueBank/AudioEngine; BankSound stays as a test fixture. Every combat event (Fight, Exchange, Sweep, Cut, Gore) maps to a cue id by a presentation-side (kind x surface) lookup; FightEvent gets no new field.
- audio: Occlusion returns a gain and a cutoff (two bands per Material); distance adds air absorption; the reverb send is a weighted blend of three zone presets, not an in/out circle.
- audio: Hit sync is measured, not assumed: device.rs reads OutputCallbackInfo and prints p50/p95; budget p95 <= 40 ms from hit tick to speaker, else the cue is scheduled early by a sample offset.
- ui: The HUD is a flat command list (rect, border, text, scissor) emitted from DuelOverlay snapshots; render's hud pass draws it. Text >= 28 px at 1080p, contrast >= 4.5:1 for text and 3:1 for bars, never colour alone (shape or blink rate too).
  Poise reads as segments on a ring around the fighter, stamina as a thin bar under hp, hp as pips with a chip-lag tail (Hud::chip, ui/lib.rs:576). Potato tier keeps the 5x7 atlas quads.
- ui: Keep the model "retained data + immediate command list" (the Clay shape), not a widget tree; egui stays tools-only; menus choose Taffy or a Clay port after XX1 has measured the HUD cost.
- audio: When SN4/SN5 un-park, the HRIR table is baked offline from SADIE II D1 (72 az x 3 el, 32 taps); Steam Audio is never vendored; occlusion math is studied, not linked.
- input: Raw device -> conditions in integer ticks -> FighterInput -> tape. Action sets stack (Duel, Menu, Pause). Last-used device picks prompt glyphs. A rumble table maps cue to (lo, hi, trigger L, trigger R, ms).
- hard problem (section 4): three clocks meet at the hit: the 60 Hz tick, the audio callback block, the display frame. Measure all three on one run; never move the tick to fix a feel gap.
- hard problem (section 4): text shaping on a weak CPU: cache laid-out strings by (string, size); numbers change, labels do not.

=== 5. NOT WORTH IT ===
- Vendor Steam Audio C++ (reflection_simulator.cpp alone is 39 KB of ray-traced propagation; native toolchain): the arena is static, so 3 baked presets plus the occlusion math above are enough (parked.md:74 CUT stands).
- kira, Firewheel or oddio as our mixer: slim law forbids a second mixer (parked.md section 6); oddio last pushed 2023-10-15; kira's tween (112 lines) and start time (55 lines) are copy-the-idea only.
- egui or Bevy UI or Fyrox UI for the in-game HUD: egui is tools only; the other two are retained widget trees (Fyrox UI has about 70 widget files); a dozen quads plus text do not need one.
- Taffy as a HUD dependency: it has no text measure and the HUD needs anchors only; decide Taffy vs a Clay port only when menus need real layout (steal 10).
- NVIDIA Reflex / Anti-Lag markers (Dagor prog/engine/lowLatency/lowLatency.cpp sleep(frame_id)): vendor SDK binaries and wgpu hides the device; use frame latency 1 plus the present timing we have.
- Wwise or FMOD middleware (Dagor's README lists FMOD as optional): licensed SDKs, and "licensed soundtrack bundles" are already CUT in parked.md.

=== PART B. CAPABILITY MATRIX: engine2040 vs open engines (2026-10-12) ===
Legend: B Bevy 0.20 (Apache-2.0 OR MIT), F Fyrox 1.0 (MIT), G Godot 4.7 (MIT), W Wicked 0.72 (MIT), X Flax 1.12 (custom: ideas only), S Stride 4.4 (MIT), O O3DE 2605 (Apache-2.0 OR MIT),
D Defold 1.13 (custom: ideas only), L Blade (MIT, raw Vulkan: skip for wgpu), A Dagor (BSD-3-Clause). Cells name only what I read (release notes, README, file lists); "not read" means no claim.
Newer engines checked: Dagor (real, 2,974 stars, pushed 2026-10-11), Pulsar-Native (MIT, early, 396 stars), renzora (Bevy 0.19.1 editor, alpha), ply-engine (0BSD UI, macroquad-based), Infernux (earlier scout). Only Dagor adds new donors.
row | E2040 has | best donor (why)
rendering | forward PBR, frame graph as data (render/graph.rs:1459), SMAA+dyn-res (aa_scale.rs), decals, fog, GPU particles; live passes still wiring (potato.md:11) | W: features.txt lists impostors, virtual textures, DDGI, surfel GI, VRS, splats. B 0.19/0.20: GPU light clustering, mesh shaders, render graph replaced by schedules. A: daGI2, daSWRT, daSkies2 dirs
lighting | clustered lights, cascades, CPU-only shadow filter (steals.md:13), baked GI (baked_gi.rs), dyn GI (gi_dyn.rs), GTAO, IBL | B 0.19: contact shadows, rect area lights, parallax cubemaps, screen-space reflections: WGSL we can port. W: DDGI + RT shadows. G 4.7: AreaLight3D
animation | motion match, inertialize, FBIK, hit-react, layers, state machine, LOD (anim/src list) | G 4.6: IK set (TwoBone, Spline, FABRIK, CCD, Jacobian). O: Gems/MotionMatching + EMotionFX. F: ABSM editor (editor/src/plugins/absm/*). Ours also has net_pose.rs and lod.rs (anim/src).
physics | own integer solver + rapier + ragdoll, powered, balance, cloth, vehicle (physics/src) | G 4.6 made Jolt the 3D default; JoltPhysics (MIT, pushed 2026-10-11) is the engine Godot chose; not read for ragdoll. F tracks Rapier 0.32.
audio | Part A above | G: buses + interactive/synchronized/randomizer streams. F: fyrox-sound bus.rs, effects/reverb.rs, renderer/hrtf.rs. Firewheel: Rust node graph (Apache-2.0).
ui | data lane + 5x7 font, HUD undrawn (Part A section 1) | B 0.19: parley text, EditableText, Feathers widgets, FixedNode. F: fyrox-ui widget set. Clay model for a HUD command list.
input | 7 actions, plain-data pad (bindings.rs:21, pad.rs:21) | B ecosystem: bevy_enhanced_input conditions. A: daInput action sets. G 4.7: gyro aiming, VirtualJoystick; G 4.6: gamepad LED. W: vibration + LED in features.txt.
networking | rollback tape, QUIC, lobby, dedicated, Steam relay, lag comp, interest, anticheat (net/src 32 files; rollback.rs:1) | We lead for a deterministic duel. O: Gems/Multiplayer + MultiplayerCompression (listed, not read). A: prog/engine/daNet (enet, delta) read as listing only.
scripting/modding | Rhai integers-only sandbox, hash in the tape (rules/lib.rs:3); PLF4 workshop LATER | G 4.6: JSON-based GDExtension interface. F: plugins + scripts with hot reload. We are unique on replay-exact mods; no donor needed for the sandbox.
editor | egui 0.36 + egui_dock 0.21 + transform-gizmo (Cargo.toml:186-191), inspector, outline, play-in, tuning (editor/src, 31 files); no animation or timeline panel | G 4.6/4.7: modern theme, ObjectDB snapshots. F: absm + animation editors. A: gameLibs/animTreeEditor, daEditorE. B: Feathers/BSN, .bsn not shipped.
asset pipeline | 7-gate import, cooker, bundle cook, mesh cook, meshlets, LOD swap, watcher, recook (assets/src); BC1/BC3 encode only (bc.rs:1) | S 4.4: asset compiler changes, shader compiler overhaul, `stride` CLI. B 0.20: CompressedImageSaver (BCn/ASTC + mips, crate ctt). F 1.0: export-cli for CI. A: assets_import.
hot reload | poll watcher over mtimes (assets/watcher.rs:1-8); shader hot reload refused off a frame boundary (render/live.rs:809) | F 1.0: prefab and plugin hot reload fixes. D and G also (not read). B shipped none new in 0.19/0.20.
profiling | puffin scopes + GPU timestamps + counting allocator (perf/profiler.rs:1-12, alloc.rs); nothing visible in game | X 1.11: Tracy GPU + memory profiler. B 0.19: DiagnosticsOverlay. A: gameLibs/lagCatcher, frameTimeMetrics, memoryProfiler. G 4.6: Tracy/Perfetto.
platform targets | Windows + CI by design (FOCUS.md:91 CUT: mobile, WASM, console) | S 4.4: all platforms stable, Proton editor. Not a duel gap; Steam Deck only needs Proton checks.
save/load | versioned envelope, CRC, 16 slots, atomic write, prev-generation fallback (save/lib.rs:9-21, fallback.rs); migrate has only v1 (:132) | B 0.19: SettingsPlugin (deferred/sync save), handle round-trip. O: Gems/SaveData (listed). Enough for a ladder.
localization | none (0 hits; PLF7 LATER parked.md:162) | G 4.6: CSV plural/context + template generation. ply-engine: fluent-bundle feature. fluent-rs (Apache-2.0 OR MIT).
accessibility | 4 palettes + contrast check (ui/lib.rs:396,410-464); UIX4 parked | G: drivers/accesskit server, landmark nav (4.7). B 0.19: AccessibleLabel over accesskit. ply-engine roles + live regions.
AI door | MCP stdio server over the editor command service (endpoint/mcp.rs:1), packs, receipts | B: bevy_remote (world.query/get/insert/mutate, +watch, schemas, inspection) is the typed discovery model to compare. Ours leads on replay + audit.

=== PART B. THE 10 CAPABILITIES A ROME DUEL WILL MISS MOST (donor to study) ===
1. Anything drawn on screen: HUD, menus, perf overlay (graph.rs:1506 pass has no executor) | Clay command list + glyphon; Bevy 0.19 DiagnosticsOverlay.
2. Fight-keyed impact, clash, sever and fire audio (4 events share one sound) | Godot AudioStreamRandomizer; Firewheel nodes.
3. A rendered crowd: render/src has 0 crowd hits; 20k-50k spectators need impostors/VAT (GS20 unbuilt) | Wicked shaders/impostor*.hlsl (MIT), Bevy 0.19 GPU batch unpacking.
4. Attack gestures (light, heavy, feint, charge) and action sets | bevy_enhanced_input conditions; Dagor daInput.
5. Live gamepad and haptics (0 code) | gilrs, Dagor forceFeedback/rumble.cpp, Wicked vibration + LED.
6. Spatial audio fidelity: muffle, air absorption, reverb zones, HRIR table | Steam Audio direct_simulator.cpp (math only), Firewheel, SADIE II.
7. A frame-data / animation timeline editor with hit windows (editor has no anim panel) | Fyrox editor/src/plugins/absm, Dagor animTreeEditor.
8. Texture cooking for a 6 GB card and the potato tier (BC1/BC3 only, no KTX2/BC7/ASTC) | Bevy 0.20 CompressedImageSaver, Stride 4.4 compiler CLI.
9. Hitch catching and an in-game perf overlay (puffin exists, nothing shown) | Dagor lagCatcher + frameTimeMetrics, Flax 1.11 Tracy GPU/memory.
10. Text beyond ASCII, localization and a screen-reader tree | Bevy 0.19 parley + AccessibleLabel, Godot CSV plurals + AccessKit, ply-engine roles + Fluent.

=== VERIFY BEFORE PORTING (what I did not prove) ===
- Every number after "->" is a target; nothing was built or measured here. Proof command names are proposals, not existing tests.
- SADIE II terms (Apache-2.0) and the 28 px / 4.5:1 / 3:1 figures came from search summaries or one opened page (28 px page opened; contrast page not opened). Open the source before shipping data or citing a number.
- Bevy and Godot release facts came through a page summarizer; O3DE Multiplayer/MotionMatching/SaveData, Wicked impostor shaders, Dagor daNet and Fyrox editor plugins are file or directory listings only (not read).
- WGI trigger motors work only on Xbox One-class pads; gilrs mirror on GitHub was last pushed 2025-11-11 (upstream is GitLab): check upstream before pinning.

DONE 14 steals, 50 donor files read.
