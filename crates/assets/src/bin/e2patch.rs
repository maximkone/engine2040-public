//! e2patch bin: build a content patch by blob hash, or check one against a base.
//!
//! Consumer: a content update ships the patch, the client keeps its base and
//! fetches only the blobs the patch carries. The patch is the `.e2patch` file
//! [`PatchSet`] writes: the new manifest plus the changed blobs. Native only:
//! the browser has no file system, so the wasm build of this bin is empty.
//!
//! ```text
//! e2patch diff <base-dir> <new-dir> <out.e2patch>
//! e2patch verify <base-dir> <new-dir> <patch>
//! ```
//! `diff` writes the patch outside both trees and prints how many files are
//! unchanged, changed, added and removed, the patch size and its digest.
//! `verify` loads the patch, checks the base against the new manifest, applies
//! it over the base and compares every file with `<new-dir>` byte for byte.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use engine_assets::asset_source::{Manifest, PatchSet};
    use engine_assets::{AssetSource, FsSource};
    use std::path::Path;

    fn usage() -> String {
        [
            "usage:",
            "  e2patch diff <base-dir> <new-dir> <out.e2patch>",
            "  e2patch verify <base-dir> <new-dir> <patch>",
        ]
        .join("\n")
    }

    /// Run one command line; the lines to print, or the failure text.
    pub fn run(args: &[String]) -> Result<Vec<String>, String> {
        match args.split_first() {
            Some((cmd, rest)) if cmd == "diff" => diff(rest),
            Some((cmd, rest)) if cmd == "verify" => verify(rest),
            _ => Err(usage()),
        }
    }

    fn diff(args: &[String]) -> Result<Vec<String>, String> {
        let [base_dir, new_dir, out] = args else {
            return Err(usage());
        };
        let base = FsSource::new(base_dir);
        let new = FsSource::new(new_dir);
        let manifest = Manifest::from_source(&base).map_err(|err| err.to_string())?;
        let (patch, stats) = PatchSet::build(&manifest, &new).map_err(|err| err.to_string())?;
        let bytes = patch.to_bytes().map_err(|err| err.to_string())?;
        write_patch(out, &bytes)?;
        Ok(vec![format!(
            "e2patch diff OK unchanged={} changed={} added={} removed={} entries={} blobs={} blob_bytes={} patch_bytes={} digest={:016x}",
            stats.unchanged,
            stats.changed,
            stats.added,
            stats.removed,
            stats.entries,
            stats.blobs,
            stats.blob_bytes,
            stats.patch_bytes,
            stats.digest
        )])
    }

    fn verify(args: &[String]) -> Result<Vec<String>, String> {
        let [base_dir, new_dir, patch_file] = args else {
            return Err(usage());
        };
        let bytes = std::fs::read(patch_file).map_err(|err| format!("{patch_file}: {err}"))?;
        let patch = PatchSet::parse(&bytes).map_err(|err| err.to_string())?;
        let base = FsSource::new(base_dir);
        patch.verify_base(&base).map_err(|err| err.to_string())?;
        let new = FsSource::new(new_dir);
        let names = new.list("").map_err(|err| err.to_string())?;
        let patched = patch.apply(&base);
        if patched.list("").map_err(|err| err.to_string())? != names {
            return Err(format!("the patch manifest does not match {new_dir}"));
        }
        for name in &names {
            let want = new.read(name).map_err(|err| err.to_string())?;
            let got = patched.read(name).map_err(|err| err.to_string())?;
            if want != got {
                return Err(format!("{name:?} differs from the base plus patch"));
            }
            if !patched.patched(name) {
                let kept = base.read(name).map_err(|err| err.to_string())?;
                if kept != got {
                    return Err(format!("{name:?} changed without a patch blob"));
                }
            }
        }
        Ok(vec![format!(
            "e2patch verify OK entries={} blobs={} unchanged={} every file matches {new_dir} digest={:016x}",
            patch.manifest().len(),
            patch.stored_len(),
            patch.manifest().len().saturating_sub(patch.stored_len()),
            engine_core::hash_bytes(&bytes),
        )])
    }

    /// Write the patch, creating its parent directories.
    fn write_patch(path: &str, bytes: &[u8]) -> Result<(), String> {
        let out = Path::new(path);
        if let Some(parent) = out.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        std::fs::write(out, bytes).map_err(|err| format!("{path}: {err}"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn args(words: &[&str]) -> Vec<String> {
            words.iter().map(|word| (*word).to_string()).collect()
        }

        fn write_tree(root: &Path, files: &[(&str, &[u8])]) {
            for (path, bytes) in files {
                let host = root.join(path);
                std::fs::create_dir_all(host.parent().expect("fixture parent")).expect("mkdir");
                std::fs::write(host, bytes).expect("fixture file");
            }
        }

        /// The base tree: its sand texture is 32 bytes of `1`.
        const BASE_FILES: [(&str, &[u8]); 3] = [
            ("placement.json", b"{\"actors\":[]}"),
            ("src/crate.gltf", &[7u8; 48]),
            ("textures/sand.bmp", &[1u8; 32]),
        ];
        /// The new tree: only the sand texture moved, to 40 bytes of `9`.
        const NEW_FILES: [(&str, &[u8]); 3] = [
            ("placement.json", b"{\"actors\":[]}"),
            ("src/crate.gltf", &[7u8; 48]),
            ("textures/sand.bmp", &[9u8; 40]),
        ];

        #[test]
        fn diff_then_verify_loads_the_new_file_and_keeps_the_rest() {
            let dir = std::env::temp_dir().join(format!("e2patch_bin_{}", std::process::id()));
            let base = dir.join("base");
            let new = dir.join("new");
            write_tree(&base, &BASE_FILES);
            write_tree(&new, &NEW_FILES);
            let patch = dir.join("update").join("content.e2patch");
            let (base_s, new_s, patch_s) = (
                base.display().to_string(),
                new.display().to_string(),
                patch.display().to_string(),
            );

            let made = run(&args(&["diff", &base_s, &new_s, &patch_s])).expect("diff");
            for field in [
                "unchanged=2",
                "changed=1",
                "added=0",
                "removed=0",
                "entries=3",
                "blobs=1",
                "blob_bytes=40",
                "digest=",
            ] {
                assert!(made[0].contains(field), "{field} missing: {made:?}");
            }
            let checked = run(&args(&["verify", &base_s, &new_s, &patch_s])).expect("verify");
            assert!(
                checked[0].contains("entries=3 blobs=1 unchanged=2"),
                "{checked:?}"
            );
            assert!(checked[0].contains("every file matches"), "{checked:?}");
            let digest = |line: &str| line.rsplit("digest=").next().map(str::to_string);
            assert_eq!(
                digest(&made[0]),
                digest(&checked[0]),
                "same patch, same digest"
            );

            // A damaged patch blob is refused by name before any file loads.
            let good = std::fs::read(&patch).expect("patch file");
            let mut damaged = good.clone();
            let last = damaged.len() - 1;
            damaged[last] ^= 0xFF;
            std::fs::write(&patch, &damaged).expect("damaged patch");
            let err = run(&args(&["verify", &base_s, &new_s, &patch_s])).expect_err("damaged");
            assert!(err.contains("textures/sand.bmp"), "{err}");
            // A patch cut inside its blob names that blob too.
            std::fs::write(&patch, &good[..last]).expect("cut patch");
            let err = run(&args(&["verify", &base_s, &new_s, &patch_s])).expect_err("cut");
            assert!(err.contains("textures/sand.bmp"), "{err}");

            // A base file edited behind the patch is refused by name too.
            std::fs::write(&patch, &good).expect("restore patch");
            std::fs::write(base.join("src").join("crate.gltf"), [8u8; 48]).expect("edit base");
            let err = run(&args(&["verify", &base_s, &new_s, &patch_s])).expect_err("stale base");
            assert!(err.contains("src/crate.gltf"), "{err}");

            assert!(run(&args(&["diff"])).is_err());
            assert!(run(&args(&["verify", &base_s])).is_err());
            assert!(run(&args(&["verify", "--nope"])).is_err());
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lines = match native::run(&args) {
        Ok(lines) => lines,
        Err(message) => {
            eprintln!("e2patch FAIL: {message}");
            return std::process::ExitCode::FAILURE;
        }
    };
    for line in lines {
        println!("{line}");
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(target_arch = "wasm32")]
fn main() {}
