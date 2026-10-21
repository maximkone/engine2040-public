//! kit_assets: the arena kit loads through one queue.
//!
//! Owner: `engine-game`. Every kit file is named by logical path and read
//! through [`engine_assets::LoadQueue`], so the native build reads files and
//! the browser reads a fetched bundle through the same code. The bytes are
//! held by a generational handle and unloaded when the import is done, so
//! the game never holds raw asset bytes outside a handle. Units: bytes for
//! the budget, counts for the caps. Determinism: priority first, then path
//! order; no wall clock, no RNG. Errors: typed [`KitError`], never panics.
//!
//! Consumer: `crates/game/src/arena_kit.rs` (the headless and the app leg
//! of the kit import).

use engine_assets::asset_source::AssetSource;
use engine_assets::{AssetHandle, HandleBudget, LoadEvent, LoadQueue};

use crate::arena_kit::{bound_diag, KitError};

use std::fmt;

/// Byte budget of one kit load; the arena kit fits with room to spare.
pub const KIT_LOAD_BYTES: u64 = 64 * 1024 * 1024;

/// Live-asset cap of one kit load: the 8 kit files and their maps.
pub const KIT_LOAD_ASSETS: u32 = 64;

/// Load priority per kit file: 2 is the ground and the collision pieces (the
/// arena stands and is walkable before the dressing lands), 1 is the
/// dressing that only refines the picture.
const KIT_LOAD_PRIORITIES: [(&str, u8); 8] = [
    ("sand_floor.gltf", 2),
    ("stands.gltf", 2),
    ("walls.gltf", 2),
    ("gate_tunnel.gltf", 2),
    ("crate.gltf", 2),
    ("debris.gltf", 2),
    ("torch.gltf", 1),
    ("banner.gltf", 1),
];

/// The priority one kit file loads at; an unknown file loads last.
#[must_use]
pub fn kit_load_priority(file: &str) -> u8 {
    KIT_LOAD_PRIORITIES
        .iter()
        .find(|(name, _)| *name == file)
        .map_or(0, |(_, level)| *level)
}

/// The logical path one kit file has inside a content source.
#[must_use]
pub fn kit_logical_path(file: &str) -> String {
    format!("src/{file}")
}

/// The kit files staged by one load: the bytes live behind generational
/// handles, and [`KitAssets::unload`] gives them back.
///
/// The queue owns the payloads, so a staged file is read through its handle
/// and never as a loose byte slice the game owns.
pub struct KitAssets<'a> {
    /// The queue that read the files.
    queue: LoadQueue<'a>,
    /// One handle per staged file, in request order.
    files: Vec<(String, AssetHandle)>,
}

impl fmt::Debug for KitAssets<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KitAssets")
            .field("files", &self.files.len())
            .field("resident_bytes", &self.resident_bytes())
            .finish()
    }
}

impl<'a> KitAssets<'a> {
    /// Stage every kit file through one queue over `source`.
    ///
    /// The files are queued in priority order (the ground and the collision
    /// pieces first) and read in one pump under the byte budget. A file the
    /// source refuses fails the load by name; nothing is left half staged.
    pub fn load(source: &'a dyn AssetSource, files: &[&str]) -> Result<Self, KitError> {
        let budget = HandleBudget::new(KIT_LOAD_BYTES, KIT_LOAD_ASSETS);
        let mut queue = LoadQueue::new(source, budget);
        let mut staged: Vec<(String, AssetHandle)> = Vec::with_capacity(files.len());
        for file in files {
            let path = kit_logical_path(file);
            let handle = queue
                .enqueue(&path, kit_load_priority(file), 0)
                .map_err(|err| KitError::Io(bound_diag(&err.to_string())))?;
            staged.push((path, handle));
        }
        for event in queue.pump(KIT_LOAD_BYTES) {
            match event {
                LoadEvent::Loaded { .. } => {}
                LoadEvent::Deferred { path, reason, .. } => {
                    return Err(KitError::Io(bound_diag(&format!("{path}: {reason}"))));
                }
                LoadEvent::Failed { path, reason, .. } => {
                    return Err(KitError::Io(bound_diag(&format!("{path}: {reason}"))));
                }
                LoadEvent::Cancelled { path, .. } => {
                    return Err(KitError::Io(bound_diag(&format!("{path}: cancelled"))));
                }
                LoadEvent::Unloaded { .. } => {}
            }
        }
        for (path, handle) in &staged {
            if !queue.is_ready(*handle) {
                return Err(KitError::Io(bound_diag(&format!("{path}: not loaded"))));
            }
        }
        Ok(Self {
            queue,
            files: staged,
        })
    }

    /// The bytes of one staged file; the bytes belong to the queue.
    pub fn bytes(&self, path: &str) -> Result<&[u8], KitError> {
        let handle = self
            .files
            .iter()
            .find(|(staged, _)| staged == path)
            .map(|(_, handle)| *handle)
            .ok_or_else(|| KitError::Io(bound_diag(&format!("{path}: not staged"))))?;
        self.queue
            .payload(handle)
            .map_err(|err| KitError::Io(bound_diag(&err.to_string())))
    }

    /// The handle of one staged file.
    pub fn handle(&self, path: &str) -> Option<AssetHandle> {
        self.files
            .iter()
            .find(|(staged, _)| staged == path)
            .map(|(_, handle)| *handle)
    }

    /// Staged files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when nothing is staged.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Bytes resident in the staged files; units: bytes.
    pub fn resident_bytes(&self) -> u64 {
        self.queue.resident_bytes()
    }

    /// Unload every staged file: the handles go stale and the bytes go back
    /// to the budget. Returns the bytes given back; units: bytes.
    pub fn unload(&mut self) -> u64 {
        let mut freed = 0u64;
        for (_, handle) in &self.files {
            if let Ok(LoadEvent::Unloaded { bytes, .. }) = self.queue.unload(*handle) {
                freed = freed.saturating_add(bytes);
            }
        }
        self.files.clear();
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::asset_source::MemorySource;

    /// A source with three kit files of known size.
    fn source() -> MemorySource {
        let mut source = MemorySource::new();
        source
            .insert("src/sand_floor.gltf", vec![1u8; 12])
            .expect("path");
        source
            .insert("src/walls.gltf", vec![2u8; 24])
            .expect("path");
        source
            .insert("src/banner.gltf", vec![3u8; 8])
            .expect("path");
        source
    }

    fn files() -> [&'static str; 3] {
        ["sand_floor.gltf", "walls.gltf", "banner.gltf"]
    }

    #[test]
    fn kit_files_load_by_handle_and_unload_back_to_the_budget() {
        let source = source();
        let mut assets = KitAssets::load(&source, &files()).expect("kit loads");
        assert_eq!(assets.len(), 3);
        assert_eq!(assets.resident_bytes(), 44);
        let path = kit_logical_path("walls.gltf");
        assert_eq!(assets.bytes(&path).expect("bytes").len(), 24);
        let handle = assets.handle(&path).expect("handle");
        assert!(handle.is_valid());
        assert_eq!(assets.bytes(&path).expect("bytes"), &[2u8; 24]);
        // Unload gives every byte back and stales the handles.
        assert_eq!(assets.unload(), 44);
        assert_eq!(assets.resident_bytes(), 0);
        assert!(assets.is_empty());
        assert!(
            assets.bytes(&path).is_err(),
            "an unloaded file reads nothing"
        );
    }

    #[test]
    fn kit_load_reads_the_collision_pieces_before_the_dressing() {
        let source = source();
        let assets = KitAssets::load(&source, &files()).expect("kit loads");
        let ground = assets
            .handle(&kit_logical_path("sand_floor.gltf"))
            .expect("handle");
        let banner = assets
            .handle(&kit_logical_path("banner.gltf"))
            .expect("handle");
        assert!(
            kit_load_priority("sand_floor.gltf") > kit_load_priority("banner.gltf"),
            "the ground and the collision pieces load first"
        );
        assert_eq!(kit_load_priority("sand_floor.gltf"), 2);
        assert_eq!(kit_load_priority("banner.gltf"), 1);
        assert_eq!(kit_load_priority("unknown.gltf"), 0);
        assert!(ground.is_valid() && banner.is_valid());
    }

    #[test]
    fn kit_load_names_the_file_the_source_refuses() {
        let source = source();
        let err = KitAssets::load(&source, &["sand_floor.gltf", "gone.gltf"])
            .expect_err("the missing file fails the load");
        match err {
            KitError::Io(text) => {
                assert!(text.contains("gone.gltf"), "{text}");
                assert!(text.contains("not found"), "{text}");
            }
            other => panic!("expected a kit io failure, got {other:?}"),
        }
    }
}
