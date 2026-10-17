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

Total on disk: 14,266,776 bytes (about 13.6 MiB) for the five directories, before PROVENANCE.md itself.

Zip download URL pattern: `https://ambientcg.com/get?file=<AssetId>_1K-JPG.zip`. 2K versions exist (stone Bricks097 2K is about 15.9 MB zipped) if a landing wants them; not taken, to keep the bundle small.
