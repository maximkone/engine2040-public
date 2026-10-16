# CT35 hero LOD bake receipt (splitter piece A)

Bake command (lane FLAGS):
`cargo run --manifest-path .lanes/ct35/Cargo.toml --target-dir target/pool-1 -j 4 --locked -p engine-assets --features import --example ct35_lod_bake -- .lanes/ct35/content/fighters/glb/hero_body.glb .lanes/ct35/content/fighters/lod/hero.json`
Exit 0. Input `hero_body.glb`, 2072320 bytes.

Bake line (pasted, not invented):
`CT35-LOD-BAKE meshes=4 mesh0 lod_tris=[1448,723,362,180] digest=2ea9b9b1b504edc2 mesh1 lod_tris=[2232,1115,558,279] digest=cc1d9ea78c322a9f mesh2 lod_tris=[1434,717,359,180] digest=06e81b6c8ee1b370 mesh3 lod_tris=[736,368,184,92] digest=a898fb422d898f35 asset_digest=fcc2708f5643c9ef gate=check_chain:ok`

Accept facts:
- Cook x2 identical: the tool cooks the same bytes twice and refuses on drift; exit 0 means identical. Sidecar `hero.json` carries `"cook_x2_identical": true` plus `asset_digest fcc2708f5643c9ef`.
- LOD1-3 monotonic: every mesh chain strictly decreases (1448>723>362>180; 2232>1115>558>279; 1434>717>359>180; 736>368>184>92), enforced by `check_chain` (growing chains refuse naming the level).
- Hulls containing: all 16 LOD hulls report `"hull_contains": true` in `hero.json`.
- LOD0 budgets: all base counts (1448, 2232, 1434, 736) sit under Hero max 40000.
- Tests: `cargo test --locked -p engine-assets --features import lod_swap` 9 passed (incl. `steady_state_poll_allocates_nothing`); `mesh_cook` 13 passed (incl. `cook_identical_twice`, `hulls_contain_their_mesh`); example `ct35_lod_bake` 8 passed.
- 0 tick allocs: proven by the existing `steady_state_poll_allocates_nothing` test, green above.

Consumer: `hero.json` names `"consumer": "CT41 LOD ms/VRAM table + lod_swap picker"` and `"gate": "check_chain:ok"`. P6 ms/VRAM per LOD rides CT41 (pilot-only, reference GPU).
