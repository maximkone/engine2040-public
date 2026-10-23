//! Pose shelf (W4): named poses over the recorded pose tape.
//!
//! Owner: the editor session. The pose recorder hands a finished [`PoseTape`]
//! here under an artist's name, and the shelf keeps one bounded row per pose:
//! joint and frame counts, the tape digest (the pose identity) and an optional
//! 1x1 RGBA swatch the caller rendered. Nothing here records or replays: the
//! tape does that, and this module never owns scene state.
//! Determinism: rows keep save order, so listing is insertion order.
//! Errors: [`ShelfError`]; a refused save changes nothing.

use crate::pose_tape::PoseTape;

/// Most poses one shelf keeps.
pub const MAX_POSES: usize = 64;
/// Longest pose name in chars.
pub const MAX_NAME_CHARS: usize = 64;

/// One shelf row: the pose's identity, shape and swatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfEntry {
    /// Artist's name for the pose.
    pub name: String,
    /// Rig joint count of the tape.
    pub joints: usize,
    /// Recorded frame count of the tape.
    pub frames: usize,
    /// Tape digest; two saves of the same tape share it.
    pub digest: u64,
    /// 1x1 RGBA swatch supplied by the caller, when it rendered one.
    pub thumb: Option<[u8; 4]>,
}

/// Bounded shelf of named poses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoseShelf {
    entries: Vec<ShelfEntry>,
}

impl PoseShelf {
    /// Empty shelf.
    pub fn new() -> Self {
        Self::default()
    }

    /// Kept pose count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no pose is kept.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every pose in save order.
    pub fn list(&self) -> &[ShelfEntry] {
        &self.entries
    }

    /// One pose by name.
    pub fn get(&self, name: &str) -> Option<&ShelfEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Save one recorded tape under `name`.
    ///
    /// An empty or over-long name, a name already on the shelf and the
    /// [`MAX_POSES`] bound refuse without change. The row keeps the tape's
    /// digest and shape, so the shelf never holds a second copy of the frames.
    pub fn save(
        &mut self,
        name: &str,
        tape: &PoseTape,
        thumb: Option<[u8; 4]>,
    ) -> Result<(), ShelfError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ShelfError::EmptyName);
        }
        let name_len = name.chars().count();
        if name_len > MAX_NAME_CHARS {
            return Err(ShelfError::NameTooLong {
                len: name_len,
                max: MAX_NAME_CHARS,
            });
        }
        if self.get(name).is_some() {
            return Err(ShelfError::Duplicate {
                name: name.to_string(),
            });
        }
        if self.entries.len() >= MAX_POSES {
            return Err(ShelfError::Full {
                len: self.entries.len(),
                max: MAX_POSES,
            });
        }
        self.entries.push(ShelfEntry {
            name: name.to_string(),
            joints: tape.joints(),
            frames: tape.frames(),
            digest: tape.digest(),
            thumb,
        });
        Ok(())
    }
}

/// What refused a save. Nothing was kept on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShelfError {
    /// The name held no text.
    EmptyName,
    /// The name went over the char bound.
    NameTooLong {
        /// Chars given.
        len: usize,
        /// Chars allowed.
        max: usize,
    },
    /// A pose already carries this name.
    Duplicate {
        /// The repeated name.
        name: String,
    },
    /// The shelf already holds [`MAX_POSES`] poses.
    Full {
        /// Poses held now.
        len: usize,
        /// The bound.
        max: usize,
    },
}

impl std::fmt::Display for ShelfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("pose name holds no text; pass 1 to 64 chars"),
            Self::NameTooLong { len, max } => write!(
                f,
                "pose name holds {len} chars over the {max} bound; shorten it to {max} chars"
            ),
            Self::Duplicate { name } => write!(
                f,
                "a pose named `{name}` is already on the shelf; pick a new name or overwrite it"
            ),
            Self::Full { len, max } => write!(
                f,
                "shelf holds {len} of {max} poses; delete one before saving"
            ),
        }
    }
}

impl std::error::Error for ShelfError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(angles: &[i32]) -> PoseTape {
        let mut tape = PoseTape::new(angles.len()).unwrap();
        tape.record(angles).unwrap();
        tape
    }

    #[test]
    fn two_poses_roundtrip_and_refuse_bad_saves() {
        let mut shelf = PoseShelf::new();
        assert!(shelf.is_empty());
        let guard = tape(&[0, -90_000]);
        let thrust = tape(&[45_000, 0, 90_000]);
        shelf
            .save("shield-wall idle", &guard, Some([10, 20, 30, 255]))
            .unwrap();
        shelf.save("spear thrust", &thrust, None).unwrap();

        assert_eq!(shelf.len(), 2);
        assert_eq!(shelf.list()[0].name, "shield-wall idle");
        assert_eq!(shelf.list()[0].joints, 2);
        assert_eq!(shelf.list()[0].frames, 1);
        assert_eq!(shelf.list()[0].thumb, Some([10, 20, 30, 255]));
        let found = shelf.get("spear thrust").unwrap();
        assert_eq!(found.digest, thrust.digest());
        assert_eq!(found.joints, 3);
        assert_eq!(shelf.list()[1].digest, thrust.digest());

        assert_eq!(shelf.save("  ", &guard, None), Err(ShelfError::EmptyName));
        assert_eq!(
            shelf.save("shield-wall idle", &guard, None),
            Err(ShelfError::Duplicate {
                name: "shield-wall idle".to_string()
            })
        );
        let long = "p".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(
            shelf.save(&long, &guard, None),
            Err(ShelfError::NameTooLong {
                len: MAX_NAME_CHARS + 1,
                max: MAX_NAME_CHARS
            })
        );
        // The shelf fills to the bound and then refuses, keeping two rows.
        for n in shelf.len()..MAX_POSES {
            shelf.save(&format!("pose {n}"), &guard, None).unwrap();
        }
        assert_eq!(
            shelf.save("one too many", &guard, None),
            Err(ShelfError::Full {
                len: MAX_POSES,
                max: MAX_POSES
            })
        );
        assert_eq!(shelf.len(), MAX_POSES);
    }
}
