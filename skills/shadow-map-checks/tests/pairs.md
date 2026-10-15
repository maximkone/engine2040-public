# Skill-on vs skill-off pairs: shadow-map-checks

Three arena lighting situations. "Off" is the usual first guess; "On" names the check, the fix, and the proof that fails before and passes after.

## P1: torch lights the whole corridor and shimmers at the edge (checks 2, 5, 6)

Input: a diff adds a campfire point light to `crates/render/src/shadow_cluster.rs` with bare inverse-square falloff and no radius, shading the full frame per light. The corridor floor far from the fire still runs the light shader, and nearby walls pop as the camera moves.

Answer with the skill: Bound it. Check 2: give the torch a max radius with attenuation that hits zero there, stays capped near zero, and eases out at the edge instead of pure 1/r2. Check 5: tighten with a tangent-solved scissor rect that handles behind-camera and camera-inside-sphere cases. Check 6: set depth bounds from the sphere z extents, mirrored for the active depth direction. Prove with a capture that lit pixels shrink to the fire region plus `cargo test -p engine-render --lib` green.

What a skill-less answer misses: raising brightness instead of fixing falloff bakes dim-over-most-range waste in, and center-plus-minus-radius scissoring either clips lit pixels or shades the whole view.

## P2: spot gate light rings and leaks past its cone (checks 3, 7)

Input: a diff adds a gate spot light whose cone ends in a single hard cutoff, with one binary shadow tap per pixel. Edges stair-step, a bright ring sits at the cutoff, and the projected picture smears past the cone onto the wall behind.

Answer with the skill: Shape and filter it. Check 3: ease inner to outer cone from full to zero and clamp the projected map to a black border outside the pyramid. Check 7: render the spot shadow as a depth-only pass from the light and average the projected-z compare over nearby taps spaced by texel size. Prove with a close-up capture showing a smooth cone edge plus the tap-average factor logged in the receipt.

What a skill-less answer misses: single-tap shadows can never soften, and without a black clamped border the edge texels smear light where the cone should already be dark.

## P3: sun shadow edges crawl when the camera moves (checks 4, 9)

Input: a diff renders the arena sun (`crates/render/src/shadow_cascade.rs` plus `twin_shadow.rs`) as one giant map positioned from the sun like a point light, and the shadow edges swim whenever the camera pans. Distant cascades waste texels while near-field edges blur.

Answer with the skill: Make it a stable sun. Check 4: treat the sun as constant direction plus constant illuminance with no position falloff. Check 9: split the view range into overlapping cascades with fixed physical texel size, snapped centers, and blended overlap samples. Prove with two captures from slightly moved cameras showing identical texel-stable edges plus `cargo test -p engine-render --lib splits_pinned_as_data` green.

What a skill-less answer misses: a point-style sun position re-fits every frame so texels never line up, and one map either blurs the near field or exceeds what the GPU can hold.
