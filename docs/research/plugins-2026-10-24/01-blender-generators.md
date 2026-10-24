# Blender generator addons and geometry-nodes kits for the engine2040 asset pipeline

Research date: 2026-10-24. Scope: Blender addons and geometry-nodes kits that generate 3D content procedurally, judged for the headless `tools/blender/prop_gen.py` path and for game assets that must also run on weak PCs.

## How to read this

- **Verified**: the product or repo page was fetched and returned the facts listed.
- **Partial**: the page loaded but showed only its title. The facts come from the search snippet and are marked as such. Re-check before relying on them.
- **(seen)**: the entry's repo is already in `docs/research/hunt-2026-10-21/donors-seen.txt`.
- Prices are as listed on the vendor page at fetch time. Superhive and Gumroad prices change often; some pages disagreed with their own search snippets, and both figures are shown where that happened.
- Blender compatibility is the version range the listing states, not one tested by us.
- Searches did not surface the Blender Extensions platform directly, so the free-extension list is thinner than it should be. A follow-up pass on extensions.blender.org would fill that gap.

### donors-seen check

`donors-seen.txt` lists `ahujasid/blender-mcp` (line 126) and `ahujasid/mcp-for-blender` (line 127). Both are the same repo under two names, and both are flagged below as (seen). `dgreenheck/ez-tree` is also listed, but it is a Three.js tree tool, not a Blender addon, so it is left out. `steph4n-gh/voxel-demolition` is also in the list and is not a Blender addon either.

---

## A. Machines, greebles, sci-fi kitbash

### 1. Plating Generator and Greebles (Mark Kingsnorth)
- Link: https://superhivemarket.com/products/plating-generator (also on Gumroad: https://mark-kingsnorth.gumroad.com/l/plating-generator-and-greebles)
- Price / license: $30 on Gumroad (the page's structured data shows $18); GPL. Verified.
- Blender: 2.8 to 5.2 (verified).
- Generates: hull-style panel patterns on mesh faces, with greebles scattered over flat or curved surfaces.
- How it works: geometry-nodes node groups named Panels, Plating and Greebles, added from the node editor. Panel patterns are placed by face.
- Steal idea: a face-driven panel and greeble pass for hard-surface props, driven from `prop_gen.py` by setting node-group inputs and applying modifiers headless.

### 2. GreebleFX (AlbertoFX)
- Link: https://superhivemarket.com/products/greeblefx-non-destructive-greeble-generator
- Price / license: listed at $20 (structured data says $5); GPL. Verified.
- Blender: 2.79 to 2.9 only (verified). Old, so treat as a reference, not a dependency.
- Generates: basic panels and "nurnies" (greebles) on low-poly objects; works best on quad meshes.
- How it works: a non-destructive panel and greeble generator added to an existing object. Not a node system.
- Steal idea: the quad-mesh requirement is a useful rule for our kit meshes; copy the rule, not the code.

### 3. SCI-FIX (Building Generator)
- Link: https://superhivemarket.com/products/scifix
- Price / license: $6.99; GPL. Verified.
- Blender: 4.5 to 5.0 (verified).
- Generates: futuristic towers with roof details, emissive panels, and city-scale layouts.
- How it works: a geometry-nodes toolset.
- Steal idea: cheap sci-fi tower kit to borrow the emissive-panel strip rule for a set of skyline props.

### 4. Ultimate Generators Bundle V2 +50 (Asset Library)
- Link: https://superhivemarket.com/products/ultimate-generators-bundle-v2
- Price / license: $20.30 (shown beside $29); Royalty Free. Verified.
- Blender: 4.0 to 4.5 (verified).
- Generates: a sidebar wrapper over 50+ geometry-nodes setups: city, castle, island builders, fluid and voxel sims, stylized effects, fractures and cracks.
- How it works: each generator is a geometry-nodes graph exposed through a panel.
- Steal idea: a catalogue to mine for ideas and node patterns; the licence is clear enough to study the graphs for our own versions.

### 5. Random Sci-Fi Generator (80.lv article on the add-on)
- Link: https://80.lv/articles/a-new-add-on-for-generating-sci-fi-scenes-in-blender/
- Price / license: $12. License not stated on the article. Verified price and Blender version from the article.
- Blender: 3.0 or later (verified).
- Generates: a full sci-fi environment built from a modular set you model yourself, then adjusted.
- How it works: geometry nodes place your modular pieces procedurally across a layout.
- Steal idea: "model a kit, let the generator scatter it" is the workflow we want the AI to follow for environment dressing.

### 6. Starship Generator (Mark Kingsnorth) (partial)
- Link: https://mark-kingsnorth.gumroad.com/l/starship-generator (also Blender Market and Superhive)
- Price / license: not confirmed (page showed the title only).
- Blender: not confirmed.
- Generates: long-hull starships (Sulaco / Rocinante style). Adds from Add > Mesh; a single seed number changes the design (search snippet).
- How it works: geometry nodes, customisable without code (search snippet).
- Steal idea: seed-in, mesh-out is the contract our prop generator should mirror.

### 7. blender_spaceship_generator (ldo, GitHub)
- Link: https://github.com/ldo/blender_spaceship_generator
- Price / license: free, MIT. Verified.
- Blender: 3.0 or greater (verified).
- Generates: procedural spaceships from a random seed.
- How it works: a Python script, not a node graph. It generates the mesh directly.
- Steal idea: the best fit in this list for us. It is plain bpy, so it can run headless from `prop_gen.py` with a seed and no GUI.

### 8. Pipeify (instant non-destructive pipes and cables)
- Link: https://superhivemarket.com/products/pipeify-instant-non-destructive-pipes-cables-tool
- Price / license: $11.24 on sale (list $14.99); GPL. Verified.
- Blender: 4.2 to 5.0 (verified).
- Generates: bent pipes, tubes and cables from cylinder meshes.
- How it works: a geometry-nodes bend stack on a cylinder base; radii and angles stay editable.
- Steal idea: a bend-along-path pipe rule for sci-fi and industrial props, built from a few cylinders, which keeps triangle counts low.

### 9. GeoPipes (Geometry Nodes Pipes)
- Link: https://superhivemarket.com/products/geopipes (also https://blendermarket.com/products/geopipes)
- Price / license: $1.40 on the page (the same page also shows $2); GPL. Verified.
- Blender: 3.6 to 4.5 listed (the description says 3.0+). Verified.
- Generates: procedural pipes placed by clicking on a surface in Object mode.
- How it works: geometry nodes; pipe points are placed on a surface and extended.
- Steal idea: placement by surface point, useful for pipes on a generated hull.

### 10. OhMyGear (drdanon, free tier) (partial)
- Link: https://drdanon.gumroad.com/l/Free_Gears
- Price / license: free tier; paid "Gears System" at $11+ (search snippet). Page showed the title only.
- Blender: not confirmed; listings in the search are old (2.83 to 2.93 for other gear files).
- Generates: a single gear with tooth count, radius, roundness and twist; paid tier links gears in a synced system.
- How it works: geometry nodes; the free tier exposes the tooth parameters.
- Steal idea: parameter-driven gears as a machine-prop generator; the linked-speed sync rule suits a scripted gearbox.

---

## B. Buildings, cities, ruins, Roman architecture

### 11. Buildify (Pavel Oliva)
- Link: https://www.blendernation.com/2022/07/19/buildify-free-city-creation-add-on-with-geometry-nodes-and-osm/ (Gumroad page: https://paveloliva.gumroad.com/l/buildify, shows the title "Buildify 1.0" only)
- Price / license: free (article); source GPL per a reader comment, not confirmed on the page.
- Blender: not confirmed; the 2022 coverage is old.
- Generates: buildings from modular wall and flat-roof parts, drawn as footprint curves or boxes; OSM import for whole cities.
- How it works: a geometry-nodes library; ships with a low-poly proxy kit that users replace with their own pieces.
- Steal idea: the proxy-kit-swap pattern. Our AI pipeline can supply the kit meshes and let the nodes do the layout.

### 12. BuildingNodes (Durman, GitHub)
- Link: https://github.com/durman/buildingnodes
- Price / license: free; license not shown on the page. Blender 3.0+ (verified).
- Generates: buildings by applying a node-based style to a base mesh (panel-housing style).
- How it works: a node-based building style system on top of a base mesh.
- Steal idea: style-as-node-graph separated from base mass; the base-mass plus style split keeps ruins and towers from one kit.

### 13. Geometry Nodes Architecture Toolkit (Fulm3D)
- Link: https://superhivemarket.com/products/geometry-nodes-architecture-toolkit (also on Fab and ArtStation)
- Price / license: $35.99 on the page (down from $44.99); the search snippet said $31.49. Royalty Free. Verified.
- Blender: 4.5 (verified).
- Generates: walls (length, height, randomness, damage, scale), stone floor tiles, and stairs. Example scenes include ruins, a spiral tower, a low wall and arches.
- How it works: three geometry-nodes generators; the walls generator's damage option is the ruin lever.
- Steal idea: the closest match in this list to a Roman ruins generator. Use the damage parameter with a seeded randomness to make ruined walls for the game.

### 14. Column Gen (Bissieres) (partial)
- Link: https://bissieres.gumroad.com/l/ColumGen
- Price / license: listed as free in the search snippet; the page showed the title only.
- Blender: not confirmed.
- Generates: procedural columns for temples and ruins, with a float-curve profile, presets, and a Destruction Mode that segments and damages the column.
- How it works: geometry nodes; the profile is a float curve.
- Steal idea: column profile as a curve input. Great for Roman colonnades, and the Destruction Mode pattern is a cheap ruin effect.

### 15. SceneCity (Arnaud Couturier)
- Link: https://www.blendernation.com/2019/10/24/scenecity-1-6-released/ (product: https://blenderartists.org/t/scenecity/432348)
- Price / license: paid. The 2019 forum thread said about $79 on Gumroad; this is old and unverified. Not free.
- Blender: 1.6 is the 2019 release; current support not confirmed.
- Generates: procedural cities from user-chosen assets, with grid layouts and district types (commercial, industrial, residential) since 1.3; preset cities since 1.6.
- How it works: a Python add-on that places your own asset library on a layout. It is not a node system.
- Steal idea: the layout-plus-asset-library split, reproduced headless: our kit meshes go in, a city layout comes out.

### 16. The City Generator (blender-addons.org)
- Link: https://blender-addons.org/the-city-generator/
- Price / license: not stated on the page; sold on Gumroad and Superhive. Verified page loads.
- Blender: not stated.
- Generates: procedural cities with layout controls, traffic simulation, night scenes and procedural material adjustments.
- How it works: not confirmed from the page content.
- Steal idea: the traffic and night-scene presets are a cheap way to add life to a city preview, not to the game itself.

### 17. ICity (listing covered by search only)
- Not fetched: the CG Architect article redirects to a chaos.com URL, and the redirect was not followed. Listed only as a lead; see the dropped section.

---

## C. Weapons, props, rocks, trees, terrain, scatter, cables and chains

### 18. Geo-Scatter (Renderjuice)
- Link: https://www.renderjuice.com/blender/extensions/geo-scatter
- Price / license: paid perpetual individual, team and studio licences; prices not listed on the page. Verified.
- Blender: 2.83 to 5.2 (verified).
- Generates: scatter of objects, plants and environment detail across scenes.
- How it works: masks and terrain rules drive where instances land; the verified page lists these controls.
- Steal idea: mask-driven scatter is the cheapest way to dress terrain. Our headless step could write the scatter rules as data and let Geo-Scatter-style masks place instances.

### 19. botaniq (polygoniq)
- Link: https://polygoniq.com/3d/botaniq/ (store: https://polygoniq.gumroad.com/l/botaniq-trees)
- Price / license: not listed on the page; sold on Superhive and Gumroad. Verified page loads.
- Blender: not stated on the page.
- Generates: about 700 tree, grass, flower and plant assets with 50+ scatter presets (verified); the search says v7 became an asset pack usable with the native asset browser.
- How it works: an asset library plus scatter presets, compatible with Geo-Scatter (search snippet).
- Steal idea: asset-library presets are the model for our tree and grass kits. The asset set is a paid library, so take the design, not the files.

### 20. Sapling Tree Gen (built into Blender)
- Link: https://docs.blender.org/manual/en/4.1/addons/add_curve/sapling.html
- Price / license: free; GPL; bundled with Blender (manual page verified; licence per the manual's add-on credits in the search snippet).
- Blender: documented for 3.0 to 4.1 in the manual.
- Generates: procedural trees via a set of parameters, with presets you can export and reload.
- How it works: parametric tree generator from a published method (Weber and Penn); exposed as curve-based settings and presets. Not a node graph.
- Steal idea: presets as data. A tree preset is a small file that bpy can apply headless, which gives us a cheap tree variety without new modelling.

### 21. ANT Landscape (Jimmy Hazevoet, built into Blender)
- Link: https://docs.blender.org/manual/en/4.1/addons/add_mesh/ant_landscape.html
- Price / license: free; GPL; bundled (manual verified).
- Generates: terrain from several noise types, with erosion, mesh displace, and a weight-from-slope vertex group.
- How it works: a noise-driven heightfield in a Python add-on, with sidebar tools to adjust the result.
- Steal idea: "weight from slope" is the mask we want for placing rocks, moss and cliff props on a generated terrain.

### 22. Tissue (Alessandro Zomparelli, GitHub)
- Link: https://github.com/alessandro-zomparelli/tissue
- Price / license: free; GPL per the search snippet (not shown on the repo page we fetched). Verified repo loads; the README points to Blender 5 and a releases page.
- Generates: mesh design operations. The key one is Tessellate, which places a component object across the faces of a generator object and fits each copy to the face shape. Also Dual Mesh and a colour-weight exchanger.
- How it works: Python operators with an Update option that re-runs the tessellation when either object changes.
- Steal idea: a headless "tessellate component on faces" step gives greebles, bolts and rivets on hull faces with a fixed triangle budget. It is probably the single best candidate for our greeble and rivet pass.

### 23. Pipeify, GeoPipes, GeoCables (see entries 8, 9)
- GeoCables: https://amanbairwal.gumroad.com/l/GeoCables. Free per the page title; the page otherwise showed the title only (partial). Search snippet: hanging cables with geometry nodes, Blender 3.0+, v2.0.0 added cable animation.
- Steal idea: a hanging-cable kit for cables between props. Animation is not needed for static assets, so skip that part.

### 24. Simple Chain Physics (shiftyseer) (partial)
- Link: https://shiftyseer.gumroad.com/l/simple-chain-physics
- Price / license: free per search snippet ($0+); page showed the title only. Blender 4.3+ per search snippet.
- Generates: chain or rope physics from two node groups; one links two points on a mesh, the other turns curves into physics-simulated chains.
- Steal idea: chains for props only, baked to static meshes. The physics solver is not something to ship in the game.

### 25. Geo Gen Vault (PaperParachute) (partial)
- Link: https://paperparachute.gumroad.com/l/GeoGenVault
- Price / license: about INR 1,999 per search snippet; page showed the title only.
- Generates: a chains generator and a ropes generator (search snippet), built entirely in geometry nodes.
- Steal idea: a paid second source for chains and ropes if GeoCables does not cover our needs.

### 26. Stone & Rock Geometry Node (3DTudor) (partial)
- Link: https://3dtudor.gumroad.com/l/blender4_stone_rock_geonode
- Price / license: $4.99 standard licence per search snippet; page showed the title only.
- Blender: "4" per the title. Also on Superhive with live UV unwrapping (search snippet).
- Generates: stone paths, slabs and blocks by converting curves into stone blocks.
- Steal idea: block-from-curve for Roman paving and wall courses. Columns and arches must be built separately.

---

## D. Node frameworks and AI-in-Blender

### 27. Sverchok (nortikin, GitHub)
- Link: https://github.com/nortikin/sverchok
- Price / license: free; GPL-3.0. Verified.
- Blender: 2.8, 3.6, 4.5 and 5.1 (verified).
- Generates: node-based parametric geometry (math, mesh, splines, NURBS, cross sections, vector fields, custom Python nodes).
- How it works: a Grasshopper-style node editor; node graphs are data.
- Steal idea: a node graph saved as data and run headless is a clean way to let an AI author parametric props. The graph is harder for an AI to write than Python, so use it only if the Python path runs out.

### 28. Animation Nodes (Jacques Lucke, GitHub)
- Link: https://github.com/JacquesLucke/animation_nodes
- Price / license: free; a licence file is named but the type is not stated on the page. Verified.
- Blender: support range not stated on the page; the search results are 2017 to 2020 coverage.
- Generates: motion graphics and procedural animation, not static asset geometry.
- Steal idea: low value for asset generation. Listed as a framework reference only.

### 29. BlenderMCP / mcp-for-blender (ahujasid) (seen)
- Link: https://github.com/ahujasid/blender-mcp (redirects to https://github.com/ahujasid/mcp-for-blender)
- Price / license: free; MIT. Verified.
- Generates: not a generator itself. It lets an LLM control Blender over MCP: scene inspection, code execution, viewport renders, asset search (Poly Haven, Sketchfab, Poly Pizza) and AI 3D generation through services such as Hyper3D Rodin.
- Seen: listed in `donors-seen.txt` at lines 126 and 127.
- Steal idea: already known. The reusable part is the MCP tool surface for a Blender session, which is relevant if our pipeline ever drives Blender from an agent instead of a script.

### 30. Meshy for Blender (Meshy)
- Link: https://docs.meshy.ai/meshy-for-blender-text-to-texture
- Price / license: not stated on the page; Meshy's web app needs a Pro plan or higher to send models (verified).
- Correction: the page does not generate or texture models in Blender. It imports models from the Meshy web app for 3D-print preparation. The search snippet said text-to-texture; the page does not support that claim.
- Steal idea: not usable as a generator. Dropped from the top list.

---

## E. Physics and destruction

### 31. Cell Fracture (built into Blender)
- Link: https://docs.blender.org/manual/en/4.1/addons/object/cell_fracture.html
- Price / license: free; GPL per the search snippet (not shown on the page we fetched). Bundled.
- Generates: breaks an object into fragments, with a "Cell fracture selected mesh" operator. The manual advises small values first, since large ones are slow.
- How it works: a Python operator run from the Operator Search; sets fragment count and seed.
- Steal idea: pre-fracture rubble props headless and bake the shards as static meshes. Runtime destruction is out of budget on a weak PC, but baked shards are not.

### 32. GeoBreak (Superhive)
- Link: https://superhivemarket.com/products/geobreak
- Price / license: $19 full, $0 Lite trial. Verified.
- Blender: 4.2 to 4.5 (verified).
- Generates: fractures that split an object evenly or within target-object zones, with deformed cut shapes for irregular breaks.
- How it works: a paid add-on; the Lite trial lets us check the workflow before paying.
- Steal idea: a better fracture pattern than Cell Fracture for stone and masonry. Try the Lite trial first, and only buy if the baked output is worth it.

### 33. Fracture helper (JT-a, GitHub fork of scorpion81)
- Link: https://github.com/JT-a/blender-fracture-helper
- Price / license: free; licence not stated on the page. Verified page loads.
- Note: the Fracture Modifier itself is a custom Blender build, not an addon (per the Blender wiki and a devtalk answer in the search). This helper is the add-on companion.
- Generates: helper objects, path and mouse-driven fracture, and smoke, dust and debris effects, in the Tool Panel.
- Steal idea: the helper's debris-effect pattern is a useful reference, but its licence is unknown, so read the code only and do not vendor it.

### 34. FLIP Fluids (rlguy, GitHub; add-on sold on Gumroad and Superhive)
- Links: https://github.com/rlguy/Blender-FLIP-Fluids; https://flipfluids.gumroad.com/l/flipfluids
- Price / license: GitHub repo GPL-3.0 and MIT plus a royalty-free content licence (verified). Search snippet: $79 single-user on Superhive (not confirmed on the Gumroad page we fetched, which showed only the title).
- Blender: 4.5 to 5.2 (verified from the repo page).
- Generates: liquid simulations, whitewater, viscosity and surface tension.
- Steal idea: VFX only. Fluid sims are not a game-asset tool for weak PCs. Listed for completeness, not for the top list.

---

## Dropped, not verified

These came up in search results but the link was not fetched, or the fetch failed, so they are left out of the ranking:

- Procedural Lightsaber Generator (Monty Radford), Superhive. Not fetched.
- Spaceship generator (Huy Khoi Tran Quang), Blender Market and Gumroad. Not fetched.
- Random Flow (blenderguppy), Gumroad. Not fetched.
- GearsBox (nicolagennaro), Gumroad. Only an old Blender 2.93-alpha file in the snippet.
- Procedural Generator Bundle Pack (Superhive), Blender 5.0+. Its listed generators (crystal, droplet, fur, Lego, liquid) are not weapon, armour or prop tools.
- 3D Model Generator (Blender Market, redirected to Superhive). Redirect not followed; text-to-3D backend unknown.
- Rodin (Hyper3D) add-on and 3D-Agent add-on. Not fetched.
- ICity (CG Architect article). The page redirects to cgconnect.chaos.com, and the redirect was not followed.
- Weapon and armour kits (Canopy Games "Making Procedural Weapons In Blender 3", $30; Udemy axe generator course). Course and listing contents not verified.
- Blender Extensions platform entries for gears, greebles and kitbash. Not surfaced by search.

---

## TOP 12 for AI-generated game assets that run on weak PCs

Ranked by: a seed or parameter contract an AI can drive, output that bakes to low-poly static meshes, a licence we can use, and headless (bpy) operation.

1. **blender_spaceship_generator (ldo)**, MIT, seeded plain-bpy generator. Drops straight into `prop_gen.py`. Entry 7.
2. **Tissue (Tessellate)**, free, GPL, Python operators to place greebles, bolts and rivets on faces with a fixed triangle budget. Entry 22.
3. **Sapling Tree Gen**, built into Blender, GPL, presets as data, cheap tree variety with no new modelling. Entry 20.
4. **Geometry Nodes Architecture Toolkit (Fulm3D)**, royalty free, walls with damage for ruins, plus floors and stairs. Closest to a Roman ruins kit. Entry 13.
5. **Plating Generator and Greebles**, GPL, face-driven panels and greebles for hard-surface props. Entry 1.
6. **Buildify**, free, modular kit with a replaceable low-poly proxy kit, city and building layouts. Entry 11.
7. **Cell Fracture**, built into Blender, GPL; bake rubble shards as static meshes. Entry 31.
8. **ANT Landscape**, built into Blender, GPL; weight-from-slope masks for placing rocks and moss on terrain. Entry 21.
9. **Pipeify**, GPL, bend-along-path pipes and cables from cylinder meshes. Entry 8.
10. **Geo-Scatter**, paid; mask-driven scatter that keeps instance counts controllable. Entry 18.
11. **Ultimate Generators Bundle V2**, royalty free, 50+ geometry-nodes setups to study and rebuild as our own. Entry 4.
12. **Random Sci-Fi Generator**, $12; "model a kit, let the generator scatter it" is the environment workflow we want. Entry 5.

## Notes on the list

- Entries 1, 3, 5, 7, 8, 9, 13 and 21 have the most complete verified facts.
- Blender 4.x and 5.x support is the most important check before any purchase; several listings stop at 4.5 or older.
- Paid assets (botaniq, Geo-Scatter, GeoBreak) are design references unless the licence covers shipping their output in the game.
- Nothing here was installed, run or tested on our pipeline. The steal ideas are proposals.
