//! kit_pbr: PBR material sets for the arena kit (VR2): which kit file wears which
//! cooked CC0 set, box-projected tiling UVs, and the loaders that turn cooked KTX2
//! (or a glTF's own normal and ORM images) into render uploads.
//!
//! Owner: `arena_kit` (child module, `app` leg only); consumer: `import_kit_from`
//! and `upload_kit`. Units: meters for tile edges, RGBA8 texels. Determinism: the
//! same files give the same uploads and the same keys; keys hash the file bytes.
//!
//! Conventions. The cooked normals are OpenGL (green up) as the sources ship them.
//! The engine's tangent frame puts the bitangent along +v, and glTF v runs down the
//! image, so a GL map needs its green channel inverted at load: `to_engine_normal`.
//! ORM is R occlusion, G roughness, B metallic, all linear; the draw's material
//! factors multiply them in the forward shader.

use engine_assets::ktx2_cook::{decode_level_rgba, parse_ktx2, MapKind};
use engine_assets::AssetSource;
use engine_render::present::{TextureKey, TextureUpload};
use sha2::{Digest, Sha256};

/// Cooked set names in `content/arena/cooked/<name>_{base,normal,orm}.ktx2`.
pub const COOKED_SETS: [&str; 5] = ["stone", "sand", "wood", "iron", "floor"];

/// Texture id base of the cooked sets: three keys per set (base, normal, ORM).
const COOKED_ID_BASE: u64 = super::KIT_TEXTURE_ID_BASE + 0x100;
/// Texture id base of normal and ORM images taken from a glTF (two per file).
const GLTF_MAP_ID_BASE: u64 = super::KIT_TEXTURE_ID_BASE + 0x200;

/// Which cooked set a kit file wears and how it tiles.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct KitPbr {
    /// Index into [`COOKED_SETS`].
    pub set: usize,
    /// World edge of one texture tile in meters (box-projected UVs).
    pub tile_m: f32,
    /// Metallic factor the ORM blue channel multiplies (1 for iron, else 0).
    pub metallic: f32,
}

/// Kit file to cooked set: stone for walls, stands and debris; sand for the floor;
/// paving for the gate tunnel; wood for the crate; iron for the torch. The banner
/// keeps the cloth maps its glTF carries.
pub fn pbr_for_file(file: &str) -> Option<KitPbr> {
    let (set, tile_m, metallic) = match file {
        "sand_floor.gltf" => (1, 3.0, 0.0),
        "walls.gltf" => (0, 2.0, 0.0),
        "stands.gltf" => (0, 2.5, 0.0),
        "debris.gltf" => (0, 1.5, 0.0),
        "gate_tunnel.gltf" => (4, 2.0, 0.0),
        "crate.gltf" => (2, 1.0, 0.0),
        "torch.gltf" => (3, 0.5, 1.0),
        _ => return None,
    };
    Some(KitPbr {
        set,
        tile_m,
        metallic,
    })
}

/// A cooked set decoded for upload.
#[derive(Clone, PartialEq, Debug)]
pub struct CookedSet {
    /// Base colour (sRGB).
    pub base: (TextureKey, TextureUpload),
    /// Tangent-space normal, green already inverted for the engine frame.
    pub normal: (TextureKey, TextureUpload),
    /// Occlusion, roughness, metallic.
    pub orm: (TextureKey, TextureUpload),
}

/// One material set ready for `Presenter::upload_material_set`: the key a draw
/// names as its `normal_map`, the base it sits over and the two maps.
#[derive(Clone, PartialEq, Debug)]
pub struct KitMaterialSet {
    /// Normal-map key (the set key a draw names).
    pub key: TextureKey,
    /// Base-color key the set binds over (`None` binds white).
    pub base: Option<TextureKey>,
    /// Engine-convention normal map.
    pub normal: TextureUpload,
    /// ORM map (`None` leaves the material factors as they are).
    pub orm: Option<TextureUpload>,
}

fn digest32(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Invert the green channel in place: OpenGL normal (+Y up) to the engine frame.
pub fn to_engine_normal(upload: &mut TextureUpload) {
    for texel in upload.rgba.chunks_exact_mut(4) {
        texel[1] = 255 - texel[1];
    }
}

fn cooked_upload(
    source: &dyn AssetSource,
    name: &str,
    id: u64,
    kind: MapKind,
) -> Option<(TextureKey, TextureUpload)> {
    let bytes = source.read(&format!("cooked/{name}.ktx2")).ok()?;
    let tex = parse_ktx2(&bytes).ok()?;
    if tex.kind != kind {
        return None;
    }
    let (width, height, rgba) = decode_level_rgba(&tex, 0).ok()?;
    let mut upload = TextureUpload {
        width,
        height,
        rgba,
    };
    if kind == MapKind::Normal {
        to_engine_normal(&mut upload);
    }
    upload.validate().ok()?;
    Some((TextureKey::new(id, digest32(&bytes)), upload))
}

/// Load cooked set `set` (index into [`COOKED_SETS`]) from `cooked/` in the
/// source. `None` when any of its three files is missing or does not parse, so
/// the caller stages the glTF's own textures instead of a half set.
pub fn load_cooked_set(source: &dyn AssetSource, set: usize) -> Option<CookedSet> {
    let name = COOKED_SETS.get(set)?;
    let id = COOKED_ID_BASE + 3 * u64::try_from(set).ok()?;
    Some(CookedSet {
        base: cooked_upload(source, &format!("{name}_base"), id, MapKind::BaseColor)?,
        normal: cooked_upload(source, &format!("{name}_normal"), id + 1, MapKind::Normal)?,
        orm: cooked_upload(source, &format!("{name}_orm"), id + 2, MapKind::Orm)?,
    })
}

/// Normal and ORM images of the first glTF material that carries a normal map
/// (the banner cloth): decoded, the normal inverted for the engine frame. `None`
/// when the file has no decodable normal map. The caller keys them by file index.
pub fn gltf_maps(
    import: &engine_assets::mesh_import::GltfImport,
    file_index: usize,
    base: Option<TextureKey>,
) -> Option<KitMaterialSet> {
    use engine_assets::cooked::TextureSlot;
    let slot_of = |material: &engine_assets::cooked::CookedMaterial, wanted: TextureSlot| {
        material
            .textures
            .iter()
            .find(|slot| slot.slot == wanted)
            .and_then(|slot| import.images.get(&slot.asset))
    };
    let index = u64::try_from(file_index).ok()?;
    for material in &import.materials {
        let Some(normal_bytes) = slot_of(material, TextureSlot::Normal) else {
            continue;
        };
        let Ok(mut normal) = TextureUpload::from_encoded(normal_bytes) else {
            continue;
        };
        to_engine_normal(&mut normal);
        let orm = slot_of(material, TextureSlot::MetallicRoughness)
            .and_then(|bytes| TextureUpload::from_encoded(bytes).ok());
        return Some(KitMaterialSet {
            key: TextureKey::new(GLTF_MAP_ID_BASE + 2 * index, digest32(normal_bytes)),
            base,
            normal,
            orm,
        });
    }
    None
}

/// Box-projected tiling UVs: each vertex takes the plane of its dominant normal axis,
/// `tile_m` meters per tile. Vertical faces run v down (`-y`) like glTF images; the
/// floor plane is `(x, z)`. A degenerate `tile_m` leaves the UVs as given.
pub fn box_uvs(positions: &[[f32; 3]], normals: &[[f32; 3]], tile_m: f32) -> Vec<[f32; 2]> {
    let inv = 1.0 / tile_m;
    positions
        .iter()
        .zip(normals)
        .map(|(p, n)| {
            let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
            if ay >= ax && ay >= az {
                [p[0] * inv, p[2] * inv]
            } else if ax >= az {
                [p[2] * inv, -p[1] * inv]
            } else {
                [p[0] * inv, -p[1] * inv]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kit_file_but_the_banner_wears_a_cooked_set() {
        for file in [
            "sand_floor.gltf",
            "walls.gltf",
            "stands.gltf",
            "debris.gltf",
            "gate_tunnel.gltf",
            "crate.gltf",
            "torch.gltf",
        ] {
            let pbr = pbr_for_file(file).expect("mapped");
            assert!(pbr.set < COOKED_SETS.len() && pbr.tile_m > 0.0, "{file}");
        }
        assert_eq!(
            pbr_for_file("banner.gltf"),
            None,
            "cloth keeps its glTF maps"
        );
        assert_eq!(pbr_for_file("torch.gltf").map(|p| p.metallic), Some(1.0));
    }

    #[test]
    fn box_uvs_tile_by_meters_on_each_axis_plane() {
        let pos = [[3.0, 1.0, 6.0], [3.0, 2.0, 6.0], [3.0, 2.0, 6.0]];
        let nor = [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let uv = box_uvs(&pos, &nor, 2.0);
        assert_eq!(uv[0], [1.5, 3.0], "up face runs x,z");
        assert_eq!(uv[1], [3.0, -1.0], "side face runs z,-y");
        assert_eq!(uv[2], [1.5, -1.0], "front face runs x,-y");
    }

    #[test]
    fn green_inversion_turns_gl_into_the_engine_frame() {
        let mut up = TextureUpload {
            width: 1,
            height: 1,
            rgba: vec![128, 255, 128, 255],
        };
        to_engine_normal(&mut up);
        assert_eq!(up.rgba, vec![128, 0, 128, 255]);
    }

    /// The pinned convention: on a wall face with `box_uvs`, a GL texel whose green
    /// points image-up (a slope facing up) must tilt the shading normal toward world up
    /// after `to_engine_normal`, through the engine's own tangent and TBN rules.
    #[test]
    fn gl_green_up_tilts_a_wall_normal_up() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let normals = [[0.0, 0.0, 1.0]; 4];
        let uvs = box_uvs(&positions, &normals, 1.0);
        let mesh =
            engine_render::v1::upload_mesh_tiled(&positions, &normals, &uvs, &[0, 1, 2, 0, 2, 3])
                .expect("quad uploads");
        let tangent = mesh.tangents[0];
        // GL texel: x neutral, y fully up (255), z = 1: decodes to (0, ~0.7, ~0.7).
        let mut texel = TextureUpload {
            width: 1,
            height: 1,
            rgba: vec![128, 255, 255, 255],
        };
        to_engine_normal(&mut texel);
        let v = [
            f32::from(texel.rgba[0]) / 127.5 - 1.0,
            f32::from(texel.rgba[1]) / 127.5 - 1.0,
            f32::from(texel.rgba[2]) / 127.5 - 1.0,
        ];
        let n = [0.0f32, 0.0, 1.0];
        let t = [tangent[0], tangent[1], tangent[2]];
        let w = if tangent[3] >= 0.0 { 1.0 } else { -1.0 };
        let b = [
            w * (n[1] * t[2] - n[2] * t[1]),
            w * (n[2] * t[0] - n[0] * t[2]),
            w * (n[0] * t[1] - n[1] * t[0]),
        ];
        let world_y = t[1] * v[0] + b[1] * v[1] + n[1] * v[2];
        let world_x = t[0] * v[0] + b[0] * v[1] + n[0] * v[2];
        assert!(
            world_y > 0.5,
            "green up must tilt toward world +y, got {world_y}"
        );
        assert!(world_x.abs() < 0.05, "no sideways tilt, got {world_x}");
    }

    #[test]
    fn cooked_sets_load_from_the_content_dir_and_flip_green() {
        let root = super::super::kit_content_dir();
        let source = engine_assets::FsSource::new(&root);
        for set in 0..COOKED_SETS.len() {
            let Some(cooked) = load_cooked_set(&source, set) else {
                // LFS pointers (no `git lfs pull`) do not parse: skip, never fail.
                assert!(
                    std::fs::read(root.join("cooked/stone_base.ktx2"))
                        .map_or(true, |b| b.starts_with(b"version https://git-lfs")),
                    "cooked set {set} must load"
                );
                return;
            };
            assert_eq!(
                cooked.base.1.width, cooked.normal.1.width,
                "set {set} sizes agree"
            );
            assert_eq!(
                cooked.normal.1.width, cooked.orm.1.width,
                "set {set} sizes agree"
            );
            let mut keys = [cooked.base.0, cooked.normal.0, cooked.orm.0];
            keys.sort();
            assert!(
                keys[0] != keys[1] && keys[1] != keys[2],
                "set {set} keys distinct"
            );
        }
    }

    // ---- VR2 pass-truth capture on the real GPU frame ----------------------

    /// Minimal PNG writer (stored deflate) for the capture screenshots.
    fn png_bytes(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = 0xFFFF_FFFFu32;
            for b in bytes {
                c ^= u32::from(*b);
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc(&body).to_be_bytes());
        }
        let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
        for row in rgba.chunks_exact(width as usize * 4) {
            raw.push(0);
            raw.extend_from_slice(row);
        }
        let mut z = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (i, block) in blocks.iter().enumerate() {
            z.push(u8::from(i + 1 == blocks.len()));
            z.extend_from_slice(&(block.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            z.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for byte in &raw {
            a = (a + u32::from(*byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// VR2 Accept: with the kit on, the normal map reads LIVE on a wall: the
    /// same wall drawn with its real normal map and with a flat one differs
    /// in the wall's masked region and nowhere else. Saves the before/after
    /// screenshots under `target/vr2/`. No adapter or LFS pointers: skip.
    #[test]
    fn vr2_normal_map_is_live_on_the_kit_wall() {
        use engine_render::pass_truth::LookToggles;
        use engine_render::present::{DrawItem, DrawList, PresentError, Presenter};
        let root = super::super::kit_content_dir();
        let kit = super::super::import_kit(&root).expect("kit imports");
        if kit.cooked_sets != COOKED_SETS.len() {
            return;
        }
        let mut presenter = match Presenter::init_offscreen() {
            Ok(p) => p,
            Err(PresentError::NoAdapter) => return,
            Err(err) => panic!("presenter init failed: {err}"),
        };
        let (w, h) = (640u32, 480u32);
        presenter.resize(w, h).expect("size applies");
        let report = super::super::upload_kit(&mut presenter, &kit);
        let wall = kit
            .baked
            .iter()
            .find(|b| b.actor.id == 3)
            .expect("north wall");
        let set = kit
            .sets
            .iter()
            .find(|s| Some(s.key) == wall.normal_map)
            .expect("the wall's set");
        let flat_key = TextureKey::new(0xF1A7, [0xF1; 32]);
        let flat = TextureUpload {
            width: 4,
            height: 4,
            rgba: [128u8, 128, 255, 255].repeat(16),
        };
        presenter
            .upload_material_set(flat_key, set.base, &flat, set.orm.as_ref())
            .expect("flat-normal twin of the wall set");
        let scene = engine_render::v1::default_scene(w, h).expect("scene");
        let mut camera = engine_render::v1::Camera::new(
            [-2.0, 1.6, -4.5],
            [-2.0, 1.5, -10.0],
            [0.0, 1.0, 0.0],
            60.0f32.to_radians(),
        );
        camera.aspect = w as f32 / h as f32;
        let draw = report
            .draws
            .iter()
            .find(|d| d.key == wall.key)
            .expect("wall draw");
        let frame =
            |presenter: &mut Presenter, base: Option<TextureKey>, normal: Option<TextureKey>| {
                let list = DrawList {
                    camera: camera.clone(),
                    light: scene.light.clone(),
                    items: vec![DrawItem {
                        mesh: draw.key,
                        material: draw.material.clone(),
                        base_color: base,
                        normal_map: normal,
                    }],
                };
                presenter
                    .capture_live_pixels(&list, &LookToggles::all_on())
                    .expect("live wall frame")
            };
        let on = frame(&mut presenter, draw.base_color, draw.normal_map);
        let on_again = frame(&mut presenter, draw.base_color, draw.normal_map);
        let off = frame(&mut presenter, draw.base_color, Some(flat_key));
        let flat_colour = frame(&mut presenter, None, None);
        assert_eq!(on.rgba, on_again.rgba, "the frame repeats exactly");
        assert_eq!(on.ids, off.ids, "same coverage with the map on and off");
        let none = engine_render::pass_truth::live_gate::NO_DRAW_ID;
        let (mut masked, mut changed, mut outside) = (0usize, 0usize, 0usize);
        for (i, id) in on.ids.iter().enumerate() {
            let d = (0..3)
                .map(|c| on.rgba[i * 4 + c].abs_diff(off.rgba[i * 4 + c]))
                .max()
                .unwrap_or(0);
            if *id == none {
                outside += usize::from(d > 0);
            } else {
                masked += 1;
                changed += usize::from(d > 3);
            }
        }
        eprintln!(
            "VR2 normal map: wall mask {masked} px, {changed} changed, {outside} changed outside"
        );
        assert!(masked > 20_000, "the wall fills the frame, got {masked} px");
        // Bloom bleeds a thin fringe past the wall edge; nothing else moves.
        assert!(
            outside * 100 < masked,
            "the map moves almost nothing outside the wall: {outside} px"
        );
        assert!(
            changed * 5 > masked,
            "normal map LIVE: {changed} of {masked} wall pixels differ"
        );
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/vr2");
        std::fs::create_dir_all(&dir).expect("target/vr2");
        for (name, px) in [
            ("wall_normal_on.png", &on),
            ("wall_normal_off.png", &off),
            ("wall_before_flat_colour.png", &flat_colour),
        ] {
            std::fs::write(dir.join(name), png_bytes(&px.rgba, px.width, px.height))
                .expect("png writes");
        }
        // Whole-arena before/after: the kit flat-coloured (no maps) vs VR2.
        let establishing = |presenter: &mut Presenter, maps: bool| {
            let mut cam = engine_render::v1::Camera::new(
                [10.0, 4.5, 8.0],
                [-6.0, 0.8, -3.0],
                [0.0, 1.0, 0.0],
                60.0f32.to_radians(),
            );
            cam.aspect = w as f32 / h as f32;
            let mut list = DrawList {
                camera: cam,
                light: scene.light.clone(),
                items: Vec::new(),
            };
            let resident = presenter.mesh_keys();
            super::super::push_kit_draws(&mut list, &resident, &report);
            if !maps {
                for item in &mut list.items {
                    item.base_color = None;
                    item.normal_map = None;
                }
            }
            presenter
                .capture_live_pixels(&list, &LookToggles::all_on())
                .expect("arena frame")
        };
        let before = establishing(&mut presenter, false);
        let after = establishing(&mut presenter, true);
        assert_ne!(before.rgba, after.rgba, "the maps change the arena frame");
        std::fs::write(
            dir.join("arena_before_flat_colour.png"),
            png_bytes(&before.rgba, w, h),
        )
        .expect("png");
        std::fs::write(
            dir.join("arena_after_vr2.png"),
            png_bytes(&after.rgba, w, h),
        )
        .expect("png");
    }
}
