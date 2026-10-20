//! npz: the smallest reader/writer of `.npz` (zip of `.npy`) this bake needs.
//!
//! Kimodo writes its teacher clips with `numpy.savez_compressed`, so a bake has
//! to read deflated zip entries; the tool's own `synth` mode writes stored
//! entries plus the CRC the zip format asks for. One `.npy` parser covers both:
//! little-endian `f4`, `f8`, `i8`, `i4`, `u1` and `b1`, C order only (NumPy's
//! default). Fortran order, big-endian, object arrays and a missing magic are
//! refused by name, never guessed.

use std::collections::BTreeMap;

/// Why an `.npz` or `.npy` blob was refused; each variant names the fault.
#[derive(Clone, Debug, PartialEq)]
pub enum NpzError {
    /// Not a zip: the end-of-central-directory record is missing.
    NotZip,
    /// A zip entry is not stored (0) or deflated (8).
    UnsupportedMethod(u16),
    /// A deflated entry did not inflate.
    BadDeflate,
    /// The zip layout points outside the file or overlaps itself.
    BadZip(&'static str),
    /// The `.npy` magic is missing.
    NotNpy,
    /// The `.npy` header is malformed; the field is carried.
    BadHeader(&'static str),
    /// The `.npy` element type is not one this reader knows.
    BadDescr(String),
    /// The array is Fortran-ordered; only C order is read.
    FortranOrder,
    /// The byte count does not match the shape and element size.
    BadLength {
        /// Bytes the shape asks for.
        want: usize,
        /// Bytes the entry holds.
        got: usize,
    },
    /// A wanted array is missing from the archive; the name is carried.
    Missing(String),
}

impl core::fmt::Display for NpzError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotZip => write!(f, "npz: no zip end record"),
            Self::UnsupportedMethod(method) => write!(f, "npz: zip method {method} unsupported"),
            Self::BadDeflate => write!(f, "npz: deflate stream is broken"),
            Self::BadZip(what) => write!(f, "npz: bad zip {what}"),
            Self::NotNpy => write!(f, "npz: entry is not a .npy array"),
            Self::BadHeader(field) => write!(f, "npz: .npy header field {field}"),
            Self::BadDescr(descr) => write!(f, "npz: element type {descr} unsupported"),
            Self::FortranOrder => write!(f, "npz: Fortran order is not read"),
            Self::BadLength { want, got } => {
                write!(f, "npz: shape wants {want} bytes, entry holds {got}")
            }
            Self::Missing(name) => write!(f, "npz: no array named {name}"),
        }
    }
}

impl std::error::Error for NpzError {}

/// One `.npy` array: element type, shape and raw little-endian payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Npy {
    /// NumPy `descr`, kept verbatim (`<f4`, `<i8`, ...).
    pub descr: String,
    /// C-order shape; a scalar has no entries.
    pub shape: Vec<usize>,
    /// Raw little-endian bytes, C order.
    pub data: Vec<u8>,
}

impl Npy {
    /// Element size in bytes, or `None` for a type this reader refuses.
    fn elem_size(&self) -> Option<usize> {
        match self.descr.as_str() {
            "<f4" | "<i4" => Some(4),
            "<f8" | "<i8" => Some(8),
            "|u1" | "|b1" => Some(1),
            _ => None,
        }
    }

    /// Element count from the shape; a scalar reads one.
    pub fn element_count(&self) -> usize {
        if self.shape.is_empty() {
            1
        } else {
            self.shape.iter().product()
        }
    }

    /// Payload as `f32`; `f8` narrows, integers convert. Refused types and a
    /// short payload name their fault.
    pub fn as_f32(&self) -> Result<Vec<f32>, NpzError> {
        Ok(match self.descr.as_str() {
            "<f4" => self
                .data
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect(),
            "<f8" => self
                .data
                .chunks_exact(8)
                .map(|b| {
                    f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
                })
                .collect(),
            "<i4" => self
                .data
                .chunks_exact(4)
                .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32)
                .collect(),
            "<i8" => self
                .data
                .chunks_exact(8)
                .map(|b| {
                    i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
                })
                .collect(),
            other => return Err(NpzError::BadDescr(other.to_string())),
        })
    }

    /// Payload as `i64`; a scalar is the only shape this is used for.
    pub fn as_i64(&self) -> Result<Vec<i64>, NpzError> {
        Ok(match self.descr.as_str() {
            "<i8" => self
                .data
                .chunks_exact(8)
                .map(|b| i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
                .collect(),
            "<i4" => self
                .data
                .chunks_exact(4)
                .map(|b| i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])))
                .collect(),
            other => return Err(NpzError::BadDescr(other.to_string())),
        })
    }
}

/// One archive: entry name -> array, in name order so reads never depend on
/// zip order.
#[derive(Clone, Debug, Default)]
pub struct Npz {
    arrays: BTreeMap<String, Npy>,
}

impl Npz {
    /// Read arrays from zip bytes; stored and deflated entries both load.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NpzError> {
        let mut arrays = BTreeMap::new();
        for (name, payload) in zip_entries(bytes)? {
            if !name.ends_with(".npy") {
                continue;
            }
            let stem = name.trim_end_matches(".npy").to_string();
            arrays.insert(stem, parse_npy(&payload)?);
        }
        Ok(Self { arrays })
    }

    /// One array by its `.npy` stem.
    pub fn get(&self, name: &str) -> Result<&Npy, NpzError> {
        self.arrays
            .get(name)
            .ok_or_else(|| NpzError::Missing(name.to_string()))
    }

    /// Entries in name order, for the fixture test's shape check.
    #[cfg(test)]
    pub fn names(&self) -> Vec<&str> {
        self.arrays.keys().map(String::as_str).collect()
    }
}

/// Walk the zip central directory and return every entry's data in order.
fn zip_entries(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, NpzError> {
    let mut eocd = None;
    let start = bytes.len().saturating_sub(22 + 65_535);
    let mut at = bytes.len().saturating_sub(22);
    loop {
        if at >= start && bytes.len() >= at + 4 && u32_at(bytes, at)? == 0x0605_4b50 {
            eocd = Some(at);
            break;
        }
        if at == 0 {
            break;
        }
        at -= 1;
    }
    let eocd = eocd.ok_or(NpzError::NotZip)?;
    let count = u16_at(bytes, eocd + 10)? as usize;
    let cd_offset = u32_at(bytes, eocd + 16)? as usize;
    let mut entries = Vec::with_capacity(count);
    let mut pos = cd_offset;
    for _ in 0..count {
        if u32_at(bytes, pos)? != 0x0201_4b50 {
            return Err(NpzError::BadZip("central directory"));
        }
        let method = u16_at(bytes, pos + 10)?;
        let compressed = u32_at(bytes, pos + 20)? as usize;
        let name_len = u16_at(bytes, pos + 28)? as usize;
        let extra_len = u16_at(bytes, pos + 30)? as usize;
        let comment_len = u16_at(bytes, pos + 32)? as usize;
        let local = u32_at(bytes, pos + 42)? as usize;
        let name_bytes = slice(bytes, pos + 46, name_len)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        pos += 46 + name_len + extra_len + comment_len;
        // Local header: 30 bytes plus its own name and extra lengths.
        if u32_at(bytes, local)? != 0x0403_4b50 {
            return Err(NpzError::BadZip("local header"));
        }
        let local_name = u16_at(bytes, local + 26)? as usize;
        let local_extra = u16_at(bytes, local + 28)? as usize;
        let data_at = local + 30 + local_name + local_extra;
        let raw = slice(bytes, data_at, compressed)?;
        let payload = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec(raw).map_err(|_| NpzError::BadDeflate)?,
            other => return Err(NpzError::UnsupportedMethod(other)),
        };
        entries.push((name, payload));
    }
    Ok(entries)
}

fn slice(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], NpzError> {
    bytes
        .get(at..at.saturating_add(len))
        .ok_or(NpzError::BadZip("truncated"))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, NpzError> {
    let b = slice(bytes, at, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, NpzError> {
    let b = slice(bytes, at, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parse one `.npy` payload: magic, header dict, then the raw data.
fn parse_npy(bytes: &[u8]) -> Result<Npy, NpzError> {
    const MAGIC: [u8; 6] = [0x93, b'N', b'U', b'M', b'P', b'Y'];
    if bytes.len() < 10 || bytes[..6] != MAGIC {
        return Err(NpzError::NotNpy);
    }
    if bytes[6] != 1 {
        return Err(NpzError::BadHeader("version"));
    }
    let header_len = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let header = slice(bytes, 10, header_len)?;
    let text = String::from_utf8_lossy(header).into_owned();
    let descr = quoted(&text, "descr").ok_or(NpzError::BadHeader("descr"))?;
    if text.contains("'fortran_order': True") || text.contains("\"fortran_order\": True") {
        return Err(NpzError::FortranOrder);
    }
    let shape = shape_of(&text).ok_or(NpzError::BadHeader("shape"))?;
    let probe = Npy {
        descr: descr.clone(),
        shape: shape.clone(),
        data: Vec::new(),
    };
    let size = probe.elem_size().ok_or(NpzError::BadDescr(descr.clone()))?;
    let want = probe
        .element_count()
        .checked_mul(size)
        .ok_or(NpzError::BadHeader("shape"))?;
    let data = slice(bytes, 10 + header_len, want)?;
    if data.len() != want {
        return Err(NpzError::BadLength {
            want,
            got: data.len(),
        });
    }
    Ok(Npy {
        descr,
        shape,
        data: data.to_vec(),
    })
}

/// The value of `'key': '<value>'` inside an `.npy` header dict.
fn quoted(header: &str, key: &str) -> Option<String> {
    let at = header.find(&format!("'{key}':"))?;
    let rest = &header[at + key.len() + 3..];
    let open = rest.find('\'')?;
    let close = rest[open + 1..].find('\'')?;
    Some(rest[open + 1..open + 1 + close].to_string())
}

/// `'shape': (3, 4)` (or `()`) as a list of sizes.
fn shape_of(header: &str) -> Option<Vec<usize>> {
    let at = header.find("'shape':")?;
    let rest = &header[at + "'shape':".len()..];
    let open = rest.find('(')?;
    let close = rest[open..].find(')')? + open;
    let mut shape = Vec::new();
    for part in rest[open + 1..close].split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        shape.push(part.parse::<usize>().ok()?);
    }
    Some(shape)
}

/// Write a `.npz` with stored entries and the CRC a zip reader asks for. The
/// arrays are written in the order given, so the same arrays give the same
/// bytes on every run.
pub fn write_npz(arrays: &[(&str, Npy)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut crc = Crc32::new();
    for (name, array) in arrays {
        let file = format!("{name}.npy");
        let payload = npy_bytes(array);
        let offset = out.len() as u32;
        let sum = crc.sum(&payload);
        // Local file header.
        push_u32(&mut out, 0x0403_4b50);
        push_u16(&mut out, 20);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0x0021);
        push_u32(&mut out, sum);
        push_u32(&mut out, payload.len() as u32);
        push_u32(&mut out, payload.len() as u32);
        push_u16(&mut out, file.len() as u16);
        push_u16(&mut out, 0);
        out.extend_from_slice(file.as_bytes());
        out.extend_from_slice(&payload);
        // Central directory row.
        push_u32(&mut central, 0x0201_4b50);
        push_u16(&mut central, 20);
        push_u16(&mut central, 20);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0x0021);
        push_u32(&mut central, sum);
        push_u32(&mut central, payload.len() as u32);
        push_u32(&mut central, payload.len() as u32);
        push_u16(&mut central, file.len() as u16);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, offset);
        central.extend_from_slice(file.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    push_u32(&mut out, 0x0605_4b50);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, arrays.len() as u16);
    push_u16(&mut out, arrays.len() as u16);
    push_u32(&mut out, central.len() as u32);
    push_u32(&mut out, cd_offset);
    push_u16(&mut out, 0);
    out
}

/// One `.npy` payload for an array whose `data` already matches `shape`.
fn npy_bytes(array: &Npy) -> Vec<u8> {
    let shape = if array.shape.is_empty() {
        "()".to_string()
    } else {
        let parts: Vec<String> = array.shape.iter().map(usize::to_string).collect();
        format!("({},)", parts.join(", "))
    };
    let mut header = format!(
        "{{'descr': '{}', 'fortran_order': False, 'shape': {shape}, }}",
        array.descr
    );
    // NumPy pads the header so the 10-byte preamble plus header is a multiple
    // of 64 bytes, ending with a newline.
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut out = Vec::with_capacity(10 + header.len() + array.data.len());
    out.extend_from_slice(&[0x93, b'N', b'U', b'M', b'P', b'Y', 1, 0]);
    push_u16(&mut out, header.len() as u16);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(&array.data);
    out
}

/// A little-endian `f32` array from values and shape.
pub fn f32_array(values: &[f32], shape: Vec<usize>) -> Npy {
    let mut data = Vec::with_capacity(values.len() * 4);
    for value in values {
        data.extend_from_slice(&value.to_le_bytes());
    }
    Npy {
        descr: "<f4".to_string(),
        shape,
        data,
    }
}

/// A 0-d little-endian `f32` scalar.
pub fn f32_scalar(value: f32) -> Npy {
    f32_array(&[value], Vec::new())
}

/// A 0-d little-endian `i64` scalar.
pub fn i64_scalar(value: i64) -> Npy {
    Npy {
        descr: "<i8".to_string(),
        shape: Vec::new(),
        data: value.to_le_bytes().to_vec(),
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// CRC-32 (IEEE, reflected): the zip checksum, not a security hash.
struct Crc32;

impl Crc32 {
    fn new() -> Self {
        Self
    }

    fn sum(&mut self, bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_archive_reads_back_bit_exact() {
        let values: Vec<f32> = (0..12).map(|i| i as f32 * 0.25 - 1.0).collect();
        let array = f32_array(&values, vec![2, 3, 2]);
        let bytes = write_npz(&[("posed_joints", array), ("fps", f32_scalar(30.0))]);
        let npz = Npz::from_bytes(&bytes).expect("reads its own bytes");
        assert_eq!(npz.names(), vec!["fps", "posed_joints"]);
        let back = npz
            .get("posed_joints")
            .expect("array")
            .as_f32()
            .expect("f32");
        assert_eq!(back, values);
        assert_eq!(npz.get("posed_joints").expect("array").shape, vec![2, 3, 2]);
        let fps = npz.get("fps").expect("fps").as_f32().expect("f32");
        assert_eq!(fps, vec![30.0]);
    }

    #[test]
    fn a_foreign_blob_is_refused_by_name() {
        assert_eq!(Npz::from_bytes(b"not a zip").err(), Some(NpzError::NotZip));
        let array = f32_array(&[1.0, 2.0], vec![2]);
        let mut bytes = write_npz(&[("a", array)]);
        let len = bytes.len();
        bytes.truncate(len - 3);
        assert!(Npz::from_bytes(&bytes).is_err(), "a cut archive is refused");
        let npz = Npz::from_bytes(&write_npz(&[("a", f32_array(&[], vec![0]))])).expect("empty");
        assert_eq!(npz.get("b").err(), Some(NpzError::Missing("b".to_string())));
    }
}
