# Skill-on vs skill-off pairs: asset-budget-checks

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the file, symbol or test that proves it.

## P1: 4100-tri fighter clamped silently (gems1 1.3)

Situation: A builder cooks a 4100-tri fighter for the potato tier. The cooker clamps it to 4000 tris and reports green. The GS15 accept needs a 3900-tri pass and a 4100-tri `tris` finding with both numbers.

Off: "Clamped to 4000 tris and green. The potato budget is met."

On: "Rules 1 and 3: keep one `Caps` table and report both numbers. Search `node tools/books.mjs search \"memory budget middleware allocation\"` and cite [gems1 1.3]. Route the stats through `crates/assets/src/budget.rs::check` and refuse at `crates/assets/src/import_repair.rs::CODE_BUDGET_TRIS`; a 3900-tri fighter passes, a 4100-tri one reports `tris` with 4100 vs 4000."

What skill-less misses: that a silent clamp hides the over-budget file. The budget must fail loudly with both numbers, not take more from the main heap.

## P2: 76-bone section cooks clean (gems1 1.3)

Situation: A builder cooks a crowd section with 76 bones. The skin cooks clean with no finding. The GS15 accept needs a `bones` finding for any section over 75.

Off: "76 bones is one over. The skin still works, so keep it."

On: "Rules 1 and 4: one table owns the limit and the caller refuses. Search `node tools/books.mjs search \"memory budget middleware allocation\"` and cite [gems1 1.3]. Check `crates/assets/src/import_repair.rs::BudgetSet` before `crates/assets/src/skin_import.rs::MAX_INFLUENCES` skinning; report `bones` with 76 vs 75 and refuse the file."

What skill-less misses: who owns the limit. The budget table decides, the cooker only reports; without an owner the 76th bone leaks into every potato build.

## P3: 2k albedo on a potato fighter (gems1 1.3)

Situation: A builder ships a potato fighter with a 2048 px base-colour map. The frame still runs, so the review passes it. The GS15 cap is 1024 px.

Off: "A 2k albedo looks better and still runs. Keep it for potato."

On: "Rules 1 and 7: caps are data and pixels are capped. Search `node tools/books.mjs search \"memory budget middleware allocation\"` and cite [gems1 1.3]. Gate base colour at 1024 px through `crates/assets/src/texture.rs::MAX_TEXTURE_PIXELS` and `crates/assets/src/texture.rs::MAX_TEXTURE_DIM`; report `tex_px` with both numbers and refuse at the import gate."

What skill-less misses: that one 2k map breaks the whole tier. Potato means every fighter fits the same small pool; an uncapped texture blows the pool while each single frame still looks fine.
