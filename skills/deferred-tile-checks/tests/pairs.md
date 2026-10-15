# Skill-on vs skill-off pairs: deferred-tile-checks

Three review situations. "Off" is what a reviewer says from habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it prevents, and the capture or ms proof it asks for.

## P1: full-screen soft shadows on every pixel (items 1, 4, 5)

Situation: A builder adds deferred sun lighting in `crates/render` with an 8-tap shadow filter run over the whole frame. The arena still hits 60 fps in an empty scene and the builder calls it done.

Off: "Shadows look soft and the empty scene is fast, so the pass is fine. Ship it."

On: "Not yet. Item 5: costly multi-tap filtering belongs only on shadow-edge tiles, with one tap elsewhere. Item 4: fully shadowed tiles must skip sun math entirely. Item 1: show classify cost versus shade saved for the chosen tile size on the real arena view. Proof: sun-pass ms naive versus tiled plus a capture where edge and solid regions match."

What skill-less misses: it accepts empty-scene fps as proof. It never asks which pixels paid for the soft filter, so the full arena pays 8 taps everywhere.

## P2: sky and backfaces lit like sunlit sand (items 2, 3)

Situation: A builder packs albedo, normal, and motion id into the G-buffer in `crates/render` but gives sky no stable id and lights every non-sky normal with sun diffuse plus spec. Night sky glows and walls facing away shimmer.

Off: "The G-buffer holds normals and the sun lights the scene, so lighting works. Bright sky is a tuning value."

On: "Two findings. Item 2: sky tiles must copy color with no light math, keyed by a stable sky id of zero. Item 3: tiles whose normals face away must skip sun diffuse and spec. Proof: on-vs-off sky capture identical to the sky color, plus a backface view where facing-away tiles go dark."

What skill-less misses: the fast-path rules. Without a sky id and a facing test every pixel runs the sun path, so sky glows and backfaces light.

## P3: pops at shadow range end plus shimmer on edges (items 6, 7, 8, 9)

Situation: A builder ships tiled lighting with a hard shadow-distance cutoff, scattering on every tile, single-sample edges, and one hand shader per tile id with one draw per tile. Distant fighters pop, near sand pays fog, edges crawl, draw count spikes.

Off: "Tiles are in and the picture matches. Cutoff, fog, and edges are minor polish."

On: "Four findings. Item 6: shadow must fade to unshadowed by depth near the draw-distance end, no hard pop. Item 7: scattering runs only past a start distance read from depth. Item 8: edge tiles shade all fragments and compare normals to catch intersections. Item 9: one source per needed id with dead combos folded by table and same-id tiles merged into fewer quads. Proof: range-exit video shows no pop, near-vs-far ms split, edge capture stable x2, draw count near id count not tile count."

What skill-less misses: range, depth-gated cost, edge coverage, and variant control. It sees tiles and stops, so pops, wasted scatter, shimmer, and draw spam all ship.
