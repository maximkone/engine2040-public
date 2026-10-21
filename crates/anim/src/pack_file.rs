//! Load a baked clip pack from disk.
//!
//! AM27 bakes a teacher clip into an `.am5p` wire and proves it round-trips.
//! But nothing read that file back: `PackedClip::from_bytes` wants bytes, and
//! the tick had no way to get them, so a baked clip could be made and checked
//! and never played. This is that way in: a path in, a clip out, with the wire
//! checked by the same decoder the bake used, so a file that is not a pack
//! refuses loudly instead of sampling noise.
//!
//! The pack carries no track names: the names live in the receipt written next
//! to it, and the order is the rig's order. So the caller says which tracks it
//! asked for, and this file only promises the bytes and the clip.

use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

use crate::clip::QClip;
use crate::pack::{unpack_qclip, PackError, PackedClip};

/// A pack file read from disk, with the bytes it came from.
#[derive(Clone, Debug)]
pub struct LoadedPack {
    /// Where the file was read from.
    pub path: PathBuf,
    /// The file's bytes, exactly as read.
    pub bytes: Vec<u8>,
    /// The decoded pack.
    pub clip: PackedClip,
}

/// Why a pack could not be loaded.
#[derive(Debug)]
pub enum PackFileError {
    /// The file could not be read.
    Io {
        /// The path that was tried.
        path: PathBuf,
        /// What the file system said.
        why: std::io::Error,
    },
    /// The bytes are not a pack this engine knows, or they do not decode back
    /// to the same pack (a lossy pack refuses, never loads).
    Pack(PackError),
    /// The bytes decode, but not back to themselves.
    NotRoundTrip,
}

impl fmt::Display for PackFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackFileError::Io { path, why } => {
                write!(f, "pack file {}: {why}", path.display())
            }
            PackFileError::Pack(why) => write!(f, "pack file: {why}"),
            PackFileError::NotRoundTrip => write!(
                f,
                "pack file: the bytes do not decode back to the same pack, refusing a lossy pack"
            ),
        }
    }
}

impl std::error::Error for PackFileError {}

impl From<PackError> for PackFileError {
    fn from(cause: PackError) -> Self {
        PackFileError::Pack(cause)
    }
}

/// Read a pack file and decode it.
///
/// The file is checked twice: the wire decoder names a broken header, and the
/// decoded pack must re-encode to the same bytes. A file that was cut in half
/// or edited therefore refuses instead of playing wrong motion.
///
/// Native only: a browser tab has no file system, so the wasm build gets its
/// pack bytes from the loader that fetched them, not from a path.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_pack(path: &Path) -> Result<LoadedPack, PackFileError> {
    let bytes = std::fs::read(path).map_err(|why| PackFileError::Io {
        path: path.to_path_buf(),
        why,
    })?;
    let clip = PackedClip::from_bytes(&bytes)?;
    if clip.to_bytes() != bytes {
        return Err(PackFileError::NotRoundTrip);
    }
    Ok(LoadedPack {
        path: path.to_path_buf(),
        bytes,
        clip,
    })
}

/// The clip inside a loaded pack, in the rig's track order.
pub fn clip_of(loaded: &LoadedPack) -> QClip {
    unpack_qclip(&loaded.clip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::qclip_hash;

    /// Repository root, one level above the crate.
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    /// `qclip_hash` two seed-7 tool runs printed; the committed pack is one run.
    const SEED7_QCLIP_HASH: u64 = 0x34f7_2e4f_8b72_217a;

    #[test]
    fn pack_file_loads_the_committed_fixture() {
        let path = std::path::Path::new(ROOT).join("fixtures/clips/stagger.am5p");
        let loaded = load_pack(&path).expect("the committed pack loads");
        assert_eq!(&loaded.bytes[..4], b"AM5P", "the wire magic");
        assert_eq!(loaded.clip.tracks.len(), 2, "left foot then right foot");
        assert_eq!(
            qclip_hash(&clip_of(&loaded)),
            SEED7_QCLIP_HASH,
            "the seed-7 clip the bake printed"
        );
    }

    #[test]
    fn pack_file_refuses_a_file_that_is_not_there() {
        let path = std::path::Path::new(ROOT).join("fixtures/clips/no-such-pack.am5p");
        let err = load_pack(&path).expect_err("a missing file refuses loud");
        match err {
            PackFileError::Io { path: said, .. } => assert_eq!(said, path, "the path is named"),
            other => panic!("expected the read to fail, got {other}"),
        }
    }

    #[test]
    fn pack_file_refuses_bytes_that_are_not_a_pack() {
        let err = PackedClip::from_bytes(b"this is not a pack at all")
            .expect_err("the wire check names the fault");
        assert!(matches!(
            err,
            PackError::BadMagic { .. } | PackError::BadVersion { .. }
        ));
    }
}
