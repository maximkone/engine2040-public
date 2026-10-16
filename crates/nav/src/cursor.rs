//! cursor: the one bounds-checked little-endian byte reader behind every nav
//! blob decoder. Running out of input is a typed value, never a panic, and a
//! failed read leaves the position where it was.

/// The input ended before the requested bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Truncated;

/// Forward-only cursor over a byte slice.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Cursor at the start of `bytes`.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Bytes not yet read.
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// The next `count` bytes, or [`Truncated`] when fewer remain.
    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8], Truncated> {
        let end = self.pos.saturating_add(count);
        if end > self.bytes.len() {
            return Err(Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// The next `N` bytes as an array.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Truncated> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// One byte.
    pub(crate) fn byte(&mut self) -> Result<u8, Truncated> {
        Ok(self.take(1)?[0])
    }

    /// Little-endian `u16`.
    pub(crate) fn u16(&mut self) -> Result<u16, Truncated> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    /// Little-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32, Truncated> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Little-endian `i32`.
    pub(crate) fn i32(&mut self) -> Result<i32, Truncated> {
        Ok(i32::from_le_bytes(self.array()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_in_order_and_tracks_remaining() {
        let bytes = [7, 0x34, 0x12, 1, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff, 9];
        let mut read = Cursor::new(&bytes);
        assert_eq!(read.byte(), Ok(7));
        assert_eq!(read.u16(), Ok(0x1234));
        assert_eq!(read.u32(), Ok(1));
        assert_eq!(read.i32(), Ok(-2));
        assert_eq!(read.remaining(), 1);
        assert_eq!(read.take(1), Ok(&bytes[11..]));
        assert_eq!(read.remaining(), 0);
    }

    #[test]
    fn short_input_is_truncated_and_does_not_advance() {
        let bytes = [1, 2, 3];
        let mut read = Cursor::new(&bytes);
        assert_eq!(read.u32(), Err(Truncated));
        assert_eq!(read.remaining(), 3, "a failed read consumes nothing");
        assert_eq!(read.take(usize::MAX), Err(Truncated));
        assert_eq!(read.u16(), Ok(0x0201));
        assert_eq!(read.u16(), Err(Truncated));
        assert_eq!(read.byte(), Ok(3));
        assert_eq!(read.byte(), Err(Truncated));
    }
}
