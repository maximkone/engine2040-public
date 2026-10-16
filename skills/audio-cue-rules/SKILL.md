---
name: audio-cue-rules
description: "Use when you change how a sound plays in crates audio or the game window: distance gain, cue definitions, voice limits, ducking or music playlists, before you add a sound or a music track."
version: 0.1.0
author: skillworks
---

# Audio cue rules

Use when you change how a sound plays: distance gain, cues, voice limits, ducking, or music. These rules cover the mix. The real-time callback rules are in `frame-loop-rules`.

1. Gain falls as 1/r between the falloff min and falloff max distances, then clamps to zero. Dialogue may bend the curve. [geaen 13.4.3.1-13.4.3.3]
2. Play cues, not clips. A cue is clips plus 3D or 2D, falloff, group and loop. Give each cue 3-4 variants. [geaen 13.5.7.2]
3. When voices run out, steal by priority, not by age. Limit instances per group. [geaen 13.5.8.2]
4. Duck the music under dialogue. [geaen 13.5.8.4]
5. Music follows tension playlists, with stingers and bar-aligned fades. [geaen 13.6.3]

## Repo facts

- `crates/game/src/bin/arena.rs::ArenaApp::new`: the arena window plays `Bank::placeholder`, three placeholder synth sounds with no music. Rules 2 and 5 are not in the window yet.
- `content/audio`: 67 real clips sit unused. Pick a cue group from them before you add a synth.
- `crates/audio/src/voice.rs::synthesize_live`: the module doc at the top of the file says live TTS always refuses. Voice output is a bake, not a live synth.

Search more: `node tools/books.mjs search "sound cue falloff voice priority"`
