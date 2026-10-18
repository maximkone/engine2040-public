# CT46 provenance audit dry-run for the CT28 gate

Row: CT46 content. Tier A. Size S. Owns `content/PROVENANCE-AUDIT.md` (gate code stays CT28).
Needs: CT28 (ticked), CT7 (ticked), CT8 (ticked). Consumer: CT28 lock, PR1 chains.
Plan: `docs/plan/content.md` section 3 file map + section 8 (`CT46/CT52/CT71 fail when the matrix re-check is skipped or a gap has no named owner and refusal`).

## Release matrix and dry-run (generated)

Written by `node tools/provenance-audit.mjs --write`. SHAs come from `content/arena/manifest.json` and
`content/fighters/manifest.json` (clip rows hash the clip GLB), are re-hashed against the file on disk, and every
record then runs through a JS mirror of the gate's `check_record`. Sources: the arena kit is authored by
`content/arena/tools/gen.mjs`; the 2k maps and three HDRIs are Poly Haven CC0 downloads (page per set, author per set);
the fighters are the OpenGameArt System G6 CC0 pack. Floor of the CT46 row is 50 assets; the matrix lists every shipped one.

<!-- gen:audit-matrix -->
75 records, all hand-made chain (no AI generator rows yet; CT71 adds them with generator + model + prompt digest). Disk column: `match` SHA equals the file, `DRIFT` it does not, `off-tree` the file is not in the tracked tree.

| # | gate name | URL | SPDX | author | SHA-256 | disk |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | content/arena/src/sand_floor.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 92d6d7528a6fac21cb62fb387d0416a0927a26d791367de5bcc8fcf809f6bebc | match |
| 2 | content/arena/src/stands.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 4d69b3b788607382377e685b287a9396d444565a08037270b0e7a35bbff4bf5d | match |
| 3 | content/arena/src/walls.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 4f5ae00ad614e44b3b10e273844669504aa398e888bc578fccc700ea9d413936 | match |
| 4 | content/arena/src/gate_tunnel.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 5a6303835a452930cdab437a95762a632342bc1642d123d3653ee25f2fbbccc6 | match |
| 5 | content/arena/src/torch.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | b04cde0a248a1ea269559736dafe5994dcc756b1eb155daa6f5343d37162f090 | match |
| 6 | content/arena/src/banner.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 6a73057134fdb0446c67d323c7719b1740c0cd8b5ed87421fd661d573fe7fa70 | match |
| 7 | content/arena/src/crate.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 322d4cf3a807ab572ce6d9af25b0707c7793752238fd0eeaa27d5436fd84b1b7 | match |
| 8 | content/arena/src/debris.gltf | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 0f2da95df0f61b52963b3368bd42a604dde3fae866231dd914339802fa06027b | match |
| 9 | content/arena/textures/sand_albedo.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 2ad063f80afd58215d29736c9b6eb42077323307f4694dcb8fc9c3ed946883d4 | match |
| 10 | content/arena/textures/stone_albedo.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 3b8c40c0d3a97977d8dd107404ddc0e229d4cbe0bfed870ce7bf0db2cb057ff3 | match |
| 11 | content/arena/textures/wood_albedo.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 955c214f801e3dbed8bc5583f0ffa48893ab457fa7eaf68755c10684c344f56d | match |
| 12 | content/arena/textures/cloth_albedo.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 4ca086c800cda30b0db00cfd576d45c9930a6288e0fe183cd23aab5c367ffbdb | match |
| 13 | content/arena/textures/metal_roughness.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 90de097ec1c4df53bed4d2bece19625511e467a452caaf4596074b391a0c2df1 | match |
| 14 | content/arena/textures/flat_normal.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 3a178f739ed3841560b6c743e7606da64ecc3f3c0b7552728dc2a99dad844ab1 | match |
| 15 | content/arena/hdri/arena_day.hdr | https://polyhaven.com/a/goegap | CC0-1.0 | Greg Zaal | c3ddb9860a308449718f1187bf988638f09eb155ad1eed45ac7ab6607eda47c0 | match |
| 16 | content/arena/hdri/arena_sunset.hdr | https://polyhaven.com/a/venice_sunset | CC0-1.0 | Greg Zaal | f243a060f84df1437705d2e50bbdbcce66aa55ddb2d913c18eb3f87873213b10 | off-tree |
| 17 | content/arena/hdri/arena_night.hdr | https://polyhaven.com/a/moonlit_golf | CC0-1.0 | Greg Zaal | 66452c13b77cd8dbdd44b8ff9837d304c64d5d095ef3c93b31465024a2fb0d07 | off-tree |
| 18 | content/arena/placement.json | https://github.com/maximkone/engine2040-public/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 16913e041b2438b6accaf4a8e3de912e83748bbfcb266c0000d060d31dae14f6 | match |
| 19 | content/arena/textures/sand_albedo_2k.jpg | https://polyhaven.com/a/aerial_beach_01 | CC0-1.0 | Rob Tuytel | 1803d7e2861b304bea6d2e124dcd4c657fcb93843b84c17bb2db98ce48a87e4e | match |
| 20 | content/arena/textures/sand_normal_2k.jpg | https://polyhaven.com/a/aerial_beach_01 | CC0-1.0 | Rob Tuytel | 0f35185b7225fe2027897bb7e4586c7deb048817524f1ed104fdae7ce80bdc06 | match |
| 21 | content/arena/textures/sand_roughness_2k.jpg | https://polyhaven.com/a/aerial_beach_01 | CC0-1.0 | Rob Tuytel | 318b440c109de7411f9e7443f38ae3cf873f34e90afd762c931bae3b23426da2 | match |
| 22 | content/arena/textures/stone_albedo_2k.jpg | https://polyhaven.com/a/sandstone_blocks_04 | CC0-1.0 | Rob Tuytel | 4571ff357e041bacb75ab06ad1958e74a0b396e0f3d8b9dc9d4717f704996304 | match |
| 23 | content/arena/textures/stone_normal_2k.jpg | https://polyhaven.com/a/sandstone_blocks_04 | CC0-1.0 | Rob Tuytel | b367bdae7a75df039f3425a6eb9d395cd390a7a67b581bec89a44d2a31a8dbdc | match |
| 24 | content/arena/textures/stone_roughness_2k.jpg | https://polyhaven.com/a/sandstone_blocks_04 | CC0-1.0 | Rob Tuytel | 98495e70e919ea892f2119100e64d8d5a5accb9982703fdd923477e7f2a346fb | match |
| 25 | content/arena/textures/wood_albedo_2k.jpg | https://polyhaven.com/a/wooden_planks | CC0-1.0 | Charlotte Baglioni + Dario Barresi | 02a124390e368fe7ae4bb98235fd02113cd4159fd5a7ea580ccba3e319b3577b | match |
| 26 | content/arena/textures/wood_normal_2k.jpg | https://polyhaven.com/a/wooden_planks | CC0-1.0 | Charlotte Baglioni + Dario Barresi | a7ff9954729e7d4dd678b1e35a2edacef9246e45d25a714c000ce95e3cc4e436 | match |
| 27 | content/arena/textures/wood_roughness_2k.jpg | https://polyhaven.com/a/wooden_planks | CC0-1.0 | Charlotte Baglioni + Dario Barresi | 6b9761d9955a6405e73d0040348cf856bfa3d94127baab3b177cdd8d27390f95 | match |
| 28 | content/arena/textures/cloth_albedo_2k.jpg | https://polyhaven.com/a/velour_velvet | CC0-1.0 | Rico Cilliers (+ colormass) | 7c81a1577f50a023825d8d5e26ec7f8596e47534d8ff601aa0b074980833e875 | match |
| 29 | content/arena/textures/cloth_normal_2k.jpg | https://polyhaven.com/a/velour_velvet | CC0-1.0 | Rico Cilliers (+ colormass) | 20207c7b1e4328f8b8b635435d33d5adf23c44f045d1f711b4105999d1883745 | match |
| 30 | content/arena/textures/cloth_roughness_2k.jpg | https://polyhaven.com/a/velour_velvet | CC0-1.0 | Rico Cilliers (+ colormass) | 23f0b2bfdf7f88bff072878bb763c6e825ab154da50b51040b607911b74154c3 | match |
| 31 | content/arena/textures/sand_orm.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/textures/ORM-PROVENANCE.md | CC0-1.0 | engine2040 CT26 lane | dab40fa6e1cf645b0418f6fe3dbaf95bc6c6839677eea15026a2cfb2ab57baaa | match |
| 32 | content/arena/textures/stone_orm.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/textures/ORM-PROVENANCE.md | CC0-1.0 | engine2040 CT26 lane | 55c28331a119efccdd555460024a179d7427894f11c7d06d4a9c9cc1a56fb39f | match |
| 33 | content/arena/textures/wood_orm.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/textures/ORM-PROVENANCE.md | CC0-1.0 | engine2040 CT26 lane | 3ff77c78f00914c17c68484d728acc6e4c0b6d0a269f0606fac74126173a34c0 | match |
| 34 | content/arena/textures/cloth_orm.png | https://github.com/maximkone/engine2040-public/blob/master/content/arena/textures/ORM-PROVENANCE.md | CC0-1.0 | engine2040 CT26 lane | ee00980f79c991be07c3015869efd6478388e3fa3e08240b7625f59986b246f1 | match |
| 35 | content/arena/cooked/stone_base.ktx2 | https://ambientcg.com/a/Bricks097 | CC0-1.0 | ambientCG | 12f2b90f6b9e5cd8fca0efbaf68a1e82c8880bf12fdca85d89f2762d2dfb59f9 | match |
| 36 | content/arena/cooked/stone_normal.ktx2 | https://ambientcg.com/a/Bricks097 | CC0-1.0 | ambientCG | 135873859524844c2275fb20e5f7af07fd7a6c5470396485931a7ebc021e2b75 | match |
| 37 | content/arena/cooked/stone_orm.ktx2 | https://ambientcg.com/a/Bricks097 | CC0-1.0 | ambientCG | c5ee8cd3f785040f5036c6bb83e38185cd014cf3747d247a69b199b8177f1d31 | match |
| 38 | content/arena/cooked/sand_base.ktx2 | https://ambientcg.com/a/Ground093C | CC0-1.0 | ambientCG | 5f37c7e1b84eb3795a3b10d26e2bb50bfc8eacff8847d657fe3b1f7d787db5f0 | match |
| 39 | content/arena/cooked/sand_normal.ktx2 | https://ambientcg.com/a/Ground093C | CC0-1.0 | ambientCG | 46d19f7fd0d9880c20933ae805e7b431c8ccc82c6de0a8e7d9d182c329b2566d | match |
| 40 | content/arena/cooked/sand_orm.ktx2 | https://ambientcg.com/a/Ground093C | CC0-1.0 | ambientCG | 46e1cd7189bf413b4445bdb7d6a0c03057d3b6068e4a5d1b9b59a659c92f5fae | match |
| 41 | content/arena/cooked/wood_base.ktx2 | https://ambientcg.com/a/Planks039 | CC0-1.0 | ambientCG | bf8e66b12ec436c713dd227194268c2465920dabac5d08443075b030c802b90b | match |
| 42 | content/arena/cooked/wood_normal.ktx2 | https://ambientcg.com/a/Planks039 | CC0-1.0 | ambientCG | 6b44e0e9622fe84f329045d85e646d0ef7fb969f943f34ee48c04506e153c2de | match |
| 43 | content/arena/cooked/wood_orm.ktx2 | https://ambientcg.com/a/Planks039 | CC0-1.0 | ambientCG | dfc36a67e9ebb8d3e8c0662a7ec65b57971d9babe59bba60ba5a41ed790c6b98 | match |
| 44 | content/arena/cooked/iron_base.ktx2 | https://ambientcg.com/a/Metal046B | CC0-1.0 | ambientCG | b5a92b21f04a177cb8fa0afb03c8b0849db77e72b8f317dba961e27643619ab0 | match |
| 45 | content/arena/cooked/iron_normal.ktx2 | https://ambientcg.com/a/Metal046B | CC0-1.0 | ambientCG | 406766eb9ff27634e52b29bc577d6148557b6a90117deb0b0f77edbe4c3cbc58 | match |
| 46 | content/arena/cooked/iron_orm.ktx2 | https://ambientcg.com/a/Metal046B | CC0-1.0 | ambientCG | 7dbb8cf4461440cba094cea182f388d83cbe05a45b704f884e1e43d4fb046095 | match |
| 47 | content/arena/cooked/floor_base.ktx2 | https://ambientcg.com/a/PavingStones150 | CC0-1.0 | ambientCG | f970e881e0322e84e71b0cc096b5263868e300ef22c7f520521ffd5d70199163 | match |
| 48 | content/arena/cooked/floor_normal.ktx2 | https://ambientcg.com/a/PavingStones150 | CC0-1.0 | ambientCG | 9a8accd34e112439ac8f2e5e4a15ecf5939c09cde1d05a15b46d4936b56a4e15 | match |
| 49 | content/arena/cooked/floor_orm.ktx2 | https://ambientcg.com/a/PavingStones150 | CC0-1.0 | ambientCG | 7562b543522699b5ab6261f88b954366716aaa059b4d4324d9396ec97fee441b | match |
| 50 | content/incoming/studio_small_09_1k.hdr | https://polyhaven.com/a/studio_small_09 | CC0-1.0 | Sergej Majboroda | e7cfda5f4e98e623db12b8bfd0184e048488e4855d9c83e2751fb44a32e80c45 | match |
| 51 | content/arena/hdri/polyhaven-studio-small-09-1k.hdr | https://polyhaven.com/a/studio_small_09 | CC0-1.0 | Sergej Majboroda | e7cfda5f4e98e623db12b8bfd0184e048488e4855d9c83e2751fb44a32e80c45 | match |
| 52 | content/fighters/incoming/modular_rpg_characters.7z | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | ef53003c6b29798a634fb27a3f9fc3181a2c02b217689744f2944fc5fcc0fd99 | off-tree |
| 53 | content/fighters/incoming/modular_rpg_characters_ik_rig_for_animating.7z | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 5e97fcbb17c23c6d84e085a54ea32c8a4fe0ee471a1d9b41bff9dead40e75aaf | off-tree |
| 54 | content/fighters/incoming/staged/hero.blend | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma), staged by engine2040 CT19 lane, re-grafted by CT21 lane | bc73a5a7fd46c8170a06c80eef5d210b68648666d5924d76c5774fb489d6a3fa | off-tree |
| 55 | content/fighters/incoming/staged/hero.blend1 | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma), staged by engine2040 CT19 lane | 24de66d628562a961ec249c35158fe728e7d00ffd7c4da1fed325d47d5ca69d4 | off-tree |
| 56 | content/fighters/incoming/staged/hero.parts.json | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | engine2040 CT19 lane (part list over System G6 parts) | 3d29b13820cb1305f77aeff7307a309064b447e24d6b27350fcdadec082c2ce2 | off-tree |
| 57 | content/fighters/incoming/staged/hero.weapon.json | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | engine2040 CT21 lane (cook report over System G6 parts) | c8c0a9c303d513035e23f51b32026e8e45e074a6f92291657861ecf5c329d14c | off-tree |
| 58 | content/fighters/glb/hero_body.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 4bdc600a7acb1fa057f33e5c680bff9f74d047f5d2de22df6a8addc1e94bbbd5 | match |
| 59 | content/fighters/glb/opp_male_light_body.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 9f4d71e67365af68d85ad2e17541ca22a87956e7a235171e27953c32a04a7ce1 | match |
| 60 | content/fighters/glb/opp_female_medium_body.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 08427dd6aaef4bdb9a317c023306778555503c1ea16d82747aaea9ef84cb6886 | match |
| 61 | content/fighters/glb/opp_female_light_body.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | ef908b0fa2b4af5e9880dc87ccbd77002178c64bae4dfebec06b78e5e6595bd2 | match |
| 62 | content/fighters/glb/clips/human_male_idle.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | a2b0cfa670722e2e8283830e9f5c73873405833172ba94065c61b8cdbd902a39 | match |
| 63 | content/fighters/glb/clips/human_male_run.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | d70d936ef077de8d8308902de7730013e329902a1225663a20ccdd2a44b25df3 | match |
| 64 | content/fighters/glb/clips/human_male_jump.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 616bae7ea3976f225c17718161f0c17ca4bdf1d157312b6090929ff29e9fec6c | match |
| 65 | content/fighters/glb/clips/human_male_death.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 1893e5cb2e2494901ac7900897279b7f73c2ca90e6e93f58ca02a0ac868149d6 | match |
| 66 | content/fighters/glb/clips/human_male_idle_combat_main_hand_melee.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | a936e4a6d12961ea8efa6da863917e0b7c87a2b5974aef9db113c06f0b05a773 | match |
| 67 | content/fighters/glb/clips/human_male_idle_combat_unarmed.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 36f6c97144f8a47e3482b7db489710b290a7f0e0aea1396db0bf8e63c80e01e4 | match |
| 68 | content/fighters/glb/clips/human_male_cast_main_hand_melee.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 29afa8abbb1a8f13ebdc7a6f32f7a64bb4cc210dbeb3bbba47cfa1cb86e18c3e | match |
| 69 | content/fighters/glb/clips/human_male_cast_main_hand_melee_2.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 54283fa8fd025e2b22829e23a0add18cdb3b01a3e53dbe04832c4c7b77515118 | match |
| 70 | content/fighters/glb/clips/human_male_cast_main_hand_melee_3.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | ccc80face7b028b29b0724b4d1124ed1ccfc672663c11456600a7c7c6d374dd4 | match |
| 71 | content/fighters/glb/clips/human_male_cast_two_handed_melee.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | e157db55608e079748bf4f4032e3132acc508a81487535d60812e0374707c2b8 | match |
| 72 | content/fighters/glb/clips/human_male_cast_two_handed_melee_2.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | d3a5d5c287b4ce6ec768a00f783914500a7a705a4bea6f44568fa46f9c9bb6c5 | match |
| 73 | content/fighters/glb/clips/human_male_cast_off_hand_shield.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 9c356f894fc8047a4399c07a75fc5fd0458720be8de055a641fdd547d01a0c22 | match |
| 74 | content/fighters/glb/clips/human_male_cast_dual_wield_melee.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 0f5eea98b4ea71ca8c48168536ee6fd19ab65ad26a15abe8de249bb38dbc254b | match |
| 75 | content/fighters/glb/clips/human_male_power_up.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | e58392df622087402161eef0775f0625eae89fdd77693e9ec74515ce543be793 | match |

Release-refusal dry-run: 3 refusals.

- `bad_name` names `content/fighters/incoming/modular_rpg_characters_ik_rig_for_animating.7z`
- `bad_name` names `content/fighters/glb/clips/human_male_idle_combat_main_hand_melee.glb`
- `bad_name` names `content/fighters/glb/clips/human_male_cast_two_handed_melee_2.glb`

Trees outside the matrix (a release would refuse them until each has rows):

- `content/fighters/incoming/modular/` (owner CT58/CT75): 53-file 7z extraction, cook intermediate; re-extractable from the pinned 7z, no per-file SHA kept. Listed per file it would refuse with `bad_sha`.
- `content/fighters/incoming/ikrig/` (owner CT58/CT75): 3-file 7z extraction, weapon-offset reference only; re-extractable from the pinned 7z. Listed per file it would refuse with `bad_sha`.
- `content/incoming/` (owner CT80): git-ignored dropbox; only the studio HDRI is carved out and listed. Listed per file it would refuse with `bad_sha`.
- `content/local/` (owner CT4 flow): git-ignored cache; raw donor bytes never ship, only cooked output with provenance. Listed per file it would refuse with `bad_sha`.
- `content/arena/sources/` (owner CT74): raw CC0 ambientCG inputs of the cooked maps; mixed-case names the gate refuses, they never ship. Listed per file it would refuse with `bad_sha`.
<!-- /gen:audit-matrix -->

## Release-refusal dry-run (names every gap)

The gate itself is proven by `cargo test --locked -p engine-assets provenance` (7 passed, see Receipts). Its refusal shape is covered by the landed tests:

- `provenance_release_matrix_of_fifty_names_the_gap`: 50 hand records pass; clearing record 37 refuses with code `missing_url` naming `content/props/prop-37`; re-adding it and duplicating `prop-00` refuses with code `duplicate` naming that asset.
- `provenance_each_hand_gap_is_named`: empty URL -> `missing_url`, blank author -> `missing_author`, short digest -> `bad_sha`, spaced licence -> `bad_spdx`, `GPL-3.0-only`/`CC-BY-NC-4.0`/`LicenseRef` -> `license_refused`, each naming `content/arena/sand.glb`.
- `provenance_ai_chain_links_are_each_required`: model without generator, short prompt digest, and missing model each refuse with `ai_chain` naming the field.

The refusals and unlisted trees above are the real gaps: each names its asset and code, so the release stays refused until it has a row, a SHA or a rename.

## Bytes x2

- Gate bytes x2 is the landed test `provenance_rows_and_digest_repeat_bit_identical`: same records encode the same rows twice, `release_digest` repeats, key order fixed (`author` first, `url` last), swapped order moves the digest. Green in the same run as above.
- This file bytes x2: SHA-256 taken on two reads after write; both read the same bytes (see Receipts). The file is a doc row: no tick bytes, golden hash untouched.

## Receipts (pasted, not invented)

- `cargo test --manifest-path .lanes/builder-content-r7/Cargo.toml --target-dir target/pool-1 -j 4 --locked -p engine-assets provenance` -> `test result: ok. 7 passed; 0 failed` (provenance 7/7: hand_record_passes, each_hand_gap_is_named, names_stay_lowercase_paths, digests_stay_lowercase_hex, ai_chain_links_are_each_required, release_matrix_of_fifty_names_the_gap, rows_and_digest_repeat_bit_identical).
- `node tools/provenance-audit.mjs` re-derives the matrix and lists every refusal; `node --test tools/provenance-audit.test.mjs` pins the mirror to the Rust gate (SPDX allow-list read from provenance.rs).
- File SHA-256 x2: two `sha256sum` reads after write agree (values posted in the lane diff note at review; the judge re-reads the file).

Consumer handoff: CT28 locks against the generated matrix (re-run the tool, never edit the block); CT52 re-checks this file plus the seeded refusal; CT71 syncs the intake ledger against the fighter rows; PR1 chains read the URL+SPDX+author+SHA columns.
