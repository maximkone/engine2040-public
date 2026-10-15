---
name: shadow-map-checks
description: "Use when a builder wires arena point spot or sun lights or shadow maps and when a reviewer checks the lit frame before it ships"
---

# Shadow map checks

Nine checks from Lengyel Vol 2 ch 8. See render-math-checklist for depth bias (ch 8.3.4). Name the crate per finding. PASS needs a test or capture, never a promise.

1. **Pick the light type by shape.** Ask: is a bulb a point, a cone a spot, the sun an infinite light? Flag: one type stretched to every job, like a point light posed as the sun. (crates/render)
2. **Give every local light a finite reach.** Ask: does attenuation hit zero at a max radius, stay capped near zero, and ease out at the edge? Flag: bare inverse square with no radius, dim over most of its range. (crates/render)
3. **Shape the spot cone and its picture.** Ask: do inner and outer cone angles ease full to zero, with a projected map clamped to a black border outside? Flag: a single hard cutoff that rings or smears edge texels inward. (crates/render)
4. **Treat the sun as direction plus steady brightness.** Ask: is sunlight a constant direction and constant illuminance with no position falloff? Flag: sun distance math copied from point lights, so brightness shifts across the arena. (crates/render)
5. **Tighten each light with a tangent scissor.** Ask: is the scissor rect solved from tangent planes with behind-camera and camera-inside-sphere cases handled? Flag: center plus-minus radius projected, which clips lit pixels or shades the whole view. (crates/render)
6. **Tighten depth with sphere z bounds.** Ask: are min and max depths set from the sphere z extents, mirrored for reverse-Z and clamped to near and far? Flag: full-range lighting with no depth bounds, or one formula for both depth directions. (crates/render)
7. **Filter spot shadows over several taps.** Ask: is the spot map a depth-only pass from the light, with projected-z compare averaged over nearby taps? Flag: one binary tap per pixel, so edges stair-step and shimmer. (crates/render)
8. **Keep cube samples on their face.** Ask: do point shadows use six 90-degree faces with mirrored-face winding flipped and offsets clamped to the chosen face? Flag: offsets that spill across cube edges, drawing wrong-face stripes. (crates/render)
9. **Stabilize sun cascades and blend overlaps.** Ask: are view ranges split into overlapping cascades with fixed texel size, snapped centers, and blended overlap samples? Flag: one giant sun map, or unsnapped splits whose edges crawl as the camera moves. (crates/render)

## Do / Don't
- Do: bound each light by radius and cone, tighten with scissor plus depth bounds, and filter plus stabilize each shadow map. Don't: light the full frame per light with single-tap crawling shadows.
source: Lengyel, Foundations of Game Engine Development Vol 2, ch. 8.1-8.3.3
