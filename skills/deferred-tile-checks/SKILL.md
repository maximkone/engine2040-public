---
name: deferred-tile-checks
description: "Use when a builder wires deferred sun lighting, tile classification, or G-buffer packing in crates/render and a reviewer checks the light pass before it lands."
---
# Deferred tile checks

Nine checks for a deferred sun pass. Name the crate per finding (render, assets, perf). PASS needs a capture or ms proof per item, never a promise.

1. **Tile size pinned.** Ask: is the frame split into small fixed tiles with classify cost measured against shade saved? Flag: fullscreen shade or tiny tiles that cost more than they save. (render)
2. **Sky fast path.** Ask: do sky tiles copy color with no light math, keyed by a stable sky id of zero? Flag: sky paying for sun or shadow taps. (render)
3. **Facing test.** Ask: does each tile skip sun diffuse and spec when normals face away from the sun? Flag: backfaces lit as if facing the sun. (render)
4. **Solid shadow skip.** Ask: do fully shadowed tiles skip sun and shadow taps entirely? Flag: dark tiles still running the sun shader. (render)
5. **Soft edge only.** Ask: is costly multi-tap filtering kept to shadow-edge tiles, one tap elsewhere? Flag: full-screen soft filter on every pixel. (render)
6. **Fade at range end.** Ask: does shadow fade to unshadowed by depth near the draw-distance end? Flag: hard pop when geometry leaves range. (render)
7. **Scatter by depth.** Ask: is scattering applied only past a start distance read from depth? Flag: near tiles paying scatter they cannot show. (render)
8. **Edge fragments.** Ask: do edge tiles shade all fragments and compare normals to catch intersections? Flag: single-sample edges that shimmer or miss cuts. (render)
9. **Few variants, merged tiles.** Ask: is one source built per needed id with dead combos folded by table and same-id tiles merged into fewer quads? Flag: dozens of hand shaders or one draw per tile. (render)

## Do / Don't
- Do: classify once per tile, run the cheapest shade that tile needs, and prove ms saved plus pixels unchanged. Don't: shade every pixel with the full sun plus soft-shadow path.

source: Game Engine Gems 2, ch. 4
