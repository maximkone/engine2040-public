# Steal cards: finish-line speedups verified live 2026-10-15

Goal: move G2 (lit 60 fps), G5 (pops), G8 (multiplayer last). Method: every donor file opened live this session; rev pinned; license read. Port = reimplement, no vendored bytes (AGENTS rule 4). All view/presentation or offline; never hashed tick state.

## CARD 1 — critically-damped springs + inertialization (G5 pops)

- PORT(orangeduck/Spring-It-On@4d97b497:common.h, inertialization.c). MIT Daniel Holden 2021.
- Verified: `damper_exact` (rational `fast_negexp`, no `exp`), `damper_decay_exact`, `halflife_to_damping`, `critical_spring_damper_exact:366`, `decay_spring_damper_exact:403`, `spring_damper_exact_stiffness_damping` (critical/under/over); `inertialize_transition` (`off = (src+off) - dst`); `inertialize_update` (decay offsets, `out = in + off`). Donor is scalar-only: quaternion difference must be written.
- Take: decay helper + transition/update + halflife converters, Rust f32, view-only. Clip id + frame stay integers in sim.
- Moves: G5 128 pops/300 frames -> <10; feeds LF10/LF11/LF17/EG18.
- Kill: 0->1 step at 60/30 Hz for 2 s: overshoot >1e-5 kills; end disagree >1e-4 kills; switch first frame vs src last pose >1e-5 kills.
- Rows: feel-spring S, inertialize S (needs spring).

## CARD 2 — WGSL EASU upscale + RCAS (G2 lit 60 fps)

- PORT(pmndrs/upscaler@96184a68:src/shaders/easu.ts, rcas.ts, wgsl.ts, common.ts). MIT Dennis Smolek 2026; shaders derived from AMD FSR1 MIT (notice kept).
- Verified: EASU = faithful WGSL `FsrEasuF` (4-texel gradient edge analysis, 12-tap rotated Lanczos kernel, min/max-of-4 dering, exact `1/x`/`inverseSqrt`, `textureLoad`); RCAS = 5-tap cross + ring min/max + lobe + denoise gate, reads b1 input/b2 exposureTex/b4 alpha, writes b3 rgba16float storage; b0 uniform 96B (`displaySize:8`, `sharpness:64`, `flags:80`; `RCAS_LIMIT=0.25-1/16`); `@workgroup_size(8,8)`. wgpu 30 blockers: none (keep 96B 16-align on b0).
- Take: WGSL kernel bodies; rewire `C.displaySize`, `C.sharpness` (`peak=exp2(-2*(1-s))`), `FLAG_INPUT_REINHARD`/`FLAG_RCAS_DENOISE`. Render 720p -> 1080p, RCAS after. Temporal path later, spatial first.
- Moves: G2/FC3; GS12 dynres feeds lower render scales.
- Kill: FLIP delta improves <10% kills; per-pass ms over bar kills; any tick drift kills.
- Rows: easu-spatial S, rcas-sharpen S (needs easu).

## CARD 3 — GGRS rollback shape (G8 last)

- PORT(gschup/ggrs v0.13.0 API shape; MIT OR Apache-2.0). Verified: `Config` trait, `InputPredictor::predict` (+`PredictRepeatLast`/`PredictDefault`), `InputStatus`, ring-128 `InputQueue` with `first_incorrect_frame`, `set_frame_delay`, `SyncLayer` queues + `SavedStates(max_pred+1)`, `GameStateCell::save/load`, `GgrsRequest::{SaveGameState,LoadGameState,AdvanceFrame}`, sparse-saving flag, `Option<u128>` checksums + `DesyncDetection`, `SessionBuilder` flags.
- Take: request-loop shape only (add_local_input -> poll_remote -> advance_frame -> Save/Load/Advance -> drain events). Keep 8-tick resim + adaptive input delay (FOCUS). Checksums feed HS2.
- Moves: G8 last; MP24 first.
- Kill: seeded loss/jitter fixture: undescribed desync kills; resim outside 8 ticks kills; peer byte drift kills.
- Rows: ggrs-request-loop S, sparse-checksum S (needs request-loop).

## Candidate cross-check (steal-A/B/C 2026-10-15)

- Take later, not this round: memchr SIMD scans (asset/log text paths, S); lz4_flex safe decode (only if packs use LZ4, S); smallvec/bumpalo per-tick scratch (S/M, needs alloc proof); slotmap handles (M); rkyv zero-copy cook (L, needs format decision); snip/rtk output filters for agent context burn (S/M, tooling seat).
- Never: crossbeam-deque (threads on a saturated PC); cranelift/sccache/per-agent target dirs (round-2 reject); cargo-difftests (no license); rtk engine/proxy (BUSL-1.1); Flax core (EULA, idea-only); TUnit/WFC/steering (forge-side, not engine2040).
- arXiv anchors: MoMask 2312.00063 (offline motion bake), neural-microfacet BRDF 2608.09604 (offline LUT), strand hair LOD 2405.10565, frameless rendering 2510.15876 (dynres shape).
