# Skill-on vs skill-off pairs: render-math-checklist

Three builder situations. "Off" is the usual first guess; "On" is what the checklist leads to. Pass = the On answer names the root cause (item number), the fix, and a check that would fail before the fix and pass after.

## P1: heavy enemy variant lights wrongly (items 5, 6)

Situation: a variant is made by scaling the model (2, 1, 1). The vertex shader computes `normal_ws = (model * vec4(normal, 0.0)).xyz`. The stretched mesh looks flat on one side and highlights sit in the wrong place. The mesh uses a normal map.

Off: "Normals look unnormalized. Add `normalize()` and raise the normal-map strength; maybe the light is too bright."

On: "Item 5: normals must use the inverse-transpose of the upper 3x3, not the model matrix, because non-uniform scale tilts them off perpendicular; renormalize after. Item 6: also orthonormalize the tangent frame in the pixel shader and keep the handedness sign in tangent.w. Check: for scale (2,1,1) and a 45-degree face, dot(n', M t) is 0 with the inverse-transpose and not 0 with the model matrix; unit test it on the CPU before touching the shader."

## P2: character blend spins the long way and the rig skews (items 3, 4)

Situation: an `anim` blend lerps two rotation quaternions component-wise and renormalizes every few seconds. Blending from a pose at 0 degrees to one at 350 degrees swings the whole way round for a moment. After a long session the model looks slightly sheared.

Off: "Use a smoother easing curve for the blend weight, and clamp the blend speed. The shear is probably an animation asset issue."

On: "Item 4: q and -q are the same rotation. When dot(a, b) < 0, negate b before lerp or slerp, then renormalize, or the blend takes the long arc; also normalize after every multiply, not every few seconds. Item 3: the shear is accumulated rounding in repeated matrix products, so rebuild the transform from position, quaternion and scale each frame. Checks: blend identity with a 350-degree z-rotation at t=0.5 gives about -5 degrees, not 175; after 10k composed frames M^T M stays near identity."

## P3: shadows break after switching to reverse-Z (items 7, 8)

Situation: the camera projection is swapped to the infinite reversed form. The sky and far terrain now look right, but shadow-casting surfaces show heavy acne stripes, and after someone raises the constant bias they show detached shadows under thin walls.

Off: "Keep increasing the depth bias until the acne is gone, and tune near/far if the walls look odd."

On: "Item 7: the switch has four parts that must change together: projection, depth clear (0), depth compare (Greater), and the shadow pass. Item 8: for a reversed projection the constant offset is subtracted and the slope-scaled factor is negative; use a small constant offset (about 2^-19) plus a slope-scaled offset (about 3x the largest depth slope) clamped to roughly 1/128, instead of a big constant. Checks: sun at 80 degrees over a flat floor shows no stripes; a thin wall's shadow touches its base; grep that clear value and compare op sit next to the projection."
