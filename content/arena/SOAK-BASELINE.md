# CT61 empty-stands baseline for the CT27 soak

<!-- gen:soak-baseline -->
| Part | Estimate |
| --- | --- |
| Arena kit LOD0 triangles (empty stands, 11 placed actors) | 7472 |
| Kit PBR maps + one day HDRI, VRAM bytes | 223412220 |
| 4 fighter body atlases (shared by 8 fighters), VRAM bytes | 22369620 |
| 8 x hero LOD0 mesh buffers, bytes | 2071872 |
| **Total VRAM estimate (RGBA8 + mips upper bound)** | **247853712 bytes (236.4 MiB)** |
| CPU ms / GPU ms | PENDING (reference GPU run) |

- Cap, potato tier: 268435456 bytes (256.0 MiB): PASS.
- Findings: spatial 0 over 11 actors, verdict clean; cook x2 identical: bundle true, spatial true.
- The 8-fighter day capture with CPU/GPU ms and measured VRAM (adapter + driver line) needs the quiesced reference-GPU run (PF13 rule: CPU load <10%, 3 runs within 10%, 21 samples). None is claimed here.
<!-- /gen:soak-baseline -->
