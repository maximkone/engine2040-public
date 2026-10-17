---
name: strain-based-body-checks
description: "Use when a builder adds or reviews soft-body or cloth deformation (element constraints, bending, volume hold) in crates/physics and when a judge checks the solve before it lands."
---

# Strain based body checks

Eight checks for soft-body solves. Name the crate per finding (physics, destruction). PASS needs element-constraint tests plus x2 equal with `cargo test -p engine-physics --lib` green, never a lively capture alone.

Cut note: the soft-body module `crates/physics/src/cloth.rs` and its tape `crates/physics/tests/cloth_tape.rs` were cut with the physics dead-module wave on 2026-10-16 (they had no production caller). The checks below still name the API shape they held (`ClothWorld`, `ClothParams`, `step_cloth`, `MAX_CONSTRAINTS`); apply them to the module that lands next, and recover the old shape with `git log --follow -- crates/physics/src/cloth.rs` or the sketch in `docs/plan/body-and-break.md`.

1. **One constraint per element.** Ask: does each triangle or tet carry one element constraint solved per pass in a single `ClothWorld` solve, not three edge distances per triangle? Flag: per-edge soup where element shear is invisible to the solver. (physics)
2. **Stretch split from shear.** Ask: are diagonal stretch and off-diagonal shear corrected with separate stiffness fractions (anisotropic per axis allowed), rest stretch defaulting to one, beside the `ClothParams` split? Flag: one k for both, so cloth either never shears or collapses. (physics)
3. **Rest inverse baked once.** Ask: is the per-element rest-state inverse built once at init and stored, with degenerate rest shapes refused like `ClothWorld::new_grid` validates params? Flag: inverse recomputed per tick per element, or a zero-area rest triangle smuggling NaN. (physics)
4. **Bending keeps its rest angle.** Ask: does each bending element store its rest dihedral angle at init, with stiffness scaled by iteration count and capped at one, solved after the element pass like the cut `step_cloth` order? Flag: bending with no rest angle, or raw k per iteration so more iterations silently means stiffer. (physics)
5. **Volume and area held.** Ask: do tets carry a volume constraint (tris an area one) with analytic gradients that sum to zero, inside one fixed constraint cap (the cut `MAX_CONSTRAINTS` bound was 16384), so size survives while shape deforms? Flag: no size constraint, so squeezed elements invert or shrink to nothing. (physics)
6. **Zeroed scratch, fixed order.** Ask: is the correction accumulator cleared each solve and constraints run in index order for a fixed iteration count, giving x2-equal bytes? Flag: corrections leaking across ticks, or hash-order merges that drift run to run. (physics)
7. **Damp along the correction, pin the anchors.** Ask: are velocities damped along the accumulated correction and pinned particles held at current position, never integrated, like the cut `ClothWorld::step_ticks` anchor handling? Flag: energy gained per tick, or anchors integrated then snapped back. (physics)
8. **Collide last, draw from a map.** Ask: do colliders run after the internal passes (push out, kill velocity like `resolve_colliders`), with surface verts interpolated from a barycentric map built once and normals recomputed, timed against the frame budget? Flag: collide-then-solve re-penetration, or a per-frame nearest-element search. (physics)

## Do / Don't
- Do: solve one element constraint per triangle or tet with split stretch and shear stiffness, rest inverses and rest angles baked at init, collide after the solve, and prove x2 equal plus frame ms.
- Don't: pile per-edge distances, share one stiffness, invert rest state per tick, collide before solving, or ship looks-soft with no numbers.

source: Game Engine Gems 3, ch. 12
