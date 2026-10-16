# Skill-on vs skill-off pairs: msaa-resolve-checks
Three MSAA review cases. Off is habit without the checklist. On names the check, the file or symbol, and the proof it asks for.
## P1: 4x targets with no resolve (checks 3, 4)
Input: a diff in `render` sets `sample_count: 4` on color in `crates/render/src/present/gpu_present/presenter.rs` but leaves depth at 1 and `resolve_target: None` in `crates/render/src/v1_skin.rs`, then presents the multisample texture directly.
With skill: reject. Check 3: color, depth and stencil must all grow 4x with slots per pixel. Check 4: the buffer must average down before post and blit. Proof asked: an edge-pixel run in `crates/render/src/present/tests_unit.rs` plus `frame_split_line` before/after.
Without skill misses: approves the 4x allocation as done, so edges stay stepped and the present reads an unresolved buffer.
## P2: MSAA always on, no A/B proof (checks 5, 6, 7)
Input: a diff in `render` turns on 4x MSAA everywhere with no setting, cites one screenshot, and touches `crates/render/src/present/aa.rs::AaChoice` without using the off path.
With skill: reject. Check 6: keep one switch that picks MSAA off and the fallback. Check 5: edge pixels must blend with MSAA on and step with it off. Check 7: `crates/game/src/bin/arena.rs::frame_split_line` p50 must rise at most 2 ms. Proof asked: on/off edge values plus p50 pair.
Without skill misses: accepts the picture as proof, so a later perf regression and an untestable always-on path land together.
## P3: MSAA mixed into the temporal path (checks 1, 2, 8)
Input: a diff in `render` routes MSAA through `crates/render/src/post.rs::taa_resolve` and the `aa_scale.rs` SMAA fallback to save a pass, and renders the scene at 2x width as full supersample.
With skill: reject. Check 1: 4x multisample shades once per pixel, not a 4x wider frame. Check 2: coverage and depth run per subsample. Check 8: the MSAA resolve stays apart from `taa_resolve` and `select_aa`. Proof asked: `sample_count` sites in `v1_gpu.rs` plus a resolve-before-blit trace.
Without skill misses: calls the mix a speedup, so interiors soften, shading cost jumps 4x, and the temporal history feeds on resolved-plus-unresolved frames.
