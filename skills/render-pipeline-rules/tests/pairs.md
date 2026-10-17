# Skill-on vs skill-off pairs: render-pipeline-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the file or capture that proves it.

## P1: bind probe reported as a live pass (render)

Situation: A builder adds a fog pass to the live window. The submit returns with 0 validation errors, and the builder reports the pass as GPU-live.

Off: "The fog pass is live. It submitted with 0 validation errors, so it runs on the GPU."

On: "Rule 1: a pass counts only if the picture changes with it on and off. Zero validation errors proves nothing. `crates/render/src/sky.rs:989` is a bind probe that copies a buffer. Capture the frame with the fog pass on and off, and compare the two pixel sets before you call it live."

What skill-less misses: it treats validation as proof. It never checks whether the pixels changed, so a copy passes as a pass.

## P2: blocking readback in the frame (render)

Situation: A builder needs the depth buffer on the CPU for fog. They call `map_readback_f32` inside `present_frame` and poll with `wait_indefinitely` so the data is ready in the same frame.

Off: "Map the buffer and poll with wait_indefinitely. It is the simplest way and the data is correct."

On: "Rule 2: never map, poll or wait on a GPU result inside a frame. Read it two frames later with a non-blocking map. Search `node tools/books.mjs search "gpu readback frame"` and cite [gems1 9.6, 11.3-11.4]. The current wait is `crates/render/src/present/gpu_present/submit.rs:1089-1103`, the stall to remove."

What skill-less misses: the frame stall. The fps drop comes from this wait, and the simplest fix is the one that hurts most.

## P3: cascade sized by eye (render)

Situation: A builder changes the shadow cascades so each one uses the main draw list. They size the cascades from the window width and tune the numbers until the shadows look right.

Off: "Each cascade shares the main draw list. Size them from the screen width and tune until it looks right."

On: "Rule 6: a shadow cascade is a view with its own cull and draw list. Size it from the camera-space frustum diagonal, snap its origin to the texel grid, and clamp depth. Search `node tools/books.mjs search "shadow cascade"` and cite [found2 8.3.3-8.3.4]. The bias numbers belong to render-math-checklist item 8."

What skill-less misses: the separate cull and draw lists, the size rule, and the texel snap. Without the snap the shadows shimmer as the camera moves.
