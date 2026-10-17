//! e2bundle bin: pack a content directory into a `.e2bundle`, or check one.
//!
//! Consumer: the web build packs the arena content once and serves the file
//! next to the page; the browser fetches it and reads it through
//! `BundleSource`. Native only: the browser has no file system, so the wasm
//! build of this bin is empty.
//!
//! ```text
//! e2bundle pack <root> <out.e2bundle> [--include <prefix>]... [--priority <prefix>=<tier>]...
//! e2bundle verify <bundle> [<root>]
//! ```
//! `pack` selects files by path prefix and ranks them in tiers (0 first, the
//! coarse pass). `verify` re-reads every blob against its hash, prints how
//! many bytes each tier needs, and with `<root>` also compares every file
//! byte for byte with the directory.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use engine_assets::asset_source::{bundle_pack, content_digest, PackRules};
    use engine_assets::{AssetSource, BundleSource, FsSource};
    use std::path::Path;

    fn usage() -> String {
        [
            "usage:",
            "  e2bundle pack <root> <out.e2bundle> [--include <prefix>]... [--priority <prefix>=<tier>]...",
            "  e2bundle verify <bundle> [<root>]",
        ]
        .join("\n")
    }

    /// Run one command line; the lines to print, or the failure text.
    pub fn run(args: &[String]) -> Result<Vec<String>, String> {
        match args.split_first() {
            Some((cmd, rest)) if cmd == "pack" => pack(rest),
            Some((cmd, rest)) if cmd == "verify" => verify(rest),
            _ => Err(usage()),
        }
    }

    fn pack(args: &[String]) -> Result<Vec<String>, String> {
        let mut rules = PackRules::new();
        let mut paths: Vec<&String> = Vec::new();
        let mut flags = args.iter();
        while let Some(arg) = flags.next() {
            match arg.as_str() {
                "--include" => {
                    let prefix = flags.next().ok_or("--include needs a path prefix")?;
                    rules = rules.include(prefix);
                }
                "--priority" => {
                    let spec = flags.next().ok_or("--priority needs <prefix>=<tier>")?;
                    rules = rules.priority_spec(spec).map_err(|err| err.to_string())?;
                }
                flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
                _ => paths.push(arg),
            }
        }
        let [root, out] = paths.as_slice() else {
            return Err(usage());
        };
        let report =
            bundle_pack(Path::new(root), Path::new(out), &rules).map_err(|err| err.to_string())?;
        Ok(vec![format!(
            "e2bundle pack OK entries={} bytes={} digest={:016x}",
            report.entries, report.bytes, report.digest
        )])
    }

    fn verify(args: &[String]) -> Result<Vec<String>, String> {
        let (file, root) = match args {
            [file] => (file, None),
            [file, root] => (file, Some(root)),
            _ => return Err(usage()),
        };
        let bytes = std::fs::read(file).map_err(|err| format!("{file}: {err}"))?;
        let bundle = BundleSource::open(bytes).map_err(|err| err.to_string())?;
        // Reading every file checks every blob hash.
        let digest = content_digest(&bundle, "").map_err(|err| err.to_string())?;
        let mut lines = vec![format!(
            "e2bundle verify OK entries={} bytes={} digest={digest:016x}",
            bundle.len(),
            bundle.byte_len()
        )];
        // (tier, files, byte where the tier ends): tiers are file prefixes.
        let mut tiers: Vec<(u8, usize, usize)> = Vec::new();
        for entry in bundle.entries_in_file_order() {
            let end = entry.offset + entry.len;
            match tiers.last_mut() {
                Some(tier) if tier.0 == entry.priority => {
                    tier.1 += 1;
                    tier.2 = end;
                }
                _ => tiers.push((entry.priority, 1, end)),
            }
        }
        for (tier, files, end) in tiers {
            lines.push(format!("  tier {tier}: {files} files, ends at byte {end}"));
        }
        if let Some(root) = root {
            let fs = FsSource::new(root);
            for path in bundle.list("").map_err(|err| err.to_string())? {
                let on_disk = fs.read(&path).map_err(|err| err.to_string())?;
                let packed = bundle.read(&path).map_err(|err| err.to_string())?;
                if on_disk != packed {
                    return Err(format!("{path:?} differs between the bundle and {root}"));
                }
            }
            lines.push(format!("  every file matches {root}"));
        }
        Ok(lines)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn args(words: &[&str]) -> Vec<String> {
            words.iter().map(|word| (*word).to_string()).collect()
        }

        #[test]
        fn pack_then_verify_round_trips_and_names_bad_input() {
            let dir = std::env::temp_dir().join(format!("e2bundle_bin_{}", std::process::id()));
            let root = dir.join("content");
            std::fs::create_dir_all(root.join("src")).expect("mkdir");
            std::fs::write(root.join("placement.json"), b"{}").expect("write");
            std::fs::write(root.join("src").join("a.gltf"), [5u8; 64]).expect("write");
            let out = dir.join("arena.e2bundle");
            let (root_s, out_s) = (root.display().to_string(), out.display().to_string());

            let packed =
                run(&args(&["pack", &root_s, &out_s, "--priority", "src/=1"])).expect("packs");
            assert!(packed[0].contains("entries=2"), "{packed:?}");
            let checked = run(&args(&["verify", &out_s, &root_s])).expect("verifies");
            assert!(checked[0].contains("entries=2"), "{checked:?}");
            assert!(
                checked.iter().any(|line| line.contains("tier 1: 1 files")),
                "{checked:?}"
            );
            assert!(
                checked.last().is_some_and(|line| line.contains("matches")),
                "{checked:?}"
            );
            // The digest printed by pack is the digest verify reads back.
            let digest = |line: &str| line.rsplit("digest=").next().map(str::to_string);
            assert_eq!(digest(&packed[0]), digest(&checked[0]));

            std::fs::write(root.join("src").join("a.gltf"), [6u8; 64]).expect("edit");
            let err = run(&args(&["verify", &out_s, &root_s])).expect_err("edited file");
            assert!(err.contains("src/a.gltf"), "{err}");
            assert!(run(&args(&["pack"])).is_err());
            assert!(run(&args(&["pack", "--nope"])).is_err());
            assert!(run(&args(&["verify", &dir.join("none").display().to_string()])).is_err());
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match native::run(&args) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        Err(message) => {
            eprintln!("e2bundle FAIL: {message}");
            std::process::exit(1);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
