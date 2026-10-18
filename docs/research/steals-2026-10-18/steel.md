STEAL-STEEL scout, 2026-10-18. Wave 2026-10-18/1, block 1: github.com/topics/game-engine (l=rust, o=desc, s=updated), position 21. Area: screen picking via an id pass, reflected editable data, asset fallback.
Read only, ideas only; no code copied, nothing vendored. Every donor file below was fetched live on 2026-10-18.

== 1. WHAT IT IS ==
Steel (SSSxCCC, 12 stars, updated 2026-10-18) is a small Rust engine with an **integrated editor** (the editor is a build feature of an application, not a separate binary), currently at 0.3.0. It renders through Vulkano with both a rasterization path and a real-time ray-tracing/path-tracing path, uses `shipyard` ECS, `glam`, `egui`, `rapier` (2D only so far), Windows + Android clients, and ships a tutorial plus a one-file hello-world example. Its roadmap is honest: 3D physics, audio, animation, particles, networking, AI and tests are all unchecked. Our interest is the editor-facing plumbing: entity-id picking straight out of the frame, a reflected data/limit model for the inspector, and how a missing asset degrades.

== 2. LICENCE ==
MIT (SPDX `MIT` via GitHub API 2026-10-18; LICENSE read live 2026-10-18: https://raw.githubusercontent.com/SSSxCCC/steel/main/LICENSE — "MIT License, Copyright (c) 2024 SSSxCCC"). Note: `dependencies/` vendors vulkano, vulkano-util and egui_winit_vulkano under their own (MIT/Apache-2.0) licences — not used here. Ideas only.

== 3. STEALS (ranked) ==
1. Every drawn primitive carries an entity id; picking is one pixel read from a separate id image | SSSxCCC/steel@main:crates/steel-internal/src/render/canvas.rs:29-51,360-420 | PRESENTATION (editor/MCP) | engine2040 editor picking (`editor/src/viewport.rs:1-20`), MCP door "what is under the cursor" | M
   Idea: the per-frame `Canvas` stores its draw lists **with the `EntityId` in the payload** — points as `(pos, color, eid)`, lines as two such vertices, meshes/circles/spheres as `(…, model, eid)` (:29-51) — and the doc comment states the rule outright: "All the drawing data requires an EntityId for screen object picking" (:31). The canvas keeps one `eid_image` per window and per in-flight image (:258,285-295) and renders that id buffer alongside the colour frame (:317-347). `get_entity_at_screen_system` (:361-420) reads back the two `u32` at the cursor pixel and returns the entity, or `EntityId::dead()` when nothing was drawn there (:375,416).
   Maps: engine2040 picking today is a separate raycast; an id image resolved by one pixel readback gives the same answer for sprites, debug lines and thin geometry that a physics ray misses, and it is the natural way for the MCP door to answer "what is under this screen point" without a physics query.
   Kill: the eid pass must not write into the beauty frame or move the golden; if the readback is ever a full-frame stall instead of one pixel scoped after the present, drop it.

2. Reflected editable data with declarative limits, separate from the component's own type | SSSxCCC/steel@main:crates/steel-common/src/data.rs:11-45,47-60 | TOOLING (editor) | engine2040 `editor/src/inspector_auto.rs:1-10` (schema rows without reflection) | M
   Idea: one `Limit` enum (:11-45) names every inspector constraint in data, not code — int/f32/f64 ranges, an int enum with display strings, a rotation clamp that shows degrees, `Vec3Color`/`Vec4Color` to pick a colour, `StringMultiline`, `ReadOnly` — and one `Value` enum (:47-60) is the reflected value (`Bool`, `Int32`…`Vec2/Vec3/Vec4`, `String`). A `#[derive(Edit)]` puts a `#[edit(limit = "Limit::Vec4Color")]` attribute next to the field (see `render/texture.rs:30-38`), so the limit lives with the field and the panel just renders what the limit says.
   Maps: engine2040's inspector derives rows without reflection, so a colour, a clamped scalar and a read-only field all look the same; a small `Limit` enum would let one panel render every type and keep the constraint beside the data it constrains.
   Kill: if a new component needs a new `Limit` variant or a hand-written panel, or the derive churns the schema rows -> cap the variant set and fail the build on an unmapped type.

3. Asset bytes cached once; an invalid asset degrades to a solid colour instead of failing | SSSxCCC/steel@main:crates/steel-internal/src/asset.rs:35-57; render/texture.rs:28-38 | PRESENTATION/assets | engine2040 `assets` last-good payloads, texture/material fallback | S
   Idea: `AssetManager::get_asset_content` reads a file the first time, caches the bytes in an `Arc<Vec<u8>>`, and returns immediately after (:36-57); a read failure logs a warning and returns `None` rather than propagating (:48-51). The `Texture` component documents the same rule and acts on it: "When texture asset is invalid, this texture is a solid color" — `color` doubles as the fallback tint when `asset` resolves to nothing (:28-38).
   Maps: engine2040 already keeps last-good cooked payloads; making the *component* carry its own fallback (a tint / a placeholder) means a missing or not-yet-cooked asset still draws a deterministic pixel instead of a gap or a panic.
   Kill: if the byte cache is unbounded across a long session (needs an eviction rule) or the fallback colour differs between tiers -> pin the fallback and bound the cache, else skip.

== 4. NOT WORTH STEALING ==
- `crates/steel-internal/src/time.rs:1-45`: wall-clock `now`/`delta` with a `reset()`; engine2040's fixed-step tick owns sim time and wall clock never enters the hash, so this is the pattern we deliberately avoid.
- The Vulkano rasterization/ray-tracing pipeline split and the vendored `dependencies/vulkano` fork (README: kept for an upstream issue): engine2040 is wgpu, so no code and little idea transfers.
- The path-tracing "ray tracing in one weekend" example: a demo, not a shape we need for the duel.

== 5. FILES READ ==
README.md, LICENSE, crates/steel-shader/src/lib.rs, crates/steel-internal/src/render/canvas.rs, render/texture.rs, render/mesh.rs, crates/steel-internal/src/asset.rs, crates/steel-internal/src/time.rs, crates/steel-common/src/data.rs (all @main, 2026-10-18).
