//! cursor: bounds-checked little-endian reads over a byte slice, shared by
//! the snapshot decoders so hostile bytes refuse instead of panicking.
//!
//! Consumer: `bonded::snapshot`, `budget::snapshot` and `snapshot`; each
//! builds the cursor with its own typed error, returned by every failed read.
//! Units: bytes. Determinism: pure over the slice. Budget: `O(1)` per read,
//! no allocation.

/// Read cursor; every read either returns the value and moves on, or
/// returns the cursor's error and leaves the position where it was.
pub(crate) struct Cursor<'a, E: Copy> {
    bytes: &'a [u8],
    at: usize,
    err: E,
}

impl<'a, E: Copy> Cursor<'a, E> {
    /// Cursor at the start of `bytes` that fails with `err`.
    pub(crate) fn new(bytes: &'a [u8], err: E) -> Self {
        Self { bytes, at: 0, err }
    }

    /// True when every byte was consumed.
    pub(crate) fn is_done(&self) -> bool {
        self.at == self.bytes.len()
    }

    /// The next `n` bytes.
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], E> {
        let end = self.at.checked_add(n).ok_or(self.err)?;
        let slice = self.bytes.get(self.at..end).ok_or(self.err)?;
        self.at = end;
        Ok(slice)
    }

    /// The next `N` bytes as an array.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], E> {
        let err = self.err;
        self.take(N)?.try_into().map_err(|_| err)
    }

    /// One byte.
    pub(crate) fn read_u8(&mut self) -> Result<u8, E> {
        self.array::<1>().map(|b| b[0])
    }

    /// One little-endian `u32`.
    pub(crate) fn read_u32(&mut self) -> Result<u32, E> {
        self.array::<4>().map(u32::from_le_bytes)
    }

    /// One little-endian `i32`.
    pub(crate) fn read_i32(&mut self) -> Result<i32, E> {
        self.array::<4>().map(i32::from_le_bytes)
    }

    /// One little-endian `u64`.
    pub(crate) fn read_u64(&mut self) -> Result<u64, E> {
        self.array::<8>().map(u64::from_le_bytes)
    }

    /// One `f32` from its little-endian bits; not finite is an error.
    pub(crate) fn read_finite(&mut self) -> Result<f32, E> {
        let v = f32::from_bits(self.read_u32()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(self.err)
        }
    }

    /// One flag byte: `0` or `1`, anything else is an error.
    pub(crate) fn read_flag(&mut self) -> Result<bool, E> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_values_in_order_and_refuses_past_the_end() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&(-3i32).to_le_bytes());
        bytes.extend_from_slice(&9u64.to_le_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&1.5f32.to_bits().to_le_bytes());
        let mut c = Cursor::new(&bytes, "bad");
        assert_eq!(c.read_u32(), Ok(7));
        assert_eq!(c.read_i32(), Ok(-3));
        assert_eq!(c.read_u64(), Ok(9));
        assert_eq!(c.read_flag(), Ok(true));
        assert_eq!(c.read_finite(), Ok(1.5));
        assert!(c.is_done());
        assert_eq!(c.read_u8(), Err("bad"));
        assert_eq!(c.take(1), Err("bad"));
    }

    #[test]
    fn a_short_read_leaves_the_position_alone() {
        let mut c = Cursor::new(&[1, 2, 3], 0u8);
        assert_eq!(c.read_u32(), Err(0));
        assert_eq!(c.read_u8(), Ok(1));
        assert!(!c.is_done());
    }

    #[test]
    fn flags_and_floats_refuse_bad_values() {
        let mut c = Cursor::new(&[2], ());
        assert_eq!(c.read_flag(), Err(()));
        let nan = f32::NAN.to_bits().to_le_bytes();
        assert_eq!(Cursor::new(&nan, ()).read_finite(), Err(()));
        let inf = f32::INFINITY.to_bits().to_le_bytes();
        assert_eq!(Cursor::new(&inf, ()).read_finite(), Err(()));
    }
}
