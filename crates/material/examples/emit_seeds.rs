//! Write the shipped material records as JSON files under
//! `content/materials/<set>/<name>.json`, in the canonical text the crate
//! emits. The `shipped_files_match_the_emitter` test pins those files to
//! the emitter, so run this after changing a seed and commit the result.
//! Reference records the siege kit already owns (marble, cloth) are not
//! written twice.

use std::fs;
use std::path::PathBuf;

use engine_material::MaterialSet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/materials");
    let mut written = 0usize;
    let siege = MaterialSet::siege_kit();
    for (folder, set) in [
        ("siege", MaterialSet::siege_kit()),
        ("reference", MaterialSet::reference()),
    ] {
        let dir = root.join(folder);
        fs::create_dir_all(&dir)?;
        for record in set.records() {
            // The siege folder already ships marble and cloth; no second copy.
            if folder == "reference" && siege.get(&record.name).is_some() {
                continue;
            }
            fs::write(dir.join(format!("{}.json", record.name)), record.to_text())?;
            written += 1;
        }
    }
    println!("wrote {written} records under {}", root.display());
    Ok(())
}
