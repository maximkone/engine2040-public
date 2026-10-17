---
name: engine-assets
description: Use when a content row (ASSETS) asks you to find, record the source and terms of, download and cook CC0/CC-BY characters, animations, kits, textures and HDRIs into content/ with a proven import.
---
# engine-assets

Merged on 2026-09-27 (owner GO: fewer, stronger roles). Load it for content rows.

## From the asset-specialist agent

_Content specialist. Finds, records the source and terms of, downloads and cooks CC0/CC-BY characters, animations, kits, textures and HDRIs into content/ with a provenance row per asset (rows CT7-CT9 and map content)._

Follow AGENTS.md and `docs/plan/content.md`. Take one CONTENT board row with
its row text, owned `content/` paths (one tree, master), budgets and the
consumer that must load the result.

1. Find sources with websearch (Quaternius, Poly Haven, ambientCG, Kenney,
   100STYLE and other CC0/CC-BY libraries). Read each license page live;
   CC0 or CC-BY only for anything in git. Mixamo, Fab and similar raw files
   stay in git-ignored `content/local/` and ship only as cooked output.
2. Download into `content/incoming/`, then cook through the engine's own
   paths (CT1 retarget, CT2 mesh cook, texture cook), within the triangle and
   texture budgets. Blender 5.2 is installed for fixes (CT4 loop).
3. Note source URL and SHA in the asset's cook manifest. Any license is usable
   (owner 2026-10-10); no EVIDENCE row.
4. Prove it: the consumer loads it (window build or headless import test),
   cook is identical twice, budgets hold. If no caller loads it yet, report
   OPEN with a named follow-up row (CT15-style) instead of claiming done.

Commit by path (skill `commit-by-path`). Report files, provenance rows, budgets and the proof.

Eyes: render a preview of every cooked asset (Blender 5.2
headless or the engine capture) and open it with the Read tool: silhouette,
scale next to a 1.8 m reference, UV stretch, seams, rig breakage. Refuse what
looks broken even if the import test passes.
