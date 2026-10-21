//! Load a baked clip pack from disk.
//!
//! AM27 bakes a teacher clip into an `.am5p` wire and proves it round-trips.
//! But nothing read that file back: `PackedClip::from_bytes` wants bytes, and
//! the tick had no way to get them, so a baked clip could be made and checked
//! and never played. This is that way in: a path in, a clip out, with the wire
//! checked by the same decoder the bake used, so a file that is not a pack
//! refuses loudly instead of sampling noise.
//!
//! The pack carries one track per rig joint, in the rig's own joint order, so
//! the track order is the rig's order and the caller does not remap anything.
//! The names live in the receipt written next to the pack. The pose helper
//! here reads the track for the joint it wants and refuses a pack baked for
//! another rig.

use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

use crate::clip::QClip;
use crate::contract::{LocalPose, Skeleton};
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
    /// The pack carries a different number of tracks than the rig has joints:
    /// it was baked for another rig.
    TrackCount {
        /// Tracks the pack holds.
        got: usize,
        /// Joints the rig has.
        expected: usize,
    },
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
            PackFileError::TrackCount { got, expected } => write!(
                f,
                "pack file: the pack holds {got} tracks, the rig has {expected} joints, it was baked for another rig"
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

/// The pose a loaded pack makes at `time` seconds, for a rig of the same joint
/// count.
///
/// The pack carries one track per rig joint, so the track order is the rig's
/// joint order and the pose needs no remapping. A pack baked for another rig
/// is refused here, not played on the wrong skeleton. Rotations are identity:
/// the pack carries positions only, so the rig's own bind rotations stand.
pub fn sample_pose(
    loaded: &LoadedPack,
    skeleton: &Skeleton,
    time: f32,
) -> Result<LocalPose, PackFileError> {
    let clip = clip_of(loaded);
    if clip.tracks.len() != skeleton.len() {
        return Err(PackFileError::TrackCount {
            got: clip.tracks.len(),
            expected: skeleton.len(),
        });
    }
    let mut positions = Vec::with_capacity(clip.tracks.len());
    for track in &clip.tracks {
        let mut hint = 0usize;
        positions.push(crate::clip::sample_track(track, time, &mut hint));
    }
    let rotations = vec![[0.0, 0.0, 0.0, 1.0]; skeleton.len()];
    Ok(LocalPose {
        positions,
        rotations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::qclip_hash;
    use crate::contract::Joint;

    /// Repository root, one level above the crate.
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    /// `qclip_hash` two seed-7 tool runs printed; the committed pack is one run.
    const SEED7_QCLIP_HASH: u64 = 0xbc8c_1300_6a5a_ad0a;
    /// The fighter rig's joint count: one track per joint.
    const FIGHTER_JOINTS: usize = 65;

    /// A rig of `n` joints in one chain: enough to play a pack on.
    fn chain(n: usize) -> Skeleton {
        let mut joints = Vec::with_capacity(n);
        for index in 0..n {
            joints.push(Joint {
                name: format!("j{index}"),
                parent: if index == 0 { None } else { Some(index - 1) },
            });
        }
        Skeleton { joints }
    }

    #[test]
    fn pack_file_loads_the_committed_fixture() {
        let path = std::path::Path::new(ROOT).join("fixtures/clips/stagger.am5p");
        let loaded = load_pack(&path).expect("the committed pack loads");
        assert_eq!(&loaded.bytes[..4], b"AM5P", "the wire magic");
        assert_eq!(
            loaded.clip.tracks.len(),
            FIGHTER_JOINTS,
            "the pack carries the whole fighter body"
        );
        assert_eq!(
            qclip_hash(&clip_of(&loaded)),
            SEED7_QCLIP_HASH,
            "the seed-7 clip the bake printed"
        );
    }

    #[test]
    fn pack_file_plays_a_pose_on_a_rig_of_the_same_size() {
        let path = std::path::Path::new(ROOT).join("fixtures/clips/stagger.am5p");
        let loaded = load_pack(&path).expect("the committed pack loads");
        let rig = chain(FIGHTER_JOINTS);
        let pose = sample_pose(&loaded, &rig, 0.5).expect("the pack plays");
        assert_eq!(
            pose.positions.len(),
            FIGHTER_JOINTS,
            "one position per joint"
        );
        assert_eq!(
            pose.rotations.len(),
            FIGHTER_JOINTS,
            "one rotation per joint"
        );
        assert!(
            pose.positions.iter().flatten().all(|v| v.is_finite()),
            "every position is finite"
        );
        // The rig holds its own stance: the tracks are the fighter's local
        // offsets, so the lowest one is the ankle's bind offset and the root
        // carries the standing height. A pose that drifted would miss both.
        let hips = pose.positions[0];
        assert!(
            (hips[1] - 0.95).abs() < 0.25,
            "the rig stands at its own height: {hips:?}"
        );
        let lowest = pose
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min);
        assert!(
            (lowest + 0.504).abs() < 0.02,
            "the feet carry the rig's own bind offset: {lowest}"
        );
    }

    #[test]
    fn pack_file_refuses_a_pack_baked_for_another_rig() {
        let path = std::path::Path::new(ROOT).join("fixtures/clips/stagger.am5p");
        let loaded = load_pack(&path).expect("the committed pack loads");
        let err = sample_pose(&loaded, &chain(40), 0.5).expect_err("another rig refuses loud");
        match err {
            PackFileError::TrackCount { got, expected } => {
                assert_eq!(got, FIGHTER_JOINTS, "the pack's track count");
                assert_eq!(expected, 40, "the rig's joint count");
            }
            other => panic!("expected the track count to refuse, got {other}"),
        }
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
