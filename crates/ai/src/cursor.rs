//! cursor: the one bounded reader over untrusted little-endian bytes.
//!
//! Owner: `engine-ai`. Consumers: memory saves, dialogue bakes and provenance
//! logs, each mapping [`ShortRead`] onto its own typed `Corrupt` error through
//! `From`. A short read consumes nothing, so a refused load never leaves a
//! half-advanced cursor behind.

/// A read asked for more bytes than the input still holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ShortRead;

/// Forward-only cursor over borrowed bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ByteCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    /// Cursor at the start of `bytes`.
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// True once every byte has been consumed (no trailing data).
    pub(crate) const fn is_done(&self) -> bool {
        self.pos == self.bytes.len()
    }

    /// The next `count` bytes, or [`ShortRead`] when fewer remain.
    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8], ShortRead> {
        let end = self.pos.saturating_add(count);
        let slice = self.bytes.get(self.pos..end).ok_or(ShortRead)?;
        self.pos = end;
        Ok(slice)
    }

    /// One byte.
    pub(crate) fn take_u8(&mut self) -> Result<u8, ShortRead> {
        Ok(self.take(1)?[0])
    }

    /// Little-endian `u32`.
    pub(crate) fn take_u32(&mut self) -> Result<u32, ShortRead> {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(raw))
    }

    /// Little-endian `u64`.
    pub(crate) fn take_u64(&mut self) -> Result<u64, ShortRead> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(raw))
    }

    /// Every remaining byte; the cursor is done afterwards.
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let slice = &self.bytes[self.pos..];
        self.pos = self.bytes.len();
        slice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_in_order() {
        let bytes = [7, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 9];
        let mut cursor = ByteCursor::new(&bytes);
        assert_eq!(cursor.take_u8(), Ok(7));
        assert_eq!(cursor.take_u32(), Ok(1));
        assert_eq!(cursor.take_u64(), Ok(2));
        assert!(!cursor.is_done());
        assert_eq!(cursor.rest(), &[9]);
        assert!(cursor.is_done());
    }

    #[test]
    fn short_read_consumes_nothing() {
        let bytes = [1, 2, 3];
        let mut cursor = ByteCursor::new(&bytes);
        assert_eq!(cursor.take_u32(), Err(ShortRead));
        assert_eq!(cursor.take(3), Ok(&bytes[..]));
        assert_eq!(cursor.take_u8(), Err(ShortRead));
        assert!(cursor.is_done());
    }

    #[test]
    fn huge_count_refuses_instead_of_wrapping() {
        let mut cursor = ByteCursor::new(&[0; 4]);
        assert_eq!(cursor.take(usize::MAX), Err(ShortRead));
        assert_eq!(cursor.take_u32(), Ok(0));
    }
}
