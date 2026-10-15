---
name: render-math-checklist
description: Use when a builder writes or changes render, camera, skinning or animation-blend code that does matrix or quaternion math, normals or tangents, projection and depth, shadow maps, or color and gamma, and when a frame shows mirrored, inside-out, too dark, flickering, skewed or vanishing geometry. Load before editing. Not for tick timing or memory (see engine-subsystem-review).
version: 0.1.0
author: skillworks
---

# Render math checklist

Ten checks. Each names the symptom it prevents and a quick check for a unit test or a one-frame capture. Written for glam (column vectors, column-major) feeding wgpu (depth 0..1) as in the engine2040 workspace; adapt names if the stack differs.

1. **One written convention.** Column vectors, `M2 * M1 * v` (last applied is leftmost), column-major storage; handedness, up axis and NDC depth range are stated once in the render docs.
   Symptom: mirrored scene, wrong rotation direction, translation in the wrong place, matrix transposed in the shader. Check: `translate(5,0,0) * rotate_z(90deg)` on (1,0,0) gives (5,1,0); upload a matrix with translation (7,8,9) and read it back in the shader.
2. **Mirrors flip winding.** A negative determinant of the upper 3x3 changes handedness, so front faces become back faces.
   Symptom: a mesh with negative scale or a mirrored import vanishes or is lit from inside. Check: draw scale(-1,1,1) of a cube; flip the front-face rule for any draw whose determinant is negative.
3. **Drift.** Long products and deep hierarchies lose orthogonality; rebuild from position, quaternion and scale each frame, or re-orthonormalize (Gram-Schmidt).
   Symptom: slow skew or size creep. Check: after 10k composed frames, M^T M stays near identity (about 1e-4 in f32).
4. **Quaternions.** Keep them unit length: normalize after every multiply or integration chain. q and -q are the same rotation, so negate one operand when dot < 0 before lerp or slerp, then renormalize.
   Symptom: a character shrinks or skews; a blend spins the long way round. Check: debug-assert |q|^2 is within 1e-4 of 1; blend identity with a 350-degree z-rotation at t=0.5 and expect about -5 degrees, not 175.
5. **Normals use the inverse-transpose.** Positions and tangents use M; normals use the inverse-transpose of the upper 3x3, then renormalize. Plain M is only safe for rotation plus uniform scale.
   Symptom: lighting wrong on stretched meshes. Check: with scale (2,1,1) and a 45-degree face, dot(n', M t) must be 0.
6. **Tangent frame.** Orthonormalize t, b, n after interpolation, store the handedness sign in tangent.w, bitangent = w * (n x t). Blend two normal maps by converting to slopes (divide xy by z), adding, renormalizing; never average raw normals.
   Symptom: bumps inverted on mirrored UV halves, seams, washed-out detail maps. Check: one quad, bump map, light from +x: the lit side faces the light on mirrored and unmirrored islands.
7. **Reverse-Z.** With a standard projection half the depth range sits between near and 2x near. Use a float depth buffer, infinite far, clear 0, compare Greater; switch all four together.
   Symptom: distant z-fighting, or everything vanishes after a half-done switch. Check: two surfaces 1 cm apart at 500 m resolve; clear value and compare op are grepped next to the projection.
8. **Shadow bias.** Constant offset (about 2^-19 in the shadow projection) plus slope-scaled offset (about 3x the largest depth slope) clamped (about 1/128). Signs flip for a reversed projection.
   Symptom: acne stripes, or detached shadows when the bias is too big. Check: sun at 80 degrees over a flat floor shows no stripes; a thin wall's shadow touches its base.
9. **Color is linear inside.** Albedo and emissive use sRGB formats, data maps (normal, roughness, masks) stay linear; light, blend and filter in linear; encode once at output.
   Symptom: dark and contrasty, or washed out when encoded twice. Check: linear 0.5 must land near byte 188; a black/white checker blurred in linear reads about 188, not 128.
10. **Float hygiene.** No `==` on transformed values; test length > eps before normalizing (zero length gives NaN); clamp acos and asin inputs to [-1,1]; draw camera-relative far from the origin.
    Symptom: NaN black pixels, shimmer far from origin. Check: shift every position by 1e5, no shimmer; assert uploaded matrices are finite.

## Do / Don't
- Do: transform normals with the inverse-transpose and renormalize. Don't: multiply normals by the model matrix because uniform-scale meshes looked fine.
- Do: tag color maps sRGB, data maps linear, and encode once. Don't: add pow(x, 2.2) on top of an sRGB format, or load color maps as plain Unorm.
- Do: change projection, depth clear, depth compare and shadow bias sign together for reverse-Z. Don't: swap only the projection and tune near and far until it looks right.

source: Lengyel, Foundations of Game Engine Development Vol 1 (ch. 1.3, 1.6, 2.1, 2.7, 3.2) and Vol 2 (ch. 5.3-5.4, 6.3, 7.5-7.6, 8.3.4)
