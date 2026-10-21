//! fighter_bundle bin: cook the CT7 fighter rig into one bundle the page
//! fetches.
//!
//! Consumer: the page fetches `assets/fighters.e2bundle` and opens it with
//! `BundleSource`, then hands that source to `engine_game::fighter_look`.
//! The native window keeps reading `content/fighters` from disk, so nothing
//! here touches the cooked fighters.
//!
//! ```text
//! fighter_bundle [--root content/fighters] [--out <file>]
//! ```
//!
//! The receipt line names the entries, the bytes per tier, the first rig
//! prefix, the bar and the verdict. Same sources give the same bytes (run it
//! twice and diff the files).
//!
//! Native only: the browser has no file system, so the wasm build of this bin
//! is empty.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use engine_assets::asset_source::{BundleSource, FsSource};
    use engine_assets::fighter_bundle::cook;
    use engine_assets::AssetSource;

    /// Default input: the fighters content dir, next to `glb/`.
    const DEFAULT_ROOT: &str = "content/fighters";
    /// Default output: the bundle beside the rig it packs.
    const DEFAULT_OUT: &str = "content/fighters/fighters.e2bundle";

    /// The command line: the rig dir and the bundle to write.
    struct Config {
        /// The rig tree: `manifest.json` beside `glb/`.
        root: std::path::PathBuf,
        /// The bundle to write.
        out: std::path::PathBuf,
    }

    fn usage() -> String {
        format!("usage: fighter_bundle [--root {DEFAULT_ROOT}] [--out {DEFAULT_OUT}]")
    }

    fn parse_args(args: &[String]) -> Result<Config, String> {
        let mut root = std::path::PathBuf::from(DEFAULT_ROOT);
        let mut out = std::path::PathBuf::from(DEFAULT_OUT);
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => {
                    i += 1;
                    let value = args.get(i).ok_or("--root needs a path")?;
                    root = std::path::PathBuf::from(value);
                }
                "--out" => {
                    i += 1;
                    let value = args.get(i).ok_or("--out needs a path")?;
                    out = std::path::PathBuf::from(value);
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other:?}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Config { root, out })
    }

    /// Cook the rig, write the bundle and read it back through the reader the
    /// page uses.
    pub fn run(args: &[String]) -> Result<String, String> {
        let config = parse_args(args)?;
        let source = FsSource::new(&config.root);
        let cooked = cook(&source).map_err(|err| err.to_string())?;
        if let Some(parent) = config.out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        std::fs::write(&config.out, cooked.bytes.as_ref())
            .map_err(|err| format!("{}: {err}", config.out.display()))?;
        // Re-read the file from disk through the reader the page uses and check
        // every rig file comes back byte for byte (the reader hashes each blob).
        let on_disk =
            std::fs::read(&config.out).map_err(|err| format!("{}: {err}", config.out.display()))?;
        let bundle = BundleSource::open(on_disk).map_err(|err| err.to_string())?;
        for path in bundle.list("").map_err(|err| err.to_string())? {
            let want = source.read(&path).map_err(|err| err.to_string())?;
            let got = bundle.read(&path).map_err(|err| err.to_string())?;
            if want != got {
                return Err(format!("{path}: read back differs from the source"));
            }
        }
        let verdict = if cooked.first_prefix_fits() {
            "ok"
        } else {
            "over"
        };
        let line = format!(
            "fighter_bundle: entries={} bytes={} tiers=0:{} 1:{} 2:{} first_prefix={} bar={} \
{verdict} digest={:016x} out={}",
            cooked.entries,
            cooked.bytes.len(),
            cooked.tier_bytes[0],
            cooked.tier_bytes[1],
            cooked.tier_bytes[2],
            cooked.first_prefix_bytes,
            cooked.bar,
            cooked.digest,
            config.out.display()
        );
        if verdict == "over" {
            return Err(format!(
                "{line}\nthe first rig prefix is over the {} B bar; the page fetches it before \
the other bodies",
                cooked.bar
            ));
        }
        Ok(line)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match native::run(&args) {
        Ok(line) => println!("{line}"),
        Err(message) => {
            eprintln!("fighter_bundle FAIL: {message}");
            std::process::exit(1);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
