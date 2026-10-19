//! Machine eyes: integer image metrics over RGBA8 frames.
//!
//! Consumer: the look judge (`super::judge`) and any gate that scores a
//! capture against a reference instead of opening it by eye. Data owner: the
//! caller owns the pixels; [`Frame`] owns one validated, decoded copy.
//! Units: SSIM in fixed point where [`SSIM_ONE`] (10000) is 1.0, deltaE in
//! hundredths of a CIE76 unit, histogram distances in fixed point where
//! 10000 is the largest possible distance, FLIP visibility in ten-thousandths
//! of the full black/white difference ([`FLIP_ONE`] is the largest mean),
//! luma in `0..=255`.
//! Determinism: integer and fixed-point only (one 256-entry sRGB table, one
//! integer cube root, one integer square root), row-major order, no float
//! anywhere, so the same inputs give the same numbers on every machine and
//! the JSON built from them is byte-identical twice.
//! Errors: [`MetricError`] names the bad dimension, length, region or PNG;
//! nothing here panics on caller data.
//! Budget: one or a few passes over the region per metric; a 2048 x 2048
//! frame is a few tens of milliseconds in release (FLIP adds two smoothing
//! passes and one Sobel pass). PNG decode is bounded to the capture ceiling
//! ([`MAX_DIM`] per side) before any pixel buffer is allocated.
//!
//! Alpha never counts: captures are opaque, and every metric reads the RGB
//! channels only. Colour management is not applied: the PNG's 8-bit sRGB
//! values are scored as they are.

use std::fmt;
use std::io::Cursor;

use image::{ImageFormat, ImageReader, Limits};

use crate::frame_check::MAX_FRAME_DIM;

/// Fixed-point 1.0 for [`Ssim::score_x10000`]: identical images score this.
pub const SSIM_ONE: i32 = 10_000;
/// SSIM window edge in pixels (non-overlapping 8 x 8 tiles).
pub const WINDOW: usize = 8;
/// Luminance histogram bins (luma divided by four).
pub const HIST_BINS: usize = 64;
/// Largest frame edge accepted, equal to the frame-check ceiling.
pub const MAX_DIM: u32 = MAX_FRAME_DIM;
/// Largest decoded PNG allocation in bytes (RGBA8 at [`MAX_DIM`] squared).
const MAX_DECODE_BYTES: u64 = 4 * 2048 * 2048;

/// Why a metric refused its inputs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MetricError {
    /// Width or height is zero or above [`MAX_DIM`].
    BadDimensions {
        /// Offered width in pixels.
        width: u32,
        /// Offered height in pixels.
        height: u32,
    },
    /// The RGBA byte count is not `width * height * 4`.
    BadLength {
        /// Bytes the dimensions call for.
        expected: usize,
        /// Bytes offered.
        got: usize,
    },
    /// The two frames differ in size.
    SizeMismatch {
        /// First frame `(width, height)`.
        left: (u32, u32),
        /// Second frame `(width, height)`.
        right: (u32, u32),
    },
    /// A rectangle reaches outside the frame or has no area.
    BadRect {
        /// The offered rectangle.
        rect: Rect,
        /// Frame `(width, height)` it was checked against.
        frame: (u32, u32),
    },
    /// A mask is not one flag per pixel.
    BadMask {
        /// Flags the frame needs.
        expected: usize,
        /// Flags offered.
        got: usize,
    },
    /// The region holds no pixel (or no whole SSIM window, or an empty histogram).
    EmptyRegion,
    /// A clip holds fewer frames than one consecutive pair.
    TooFewFrames {
        /// Frames offered.
        got: usize,
        /// Frames one pair needs.
        need: usize,
    },
    /// The PNG did not decode within the capture bounds.
    BadPng(String),
}

impl fmt::Display for MetricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDimensions { width, height } => write!(
                f,
                "bad frame size {width}x{height}: use 1..={MAX_DIM} on each side"
            ),
            Self::BadLength { expected, got } => write!(
                f,
                "frame holds {got} RGBA bytes but its size needs {expected}"
            ),
            Self::SizeMismatch { left, right } => write!(
                f,
                "frames differ in size ({}x{} vs {}x{}): compare equal sizes",
                left.0, left.1, right.0, right.1
            ),
            Self::BadRect { rect, frame } => write!(
                f,
                "rect {}x{} at ({},{}) does not fit inside the {}x{} frame",
                rect.w, rect.h, rect.x, rect.y, frame.0, frame.1
            ),
            Self::BadMask { expected, got } => {
                write!(f, "mask holds {got} flags but the frame needs {expected}")
            }
            Self::EmptyRegion => write!(
                f,
                "region holds no scorable pixels: widen the rect or mask (SSIM needs a whole 8x8 window)"
            ),
            Self::TooFewFrames { got, need } => write!(
                f,
                "clip holds {got} frames but flicker needs at least {need}: score one consecutive pair"
            ),
            Self::BadPng(reason) => write!(f, "png did not decode: {reason}"),
        }
    }
}

impl std::error::Error for MetricError {}

/// One axis-aligned pixel rectangle, origin top-left.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

/// Where a metric scores: the whole frame, a rectangle or a per-pixel mask.
#[derive(Clone, Copy, Debug)]
pub enum Region<'a> {
    /// Every pixel.
    Full,
    /// Pixels inside the rectangle only.
    Rect(Rect),
    /// Pixels whose flag is set, row-major, one flag per pixel.
    Mask(&'a [bool]),
}

/// A region resolved against one frame size: bounds plus the optional mask.
struct Resolved<'a> {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    width: usize,
    mask: Option<&'a [bool]>,
}

impl<'a> Resolved<'a> {
    fn new(region: Region<'a>, width: usize, height: usize) -> Result<Self, MetricError> {
        match region {
            Region::Full => Ok(Self {
                x0: 0,
                y0: 0,
                x1: width,
                y1: height,
                width,
                mask: None,
            }),
            Region::Rect(rect) => {
                let bad = || MetricError::BadRect {
                    rect,
                    frame: (to_u32(width), to_u32(height)),
                };
                let x0 = usize::try_from(rect.x).map_err(|_| bad())?;
                let y0 = usize::try_from(rect.y).map_err(|_| bad())?;
                let w = usize::try_from(rect.w).map_err(|_| bad())?;
                let h = usize::try_from(rect.h).map_err(|_| bad())?;
                let x1 = x0.checked_add(w).ok_or_else(bad)?;
                let y1 = y0.checked_add(h).ok_or_else(bad)?;
                if w == 0 || h == 0 || x1 > width || y1 > height {
                    return Err(bad());
                }
                Ok(Self {
                    x0,
                    y0,
                    x1,
                    y1,
                    width,
                    mask: None,
                })
            }
            Region::Mask(mask) => {
                let expected = width * height;
                if mask.len() != expected {
                    return Err(MetricError::BadMask {
                        expected,
                        got: mask.len(),
                    });
                }
                Ok(Self {
                    x0: 0,
                    y0: 0,
                    x1: width,
                    y1: height,
                    width,
                    mask: Some(mask),
                })
            }
        }
    }

    fn contains(&self, x: usize, y: usize) -> bool {
        self.mask.is_none_or(|mask| mask[y * self.width + x])
    }

    /// Visit every pixel index inside the region in row-major order.
    fn for_each(&self, mut visit: impl FnMut(usize)) {
        for y in self.y0..self.y1 {
            for x in self.x0..self.x1 {
                if self.contains(x, y) {
                    visit(y * self.width + x);
                }
            }
        }
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// One validated RGBA8 frame, row-major from the top-left.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Frame {
    /// Wrap raw RGBA8 bytes, refusing a zero or oversize edge and a wrong
    /// byte count.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, MetricError> {
        let bad = MetricError::BadDimensions { width, height };
        if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
            return Err(bad);
        }
        let w = usize::try_from(width).map_err(|_| bad.clone())?;
        let h = usize::try_from(height).map_err(|_| bad)?;
        let expected = w * h * 4;
        if rgba.len() != expected {
            return Err(MetricError::BadLength {
                expected,
                got: rgba.len(),
            });
        }
        Ok(Self {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Decode PNG bytes into RGBA8 within the capture bounds. The size is
    /// checked from the header, before the pixel buffer exists.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, MetricError> {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIM);
        limits.max_image_height = Some(MAX_DIM);
        limits.max_alloc = Some(MAX_DECODE_BYTES);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|error| MetricError::BadPng(error.to_string()))?
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        Self::from_rgba(width, height, decoded.into_raw())
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        to_u32(self.width)
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        to_u32(self.height)
    }

    /// Raw RGBA8 bytes, row-major.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Pixel count (`width * height`).
    pub fn pixels(&self) -> usize {
        self.width * self.height
    }

    /// RGB of the pixel at a row-major index, or black when out of range.
    pub fn rgb_at(&self, index: usize) -> [u8; 3] {
        let at = index * 4;
        match self.rgba.get(at..at + 3) {
            Some([r, g, b]) => [*r, *g, *b],
            _ => [0, 0, 0],
        }
    }

    /// Luma plane, one byte per pixel (BT.601 weights in integers).
    pub fn luma_plane(&self) -> Vec<u8> {
        self.rgba
            .chunks_exact(4)
            .map(|px| luma_of([px[0], px[1], px[2]]))
            .collect()
    }

    fn same_size(&self, other: &Self) -> Result<(), MetricError> {
        if self.width == other.width && self.height == other.height {
            Ok(())
        } else {
            Err(MetricError::SizeMismatch {
                left: (self.width(), self.height()),
                right: (other.width(), other.height()),
            })
        }
    }
}

/// BT.601 luma of one RGB pixel in integers: weights 77, 150, 29 over 256.
pub fn luma_of(rgb: [u8; 3]) -> u8 {
    let sum = 77 * u32::from(rgb[0]) + 150 * u32::from(rgb[1]) + 29 * u32::from(rgb[2]) + 128;
    u8::try_from(sum >> 8).unwrap_or(u8::MAX)
}

/// Round-half-up integer division for a positive denominator.
fn div_round(num: i128, den: i128) -> i128 {
    (2 * num + den).div_euclid(2 * den)
}

// ---------------------------------------------------------------------------
// SSIM
// ---------------------------------------------------------------------------

/// Structural similarity over 8 x 8 luma windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ssim {
    /// Mean window SSIM, fixed point: [`SSIM_ONE`] is identical, `0` is
    /// unrelated, negative is anti-correlated.
    pub score_x10000: i32,
    /// Windows scored.
    pub windows: u32,
}

/// Window SSIM scaled by 1,000,000 from the five window sums over 64 luma
/// samples. Constants are `(0.01 * 255)^2` and `(0.03 * 255)^2` times
/// `64^2`, so the whole expression stays integer. Largest term is
/// `2 * 16320 * 16320 + C1` times a similar factor, near `2.8e17`, inside
/// `i64`; the final scaling uses `i128`.
fn window_ssim_ppm(sx: i64, sy: i64, sxx: i64, syy: i64, sxy: i64) -> i64 {
    const N: i64 = 64;
    const C1_N2: i64 = 26_634;
    const C2_N2: i64 = 239_708;
    let lum_num = 2 * sx * sy + C1_N2;
    let lum_den = sx * sx + sy * sy + C1_N2;
    let cs_num = 2 * (N * sxy - sx * sy) + C2_N2;
    let cs_den = N * (sxx + syy) - sx * sx - sy * sy + C2_N2;
    let num = i128::from(lum_num) * i128::from(cs_num);
    let den = i128::from(lum_den) * i128::from(cs_den);
    i64::try_from(div_round(num * 1_000_000, den).clamp(-1_000_000, 1_000_000)).unwrap_or(0)
}

/// Mean SSIM of two equal-size frames over luma, in non-overlapping 8 x 8
/// windows tiled from the region origin (the rectangle's corner, else the
/// frame's). A mask scores only windows whose 64 pixels are all inside it.
/// Edge pixels that do not fill a window are not scored.
///
/// # Errors
///
/// [`MetricError::SizeMismatch`], a bad rectangle or mask, or
/// [`MetricError::EmptyRegion`] when no whole window fits.
pub fn ssim(a: &Frame, b: &Frame, region: Region<'_>) -> Result<Ssim, MetricError> {
    a.same_size(b)?;
    let area = Resolved::new(region, a.width, a.height)?;
    let (la, lb) = (a.luma_plane(), b.luma_plane());
    let mut total: i64 = 0;
    let mut windows: u32 = 0;
    let mut wy = area.y0;
    while wy + WINDOW <= area.y1 {
        let mut wx = area.x0;
        while wx + WINDOW <= area.x1 {
            if window_inside(&area, wx, wy) {
                let sums = window_sums(&la, &lb, a.width, wx, wy);
                total += window_ssim_ppm(sums[0], sums[1], sums[2], sums[3], sums[4]);
                windows += 1;
            }
            wx += WINDOW;
        }
        wy += WINDOW;
    }
    if windows == 0 {
        return Err(MetricError::EmptyRegion);
    }
    let mean = div_round(i128::from(total), 100 * i128::from(windows));
    let score =
        i32::try_from(mean.clamp(-i128::from(SSIM_ONE), i128::from(SSIM_ONE))).unwrap_or(SSIM_ONE);
    Ok(Ssim {
        score_x10000: score,
        windows,
    })
}

fn window_inside(area: &Resolved<'_>, wx: usize, wy: usize) -> bool {
    area.mask.is_none() || (wy..wy + WINDOW).all(|y| (wx..wx + WINDOW).all(|x| area.contains(x, y)))
}

/// `[sum a, sum b, sum a^2, sum b^2, sum a*b]` over one window of luma.
fn window_sums(la: &[u8], lb: &[u8], width: usize, wx: usize, wy: usize) -> [i64; 5] {
    let mut sums = [0i64; 5];
    for y in wy..wy + WINDOW {
        let row = y * width + wx;
        for (&pa, &pb) in la[row..row + WINDOW].iter().zip(&lb[row..row + WINDOW]) {
            let (pa, pb) = (i64::from(pa), i64::from(pb));
            sums[0] += pa;
            sums[1] += pb;
            sums[2] += pa * pa;
            sums[3] += pb * pb;
            sums[4] += pa * pb;
        }
    }
    sums
}

// ---------------------------------------------------------------------------
// deltaE (CIE76)
// ---------------------------------------------------------------------------

/// sRGB 8-bit value to linear light, scaled by 10,000,000 (IEC 61966-2-1).
const SRGB_LINEAR_E7: [u32; 256] = [
    0, 3_035, 6_071, 9_106, 12_141, 15_176, 18_212, 21_247, 24_282, 27_317, 30_353, 33_465, 36_765,
    40_247, 43_914, 47_770, 51_815, 56_054, 60_488, 65_121, 69_954, 74_990, 80_232, 85_681, 91_341,
    97_212, 103_298, 109_601, 116_122, 122_865, 129_830, 137_021, 144_438, 152_085, 159_963,
    168_074, 176_420, 185_002, 193_824, 202_886, 212_190, 221_739, 231_534, 241_576, 251_869,
    262_412, 273_209, 284_260, 295_568, 307_134, 318_960, 331_048, 343_398, 356_013, 368_895,
    382_044, 395_462, 409_152, 423_114, 437_350, 451_862, 466_651, 481_718, 497_066, 512_695,
    528_606, 544_803, 561_285, 578_054, 595_112, 612_461, 630_100, 648_033, 666_259, 684_782,
    703_601, 722_719, 742_136, 761_854, 781_874, 802_198, 822_827, 843_762, 865_005, 886_556,
    908_417, 930_590, 953_075, 975_873, 998_987, 1_022_417, 1_046_165, 1_070_231, 1_094_617,
    1_119_324, 1_144_354, 1_169_707, 1_195_384, 1_221_388, 1_247_718, 1_274_377, 1_301_365,
    1_328_683, 1_356_333, 1_384_316, 1_412_633, 1_441_285, 1_470_273, 1_499_598, 1_529_262,
    1_559_265, 1_589_608, 1_620_294, 1_651_322, 1_682_694, 1_714_411, 1_746_474, 1_778_884,
    1_811_642, 1_844_750, 1_878_208, 1_912_017, 1_946_178, 1_980_693, 2_015_563, 2_050_787,
    2_086_369, 2_122_308, 2_158_605, 2_195_262, 2_232_280, 2_269_659, 2_307_400, 2_345_506,
    2_383_976, 2_422_811, 2_462_013, 2_501_583, 2_541_521, 2_581_829, 2_622_507, 2_663_556,
    2_704_978, 2_746_773, 2_788_943, 2_831_487, 2_874_408, 2_917_706, 2_961_383, 3_005_438,
    3_049_873, 3_094_689, 3_139_887, 3_185_468, 3_231_432, 3_277_781, 3_324_515, 3_371_636,
    3_419_144, 3_467_041, 3_515_326, 3_564_001, 3_613_068, 3_662_526, 3_712_377, 3_762_621,
    3_813_260, 3_864_294, 3_915_725, 3_967_552, 4_019_778, 4_072_402, 4_125_426, 4_178_851,
    4_232_677, 4_286_905, 4_341_536, 4_396_572, 4_452_012, 4_507_858, 4_564_110, 4_620_770,
    4_677_838, 4_735_315, 4_793_202, 4_851_499, 4_910_208, 4_969_330, 5_028_865, 5_088_813,
    5_149_177, 5_209_956, 5_271_151, 5_332_764, 5_394_795, 5_457_245, 5_520_114, 5_583_404,
    5_647_115, 5_711_248, 5_775_804, 5_840_784, 5_906_188, 5_972_018, 6_038_273, 6_104_956,
    6_172_066, 6_239_604, 6_307_571, 6_375_969, 6_444_797, 6_514_056, 6_583_748, 6_653_873,
    6_724_432, 6_795_425, 6_866_853, 6_938_718, 7_011_019, 7_083_758, 7_156_935, 7_230_551,
    7_304_607, 7_379_104, 7_454_042, 7_529_422, 7_605_245, 7_681_511, 7_758_222, 7_835_378,
    7_912_979, 7_991_027, 8_069_523, 8_148_466, 8_227_858, 8_307_699, 8_387_990, 8_468_732,
    8_549_926, 8_631_572, 8_713_671, 8_796_224, 8_879_231, 8_962_694, 9_046_612, 9_130_987,
    9_215_819, 9_301_109, 9_386_857, 9_473_065, 9_559_734, 9_646_862, 9_734_453, 9_822_506,
    9_911_021, 10_000_000,
];

/// sRGB to XYZ (D65) matrix scaled by 10,000,000.
const RGB_TO_XYZ_E7: [[i64; 3]; 3] = [
    [4_124_564, 3_575_761, 1_804_375],
    [2_126_729, 7_151_522, 721_750],
    [193_339, 1_191_920, 9_503_041],
];
/// D65 white point scaled by 10,000,000.
const WHITE_E7: [i64; 3] = [9_504_700, 10_000_000, 10_888_300];
/// CIE epsilon `(6/29)^3` scaled by 10,000,000.
const LAB_EPSILON_E7: i64 = 88_565;
/// Floor of the cube root of `v`, by binary search (no float).
fn icbrt(v: u64) -> u64 {
    let (mut lo, mut hi) = (0u64, 2_097_152u64);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if mid * mid * mid <= v {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// Lab companding function at scale 1,000,000 for a ratio at scale 10^7.
fn lab_f_e6(t_e7: i64) -> i64 {
    if t_e7 > LAB_EPSILON_E7 {
        i64::try_from(icbrt(u64::try_from(t_e7).unwrap_or(0) * 100_000_000_000)).unwrap_or(0)
    } else {
        (t_e7 * 7_787_037 + 50_000_000) / 100_000_000 + 137_931
    }
}

/// CIE L*a*b* of one sRGB pixel in hundredths of a unit: `[L, a, b]`.
pub fn lab_x100(rgb: [u8; 3]) -> [i64; 3] {
    let lin = rgb.map(|c| i64::from(SRGB_LINEAR_E7[usize::from(c)]));
    let xyz = RGB_TO_XYZ_E7
        .map(|row| (row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2] + 5_000_000) / 10_000_000);
    let f = [0, 1, 2].map(|i| lab_f_e6(xyz[i] * 10_000_000 / WHITE_E7[i]));
    let to_x100 = |e6: i64| i64::try_from(div_round(i128::from(e6), 10_000)).unwrap_or(0);
    [
        to_x100(116 * f[1] - 16_000_000),
        to_x100(500 * (f[0] - f[1])),
        to_x100(200 * (f[1] - f[2])),
    ]
}

/// CIE76 distance between two Lab triples (see [`lab_x100`]) in hundredths
/// of a unit: the rounded Euclidean norm, integer square root only.
pub fn lab_distance_x100(a: [i64; 3], b: [i64; 3]) -> u32 {
    let sq: i64 = (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum();
    let sq = u64::try_from(sq).unwrap_or(0);
    let root = sq.isqrt();
    let rounded = if sq - root * root > root {
        root + 1
    } else {
        root
    };
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

/// CIE76 colour difference of two sRGB pixels in hundredths of a unit.
pub fn delta_e_x100(a: [u8; 3], b: [u8; 3]) -> u32 {
    if a == b {
        return 0;
    }
    lab_distance_x100(lab_x100(a), lab_x100(b))
}

/// Mean and worst CIE76 colour difference over a region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeltaE {
    /// Mean per-pixel deltaE in hundredths of a unit.
    pub mean_x100: u32,
    /// Largest per-pixel deltaE in hundredths of a unit.
    pub max_x100: u32,
    /// Pixels scored.
    pub pixels: u64,
}

/// Mean CIE76 deltaE between two equal-size frames over a region.
///
/// # Errors
///
/// [`MetricError::SizeMismatch`], a bad rectangle or mask, or
/// [`MetricError::EmptyRegion`] when the mask selects nothing.
pub fn mean_delta_e(a: &Frame, b: &Frame, region: Region<'_>) -> Result<DeltaE, MetricError> {
    a.same_size(b)?;
    let area = Resolved::new(region, a.width, a.height)?;
    let (mut sum, mut max, mut pixels) = (0u64, 0u32, 0u64);
    area.for_each(|index| {
        let de = delta_e_x100(a.rgb_at(index), b.rgb_at(index));
        sum += u64::from(de);
        max = max.max(de);
        pixels += 1;
    });
    if pixels == 0 {
        return Err(MetricError::EmptyRegion);
    }
    let mean = div_round(i128::from(sum), i128::from(pixels));
    Ok(DeltaE {
        mean_x100: u32::try_from(mean).unwrap_or(u32::MAX),
        max_x100: max,
        pixels,
    })
}

// ---------------------------------------------------------------------------
// Luminance histogram
// ---------------------------------------------------------------------------

/// 64-bin luminance histogram (luma divided by four) over a region.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Histogram {
    bins: [u32; HIST_BINS],
    total: u32,
}

impl Histogram {
    /// Pixel count per bin, dark to bright.
    pub fn bins(&self) -> &[u32; HIST_BINS] {
        &self.bins
    }

    /// Pixels counted.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// Bins holding at least `permille` of the pixels (at least one pixel).
    pub fn occupied_bins(&self, permille: u32) -> u32 {
        let floor = (u64::from(self.total) * u64::from(permille))
            .div_ceil(1000)
            .max(1);
        let count = self.bins.iter().filter(|&&n| u64::from(n) >= floor).count();
        to_u32(count)
    }
}

/// Luminance histogram of one frame over a region.
///
/// # Errors
///
/// A bad rectangle or mask, or [`MetricError::EmptyRegion`] when the mask
/// selects nothing.
pub fn luma_histogram(frame: &Frame, region: Region<'_>) -> Result<Histogram, MetricError> {
    let area = Resolved::new(region, frame.width, frame.height)?;
    let mut bins = [0u32; HIST_BINS];
    let mut total = 0u32;
    area.for_each(|index| {
        let bin = usize::from(luma_of(frame.rgb_at(index)) >> 2);
        bins[bin] += 1;
        total += 1;
    });
    if total == 0 {
        return Err(MetricError::EmptyRegion);
    }
    Ok(Histogram { bins, total })
}

/// Distance between two luminance histograms, both normalised to unit mass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HistDistance {
    /// Total variation: half the summed absolute bin difference, `0` for
    /// identical mass, `10000` for disjoint bins.
    pub tv_x10000: u32,
    /// Earth mover's distance over the bin line, `10000` when all mass moves
    /// from the darkest to the brightest bin. Grows with the size of a
    /// brightness shift where total variation saturates.
    pub emd_x10000: u32,
}

/// Distance between two histograms; either side may count different pixel
/// totals.
///
/// # Errors
///
/// [`MetricError::EmptyRegion`] when either histogram is empty.
pub fn histogram_distance(a: &Histogram, b: &Histogram) -> Result<HistDistance, MetricError> {
    if a.total == 0 || b.total == 0 {
        return Err(MetricError::EmptyRegion);
    }
    let (ta, tb) = (i128::from(a.total), i128::from(b.total));
    let mut tv_sum: i128 = 0;
    let mut emd_sum: i128 = 0;
    let (mut cum_a, mut cum_b) = (0i128, 0i128);
    for (&na, &nb) in a.bins.iter().zip(&b.bins) {
        tv_sum += (i128::from(na) * tb - i128::from(nb) * ta).abs();
        cum_a += i128::from(na);
        cum_b += i128::from(nb);
        emd_sum += (cum_a * tb - cum_b * ta).abs();
    }
    let tv = div_round(tv_sum * 10_000, 2 * ta * tb);
    let steps = i128::from(to_u32(HIST_BINS - 1));
    let emd = div_round(emd_sum * 10_000, ta * tb * steps);
    Ok(HistDistance {
        tv_x10000: u32::try_from(tv).unwrap_or(10_000),
        emd_x10000: u32::try_from(emd).unwrap_or(10_000),
    })
}

// ---------------------------------------------------------------------------
// Flicker (edge-weighted visibility)
// ---------------------------------------------------------------------------

/// Bar in permille: a clip at or above this reads as visible flicker.
pub const FLICKER_BAR_PERMILLE: u32 = 100;
/// Largest motion-compensation shift searched on each axis, in pixels.
pub const FLICKER_SHIFT_MAX: i32 = 2;
/// Sobel gradient below this is flat and never weighs into the score.
const FLICKER_EDGE_FLOOR: u64 = 12;
/// Largest edge weight, so one hard edge cannot drown the rest of a frame.
const FLICKER_EDGE_CAP: u64 = 255;

/// Flicker visibility of a clip: the edge-weighted residual the eye would see.
///
/// Model (idea only, no transplanted bytes): consecutive frames are motion
/// compensated by a small integer shift, and the leftover luma residual is
/// weighted by local edge energy. Flat areas that change brightness are
/// invisible and weigh zero; hard edges that alternate frame to frame weigh
/// the most. Integer only, so the same clip scores the same twice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Flicker {
    /// Mean pair score in permille, `0..=1000` ([`FLICKER_BAR_PERMILLE`] is
    /// the visible bar).
    pub permille: u32,
    /// Worst consecutive pair in permille, `0..=1000`.
    pub worst_permille: u32,
    /// Consecutive pairs scored.
    pub pairs: u32,
}

/// Edge energy of one luma plane: integer Sobel magnitude per pixel, less the
/// flat `floor` and capped at `cap`. Zero on a flat area.
fn sobel_weighted(luma: &[u8], width: usize, height: usize, floor: u64, cap: u64) -> Vec<u16> {
    let mut weights = vec![0u16; width * height];
    if width < 3 || height < 3 {
        return weights;
    }
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let at = |dx: usize, dy: usize| i64::from(luma[(y + dy - 1) * width + (x + dx - 1)]);
            let gx = (at(2, 0) + 2 * at(2, 1) + at(2, 2)) - (at(0, 0) + 2 * at(0, 1) + at(0, 2));
            let gy = (at(0, 2) + 2 * at(1, 2) + at(2, 2)) - (at(0, 0) + 2 * at(1, 0) + at(2, 0));
            let magnitude = gx.unsigned_abs() + gy.unsigned_abs();
            let weight = magnitude.saturating_sub(floor).min(cap);
            weights[y * width + x] = u16::try_from(weight).unwrap_or(0);
        }
    }
    weights
}

/// Edge energy of one luma plane for the flicker score: Sobel magnitude less
/// the flat floor, capped at [`FLICKER_EDGE_CAP`]. Zero on a flat area.
fn flicker_edge_weights(luma: &[u8], width: usize, height: usize) -> Vec<u16> {
    sobel_weighted(luma, width, height, FLICKER_EDGE_FLOOR, FLICKER_EDGE_CAP)
}

/// Weighted residual in permille of full scale, rounded; zero without weight.
fn residual_permille(numerator: u64, weight: u64) -> u32 {
    if weight == 0 {
        return 0;
    }
    let scaled =
        (u128::from(numerator) * 1000 + u128::from(weight) * 255 / 2) / (u128::from(weight) * 255);
    u32::try_from(scaled.min(1000)).unwrap_or(1000)
}

/// Flicker score of one consecutive pair in permille.
///
/// The pair is compensated by the best shift within
/// [`FLICKER_SHIFT_MAX`] pixels (smallest weighted residual; the zero shift
/// wins a tie), then the residual is weighted by local edge energy from the
/// first frame. A flat region scores zero however far its brightness moves.
///
/// # Errors
///
/// [`MetricError::SizeMismatch`], a bad rectangle or mask,
/// [`MetricError::EmptyRegion`] when the region holds no pixel with room for
/// the shift search.
pub fn flicker_pair(a: &Frame, b: &Frame, region: Region<'_>) -> Result<u32, MetricError> {
    a.same_size(b)?;
    let area = Resolved::new(region, a.width, a.height)?;
    let (w, h) = (a.width, a.height);
    let (la, lb) = (a.luma_plane(), b.luma_plane());
    let weights = flicker_edge_weights(&la, w, h);
    let margin = 1 + usize::try_from(FLICKER_SHIFT_MAX).unwrap_or(0);
    if w <= 2 * margin || h <= 2 * margin {
        return Err(MetricError::EmptyRegion);
    }
    let (x0, x1) = (margin, w - margin);
    let (y0, y1) = (margin, h - margin);
    let mut weight_total = 0u64;
    let mut visited = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            if area.contains(x, y) {
                visited += 1;
                weight_total += u64::from(weights[y * w + x]);
            }
        }
    }
    if visited == 0 {
        return Err(MetricError::EmptyRegion);
    }
    if weight_total == 0 {
        return Ok(0);
    }
    let shift_max = FLICKER_SHIFT_MAX;
    let mut shifts: Vec<(i32, i32)> = Vec::with_capacity(25);
    shifts.push((0, 0));
    for dy in -shift_max..=shift_max {
        for dx in -shift_max..=shift_max {
            if dx != 0 || dy != 0 {
                shifts.push((dx, dy));
            }
        }
    }
    let mut best = u64::MAX;
    for &(dx, dy) in &shifts {
        let (dx, dy) = (i64::from(dx), i64::from(dy));
        let mut residual = 0u64;
        for y in y0..y1 {
            let by = usize::try_from(i64::try_from(y).unwrap_or(0) + dy).unwrap_or(0);
            for x in x0..x1 {
                if !area.contains(x, y) {
                    continue;
                }
                let bx = usize::try_from(i64::try_from(x).unwrap_or(0) + dx).unwrap_or(0);
                let weight = u64::from(weights[y * w + x]);
                if weight == 0 {
                    continue;
                }
                let diff = u64::from(la[y * w + x].abs_diff(lb[by * w + bx]));
                residual += diff * weight;
            }
        }
        if residual < best {
            best = residual;
        }
    }
    Ok(residual_permille(best, weight_total))
}

/// Flicker visibility of a whole clip over a region (see [`Flicker`]).
///
/// # Errors
///
/// [`MetricError::TooFewFrames`] under two frames, plus every error
/// [`flicker_pair`] raises for a pair.
pub fn flicker_hvei(frames: &[Frame], region: Region<'_>) -> Result<Flicker, MetricError> {
    if frames.len() < 2 {
        return Err(MetricError::TooFewFrames {
            got: frames.len(),
            need: 2,
        });
    }
    let mut sum = 0u64;
    let mut worst = 0u32;
    for pair in frames.windows(2) {
        let score = flicker_pair(&pair[0], &pair[1], region)?;
        sum += u64::from(score);
        worst = worst.max(score);
    }
    let pairs = to_u32(frames.len() - 1);
    let mean = u32::try_from((sum + u64::from(pairs) / 2) / u64::from(pairs)).unwrap_or(u32::MAX);
    Ok(Flicker {
        permille: mean,
        worst_permille: worst,
        pairs,
    })
}

// ---------------------------------------------------------------------------
// FLIP (feature-aware perceptual difference)
// ---------------------------------------------------------------------------

/// Fixed-point 1.0 for [`Flip`]: the largest mean a pair can reach (a full
/// black-against-white difference at every pixel).
pub const FLIP_ONE: u32 = 10_000;
/// Chroma weight per 10,000 against the luma difference: chroma is the less
/// visible half of a colour change.
const FLIP_CHROMA_WEIGHT: u32 = 4_500;
/// Visibility weight per 10,000 on a flat pixel: away from edges a difference
/// counts for less.
const FLIP_BASE_WEIGHT: u32 = 4_000;
/// Visibility weight per 10,000 at a full-strength edge.
const FLIP_FEATURE_WEIGHT: u32 = 14_000;
/// Sobel magnitude that counts as a full-strength feature.
const FLIP_FEATURE_CAP: u64 = 255;

/// Feature-aware perceptual difference of one frame against one reference.
///
/// The FLIP shape (NVlabs, HPG 2020; idea and structure only, no transplanted
/// bytes or weight tables): the colour difference is taken in YCxCyCz over
/// linear light, smoothed once with a contrast-sensitivity kernel (binomial
/// `1,4,6,4,1` over 16), then weighted up where either frame carries an edge.
/// Integer and fixed point only, so the same pair scores the same on every
/// machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Flip {
    /// Mean visibility over the region in ten-thousandths of the full
    /// black/white difference ([`FLIP_ONE`] is the largest possible mean);
    /// `0` when the frames are identical.
    pub mean_x10000: u32,
    /// Worst single pixel in the same units.
    pub max_x10000: u32,
    /// Pixels scored.
    pub pixels: u64,
}

/// Perceptual YCxCyCz of one sRGB pixel at scale 10^7: `[Y, Cx, Cy, Cz]`
/// (BT.709 luma over linear light, then each linear channel minus Y).
fn ycxcycz_e7(rgb: [u8; 3]) -> [i64; 4] {
    let lin = rgb.map(|c| i64::from(SRGB_LINEAR_E7[usize::from(c)]));
    let y = (2_126_560 * lin[0] + 7_151_580 * lin[1] + 721_860 * lin[2] + 5_000_000) / 10_000_000;
    [y, lin[0] - y, lin[1] - y, lin[2] - y]
}

/// Raw colour difference of two pixels at scale 10^7: `|dY|` plus the summed
/// chroma magnitude at [`FLIP_CHROMA_WEIGHT`], capped at the full scale.
fn flip_raw_diff(a: [u8; 3], b: [u8; 3]) -> u32 {
    let (ya, yb) = (ycxcycz_e7(a), ycxcycz_e7(b));
    let luma = (ya[0] - yb[0]).unsigned_abs();
    let chroma = (ya[1] - yb[1]).unsigned_abs()
        + (ya[2] - yb[2]).unsigned_abs()
        + (ya[3] - yb[3]).unsigned_abs();
    let chroma = (chroma * u64::from(FLIP_CHROMA_WEIGHT) + 5_000) / 10_000;
    u32::try_from((luma + chroma).min(10_000_000)).unwrap_or(10_000_000)
}

/// One contrast-sensitivity pass over `src`: the binomial `1,4,6,4,1` over 16
/// along rows (`along_rows`) or columns, edges clamped, scale unchanged.
fn csf_pass(src: &[u32], width: usize, height: usize, along_rows: bool) -> Vec<u32> {
    let mut out = vec![0u32; src.len()];
    for y in 0..height {
        for x in 0..width {
            let (count, at) = if along_rows { (width, x) } else { (height, y) };
            let mut sum = 0u64;
            for (k, weight) in [1u64, 4, 6, 4, 1].into_iter().enumerate() {
                let pos = (at + k).saturating_sub(2).min(count - 1);
                let index = if along_rows {
                    y * width + pos
                } else {
                    pos * width + x
                };
                sum += weight * u64::from(src[index]);
            }
            out[y * width + x] = u32::try_from((sum + 8) / 16).unwrap_or(u32::MAX);
        }
    }
    out
}

/// Visibility weight per 10,000 for a pixel whose best edge strength is `s`.
fn flip_feature_weight(s: u16) -> u64 {
    let base = u64::from(FLIP_BASE_WEIGHT);
    let span = u64::from(FLIP_FEATURE_WEIGHT - FLIP_BASE_WEIGHT);
    base + span * u64::from(s) / FLIP_FEATURE_CAP
}

/// Mean and worst FLIP visibility between two equal-size frames over a region.
///
/// # Errors
///
/// [`MetricError::SizeMismatch`], a bad rectangle or mask, or
/// [`MetricError::EmptyRegion`] when the region selects nothing.
pub fn flip(a: &Frame, b: &Frame, region: Region<'_>) -> Result<Flip, MetricError> {
    a.same_size(b)?;
    let area = Resolved::new(region, a.width, a.height)?;
    let (w, h) = (a.width, a.height);
    let raw: Vec<u32> = (0..w * h)
        .map(|i| flip_raw_diff(a.rgb_at(i), b.rgb_at(i)))
        .collect();
    let smoothed = csf_pass(&csf_pass(&raw, w, h, true), w, h, false);
    let features: Vec<u16> = {
        let (la, lb) = (a.luma_plane(), b.luma_plane());
        let (wa, wb) = (
            sobel_weighted(&la, w, h, 0, FLIP_FEATURE_CAP),
            sobel_weighted(&lb, w, h, 0, FLIP_FEATURE_CAP),
        );
        wa.into_iter().zip(wb).map(|(x, y)| x.max(y)).collect()
    };
    let (mut sum, mut max, mut pixels) = (0u64, 0u32, 0u64);
    area.for_each(|index| {
        let visible = u64::from(smoothed[index]) * flip_feature_weight(features[index]) / 10_000;
        sum += visible;
        max = max.max(u32::try_from(visible / 1_000).unwrap_or(FLIP_ONE));
        pixels += 1;
    });
    if pixels == 0 {
        return Err(MetricError::EmptyRegion);
    }
    let mean = div_round(i128::from(sum) * 10_000, i128::from(pixels) * 10_000_000);
    Ok(Flip {
        mean_x10000: u32::try_from(mean.clamp(0, i128::from(FLIP_ONE))).unwrap_or(FLIP_ONE),
        max_x10000: max.min(FLIP_ONE),
        pixels,
    })
}

// ---------------------------------------------------------------------------
// One-shot comparison
// ---------------------------------------------------------------------------

/// SSIM, deltaE, histogram distance and FLIP of one frame against one
/// reference.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Comparison {
    /// SSIM, [`SSIM_ONE`] is identical.
    pub ssim_x10000: i32,
    /// Mean CIE76 deltaE in hundredths.
    pub mean_de_x100: u32,
    /// Luminance histogram total variation, `0` identical.
    pub hist_tv_x10000: u32,
    /// Luminance histogram earth mover's distance, `0` identical.
    pub hist_emd_x10000: u32,
    /// FLIP mean visibility, `0` identical.
    pub flip_x10000: u32,
}

/// Score `frame` against `reference` over `region` with every metric.
///
/// # Errors
///
/// Any [`MetricError`] the five metrics raise.
pub fn compare(
    frame: &Frame,
    reference: &Frame,
    region: Region<'_>,
) -> Result<Comparison, MetricError> {
    let structure = ssim(frame, reference, region)?;
    let colour = mean_delta_e(frame, reference, region)?;
    let distance = histogram_distance(
        &luma_histogram(frame, region)?,
        &luma_histogram(reference, region)?,
    )?;
    let visible = flip(frame, reference, region)?;
    Ok(Comparison {
        ssim_x10000: structure.score_x10000,
        mean_de_x100: colour.mean_x100,
        hist_tv_x10000: distance.tv_x10000,
        hist_emd_x10000: distance.emd_x10000,
        flip_x10000: visible.mean_x10000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

    const W: u32 = 64;
    const H: u32 = 48;

    /// Textured sand: a fixed integer pattern over a tan base, so windows
    /// carry real variance (flat windows cannot show a contrast change).
    fn sand_at(x: u32, y: u32) -> [u8; 3] {
        let t = i32::try_from((x * 7 + y * 13 + (x * y) % 5) % 23).expect("small") - 11;
        let base = [194i32, 160, 104];
        base.map(|c| u8::try_from((c + t).clamp(0, 255)).expect("clamped"))
    }

    fn build(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 3]) -> Frame {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = paint(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Frame::from_rgba(width, height, rgba).expect("frame builds")
    }

    fn brighten20(rgb: [u8; 3]) -> [u8; 3] {
        rgb.map(|v| u8::try_from(((u32::from(v) * 6 + 2) / 5).min(255)).expect("clamped"))
    }

    fn sand() -> Frame {
        build(W, H, sand_at)
    }

    /// The sand region (every row from y = 8 down) brightened by 20 percent.
    fn brightened_sand() -> Frame {
        build(W, H, |x, y| {
            if y >= 8 {
                brighten20(sand_at(x, y))
            } else {
                sand_at(x, y)
            }
        })
    }

    fn shifted_sand() -> Frame {
        build(W, H, |x, y| sand_at((x + 1).min(W - 1), y))
    }

    fn png_of(frame: &Frame) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(
                frame.rgba(),
                frame.width(),
                frame.height(),
                ExtendedColorType::Rgba8,
            )
            .expect("png encodes");
        out
    }

    #[test]
    fn look_flicker_clean_clip_scores_below_the_bar() {
        // A clean pan: one integer pixel per frame. Motion compensation makes
        // the residual zero, so the clip reads as no flicker.
        let pan = |offset: u32| build(W, H, |x, y| sand_at((x + offset).min(W - 1), y));
        let clip = vec![pan(0), pan(1), pan(2), pan(3)];
        let score = flicker_hvei(&clip, Region::Full).expect("clean clip scores");
        assert_eq!(score.pairs, 3);
        assert!(
            score.permille < FLICKER_BAR_PERMILLE,
            "clean pan scored {} permille, worst {}",
            score.permille,
            score.worst_permille
        );
        // Moving fighter on still sand: a small body travels across the frame.
        let body = |at: u32| {
            build(W, H, |x, y| {
                let inside = (at..at + 8).contains(&x) && (20..28).contains(&y);
                if inside {
                    [235, 235, 235]
                } else {
                    sand_at(x, y)
                }
            })
        };
        let moving = vec![body(8), body(9), body(10)];
        let fighter = flicker_hvei(&moving, Region::Full).expect("fighter clip scores");
        assert!(
            fighter.permille < FLICKER_BAR_PERMILLE,
            "moving fighter scored {} permille",
            fighter.permille
        );
    }

    #[test]
    fn look_flicker_alternating_shimmer_clip_scores_above_the_bar() {
        // The same scene, alternating 40 luma steps every frame: visible
        // shimmer on every textured edge.
        let shimmer = |up: bool| {
            build(W, H, |x, y| {
                let step = if up { 40i32 } else { -40i32 };
                sand_at(x, y)
                    .map(|c| u8::try_from((i32::from(c) + step).clamp(0, 255)).expect("clamped"))
            })
        };
        let clip = vec![shimmer(true), shimmer(false), shimmer(true), shimmer(false)];
        let score = flicker_hvei(&clip, Region::Full).expect("shimmer clip scores");
        assert_eq!(score.pairs, 3);
        assert!(
            score.permille > FLICKER_BAR_PERMILLE,
            "shimmer scored {} permille, bar {}",
            score.permille,
            FLICKER_BAR_PERMILLE
        );
        assert!(score.worst_permille >= score.permille);
    }

    #[test]
    fn look_flicker_flat_region_never_trips_the_guard() {
        // Two flat frames far apart in brightness: no edge energy, no flicker.
        let flat = |luma: u8| build(W, H, move |_, _| [luma, luma, luma]);
        let score = flicker_hvei(&[flat(40), flat(220)], Region::Full).expect("flat pair scores");
        assert_eq!((score.permille, score.worst_permille), (0, 0));
        // A gentle one-step ramp is under the edge floor, same answer.
        let ramp = |offset: u32| {
            build(W, H, move |x, _| {
                let v = u8::try_from((x + offset).min(255)).expect("byte");
                [v, v, v]
            })
        };
        let soft = flicker_hvei(&[ramp(0), ramp(100)], Region::Full).expect("ramp pair scores");
        assert_eq!(soft.permille, 0);
        // The same brightness step on textured sand does score: the guard is
        // about edges, not about the amount of change.
        let textured = flicker_hvei(
            &[sand(), build(W, H, |x, y| brighten20(sand_at(x, y)))],
            Region::Full,
        )
        .expect("textured pair scores");
        assert!(textured.permille > 0);
    }

    #[test]
    fn look_flicker_same_inputs_give_identical_results_twice() {
        let clip = || vec![sand(), brightened_sand(), sand()];
        let first = flicker_hvei(&clip(), Region::Full).expect("first");
        let second = flicker_hvei(&clip(), Region::Full).expect("second");
        assert_eq!(first, second);
        assert!(first.worst_permille >= first.permille);
        // Typed errors, never a panic: too few frames, and a size mismatch.
        assert_eq!(
            flicker_hvei(&[sand()], Region::Full).unwrap_err(),
            MetricError::TooFewFrames { got: 1, need: 2 }
        );
        let small = build(16, 16, |_, _| [9, 9, 9]);
        assert_eq!(
            flicker_pair(&sand(), &small, Region::Full).unwrap_err(),
            MetricError::SizeMismatch {
                left: (64, 48),
                right: (16, 16)
            }
        );
        // A frame too thin for the shift search has no scorable pixel.
        let thin = build(5, 5, |_, _| [9, 9, 9]);
        assert_eq!(
            flicker_hvei(&[thin.clone(), thin], Region::Full).unwrap_err(),
            MetricError::EmptyRegion
        );
    }

    #[test]
    fn look_metric_equal_frame_scores_ssim_one_and_delta_e_zero() {
        let frame = sand();
        let same = ssim(&frame, &frame, Region::Full).expect("ssim");
        assert_eq!(same.score_x10000, SSIM_ONE);
        assert_eq!(same.windows, 48);
        let de = mean_delta_e(&frame, &frame, Region::Full).expect("delta e");
        assert_eq!((de.mean_x100, de.max_x100, de.pixels), (0, 0, 3072));
        let set = compare(&frame, &frame, Region::Full).expect("compare");
        assert_eq!(
            set,
            Comparison {
                ssim_x10000: SSIM_ONE,
                mean_de_x100: 0,
                hist_tv_x10000: 0,
                hist_emd_x10000: 0,
                flip_x10000: 0,
            }
        );
    }

    #[test]
    fn look_metric_brightened_sand_region_drops_ssim_below_9800() {
        // Float oracle (independent script, same frames): full 9775, sand
        // rect 9730, rows above the sand 10000.
        let (reference, live) = (sand(), brightened_sand());
        let full = ssim(&live, &reference, Region::Full).expect("full ssim");
        assert_eq!(full.score_x10000, 9775);
        assert!(full.score_x10000 < 9800);
        let rect = Rect {
            x: 0,
            y: 8,
            w: 64,
            h: 40,
        };
        let inside = ssim(&live, &reference, Region::Rect(rect)).expect("rect ssim");
        assert_eq!((inside.score_x10000, inside.windows), (9730, 40));
        let above = Rect {
            x: 0,
            y: 0,
            w: 64,
            h: 8,
        };
        let outside = ssim(&live, &reference, Region::Rect(above)).expect("outside ssim");
        assert_eq!((outside.score_x10000, outside.windows), (SSIM_ONE, 8));
    }

    #[test]
    fn look_metric_triple_pins_equal_and_brightened_sand() {
        // One `compare()` triple over `Region::Full` pins SSIM, deltaE and
        // histogram together on the sand pair the Accept names.
        let frame = sand();
        let equal = compare(&frame, &frame, Region::Full).expect("equal triple");
        assert_eq!(
            equal,
            Comparison {
                ssim_x10000: SSIM_ONE,
                mean_de_x100: 0,
                hist_tv_x10000: 0,
                hist_emd_x10000: 0,
                flip_x10000: 0,
            }
        );
        let (reference, live) = (sand(), brightened_sand());
        let diff = compare(&live, &reference, Region::Full).expect("brightened triple");
        assert_eq!(diff.ssim_x10000, 9775);
        assert!(diff.ssim_x10000 < 9800, "got {}", diff.ssim_x10000);
        assert_eq!(diff.mean_de_x100, 1111);
        assert!(diff.mean_de_x100 > 0);
        assert!(diff.hist_tv_x10000 > 0, "tv {}", diff.hist_tv_x10000);
        assert!(diff.hist_emd_x10000 > 0, "emd {}", diff.hist_emd_x10000);
        let rerun = compare(&live, &reference, Region::Full).expect("rerun triple");
        assert_eq!(diff, rerun);
    }

    #[test]
    fn look_metric_mask_matches_the_rect_it_spells() {
        let (reference, live) = (sand(), brightened_sand());
        let mask: Vec<bool> = (0..H).flat_map(|y| (0..W).map(move |_| y >= 8)).collect();
        let by_mask = ssim(&live, &reference, Region::Mask(&mask)).expect("mask ssim");
        assert_eq!((by_mask.score_x10000, by_mask.windows), (9730, 40));
        let rect = Rect {
            x: 0,
            y: 8,
            w: 64,
            h: 40,
        };
        let de_rect = mean_delta_e(&live, &reference, Region::Rect(rect)).expect("rect de");
        let de_mask = mean_delta_e(&live, &reference, Region::Mask(&mask)).expect("mask de");
        assert_eq!(de_rect, de_mask);
        assert_eq!(de_rect.pixels, 2560);
        // Float oracle: 13.33 inside the sand, 11.11 over the whole frame.
        assert_eq!(de_rect.mean_x100, 1333);
        let whole = mean_delta_e(&live, &reference, Region::Full).expect("full de");
        assert_eq!(whole.mean_x100, 1111);
        assert_eq!(whole.max_x100, 1387);
    }

    #[test]
    fn look_metric_shifted_frame_pins_its_scores() {
        // Float oracle: SSIM 2711, mean deltaE 3.46, worst 7.47 (the integer
        // Lab rounds each channel to a hundredth, so the worst pixel reads 7.48).
        let (reference, live) = (sand(), shifted_sand());
        let structure = ssim(&live, &reference, Region::Full).expect("ssim");
        assert_eq!(structure.score_x10000, 2711);
        let colour = mean_delta_e(&live, &reference, Region::Full).expect("de");
        assert_eq!(colour.mean_x100, 346);
        assert_eq!(colour.max_x100, 748);
    }

    #[test]
    fn look_metric_same_inputs_give_identical_results_twice() {
        let (reference, live) = (sand(), brightened_sand());
        let first = compare(&live, &reference, Region::Full).expect("first");
        let second = compare(&live, &reference, Region::Full).expect("second");
        assert_eq!(first, second);
        let again = compare(&brightened_sand(), &sand(), Region::Full).expect("rebuilt");
        assert_eq!(first, again);
    }

    #[test]
    fn look_metric_flip_identical_frames_score_zero() {
        let frame = sand();
        let equal = flip(&frame, &frame, Region::Full).expect("flip");
        assert_eq!((equal.mean_x10000, equal.max_x10000), (0, 0));
        assert_eq!(equal.pixels, 3072);
    }

    #[test]
    fn look_metric_flip_scores_brightened_sand_above_the_shift() {
        let reference = sand();
        let shifted = flip(&shifted_sand(), &reference, Region::Full).expect("shift");
        let bright = flip(&brightened_sand(), &reference, Region::Full).expect("brighten");
        // Pinned integers (same pair twice gives the same line): the CSF pass
        // damps the shift's high-frequency detail, while the brightness change
        // is broad and reads above it.
        assert_eq!(
            (shifted.mean_x10000, bright.mean_x10000, bright.max_x10000),
            (321, 1268, 2270)
        );
        assert!(
            shifted.mean_x10000 > 0,
            "a one-pixel shift is visible, got {}",
            shifted.mean_x10000
        );
        assert!(
            bright.mean_x10000 > shifted.mean_x10000,
            "brightened sand {} must read above the shift {}",
            bright.mean_x10000,
            shifted.mean_x10000
        );
        assert!(bright.mean_x10000 <= FLIP_ONE);
        assert!(bright.max_x10000 >= bright.mean_x10000);
        // The same pair twice gives the same numbers.
        let again = flip(&brightened_sand(), &reference, Region::Full).expect("again");
        assert_eq!(bright, again);
    }

    #[test]
    fn look_metric_flip_respects_the_region_and_refuses_bad_shapes() {
        let (reference, live) = (sand(), brightened_sand());
        let above = Rect {
            x: 0,
            y: 0,
            w: 64,
            h: 8,
        };
        let untouched = flip(&live, &reference, Region::Rect(above)).expect("above");
        assert_eq!(untouched.pixels, 512);
        // The 5-tap smoothing bleeds two rows of the change upward: the top
        // rows are byte-equal, so the reads are small but not zero.
        assert_eq!(untouched.mean_x10000, 119);
        let rows = Rect {
            x: 0,
            y: 8,
            w: 64,
            h: 40,
        };
        let sand_rows = flip(&live, &reference, Region::Rect(rows)).expect("sand rows");
        assert_eq!(sand_rows.pixels, 2560);
        assert!(
            sand_rows.mean_x10000 > untouched.mean_x10000,
            "the changed rows {} must read above the byte-equal rows {}",
            sand_rows.mean_x10000,
            untouched.mean_x10000
        );
        let small = build(16, 16, |_, _| [9, 9, 9]);
        assert_eq!(
            flip(&sand(), &small, Region::Full).unwrap_err(),
            MetricError::SizeMismatch {
                left: (64, 48),
                right: (16, 16)
            }
        );
        let none = vec![false; 3072];
        assert_eq!(
            flip(&live, &reference, Region::Mask(&none)).unwrap_err(),
            MetricError::EmptyRegion
        );
    }

    #[test]
    fn look_metric_delta_e_matches_known_colour_pairs() {
        // Float oracle: black/white 100.00, red/black 117.33, green/blue
        // 258.68, grey 128 vs 120 3.15 (hundredths, allow one unit).
        let near = |got: u32, want: u32| got.abs_diff(want) <= 1;
        assert!(near(delta_e_x100([0, 0, 0], [255, 255, 255]), 10_000));
        assert!(near(delta_e_x100([255, 0, 0], [0, 0, 0]), 11_733));
        assert!(near(delta_e_x100([0, 255, 0], [0, 0, 255]), 25_868));
        assert!(near(delta_e_x100([128, 128, 128], [120, 120, 120]), 315));
        assert_eq!(delta_e_x100([12, 34, 56], [12, 34, 56]), 0);
        assert_eq!(
            delta_e_x100([10, 200, 90], [200, 10, 90]),
            delta_e_x100([200, 10, 90], [10, 200, 90])
        );
        // The Lab of white is L 100, a 0, b 0.
        let white = lab_x100([255, 255, 255]);
        assert_eq!(white[0], 10_000);
        assert!(white[1].abs() <= 1 && white[2].abs() <= 1);
        assert_eq!(lab_x100([0, 0, 0]), [0, 0, 0]);
    }

    #[test]
    fn look_metric_srgb_table_is_monotone_with_pinned_ends() {
        assert_eq!(SRGB_LINEAR_E7[0], 0);
        assert_eq!(SRGB_LINEAR_E7[255], 10_000_000);
        assert_eq!(SRGB_LINEAR_E7[128], 2_158_605);
        assert!(SRGB_LINEAR_E7.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn look_metric_integer_cube_root_is_exact() {
        for root in [0u64, 1, 2, 999, 1_000_000, 1_038_000] {
            assert_eq!(icbrt(root * root * root), root);
            if root > 0 {
                assert_eq!(icbrt(root * root * root - 1), root - 1);
            }
        }
    }

    #[test]
    fn look_metric_histogram_distances_pin_known_shapes() {
        let dark = build(16, 16, |_, _| [0, 0, 0]);
        let bright = build(16, 16, |_, _| [255, 255, 255]);
        let near_dark = build(16, 16, |_, _| [4, 4, 4]);
        let half = build(
            16,
            16,
            |_, y| if y < 8 { [0, 0, 0] } else { [255, 255, 255] },
        );
        let hist = |frame: &Frame| luma_histogram(frame, Region::Full).expect("histogram");
        let distance =
            |a: &Frame, b: &Frame| histogram_distance(&hist(a), &hist(b)).expect("distance");
        let at = |tv_x10000, emd_x10000| HistDistance {
            tv_x10000,
            emd_x10000,
        };
        assert_eq!(distance(&dark, &dark), at(0, 0));
        assert_eq!(distance(&dark, &bright), at(10_000, 10_000));
        assert_eq!(distance(&dark, &half), at(5_000, 5_000));
        // One bin of brightness shift saturates total variation but not EMD.
        assert_eq!(distance(&dark, &near_dark), at(10_000, 159));
        let h = hist(&half);
        assert_eq!((h.bins()[0], h.bins()[63], h.total()), (128, 128, 256));
        assert_eq!(h.occupied_bins(100), 2);
        assert!(hist(&sand()).occupied_bins(1) > 4);
    }

    #[test]
    fn look_metric_histogram_respects_the_region() {
        let frame = build(
            16,
            16,
            |_, y| if y < 8 { [0, 0, 0] } else { [255, 255, 255] },
        );
        let top = Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 8,
        };
        let hist = luma_histogram(&frame, Region::Rect(top)).expect("top histogram");
        assert_eq!((hist.bins()[0], hist.total()), (128, 128));
        let none = vec![false; 256];
        assert_eq!(
            luma_histogram(&frame, Region::Mask(&none)).unwrap_err(),
            MetricError::EmptyRegion
        );
    }

    #[test]
    fn look_metric_refuses_bad_shapes_loudly() {
        let frame = sand();
        let small = build(16, 16, |_, _| [9, 9, 9]);
        assert_eq!(
            ssim(&frame, &small, Region::Full).unwrap_err(),
            MetricError::SizeMismatch {
                left: (64, 48),
                right: (16, 16)
            }
        );
        let outside = Rect {
            x: 60,
            y: 0,
            w: 8,
            h: 8,
        };
        assert!(matches!(
            mean_delta_e(&frame, &frame, Region::Rect(outside)),
            Err(MetricError::BadRect { .. })
        ));
        let empty = Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 8,
        };
        assert!(matches!(
            ssim(&frame, &frame, Region::Rect(empty)),
            Err(MetricError::BadRect { .. })
        ));
        assert_eq!(
            ssim(&frame, &frame, Region::Mask(&[true; 4])).unwrap_err(),
            MetricError::BadMask {
                expected: 3072,
                got: 4
            }
        );
        let sliver = Rect {
            x: 0,
            y: 0,
            w: 7,
            h: 48,
        };
        assert_eq!(
            ssim(&frame, &frame, Region::Rect(sliver)).unwrap_err(),
            MetricError::EmptyRegion
        );
        assert_eq!(
            Frame::from_rgba(0, 4, Vec::new()).unwrap_err(),
            MetricError::BadDimensions {
                width: 0,
                height: 4
            }
        );
        assert_eq!(
            Frame::from_rgba(2, 2, vec![0; 15]).unwrap_err(),
            MetricError::BadLength {
                expected: 16,
                got: 15
            }
        );
        assert!(matches!(
            Frame::from_rgba(MAX_DIM + 1, 1, Vec::new()),
            Err(MetricError::BadDimensions { .. })
        ));
        assert!(MetricError::EmptyRegion
            .to_string()
            .contains("whole 8x8 window"));
    }

    #[test]
    fn look_metric_ssim_extremes_stay_inside_the_fixed_point_range() {
        let black = build(16, 16, |_, _| [0, 0, 0]);
        let white = build(16, 16, |_, _| [255, 255, 255]);
        let low = ssim(&black, &white, Region::Full).expect("black vs white");
        assert!(
            (0..=1).contains(&low.score_x10000),
            "got {}",
            low.score_x10000
        );
        let stripes = build(16, 16, |x, _| {
            if x % 2 == 0 {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            }
        });
        let inverse = build(16, 16, |x, _| {
            if x % 2 == 0 {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        });
        let anti = ssim(&stripes, &inverse, Region::Full).expect("anti-correlated");
        assert!(anti.score_x10000 < 0 && anti.score_x10000 >= -SSIM_ONE);
    }

    #[test]
    fn look_metric_png_round_trips_and_refuses_bad_bytes() {
        let frame = sand();
        let bytes = png_of(&frame);
        assert_eq!(Frame::decode_png(&bytes).expect("png decodes"), frame);
        assert!(matches!(
            Frame::decode_png(b"not a png at all"),
            Err(MetricError::BadPng(_))
        ));
        assert!(matches!(
            Frame::decode_png(&bytes[..bytes.len() / 2]),
            Err(MetricError::BadPng(_))
        ));
        // The capture ceiling decodes; one pixel past it is refused from the header.
        let edge = usize::try_from(MAX_DIM).expect("fits");
        let ceiling = Frame::from_rgba(MAX_DIM, 1, vec![7; edge * 4]).expect("ceiling frame");
        assert!(Frame::decode_png(&png_of(&ceiling)).is_ok());
        let mut over = Vec::new();
        PngEncoder::new(&mut over)
            .write_image(
                &vec![7u8; (edge + 1) * 4],
                MAX_DIM + 1,
                1,
                ExtendedColorType::Rgba8,
            )
            .expect("oversize png encodes");
        assert!(matches!(
            Frame::decode_png(&over),
            Err(MetricError::BadPng(_))
        ));
    }
}
