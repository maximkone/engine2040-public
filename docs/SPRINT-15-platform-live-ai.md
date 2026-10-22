### Platform and live AI (owner 2026-09-26, `docs/plan/living-world.md`)

Shipping on Steam (platform contract with a null default, Steamworks
backend, store builds, minidumps, localization with right-to-left
scripts, sequencer, benchmark, presets) and live AI players meet (voice
out and in, lip-sync, moderation, vision judge, recall, a shipped local
model, quotas with no keys in clients). The tick never waits; model and
platform outputs enter only as recorded actions.

- [ ] PLF8 editor: cutscene sequencer — timeline tracks (camera,
  animation, audio, subtitle, event) stored as data, deterministic
  playback, scrub and keyframes through command batches with undo;
  missions use it for intros. Tier B. Size L. Owns
  `crates/editor/src/sequencer.rs`. Accept: a 30-second sequence plays
  identically x2; every edit undoes; a mission intro plays from data.
  needs: ED1, AN1.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
