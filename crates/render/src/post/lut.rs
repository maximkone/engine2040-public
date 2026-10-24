//! lut: the `.cube` 3D LUT reader and its CPU sample path (gap E11).
//!
//! A `.cube` file is a text header (`TITLE`, `DOMAIN_MIN`, `DOMAIN_MAX`,
//! `LUT_3D_SIZE`) followed by `size^3` rows of three floats, blue fastest
//! (red changes slowest) and green between them. [`parse_cube`] reads that
//! text into a [`Lut3d`]; [`Lut3d::apply`] normalizes a color through the
//! domain, clamps at the edges and trilinearly interpolates the table.
//! Units: table values are the LUT's own units (a grade LUT is display
//! `0..=1`), the domain is input color in those same units. Determinism:
//! plain float arithmetic over a fixed table, no clock, no reads, no
//! per-sample allocation, so one file and one color give the same output
//! twice. Errors are typed [`LutError`], each naming the line and one fix.
//!
//! Accepted forms: `TITLE "text"`, one `LUT_3D_SIZE n` with `n` in
//! [`LUT_SIZE_MIN`]`..=`[`LUT_SIZE_MAX`], optional `DOMAIN_MIN`/`DOMAIN_MAX`
//! (default `0 0 0` and `1 1 1`, each written once), blank lines, and `#`
//! comments (whole line or trailing). Refused by design: `LUT_1D_SIZE`
//! (this file is 3D only) and `LUT_3D_INPUT_RANGE` (write the range as
//! `DOMAIN_MIN`/`DOMAIN_MAX` instead, so one code path owns the mapping).
//!
//! Plug-in point: the table samples after `apply_grade` and before the
//! display write, so a look sits on graded display values. [`ChainConfig`]
//! beside this module holds the loaded table plus the look file name, and
//! `chain::run_chain_look` samples it (the LUT-free `run_chain` keeps today's
//! bytes). The future `view.look` door verb names the `.cube` file to load
//! (today the view pack has `look.judge` and `look.propose` only); this
//! module stays the GPU-free half: parse and sample, no device, no IO.
//!
//! [`ChainConfig`]: super::ChainConfig
//!
//! GPU seam: [`Lut3d::rgba16f_table`] is the upload form (four f16 channels
//! per texel, blue fastest) and [`Lut3d::sample_texel`] is the CPU statement
//! of the fetch. The ready-to-paste WGSL, to land in the shader owner's file,
//! samples the same table with a linear-filtering 3D sampler after grading:
//!
//! ```wgsl
//! @group(2) @binding(0) var lut_tex: texture_3d<f32>;
//! @group(2) @binding(1) var lut_sampler: sampler;
//!
//! fn lut_fetch(display: vec3<f32>) -> vec3<f32> {
//!     // Domain 0..=1 onto texel centers, exactly `Lut3d::apply`: the grid is
//!     // `color * (size - 1)` and the texture unit is `(grid + 0.5) / size`.
//!     // A naive `clamp(display)` fetch is wrong by half a texel and by the
//!     // `size - 1` stretch; the device test in `lut_upload` catches that.
//!     let dims = vec3<f32>(textureDimensions(lut_tex));
//!     let grid = clamp(display, vec3<f32>(0.0), vec3<f32>(1.0)) * (dims - vec3<f32>(1.0));
//!     let uv = (grid + vec3<f32>(0.5)) / dims;
//!     // The upload is blue-fastest: texture x is blue and texture z is red.
//!     return textureSampleLevel(lut_tex, lut_sampler, vec3<f32>(uv.b, uv.g, uv.r), 0.0).rgb;
//! }
//! ```

use std::fmt;

use engine_core::Fnv64;

/// Smallest accepted grid edge (`LUT_3D_SIZE`), in texels per axis.
pub const LUT_SIZE_MIN: u16 = 2;
/// Largest accepted grid edge: `129^3` texels is about 25 MB of f32
/// triples, the biggest `.cube` the art presets ship.
pub const LUT_SIZE_MAX: u16 = 129;

/// Why a `.cube` text was refused. Every variant carries the 1-based line
/// the reader stopped on; [`fmt::Display`] adds the one fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LutError {
    /// A line the reader does not accept (`detail` says which shape).
    Syntax {
        /// 1-based line the reader stopped on.
        line: usize,
        /// What the line looked like, in a few words.
        detail: &'static str,
    },
    /// `LUT_1D_SIZE`: a 1D curve, not the 3D form this reader takes.
    OneD {
        /// 1-based line of the refused header.
        line: usize,
        /// Edge the file asked for (`0` when the count did not parse).
        size: u32,
    },
    /// The grid edge is outside [`LUT_SIZE_MIN`]`..=`[`LUT_SIZE_MAX`].
    Size {
        /// 1-based line of the size header.
        line: usize,
        /// Edge the file asked for.
        size: u32,
    },
    /// A second `LUT_3D_SIZE` line; one size owns the file.
    DuplicateSize {
        /// 1-based line of the second size header.
        line: usize,
    },
    /// The table row count disagrees with `size^3`.
    RowCount {
        /// Line of the row that broke the count (or the last one seen).
        line: usize,
        /// Rows the file holds.
        rows: usize,
        /// Rows `size^3` needs.
        expected: usize,
    },
    /// A table value parsed to NaN or an infinity.
    NotFinite {
        /// 1-based line of the offending row.
        line: usize,
    },
    /// `DOMAIN_MIN` is not below `DOMAIN_MAX` on every channel.
    Domain {
        /// 1-based line that completed the domain pair.
        line: usize,
    },
    /// `LUT_3D_INPUT_RANGE` was seen; this reader takes the domain headers.
    InputRange {
        /// 1-based line of the refused header.
        line: usize,
    },
}

impl fmt::Display for LutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, detail } => write!(
                f,
                "lut line {line}: {detail}; write a known header or three numbers, or start the line with #"
            ),
            Self::OneD { line, size } => write!(
                f,
                "lut line {line}: LUT_1D_SIZE {size} is a 1D curve; give LUT_3D_SIZE n and n*n*n rows"
            ),
            Self::Size { line, size } => write!(
                f,
                "lut line {line}: LUT_3D_SIZE {size} is outside {LUT_SIZE_MIN}..={LUT_SIZE_MAX}; pick an edge in that band"
            ),
            Self::DuplicateSize { line } => write!(
                f,
                "lut line {line}: a second LUT_3D_SIZE; keep one size line"
            ),
            Self::RowCount {
                line,
                rows,
                expected,
            } => write!(
                f,
                "lut line {line}: {rows} table rows for {expected} texels; write exactly {expected} rows, blue fastest"
            ),
            Self::NotFinite { line } => write!(
                f,
                "lut line {line}: a table value is not finite; write plain decimal numbers"
            ),
            Self::Domain { line } => write!(
                f,
                "lut line {line}: DOMAIN_MIN is not below DOMAIN_MAX; order the two headers"
            ),
            Self::InputRange { line } => write!(
                f,
                "lut line {line}: LUT_3D_INPUT_RANGE is not read; express the range with DOMAIN_MIN and DOMAIN_MAX"
            ),
        }
    }
}

impl std::error::Error for LutError {}

/// A parsed 3D LUT: grid edge, input domain and the texel table in `.cube`
/// order (red slowest, green, blue fastest). Built by [`parse_cube`] alone,
/// so `table.len() == size^3` always holds and a sample never goes out of
/// range.
#[derive(Clone, PartialEq, Debug)]
pub struct Lut3d {
    size: u16,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    table: Vec<[f32; 3]>,
    title: Option<String>,
}

impl Lut3d {
    /// Grid edge in texels per axis.
    pub fn size(&self) -> u16 {
        self.size
    }

    /// Lowest input color the table covers, per channel.
    pub fn domain_min(&self) -> [f32; 3] {
        self.domain_min
    }

    /// Highest input color the table covers, per channel.
    pub fn domain_max(&self) -> [f32; 3] {
        self.domain_max
    }

    /// `TITLE` text as written, quotes stripped; `None` when the file had none.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Texels in `.cube` order: red slowest, then green, then blue fastest.
    pub fn table(&self) -> &[[f32; 3]] {
        &self.table
    }

    /// Texel count, always `size^3`.
    pub fn texels(&self) -> usize {
        self.table.len()
    }

    /// Map one color through the table: normalize into the domain, clamp at
    /// the edges, trilinear interpolate. An identity table returns the input
    /// within `1e-4`; a non-finite channel lands on the low domain edge, so
    /// the returned triple is always finite. Equals [`Self::sample_texel`] at
    /// the same grid position (the GPU and CPU reference share one shape).
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut cell = [(0usize, 0.0f32); 3];
        for (channel, slot) in cell.iter_mut().enumerate() {
            *slot = self.axis(channel, rgb[channel]);
        }
        let (r0, fr) = cell[0];
        let (g0, fg) = cell[1];
        let (b0, fb) = cell[2];
        self.interpolate(r0, fr, g0, fg, b0, fb)
    }

    /// Trilinear reference at a grid position in texel units: `px` along red,
    /// `py` along green, `pz` along blue, each in `0..=size-1` and clamped at
    /// the edges. This is the CPU statement of what a linear-filtered 3D
    /// texture sample returns, so the GPU fetch can be compared against it.
    pub fn sample_texel(&self, px: f32, py: f32, pz: f32) -> [f32; 3] {
        let (r0, fr) = self.grid_cell(px);
        let (g0, fg) = self.grid_cell(py);
        let (b0, fb) = self.grid_cell(pz);
        self.interpolate(r0, fr, g0, fg, b0, fb)
    }

    /// [`Self::rgba16f_table`] in raw bytes: four little-endian f16 channels
    /// per texel (R, G, B from the table, A = 1.0), in `.cube` order, so the
    /// bytes upload straight into an `Rgba16Float` 3D texture of
    /// `size x size x size` with blue on the fastest axis.
    pub fn rgba16f_table(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.table.len().saturating_mul(8));
        for texel in &self.table {
            for channel in texel {
                bytes.extend_from_slice(&f16_bits(*channel).to_le_bytes());
            }
            // Alpha marks a filled texel; a linear fetch ignores it today.
            bytes.extend_from_slice(&f16_bits(1.0).to_le_bytes());
        }
        bytes
    }

    /// Apply the table to a caller slice in place: every pixel inside `px` is
    /// rewritten, nothing outside it is touched, and nothing is allocated.
    pub fn apply_in_place(&self, px: &mut [[f32; 3]]) {
        for pixel in px {
            *pixel = self.apply(*pixel);
        }
    }

    /// Digest over the grid edge, both domain corners and every texel bit
    /// through [`Fnv64`]: two parses of one file agree, a changed table does
    /// not. `TITLE` is free text and stays out of the digest.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u16(self.size);
        for value in self.domain_min.iter().chain(self.domain_max.iter()) {
            hasher.write_f32(*value);
        }
        hasher.write_len(self.table.len());
        for texel in &self.table {
            for value in texel {
                hasher.write_f32(*value);
            }
        }
        hasher.finish()
    }

    /// Last grid index on every axis (`size - 1`, at least 1).
    fn last(&self) -> usize {
        usize::from(self.size) - 1
    }

    /// Where one channel lands on the grid: the cell below it plus the
    /// in-cell share. A value inside the domain normalizes normally, every
    /// value outside clamps to the nearest edge, and a non-finite value
    /// lands on the low edge (NaN never enters the share math).
    fn axis(&self, channel: usize, value: f32) -> (usize, f32) {
        let low = self.domain_min[channel];
        let span = self.domain_max[channel] - low;
        let value = if value.is_finite() { value } else { low };
        let share = ((value - low) / span).clamp(0.0, 1.0);
        let scaled = share * index_f32(self.last());
        let last = self.last();
        let cell = cell_index(scaled, last).min(last - 1);
        (cell, (scaled - index_f32(cell)).clamp(0.0, 1.0))
    }

    /// One texel by grid index; the caller stays below `size` on every axis.
    fn texel(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        let n = usize::from(self.size);
        self.table[(r * n + g) * n + b]
    }

    /// The eight-tap trilinear blend around one cell, shared by [`Self::apply`]
    /// and [`Self::sample_texel`] so the CPU paths cannot drift apart.
    fn interpolate(&self, r0: usize, fr: f32, g0: usize, fg: f32, b0: usize, fb: f32) -> [f32; 3] {
        let last = self.last();
        let r1 = (r0 + 1).min(last);
        let g1 = (g0 + 1).min(last);
        let b1 = (b0 + 1).min(last);
        let low_b = blend(self.texel(r0, g0, b0), self.texel(r0, g0, b1), fb);
        let high_b = blend(self.texel(r1, g0, b0), self.texel(r1, g0, b1), fb);
        let low_r = blend(self.texel(r0, g1, b0), self.texel(r0, g1, b1), fb);
        let high_r = blend(self.texel(r1, g1, b0), self.texel(r1, g1, b1), fb);
        let low = blend(low_b, low_r, fg);
        let high = blend(high_b, high_r, fg);
        blend(low, high, fr)
    }

    /// Cell below a grid coordinate in texel units plus the in-cell share,
    /// with the edges clamped (a coordinate outside the grid reads the edge).
    fn grid_cell(&self, position: f32) -> (usize, f32) {
        let last = self.last();
        let clamped = if position.is_finite() {
            position.clamp(0.0, index_f32(last))
        } else {
            0.0
        };
        let cell = cell_index(clamped, last).min(last - 1);
        (cell, (clamped - index_f32(cell)).clamp(0.0, 1.0))
    }
}

/// Read a `.cube` text into a [`Lut3d`]. No file IO: the caller reads the
/// bytes and hands the text over.
///
/// # Errors
///
/// [`LutError`], one variant per refusal (1D file, size outside the band, a
/// second size line, row count, non-finite value, inverted domain, input
/// range header, or a line that is not a known header or a row), each
/// carrying the line and one fix.
pub fn parse_cube(text: &str) -> Result<Lut3d, LutError> {
    let mut reader = CubeReader::new();
    for (index, line) in text.lines().enumerate() {
        reader.line = index + 1;
        reader.feed(line)?;
    }
    reader.finish()
}

/// Component blend: `a` at `t = 0`, `b` at `t = 1`.
fn blend(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// A grid index as `f32`, without a float cast: every accepted edge fits
/// `u16`, so `From` covers the whole index range.
fn index_f32(index: usize) -> f32 {
    f32::from(u16::try_from(index).unwrap_or(u16::MAX))
}

/// f16 bits of one table value: normals round to nearest even, magnitudes
/// under the smallest normal half (`2^-14`) flush to signed zero, and
/// overflow or a non-finite value becomes signed infinity. The CPU oracle
/// stays f32; this is the upload form the 3D texture takes.
fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = half_u16((bits >> 16) & 0x8000);
    let abs = bits & 0x7fff_ffff;
    if !value.is_finite() || abs >= 0x477f_f000 {
        return sign | 0x7c00;
    }
    let exp = abs >> 23;
    if exp < 113 {
        return sign;
    }
    let keep = abs & 0x007f_ffff;
    let mut half_exp = exp - 112;
    let mut mantissa = keep >> 13;
    let dropped = keep & 0x1fff;
    if dropped > 0x1000 || (dropped == 0x1000 && mantissa & 1 == 1) {
        mantissa += 1;
        if mantissa == 0x0400 {
            mantissa = 0;
            half_exp += 1;
        }
    }
    if half_exp >= 0x1f {
        return sign | 0x7c00;
    }
    sign | half_u16(half_exp << 10) | half_u16(mantissa)
}

/// One narrow field of [`f16_bits`] as `u16` (the value always fits).
fn half_u16(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Grid cell holding `scaled` in `0..=last`: halving search over the index
/// range (log2 steps, no float-to-int cast on the sample path), the same
/// idiom the auto-exposure bin search uses.
fn cell_index(scaled: f32, last: usize) -> usize {
    let mut low = 0usize;
    let mut high = last;
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if scaled >= index_f32(mid) {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

/// One finite table value, or the typed error the line deserves.
fn finite(token: &str, line: usize) -> Result<f32, LutError> {
    match token.parse::<f32>() {
        Ok(value) if value.is_finite() => Ok(value),
        Ok(_) => Err(LutError::NotFinite { line }),
        Err(_) => Err(LutError::Syntax {
            line,
            detail: "a table row holds three numbers",
        }),
    }
}

/// Parse state: one line at a time, headers first, then `size^3` rows.
struct CubeReader {
    /// 1-based line being read.
    line: usize,
    size: Option<u16>,
    size_line: usize,
    title: Option<String>,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    domain_line: usize,
    has_min: bool,
    has_max: bool,
    rows: Vec<[f32; 3]>,
    last_row_line: usize,
}

impl CubeReader {
    /// Empty reader: default domain `0..=1`, no size yet.
    fn new() -> Self {
        Self {
            line: 0,
            size: None,
            size_line: 0,
            title: None,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            domain_line: 0,
            has_min: false,
            has_max: false,
            rows: Vec::new(),
            last_row_line: 0,
        }
    }

    /// Read one line: strip the comment, then dispatch on its first word.
    fn feed(&mut self, raw: &str) -> Result<(), LutError> {
        let body = raw.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            return Ok(());
        }
        let mut words = body.split_whitespace();
        let head = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        match head {
            "TITLE" => self.title(&rest),
            "DOMAIN_MIN" => self.domain(&rest, true),
            "DOMAIN_MAX" => self.domain(&rest, false),
            "LUT_3D_SIZE" => self.size(&rest),
            "LUT_1D_SIZE" => Err(LutError::OneD {
                line: self.line,
                size: rest.first().and_then(|t| t.parse().ok()).unwrap_or(0),
            }),
            "LUT_3D_INPUT_RANGE" => Err(LutError::InputRange { line: self.line }),
            _ => self.row(body),
        }
    }

    /// `TITLE "text"`: free text, optionally quoted, kept once.
    fn title(&mut self, rest: &[&str]) -> Result<(), LutError> {
        if self.title.is_some() || !self.rows.is_empty() {
            return Err(LutError::Syntax {
                line: self.line,
                detail: "TITLE already read or written after the table",
            });
        }
        let joined = rest.join(" ");
        let trimmed = joined.trim();
        let text = trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(trimmed)
            .to_string();
        self.title = Some(text);
        Ok(())
    }

    /// `DOMAIN_MIN r g b` or `DOMAIN_MAX r g b`: one of each, before rows.
    fn domain(&mut self, rest: &[&str], is_min: bool) -> Result<(), LutError> {
        if !self.rows.is_empty() {
            return Err(LutError::Syntax {
                line: self.line,
                detail: "a domain header after the table",
            });
        }
        let already = if is_min { self.has_min } else { self.has_max };
        if already || rest.len() != 3 {
            return Err(LutError::Syntax {
                line: self.line,
                detail: "each domain header is written once, with three numbers",
            });
        }
        let mut corner = [0.0f32; 3];
        for (slot, token) in rest.iter().enumerate() {
            corner[slot] = finite(token, self.line)?;
        }
        if is_min {
            self.domain_min = corner;
            self.has_min = true;
        } else {
            self.domain_max = corner;
            self.has_max = true;
        }
        if self.has_min && self.has_max {
            self.domain_line = self.line;
        }
        Ok(())
    }

    /// `LUT_3D_SIZE n`: one line, inside the accepted band, before rows.
    fn size(&mut self, rest: &[&str]) -> Result<(), LutError> {
        if self.size.is_some() {
            return Err(LutError::DuplicateSize { line: self.line });
        }
        if !self.rows.is_empty() {
            return Err(LutError::Syntax {
                line: self.line,
                detail: "the size header after the table",
            });
        }
        let edge = match rest {
            [token] => token.parse::<u32>().map_err(|_| LutError::Syntax {
                line: self.line,
                detail: "LUT_3D_SIZE needs one whole number",
            })?,
            _ => {
                return Err(LutError::Syntax {
                    line: self.line,
                    detail: "LUT_3D_SIZE needs one whole number",
                })
            }
        };
        if edge < u32::from(LUT_SIZE_MIN) || edge > u32::from(LUT_SIZE_MAX) {
            return Err(LutError::Size {
                line: self.line,
                size: edge,
            });
        }
        self.size = Some(u16::try_from(edge).unwrap_or(LUT_SIZE_MAX));
        self.size_line = self.line;
        Ok(())
    }

    /// One table row: three finite numbers, after the size, at most `size^3`.
    fn row(&mut self, body: &str) -> Result<(), LutError> {
        let tokens: Vec<&str> = body.split_whitespace().collect();
        if tokens.len() != 3 {
            return Err(LutError::Syntax {
                line: self.line,
                detail: "a line is a header or three numbers",
            });
        }
        let size = match self.size {
            Some(size) => size,
            None => {
                return Err(LutError::Syntax {
                    line: self.line,
                    detail: "a table row before LUT_3D_SIZE",
                })
            }
        };
        let expected = usize::from(size) * usize::from(size) * usize::from(size);
        if self.rows.len() >= expected {
            return Err(LutError::RowCount {
                line: self.line,
                rows: expected + 1,
                expected,
            });
        }
        let mut texel = [0.0f32; 3];
        for (slot, token) in tokens.iter().enumerate() {
            texel[slot] = finite(token, self.line)?;
        }
        self.rows.push(texel);
        self.last_row_line = self.line;
        Ok(())
    }

    /// Complete file: size present, domain ordered, row count exact. Header
    /// errors come first: an inverted domain is a broken header, not a short
    /// table.
    fn finish(self) -> Result<Lut3d, LutError> {
        let size = match self.size {
            Some(size) => size,
            None => {
                return Err(LutError::Syntax {
                    line: self.line.max(1),
                    detail: "no LUT_3D_SIZE line",
                })
            }
        };
        for channel in 0..3 {
            if self.domain_min[channel] >= self.domain_max[channel] {
                return Err(LutError::Domain {
                    line: self.domain_line.max(self.size_line).max(1),
                });
            }
        }
        let expected = usize::from(size) * usize::from(size) * usize::from(size);
        if self.rows.len() != expected {
            return Err(LutError::RowCount {
                line: self.last_row_line.max(self.size_line).max(1),
                rows: self.rows.len(),
                expected,
            });
        }
        Ok(Lut3d {
            size,
            domain_min: self.domain_min,
            domain_max: self.domain_max,
            table: self.rows,
            title: self.title,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2x2 identity cube with a title, a full domain pair and eight rows
    /// in blue-fastest order.
    const IDENTITY_2: &str = "\
# 2x2x2 identity
TITLE \"identity\"
LUT_3D_SIZE 2
DOMAIN_MIN 0 0 0
DOMAIN_MAX 1 1 1
0 0 0
0 0 1
0 1 0
0 1 1
1 0 0
1 0 1
1 1 0
1 1 1
";

    /// Build a `.cube` text of `size^3` rows from a per-vertex map. Rows go
    /// in `.cube` order: red slowest, blue fastest.
    fn cube<F: Fn(f32, f32, f32) -> [f32; 3]>(size: usize, map: F) -> String {
        let last = index_f32(size - 1);
        let mut text = format!("LUT_3D_SIZE {size}\n");
        for r in 0..size {
            for g in 0..size {
                for b in 0..size {
                    let out = map(
                        index_f32(r) / last,
                        index_f32(g) / last,
                        index_f32(b) / last,
                    );
                    text.push_str(&format!("{} {} {}\n", out[0], out[1], out[2]));
                }
            }
        }
        text
    }

    /// A warm shift: red up by a tenth, green straight through, blue down a
    /// tenth, so the middle of the grid moves off the grey axis.
    fn warm(r: f32, g: f32, b: f32) -> [f32; 3] {
        [r * 0.9 + 0.1, g, b * 0.9]
    }

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol && (a[2] - b[2]).abs() <= tol
    }

    /// The 2x2x2 identity parses with its header and returns every input
    /// unchanged within the posted tolerance, corners and interior alike.
    #[test]
    fn lut_identity_2x2x2_is_identity() {
        let lut = parse_cube(IDENTITY_2).expect("identity parses");
        assert_eq!(lut.size(), 2, "the edge reads back");
        assert_eq!(lut.texels(), 8, "eight texels for a 2x2x2 table");
        assert_eq!(lut.title(), Some("identity"), "the quoted title reads back");
        assert_eq!(lut.domain_min(), [0.0, 0.0, 0.0], "default min");
        assert_eq!(lut.domain_max(), [1.0, 1.0, 1.0], "default max");
        for probe in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.5, 0.5, 0.5],
            [0.25, 0.75, 0.125],
            [0.9, 0.1, 0.6],
        ] {
            let out = lut.apply(probe);
            assert!(
                close(out, probe, 1e-4),
                "identity moves {probe:?} to {out:?}"
            );
        }
    }

    /// The warm 3x3x3 cube moves mid grey up in red and down in blue.
    #[test]
    fn lut_warm_shift_moves_red_up_and_blue_down() {
        let lut = parse_cube(&cube(3, warm)).expect("warm cube parses");
        assert_eq!(lut.size(), 3, "the edge reads back");
        assert_eq!(lut.texels(), 27, "27 texels for a 3x3x3 table");
        let out = lut.apply([0.5, 0.5, 0.5]);
        assert!(
            close(out, [0.55, 0.5, 0.45], 1e-4),
            "mid grey lands at {out:?}, not the warm shift"
        );
        assert!(out[0] > 0.5, "red must rise, landed at {}", out[0]);
        assert!(out[2] < 0.5, "blue must fall, landed at {}", out[2]);
        let dark = lut.apply([0.1, 0.1, 0.1]);
        assert!(
            dark[0] > 0.1 && dark[2] < 0.1,
            "the shift holds off centre too: {dark:?}"
        );
    }

    /// A 16x16x16 cube parsed twice gives the same digest, and a different
    /// table gives a different one.
    #[test]
    fn lut_16_cube_parses_twice_with_equal_digests() {
        let text = cube(16, |r, g, b| [r * 0.7 + g * 0.2, g, b * b]);
        let first = parse_cube(&text).expect("16 cube parses");
        let second = parse_cube(&text).expect("16 cube parses again");
        assert_eq!(first.size(), 16, "the edge reads back");
        assert_eq!(first.texels(), 4096, "a 16 edge holds 4096 texels");
        assert_eq!(
            first.digest(),
            second.digest(),
            "two parses of one file disagree on the digest"
        );
        let other = parse_cube(&text.replace("LUT_3D_SIZE 16", "DOMAIN_MIN 0 0 0\nLUT_3D_SIZE 16"))
            .expect("domain variant parses");
        assert_eq!(
            first.digest(),
            other.digest(),
            "an equal table digests alike"
        );
        let changed = parse_cube(&cube(16, |r, g, b| [r, g, b * b])).expect("changed cube parses");
        assert_ne!(
            first.digest(),
            changed.digest(),
            "a changed table must change the digest"
        );
    }

    /// A short table names the line it ran out on and the row count it needs.
    #[test]
    fn lut_short_table_reports_the_row_line() {
        let text = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n";
        match parse_cube(text) {
            Err(LutError::RowCount {
                line,
                rows,
                expected,
            }) => {
                assert_eq!((line, rows, expected), (8, 7, 8), "the last row is line 8");
            }
            other => panic!("short table gives {other:?}, not a row count"),
        }
    }

    /// A long table stops at the first extra row instead of growing.
    #[test]
    fn lut_long_table_stops_at_the_extra_row() {
        let text = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n1 1 1\n";
        match parse_cube(text) {
            Err(LutError::RowCount {
                line,
                rows,
                expected,
            }) => {
                assert_eq!(
                    (line, rows, expected),
                    (10, 9, 8),
                    "the ninth row is line 10"
                );
            }
            other => panic!("long table gives {other:?}, not a row count"),
        }
    }

    /// Edges below and above the accepted band both fail with the size error.
    #[test]
    fn lut_size_outside_the_band_names_the_line() {
        for edge in [1u32, 200] {
            let text = format!("LUT_3D_SIZE {edge}\n");
            match parse_cube(&text) {
                Err(LutError::Size { line, size }) => {
                    assert_eq!((line, size), (1, edge), "the size line is line 1");
                }
                other => panic!("size {edge} gives {other:?}, not a size error"),
            }
        }
    }

    /// A NaN value fails with the row's line, and the message carries it.
    #[test]
    fn lut_nan_names_the_row_line() {
        let text = "LUT_3D_SIZE 2\n0 0 NaN\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        match parse_cube(text) {
            Err(error @ LutError::NotFinite { line }) => {
                assert_eq!(line, 2, "the NaN sits on line 2");
                let shown = error.to_string();
                assert!(
                    shown.contains("line 2") && shown.contains("finite"),
                    "the message names the line and the fix: {shown}"
                );
            }
            other => panic!("NaN gives {other:?}, not a finite check"),
        }
    }

    /// A 1D file is refused and the message names the legal 3D form.
    #[test]
    fn lut_one_dimensional_file_names_the_3d_form() {
        match parse_cube("LUT_1D_SIZE 16\n") {
            Err(error @ LutError::OneD { line, size }) => {
                assert_eq!((line, size), (1, 16), "the 1D header is line 1");
                let shown = error.to_string();
                assert!(
                    shown.contains("LUT_3D_SIZE"),
                    "the message names the legal form: {shown}"
                );
            }
            other => panic!("1D file gives {other:?}, not a 1D refusal"),
        }
    }

    /// The input-range header is refused by design; the domain headers own
    /// the mapping.
    #[test]
    fn lut_input_range_header_is_refused() {
        match parse_cube("LUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 0 1\n") {
            Err(error @ LutError::InputRange { line }) => {
                assert_eq!(line, 2, "the range header is line 2");
                assert!(
                    error.to_string().contains("DOMAIN_MIN"),
                    "the message names the supported headers: {error}"
                );
            }
            other => panic!("input range gives {other:?}, not a range refusal"),
        }
    }

    /// Two size lines and a row before the size are both syntax refusals
    /// with their line numbers.
    #[test]
    fn lut_header_order_is_checked() {
        match parse_cube("LUT_3D_SIZE 2\nLUT_3D_SIZE 2\n") {
            Err(LutError::DuplicateSize { line }) => assert_eq!(line, 2, "the second line"),
            other => panic!("two sizes give {other:?}, not a duplicate"),
        }
        match parse_cube("0 0 0\nLUT_3D_SIZE 2\n") {
            Err(LutError::Syntax { line, detail }) => {
                assert_eq!(line, 1, "the stray row is line 1");
                assert!(
                    detail.contains("before"),
                    "the fix names the order: {detail}"
                );
            }
            other => panic!("a row before the size gives {other:?}, not syntax"),
        }
        match parse_cube("BOGUS\n") {
            Err(LutError::Syntax { line, .. }) => assert_eq!(line, 1, "the stray line"),
            other => panic!("an unknown header gives {other:?}, not syntax"),
        }
    }

    /// An inverted domain is refused at the line that completed the pair.
    #[test]
    fn lut_inverted_domain_is_refused() {
        let text = "LUT_3D_SIZE 2\nDOMAIN_MIN 0.5 0 0\nDOMAIN_MAX 0 1 1\n";
        match parse_cube(text) {
            Err(LutError::Domain { line }) => assert_eq!(line, 3, "the max header is line 3"),
            other => panic!("an inverted domain gives {other:?}, not a domain error"),
        }
    }

    /// Input outside the domain clamps to the edge texels on both sides.
    #[test]
    fn lut_out_of_domain_input_clamps() {
        let text = cube(2, |r, g, b| [r * 2.0 - 1.0, g * 2.0 - 1.0, b * 2.0 - 1.0]).replace(
            "LUT_3D_SIZE",
            "DOMAIN_MIN -1 -1 -1\nDOMAIN_MAX 1 1 1\nLUT_3D_SIZE",
        );
        let lut = parse_cube(&text).expect("signed cube parses");
        assert_eq!(
            lut.domain_min(),
            [-1.0, -1.0, -1.0],
            "the min corner reads back"
        );
        assert_eq!(
            lut.domain_max(),
            [1.0, 1.0, 1.0],
            "the max corner reads back"
        );
        assert!(
            close(lut.apply([-5.0, -5.0, -5.0]), [-1.0, -1.0, -1.0], 1e-5),
            "far below the domain clamps to the low corner"
        );
        assert!(
            close(lut.apply([5.0, 0.0, -5.0]), [1.0, 0.0, -1.0], 1e-5),
            "far above clamps while in-domain channels stay put"
        );
        assert!(
            close(
                lut.apply([f32::NEG_INFINITY, 0.5, f32::NAN]),
                [-1.0, 0.5, -1.0],
                1e-5
            ),
            "a non-finite channel clamps to an edge, never NaN"
        );
    }

    /// The in-place path matches the one-pixel path on every element of the
    /// caller slice and leaves the length alone.
    #[test]
    fn lut_apply_in_place_matches_apply() {
        let lut = parse_cube(&cube(3, warm)).expect("warm cube parses");
        let input = [[0.0, 0.0, 0.0], [0.5, 0.25, 0.75], [1.0, 1.0, 1.0]];
        let mut pixels = input;
        lut.apply_in_place(&mut pixels);
        assert_eq!(pixels.len(), input.len(), "the slice length stays");
        for (out, probe) in pixels.iter().zip(input.iter()) {
            assert!(
                close(*out, lut.apply(*probe), 0.0),
                "in place {out:?} differs from apply {:?}",
                lut.apply(*probe)
            );
        }
        let mut empty: [[f32; 3]; 0] = [];
        lut.apply_in_place(&mut empty);
        assert!(empty.is_empty(), "an empty slice stays empty");
    }

    /// One f16 bit pattern back to f32, for reading the upload bytes.
    fn half_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = (bits >> 10) & 0x1f;
        let mantissa = f32::from(bits & 0x03ff);
        if exp == 0 {
            return sign * mantissa * 2f32.powi(-24);
        }
        if exp == 0x1f {
            return sign * f32::INFINITY;
        }
        sign * (1.0 + mantissa / 1024.0) * 2f32.powi(i32::from(exp) - 15)
    }

    /// The upload form is eight bytes per texel (four f16 channels), in
    /// blue-fastest order with alpha at 1.0, and the RGB halves read back as
    /// the table within f16 precision.
    #[test]
    fn lut_rgba16f_table_is_eight_bytes_per_texel() {
        let lut = parse_cube(&cube(3, warm)).expect("warm cube parses");
        let bytes = lut.rgba16f_table();
        assert_eq!(bytes.len(), 27 * 8, "four f16 channels for 27 texels");
        let half = |at: usize| -> f32 {
            let low = bytes[at];
            let high = bytes[at + 1];
            half_to_f32(u16::from(high) << 8 | u16::from(low))
        };
        for (texel, table) in lut.table().iter().enumerate() {
            let base = texel * 8;
            for (channel, want) in table.iter().take(3).enumerate() {
                let read = half(base + channel * 2);
                assert!(
                    (read - want).abs() <= 2e-3,
                    "texel {texel} channel {channel}: f16 {read} vs table {want}"
                );
            }
            assert_eq!(half(base + 6), 1.0, "alpha marks a filled texel");
        }
        let identity = parse_cube(IDENTITY_2).expect("identity parses");
        assert_eq!(
            identity.rgba16f_table().len(),
            8 * 8,
            "a 2x2x2 table uploads 8 texels of 8 bytes"
        );
    }

    /// The trilinear reference agrees with `apply` at the grid corners and at
    /// interior points: one shape, two entry points.
    #[test]
    fn lut_sample_texel_matches_apply() {
        let lut = parse_cube(&cube(3, warm)).expect("warm cube parses");
        let probes = [
            ([0.0, 0.0, 0.0], (0.0, 0.0, 0.0)),
            ([1.0, 1.0, 1.0], (2.0, 2.0, 2.0)),
            ([1.0, 0.0, 0.5], (2.0, 0.0, 1.0)),
            ([0.3, 0.6, 0.45], (0.6, 1.2, 0.9)),
            ([0.8, 0.2, 0.9], (1.6, 0.4, 1.8)),
            ([0.05, 0.95, 0.5], (0.1, 1.9, 1.0)),
        ];
        for (rgb, grid) in probes {
            let by_color = lut.apply(rgb);
            let by_texel = lut.sample_texel(grid.0, grid.1, grid.2);
            assert!(
                close(by_color, by_texel, 1e-3),
                "apply {rgb:?} = {by_color:?} but sample_texel {grid:?} = {by_texel:?}"
            );
        }
        // The edges clamp on both entries alike.
        assert!(close(
            lut.sample_texel(-5.0, 5.0, 9.0),
            lut.apply([0.0, 1.0, 1.0]),
            1e-3
        ));
    }
}
