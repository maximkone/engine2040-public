//! Stable scene identities and content digests.
//!
//! An authored object keeps its [`ObjectId`] for life (reimport, cook and
//! migration preserve it so references stay valid). What changed is answered
//! by [`content_digest`], a BLAKE3 hash of the canonical bytes. Identity and
//! content version are separate values by design.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Scene schema version carried by saved documents. Bump with a migration.
pub const SCHEMA_VERSION: u32 = 1;

/// Persistent identity of an authored object or asset.
///
/// A random (v4) UUID minted once at authoring time. Copying an object mints
/// a new id; reimporting or cooking the same object keeps it.
///
/// Ordering is by the wrapped UUID value (`as_uuid` order), matching the
/// existing id-sorted traversals; `BTreeMap` iteration over ids is therefore
/// deterministic for a given set of ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectId(Uuid);

/// Rejected [`ObjectId`] text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseIdError;

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid object id")
    }
}

impl std::error::Error for ParseIdError {}

impl ObjectId {
    /// Nil id. Never minted; useful as an explicit "no reference" slot.
    pub const NIL: Self = Self(Uuid::nil());

    /// Mint a fresh random (v4) id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing UUID (import boundaries only).
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// The wrapped UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }

    /// True for the nil id.
    pub fn is_nil(self) -> bool {
        self.0.is_nil()
    }

    /// Parse hyphenated or simple-hex UUID text.
    pub fn parse(text: &str) -> Result<Self, ParseIdError> {
        Uuid::parse_str(text).map(Self).map_err(|_| ParseIdError)
    }
}

impl Default for ObjectId {
    fn default() -> Self {
        Self::NIL
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

impl FromStr for ObjectId {
    type Err = ParseIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// BLAKE3 digest of canonical content bytes, lowercase hex.
///
/// Versions the bytes; it is not an identity. Two objects may share content
/// (same digest) while holding distinct [`ObjectId`]s.
pub fn content_digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_ids_are_unique_v4() {
        let a = ObjectId::new();
        let b = ObjectId::new();
        assert_ne!(a, b);
        assert_eq!(a.as_uuid().get_version_num(), 4);
        assert!(!a.is_nil());
    }

    #[test]
    fn display_parse_roundtrip() {
        let id = ObjectId::new();
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(text.parse::<ObjectId>().unwrap(), id);
        assert_eq!(ObjectId::parse(&text).unwrap(), id);
    }

    #[test]
    fn parse_accepts_simple_hex() {
        let id = ObjectId::new();
        let simple = id.as_uuid().simple().to_string();
        assert_eq!(ObjectId::parse(&simple).unwrap(), id);
    }

    #[test]
    fn parse_rejects_garbage() {
        for bad in ["", "not-an-id", "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz"] {
            assert_eq!(ObjectId::parse(bad), Err(ParseIdError));
        }
    }

    #[test]
    fn serde_roundtrip_is_hyphenated_string() {
        let id = ObjectId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<ObjectId>(&json).unwrap(), id);
    }

    #[test]
    fn digest_matches_blake3_empty_vector() {
        assert_eq!(
            content_digest(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn digest_is_deterministic_and_content_sensitive() {
        assert_eq!(content_digest(b"abc"), content_digest(b"abc"));
        assert_ne!(content_digest(b"abc"), content_digest(b"abd"));
        assert_ne!(content_digest(b"abc"), content_digest(b""));
    }

    #[test]
    fn identity_is_separate_from_content_version() {
        // Same bytes hash alike, yet each authored object owns its id.
        let bytes = b"shared mesh bytes";
        assert_eq!(content_digest(bytes), content_digest(bytes));
        assert_ne!(ObjectId::new(), ObjectId::new());
    }

    #[test]
    fn schema_version_is_current() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn ordering_matches_uuid_order() {
        let a = ObjectId::new();
        let b = ObjectId::new();
        assert_eq!(
            a.cmp(&b),
            a.as_uuid().cmp(&b.as_uuid()),
            "id order must follow wrapped UUID order"
        );
        let mut ids = [b, a];
        ids.sort();
        let mut uuids = [b.as_uuid(), a.as_uuid()];
        uuids.sort();
        assert_eq!(ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>(), uuids);
        // Nil is the zero UUID, so it sorts before any minted id.
        assert!(ObjectId::NIL < a);
        assert!(ObjectId::NIL < b);
    }
}
