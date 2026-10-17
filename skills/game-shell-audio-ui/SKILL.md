---
name: game-shell-audio-ui
description: "Use when you add or change the title, pause, result or rematch shell, a HUD or text overlay, menu layout, or how sound plays (distance gain, cues, voice limits, ducking, music) in crates game, ui or audio: one Flow driver, pause halts the accumulator, 0..1 coords."
---

# Game shell, HUD and audio

Three parts around one match: the window shell (Flow), the HUD overlay, the sound mix. Presentation only. None of it enters the hash. Golden stays `d57b5c8ae3096614`.

Real-time audio callback rules: skill `rust-safety` and skill `tick-and-runtime`. Capture and pixel proof: skill `test-and-proof`.

## 1. Flow shell (`crates/game/src/flow.rs`)

`Flow` has four states: `Title`, `Match`, `Paused`, `Result`. `FlowEvent` is the single input: `Start`, `TogglePause`, `MatchOver`, `Rematch`, `ToTitle`. `Flow::on` is pure. Unknown pairs hold the state. `Flow::ticks_run` is true only in `Match`. The `arena` binary drives it (`crates/game/src/bin/arena.rs`: `ticks_owed` plus Escape).

PASS needs a test plus the unchanged golden x2, never a promise.

1. **One driver.** One `Flow` owns Title, Match, Paused, Result and drives menus. Flag: state split across the windowed path and the bin with no single owner, or two places that advance the match.
2. **Objectives as graph.** Objectives are a sequence tree or graph, not one per chunk. Flag: level id equals objective id, so a chunk rename rewrites progress and saves.
3. **Success and failure.** Each objective states its success move and its failure move. Flag: a win with no next step, or a loss that strands the player with no rematch.
4. **Loose chunks.** Chunks and objectives change apart, mapped by id. Flag: a chunk edit forces a flow edit, or one table holds mesh refs and win rules.
5. **Pause stops ticks.** Pause halts the accumulator: 60 paused frames add zero ticks. Flag: pause hides the window while ticks keep stepping, so the replay hash drifts. Fixed-step rules: skill `tick-and-runtime`.
6. **Result rematches.** `Result` leads to the next match with prior state cleared. Flag: old scores or timers leak into the next bout, or no fresh-seed path exists.
7. **No settings creep.** The shell adds flow only. Settings are ui rows (`crates/ui/src/settings.rs`, `menus.rs`). Flag: a settings screen smuggled into the flow diff.
8. **Hash proof.** A test covers the cycle (`flow.rs::tests::flow_cycle_title_match_result_match`) and golden x2 is unchanged. Flag: manual clicks with no test, or a pause-in-match hash that is never compared.

Do: pause the accumulator at the frame edge and keep the tick count still. Don't: keep stepping hidden ticks and call it a pause.

## 2. HUD and text overlay

Files: `crates/render/src/overlay.rs` (CPU quad ring, `OVERLAY_MAX_QUADS` = 128, `push_quad`, `push_hp_bar`, `push_text`, `OVERLAY_WGSL`), `crates/render/src/text.rs` (atlas glyph UVs), `crates/ui/src/text.rs` (layout over the bitmap atlas, ASCII only), `crates/ui/src/play_hud.rs` (`PlayHud`, `Crosshair`).

PASS needs a pixel test plus `cargo test -p engine-render --lib` green.

1. **Draw last, no depth.** The overlay pass runs after the blit with depth test off, so quads sit over the scene. Flag: HUD quads in the 3D pass, or depth-tested so fighters hide them. (render)
2. **One quad pipeline.** One textured-quad pipeline with alpha blend and a single draw for all cells. Flag: a pipeline per widget, or a draw per quad that grows with HUD size. (render)
3. **Layout feeds quads.** A dynamic quad ring fed by ui layout cells owns positions. Game sends only hp and state. Flag: scene draws building the HUD, or game pushing pixels per frame. (game)
4. **Normalized y, aspect x.** Overlay coords are 0..1 on y, with x scaled by aspect (1.33 at 4:3, 1.78 at 16:9). `overlay.rs` notes "aspect stays in MVP". Flag: pixel coords baked for one resolution, so layout breaks on resize. (ui)
5. **No square stretch.** No layout is 0..1 on both axes at once. Flag: a bar square in data but wide on 16:9. (ui)
6. **Anchored edges.** Positive offsets anchor top-left, negative anchor bottom-right, with center or element anchors where needed. Flag: right-edge bars that drift when aspect changes. (ui)
7. **Atlas plus metrics.** One font atlas carries glyph boxes, kerning and baseline in a metrics file. Flag: per-glyph uploads, or a missing advance so text overlaps. (render)
8. **Text fits the game.** Text covers the shipped charsets and read directions, with only the features the game uses. Flag: a full animation suite for static hp numbers. (ui)
9. **Pixel proof.** The arena capture shows hp bars where scene pixels sat, and on-vs-off rects differ. Flag: a receipt line with no capture rect that moved. (game)

Do: draw one blended quad ring after the blit in 0..1 coords and prove hp rects move pixels. Don't: depth-test HUD in the 3D pass, or bake pixels for one window size. Eyes rule for the capture: skill `design-and-playtest`.

## 3. Audio cues and mix (`crates/audio`)

These rules cover the mix. Presentation only, never hashed.

1. **Gain falls as 1/r** between falloff min and falloff max, then is zero. Dialogue may bend the curve. Code: `crates/audio/src/spatial.rs` (`falloff_gain`, `attenuation`, `NEAR_CM` 100, `FAR_CM` 3000, `SpatialMixer::set_listener_3d`). Positions are integer cm. The math is integer-friendly: no `sin`, `cos`, `exp`, `powf`.
2. **Play cues, not clips.** A cue is clips plus 3D or 2D, falloff, group and loop. Give each cue 3-4 variants, picked by a seeded picker so a script replays the same bytes. Code: `crates/audio/src/cue.rs`, `combat_cues.rs`, `mix.rs::CombatMix`.
3. **Steal by priority, not age** when voices run out. Limit instances per group. A play beyond max falloff is culled with no voice. Code: `mix.rs`.
4. **Duck the music** under dialogue and stingers: the cue flag `ducks_music` (`cue.rs`), `Group::Stinger`.
5. **Music follows tension.** `crates/audio/src/playlist.rs`: `MusicBank` loops per tension level (calm, build, fight, climax). A change waits for the next bar line and crossfades over one bar. Stingers are ordinary cues that duck the bed. With no recorded music the bank is baked from the seeded `music` module.

Repo facts (verified 2026-10-17, re-check before you rely on them):
- The `arena` binary stages the real cue bank once at startup: `crates/game/src/windowed.rs::stage_arena_bank` loads `content/audio` and fails loudly when clips are missing. It never falls back to silent synth.
- `Bank::placeholder` (`crates/audio`) is still used by the headless smokes and by the offline mirror mixer in `crates/game/src/bin/arena.rs`. It is not what the window plays.
- `crates/audio/src/voice.rs::synthesize_live` always refuses (`VoiceError::LiveSynthRefused`). Voice output is an offline bake. No runtime TTS.
- Before you add a synth sound, pick a cue group from the clips in `content/audio` (impacts, footsteps, ambience).
- The cue feed to the device is a real-time path. Do not block it (skill `rust-safety`).

More on cues and falloff: `node tools/books.mjs search "sound cue falloff voice priority"` (skill `research`).

Sources: Gregory, Game Engine Architecture 2nd ed. ch. 10.4.4 (HUD), ch. 13.4.3.1-13.4.3.3, 13.5.7.2, 13.5.8.2, 13.5.8.4, 13.6.3 (audio), ch. 14.1.3 (flow).
