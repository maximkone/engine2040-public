# Arena texture sources (VR2 landing 1)

Raw CC0 PBR sets for the arena kit, 1K JPG, downloaded as-is from ambientCG on 2026-10-17 (owner approved the download in chat the same day). Nothing is cooked yet: cooking to KTX2 is a later VR2 landing. Each directory holds only the image files plus a `LICENCE.txt`; the zips and the .blend/.usdc/.mtlx/.tres/preview files were not kept.

Licence line the site states (https://docs.ambientcg.com/license/): "All ambientCG assets are provided under the Creative Commons CC0 1.0 Universal License."

Normal maps: every kept normal is the `NormalGL` file, OpenGL convention (+Y up, green up). The DirectX variant was dropped; a DX normal is the GL one with the green channel inverted, so no flip is needed for GL-style engines. Maps: Color = sRGB base colour, Roughness and AmbientOcclusion and Displacement and Metalness are linear greyscale.

| asset id | source URL | licence | downloaded | files (bytes) | normal | arena material |
|---|---|---|---|---|---|---|
| Bricks097 | https://ambientcg.com/a/Bricks097 | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-17 | Bricks097_1K-JPG_Color.jpg 831880; Bricks097_1K-JPG_NormalGL.jpg 907097; Bricks097_1K-JPG_Roughness.jpg 386766; Bricks097_1K-JPG_AmbientOcclusion.jpg 355118; Bricks097_1K-JPG_Displacement.jpg 199314; LICENCE.txt 457 | GL (OpenGL) | stone (walls, stands) |
| Ground093C | https://ambientcg.com/a/Ground093C | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-17 | Ground093C_1K-JPG_Color.jpg 563256; Ground093C_1K-JPG_NormalGL.jpg 1058031; Ground093C_1K-JPG_Roughness.jpg 593683; Ground093C_1K-JPG_AmbientOcclusion.jpg 178366; Ground093C_1K-JPG_Displacement.jpg 273026; LICENCE.txt 459 | GL (OpenGL) | sand (arena ground) |
| Planks039 | https://ambientcg.com/a/Planks039 | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-17 | Planks039_1K-JPG_Color.jpg 716040; Planks039_1K-JPG_NormalGL.jpg 575442; Planks039_1K-JPG_Roughness.jpg 301872; Planks039_1K-JPG_AmbientOcclusion.jpg 323215; Planks039_1K-JPG_Displacement.jpg 113423; LICENCE.txt 457 | GL (OpenGL) | wood (rails, stands, props) |
| Metal046B | https://ambientcg.com/a/Metal046B | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-17 | Metal046B_1K-JPG_Color.jpg 706330; Metal046B_1K-JPG_NormalGL.jpg 1272514; Metal046B_1K-JPG_Roughness.jpg 502726; Metal046B_1K-JPG_Metalness.jpg 968767; Metal046B_1K-JPG_Displacement.jpg 674122; LICENCE.txt 457 (no AO map in this set) | GL (OpenGL) | iron (dark, dirty metal: gates, bars, fittings) |
| PavingStones150 | https://ambientcg.com/a/PavingStones150 | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-17 | PavingStones150_1K-JPG_Color.jpg 737414; PavingStones150_1K-JPG_NormalGL.jpg 995888; PavingStones150_1K-JPG_Roughness.jpg 352657; PavingStones150_1K-JPG_AmbientOcclusion.jpg 390645; PavingStones150_1K-JPG_Displacement.jpg 286885; LICENCE.txt 469 | GL (OpenGL) | floor (cobble / flagstone) |
| marble_01 | https://polyhaven.com/a/marble_01 | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-24 | LICENCE.txt 545; marble_01_1K-JPG_AmbientOcclusion.jpg 196544; marble_01_1K-JPG_Color.jpg 305029; marble_01_1K-JPG_NormalGL.jpg 110230; marble_01_1K-JPG_Roughness.jpg 91848 | GL (OpenGL) | marble (white, veined: statues, column drums, revetment) |
| marble_tiles | https://polyhaven.com/a/marble_tiles | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-24 | LICENCE.txt 551; marble_tiles_1K-JPG_AmbientOcclusion.jpg 549779; marble_tiles_1K-JPG_Color.jpg 957660; marble_tiles_1K-JPG_NormalGL.jpg 601407; marble_tiles_1K-JPG_Roughness.jpg 759292 | GL (OpenGL) | marble floor tiles (podium paving) |
| dense_sand | https://polyhaven.com/a/dense_sand | CC0 1.0 Universal (Creative Commons CC0 1.0 Universal License) | 2026-10-24 | LICENCE.txt 768; dense_sand_arm_1k.jpg 685180; dense_sand_diff_1k.jpg 681515; dense_sand_nor_gl_1k.jpg 993025; dense_sand_rough_1k.jpg 476589 | GL (OpenGL) | sand close-up layer (2 m set under the arena floor, 0.25 m tiled detail over it); the `arm` image packs AO in R, roughness in G and metalness in B, so no separate AO or metal file is kept |

Total on disk: 20,676,738 bytes (about 19.7 MiB) for the 8 directories, before PROVENANCE.md itself.

dense_sand was fetched on 2026-10-24 from `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/dense_sand/`
with every served byte count and md5 matching the SW124 row (`diff` 681,515 / `1c3d2c1f…`,
`nor_gl` 993,025 / `ade5aef2…`, `arm` 685,180 / `1310b206…`, `rough` 476,589 / `5e14009b…`).

ambientCG zip download URL pattern: `https://ambientcg.com/get?file=<AssetId>_1K-JPG.zip`. 2K versions exist (stone Bricks097 2K is about 15.9 MB zipped) if a landing wants them; not taken, to keep the bundle small.

Poly Haven sets (2026-10-24): `marble_01` and `marble_tiles` came from `https://api.polyhaven.com/files/<id>`, taking the 1K JPG of each map (`Diffuse` -> Color, `nor_gl` -> NormalGL, `Rough` -> Roughness, `AO` -> AmbientOcclusion). 2K/4K/8K, png, exr, spec, displacement, blend, gltf and mtlx were not kept. Poly Haven states CC0 on `https://polyhaven.com/license`.
