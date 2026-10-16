---
name: bevy-rust-ecs
description: Use when stealing or porting Bevy 0.19 code or methods into engine2040's own renderer (crates/render) or sim crates, or when a Bevy name does not compile. No Bevy viewer exists or may be built.
---

# Bevy to engine2040 port map

The Bevy viewer was dropped on 2026-10-07. The picture is our own renderer in `crates/render`. Do not build or extend a Bevy viewer. Reading Bevy source and stealing its methods into our code stays fine.

Reference version: Bevy 0.19.1. Bevy 0.20 is a release candidate: do not use it. Old tutorials (0.12 to 0.18) use names that no longer exist.
Version note: the WGSL ports in `crates/render/src/v1_gpu.rs` were taken from Bevy `v0.15.0`, so the names below are for new ports only. Write the donor version in the header of each port.

## Names that changed in Bevy 0.19 (old -> current)

- `SceneRoot` -> `WorldAssetRoot`; `SceneInstanceReady` -> `WorldInstanceReady` (import from `bevy::world_serialization`).
- `Camera3dBundle`, `PbrBundle`, `SceneBundle`, `DirectionalLightBundle`: no bundles now. Spawn a tuple of components; required components add `Transform`, `Visibility` and `Camera`.
- `EventReader`, `EventWriter`, `add_event` -> `MessageReader`, `MessageWriter` (`.write(..)`), `add_message`.
- Observers take `On<E>`, not `Trigger<E>`.
- `AnimationPlayer::play` takes an `AnimationNodeIndex` from an `AnimationGraph`; the entity also needs an `AnimationGraphHandle`.
- `get_single()` -> `single()` (a Result); `Input<KeyCode>` -> `ButtonInput<KeyCode>`; the ambient light resource -> `GlobalAmbientLight`; `delta_seconds()` -> `delta_secs()`; `despawn_recursive()` -> `despawn()`.
- A type cannot derive both `Component` and `Resource`: a Resource is a Component now.

## How to port a Bevy method

- Write the steal as plain Rust over our own types. A Bevy system becomes a function over our own state; the sim crates never depend on Bevy.
- Keep the fixed tick free of Bevy scheduling. Time and order come from our tick (`docs/HASH-LAW.md`).
- Before you use a Bevy name you are unsure of, check it in the Bevy 0.19.1 source. Do not trust a tutorial's name.
- Name the donor in the steal note: `bevy@<tag> <file>:<line>`, then the line you wrote in our code.
