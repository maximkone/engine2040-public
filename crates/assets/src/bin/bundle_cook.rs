//! bundle_cook bin: cook entry feeding cooked bytes into E204BNDL.
//!
//! Consumer: the packaged game stages the `--out` file via `BUNDLE_RESOLVE`
//! (`--bundle`). Default cooks the deterministic demo staging (mesh shape +
//! texture shape + material/skin/clip); `--texture-png name=path` cooks one
//! more BaseColor texture per flag through the real `cook_texture` path.
//! Every `--out` cook also writes `<out>.manifest` beside the bundle with the
//! seed, thread count, profile and per-entry blake3 digests.
//! The 16-byte stub writes/loads only with `--allow-stub`.

#[cfg(all(feature = "import", feature = "registry"))]
fn run() -> i32 {
    match run_cook(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("bundle_cook FAIL: {message}");
            1
        }
    }
}

#[cfg(not(all(feature = "import", feature = "registry")))]
fn run() -> i32 {
    eprintln!("bundle_cook needs --features import,registry");
    2
}

fn main() {
    std::process::exit(run());
}

#[cfg(all(feature = "import", feature = "registry"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Config {
    out: Option<std::path::PathBuf>,
    check: Option<std::path::PathBuf>,
    textures: Vec<(String, std::path::PathBuf)>,
    empty: bool,
    allow_stub: bool,
}

#[cfg(all(feature = "import", feature = "registry"))]
fn usage() -> String {
    [
        "bundle_cook: cook staged content into an E204BNDL bundle file.",
        "",
        "Usage:",
        "  bundle_cook --out <bundle.bin> [--texture-png <name=path> ...] [--empty] [--allow-stub]",
        "  bundle_cook --check <bundle.bin> [--allow-stub]",
        "",
        "Rules: default cooks the demo staging (non-empty); --empty writes the",
        "16-byte stub, refused without --allow-stub; --check verifies header +",
        "digests the way ship pack BUNDLE_RESOLVE accepts them.",
    ]
    .join("\n")
}

#[cfg(all(feature = "import", feature = "registry"))]
fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut out: Option<std::path::PathBuf> = None;
    let mut check: Option<std::path::PathBuf> = None;
    let mut textures: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut empty = false;
    let mut allow_stub = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                let value = args.get(i).ok_or("missing value for --out")?;
                if value.is_empty() {
                    return Err("--out needs a non-empty path".to_string());
                }
                out = Some(std::path::PathBuf::from(value));
            }
            "--check" => {
                i += 1;
                let value = args.get(i).ok_or("missing value for --check")?;
                if value.is_empty() {
                    return Err("--check needs a non-empty path".to_string());
                }
                check = Some(std::path::PathBuf::from(value));
            }
            "--texture-png" => {
                i += 1;
                let value = args.get(i).ok_or("missing value for --texture-png")?;
                let (name, path) = value
                    .split_once('=')
                    .ok_or_else(|| "--texture-png wants <name=path>".to_string())?;
                if name.is_empty() || path.is_empty() {
                    return Err("--texture-png wants <name=path>".to_string());
                }
                textures.push((name.to_string(), std::path::PathBuf::from(path)));
            }
            "--empty" => empty = true,
            "--allow-stub" => allow_stub = true,
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown flag {other:?}; pass --help")),
        }
        i += 1;
    }
    if check.is_some() && out.is_some() {
        return Err("pass --check or --out, not both".to_string());
    }
    if check.is_none() && out.is_none() {
        return Err(format!("pass --out or --check\n{}", usage()));
    }
    if empty && !textures.is_empty() {
        return Err("--empty takes no --texture-png".to_string());
    }
    Ok(Config {
        out,
        check,
        textures,
        empty,
        allow_stub,
    })
}

#[cfg(all(feature = "import", feature = "registry"))]
fn run_cook(args: &[String]) -> Result<i32, String> {
    let config = parse_args(args)?;
    if let Some(path) = config.check {
        let (loaded, receipt) =
            engine_assets::load_bundle_file(&path, config.allow_stub).map_err(|e| e.to_string())?;
        println!("{receipt}");
        for entry in &loaded.entries {
            println!("entry {} {} {}", entry.kind, entry.name, entry.digest);
        }
        return Ok(0);
    }
    let out = config.out.ok_or("missing --out")?;
    let mut staged = if config.empty {
        engine_assets::StagedCooked::new()
    } else {
        engine_assets::demo_staged()
    };
    for (name, png_path) in &config.textures {
        let encoded =
            std::fs::read(png_path).map_err(|e| format!("read {}: {e}", png_path.display()))?;
        let cooked = engine_assets::cook_texture(&encoded, engine_assets::TextureSlot::BaseColor)
            .map_err(|e| format!("cook_texture {name}: {e:?}"))?;
        staged.textures.push((name.clone(), cooked));
    }
    let receipt = engine_assets::cook_bundle_with_manifest(
        &staged,
        &out,
        config.allow_stub,
        engine_assets::manifest::DEFAULT_SEED,
        engine_assets::manifest::DEFAULT_THREADS,
        engine_assets::manifest::PROFILE_POTATO,
    )
    .map_err(|e| e.to_string())?;
    println!("{receipt}");
    Ok(0)
}

#[cfg(all(test, feature = "import", feature = "registry"))]
mod tests {
    use super::*;

    fn args(flags: &[&str]) -> Vec<String> {
        flags.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn out_and_check_parse() {
        let config = parse_args(&args(&["--out", "a.bin"])).unwrap();
        assert_eq!(config.out.unwrap().to_string_lossy(), "a.bin");
        assert!(!config.allow_stub);
        let config = parse_args(&args(&["--check", "a.bin", "--allow-stub"])).unwrap();
        assert_eq!(config.check.unwrap().to_string_lossy(), "a.bin");
        assert!(config.allow_stub);
    }

    #[test]
    fn out_writes_manifest_beside_bundle() {
        let dir = std::env::temp_dir().join(format!("bundle_cook_manifest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");
        let out = dir.join("demo.e2bundle");
        let out_text = out.to_string_lossy().to_string();
        let code = run_cook(&args(&["--out", &out_text])).expect("cook runs");
        assert_eq!(code, 0);
        let manifest_path = dir.join("demo.e2bundle.manifest");
        let bytes = std::fs::read(&manifest_path).expect("manifest written");
        let manifest = engine_assets::Manifest::parse(&bytes).expect("manifest parses");
        assert_eq!(manifest.seed(), engine_assets::manifest::DEFAULT_SEED);
        assert_eq!(manifest.threads(), engine_assets::manifest::DEFAULT_THREADS);
        assert_eq!(manifest.profile(), engine_assets::manifest::PROFILE_POTATO);
        assert_eq!(manifest.artifacts().count(), 1);
        assert_eq!(manifest.sources().count(), 5);
        manifest.verify_artifacts(&dir).expect("artifact verifies");
    }

    #[test]
    fn bad_flags_rejected() {
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["--out", "a.bin", "--check", "b.bin"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
        assert!(parse_args(&args(&["--texture-png", "noname"])).is_err());
        assert!(parse_args(&args(&["--texture-png", "=p.png"])).is_err());
        assert!(parse_args(&args(&[
            "--empty",
            "--texture-png",
            "n=p.png",
            "--out",
            "o"
        ]))
        .is_err());
        assert!(parse_args(&args(&["--out"])).is_err());
    }
}
