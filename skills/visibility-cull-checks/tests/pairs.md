# Skill-on vs skill-off pairs: visibility-cull-checks

Three arena culling situations. "Off" is the usual first guess; "On" names the check, the fix, and the proof that fails before and passes after.

## P1: walls behind the camera survive and every frame scans every mesh (checks 1, 2, 3)

Input: a diff in `crates/render/src/draw.rs` (the live frustum-cull owner) rebuilds a bounding box per frame from the mesh average center, tests planes in camera space, drops the near plane, and loops over all arena props flat. Props behind the camera stay drawn and frame time grows with prop count.

Answer with the skill: Bake and hierarchize. Check 1: bake sphere and box bounds once at cook time from all vertices for small cross-section. Check 2: move planes to world space with inward normals and keep near plus four sides so behind-camera walls fall to the near test. Check 3: group props under a parent volume so a miss skips children and a full hit accepts them. Prove with `cargo test -p engine-spatial --lib cull_behind_camera` green plus a capture showing behind-camera draws at zero.

What a skill-less answer misses: average centers make loose spheres that never cull, camera-space planes misclassify after every move, and without near plus hierarchy the flat loop pays O(n) forever.

## P2: every torch lights everything and shadows vanish at the screen edge (checks 6, 7)

Input: a diff in `crates/render/src/core.rs` (`Scene::cull_lights`) shades all objects for all point lights with no range test, and drops casters once they leave the view. The sun shadow pass draws the whole world into one map each frame while a pillar outside the view stops casting its in-view shadow.

Answer with the skill: Cull lights then shrink shadows. Check 6: skip point lights past max range and spot pyramids outside the frustum, and exclude unlit objects per light. Check 7: build the caster set as the hull of view plus light position from silhouette edges and back faces, per cascade, clipped to the ground plane. Prove with a capture where lit pixels hug each torch plus a count showing shadow draws fall while the pillar shadow stays.

What a skill-less answer misses: full-frame per-light shading burns ms on unlit pixels, and culling casters by the view alone deletes real shadows; only the light-plus-view hull keeps them with fewer draws.

## P3: the crypt draws every room twice and pillars pop through walls (checks 8, 9)

Input: a diff in `crates/arena/src/zones.rs` (the GP36 zone classifier) frustum-tests every crypt room each frame, draws shared doorway walls twice, and hides any prop touching a blocker box even when half of it sticks out. Overlap walls double-shade and half-hidden urns blink out as the camera moves.

Answer with the skill: Portals then full-inside only. Check 8: enter zones only through front-facing visible portals with clipped extruded regions, tagging objects already drawn so overlaps render once. Check 9: skip an object only when its whole volume sits inside an occlusion region whose blocker itself tested visible. Prove with a capture showing only portal-reached rooms drawn once plus `cargo test -p engine-game --lib portal_once` green.

What a skill-less answer misses: testing all zones throws away the portal win and double tags cost fill, while partial-overlap hiding turns a conservative test into visible popping.
