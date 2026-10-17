# API reference (rustdoc)

The handbook explains how the pieces fit. The signatures live in rustdoc, built from the same commit as this page by the Pages workflow: every link below opens the generated crate page under `docs/api/`. A game author usually starts at `engine-game` and `engine-arena`, and drives the engine through `engine-endpoint` (the AI door, see the [door reference](door-reference.md)).

Each line is the first sentence of the crate's own docs.

- [engine-core](api/engine_core/index.html): core: fixed tick, seeded rng, per-tick hash (FNV-1a 64).
- [engine-arena](api/engine_arena/index.html): arena: headless deterministic match spine (contracts C1 + AR2).
- [engine-combat](api/engine_combat/index.html): combat: frame-data combat. Timing as data (hitstop, cancel/parry windows, trauma). Never tick-coupled.
- [engine-physics](api/engine_physics/index.html): physics: fixed-dt semi-implicit Euler bodies + AABB overlap query.
- [engine-physfx](api/engine_physfx/index.html): physfx: the physical layer, a float Rapier world seeded by authority events, never hashed.
- [engine-anim](api/engine_anim/index.html): anim: SoA clip sampler + blend + quant. Copies [f32;3] out as Position, never shared types.
- [engine-render](api/engine_render/index.html): render: NullRenderer (headless proof). 16x16 tiles, MAX lights/tile.
- [engine-game](api/engine_game/index.html): game: packaged arena app skeleton.
- [engine-net](api/engine_net/index.html): net: rollback ring + repeat-last predict + input-delay buffer + snapshot delta over UDP/QUIC/relay transports (UdpTransport/RelayTransport/SimQuicNet/QuinnEndpoint).
- [engine-assets](api/engine_assets/index.html): assets: manifest entry + 7-gate checklist. Refs store id, never path.
- [engine-input](api/engine_input/index.html): input: deterministic action map for fixed-tick sim. Bits hash into the tick hash.
- [engine-endpoint](api/engine_endpoint/index.html): endpoint: agent driving seam. Core is std-only; the `mcp` feature adds the rmcp stdio server over the command service (see `bin/e2040-mcp.rs`).

## Build it yourself

The Pages job runs `cargo doc --no-deps` for exactly these twelve crates with the features that show their public API (`engine-physics/rapier`, `engine-physfx/rapier`, `engine-endpoint/mcp`). To read the same pages from a checkout:

```sh
cargo doc --no-deps --locked -p engine-core -p engine-arena -p engine-combat -p engine-physics -p engine-physfx -p engine-anim -p engine-render -p engine-game -p engine-net -p engine-assets -p engine-input -p engine-endpoint
```

Then open `target/doc/engine_game/index.html`. The links on this page only resolve on the published site, where rustdoc sits under `docs/api/`.
