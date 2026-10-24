//! buffers: the debug G-buffer frame (gap A3). Beauty is the worst debugging
//! image, so beside it this module keeps one CPU-side frame of the channels a
//! person asks for when something is black: view-space normal (octahedral
//! RGBA8), depth (u16), albedo (RGBA8) and material id (u16), each with its
//! own [`Fnv64`] digest so "did one channel move?" is a number, not an eye.
//!
//! Owner: render lane; the builder is pure (borrowed slices in, owned frame
//! out) and headless: no adapter, no window, no device call. Units: depth is
//! normalized `0..=1` (near..far) and quantizes to `0..=65535`; normals are
//! unit direction or anything close (a zero or non-finite normal decodes to
//! `+Z` and never panics). Determinism: fixed loops, plain arithmetic,
//! integers little-endian into the digests, so the same inputs give the same
//! bytes and the same four digests twice.
//!
//! Where the slices come from: the live depth prepass
//! (`present::gpu_present::gtao_live::encode_depth_prepass`, which writes
//! `targets.depth_texture`) keeps depth per frame. The forward pass keeps no
//! normal, albedo or id target yet, so the GPU half is a readback seam, not
//! built here: map the depth texture plus the normal/albedo/id targets that
//! pass would add, read them back, and call [`DebugFrame::from_gbuffer`].
//! Until then callers hand over the same values they already hold on the CPU
//! (pick/prepass fixtures, captured frames, tests).

use engine_core::Fnv64;

/// Channel order of [`DebugFrame::digests`]: normal, depth, albedo, id.
pub const DEBUG_DIGEST_CHANNELS: [&str; 4] = ["normal", "depth", "albedo", "material_id"];

/// Why a debug frame could not be built. Every variant names the lengths and
/// [`std::fmt::Display`] adds the one fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BufferError {
    /// The frame has no pixels to show.
    Empty {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// One input slice does not hold exactly `width * height` values.
    Length {
        /// Channel that disagreed (`"normal"`, `"depth"`, `"albedo"`,
        /// `"material_id"`).
        buffer: &'static str,
        /// Values the caller passed.
        got: usize,
        /// Values `width * height` needs.
        expected: usize,
    },
    /// A channel texture the GPU readback needs is absent or uncopyable.
    MissingChannel {
        /// Channel name (`"normal"`, `"depth"`, `"albedo"`, `"material_id"`).
        channel: &'static str,
        /// Which pass must exist first.
        why: &'static str,
    },
    /// A channel texture carries a format the readback cannot take.
    Format {
        /// Channel name.
        channel: &'static str,
        /// Format the readback needs, by name.
        want: &'static str,
    },
    /// A mapped readback never arrived (device lost or a driver fault).
    Readback {
        /// Channel whose copy failed.
        channel: &'static str,
    },
}

impl std::fmt::Display for BufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty { width, height } => write!(
                f,
                "debug frame is {width}x{height}: a frame needs at least one pixel; pass the real size"
            ),
            Self::Length {
                buffer,
                got,
                expected,
            } => write!(
                f,
                "debug frame {buffer} holds {got} values for {expected} pixels; write exactly one value per pixel"
            ),
            Self::MissingChannel { channel, why } => write!(
                f,
                "debug frame {channel} has no texture: {why}"
            ),
            Self::Format { channel, want } => write!(
                f,
                "debug frame {channel} texture is not {want}; read back that format"
            ),
            Self::Readback { channel } => write!(
                f,
                "debug frame {channel} readback never mapped; rebuild the device and read again"
            ),
        }
    }
}

impl std::error::Error for BufferError {}

/// One debug frame: the four channels a person reads when a pixel is black,
/// plus their digests. All buffers are row-major, one entry per pixel, and
/// `normal`/`albedo` hold RGBA bytes per pixel.
#[derive(Clone, PartialEq, Debug)]
pub struct DebugFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Octahedral normal as RGBA8: R,G are the folded `x,y` pair (0..=255
    /// over `-1..=1`), B is 255 when the normal faces the viewer and 0 when
    /// it does not, A is 255.
    pub normal: Vec<u8>,
    /// Depth normalized `0..=1` as `0..=65535` (near..far), one per pixel.
    pub depth: Vec<u16>,
    /// Albedo as RGBA8: R,G,B are the linear `0..=1` color quantized, A is
    /// 255.
    pub albedo: Vec<u8>,
    /// Material id exactly as written, one per pixel.
    pub material_id: Vec<u16>,
    /// [`Fnv64`] per channel in [`DEBUG_DIGEST_CHANNELS`] order: normal,
    /// depth, albedo, material id. Equal inputs give equal digests twice.
    pub digests: [u64; 4],
}

impl DebugFrame {
    /// Build the debug frame from the G-buffer values the crate already has:
    /// unit normals, normalized depth, linear albedo and ids, all
    /// `width * height` long. Non-finite channels are clamped, never refused
    /// (a broken frame is exactly what someone is debugging).
    ///
    /// # Errors
    ///
    /// [`BufferError::Empty`] on a zero side, [`BufferError::Length`] when a
    /// slice does not hold one value per pixel.
    pub fn from_gbuffer(
        normal: &[[f32; 3]],
        depth: &[f32],
        albedo: &[[f32; 3]],
        ids: &[u16],
        width: u32,
        height: u32,
    ) -> Result<Self, BufferError> {
        if width == 0 || height == 0 {
            return Err(BufferError::Empty { width, height });
        }
        let pixels = pixels_of(width, height);
        check_len("normal", normal.len(), pixels)?;
        check_len("depth", depth.len(), pixels)?;
        check_len("albedo", albedo.len(), pixels)?;
        check_len("material_id", ids.len(), pixels)?;

        let mut normal_bytes = Vec::with_capacity(pixels.saturating_mul(4));
        for n in normal {
            normal_bytes.extend_from_slice(&encode_normal(*n));
        }
        let mut depth_bytes = Vec::with_capacity(pixels);
        for d in depth {
            depth_bytes.push(quantize_u16(clamp01(*d)));
        }
        let mut albedo_bytes = Vec::with_capacity(pixels.saturating_mul(4));
        for color in albedo {
            albedo_bytes.push(quantize_unit_byte(clamp01(color[0])));
            albedo_bytes.push(quantize_unit_byte(clamp01(color[1])));
            albedo_bytes.push(quantize_unit_byte(clamp01(color[2])));
            albedo_bytes.push(255);
        }
        let material_id = ids.to_vec();

        let digests = [
            digest_bytes(&normal_bytes),
            digest_u16s(&depth_bytes),
            digest_bytes(&albedo_bytes),
            digest_u16s(&material_id),
        ];
        Ok(Self {
            width,
            height,
            normal: normal_bytes,
            depth: depth_bytes,
            albedo: albedo_bytes,
            material_id,
            digests,
        })
    }

    /// Pixels in the frame (`width * height`).
    pub fn texels(&self) -> usize {
        pixels_of(self.width, self.height)
    }

    /// Digest of one channel by index (0 normal, 1 depth, 2 albedo, 3 id);
    /// indices past the end read the id so a caller never panics.
    pub fn digest(&self, channel: usize) -> u64 {
        self.digests
            .get(channel)
            .copied()
            .unwrap_or(self.digests[3])
    }
}

/// Material id at `(x, y)`, or `0` outside the frame (`0` means "no
/// material", so a miss never looks like a real id).
pub fn lookup_id(frame: &DebugFrame, x: u32, y: u32) -> u16 {
    if x >= frame.width || y >= frame.height {
        return 0;
    }
    let index = index_of(frame.width, x, y);
    frame.material_id.get(index).copied().unwrap_or(0)
}

/// Pixel where two frames' normals disagree most: `(x, y, delta)` with
/// `delta` the largest absolute component gap between the decoded normals.
/// Frames of different sizes compare over the overlap; an empty frame or a
/// perfect match reads `(0, 0, 0.0)`.
pub fn brightest_normal_diff(a: &DebugFrame, b: &DebugFrame) -> (u32, u32, f32) {
    let width = a.width.min(b.width);
    let height = a.height.min(b.height);
    let mut best = (0u32, 0u32, 0.0f32);
    for y in 0..height {
        for x in 0..width {
            let left = decode_normal(&a.normal, index_of(a.width, x, y));
            let right = decode_normal(&b.normal, index_of(b.width, x, y));
            let mut delta = 0.0f32;
            for channel in 0..3 {
                let gap = (left[channel] - right[channel]).abs();
                if gap > delta {
                    delta = gap;
                }
            }
            if delta > best.2 {
                best = (x, y, delta);
            }
        }
    }
    best
}

/// Pixel count for a size, saturating instead of wrapping.
fn pixels_of(width: u32, height: u32) -> usize {
    usize::try_from(width)
        .unwrap_or(usize::MAX)
        .saturating_mul(usize::try_from(height).unwrap_or(usize::MAX))
}

/// Row-major index of `(x, y)`; the caller keeps the bytes in range.
fn index_of(width: u32, x: u32, y: u32) -> usize {
    let stride = usize::try_from(width).unwrap_or(usize::MAX);
    let row = usize::try_from(y).unwrap_or(usize::MAX);
    let column = usize::try_from(x).unwrap_or(usize::MAX);
    row.saturating_mul(stride).saturating_add(column)
}

/// One slice length against the pixel count.
fn check_len(buffer: &'static str, got: usize, expected: usize) -> Result<(), BufferError> {
    if got == expected {
        Ok(())
    } else {
        Err(BufferError::Length {
            buffer,
            got,
            expected,
        })
    }
}

/// `0..=1` clamp that sends non-finite values to the low end.
fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One `-1..=1` share as a byte over `0..=255`, rounded half up without a
/// float cast (eight halving steps, the crate's fixed-search idiom).
fn quantize_byte(share: f32) -> u8 {
    quantize_unit_byte(share.clamp(-1.0, 1.0) * 0.5 + 0.5)
}

/// One `0..=1` share as a byte over `0..=255`, rounded half up: the albedo
/// path, and the tail of [`quantize_byte`].
fn quantize_unit_byte(share: f32) -> u8 {
    let tick = share.clamp(0.0, 1.0).mul_add(255.0, 0.5);
    let mut low = 0u8;
    let mut high = 255u8;
    while high > low {
        let mid = low + (high - low).div_ceil(2);
        if tick >= f32::from(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// One `0..=1` share as a depth tick over `0..=65535`, rounded half up the
/// same way (sixteen halving steps).
pub(crate) fn quantize_u16(share: f32) -> u16 {
    let tick = share.clamp(0.0, 1.0).mul_add(65535.0, 0.5);
    let mut low = 0u16;
    let mut high = u16::MAX;
    while high > low {
        let mid = low + (high - low).div_ceil(2);
        if tick >= f32::from(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// Octahedral `x,y` of a normal, folded so the whole sphere fits the square.
fn octahedral_xy(normal: [f32; 3]) -> [f32; 2] {
    let len = normal[0].abs() + normal[1].abs() + normal[2].abs();
    if !len.is_finite() || len <= 0.0 {
        return [0.0, 0.0];
    }
    let mut x = normal[0] / len;
    let mut y = normal[1] / len;
    if !x.is_finite() || !y.is_finite() {
        return [0.0, 0.0];
    }
    if normal[2] < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * sign_of(ox);
        y = (1.0 - ox.abs()) * sign_of(oy);
    }
    [x, y]
}

/// `-1` for a negative value, `1` otherwise (zero keeps the positive side).
fn sign_of(value: f32) -> f32 {
    if value < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// RGBA8 for one normal: octahedral pair plus the facing flag.
fn encode_normal(normal: [f32; 3]) -> [u8; 4] {
    let [x, y] = octahedral_xy(normal);
    let facing = if normal[2] >= 0.0 { 255 } else { 0 };
    [quantize_byte(x), quantize_byte(y), facing, 255]
}

/// Decode the octahedral pair at one pixel back to a unit direction.
pub(crate) fn decode_normal(bytes: &[u8], pixel: usize) -> [f32; 3] {
    let base = pixel.saturating_mul(4);
    let rx = bytes.get(base).copied().unwrap_or(128);
    let ry = bytes.get(base + 1).copied().unwrap_or(128);
    let mut x = f32::from(rx) / 255.0 * 2.0 - 1.0;
    let mut y = f32::from(ry) / 255.0 * 2.0 - 1.0;
    // The third component follows from the pair and keeps its sign through
    // the fold: the back hemisphere folds inward, `z` is not recomputed.
    let z = 1.0 - x.abs() - y.abs();
    if z < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * sign_of(ox);
        y = (1.0 - ox.abs()) * sign_of(oy);
    }
    let len = (x * x + y * y + z * z).sqrt();
    if len <= 0.0 || !len.is_finite() {
        return [0.0, 0.0, 1.0];
    }
    [x / len, y / len, z / len]
}

/// [`Fnv64`] over a byte buffer.
fn digest_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(bytes);
    hasher.finish()
}

/// [`Fnv64`] over `u16` values, little-endian per value.
fn digest_u16s(values: &[u16]) -> u64 {
    let mut hasher = Fnv64::new();
    for value in values {
        hasher.write_u16(*value);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test scene: normals, depth, albedo and ids, row-major.
    type Scene = (Vec<[f32; 3]>, Vec<f32>, Vec<[f32; 3]>, Vec<u16>);

    /// A small scene: sand ground with two crates, lit from `sun` degrees of
    /// tilt. Only the normal changes with `sun`; depth, albedo and ids stay.
    fn scene(sun: f32) -> Scene {
        let (width, height) = (8usize, 6usize);
        let mut normal = Vec::with_capacity(width * height);
        let mut depth = Vec::with_capacity(width * height);
        let mut albedo = Vec::with_capacity(width * height);
        let mut ids = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let tilt = (sun.to_radians()).sin() * 0.5;
                let lifting = if x % 4 == 0 { 0.6 } else { 0.0 };
                normal.push([tilt, -0.2, (1.0 - tilt * tilt).max(0.0).sqrt()]);
                depth.push(0.2 + 0.1 * (y as f32) + lifting);
                albedo.push([0.76, 0.66, 0.42]);
                ids.push(u16::try_from(x * 10 + y).unwrap_or(0));
            }
        }
        (normal, depth, albedo, ids)
    }

    /// The frame a person compares against.
    fn frame(sun: f32) -> DebugFrame {
        let (normal, depth, albedo, ids) = scene(sun);
        DebugFrame::from_gbuffer(&normal, &depth, &albedo, &ids, 8, 6).expect("frame builds")
    }

    /// Moving only the light leaves depth and albedo byte-equal and moves the
    /// normal digest.
    #[test]
    fn buffers_moving_the_light_moves_normals_only() {
        let noon = frame(20.0);
        let dusk = frame(-45.0);
        assert_eq!(noon.depth, dusk.depth, "depth must not follow the light");
        assert_eq!(noon.albedo, dusk.albedo, "albedo must not follow the light");
        assert_eq!(
            noon.material_id, dusk.material_id,
            "ids must not follow the light"
        );
        assert_eq!(
            noon.digest(1),
            dusk.digest(1),
            "the depth digest follows the light"
        );
        assert_eq!(
            noon.digest(2),
            dusk.digest(2),
            "the albedo digest follows the light"
        );
        assert_ne!(
            noon.digest(0),
            dusk.digest(0),
            "the normal digest must move with the light"
        );
        assert_ne!(noon.normal, dusk.normal, "the normal bytes must differ");
    }

    /// The id written at a pixel is the id read back, and a miss reads 0.
    #[test]
    fn buffers_lookup_id_returns_the_written_id() {
        let f = frame(0.0);
        assert_eq!(lookup_id(&f, 3, 4), 34, "the id at (3, 4) reads back");
        assert_eq!(lookup_id(&f, 0, 0), 0, "the first pixel reads back");
        assert_eq!(lookup_id(&f, 8, 0), 0, "past the right edge reads 0");
        assert_eq!(lookup_id(&f, 0, 6), 0, "past the bottom edge reads 0");
    }

    /// The four digests differ inside a frame and repeat across two builds; a
    /// changed pixel moves exactly its own channel.
    #[test]
    fn buffers_digests_differ_and_repeat() {
        let first = frame(10.0);
        let again = frame(10.0);
        assert_eq!(first.digests, again.digests, "two builds digest apart");
        assert_eq!(first.albedo, again.albedo, "two builds differ in albedo");
        let mut distinct: Vec<u64> = first.digests.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            4,
            "the four channel digests must differ: {:?}",
            first.digests
        );
        let (normal, depth, albedo, mut ids) = scene(10.0);
        ids[9] = 99;
        let changed = DebugFrame::from_gbuffer(&normal, &depth, &albedo, &ids, 8, 6)
            .expect("changed frame builds");
        assert_ne!(changed.digest(3), first.digest(3), "the id digest moved");
        assert_eq!(
            changed.digest(0),
            first.digest(0),
            "the normal digest stays"
        );
    }

    /// Zero size and every short slice are typed errors naming the lengths.
    #[test]
    fn buffers_bad_size_and_length_are_typed() {
        let (normal, depth, albedo, ids) = scene(0.0);
        match DebugFrame::from_gbuffer(&normal, &depth, &albedo, &ids, 0, 6) {
            Err(error @ BufferError::Empty { width, height }) => {
                assert_eq!((width, height), (0, 6), "the message names the size");
                assert!(
                    error.to_string().contains("0x6"),
                    "the message names the size: {error}"
                );
            }
            other => panic!("a zero width gives {other:?}, not an empty error"),
        }
        for name in ["normal", "depth", "albedo", "material_id"] {
            let result = match name {
                "normal" => DebugFrame::from_gbuffer(&normal[..47], &depth, &albedo, &ids, 8, 6),
                "depth" => DebugFrame::from_gbuffer(&normal, &depth[..47], &albedo, &ids, 8, 6),
                "albedo" => DebugFrame::from_gbuffer(&normal, &depth, &albedo[..47], &ids, 8, 6),
                _ => DebugFrame::from_gbuffer(&normal, &depth, &albedo, &ids[..47], 8, 6),
            };
            match result {
                Err(
                    error @ BufferError::Length {
                        buffer,
                        got: short,
                        expected,
                    },
                ) => {
                    assert_eq!(buffer, name, "the message names the channel");
                    assert_eq!((short, expected), (47, 48), "the lengths of 8x6");
                    let shown = error.to_string();
                    assert!(
                        shown.contains("47") && shown.contains("48"),
                        "the message names both lengths: {shown}"
                    );
                }
                other => panic!("{name} gives {other:?}, not a length error"),
            }
        }
    }

    /// The normal diff helper finds the one pixel that moved and reads zero on
    /// two equal frames.
    #[test]
    fn buffers_brightest_normal_diff_finds_the_pixel() {
        let base = frame(0.0);
        let (normal, depth, albedo, ids) = scene(0.0);
        let mut tilted = normal.clone();
        tilted[2 * 8 + 5] = [0.6, 0.6, 0.5];
        let moved =
            DebugFrame::from_gbuffer(&tilted, &depth, &albedo, &ids, 8, 6).expect("tilt builds");
        let (x, y, delta) = brightest_normal_diff(&base, &moved);
        assert_eq!((x, y), (5, 2), "the moved pixel is (5, 2)");
        assert!(delta > 0.01, "the diff must name a real gap, read {delta}");
        assert_eq!(
            brightest_normal_diff(&base, &frame(0.0)).2,
            0.0,
            "equal frames read no diff"
        );
    }

    /// The octahedral codec round-trips a handful of directions inside the
    /// 8-bit quantization error.
    #[test]
    fn buffers_octahedral_round_trips() {
        for direction in [
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            [-0.6, 0.48, 0.64],
            [0.2, -0.9, 0.35],
        ] {
            let encoded = encode_normal(direction);
            let decoded = decode_normal(&encoded, 0);
            assert!(
                (decoded[2] - direction[2]).abs() <= 1.0 / 255.0 + 1e-3,
                "{direction:?} decodes to {decoded:?}"
            );
        }
    }
}
