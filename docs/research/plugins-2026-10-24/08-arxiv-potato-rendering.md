# arXiv and venue hunt: rendering that runs on a potato PC (2026-10-24)

Scope: real-time rendering papers from 2023 to 2026 (arXiv, JCGT, HPG, EGSR, I3D, SIGGRAPH and SIGGRAPH Asia, CESCG, CVPR, ECCV, NeurIPS, ICLR, ACM TOG, IEEE TVCG) that look expensive but could run on an integrated GPU with 2-4 GB VRAM and 8 GB RAM. Target: engine2040 (Rust, wgpu, browser build).

## How to read this

- [F] = the page was fetched in this pass and the facts come from it (abstract or landing page; PDF bodies were mostly unreadable through the fetcher).
- [S] = the fact comes only from a search-result snippet. The page was not fetched, or the fetch was blocked. Re-check before relying on it.
- [code] = a repo link was seen. [code?] = a repo is listed but not confirmed. No tag = not checked.
- "iGPU cost" = what the source measured, and on which device. No source here measures a 4 GB integrated GPU. Where a device is named, it is given.
- Picks are grouped by topic. The TOP 15 at the end is ranked by look gained per millisecond on an iGPU.

## Bottom line

- 36 picks. 34 were fetched and read (abstract or landing page). 2 are search-only: #12 (MNSS) and #31 (CrowdSplat perceived-LOD study).
- No pick has an integrated-GPU measurement. The closest hard numbers: soft shadows at 423 FPS on a GTX 1070 (#8); a neural far-LOD renderer at 60 FPS on an iPhone XS (#32, [S]); a splat renderer at 116 FPS on a Snapdragon 8 Gen 3 GPU (#19, [S], search preview only).
- Gaussian splats and neural tricks: none is shown to run on a 4 GB iGPU. They are "maybe", as the brief requires.
- Several requested areas had no 2023-2026 hit at all (see Gaps). Older classics are listed under Out of window so nobody re-hunts them.

## Skipped: already in donors-seen.txt or in the plugin docs

donors-seen.txt was read in full. Line numbers are from that file.

- Radiance cascades (3D, WebGPU): wroughtinator/split-radiance-cascades-webgpu (line 2173). This is the donor for Split Radiance Cascades (arXiv 2607.20384, Freeman and Sannikov, 22 Jul 2026). Not a new pick.
- Anti-aliasing and upscaling: iryoku/smaa (1004), GameTechDev/XeGTAO (769), GPUOpen-Effects/FidelityFX-FSR (877), Kits/FidelityFX (1069), pmndrs/upscaler (1506), bevy_anti_alias (256).
- Subsurface and shadows: iryoku/separable-sss (1003), SSSSBlurVS/SSSSBlurPS (1880), selfshadow/ltc_code (1754), TheRealMJP/Shadows (1976), EVSM/EVSSM (659).
- Atmosphere and BRDF: Bruneton/Hillaire (314), ebruneton/precomputed_atmospheric_scattering (605), lut/brdf_fg.hlsl (1160), sobotka/AgX (1834), h3r2tic/tony-mc-mapface (906), KhronosGroup/ToneMapping (1062).
- Impostors, VAT, skinning: ektogamat/octahedral-impostor-component (618), impostors/VAT (969), antzGames Godot VAT plugin (156), chengkehan/GPUSkinning (374), Unity-Technologies/Animation-Instancing (2056), keijiro/HdrpVatExample (1051).
- Texture and memory: bhouston/three-ntc (260), themaister/Granite (1975), francisco-rivas/SparseTextures (739), BinomialLLC/basis_universal (264), KhronosGroup/Basis-Universal-Transcoders (1058), KTX2/Basis (1078), hasenbanck/block_compression (910), mmikk/hextile-demo (1256).
- Real-time GI and ray-traced denoise: CXUtk/ReSTIR_SVGF (482), NVIDIA-RTX/SHARC (1339), NRD/RTXDI (1327), RenderKit/oidn (1642), cdrinmatane/SSRT3 (358), EmbarkStudios/kajiya (625).
- Gaussian splat repos: teamchong/splat (1958), magikerwin/burn-nerf-guided-splats (1166).
- Metrics and particles: NVlabs/flip (1342), gdenes355/flicker_metric_hvei (773), Velvet/ParticleInstancedRenderer (2087), celyk/GPUTrail (360).
- Plugin docs: Unreal Lumen low-end ladder, TSR, Nanite fallback, Mass ISM LOD and Animation Budget Allocator (05, items 1, 2, 12, 13, 23); Bakery GPU lightmapper and Mesh Baker (06, items 23, 26); Toony Colors Pro 2 (06, item 22); Crest Water 5 (06, item 19); Weather FX Free (06, item 20); Amplify Impostors, Rawteous, Dynamic Fog and Mist 2 (03); godot-gaussian-splatting, UnityGaussianSplatting and gsplat (07, items 25-27).

## Out of window (pre-2023, seen while searching, not picked)

- Specular AA: Tokuyoshi and Kaplanyan (JCGT 2021, projected-space NDF filtering); Chermain et al. (2021, glint AA with LEAN mapping).
- Multiple scattering: arXiv 2109.14398 (2021, position-free multiple-bounce for Smith BSDFs); Fdez-Aguera (JCGT 2019).
- Sheen: Zeltner, Burley and Chiang (SIGGRAPH 2022, LTC sheen).
- Shadows and AO: NVIDIA Alchemy AO (2011, contact shadows at about 3-4.5 ms at 720p on 2011 hardware, per snippet); RTSDF (Pacific Graphics 2020; arXiv 2210.04449, 2022; soft-shadow SDFs); stochastic soft shadow mapping (KIT group page, older).
- Probes and GI: Guo et al. efficient light probes (ACM TOG, July 2022); NVIDIA precomputed light-field probes (2017); SDF-DDGI (arXiv 2007.14394, 2020); Neural Radiance Caching (arXiv 2106.12372, 2021); stochastic substitute trees (I3D 2020).
- Anti-aliasing: SMAA (Eurographics 2012; about 1.3 ms T2x and 2.6 ms 4x at 1080p on a GTX 470, per a forum snippet).

## Picks

### A. Global illumination and ambient

1. **GI-1.0: a fast, scalable two-level radiance caching scheme for real-time GI**. Boisse, Meunier, de Dinechin, Bartels, Veselov, Eto, Harada. arXiv 2310.19855, 2023. [F]
   - Key idea: estimates GI on the fly with no scene preprocessing. It joins a cheap probe cache (coarse, slow to react) with reservoir-based resampling (detailed, noisier) in two cache levels.
   - iGPU cost: none in the abstract. No GPU named.
   - Steal for engine2040: GI and ambient term. A probe-cache level for the indirect bounce on the potato tier; keep reservoir resampling off that tier.

2. **TransGI: real-time dynamic global illumination with an object-centric neural transfer model**. Deng, Han, Fang. arXiv 2506.09909, June 2025. [F]
   - Key idea: per-vertex latent features plus a compact MLP decoder carry object-level light transfer. Local light probes record scene radiance, and neighbouring probes share radiance to make probe updates cheaper.
   - iGPU cost: the abstract claims "less than 10 ms to render a frame". No GPU or resolution named. The network is CUDA-based, so it is probably NVIDIA-only.
   - Steal: GI and ambient for moving characters. Copy only the probe-sharing rule until a non-CUDA path exists.

3. **Adaptive Dynamic Global Illumination**. Datta, Goli, Zhang. arXiv 2301.05125, January 2023. [F]
   - Key idea: places probe samples where radiosity changes over time (light or geometry edits). Claims an order of magnitude more probes and refreshes stored lighting without blocking the rest of the frame.
   - iGPU cost: none on the abstract. [S] The ar5iv copy's Figure 1 reports 15.6 ms for this method vs 26.8 ms for Q-DDGI on a modified Bistro Exterior with 192x64x192 probes. GPU not captured.
   - Steal: GI and ambient. Adaptive probe ray budget on a DDGI-style grid, so update cost follows change, not probe count.

4. **Hybrid Rendering for Dynamic Scenes**. Kuznetsov, Diolatzis, Sochenov, Kaplanyan. arXiv 2406.07906, 12 June 2024. [F]
   - Key idea: precomputes the static environment's light transport with neural networks, then renders only the discrepancy between the live scene and that baseline. Moving characters and changing light need only a small correction.
   - iGPU cost: none. The page does not say whether it uses ray tracing or rasterization.
   - Steal: baked-plus-dynamic blend for game levels. Bake the city's static light, render the fighters' residual. The cheapest "Unity-like GI" idea in this set, but unmeasured.

5. **Fast Local Neural Regression for Low-Cost, Path-Traced Lambertian Global Illumination**. Salmi, Csefalvay, Imber. arXiv 2410.11625, 15 October 2024. [F]
   - Key idea: 1 sample per pixel diffuse path tracing, reconstructed by a neural net inside a fast local linear-model denoiser. Ambient occlusion works well as the guide channel.
   - iGPU cost: none. The abstract calls it low-cost without figures. It needs a path tracer, so it is not a raster-only option.
   - Steal: GI denoise for offline bakes or a high-end path. For the potato raster path, keep only the AO-as-guide idea for the SSAO/GTAO pass.

6. **Neural Two-Level Monte Carlo Real-Time Rendering**. Dereviannykh, Klepikov, Hanika, Dachsbacher. arXiv 2412.04634, 2024. [F]
   - Key idea: splits the GI shading integral into a neural radiance-cache estimate plus an unbiased path-traced residual. The cache uses small fully fused networks trained online.
   - iGPU cost: no frame times. A cache evaluation needs roughly 2-25x less compute than one path-traced sample.
   - Steal: not for the potato tier (it needs path tracing). Idea only: a learned cache that turns a multi-bounce estimate into one lookup.

7. **Holographic Radiance Cascades (2D only)**. Freeman, Sannikov, Margel. arXiv 2505.02041, 4 May 2025. [F]
   - Key idea: a one-pass, scene-independent radiance-cascades method for 2D scenes that matches a reference in real time.
   - iGPU cost: RTX 3080 Laptop GPU: 1.85 ms at 512x512, 7.67 ms at 1024x1024 (2D).
   - Steal: 2D only, for UI or a minimap's lighting. Not usable for the 3D engine.

### B. Shadows

8. **Accurate Approximation of Soft Shadows for Real-Time Rendering**. Zerari, Azri, Meadi, Babahenini. RIA (IIETA), online 27 December 2023. [F]
   - Key idea: applies the cascaded-shadow-map idea to area-light soft shadows. The view frustum is split into subfrusta: near ones get more area-light samples and higher-resolution shadow maps, far ones fewer samples and lower-resolution maps. A per-subfrustum PCF kernel size reduces artifacts.
   - iGPU cost: 423 FPS at 800x600 for a three-subfrusta example on a GTX 1070 (a 2016 desktop card). The paper reports about 2x the frame rate of SBFS and FASSALSS.
   - Steal: shadow pass. Soft sun and lamp shadows with a distance-banded sample budget. The best measured look-per-ms item in this set, though not measured on an iGPU.

9. **Resolution-Matched Virtual Shadow Maps**. Matej Sakmary (CTU Prague), supervisor Jiri Bittner. CESCG 2025. [F]
   - Key idea: a page-based virtual layout per cascade. The depth buffer shows which virtual pages are visible, and only those get memory from a pool. Culling fills the pages, and texel density tracks screen density at any range.
   - iGPU cost: no numbers on the page; no GPU named.
   - Steal: shadow pass memory budget. Caps VRAM for cascades on the potato tier, because memory follows what the camera sees. The same author's 2025 CTU thesis covers the paging [S, not fetched].

### C. Anti-aliasing and upscaling

10. **Arm Neural Super Sampling (NSS) v1**. Arm, Hugging Face model card, 2025. [F] [weights]
    - Key idea: a small neural upscaler for mobile GPUs with three quality modes (High, Mid, Low). The demo upscales 540p (960x540) to 1080p, with inputs padded to multiples of eight.
    - iGPU cost: none measured on the card. It claims "up to 50% GPU savings" and warns of sub-10 FPS under fast camera motion. It targets mobile GPUs with neural accelerators (NX). [S] TechSpot and HotHardware give about 4 ms added per frame for 540p to 1080p, but that is a projection for future Mali hardware, not a measurement.
    - Licence: Arm AI Model Community License.
    - Steal: upscale pass, as an optional tier on NX-class GPUs. A potato PC iGPU will not have NX, so FSR1 (already seen) stays the fallback.

11. **Lightweight, Edge-Aware, and Temporally Consistent Supersampling for Mobile Real-Time Rendering**. Sipeng Yang, Jiayu Ji, Junhao Zhuge, Jinzhe Zhao, Qiang Qiu, Chen Li, Yuzhong Yan, Kerong Wang, Lingqi Yan, Xiaogang Jin. ACM TOG, DOI 10.1145/3763348, online 4 December 2025. [F: Crossref metadata]
    - Key idea [S, search snippet, body not read]: a small network for mobile real-time supersampling, with edge awareness and temporal consistency.
    - iGPU cost: not read. The PDF body was not fetched.
    - Steal: upscale pass. A candidate to replace FSR1 on a low tier if the numbers hold. Read the PDF first.

12. **MNSS: Neural Supersampling Framework for Real-Time Rendering on Mobile Devices**. Sipeng Yang and co-authors. IEEE TVCG 30(7), 2024, pp. 4271-4284, DOI 10.1109/tvcg.2023.3259141. [S: UCL repository record from search; the record page returned HTTP 403 to the fetcher]
    - Key idea [S]: changes the sub-pixel sample layout during rasterization, so the reconstruction network can stay small and still keep quality. Targets phones.
    - iGPU cost: none in the snippet.
    - Steal: upscale pass, plus the sample-pattern idea. The raster side is the cheap part: jitter the sub-pixel layout rather than add samples. Fetch the open-access PDF (about 32 MB, per the record) before use.
    - Note: some listings give arXiv 2409.18401 for MNSS. That ID is a different paper (GenesisTex2), so do not cite it.

### D. Materials, BRDF and texture memory

13. **Fast-MSX: Fast Multiple Scattering Approximation**. Rosales, Teimury, Horacsek, Salari, Qin, Bar-Lev, Zhe, Liu (Huawei). SIGGRAPH Asia 2023, Rendering session. [F]
    - Key idea: a cheap estimate of the second scattering bounce for rough microfacet materials, where single-bounce models lose energy and look too dark. Built on Zipin's V-groove idea.
    - iGPU cost: no timing or GPU given; described as low-cost for real time.
    - Steal: material BRDF, metal energy compensation. A shader-only change with no texture or pass cost.

14. **Real-Time Neural Materials using Block-Compressed Features**. Clement Weinreich, Louis de Oliveira, Antoine Houdard, Georges Nader (Ubisoft). arXiv 2311.16121, October 2023. [F]
    - Key idea: neural material features are stored in BC6 textures. Training emulates BC6 decoding, so the output is standard BC6. A small decoder runs in the shader, with smooth scale transitions and arbitrary UV sampling.
    - iGPU cost: none on the abstract; "small memory footprint" and "decoded extremely fast" are qualitative. [S] Figure labels show about 1.77 MB against 2.67 MB for a standard BC texture.
    - Steal: material cook. Compact BC-format materials for kit and fighter textures on 2-4 GB VRAM. The decode cost still needs measuring.

15. **Neural Texture Block Compression (NTBC)**. Shin Fujieda, Takahiro Harada. arXiv 2407.09543, June 2024. [F]
    - Key idea: a network learns to map plain textures into BC formats, so shaders need no change. The cost is paid at texture load time.
    - iGPU cost: no runtime number. The load-time overhead is called "modest" and not timed. Storage savings are up to about 70% in the arXiv abstract; the EG Digital Library record gives up to about 45% [S].
    - Steal: material and texture cook (offline) to fit the 2-4 GB VRAM budget. Zero runtime shader cost. The best memory-per-effort item in this set.

16. **Neural Graphics Texture Compression Supporting Random Access**. Farzad Farhadzadeh, Qiqi Hou, Hoang Le, Amir Said, Randall Rauwendaal, Alex Bourd, Fatih Porikli (Qualcomm AI Research). ECCV 2024, arXiv 2407.00021. [F]
    - Key idea: an asymmetric autoencoder (convolutional encoder, fully connected decoder queried by texel coordinate and mip level), built so GPU texture reads can decode on demand.
    - iGPU cost: none in the abstract. The text claims "much better results than conventional texture compression" with no figures.
    - Steal: texture system, later. Random-access neural decode is the piece a GPU needs. It is the next step after NTBC if neural textures are wanted.

17. **Thread-Efficient Decoding for Neural Texture Compression**. Janarbek Matai, Sho Ikeda, Lukasz Lipski, Takahiro Harada. arXiv 2608.27888, submitted 28 August 2026. [F]
    - Key idea: textures grouped by CLIP embedding share one decoder MLP (trained with gradual decoder freezing). Cuts GPU thread divergence by 25-52% at similar quality.
    - iGPU cost: up to 8.48x speedup vs no decoder sharing, measured on a Radeon RX 9070 XT (a desktop card).
    - Steal: texture decode, only if neural textures are used. The shared-decoder pattern is the copyable part.

18. **Real-Time Neural Appearance Models**. Tizian Zeltner, Fabrice Rousselle, Andrea Weidlich, Petrik Clarberg, Jan Novak, Benedikt Bitterli, Alex Evans, Tomas Davidovic, Simon Kallweit, Aaron Lefohn (NVIDIA). arXiv 2305.02678, 2023 (journal version 2024). [F]
    - Key idea: multi-level learned textures decoded by small neural nets. Layered materials are baked into one compact model. The decoders run inline in ray-traced shaders through hardware tensor math.
    - iGPU cost: the abstract claims "over an order of magnitude faster than non-neural layered materials". No GPU or frame time given. It needs tensor hardware, so it is not an iGPU path.
    - Steal: not for the potato tier. Use it as an offline layered-material bake format once ray-tracing tensor hardware is in reach.

### E. Gaussian splats and neural point rendering on mobile

Note for this section: none of these reports a 4 GB iGPU measurement. The evidence is mobile SoC GPUs and desktop GPUs. Treat each as "maybe" until measured on a potato.

19. **Mobile-GS: Real-time Gaussian Splatting for Mobile Devices**. Xiaobiao Du, Yida Wang, Kun Zhan, Xin Yu. arXiv 2603.11531, March 2026 (ICLR 2026 poster per search). [F]
    - Key idea: drops the depth sort that alpha blending needs, using a depth-aware order-free scheme. A neural view-dependent module fixes overlap artifacts. SH distillation, vector quantization and pruning shrink the model.
    - iGPU cost: the fetched abstract has no numbers. [S] Search preview: 116 FPS on the Bicycle scene on a Snapdragon 8 Gen 3 GPU; 1,098 FPS and 4.8 MB on an RTX 3090 Ti. Confirm in the PDF.
    - Steal: splat pass for captured props or backdrops, if any ship. The order-free blend matters most on iGPUs with weak sorting throughput.

20. **Voyager: Real-Time Splatting of City-Scale Gaussians on Resource-Constrained Devices**. Zheng Liu, He Zhu, Xinyang Li, Yirun Wang, Yujiao Shi, Yiming Gan, Wei Li, Jingwen Leng, Minyi Guo, Yu Feng. arXiv 2506.02774, 2025. [F]
    - Key idea: temporal LoD search (the number of Gaussians entering view stays steady while moving) and preemptive alpha filtering for the rasterization bottleneck.
    - iGPU cost: the abstract gives up to 6.6x speedup and 85% energy saving. No device or FPS named.
    - Steal: city-scale backdrop, with distant districts as splats rather than meshes. Pairs with the city builder's far-LOD plan.

21. **SeeLe: A Unified Acceleration Framework for Real-Time Gaussian Splatting**. Xiaotong Huang, He Zhu, Zihan Liu, Weikai Lin, Xiaohong Liu, Zhezhi He, Jingwen Leng, Minyi Guo, Yu Feng. arXiv 2503.05168, 2025. [F]
    - Key idea: a viewpoint-dependent scene model plus on-the-fly filtering skips Gaussians that do not matter for the view. Contribution-aware rasterization spends work on high-impact splats.
    - iGPU cost: 2.6x faster and 32.3% smaller model (abstract). No device named.
    - Steal: splat pass, same as #19 and #20. Copy the contribution-ordering idea.

22. **MetaSapiens: Real-Time Neural Rendering with Efficiency-Aware Pruning and Accelerated Foveated Rendering**. Weikai Lin, Yu Feng, Yuhao Zhu. arXiv 2407.00435, June 2024. [F]
    - Key idea: a point-based neural renderer for mobile. Pruning favours speed, and foveated rendering lowers quality in the periphery. The page does not mention Gaussian splatting.
    - iGPU cost: the abstract claims an "order of magnitude speedup" over PBNR models. No numbers on the page.
    - Steal: foveated shading as a potato tier (a fixed periphery reduction when no eye tracker is present). Note: this ID was attributed to RTGS in an earlier search; RTGS itself was not verified and is not listed.

23. **Monte Carlo Energy Aggregation for Mobile 3D Gaussian Splatting (Flux-GS)**. Xiaobiao Du, YuAn Wang, Hao Li, Bosheng Wang, Xun Sun, Xin Yu. arXiv 2606.30017, June 2026. [F]
    - Key idea: a Monte Carlo specular energy aggregator condenses high-order SH specular energy into a small latent, so only the low-order SH bands are kept at render time.
    - iGPU cost: "substantial parameter reduction" is claimed; no numbers on the page.
    - Steal: splat appearance cost on mobile-class GPUs. Read after #19.

24. **LightGaussian: Unbounded 3D Gaussian Compression with 15x Reduction and 200+ FPS**. Zhiwen Fan, Kevin Wang, Kairun Wen, Zehao Zhu, Dejia Xu, Zhangyang Wang. NeurIPS 2024, arXiv 2311.17245. [F]
    - Key idea: prunes low-contribution Gaussians with a recovery step, distills spherical harmonics to a lower degree, and applies Gaussian vector quantization.
    - iGPU cost: about 15x average compression; FPS rises from 144 to 237 on 3D-GS. GPU not named in the abstract.
    - Steal: RAM and disk budget for any splat asset. A memory win, not a look gain.

25. **Mini-Splatting: Representing Scenes with a Constrained Number of Gaussians**. Guangchi Fang, Bing Wang. ECCV 2024, arXiv 2403.14166. [F] [code claimed in the paper]
    - Key idea: a constrained Gaussian count, with densification by blur splitting and depth-based reinitialisation, and simplification by intersection preservation and sampling.
    - iGPU cost: no figures in the abstract. [S] ECCV figure labels: about 0.6M Gaussians at 430 FPS vs about 6.1M at 66 FPS for 3DGS, both at 25.2 dB PSNR. GPU not captured.
    - Steal: splat asset cook. A hard Gaussian count per asset tier, since the count is the cost.

26. **Taming 3DGS: High-Quality Radiance Fields with Limited Resources**. Saswat Subhajyoti Mallick, Rahul Goel, Bernhard Kerbl, Francisco Vicente Carrasco, Markus Steinberger, Fernando De la Torre. arXiv 2406.15643, 2024 (SIGGRAPH Asia 2024 per CMU listing). [F] [code] github.com/humansensinglab/taming-3dgs [S]
    - Key idea: budget-controlled optimisation. The Gaussian count grows toward an exact budget fixed before training.
    - iGPU cost: 4-5x smaller model and training time under a budget (abstract). GPU not named.
    - Steal: the splat budget rule for the asset cook (train to a target count).

27. **Compact 3D Gaussian Representation for Radiance Field**. Joo Chan Lee, Daniel Rho, Xiangyu Sun, Jong Hwan Ko, Eunbyung Park. arXiv 2311.13681, 2023. [F] [code] github.com/maincold2/Compact-3DGS [S]
    - Key idea: a learned mask trims the Gaussian count; view-dependent colour comes from a grid-based neural field; vector-quantized codebooks compress the geometric attributes.
    - iGPU cost: over 25x storage reduction vs 3DGS. No FPS or GPU on the page.
    - Steal: splat storage, same role as #24.

28. **HAC: Hash-grid Assisted Context for 3D Gaussian Splatting Compression**. Yihang Chen, Qianyi Wu, Weiyao Lin, Mehrtash Harandi, Jianfei Cai. ECCV 2024, arXiv 2403.14530. [F]
    - Key idea: a binary hash grid gives entropy-coding context over sparse anchor attributes, with adaptive quantization and an adaptive mask.
    - iGPU cost: over 75x smaller than vanilla 3DGS and over 11x smaller than Scaffold-GS. No FPS on the page.
    - Steal: disk and download size for splat assets. Not a runtime cost.

29. **EAGLES: Efficient Accelerated 3D Gaussians with Lightweight EncodingS**. Sharath Girish, Kamal Gupta, Abhinav Shrivastava. arXiv 2312.04564, 2023 (revised September 2024; ECCV 2024 per search). [F] [code?] A GitHub repo under eloquentarduino is listed as the implementation; not confirmed.
    - Key idea: quantized attributes, coarse-to-fine training, and pruning of the least important Gaussians.
    - iGPU cost: storage cut by more than an order of magnitude; memory 10-20x lower (abstract). No FPS or GPU named.
    - Steal: memory and storage for splat assets. Same role as #24 and #27.

### F. Crowds, LOD, impostors and avatars

30. **CrowdSplat: Exploring Gaussian Splatting for Crowd Rendering**. Xiaohan Sun, Yinghan Xu, John Dingliana, Carol O'Sullivan (Trinity College Dublin). arXiv 2501.17792, submitted 29 January 2025, revised 6 December 2025. [F] [code] github.com/RockyXu66/CrowdSplat [S]
    - Key idea: avatar Gaussians fitted to monocular video, crowd synthesis from them, and LoD rendering that cuts the Gaussian count with camera distance.
    - iGPU cost: the abstract names no FPS, count or GPU. [S] A demo at about 31 FPS with 3,500 characters on screen on an RTX 4090. [S] A third-party review lists tiers of about 202k Gaussians within 5 m, 12k at 5-10 m and 3k beyond 10 m; the companion paper's teaser shows 15k where its Figure 2 shows 12k, so the middle tier is unclear.
    - Steal: crowd LOD tiers for citizens and extras. Take the distance-driven budget, not the Gaussian renderer. The desktop numbers say nothing about an iGPU.

31. **Evaluating CrowdSplat: Perceived Level of Detail for Gaussian Crowds**. Xiaohan Sun, Yinghan Xu, John Dingliana, Carol O'Sullivan. arXiv 2501.17085, January 2025. [S]
    - Key idea [S]: pairwise judgments of which of two moving avatars looks more detailed. Factors are motion, Gaussian count and on-screen height (a stand-in for viewing distance).
    - iGPU cost: none (a perception study).
    - Steal: LOD threshold tuning. Use its perceptual tiers to set the crowd LOD budget for #30.

32. **MobileNeRF: Exploiting the Polygon Rasterization Pipeline for Efficient Neural Field Rendering on Mobile Architectures**. Zhiqin Chen, Thomas Funkhouser, Peter Hedman, Andrea Tagliasacchi. CVPR 2023, pp. 16569-16578. [F] [code] github.com/google-research/jax3d/tree/main/jax3d/projects/mobilenerf [F]
    - Key idea: a scene as textured polygons (an opacity bit and feature textures) drawn with the standard z-buffer. A small view-aware MLP in the fragment shader turns the features into colour. No ray marching.
    - iGPU cost: [S] 60 FPS on an iPhone XS for an 800x800 Ficus scene (CVPR 2023 slides). [S] About 10x faster than SNeRG at matched quality (CVPR author Q&A). The project page gives no numbers.
    - Steal: far-LOD props and distant buildings or trees as polygons plus a fragment MLP, drawn on the normal raster path. Raster only, so it fits the potato path. Dynamic characters are out of scope.

33. **From Far and Near: Perceptual Evaluation of Crowd Representations Across Levels of Detail**. Xiaohan Sun, Carol O'Sullivan. arXiv 2510.20558, posted 23 October 2025. [F: metadata only; the PDF body was not readable through the fetcher, so findings are not extracted]
    - Key idea (from the title and references): a perceptual study of how viewers judge crowd characters at different LOD across meshes, image-based impostors, NeRFs and 3D Gaussians.
    - iGPU cost: not extracted.
    - Steal: which representation to show at which distance for citizens. Read the PDF's results before use.

34. **LoDAvatar: Hierarchical Embedding and Selective Detail Enhancement for Adaptive Levels of Detail Gaussian Avatars**. Xiaonuo Dongye, Hanzhi Guo, Le Luo, Haiyan Jiang, Yihua Bao, Jie Guo, Zeyu Tian, Dongdong Weng. arXiv 2410.20789, October 2024 (Virtual Reality, 2025). [F]
    - Key idea: LOD control for drivable Gaussian avatars through hierarchical embedding and selective detail enhancement.
    - iGPU cost: no numbers on the page; no GPU named.
    - Steal: low priority. Only if Gaussian characters are ever used.

### G. Heat, clouds and rasterization

35. **Real-Time 3D Simulation of Heat-Induced Air Turbulence**. Wanqi Yuan, Ethan Chung, Man Luo, Suren Jayasuriya, Huaijin Chen, Jinwei Ye, Nianyi Li. arXiv 2603.02048, 2 March 2026. [F]
    - Key idea: compressible SPH air with heat transport and buoyancy, coupled to a volumetric refractive renderer that bends light along curved paths, with finer steps where the refractive index changes fast.
    - iGPU cost: about 40 fps in the authors' prototype. GPU and particle count not named on the page.
    - Steal: heat-haze look for forges or kilns, as a stylised screen-space distortion. The full solver is too heavy for the potato tier as described.

36. **CuRast: CUDA-Based Software Rasterization for Billions of Triangles**. Markus Schutz, Lukas Lipp, Elias Kristmann, Michael Wimmer. arXiv 2604.21749, 23 April 2026. [F]
    - Key idea: a CUDA software rasterizer for dense, opaque meshes with no precomputed acceleration structure. Small triangles are drawn immediately and the nearest fragment is kept with atomicMin.
    - iGPU cost: peak speedups of 2-5x over Vulkan (unique meshes) and 12x (instanced), per the page. GPU not named on the page. [S] The headline benchmark runs on an RTX 5090.
    - Steal: a software raster path for small triangles (foliage, rubble), only if compute rasterization can run on an iGPU. CUDA-only as published, so port the idea, not the code.

## TOP 15 (look gained per millisecond on an iGPU)

Caveat: none of these has an iGPU measurement. The rank uses the strongest available evidence (measured where given, zero runtime cost where that is the design) and the fit to engine2040's shadow, material, crowd and upscale passes.

1. **#8 Cascaded-subfrustum soft shadows**: measured 423 FPS at 800x600 on a GTX 1070; the fewest assumptions in the set.
2. **#15 NTBC**: zero runtime cost, up to about 70% less texture storage, no shader change. The best match for 2-4 GB VRAM.
3. **#32 MobileNeRF**: raster-only neural far-LOD; 60 FPS on an iPhone XS (slides); code available.
4. **#4 Hybrid rendering**: baked static GI with a dynamic residual. Strong look-per-ms case, unmeasured.
5. **#19 Mobile-GS**: order-free splat blend; 116 FPS on a Snapdragon 8 Gen 3 GPU per search preview (unconfirmed).
6. **#25 Mini-Splatting**: 430 FPS with about one-tenth of the Gaussians (figure labels; GPU unknown).
7. **#11 Yang et al. (TOG 2025)**: mobile supersampling; read the PDF for numbers.
8. **#12 MNSS (TVCG 2024)**: a sub-pixel layout that keeps the upscaler small; read the PDF.
9. **#13 Fast-MSX**: metal energy compensation with almost no shader cost.
10. **#9 Resolution-matched VSM**: shadow memory follows the visible pages.
11. **#30 CrowdSplat**: distance-tiered Gaussian counts for crowds (desktop numbers only).
12. **#20 Voyager**: city-scale splats with a temporal LoD search (up to 6.6x, no device named).
13. **#10 Arm NSS**: a tiny upscaler, but NX hardware only; FSR1 stays the potato fallback.
14. **#14 BCF neural materials**: compact features in BC6, decoded in the shader.
15. **#24 LightGaussian**: about 15x smaller splat assets. A RAM and disk win, not a look gain.

## Gaps (asked for, no 2023-2026 hit in this pass)

- Pre-integrated skin SSS: only the 2011 Penner line and the Jimenez separable SSS (seen).
- Cloth sheen: the Zeltner LTC sheen work is 2022 (out of window); nothing newer.
- Specular anti-aliasing: 2021 items only (out of window).
- SMAA and CMAA improvements: none in window.
- Sky and atmosphere: no 2023-2026 hit (Hillaire 2020 is seen).
- Volumetric fog and froxels: only an undated Gothenburg thesis on froxel TAA; not used.
- Capsule or SDF character shadows and contact shadows: no 2023-2026 hit (RTSDF is 2020-2022; Alchemy AO is 2011).
- Light propagation volumes and SSGI-lite: no 2023-2026 hit. The 2024 Frostbite GI talk (GIBS about 6-7 ms GPU, per snippet) could not be read.
- Foliage, grass and alpha-to-coverage LOD: no 2023-2026 hit.
- Hemi-octahedral impostors and vertex animation textures: nothing new in window (octahedral and VAT repos are seen). A neural-impostor paper (Neuropostors, ICPR 2024) was the closest; its page did not load.
- GPU-driven culling on iGPUs: only CuRast (CUDA) and a 2021 Eurographics skinned-meshlet paper (page returned 403; year unverified).
- Stylised-realistic shading: nothing usable in window (RyShade is a repo, not a paper).
- Lit soft particles: nothing in window.

## Parked or dropped (not verified, or wrong)

- RTGS (mobile Gaussian splatting): the ID search gave (arXiv 2407.00435) is MetaSapiens (#22). RTGS itself was not verified; dropped.
- MNSS as arXiv 2409.18401: wrong ID (the fetch returned GenesisTex2). #12 cites the TVCG record instead.
- Neuropostors (ICPR 2024; neural impostors): the MPI page returned a bot check, and the search index had no record. Dropped.
- Frostbite "Shipping Dynamic GI" (Advances in Real-Time Rendering, 2024): the PDF came back as raw bytes. The 6-7 ms figure is from a snippet. Dropped.
- Radiance cascades vs Unity SSGI and lightmaps (DiGRA CA 2026, KBTU authors): PDF unreadable. Dropped.
- Victoria University of Wellington thesis on sparse radiance probes on a phone (iPhone 7 results): page returned 403; date unknown. Parked.
- Ford and Sung volumetric clouds (SIBGRAPI 2024, UW thesis): not fetched; no GPU named in snippets. Parked.
- Conservative meshlet bounds for skinned meshes (Unterguggenberger et al.): Eurographics page returned 403; year unverified. Parked.
- Dynamic Voxel-Based GI (Computer Graphics Forum 2025; ray tracing plus voxelization): not picked (needs ray tracing).
- Ray-traced stochastic depth map for AO (Eurographics 2024): not picked (ray tracing).
- Proxy-GS (arXiv 2509.24421; occlusion priors for structured 3DGS): not picked (no cost data).
- Sogang 2026 master's thesis on phone GI feasibility: a study, not a method.
- karyon/many-lights-gi (GTX 980: about 6 ms GI; imperfect shadow maps): repo and thesis, date unknown.
- Samsung ENSS (3x FPS/W claim): a product blog, not a paper.
- Patents on neural upscaling, mixed-precision TAA, mobile variable-rate shading and fog-of-war: not papers.

## Notes

- Searches and fetches only. Only this file was written. No other file was touched and no git operations were run.
- The brief asked for measured costs on low-end hardware. The sources do not provide them. The next useful step is a real iGPU run of the top five (#8, #15, #32, #4, #19) through our own harness.
