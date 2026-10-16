---
name: asset-budget-checks
description: "Use when a builder adds or changes a potato asset budget in crates/assets: tris, bones, influences, materials and texture pixels stay inside named caps and over-budget files fail with numbers."
---

# Asset budget checks

Nine checks for one budget table. The cooker reports, the caller decides; over-budget never lands silently.

1. **One caps table.** Ask: do POTATO and COMFORT live as one `Caps` struct? Flag: magic 4000/1500 scattered in cooks. (assets) `crates/assets/src/budget.rs::Caps`, `crates/assets/src/mesh_cook.rs::HERO_MAX_TRIS`.
2. **Stats first.** Ask: does `check` take a plain `MeshStats` and return `Vec<BudgetFinding>`? Flag: reading GLB bytes inside the budget test. (assets) `crates/assets/src/budget.rs::check`, `crates/assets/src/mesh_cook.rs::tri_count`.
3. **Tris carry two numbers.** Ask: does each finding name the limit and the measured tris? Flag: a bare refusal with no counts. (assets) `crates/assets/src/import_repair.rs::CODE_BUDGET_TRIS`, `crates/assets/src/mesh_cook.rs::PROP_MAX_TRIS`.
4. **Bones per section.** Ask: is 75 bones per section enforced before skinning? Flag: a 76-bone section that cooks clean. (assets) `crates/assets/src/budget.rs::POTATO`, `crates/assets/src/import_repair.rs::BudgetSet`.
5. **Four influences max.** Ask: are skin weights clamped to 4 influences with renormalized sums? Flag: a 5th weight dropped with no finding. (assets) `crates/assets/src/skin_import.rs::MAX_INFLUENCES`, `crates/assets/src/skin_import.rs::WEIGHT_SUM_EPS`.
6. **One material per skin.** Ask: does each skin keep one material and one shared atlas? Flag: two materials on a crowd member. (assets) `crates/assets/src/cooked.rs::validate_material`, `crates/assets/src/lib.rs::GATES`.
7. **Texture pixels capped.** Ask: is base colour capped at 1024 px with total pixels bounded? Flag: a 2k albedo on a potato fighter. (assets) `crates/assets/src/texture.rs::MAX_TEXTURE_PIXELS`, `crates/assets/src/texture.rs::MAX_TEXTURE_DIM`.
8. **Caller owns the pool.** Ask: does the budget module only report while the caller allocates and refuses? Flag: `new` inside the budget path with no owner. (assets) `crates/assets/src/lib.rs`, `crates/assets/src/bundle_cook.rs::cook_and_write`.
9. **Pure and repeatable.** Ask: is `check` pure with x2 equal bytes and no clock, threads or hash order? Flag: `Instant::now` or `HashMap` iteration in the budget path. (assets) `crates/assets/src/import.rs::gate_results`, `crates/perf/src/vram.rs`.

## Do / Don't
- Do: keep one `Caps` table, report both numbers, and refuse the file at the gate. Don't: sprinkle limits in cooks, clamp silently, or let the budget allocate from the main heap.

source: Game Engine Gems 1, ch. 1.3
