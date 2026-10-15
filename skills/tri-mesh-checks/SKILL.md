---
name: tri-mesh-checks
description: Use when a builder cooks or budgets a triangle mesh in crates/assets before the import lands.
---
# Tri mesh checks
Nine checks for mesh counts. PASS needs tri_count plus validate_mesh green and x2 equal, never a picture alone.
1. **Tris are faces.** Ask: does `tri_count` in `crates/assets/src/mesh_cook.rs` return faces F (indices/3) from `crates/assets/src/cooked.rs::CookedMesh`, not verts? Flag: vert count passed as tris that hides 3x. Crate: assets.
2. **Indices form triples.** Ask: does `validate_mesh` in `crates/assets/src/cooked.rs` refuse index_len %3 !=0 and out-of-range indices from `crates/assets/src/mesh_import.rs::import_gltf_bytes`? Flag: stray indices that cook. Crate: assets.
3. **Winding stays CCW.** Ask: is outward kept by CCW winding in `validate_mesh` (`cooked.rs`) with zero-area refused by `ZERO_AREA_M2` in `mesh_cook.rs`? Flag: flipped faces with dark lighting. Crate: assets.
4. **Verts are shared.** Ask: does the cook keep shared verts (one box 8 verts, 12 tris) via `optimize_index_order` in `mesh_cook.rs`, not split verts? Flag: 36 verts for a box that inflates budgets. Crate: assets.
5. **Closed meshes balance.** Ask: for closed hulls does `CollisionHull` in `mesh_cook.rs` satisfy V-E+F=2, with open meshes marked open, not forced shut? Flag: open cape counted as closed. Crate: assets.
6. **Normals come from the cross.** Ask: are face normals cross(edge1, edge2) unit length, with missing NORMAL filled area-weighted in `mesh_import.rs`? Flag: unnormalized normals that shade seams. Crate: assets.
7. **LOD shrinks tris.** Ask: does each LOD in `mesh_cook.rs::LOD_RATIOS` hold fewer tris via `simplify` in `collapse.rs`, checked by `tri_count` in `lod_swap.rs`? Flag: LOD1 larger than LOD0. Crate: assets.
8. **Budget reads stats.** Ask: does the GS15 check read plain stats (tris, bones, influences, materials, tex_px) against `HERO_MAX_TRIS`/`PROP_MAX_TRIS` in `mesh_cook.rs`, reporting both numbers? Flag: GLB bytes read inside the budget test. Crate: assets.
9. **Same bytes twice.** Ask: is `tri_count` plus `cook_mesh` in `mesh_cook.rs` pure with x2 equal bytes, no clock or hash order? Flag: run-to-run tri drift. Crate: assets.
## Do / Don't
- Do: count faces, keep winding, share verts, and report both numbers at the gate. Don't: count verts as tris, flip winding, or split verts to pass a budget.
source: Lengyel, Foundations of Game Engine Development Vol 1, ch. 3.1-3.2
