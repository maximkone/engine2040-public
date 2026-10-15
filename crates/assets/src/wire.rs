//! wire: byte helpers shared by the archive and bundle formats, plus the
//! bounded-echo rule for diagnostic text. Std-only; every integer is
//! little-endian, so the same input reads the same on every platform.

/// A read ran past the end of the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Short {
    /// Bytes needed from the input start.
    pub(crate) want: usize,
    /// Bytes actually present.
    pub(crate) got: usize,
}

/// Forward cursor over a byte slice. A short read leaves the position
/// untouched and reports [`Short`]; it never panics and never reads past the
/// slice.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Cursor at the start of `bytes`.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self::at(bytes, 0)
    }

    /// Cursor at byte offset `pos` of `bytes` (past the end reads short).
    pub(crate) fn at(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    /// Bytes consumed so far.
    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    /// Borrow the next `len` bytes.
    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], Short> {
        let end = self.pos.saturating_add(len);
        let Some(out) = self.bytes.get(self.pos..end) else {
            return Err(Short {
                want: end,
                got: self.bytes.len(),
            });
        };
        self.pos = end;
        Ok(out)
    }

    /// Copy the next `N` bytes out.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Short> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// Next byte.
    pub(crate) fn u8(&mut self) -> Result<u8, Short> {
        Ok(self.array::<1>()?[0])
    }

    /// Next little-endian `u16`.
    pub(crate) fn u16(&mut self) -> Result<u16, Short> {
        self.array().map(u16::from_le_bytes)
    }

    /// Next little-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32, Short> {
        self.array().map(u32::from_le_bytes)
    }

    /// Next little-endian `u64`.
    pub(crate) fn u64(&mut self) -> Result<u64, Short> {
        self.array().map(u64::from_le_bytes)
    }
}

/// Append a little-endian `u16`.
pub(crate) fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Append a little-endian `u32`.
pub(crate) fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Append a little-endian `u64`.
pub(crate) fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// `text` cut to at most `max` chars, never splitting a char. Errors echo
/// caller input through this so their values stay small.
pub(crate) fn bound_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_in_order() {
        let bytes = [7, 0x34, 0x12, 1, 2, 3, 4, 9, 0, 0, 0, 0, 0, 0, 0];
        let mut cursor = Cursor::new(&bytes);
        assert_eq!(cursor.u8(), Ok(7));
        assert_eq!(cursor.u16(), Ok(0x1234));
        assert_eq!(cursor.u32(), Ok(0x0403_0201));
        assert_eq!(cursor.u64(), Ok(9));
        assert_eq!(cursor.pos(), bytes.len());
    }

    #[test]
    fn short_read_names_both_sides_and_keeps_position() {
        let bytes = [1, 2, 3];
        let mut cursor = Cursor::new(&bytes);
        assert_eq!(cursor.u8(), Ok(1));
        assert_eq!(cursor.u32(), Err(Short { want: 5, got: 3 }));
        assert_eq!(cursor.pos(), 1, "a refused read consumes nothing");
        assert_eq!(cursor.take(2), Ok(&bytes[1..3]));
        assert_eq!(
            cursor.take(usize::MAX),
            Err(Short {
                want: usize::MAX,
                got: 3
            })
        );
    }

    #[test]
    fn cursor_past_the_end_reads_short_never_panics() {
        let bytes = [0u8; 4];
        let mut cursor = Cursor::at(&bytes, 9);
        assert_eq!(cursor.u8(), Err(Short { want: 10, got: 4 }));
        assert_eq!(cursor.take(0), Err(Short { want: 9, got: 4 }));
    }

    #[test]
    fn writers_and_readers_round_trip() {
        let mut out = Vec::new();
        push_u16(&mut out, 0xBEEF);
        push_u32(&mut out, 0xDEAD_BEEF);
        push_u64(&mut out, 0x0102_0304_0506_0708);
        assert_eq!(out[..2], [0xEF, 0xBE]);
        let mut cursor = Cursor::new(&out);
        assert_eq!(cursor.u16(), Ok(0xBEEF));
        assert_eq!(cursor.u32(), Ok(0xDEAD_BEEF));
        assert_eq!(cursor.u64(), Ok(0x0102_0304_0506_0708));
    }

    #[test]
    fn bound_chars_cuts_on_char_boundaries() {
        assert_eq!(bound_chars("short", 8), "short");
        assert_eq!(bound_chars("abcdef", 3), "abc");
        assert_eq!(bound_chars("héllo wörld", 5), "héllo");
        assert_eq!(bound_chars("anything", 0), "");
    }
}
