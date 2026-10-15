# CT8-HDRI provenance — real skies for the arena kit (2026-09-27)

Replaces the generated 128x64 RGBE placeholder skies with real Poly Haven
4k HDRIs. The generator (`../tools/gen.mjs`) stays as the fallback and is
untouched; re-running it would overwrite these files (orchestrator follow-up:
gate `gen.mjs` to not clobber CC0 drops, then re-baseline `manifest.json`).

License read live 2026-09-27: https://polyhaven.com/license — all assets CC0
(CC0-1.0, "no rights reserved": commercial use allowed, attribution
appreciated, redistribution allowed). Each asset page below also badges CC0.

| Slot (`hdri/`) | Source | Author | File | SHA-256 | Bytes |
|---|---|---|---|---|---|
| `arena_day.hdr` | https://polyhaven.com/a/goegap (midday desert) | Greg Zaal | `goegap_4k.hdr` (`HDRIs/hdr/4k/`) | `c3ddb9860a308449718f1187bf988638f09eb155ad1eed45ac7ab6607eda47c0` | 22384990 |
| `arena_sunset.hdr` | https://polyhaven.com/a/venice_sunset (sunset) | Greg Zaal | `venice_sunset_4k.hdr` (`HDRIs/hdr/4k/`) | `f243a060f84df1437705d2e50bbdbcce66aa55ddb2d913c18eb3f87873213b10` | 22841596 |
| `arena_night.hdr` | https://polyhaven.com/a/moonlit_golf (night) | Greg Zaal | `moonlit_golf_4k.hdr` (`HDRIs/hdr/4k/`) | `66452c13b77cd8dbdd44b8ff9837d304c64d5d095ef3c93b31465024a2fb0d07` | 27968841 |

Direct downloads: `https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/4k/<file>`
(all HTTP 200, 2026-09-27). Originals archived 2026-10-02 to
`C:\engine2040-archive\2026-10-02\incoming\` as `polyhaven_<name>_4k.hdr`
(removed from git: byte-identical to the `hdri/arena_*.hdr` slots above,
re-downloadable from the URLs in this file).

What changed: nothing else. Geometry, textures, placement and the generator
are untouched; only the three `hdri/*.hdr` byte blobs (plus these previews
and `manifest.json` hashes) changed.

Verification (2026-09-27, node stdlib only):
- All three decode as `#?RADIANCE`, `FORMAT=32-bit_rle_rgbe`, 4096x2048.
- Every texel finite: day mean 1.472/1.422/1.420 (hard sun), sunset mean
  0.404/0.416/0.573 (golden horizon), night mean 0.335/0.289/0.281 (moon).
- `preview_day/sunset/night.png` (this dir) are Reinhard+gamma tonemaps of
  the placed files, opened and eyes-checked: clean gradients, sun/moon
  discs, no banding.
- Source-identical twice: independent re-download byte-matches the SHAs
  above (see run report).
- Engine read path: `crates/render/src/v1_ibl_shadow.rs::load_hdri` decodes
  Radiance HDR (existing `hdri_loads_*` tests green); window-build load and
  before/after window captures are UNVERIFIED here (no display server) and
  ride with the captures pilot (Q1/RA14).
