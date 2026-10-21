# Landing — audio: the mixer reports into DeviceHeal (SMX-03, re-landed)

Date: 2026-10-21. Lane: smart AI-native engine (14). Crate: `engine-audio`.
This is the re-landing after a path restore wiped the first attempt from the
shared tree: `crates/audio/src/engine.rs` and `crates/audio/src/heal.rs` were
back at HEAD, so `DeviceHeal` was again a module only a `#[cfg(test)] mod` in
`crates/game` called. Nothing else in the crate is touched.

## What changed

`crates/audio/src/heal.rs`

- `DeviceHeal::count_silent_frames(frames) -> u64`: counts frames the device
  never heard without rendering anything, so an owner that renders into its
  own buffer and then drops it (the engine) counts the frames once instead of
  rendering a second time. `render_silent_frames` now delegates to it, so the
  counting rule lives in one place.
- `DeviceHeal::state_digest() -> u64`: FNV-1a over the device state and the
  silent-frame count. This is the before/after pair a recovery proof carries.
- `DeviceHeal::try_reopen` now clears the reconnect budget with a **proved**
  recovery: it builds `VerifiedRecovery::new(DEVICE_FAULT_CODE, attempts,
  HealValidation::Recovered, before_digest, after_digest)` and calls
  `Supervisor::reset_verified`. The observe supervisor refuses a budget reset
  without a proof (`crates/observe/src/supervisor_proof.rs:53` `reset` returns
  `ResetOutcome::Refused` without one), so the old bare `reset` call could
  leave the budget spent while the heal claimed recovery. A refused proof no
  longer goes unnoticed either: the notice ends with `; the reconnect budget
  was NOT cleared`.

`crates/audio/src/engine.rs`

- `AudioEngine` owns a `DeviceHeal` (`heal: DeviceHeal`, built by
  `DeviceHeal::new()` in `with_feed`).
- `AudioEngine::report_loss(tick)` and `report_change(tick)` report a device
  fault from the mixer path into the heal and return the supervisor's
  `SupervisedDecision`.
- `AudioEngine::heal()` reads the heal state (live / silent / quarantined,
  notice, attempts, silent frames) from outside the crate.
- `AudioEngine::try_reopen(tick, opener)` reopens through the heal.
- `AudioEngine::render_stereo_into` now degrades when the device is down: the
  mixer and the music bed keep advancing, the output buffer is zeroed (silence
  on the wire) and every frame is counted by the heal. Before this, the engine
  rendered to the wire whatever the device was doing.
- Two tests, and they are in the crate's normal `#[cfg(test)]` module, but the
  code they exercise is the production render path, not a stand-in:
  `a_device_loss_degrades_the_engine_to_silence_and_the_heal_state_is_readable`
  and `a_device_change_reports_into_the_same_heal`.

`crates/audio/src/lib.rs` and `crates/audio/src/block.rs` are **not touched**:
the browser worklet's `pub mod block;` / `pub use block::{render_block,
BlockStats};` lines belong to another lane and stay exactly as they are. No
export was needed for this wiring (`pub mod heal;` already exports the type).

## Proof

| command | result |
|---|---|
| `node tools/gh-check.mjs cargo test -p engine-audio --lib` | PASS — `test result: ok. 292 passed; 0 failed; 5 ignored` (run 39517352276) |
| `node tools/gh-check.mjs cargo test -p engine-audio --lib a_device` | PASS — `test result: ok. 2 passed; 0 failed; 295 filtered out` (run 39517653516) |
| `node tools/gh-check.mjs cargo clippy -p engine-audio --all-targets --no-deps -- -D warnings` | PASS (run 39518124988) — the earlier `engine-observe` `private_interfaces` block is gone; no edit was made to crates/observe |
| `node tools/rust-quality.mjs --crate audio` | `RESULT PASS: score 98 (A), 0 ratchet regressions over 1 crate(s)` |

The two tests prove: a live render is audible and the heal reads `Live`;
`engine.report_loss(10)` puts the heal in `Silent` with a readable notice and
`attempts() == 1`; the next render of 64 samples writes pure silence and the
heal counts 32 silent frames; a verified reopen reports `recovered`, returns
the heal to `Live`, and the next render is audible again.

## Still open / red elsewhere

- `cargo clippy -p engine-audio` was blocked in the earlier pass by
  `engine-observe`'s own `private_interfaces` errors
  (`crates/observe/src/supervisor_proof.rs:305,349` then). That file belongs to
  the auto-heal lane; this lane does not edit `crates/observe`. If the block is
  still there it is named in the proof row above.
- The playtest/game side that would call `report_loss` on a real cpal error is
  `crates/game/src/audio_mix.rs` (another lane) and `DeviceHeal::reopen_live`
  stays its opener.
- Golden `d57b5c8ae3096614`: untouched by this crate (audio is presentation
  side; no simulation code changed).
