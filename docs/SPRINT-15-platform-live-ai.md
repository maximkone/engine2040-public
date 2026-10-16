### Platform and live AI (owner 2026-09-26, `docs/plan/living-world.md`)

Shipping on Steam (platform contract with a null default, Steamworks
backend, store builds, minidumps, localization with right-to-left
scripts, sequencer, benchmark, presets) and live AI players meet (voice
out and in, lip-sync, moderation, vision judge, recall, a shipped local
model, quotas with no keys in clients). The tick never waits; model and
platform outputs enter only as recorded actions.

- [ ] PLF4 game: Workshop data mods — publish and subscribe data mods
  (maps, missions, tables) validated by schema, provenance and a size
  cap; code mods stay out. Tier C. Size M. Owns
  `crates/game/src/workshop.rs`. Accept: a mission mod round-trips; a mod
  with an unknown schema or oversize payload refuses. needs: PLF2.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] PLF5 tools: store build script — `tools/ship.mjs` builds the ship
  profile, runs the GM3 clean-folder check, archives symbols by build id
  and writes a depot manifest; the upload itself stays an owner action.
  Tier B. Size S. Owns `tools/ship.mjs` (orchestrator). Accept: two runs
  give identical manifests for identical inputs; a failing clean check
  stops the script. needs: GM3, OPT5.
- [ ] PLF6 game: minidumps and a symbol store — minidump-writer beside
  the WIN4 report; symbols archived per build id; a local symbolize
  command; nothing uploads. Tier B. Size S. Owns
  `crates/game/src/crash.rs` hunks plus the tool. Accept: a forced crash
  yields a dump that symbolizes to the faulting function. needs: WIN4.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] PLF7 ui: localization — Fluent string tables, a pseudo-locale that
  lengthens and marks every string, right-to-left shaping and bidi for
  Arabic and Hebrew, font fallback, and a gate that fails on hardcoded
  UI strings. Tier B. Size L. Owns `crates/ui/src/locale.rs`. Accept:
  every menu renders in pseudo-locale without clipping; an Arabic sample
  shapes and orders correctly against a reference image; the gate
  catches a seeded literal. needs: UI2.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] PLF8 editor: cutscene sequencer — timeline tracks (camera,
  animation, audio, subtitle, event) stored as data, deterministic
  playback, scrub and keyframes through command batches with undo;
  missions use it for intros. Tier B. Size L. Owns
  `crates/editor/src/sequencer.rs`. Accept: a 30-second sequence plays
  identically x2; every edit undoes; a mission intro plays from data.
  needs: ED1, AN1.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] PLF9 game: player benchmark — a fixed fly-through reporting fps
  percentiles, tier, adapter and driver to a shareable file. Tier B.
  Size S. Owns `crates/game/src/benchmark.rs`. Accept: two runs within
  5%; the file validates. needs: OPT2.
- [ ] PLF10 game: settings presets and safe start — tier and resolution
  scale picked from RT0 plus a five-second probe; a crash at start
  relaunches in safe mode. Tier B. Size S. Owns
  `crates/game/src/presets.rs`. Accept: presets differ by adapter class;
  a seeded start crash relaunches safe. needs: RT0, UIX2.
- [ ] LAI1 provider: voice out — text-to-speech as a provider capability
  (local and remote adapters) with audio cassettes keyed by voice, text
  and style digest; a deadline falls back to subtitles alone. Tier B.
  Size M. Owns `crates/provider/src/voice.rs`. Accept: a line replays the
  same audio bytes x2 offline; a slow backend shows the subtitle within
  its deadline. needs: OPT24, PV4.
- [ ] LAI2 provider: voice in — speech-to-text for push-to-talk
  commands; transcripts map to validated recorded orders (UW18) above a
  confidence floor, else a typed refusal; keys remain the fallback.
  Tier B. Size M. Owns `crates/provider/src/listen.rs`. Accept: 50
  recorded clips map to the expected orders ≥ 90%; below-floor clips
  refuse; replays use the recorded order, never the audio. needs: PV4,
  UW18.
- [ ] LAI3 anim: live lip-sync — visemes from TTS phoneme timings or
  audio analysis drive AM12 faces; presentation only. Tier C. Size M.
  Owns `crates/anim/src/lipsync.rs`. Accept: viseme timing within 60 ms
  of the reference track; golden unchanged. needs: AM12, LAI1.
- [ ] LAI4 provider: moderation — every model line passes a rating
  profile (blocklist, optional classifier adapter, length and topic
  caps) before presentation; refusals fall back to scripted lines;
  player text to NPCs passes the injection scorer first; every decision
  recorded. Tier B. Size M. Owns `crates/provider/src/moderate.rs`.
  Accept: a 200-line red-team set is caught at the profile's bar with no
  scripted-line false positives; decisions replay x2. needs: PV3, LW8.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] LAI5 playtest: vision judge — a vision-language adapter scores
  captures against a checklist into structured per-item verdicts;
  calibrated on 50 owner-labeled captures before its scores count;
  cassette-recorded. Tier B. Size M. Owns
  `crates/playtest/src/vision_judge.rs`. Accept: agreement ≥ 80% on the
  labeled set reported per backend; replay from cassettes x2. needs:
  OPT24, OPT25.
- [ ] LAI6 ai: embedding recall — an optional embedding adapter for NPC
  memory with int8 vectors and deterministic top-k (ties by id), falling
  back to LW2 salience; recall sets recorded. Tier C. Size M. Owns
  `crates/ai/src/recall.rs`. Accept: recall sets identical x2 from
  recordings; with no adapter the LW2 path runs unchanged. needs: LW2,
  OPT24.
- [ ] LAI7 provider: shipped local model — a small model whose weights
  allow commercial use (license read, provenance row) runs on CPU through
  a llama.cpp-class runtime under the OPT27 arbiter, so NPC talk works
  offline with no account. Tier C. Size L. Owns
  `crates/provider/src/embedded.rs`. Accept: ≤ 2 s per NPC line on the
  reference CPU while the window build holds its frame budget; moderation
  applies. needs: OPT27, LAI4.
- [ ] LAI8 provider: quotas and key safety — per-player and per-session
  model budgets, rate limits, an owner-run key relay option and
  player-owned keys in the OS store; a scanner proves no key ships in
  the package. Tier B. Size M. Owns `crates/provider/src/quota.rs`.
  Accept: a seeded key in the package fails the scan; quota overruns
  fall back to scripted lines with a notice. needs: OPT26, AW9.
