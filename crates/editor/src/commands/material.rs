//! Material names for bindings: validated once, copied everywhere.
//!
//! Owner: the command service (re-exported through it); consumers bind via
//! [`Op::SetMaterial`](super::Op::SetMaterial), the save codec and the shell.
//! Units: bytes for lengths. Determinism: fixed 64-byte inline storage with
//! no maps or pointers, so digests and saves are byte-stable. Errors:
//! [`MaterialNameError`]; construction validates, every later use is total.

use std::fmt;

/// Longest accepted material asset name in bytes (asset names are short).
pub const MAX_MATERIAL_NAME_LEN: usize = 64;

/// A validated material asset name: short ASCII (`0-9 A-Z a-z - _ .`).
///
/// Copy-sized so the op enum stays `Copy`: names validate once at the
/// boundary ([`TryFrom`], the shell browser check, the save codec) and every
/// later use is infallible. Empty is unrepresentable: clearing a binding is
/// `None` on the set-material op.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialName {
    /// Name length in bytes (always `1..=MAX_MATERIAL_NAME_LEN`).
    len: u8,
    /// Name bytes, significant up to `len`.
    bytes: [u8; MAX_MATERIAL_NAME_LEN],
}

/// What rejected a material name. Every variant names the offending text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialNameError {
    /// Empty text: nothing to bind.
    Empty,
    /// Longer than [`MAX_MATERIAL_NAME_LEN`] bytes.
    TooLong {
        /// Byte length found.
        len: usize,
    },
    /// A byte outside `0-9 A-Z a-z - _ .`.
    BadChar {
        /// Byte offset of the first offender.
        at: usize,
    },
}

impl fmt::Display for MaterialNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("material name is empty"),
            Self::TooLong { len } => write!(
                f,
                "material name is {len} bytes, past the {MAX_MATERIAL_NAME_LEN}-byte cap"
            ),
            Self::BadChar { at } => {
                write!(f, "material name byte {at} is outside 0-9 A-Z a-z - _ .")
            }
        }
    }
}

impl std::error::Error for MaterialNameError {}

impl MaterialName {
    /// Name text. Total: construction only admits valid ASCII, so decoding
    /// cannot fail; the empty fallback is unreachable.
    pub fn as_str(&self) -> &str {
        let end = usize::from(self.len);
        core::str::from_utf8(&self.bytes[..end]).unwrap_or("")
    }

    /// Name length in bytes as a fixed 8-byte integer for digests.
    pub fn len_bytes(&self) -> u64 {
        u64::from(self.len)
    }
}

impl TryFrom<&str> for MaterialName {
    type Error = MaterialNameError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        if name.is_empty() {
            return Err(MaterialNameError::Empty);
        }
        if name.len() > MAX_MATERIAL_NAME_LEN {
            return Err(MaterialNameError::TooLong { len: name.len() });
        }
        for (at, byte) in name.bytes().enumerate() {
            let ok = byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.';
            if !ok {
                return Err(MaterialNameError::BadChar { at });
            }
        }
        let len =
            u8::try_from(name.len()).map_err(|_| MaterialNameError::TooLong { len: name.len() })?;
        let mut bytes = [0u8; MAX_MATERIAL_NAME_LEN];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self { len, bytes })
    }
}

impl fmt::Display for MaterialName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
