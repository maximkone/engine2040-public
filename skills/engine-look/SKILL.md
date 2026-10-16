---
name: engine-look
description: Use when tuning or reviewing engine2040 look - ART presets as data plus LOOK REVIEW panel scoring of captures vs reference with per-pass cost. Loaded by the builder for ART packets.
---

# Engine look (from builder)

Follow `docs/VISUAL-TARGET.md` and the P1 bar in `docs/FINISH-RICH.md`.

- **ART (writer, owned paths):** material layers, light rigs, sky and exposure presets, post/grading tables, quality-tier presets as data or small tuning code in owned files. Physically plausible values (albedo ranges, EV100, light units); one preset home per concern, no one-off screenshot tricks.
- **LOOK REVIEW (read-only, review panel):** score captures from the pilot against the reference set and the RT7 path tracer: silhouette, material response, lighting, shadows, temporal stability, composition. Return a score per shot, pass/fail against the P1 bar, and the three highest-value fixes with the file or preset that owns each.
- **Eyes (Muse sees images):** open every capture and its reference with the Read tool and score from the pixels side by side, never from logs or numbers alone. Name the region of the image that owns each fix (e.g. "left wall, shadow acne").
- **Cost rule:** each shot names its per-pass GPU cost against the budget in `docs/VISUAL-TARGET.md`, and once PQ4 lands its FLIP score against the reference; a prettier shot that blows the frame budget fails. Look at real captures only; without an image, visual quality is unverified.
- Research the reference (talks, art breakdowns, CC0 references) with websearch; steal structure, never text.
