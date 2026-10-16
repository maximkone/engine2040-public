# AI engines: neural layers survey (RES-AIENG-6, 2026-10-01)

Scope: in-engine learned/neural capability that keeps the HASH-LAW sim
(integer/`Fixed` tick math) untouched. Rule: sim = baked/recorded only;
presentation may use anything; model calls never enter the fixed tick.
Stars live-read via `api.github.com` 2026-10-01 unless noted; host
rate-limited (403) at the end, so no further live calls this packet.
GPU numbers are reasoning estimates for the O5 RTX 3050 6GB laptop, not runs.

Dedup (read, not repeated): `COMPARE.md` (no neural row frozen; AI-door
survey reused for mcp-for-blender), `futures.md` FU4/FU6/FU10-FU14
(offline-bake shapes + kill bars reused below), `frontier.md` pillars
(spatial/AI-door consumers), `render-2040.md` (RT6 denoise, FU14 upscale
adapter, VX7 splats missing-file owners reused).

## Survey (29 repos; bake = offline bake-only, safe for hash; pres = runtime presentation, never hashed)

| # | Repo | Stars 2026-10-01 | SPDX (live API) | Runs where | Determinism | RTX 3050 6GB est. |
|---|---|---|---|---|---|---|
| 1 | https://github.com/tracel-ai/burn | ~16.0k | Apache-2.0 | bake (train/offline); runtime pres only | weights nondeterministic; tick never calls | train no; infer small nets ~ms |
| 2 | https://github.com/huggingface/candle | ~21.1k | Apache-2.0 | bake + pres (CPU/CUDA) | same as above; no tick | tiny transformers CPU ~10s ms |
| 3 | https://github.com/sonos/tract | ~3.1k | NOASSERTION (dual MIT/Apache claimed, verify at port) | bake + pres, CPU, no-deps | deterministic-ish CPU fp, still pres-only, never hashed | small ONNX CPU ~ms |
| 4 | https://github.com/pykeio/ort | ~2.5k | Apache-2.0 | pres via onnxruntime (native lib) | external lib = nondeterministic; pres-only | full ORT GPU ~100s MB VRAM |
| 5 | https://github.com/webonnx/wonnx | ~1.8k, ARCHIVED | NOASSERTION (MIT claimed, verify) | pres via WebGPU/wgpu | archived; ideas only | n/a (dead) |
| 6 | https://github.com/robertknight/rten | ~340 | Apache-2.0 | bake + pres, tiny std-only CPU | simplest audit; pres-only, bytes x2 | tiny MLP <1 ms CPU |
| 7 | https://github.com/RenderKit/oidn | ~2.1k | Apache-2.0 | bake (denoise stills) + offline GI | bake-only safe; runtime loop hook REJECTED | stills ~100s ms, 1-2 GB |
| 8 | https://github.com/GPUOpen-Effects/FidelityFX-FSR2 | ~2.1k | NOASSERTION (AMD custom, verify text at port) | pres upscale adapter | pres-only pixels, never hashed; TAAU stays default | 1080p ~1-2 ms |
| 9 | https://github.com/nerfstudio-project/nerfstudio | ~12.0k | Apache-2.0 | bake (NeRF train offline) | bake-only; exports mesh/splat | train OOM-risk on 6GB; export only |
| 10 | https://github.com/graphdeco-inria/gaussian-splatting | ~24.1k | NOASSERTION (custom research license, FLAG) | bake (train, needs big VRAM) | ideas only until license-cleared donor | train needs >6GB; viewer ~60fps |
| 11 | https://github.com/aras-p/UnityGaussianSplatting | ~3.4k | MIT | pres viewer shape (Unity/C#) | pres-only splat-draw idea, reimplement | viewer light, <1GB |
| 12 | https://github.com/ggml-org/whisper.cpp | ~54.0k | MIT | tooling/offline STT sidecar | transcript baked to cues; never tick | tiny/base ~1GB, realtime+ CPU |
| 13 | https://github.com/rhasspy/piper | ~11.3k, ARCHIVED | MIT | bake (TTS voice-bank cook) | wav baked; `LiveSynthRefused` stays | bake CPU fast, <1GB |
| 14 | https://github.com/coqui-ai/TTS | ~46.1k | MPL-2.0 (FLAG, no port) | bake ideas only | ideas ok, no code take | n/a (flagged) |
| 15 | https://github.com/k2-fsa/sherpa-onnx | ~15.1k | Apache-2.0 | bake (STT+TTS+VAD offline) | baked cues/banks only | small models ~1GB |
| 16 | https://github.com/GuyTevet/motion-diffusion-model | ~4.1k | MIT | bake (motion clips offline) | clips cooked to glTF, hash the cook | bake needs GPU hours; infer ~s |
| 17 | search `LearnedMotionMatching`: daihaozhi/learnedMotionMatching, supersanchayrx/LearnedMotionMatching, umsukgod/LearnedMotionMatching-height | 0 stars each | NONE (no license, FLAG) | bake ideas only | no take without license | n/a |
| 18 | https://github.com/guillaumeblanc/ozz-animation | ~3.0k | NOASSERTION (MIT claimed in topics, verify LICENSE at port) | pres sampling runtime | float sampling pres-only; root motion stays Fixed | trivial CPU, KBs |
| 19 | https://github.com/google-deepmind/dm_control | ~4.7k | Apache-2.0 | bake (MuJoCo policy research) | ideas only; MuJoCo never in tick | sim CPU light; not shipped |
| 20 | https://github.com/geoelements/gns | not live-read (host 403); MIT per futures.md live-read 2026-09-30 | MIT (prior verify) | bake advisor (FU4/BX13 integer leg) | integer reimpl only; float weights never tick | offline rollout ms |
| 21 | https://github.com/NVIDIA/physicsnemo | ~3.3k | Apache-2.0 | bake (surrogate train offline) | ideas only; surrogate never authoritative | train big-GPU; infer pres-only |
| 22 | https://github.com/microsoft/TRELLIS | ~13.7k | MIT | bake (text/image-to-3D offline) | glTF baked via CT10 gate; weights never in git | infer needs 12GB+; use hosted bake |
| 23 | https://github.com/VAST-AI-Research/TripoSR | ~7.0k | MIT | bake (image-to-3D, 0.5s class) | same gate as above | infer ~7GB; hosted bake safer |
| 24 | https://github.com/threestudio-project/threestudio | ~7.1k | Apache-2.0 | bake (text-to-3D framework) | same gate; heavy pipeline | needs 12GB+; hosted bake |
| 25 | https://github.com/eloialonso/diamond | ~2.1k | MIT | research (diffusion world model, Atari) | REJECT as engine; playtest-dream ideas only | train big-GPU; n/a ship |
| 26 | https://github.com/tangjiapeng/DiffuScene | ~349 | NOASSERTION (verify at port) | bake (room layouts offline) | layouts validated by spatial solver, never raw | CPU/GPU light infer |
| 27 | https://github.com/ahujasid/mcp-for-blender | ~29.8k | MIT | tooling (external DCC, HS20 mirror) | external tool; journal stays write path | n/a (external) |
| 28 | https://github.com/concept-graphs/concept-graphs | ~962 | MIT | bake (scene-graph offload) | graph baked; SP rows validate | heavy (SAM+LVLM); offload only |
| 29 | FSR3: no `GPUOpen-Effects/FidelityFX-FSR3` (404); lives in the FidelityFX-SDK monorepo (unverified, host 403) | unknown | UNVERIFIED | pres adapter ideas only | same rule as FSR2 | ~2-3 ms est. |

License flags: no GPL found in set; MPL-2.0 coqui-ai/TTS (no port, ideas ok);
NOASSERTION/NONE: verify LICENSE text live at any port (tract, wonnx, FSR2,
3DGS-ref, ozz, DiffuScene, LMM forks); 3DGS-ref + LMM forks carry
non-commercial/no-license risk, ideas only.

## Ranked top 10 (owner crate, bounded task, acceptance; bake vs pres marked)

1. SPIKE (bake): rten tiny-ONNX bake shim beside `crates/assets` cook.
   Task: decode a toy MLP in a test, post bytes+ms. Accept:
   `cargo test -p engine-assets rten_shim` green + bytes x2 + golden unchanged.
2. ADOPT (tooling/bake): whisper.cpp sidecar for the SN5 listen rule.
   Task: transcribe 1 fixture line to baked cues in `crates/audio` tests.
   Accept: `cargo test -p engine-audio whisper_cues` green + bytes x2.
3. SPIKE (bake): Piper offline voice-bank bake (FU10 shape, `crates/audio`).
   Task: 1 line bake posts bytes+ms, `LiveSynthRefused` still errors. Accept:
   `cargo test -p engine-audio voice` green + bytes x2.
4. SPIKE (pres): FSR2-style upscale adapter behind engine TAAU (FU14,
   `crates/render/src/aa_scale.rs`, default untouched). Task: 540p->1080p
   pair posts shimmer+ms on O5. Accept: `cargo test -p engine-render upscale`
   green + adapter line + golden unchanged. License text verified at port.
5. SPIKE (bake): OIDN offline denoise of baked-GI stills (FU11,
   `crates/render/src/gi.rs`, no loop hook). Task: 64x64 pair posts loss+ms.
   Accept: `cargo test -p engine-render neur` green + O5 line.
6. ADOPT (pres): ozz SoA sampling shape beside `crates/anim` (root motion
   stays Fixed; pres-only joints). Task: sample 1 clip, post keys+ms. Accept:
   `cargo test -p engine-anim ozz_sample` green + bytes x2. LICENSE at port.
7. SPIKE (bake): MDM offline motion bake to glTF clips (`crates/anim`).
   Task: 1 prompt bakes N frames via CT10 gate. Accept:
   `cargo test -p engine-anim mdm_bake` green + cook x2.
8. SPIKE (bake): TRELLIS/TripoSR hosted bake to glTF props (`crates/assets`,
   CT10 gated import, weights never in git). Task: 1 object bake posts
   bytes+PSNR vs mesh. Accept: `cargo test -p engine-assets splat` green.
9. REJECT (tick): world-models-as-engine (DIAMOND) + runtime learned physics
   (physicsnemo/dm_control float authority). Ideas feed FU4 offline advisor
   and playtest dreams only. Kill: any model call in tick. No command.
10. REJECT (port): coqui-ai/TTS code (MPL-2.0) + 3DGS-ref code (custom
    research license) + unlicensed LMM forks. Ideas ok; MIT viewers
    (aras-p) only for splat-draw shape. No command; recheck on relicense.

Recheck triggers: ozz/FSR2/DiffuScene LICENSE reads at port; 3DGS-ref or
LMM-fork relicense to allow-list; rten vs tract bake-off numbers from
spike 1 decide the shim; 3050 GPU numbers land only via pilot captures
with adapter+driver+power lines (O5).
