# Bevy 0.19 names (old to current)

For new ports from Bevy 0.19.1 only. Check the 0.19.1 source before you trust a name.

- `SceneRoot` -> `WorldAssetRoot`. `SceneInstanceReady` -> `WorldInstanceReady` (import from `bevy::world_serialization`).
- `Camera3dBundle`, `PbrBundle`, `SceneBundle`, `DirectionalLightBundle`: no bundles now. Spawn a tuple of components. Required components add `Transform`, `Visibility` and `Camera`.
- `EventReader`, `EventWriter`, `add_event` -> `MessageReader`, `MessageWriter` (`.write(..)`), `add_message`.
- Observers take `On<E>`, not `Trigger<E>`.
- `AnimationPlayer::play` takes an `AnimationNodeIndex` from an `AnimationGraph`. The entity also needs an `AnimationGraphHandle`.
- `get_single()` -> `single()` (a Result). `Input<KeyCode>` -> `ButtonInput<KeyCode>`. The ambient light resource -> `GlobalAmbientLight`. `delta_seconds()` -> `delta_secs()`. `despawn_recursive()` -> `despawn()`.
- A type cannot derive both `Component` and `Resource`: a Resource is a Component now.
