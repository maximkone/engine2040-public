STEAL CARD: Asterber/shiloh (BLOCK 10, position 20, GitHub topic game-engine?l=rust&o=desc&s=updated). Read live 2026-10-18, read-only. No bytes vendored.
Donor head read: 2131c92099769ad05058bfb041319c02fc1bdc14 (branch master).

What it is: a from-scratch Rust engine that runs *The X-Files Game* (1998, HyperBole VirtualCinema) from the player's own copy. Two fronts: a native SDL2 window and a WebAssembly build served by a local 127.0.0.1 server. It is an adventure-storybook player, not a 3D or simulation engine: a 640x480 picture pipeline (QuickTime video decoders written in-house: RPZA, Cinepak, IMA ADPCM), a script/opcode VM for the game's events, save-file compatibility, hot-spot overlay, and a headless frame-dump mode used to verify scenes without a window. 10 stars.

Licence: MIT OR Apache-2.0 (read live 2026-10-18: LICENSE-MIT = "MIT License", LICENSE-APACHE = "Apache License, Version 2.0"; GitHub-detected SPDX: Apache-2.0). Third-party notices in port/LICENSE-THIRD-PARTY (e.g. neodb.rs basis is MIT). Game data is never shipped.

Verdict: NOT WORTH STEALING for the engine (2D-only video/storybook port: no 3D, no fixed-tick sim, no netcode). Three small, generic harness ideas below do transfer; nothing in its core is engine2040's shape.

Engine2040 anchors it lands on:
- crates/game/src/windowed.rs:639 (TickAccumulator) and :94 (WIN_MAX_STEPS_PER_FRAME) — window loop.
- the arena capture tooling (door-and-headless-runbook, window run section): headless frames.
- crates/game window/UI input mapping (pad.rs / window handlers).

=== TRANSFERABLE BITS (small; not engine steals) ===

1. Headless scene run that writes PPM frames on a game-time clock | Asterber/shiloh@2131c92:port/src/bin/xf.rs:12-18 (`xf --frames N --dump DIR [--trace T.jsonl] [--every MS]`, headless: no window) and :99,:207 (frames/dump options) | MIT OR Apache-2.0 | SIM TOOLING | lands the arena capture path | S
   Idea: the same binary runs headed or headless; `--frames N --dump DIR --every MS` drives the game without a window and writes one picture per MS of *game* time, so a scene is reproducible from a command line and can be diffed.
   Maps: engine2040 already captures arena frames. Steal the "capture cadence is game time, not wall time" rule so a capture set is byte-reproducible and a fixed step count always yields the same frames; useful for the staged fight and bug bundles.
   Kill rule: kill if the cadence is wall-clock (then frames vary per machine) or if the dump allocates on the tick path.

2. Letterbox layout + window-point -> picture-pixel mapping with high-DPI handling, integer-safe | Asterber/shiloh@2131c92:port/src/gpu/mod.rs:16-26 (letterbox: largest source-aspect rect centred, bars around) and :28-55 (window_to_picture: points -> drawable pixels -> picture pixels, floor, bars return None) | MIT OR Apache-2.0 | PRESENTATION (window/UI) | lands crates/game window/UI input mapping | S
   Idea: one pure function computes the presented rect; the inverse maps a mouse point through drawable scaling back to a source pixel and returns None on the bars, so clicks never land on a guessed coordinate. All u64/f64 math, no platform calls.
   Maps: engine2040 maps window input to the fight view. Steal this as a small pure, unit-tested helper (letterbox + inverse) so resize and DPI changes cannot desync the pick from the picture.
   Kill rule: kill if the app is always 1:1 (no letterbox) or if the inverse disagrees by a pixel at drawable != window size (the DPI case is the whole point).

3. Audio clock as the master, picture delayed by (audio latency - display latency) | Asterber/shiloh@2131c92:port/src/front/avsync.rs:1-17 (the session clock picks the video frame and the sound position; the picture is delayed by the outputs' latency difference so it follows what is heard) and :21-41 (WallClock carries the sub-millisecond remainder instead of dropping it; a stall clamp drops the clamped part) | MIT OR Apache-2.0 | PRESENTATION (audio/video pacing) | lands crates/game audio path (window_audio_space.rs) + windowed.rs accumulator | S
   Idea: measure/assume two output latencies and delay the picture by their difference so it tracks the audio clock, not the mixer's write head; and never drop the sub-millisecond remainder of the frame budget (dropping it made their clock run ~4% slow).
   Maps: engine2040 paces a fixed tick from wall time. The carry rule (keep the remainder, drop the clamped excess) is the portable part; the audio-master rule matters only if a movie/voice-over is ever frame-locked to sound.
   Kill rule: kill the carry change if TickAccumulator already keeps the fractional remainder (windowed.rs:639-661 takes f64 ms, so it may already); kill the audio-master rule if nothing on screen is frame-locked to a sound.

=== NOT WORTH STEALING ===
- The video/audio decoders (RPZA, Cinepak, IMA ADPCM), the HyperBole script VM, the original save-file format and the whole 640x480 presentation core: 2D storybook playback with no fixed-tick sim; nothing maps onto the arena.

DONE 3 transferable bits, 4 donor files read. (Verdict: 2D-only, not worth stealing for the engine.)
