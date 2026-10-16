---
name: gaffer-netcode
description: Use when you review or change crates/net session, snapshot, interp or online code that moves state, inputs or remote views across ticks.
---

# Gaffer netcode checklist

Gaffer On Games, verified 2026-10-10:
`https://gafferongames.com/post/what_every_programmer_needs_to_know_about_game_networking/`.
Own words below; one short phrase quoted per rule at most.

1. **Server owns the world.** Clients send inputs, never positions.
   Do: keep sim authority in `Server` (§Client/Server) at
   `crates/net/src/session.rs::Server`; client `InputPacket` carries input only.
2. **Predict self, accept correction.** Hide your own latency, then obey.
   Do: predict local moves at once (§Client-Side Prediction); reconcile from
   `StatePacket` in `crates/net/src/session.rs::StatePacket` by rewind-and-replay inputs.
3. **Keep past inputs to replay.** Correction needs the input log.
   Do: keep a ring of past inputs like `redundancy_window`
   (§Client-Side Prediction) in `crates/net/src/session.rs::redundancy_window` for resends.
4. **Snapshot the shared truth.** Deltas ride on full state saves.
   Do: persist full state with `save_snapshot`/`load_snapshot`
   (§Client/Server) in `crates/net/src/snapshot.rs::save_snapshot` and
   `crates/net/src/snapshot.rs::load_snapshot`; deltas only vs confirmed.
5. **Show others in the past.** Smooth remotes by interpolation, not latest.
   Do: render remotes through `SnapshotInterpolator` (§Entity Interpolation)
   in `crates/net/src/interp.rs::SnapshotInterpolator`; never snap to the newest packet.
6. **Step fixed turns, lockstep needs same start.** Turns need same commands.
   Do: advance sim in fixed ticks from one input set (§Peer-to-Peer Lockstep);
   join state comes from `ConfirmedHistory` in `crates/net/src/snapshot.rs::ConfirmedHistory`.

## Do / Don't

- Do: check path health in `crates/net/src/online.rs::resolve_path`
  with pwsh `cargo test -p engine-net --lib session`.
- Don't: let the client set its own position, feed variable dt to the sim,
  or show remote snapshots unsmoothed.
- Don't: extrapolate remote motion. `Interpolator` in `crates/net/src/delta.rs`
  clamps at the newest sample and flags it. Dead reckoning (Game Engine Gems 2,
  ch. 18) is not this engine's policy.
