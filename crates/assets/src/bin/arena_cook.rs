//! arena_cook bin: cook the CC0 arena texture sets (content/arena/sources) into
//! block-compressed KTX2 files (content/arena/cooked) with `ktx2_cook`.
//!
//! Usage: `cargo run -p engine-assets --features import --bin arena_cook -- [--root content/arena] [--max-dim 1024]`
//! Output per set: `<name>_base.ktx2` (BC7 sRGB), `<name>_normal.ktx2` (BC5, GL), `<name>_orm.ktx2`
//! (BC7 linear: R occlusion, G roughness, B metallic) and one `cook.json` receipt with sizes and PSNR.
//! Eight sets: the five ambientCG ones, `marble_01` and `marble_tiles` (SW122) and
//! Poly Haven `dense_sand` (SW124), whose `arm` image packs AO/rough/metal.
//! The same sources and cap give the same bytes (run it twice and diff).

#[cfg(feature = "import")]
fn main() {
    if let Err(message) = run() {
        eprintln!("arena_cook FAIL: {message}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "import"))]
fn main() {
    eprintln!("arena_cook needs --features import");
    std::process::exit(2);
}

/// One source set: no directory, cooked name, and the map files it ships.
/// `arm` is the packed AO/roughness/metal one image (Poly Haven), `ao` +
/// `metal` are the separate ambientCG images, and `has_ao` picks the shape.
#[cfg(feature = "import")]
struct Set {
    dir: &'static str,
    name: &'static str,
    /// `1K-JPG` style middles, per map lane.
    color: &'static str,
    normal: &'static str,
    rough: &'static str,
    ao: Option<&'static str>,
    metal: Option<&'static str>,
    /// Ambient occlusion, roughness and metalness in R, G, B of one image.
    arm: Option<&'static str>,
}

/// The arena's CC0 texture sets, in cook order. The five ambientCG sets came
/// first (AC naming); `marble_01` and `marble_tiles` (SW122) keep the AC names,
/// and `dense_sand` (SW124) is Poly Haven's `_diff_1k` / `_nor_gl_1k` /
/// `_arm_1k` / `_rough_1k` naming, so each set names its own files.
#[cfg(feature = "import")]
const SETS: [Set; 8] = [
    Set {
        dir: "Bricks097",
        name: "stone",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: Some("AmbientOcclusion"),
        metal: None,
        arm: None,
    },
    Set {
        dir: "Ground093C",
        name: "sand",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: Some("AmbientOcclusion"),
        metal: None,
        arm: None,
    },
    Set {
        dir: "Planks039",
        name: "wood",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: Some("AmbientOcclusion"),
        metal: None,
        arm: None,
    },
    Set {
        dir: "Metal046B",
        name: "iron",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: None,
        metal: Some("Metalness"),
        arm: None,
    },
    Set {
        dir: "PavingStones150",
        name: "floor",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: Some("AmbientOcclusion"),
        metal: None,
        arm: None,
    },
    Set {
        dir: "marble_01",
        name: "marble",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: Some("AmbientOcclusion"),
        metal: None,
        arm: None,
    },
    Set {
        dir: "marble_tiles",
        name: "marble_tiles",
        color: "Color",
        normal: "NormalGL",
        rough: "Roughness",
        ao: Some("AmbientOcclusion"),
        metal: None,
        arm: None,
    },
    // Poly Haven naming: `dense_sand_diff_1k.jpg`, `_nor_gl_1k`, `_arm_1k`,
    // `_rough_1k`. The arm image carries AO/rough/metal in R/G/B, so no
    // separate AO or metal file is read for this set.
    Set {
        dir: "dense_sand",
        name: "sand_dense",
        color: "diff_1k",
        normal: "nor_gl_1k",
        rough: "rough_1k",
        ao: None,
        metal: None,
        arm: Some("arm_1k"),
    },
];

#[cfg(feature = "import")]
fn run() -> Result<(), String> {
    use engine_assets::ktx2_cook::{cook_ktx2, decode_level_rgba, parse_ktx2, MapKind};
    use std::path::PathBuf;

    let mut root = PathBuf::from("content/arena");
    let mut max_dim = 1024u32;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                root = PathBuf::from(args.get(i).ok_or("--root needs a path")?);
            }
            "--max-dim" => {
                i += 1;
                max_dim = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--max-dim needs a number")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }
    let out_dir = root.join("cooked");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut receipt = String::from("{\n  \"max_dim\": ");
    receipt.push_str(&max_dim.to_string());
    receipt.push_str(",\n  \"maps\": [\n");
    let mut first = true;
    let (mut total_cooked, mut total_rgba, mut total_src) = (0usize, 0usize, 0usize);
    for set in &SETS {
        let dir = set.dir;
        let name = set.name;
        let src = root.join("sources").join(dir);
        // AmbientCG sets name their maps `<dir>_1K-JPG_<Map>.jpg`; Poly Haven
        // sets name them `<dir>_<map>_1k.jpg`, so the middle carries the shape.
        let file = |suffix: &str| {
            if suffix.ends_with("_1k") {
                src.join(format!("{dir}_{suffix}.jpg"))
            } else {
                src.join(format!("{dir}_1K-JPG_{suffix}.jpg"))
            }
        };
        let load = |path: &std::path::Path| -> Result<(image::RgbaImage, usize), String> {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            if bytes.starts_with(b"version https://git-lfs") {
                return Err(format!(
                    "{} is an LFS pointer: run git lfs pull",
                    path.display()
                ));
            }
            let img =
                image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((img.to_rgba8(), bytes.len()))
        };
        let (color, color_bytes) = load(&file(set.color))?;
        let (normal, normal_bytes) = load(&file(set.normal))?;
        let (rough, rough_bytes) = load(&file(set.rough))?;
        let (w, h) = color.dimensions();
        if normal.dimensions() != (w, h) || rough.dimensions() != (w, h) {
            return Err(format!("{dir}: map sizes disagree"));
        }
        let (ao, ao_bytes) = match set.ao {
            Some(lane) => {
                let (img, n) = load(&file(lane))?;
                (Some(img), n)
            }
            None => (None, 0),
        };
        let (metal, metal_bytes) = match set.metal {
            Some(lane) => {
                let (img, n) = load(&file(lane))?;
                (Some(img), n)
            }
            None => (None, 0),
        };
        // Poly Haven packs AO/rough/metal into one `arm` image; its R and B
        // stand in for the missing AO and metal maps, its G is checked against
        // the separate rough lane so the two cannot drift.
        let (arm, arm_bytes) = match set.arm {
            Some(lane) => {
                let (img, n) = load(&file(lane))?;
                if img.dimensions() != (w, h) {
                    return Err(format!("{dir}: arm map size disagrees"));
                }
                (Some(img), n)
            }
            None => (None, 0),
        };
        let mut orm = vec![255u8; (w * h * 4) as usize];
        for p in 0..(w * h) as usize {
            orm[p * 4] = ao.as_ref().map_or_else(
                || arm.as_ref().map_or(255, |a| a.as_raw()[p * 4]),
                |a| a.as_raw()[p * 4],
            );
            orm[p * 4 + 1] = rough.as_raw()[p * 4];
            orm[p * 4 + 2] = metal.as_ref().map_or_else(
                || arm.as_ref().map_or(0, |a| a.as_raw()[p * 4 + 2]),
                |m| m.as_raw()[p * 4],
            );
        }
        if let Some(arm) = &arm {
            // The arm's green lane and the separate rough JPG carry the same
            // roughness; JPEG quantisation moves single texels, so the guard is
            // the mean drift, not the worst texel (measured 1.29 on dense_sand).
            let total: u64 = arm
                .as_raw()
                .chunks_exact(4)
                .zip(rough.as_raw().chunks_exact(4))
                .map(|(a, r)| u64::from(a[1].abs_diff(r[0])))
                .sum();
            let texels = u64::try_from(arm.as_raw().len() / 4).unwrap_or(u64::MAX / 8);
            if total > 4 * texels {
                return Err(format!("{dir}: arm green and rough drift apart"));
            }
        }
        let maps: [(&str, MapKind, &[u8], usize); 3] = [
            ("base", MapKind::BaseColor, color.as_raw(), color_bytes),
            ("normal", MapKind::Normal, normal.as_raw(), normal_bytes),
            (
                "orm",
                MapKind::Orm,
                &orm,
                rough_bytes + ao_bytes + metal_bytes + arm_bytes,
            ),
        ];
        for (suffix, kind, rgba, src_bytes) in maps {
            let ktx2 =
                cook_ktx2(rgba, w, h, kind, max_dim).map_err(|e| format!("{dir} {suffix}: {e}"))?;
            let tex = parse_ktx2(&ktx2).map_err(|e| format!("{dir} {suffix}: reparse {e}"))?;
            let (dw, dh, decoded) =
                decode_level_rgba(&tex, 0).map_err(|e| format!("{dir} {suffix}: {e}"))?;
            // PSNR of level 0 against the source (resized when the cap shrank it).
            let reference = if (dw, dh) == (w, h) {
                rgba.to_vec()
            } else {
                image::imageops::resize(
                    &image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or("raw size")?,
                    dw,
                    dh,
                    image::imageops::FilterType::Triangle,
                )
                .into_raw()
            };
            let channels = if kind == MapKind::Normal { 2 } else { 3 };
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for (a, b) in reference.chunks_exact(4).zip(decoded.chunks_exact(4)) {
                for c in 0..channels {
                    let d = f64::from(a[c]) - f64::from(b[c]);
                    sum += d * d;
                    n += 1.0;
                }
            }
            let psnr = 10.0 * (255.0f64 * 255.0 / (sum / n).max(1e-9)).log10();
            let out = out_dir.join(format!("{name}_{suffix}.ktx2"));
            std::fs::write(&out, &ktx2).map_err(|e| format!("{}: {e}", out.display()))?;
            let rgba_bytes = (w * h * 4) as usize;
            total_cooked += ktx2.len();
            total_rgba += rgba_bytes;
            total_src += src_bytes;
            println!(
                "{name}_{suffix}: {dw}x{dh} {} levels, {} B ktx2, {src_bytes} B source jpg, {rgba_bytes} B rgba8, {:.1}x vs rgba8, {psnr:.1} dB",
                tex.levels.len(),
                ktx2.len(),
                rgba_bytes as f64 / ktx2.len() as f64,
            );
            if !first {
                receipt.push_str(",\n");
            }
            first = false;
            receipt.push_str(&format!(
                "    {{\"file\": \"{name}_{suffix}.ktx2\", \"width\": {dw}, \"height\": {dh}, \"levels\": {}, \"ktx2_bytes\": {}, \"source_jpg_bytes\": {src_bytes}, \"rgba8_bytes\": {rgba_bytes}, \"psnr_db\": {psnr:.1}}}",
                tex.levels.len(),
                ktx2.len()
            ));
        }
    }
    receipt.push_str(&format!(
        "\n  ],\n  \"total\": {{\"ktx2_bytes\": {total_cooked}, \"source_jpg_bytes\": {total_src}, \"rgba8_bytes\": {total_rgba}}}\n}}\n"
    ));
    std::fs::write(out_dir.join("cook.json"), receipt).map_err(|e| format!("cook.json: {e}"))?;
    println!(
        "total: {total_cooked} B ktx2 vs {total_rgba} B rgba8 ({:.2}x) vs {total_src} B source jpg",
        total_rgba as f64 / total_cooked as f64
    );
    Ok(())
}
