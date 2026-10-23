//! e2040-ktx2-cook: cook one PNG map into KTX2 through the existing
//! `engine-assets::ktx2_cook` lane. GEO14 adds no second encoder: the block
//! codec, the mip chain, the container and the determinism are the crate's.
//!
//! ```text
//! e2040-ktx2-cook --in <map.png> --out <map.ktx2> --kind orm|normal|base [--max-dim N]
//! ```
//!
//! Prints `ok <out> <w>x<h> <bytes>`. The same PNG, kind and cap give the same
//! bytes. Lives under tools/dcc so no file under crates/ changes.

use engine_assets::ktx2_cook::{cook_ktx2, MapKind};
use std::path::PathBuf;

fn main() {
    if let Err(message) = run() {
        eprintln!("ktx2-cook FAIL: {message}");
        std::process::exit(1);
    }
}

/// The map kind the cook is asked for.
fn kind_of(name: &str) -> Result<MapKind, String> {
    match name {
        "base" => Ok(MapKind::BaseColor),
        "normal" => Ok(MapKind::Normal),
        "orm" => Ok(MapKind::Orm),
        other => Err(format!("--kind wants base, normal or orm, got {other:?}")),
    }
}

fn run() -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut kind: Option<MapKind> = None;
    let mut max_dim = 0u32;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--in" => input = Some(PathBuf::from(args.next().ok_or("--in needs a path")?)),
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out needs a path")?)),
            "--kind" => {
                let name = args.next().ok_or("--kind needs a name")?;
                kind = Some(kind_of(&name)?);
            }
            "--max-dim" => {
                let value = args.next().ok_or("--max-dim needs a number")?;
                max_dim = value
                    .parse()
                    .map_err(|err| format!("--max-dim {value:?}: {err}"))?;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let input = input.ok_or("--in is required")?;
    let out = out.ok_or("--out is required")?;
    let kind = kind.ok_or("--kind is required")?;
    let bytes = std::fs::read(&input).map_err(|err| format!("{}: {err}", input.display()))?;
    let rgba = image::load_from_memory(&bytes)
        .map_err(|err| format!("{}: {err}", input.display()))?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    let ktx2 =
        cook_ktx2(rgba.as_raw(), width, height, kind, max_dim).map_err(|err| err.to_string())?;
    std::fs::write(&out, &ktx2).map_err(|err| format!("{}: {err}", out.display()))?;
    println!("ok {} {width}x{height} {} B", out.display(), ktx2.len());
    Ok(())
}
