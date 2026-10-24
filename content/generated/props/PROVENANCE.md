# Generated props: provenance

Everything in this folder is built by `tools/blender/prop_gen.py` and recorded in
`props_built.json` by the same run (id, file, triangles, build, classes). Sizes and
hashes below are from the working tree on 2026-10-24.

    blender -b --factory-startup --python-exit-code 1 -P tools/blender/prop_gen.py -- content/generated/props

The 24 catalogue props (weapons, armour, world props) are authored shapes from
`content/design/props.json` plus the builders in `prop_gen.py`; their tri counts are
the first 24 rows of `props_built.json`, unchanged since the kit landed.

The three machine props are seeded: `tools/blender/machine_gen.py` owns the mesh
numbers and `prop_gen.py` calls it at one fixed seed, so the run above reproduces
these files byte for byte. The mesh algorithm is ported from
[ldo/blender_spaceship_generator](https://github.com/ldo/blender_spaceship_generator)
(MIT License, Copyright (c) 2016 Michael Davies); the generated output below is
engine-owned content with no third-party bytes. Each machine also has a
`.machine.json` sidecar with its seed, bounds and mesh sha256 (the same value a
`--stats` run prints).

| file | source | licence | seed | bytes | tris | mesh sha256 |
|---|---|---|---|---|---|---|
| `machine_ballista.glb` | engine data: `machine_gen.py` preset `ballista` | engine-owned; ported algorithm MIT (Davies 2016) | 7 | 18,048 | 302 | `6e14a9d228b1d39e9e15346104c7e1a54022bb4c00c2ab58ca806a7ebf77f0cd` |
| `machine_crane.glb` | engine data: `machine_gen.py` preset `crane` | engine-owned; ported algorithm MIT (Davies 2016) | 7 | 43,148 | 770 | `60983ef821733b9d3f08be034823d84548c56413e9a8dc37ffa8fb09b540ea2d` |
| `machine_cart.glb` | engine data: `machine_gen.py` preset `cart` | engine-owned; ported algorithm MIT (Davies 2016) | 7 | 42,440 | 756 | `b1e23b1a0ca65eb6ba66ee9a635d06e2f2eeaa7a27323d7e5bf76a9508b009fd` |

Each machine is under the 1500-tri potato cap (`crates/assets/src/budget.rs::POTATO`)
and under `PROP_MAX_TRIS` (5000, `crates/assets/src/mesh_cook.rs`), with at most three
material slots (`machine_wood`, `machine_iron`, `machine_tarp`) and no textures.

Cook twice proof: the standalone command below was run into two folders, and every
GLB and sidecar came out identical; `prop_gen.py`'s machine pass then wrote the same
bytes as the standalone cook for all three.

    blender -b --factory-startup --python-exit-code 1 -P tools/blender/machine_gen.py -- <out.glb> --preset ballista --seed 7

Test: `node --test tools/blender/machine_gen.test.mjs` (the manifest lists the three
machines under the cap and each row matches the generator's own tri count and bake).
