//! arena_cook bin: cook the five CC0 arena texture sets (content/arena/sources) into
//! block-compressed KTX2 files (content/arena/cooked) with `ktx2_cook`.
//!
//! Usage: `cargo run -p engine-assets --features import --bin arena_cook -- [--root content/arena] [--max-dim 1024]`
//! Output per set: `<name>_base.ktx2` (BC7 sRGB), `<name>_normal.ktx2` (BC5, GL), `<name>_orm.ktx2`
//! (BC7 linear: R occlusion, G roughness, B metallic) and one `cook.json` receipt with sizes and PSNR.
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

/// (set directory, cooked name, has an AO map).
#[cfg(feature = "import")]
const SETS: [(&str, &str, bool); 5] = [
    ("Bricks097", "stone", true),
    ("Ground093C", "sand", true),
    ("Planks039", "wood", true),
    ("Metal046B", "iron", false),
    ("PavingStones150", "floor", true),
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
    for (dir, name, has_ao) in SETS {
        let src = root.join("sources").join(dir);
        let file = |suffix: &str| src.join(format!("{dir}_1K-JPG_{suffix}.jpg"));
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
        let (color, color_bytes) = load(&file("Color"))?;
        let (normal, normal_bytes) = load(&file("NormalGL"))?;
        let (rough, rough_bytes) = load(&file("Roughness"))?;
        let (w, h) = color.dimensions();
        if normal.dimensions() != (w, h) || rough.dimensions() != (w, h) {
            return Err(format!("{dir}: map sizes disagree"));
        }
        let (ao, ao_bytes) = if has_ao {
            let (img, n) = load(&file("AmbientOcclusion"))?;
            (Some(img), n)
        } else {
            (None, 0)
        };
        let (metal, metal_bytes) = if has_ao {
            (None, 0)
        } else {
            let (img, n) = load(&file("Metalness"))?;
            (Some(img), n)
        };
        let mut orm = vec![255u8; (w * h * 4) as usize];
        for p in 0..(w * h) as usize {
            orm[p * 4] = ao.as_ref().map_or(255, |a| a.as_raw()[p * 4]);
            orm[p * 4 + 1] = rough.as_raw()[p * 4];
            orm[p * 4 + 2] = metal.as_ref().map_or(0, |m| m.as_raw()[p * 4]);
        }
        let maps: [(&str, MapKind, &[u8], usize); 3] = [
            ("base", MapKind::BaseColor, color.as_raw(), color_bytes),
            ("normal", MapKind::Normal, normal.as_raw(), normal_bytes),
            (
                "orm",
                MapKind::Orm,
                &orm,
                rough_bytes + ao_bytes + metal_bytes,
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
