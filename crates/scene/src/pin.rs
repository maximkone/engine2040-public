//! Run pins for pre-start gates: every `play.step`/`play.run` pins the
//! scene it plays or refuses before simulating.
//!
//! Owner: the scene crate (identity plus content version, never one for
//! the other). Units: revisions count applies, digests are lowercase hex.
//! Determinism: nodes hash in id-sorted order, so insertion order never
//! moves a digest. Errors: [`PinError`], each naming the recovery.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::codec::SceneNode;
use super::ids::content_digest;

/// Pin format version stamped on every [`ScenePin`]. Bump with a migration.
pub const RUN_PIN_VERSION: u32 = 1;

/// A pinned scene for one run manifest: the exact revision plus the content
/// digest of the canonical bytes it plays.
///
/// A run carries this in its manifest; a run without one is refused by
/// [`ScenePin::require`], never simulated silently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePin {
    /// Pin format version; see [`RUN_PIN_VERSION`].
    pub version: u32,
    /// Scene schema version of the pinned bytes.
    pub schema: u32,
    /// Editor document revision pinned; 0 never pins (see [`PinError::EmptyArena`]).
    pub revision: u64,
    /// [`content_digest`] of the canonical save bytes at `revision`.
    pub digest: String,
}

/// What went wrong pinning a scene for a run.
///
/// Every variant names the recovery, so the door can quote it in refusals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinError {
    /// No pin was offered; unpinned runs never simulate.
    MissingPin,
    /// Revision 0 names the empty, unauthored arena; author first.
    EmptyArena,
    /// The document moved past the pinned revision.
    StalePin {
        /// Revision the pin holds.
        expected: u64,
        /// Live revision found.
        got: u64,
    },
    /// The live bytes no longer hash to the pinned digest.
    DigestMismatch,
}

impl fmt::Display for PinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPin => write!(
                f,
                "run needs a scene pin; call scene.read, then start again with scene_rev set to its revision"
            ),
            Self::EmptyArena => write!(
                f,
                "scene pin at revision 0 names the empty arena; apply one edit, \
                 then start again with scene_rev set to the new revision"
            ),
            Self::StalePin { expected, got } => write!(
                f,
                "scene pin holds revision {expected}, the document is at {got}; \
                 re-read the scene, then pin again"
            ),
            Self::DigestMismatch => write!(
                f,
                "scene bytes drifted from the pinned digest; re-read the scene, \
                 then pin again"
            ),
        }
    }
}

impl std::error::Error for PinError {}

impl ScenePin {
    /// Demand a pin before a run starts: `None` refuses with
    /// [`PinError::MissingPin`] and a revision-0 pin with
    /// [`PinError::EmptyArena`], so an empty arena never plays silently.
    pub fn require(pin: Option<Self>) -> Result<Self, PinError> {
        let pin = pin.ok_or(PinError::MissingPin)?;
        if pin.revision == 0 {
            return Err(PinError::EmptyArena);
        }
        Ok(pin)
    }

    /// Check this pin against the live revision plus the digest of the live
    /// bytes. A moved revision fails with [`PinError::StalePin`] naming both
    /// sides; drifted bytes fail with [`PinError::DigestMismatch`].
    pub fn check(&self, revision: u64, digest: &str) -> Result<(), PinError> {
        if self.revision != revision {
            return Err(PinError::StalePin {
                expected: self.revision,
                got: revision,
            });
        }
        if self.digest != digest {
            return Err(PinError::DigestMismatch);
        }
        Ok(())
    }
}

/// Digest of canonical save bytes: the format version plus every node in
/// id-sorted order (id, parent, then the ten pose lanes as bits).
///
/// Sorting keeps the digest independent of text order, matching decode,
/// which accepts any order. Lanes hash as `to_bits`, so the same finite
/// poses always hash alike; non-finite poses never reach a pin because
/// decode refuses them first.
pub fn canonical_digest(version: u32, nodes: &[SceneNode]) -> String {
    let mut ordered: Vec<&SceneNode> = nodes.iter().collect();
    ordered.sort_by_key(|node| node.id.as_uuid());
    let mut bytes = Vec::with_capacity(4 + ordered.len() * 72);
    bytes.extend_from_slice(&version.to_le_bytes());
    for node in ordered {
        bytes.extend_from_slice(node.id.as_uuid().as_bytes());
        bytes.extend_from_slice(node.parent.as_uuid().as_bytes());
        for lane in node
            .transform
            .position
            .iter()
            .chain(node.transform.rotation.iter())
            .chain(node.transform.scale.iter())
        {
            bytes.extend_from_slice(&lane.to_bits().to_le_bytes());
        }
    }
    content_digest(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Scene;
    use crate::transform::{Hierarchy, Transform};

    fn one_node_scene() -> Scene {
        let mut tree = Hierarchy::new();
        let id = crate::ids::ObjectId::new();
        tree.insert(
            id,
            crate::ids::ObjectId::NIL,
            Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        )
        .expect("test hierarchy inserts");
        Scene::encode(&tree)
    }

    #[test]
    fn unpinned_run_is_refused_with_a_named_error() {
        let refused = ScenePin::require(None).expect_err("no pin must refuse");
        assert_eq!(refused, PinError::MissingPin);
        assert!(refused.to_string().contains("scene pin"));
        assert!(refused.to_string().contains("scene_rev"));
    }

    #[test]
    fn revision_zero_never_pins_silently() {
        let scene = one_node_scene();
        let refused = scene.pin(0).expect_err("revision 0 must refuse");
        assert_eq!(refused, PinError::EmptyArena);
        assert!(refused.to_string().contains("revision 0"));
        assert!(refused.to_string().contains("scene_rev"));
    }

    #[test]
    fn pinned_run_carries_the_pin_in_its_manifest() {
        let scene = one_node_scene();
        let pin = scene.pin(7).expect("revision 7 pins");
        assert_eq!(pin.revision, 7);
        assert_eq!(pin.digest, scene.digest());
        let held = ScenePin::require(Some(pin.clone())).expect("held pin passes");
        assert_eq!(held, pin);
        let text = serde_json::to_string(&pin).expect("pin rides the manifest");
        let again: ScenePin = serde_json::from_str(&text).expect("manifest reads back");
        assert_eq!(again, pin);
    }

    #[test]
    fn stale_pin_refuses_naming_both_revisions() {
        let scene = one_node_scene();
        let pin = scene.pin(7).expect("revision 7 pins");
        let refused = pin
            .check(8, &pin.digest)
            .expect_err("moved revision must refuse");
        assert_eq!(
            refused,
            PinError::StalePin {
                expected: 7,
                got: 8
            }
        );
        assert!(refused.to_string().contains('7'));
        assert!(refused.to_string().contains('8'));
    }

    #[test]
    fn digest_drift_refuses_against_the_live_bytes() {
        let scene = one_node_scene();
        let pin = scene.pin(7).expect("revision 7 pins");
        let other = Scene::encode(&Hierarchy::new());
        assert_ne!(other.digest(), pin.digest, "empty scene digests differ");
        let refused = pin
            .check(7, &other.digest())
            .expect_err("drifted bytes must refuse");
        assert_eq!(refused, PinError::DigestMismatch);
    }

    #[test]
    fn digest_is_deterministic_and_content_sensitive() {
        let first = one_node_scene();
        assert_eq!(first.digest(), first.digest());
        let empty = Scene::encode(&Hierarchy::new());
        assert_ne!(first.digest(), empty.digest());
    }
}
