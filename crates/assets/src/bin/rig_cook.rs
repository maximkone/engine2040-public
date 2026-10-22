//! rig_cook bin: cook the CT7 fighter rig into the compact lane the page can
//! hold, and print the receipt that says what it costs.
//!
//! The page cannot hold the rig as it stands: the hero body's four 1024x1024
//! atlas quadrants decode to 16,777,216 B of RGBA, 84 % of the page's whole
//! linear memory, and the whole rig is 6,239,621 B brotli against a
//! 2,040,000 B first-bundle budget. This cook turns the atlas into KTX2
//! block maps and drops the images from the body, keeping the rig (the
//! skeleton, the bind pose, the inverse binds, the skin rows and the mesh)
//! byte for byte.
//!
//! ```text
//! rig_cook [--root content/fighters] [--atlas <px>] [--out <dir>]
//! ```
//!
//! The receipt names the bodies, their bytes before and after, the atlas
//! bytes, the decoded RGBA the page would hold, and the total. The same
//! sources give the same bytes (run it twice and diff the files).
//!
//! Native only: the browser has no file system, so the wasm build of this bin
//! is empty.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use engine_assets::rig_cook::{cook_body_at, cooked_names, ATLAS_MAX_DIM, COOKED_DIR};
    use std::path::PathBuf;

    /// Default input: the fighters content dir, next to `glb/`.
    const DEFAULT_ROOT: &str = "content/fighters";
    /// The bodies the cook reads, in the manifest's order.
    const BODIES: [&str; 4] = [
        "hero_body.glb",
        "opp_male_light_body.glb",
        "opp_female_light_body.glb",
        "opp_female_medium_body.glb",
    ];

    /// What the bin was asked to do.
    struct Config {
        root: PathBuf,
        atlas: u32,
        out: Option<PathBuf>,
    }

    /// The usage line.
    fn usage() -> String {
        format!(
            "rig_cook [--root {DEFAULT_ROOT}] [--atlas <px>] [--out <dir>]\n\
             --atlas caps the atlas's long edge (default {ATLAS_MAX_DIM}, 0 keeps the source size)\n\
             --out writes the cooked bodies and atlases into that directory"
        )
    }

    /// Read the arguments.
    fn parse_args(args: &[String]) -> Result<Config, String> {
        let mut root = PathBuf::from(DEFAULT_ROOT);
        let mut atlas = ATLAS_MAX_DIM;
        let mut out: Option<PathBuf> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => {
                    i += 1;
                    root = PathBuf::from(
                        args.get(i)
                            .ok_or_else(|| format!("--root needs a path\n{}", usage()))?,
                    );
                }
                "--atlas" => {
                    i += 1;
                    atlas = args
                        .get(i)
                        .ok_or_else(|| format!("--atlas needs a number\n{}", usage()))?
                        .parse::<u32>()
                        .map_err(|err| format!("--atlas needs a number: {err}"))?;
                }
                "--out" => {
                    i += 1;
                    out = Some(PathBuf::from(
                        args.get(i)
                            .ok_or_else(|| format!("--out needs a path\n{}", usage()))?,
                    ));
                }
                other => return Err(format!("unknown argument {other:?}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Config { root, atlas, out })
    }

    /// Cook every body and print the receipt.
    pub fn run(args: &[String]) -> Result<(), String> {
        let config = parse_args(args)?;
        let mut rows: Vec<String> = Vec::with_capacity(BODIES.len());
        let mut source_bytes = 0usize;
        let mut cooked_bytes = 0usize;
        let mut atlas_bytes = 0usize;
        let mut decoded_bytes = 0usize;
        for name in BODIES {
            let path = config.root.join("glb").join(name);
            let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            let cooked = cook_body_at(name, &bytes, config.atlas).map_err(|err| err.to_string())?;
            let atlas: usize = cooked.textures.iter().map(Vec::len).sum();
            let edge = usize::try_from(config.atlas.max(1024)).unwrap_or(1024);
            let decoded = 4 * edge * edge;
            source_bytes += bytes.len();
            cooked_bytes += cooked.glb.len();
            atlas_bytes += atlas;
            decoded_bytes += decoded;
            if let Some(out) = &config.out {
                let dir = out.join(COOKED_DIR);
                std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
                // The names come from `cooked_names`, the same map the loader
                // reads, so the lane can never hold a file the page will not
                // ask for. A body is `glb/<variant>_body.glb` in the rig tree,
                // so the variant is that file name without the suffix.
                let variant = name.trim_end_matches("_body.glb");
                let mut atlases = cooked.textures.iter();
                for key in cooked_names(variant).keys() {
                    let bytes = if key.ends_with(".glb") {
                        cooked.glb.as_slice()
                    } else {
                        atlases
                            .next()
                            .ok_or_else(|| format!("{name}: more cooked names than atlases"))?
                            .as_slice()
                    };
                    let path = out.join(key);
                    std::fs::write(&path, bytes).map_err(|err| format!("{key}: {err}"))?;
                }
                if atlases.next().is_some() {
                    return Err(format!("{name}: more atlases than cooked names"));
                }
            }
            rows.push(format!(
                "{name}: source={} body={} atlas={} decoded_rgba={} joints={} vertices={} \
triangles={}",
                bytes.len(),
                cooked.glb.len(),
                atlas,
                decoded,
                cooked.joints,
                cooked.vertices,
                cooked.triangles
            ));
        }
        let line = format!(
            "rig_cook: atlas={} bodies={} source={} cooked_body={} atlas={} decoded_rgba={}\n{}",
            config.atlas,
            BODIES.len(),
            source_bytes,
            cooked_bytes,
            atlas_bytes,
            decoded_bytes,
            rows.join("\n")
        );
        println!("{line}");
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(message) = native::run(&std::env::args().skip(1).collect::<Vec<String>>()) {
        eprintln!("rig_cook FAIL: {message}");
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
