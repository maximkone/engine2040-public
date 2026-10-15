# Skill-on vs skill-off pairs: tri-mesh-checks

Three mesh budget reviews. Off is habit without the checklist. On names the numbered item, the failure it stops, and the proof it asks for.

## P1: verts counted as tris (items 1, 8)

Situation: a diff in `crates/assets/src/mesh_cook.rs` sets `tri_count` to `verts.len()` to save an index pass. A 3,900-tri fighter with 2,100 verts passes the GS15 4,000 cap, and a 4,100-tri fighter with 2,200 verts also passes. Test `cargo test -p engine-assets --lib budget` stays green because it only checks small cubes.

Off: Verts track tris closely enough. The budget passes. Approve.

On: Reject. Item 1: tris are faces F (indices/3) from `CookedMesh` in `crates/assets/src/cooked.rs`, not verts. Item 8: the GS15 check must read plain stats against `HERO_MAX_TRIS` in `mesh_cook.rs` and report both numbers. Proof: a 3,900-tri mesh passes and a 4,100-tri mesh reports `tris` with measured 4100 vs limit 4000; `tri_count` equals indices/3 x2.

What skill-less misses: a 3x undercount that lets over-budget fighters cook clean. Verts look plausible on cubes and fail on real fighters.

## P2: flipped winding with dark faces (items 3, 6)

Situation: a diff in `crates/assets/src/cooked.rs::validate_mesh` drops the winding check to accept mirrored imports. A mirrored shield cooks with inward faces. The capture shows dark patches, blamed on IBL. `import_gltf_bytes` in `mesh_import.rs` still fills missing NORMAL.

Off: Mirrored files are rare. Lighting will cover it. Approve.

On: Reject. Item 3: outward stays CCW with zero-area refused by `ZERO_AREA_M2` in `mesh_cook.rs`. Item 6: face normals are cross(edge1, edge2) unit length, missing NORMAL filled area-weighted in `mesh_import.rs`. Proof: mirrored shield must be refused or rewound; face normal equals normalized cross x2.

What skill-less misses: winding as a correctness gate, not a style choice. Without it mirrored meshes shade dark and the budget counts faces that face inward.

## P3: split verts inflate the budget (items 4, 9)

Situation: a new cook path in `crates/assets/src/mesh_cook.rs` splits verts per tri before `optimize_index_order` for simpler UVs. A box reports 36 verts and 12 tris, LOD0 grows, and `cook_mesh` output drifts run to run. The PR says split verts render the same.

Off: Same pixels, so same mesh. Approve.

On: Not yet. Item 4: keep shared verts (one box 8 verts, 12 tris) via `optimize_index_order`, not split verts. Item 9: `tri_count` plus `cook_mesh` stay pure with x2 equal bytes. Proof: box cooks to 8 verts and 12 tris twice byte-equal; `simplify` in `collapse.rs` plus `tri_count` in `lod_swap.rs` confirm LOD shrink.

What skill-less misses: sharing as a budget gate. Split verts pass the tri cap while blowing vert memory and breaking x2 equality.
