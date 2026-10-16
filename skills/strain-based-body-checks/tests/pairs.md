# Skill-on vs skill-off pairs: strain-based-body-checks

Three soft-body review situations. "Off" is habit; "On" names the check, the failure it stops, and the proof it asks for.

## P1: per-edge distances with one stiffness (checks 1, 2)

Input: a diff adds cape cloth to `crates/physics/src/cloth.rs::ClothWorld` as three distance constraints per triangle sharing one `stiffness_q16`, and the cape shears like rubber on diagonal pulls. Test: a straight hang looks right.

Answer with the skill: Reject. Check 1: carry one element constraint per triangle so shear is visible to the solver, not three edge distances. Check 2: split diagonal stretch from off-diagonal shear with separate stiffness fractions, rest stretch one; prove the diagonal-pull case holds shape with `cargo test -p engine-physics --lib` green and x2-equal bytes.

What a skill-less answer misses: the hang test never pulls diagonally, so edge-only constraints pass while shear runs free, and one shared k can never fix stretch without freezing shear.

## P2: rest inverse per tick, raw bending k (checks 3, 4)

Input: a diff recomputes the rest-state inverse inside the per-tick loop and applies bending stiffness raw per Gauss-Seidel iteration, so doubling `solver_iterations` silently doubles stiffness and a zero-area rest triangle NaNs the cape. Test: default iteration count looks fine.

Answer with the skill: Reject. Check 3: bake the per-element rest inverse once at init, refuse degenerate rest shapes like `ClothWorld::new_grid` refuses bad params. Check 4: store the rest dihedral angle at init and scale bending stiffness by iteration count, capped at one; prove stiffness is iteration-count independent with two iteration-count cases giving the same drape.

What a skill-less answer misses: the per-tick inverse cost at scale, the NaN that only a degenerate triangle triggers, and the iteration-count coupling that turns a perf tuning knob into a behaviour change.

## P3: collide first, hand-moved render verts (checks 6, 8)

Input: a diff resolves `ClothCollider` contacts before the internal constraint passes and writes render verts by hand each frame with a fresh nearest-element search, so the solved constraints drag the cape back into the capsule and frame time grows with mesh size. Test: one small cape on one frame passes.

Answer with the skill: Reject. Check 8: run colliders after the internal passes (push out, kill velocity like `resolve_colliders`), feed surface verts from a barycentric map built once with normals recomputed, and time it against the frame budget. Check 6: clear the correction accumulator each solve, keep index order and a fixed iteration count, and prove x2-equal bytes across two runs.

What a skill-less answer misses: the re-penetration that only shows when constraints fight contacts over several ticks, the per-frame search cost the chapter moves to init, and the run-to-run drift from un-ordered merges.
