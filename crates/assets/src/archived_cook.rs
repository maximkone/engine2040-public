//! archived_cook: validated cook envelope borrowed in place, no decode.
//! Owner `engine-assets`; consumer mmap load path via `wrap_archive_entry`; offline only.
//! PORT(rkyv/rkyv@d963abc24301a571605a9bafd963dfed72187fd7:archive validate-once then borrow in place).
use super::archive::{ArchiveError, ArchiveKind, ArchivedBundle, MAX_ARCHIVE_PAYLOAD_BYTES};
use super::wire::{push_u32, push_u64, Cursor};
use engine_core::hash_bytes;
/// Eight-byte magic opening every cook envelope: `E204COOK`.
pub const COOK_MAGIC: &[u8; 8] = b"E204COOK";
/// Cook schema version written by [`seal`].
pub const COOK_VERSION: u32 = 1;
const COOK_HEADER_LEN: usize = 29;
const COOK_TRAILER_LEN: usize = 8;
/// Seal `root` under `kind`. Same inputs always emit the same bytes.
pub fn seal(kind: ArchiveKind, root: &[u8]) -> Result<Vec<u8>, ArchiveError> {
    if root.len() > MAX_ARCHIVE_PAYLOAD_BYTES {
        return Err(ArchiveError::TooLarge { count: root.len() });
    }
    let len =
        u32::try_from(root.len()).map_err(|_| ArchiveError::TooLarge { count: root.len() })?;
    let off = u32::try_from(COOK_HEADER_LEN)
        .map_err(|_| ArchiveError::Corrupt("cook header offset range".to_string()))?;
    let mut out = Vec::with_capacity(COOK_HEADER_LEN + root.len() + COOK_TRAILER_LEN);
    out.extend_from_slice(COOK_MAGIC);
    push_u32(&mut out, COOK_VERSION);
    out.push(match kind {
        ArchiveKind::Mesh => 1,
        ArchiveKind::Material => 2,
        ArchiveKind::Texture => 3,
        ArchiveKind::Skin => 4,
        ArchiveKind::Clip => 5,
    });
    push_u32(&mut out, off);
    push_u32(&mut out, len);
    push_u64(&mut out, hash_bytes(root));
    out.extend_from_slice(root);
    let check = hash_bytes(&out);
    push_u64(&mut out, check);
    Ok(out)
}
/// Validated envelope: `wrap` checks once, accessors borrow with no copy.
#[derive(Clone, Copy, Debug)]
pub struct ArchivedCook<'a> {
    kind: ArchiveKind,
    root: &'a [u8],
}
impl<'a> ArchivedCook<'a> {
    /// Validate once then borrow in place; no copy, no allocation, no decode.
    pub fn wrap(bytes: &'a [u8]) -> Result<Self, ArchiveError> {
        let mut head = Cursor::new(bytes);
        if head.take(8)? != COOK_MAGIC {
            return Err(ArchiveError::BadMagic);
        }
        let version = head.u32()?;
        if version > COOK_VERSION {
            return Err(ArchiveError::FutureVersion { found: version });
        }
        if version != COOK_VERSION {
            return Err(ArchiveError::Corrupt(format!("bad cook version {version}")));
        }
        let Some(kind) = ArchiveKind::from_byte(head.u8()?) else {
            return Err(ArchiveError::Corrupt("bad cook kind".to_string()));
        };
        let offset = usize::try_from(head.u32()?)
            .map_err(|_| ArchiveError::Corrupt("cook root offset range".to_string()))?;
        let len = usize::try_from(head.u32()?).map_err(|_| ArchiveError::TooLarge {
            count: MAX_ARCHIVE_PAYLOAD_BYTES,
        })?;
        if len > MAX_ARCHIVE_PAYLOAD_BYTES {
            return Err(ArchiveError::TooLarge { count: len });
        }
        let stored = head.u64()?;
        if offset != head.pos() {
            return Err(ArchiveError::Corrupt("cook root offset moved".to_string()));
        }
        let root = head.take(len)?;
        let stored_envelope = head.u64()?;
        if head.pos() != bytes.len() {
            return Err(ArchiveError::Corrupt("cook trailing bytes".to_string()));
        }
        if hash_bytes(&bytes[..head.pos() - COOK_TRAILER_LEN]) != stored_envelope {
            return Err(ArchiveError::Corrupt("cook checksum mismatch".to_string()));
        }
        if hash_bytes(root) != stored {
            return Err(ArchiveError::DigestMismatch {
                entry: kind.label().to_string(),
                expected: format!("{stored:016x}"),
                actual: format!("{:016x}", hash_bytes(root)),
            });
        }
        Ok(Self { kind, root })
    }
    /// Borrowed root bytes; zero copy, zero allocation.
    pub fn root(self) -> &'a [u8] {
        self.root
    }
    /// Mesh bytes where the layout allows (kind mesh only).
    pub fn mesh_vertices(self) -> Option<&'a [u8]> {
        (self.kind == ArchiveKind::Mesh).then_some(self.root)
    }
    /// Clip bytes where the layout allows (kind clip only).
    pub fn clip_samples(self) -> Option<&'a [u8]> {
        (self.kind == ArchiveKind::Clip).then_some(self.root)
    }
}
/// Wrap one archive entry as a cook envelope from the shared mapping.
pub fn wrap_archive_entry<'b>(
    bundle: &'b ArchivedBundle,
    kind: ArchiveKind,
    name: &str,
) -> Result<ArchivedCook<'b>, ArchiveError> {
    let Some(entry) = bundle.find(kind, name) else {
        return Err(ArchiveError::Corrupt(format!("missing cook entry {name}")));
    };
    ArchivedCook::wrap(entry.bytes)
}
#[cfg(test)]
mod tests {
    use super::super::archive::{ArchiveWriter, ArchivedBundle};
    use super::*;
    use std::sync::Arc;
    fn payload(byte: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| byte.wrapping_add(i as u8)).collect()
    }
    const SHAPES: [(ArchiveKind, &str, u8, usize); 5] = [
        (ArchiveKind::Mesh, "hero", 0x11, 1024),
        (ArchiveKind::Material, "hero-mat", 0x22, 64),
        (ArchiveKind::Texture, "hero-albedo", 0x33, 4096),
        (ArchiveKind::Skin, "hero-skin", 0x44, 512),
        (ArchiveKind::Clip, "hero-step", 0x55, 2048),
    ];
    fn build() -> Vec<u8> {
        let mut w = ArchiveWriter::new();
        for (kind, name, byte, len) in SHAPES {
            let cooked = seal(kind, &payload(byte, len)).unwrap();
            w.stage(kind, name, &cooked).unwrap();
        }
        w.finish()
    }
    #[test]
    fn archived_reads_in_place() {
        let bytes = build();
        assert_eq!(bytes, build(), "finish x2 identical");
        let mapping: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let open = ArchivedBundle::open(Arc::clone(&mapping)).unwrap();
        assert!(Arc::ptr_eq(open.buffer(), &mapping));
        for (kind, name, byte, len) in SHAPES {
            let served = wrap_archive_entry(&open, kind, name).unwrap();
            assert_eq!(served.root(), payload(byte, len).as_slice());
            let (base, at) = (mapping.as_ptr() as usize, served.root().as_ptr() as usize);
            assert!(base <= at && at + len <= base + mapping.len());
            let again = ArchivedCook::wrap(open.find(kind, name).unwrap().bytes).unwrap();
            assert!(std::ptr::eq(served.root(), again.root()));
            assert_eq!(served.mesh_vertices().is_some(), kind == ArchiveKind::Mesh);
            assert_eq!(served.clip_samples().is_some(), kind == ArchiveKind::Clip);
        }
    }
    fn err(bytes: &[u8]) -> ArchiveError {
        ArchivedCook::wrap(bytes).unwrap_err()
    }
    #[test]
    fn rejects_bad_inputs_typed() {
        let good = seal(ArchiveKind::Mesh, &payload(0x11, 32)).unwrap();
        let mut bad = good.clone();
        bad[0] ^= 0xFF;
        assert_eq!(err(&bad), ArchiveError::BadMagic);
        let mut future = good.clone();
        future[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(err(&future), ArchiveError::FutureVersion { found: 2 });
        assert!(matches!(err(&good[..10]), ArchiveError::Truncated { .. }));
        let trailing = [good.clone(), vec![0xAA]].concat();
        assert!(matches!(err(&trailing), ArchiveError::Corrupt(_)));
        let mut flip = good.clone();
        flip[COOK_HEADER_LEN] ^= 0xFF;
        let at = flip.len() - COOK_TRAILER_LEN;
        let stamped = hash_bytes(&flip[..at]).to_le_bytes();
        flip[at..].copy_from_slice(&stamped);
        assert!(matches!(err(&flip), ArchiveError::DigestMismatch { .. }));
    }
}
