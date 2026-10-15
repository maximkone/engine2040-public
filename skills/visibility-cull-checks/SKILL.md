---
name: visibility-cull-checks
description: "Use when a builder adds frustum, light, shadow, portal, or occluder culling in crates render, spatial, or game and when a reviewer checks the culled frame before it ships"
---

# Visibility cull checks

Nine checks from Lengyel Vol 2 ch 9. Name the crate per finding. PASS needs a test or capture, never a promise.

1. **Bake tight volumes once.** Ask: are sphere and box bounds built at cook time from all vertices for small cross-section, never rebuilt per frame? Flag: per-frame min-max over the mesh, average-vertex centers, or triangle meshes tested directly. (crates/spatial)
2. **Test in world space with near kept.** Ask: are region planes inward-facing in world space, testing near plus four sides with far optional? Flag: camera-space planes used raw, or near dropped so walls behind the camera survive. (crates/render)
3. **Skip and accept whole groups.** Ask: does a missed parent skip its children and a fully inside parent accept them with no more tests? Flag: flat loop over every object, or retesting children of a fully inside parent. (crates/spatial)
4. **Cull spheres by center minus radius.** Ask: is a sphere dropped only when its center lies beyond its radius behind any one plane? Flag: center-only tests that pop large objects, or exact corner math that costs more than its false positives. (crates/render)
5. **Cull boxes by projected extents.** Ask: is the per-plane radius the summed absolute projections of half-extents, with a fast axis-aligned path? Flag: sphere radius reused for long boxes, or a full vertex loop per plane. (crates/render)
6. **Cull lights before shading.** Ask: are out-of-range points and out-of-frustum spot pyramids skipped, with unlit objects excluded per light? Flag: every light shades every object, or shadows dropped once the caster leaves the view. (crates/render)
7. **Build small shadow regions.** Ask: is the caster set the hull of view plus light from silhouette edges and back faces, built per cascade and clipped to ground? Flag: whole world drawn into each shadow map, or one giant region that never shrinks. (crates/render)
8. **Enter zones only through seen portals.** Ask: are zones entered only through front-facing visible portals with clipped extruded regions, tagging twice-seen objects? Flag: every zone tested each frame, or overlap walls drawn twice. (crates/game)
9. **Hide only what is fully behind a proven blocker.** Ask: is an object skipped only when its whole volume sits inside an occlusion region whose blocker tested visible? Flag: partial overlap treated as hidden, or blockers used without their own visibility test. (crates/render)

## Do / Don't
- Do: bake tight volumes, cull groups then lights then shadows through portals, and hide only fully blocked objects. Don't: test every triangle every frame and shade every light in every zone.
source: Lengyel, Foundations of Game Engine Development Vol 2, ch. 9.3-9.9
