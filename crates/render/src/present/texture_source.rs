//! texture_source: the shapes one material texture reaches the GPU in.
//!
//! Owner: the render texture upload path ([`TextureSource`] and the receipt
//! types; the presenter's block upload lives in `gpu_present::compressed`).
//! Consumer: the game arena kit loader, which hands over cooked KTX2 bytes.
//! Units: texels for sizes, bytes for every counter. Determinism: pure values,
//! no clock and no GPU; the same bytes validate the same way everywhere.
//!
//! RGBA8 is the original shape: decoded pixels, and the presenter builds their
//! mip chain. Block is the cooked shape: the KTX2 cook's own BC7 blocks with
//! their full mip chain, uploaded as they are (no decode, no re-encode).
//!
//! BC5 has no variant here on purpose. The forward shader normalizes the texel
//! it samples (`eg12_decode_normal`), so a normal map must carry its Z channel;
//! BC5 holds two channels, so a BC5 sample would read Z at -1. Normal maps keep
//! the RGBA8 shape until the shader rebuilds Z from XY or the cook writes them
//! as BC7.

use super::surface::PresentError;
use super::MAX_TEXTURE_DIM;

/// Block edge in texels: every BC block covers 4x4 texels.
pub const BLOCK_EDGE: u32 = 4;
/// Bytes per BC7 block: 128 bits per 4x4 texels.
pub const BC7_BLOCK_BYTES: usize = 16;
/// Longest legal mip chain (4096 texels down to 1x1).
pub const MAX_MIP_LEVELS: usize = 13;

/// Texels one level holds (`None` on 32-bit overflow).
fn level_texels(width: u32, height: u32) -> Option<usize> {
    usize::try_from(u64::from(width) * u64::from(height)).ok()
}

/// Block shape of a compressed upload. The two BC7 shapes differ only in the
/// transfer rule the sampler applies: sRGB decode, or the raw linear bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BlockFormat {
    /// Colour: BC7 with the sRGB transfer rule (`Bc7RgbaUnormSrgb`).
    Bc7Srgb,
    /// Linear data: BC7 with no transfer rule (`Bc7RgbaUnorm`).
    Bc7Linear,
}

impl BlockFormat {
    /// Bytes one 4x4 block occupies.
    #[must_use]
    pub fn block_bytes(self) -> usize {
        match self {
            Self::Bc7Srgb | Self::Bc7Linear => BC7_BLOCK_BYTES,
        }
    }

    /// What a receipt prints for this shape.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Bc7Srgb => "bc7-rgba-unorm-srgb",
            Self::Bc7Linear => "bc7-rgba-unorm",
        }
    }
}

/// One mip level of block bytes, row-major in blocks, tightly packed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockLevel {
    /// Level width in texels (`1..=MAX_TEXTURE_DIM`).
    pub width: u32,
    /// Level height in texels (`1..=MAX_TEXTURE_DIM`).
    pub height: u32,
    /// Exactly `blocks_wide * blocks_high * block_bytes` bytes.
    pub bytes: Vec<u8>,
}

impl BlockLevel {
    /// Blocks across this level.
    #[must_use]
    pub fn blocks_wide(&self) -> u32 {
        self.width.div_ceil(BLOCK_EDGE)
    }

    /// Blocks down this level.
    #[must_use]
    pub fn blocks_high(&self) -> u32 {
        self.height.div_ceil(BLOCK_EDGE)
    }

    /// Byte length this level needs in `format` (`None` on 32-bit overflow).
    #[must_use]
    pub fn byte_len(format: BlockFormat, width: u32, height: u32) -> Option<usize> {
        let blocks = u64::from(width.div_ceil(BLOCK_EDGE)) * u64::from(height.div_ceil(BLOCK_EDGE));
        let total = blocks * u64::try_from(format.block_bytes()).ok()?;
        usize::try_from(total).ok()
    }
}

/// Block-compressed texture upload: base dims, shape, and the cooked block
/// bytes of every level, base level first.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockUpload {
    /// Base level width in texels.
    pub width: u32,
    /// Base level height in texels.
    pub height: u32,
    /// Block shape (sRGB colour or linear data).
    pub format: BlockFormat,
    /// Block bytes per level, base level first, down to 1x1.
    pub levels: Vec<BlockLevel>,
}

impl BlockUpload {
    /// Validate dims, the halving level chain and every level's byte length.
    /// The GPU upload refuses anything this refuses; no level is padded here.
    pub fn validate(&self) -> Result<(), PresentError> {
        if self.width == 0 || self.height == 0 {
            return Err(PresentError::InvalidTexture(format!(
                "zero area {}x{}",
                self.width, self.height
            )));
        }
        if self.width > MAX_TEXTURE_DIM || self.height > MAX_TEXTURE_DIM {
            return Err(PresentError::InvalidTexture(format!(
                "size {}x{} exceeds {MAX_TEXTURE_DIM}",
                self.width, self.height
            )));
        }
        if self.levels.is_empty() {
            return Err(PresentError::InvalidTexture(
                "block upload carries no mip level".to_string(),
            ));
        }
        // The longest legal chain is 13 levels at 4096; anything past the
        // chain would repeat 1x1 and only waste VRAM.
        if self.levels.len() > MAX_MIP_LEVELS {
            return Err(PresentError::InvalidTexture(format!(
                "{} mip levels exceed the {MAX_MIP_LEVELS} a chain may hold",
                self.levels.len()
            )));
        }
        for (index, level) in self.levels.iter().enumerate() {
            let want = ((self.width >> index).max(1), (self.height >> index).max(1));
            if (level.width, level.height) != want {
                return Err(PresentError::InvalidTexture(format!(
                    "level {index} is {}x{}, the chain says {}x{}",
                    level.width, level.height, want.0, want.1
                )));
            }
            let Some(len) = BlockLevel::byte_len(self.format, level.width, level.height) else {
                return Err(PresentError::InvalidTexture(format!(
                    "level {index} byte length overflows"
                )));
            };
            if level.bytes.len() != len {
                return Err(PresentError::InvalidTexture(format!(
                    "level {index} is {} bytes, {}x{} in {} needs {len}",
                    level.bytes.len(),
                    level.width,
                    level.height,
                    self.format.label()
                )));
            }
        }
        Ok(())
    }

    /// Block bytes of every level: what the GPU allocates for this upload.
    #[must_use]
    pub fn block_bytes(&self) -> usize {
        self.levels.iter().map(|level| level.bytes.len()).sum()
    }

    /// The same chain decoded to RGBA8 (`width * height * 4` per level):
    /// what the RGBA8 shape would have allocated for the same mips.
    #[must_use]
    pub fn rgba8_bytes(&self) -> usize {
        self.levels.iter().fold(0usize, |sum, level| {
            let texels = level_texels(level.width, level.height).unwrap_or(0);
            sum.saturating_add(texels.saturating_mul(4))
        })
    }

    /// Base level as RGBA8: the size the pre-block upload held for this map.
    #[must_use]
    pub fn base_rgba8_bytes(&self) -> usize {
        level_texels(self.width, self.height).unwrap_or(0) * 4
    }

    /// The largest mip whose longest edge is at most `max_edge`, as an upload
    /// of its own (the tier's `texture_max_px`). `None` when the upload already
    /// fits, or when `max_edge` is zero.
    #[must_use]
    pub fn capped_to(&self, max_edge: u32) -> Option<Self> {
        if max_edge == 0 || self.width.max(self.height) <= max_edge {
            return None;
        }
        let at = self
            .levels
            .iter()
            .position(|level| level.width.max(level.height) <= max_edge)?;
        let base = &self.levels[at];
        Some(Self {
            width: base.width,
            height: base.height,
            format: self.format,
            levels: self.levels[at..].to_vec(),
        })
    }
}

/// What a caller hands the presenter for one texture: decoded pixels, or the
/// cooked block bytes that go to the GPU as they are.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TextureSource {
    /// Decoded RGBA8 pixels; the presenter builds their mip chain.
    Rgba8(super::cache::TextureUpload),
    /// Block bytes with their own mip chain.
    Block(BlockUpload),
}

impl TextureSource {
    /// Base level width in texels.
    #[must_use]
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgba8(upload) => upload.width,
            Self::Block(upload) => upload.width,
        }
    }

    /// Base level height in texels.
    #[must_use]
    pub fn height(&self) -> u32 {
        match self {
            Self::Rgba8(upload) => upload.height,
            Self::Block(upload) => upload.height,
        }
    }

    /// Base level as RGBA8 bytes: the size the original upload held.
    #[must_use]
    pub fn rgba8_base_bytes(&self) -> usize {
        level_texels(self.width(), self.height()).unwrap_or(0) * 4
    }

    /// Validate whichever shape this carries.
    pub fn validate(&self) -> Result<(), PresentError> {
        match self {
            Self::Rgba8(upload) => upload.validate(),
            Self::Block(upload) => upload.validate(),
        }
    }
}

impl From<super::cache::TextureUpload> for TextureSource {
    fn from(upload: super::cache::TextureUpload) -> Self {
        Self::Rgba8(upload)
    }
}

/// The shape a texture is actually resident in, read back from the GPU
/// texture for receipts: what was asked for is not what must be reported.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureShape {
    /// `Rgba8UnormSrgb` pixels.
    Rgba8Srgb,
    /// `Rgba8Unorm` linear pixels.
    Rgba8Linear,
    /// `Bc7RgbaUnormSrgb` colour blocks.
    Bc7Srgb,
    /// `Bc7RgbaUnorm` linear data blocks.
    Bc7Linear,
    /// Any other format a future path allocates.
    Other,
}

impl TextureShape {
    /// What a receipt prints for this shape.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Rgba8Srgb => "rgba8-unorm-srgb",
            Self::Rgba8Linear => "rgba8-unorm",
            Self::Bc7Srgb => BlockFormat::Bc7Srgb.label(),
            Self::Bc7Linear => BlockFormat::Bc7Linear.label(),
            Self::Other => "other",
        }
    }

    /// True when the texture sits in VRAM as blocks.
    #[must_use]
    pub fn is_block_compressed(self) -> bool {
        matches!(self, Self::Bc7Srgb | Self::Bc7Linear)
    }
}

/// One shape's share of the resident material textures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextureShapeSlice {
    /// Format the GPU holds these maps in.
    pub shape: TextureShape,
    /// Maps resident in this shape.
    pub maps: usize,
    /// Bytes these maps occupy in VRAM, every mip level counted.
    pub bytes: u64,
    /// What the same maps would occupy as RGBA8, every mip level counted.
    pub rgba8_bytes: u64,
}

/// What the GPU actually holds for the resident material textures, and what
/// the RGBA8 shape would have held for the same maps. Built by
/// `Presenter::texture_receipt`, never assumed from the upload source.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TextureReceipt {
    /// One slice per shape, ordered by first appearance.
    pub slices: Vec<TextureShapeSlice>,
    /// Maps counted.
    pub maps: usize,
    /// Bytes held, every mip level counted.
    pub bytes: u64,
    /// Bytes the same maps would take as RGBA8, every mip level counted.
    pub rgba8_bytes: u64,
}

impl TextureReceipt {
    /// Record one resident map.
    pub fn add(&mut self, shape: TextureShape, bytes: u64, rgba8_bytes: u64) {
        self.maps += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        self.rgba8_bytes = self.rgba8_bytes.saturating_add(rgba8_bytes);
        match self.slices.iter_mut().find(|slice| slice.shape == shape) {
            Some(slice) => {
                slice.maps += 1;
                slice.bytes = slice.bytes.saturating_add(bytes);
                slice.rgba8_bytes = slice.rgba8_bytes.saturating_add(rgba8_bytes);
            }
            None => self.slices.push(TextureShapeSlice {
                shape,
                maps: 1,
                bytes,
                rgba8_bytes,
            }),
        }
    }

    /// Maps that sit in VRAM as blocks.
    #[must_use]
    pub fn block_maps(&self) -> usize {
        self.slices
            .iter()
            .filter(|slice| slice.shape.is_block_compressed())
            .map(|slice| slice.maps)
            .sum()
    }

    /// Whole percent of the RGBA8 shape this receipt holds (floored, 100 when
    /// nothing is resident). `(whole, tenths)` as printed by [`Self::line`].
    #[must_use]
    pub fn percent(&self) -> (u64, u64) {
        if self.rgba8_bytes == 0 {
            return (100, 0);
        }
        let per_mille = self.bytes.saturating_mul(1000) / self.rgba8_bytes;
        (per_mille / 10, per_mille % 10)
    }

    /// One receipt line: every shape held, its bytes, and the RGBA8 baseline.
    #[must_use]
    pub fn line(&self) -> String {
        let mut out = format!(
            "textures: {} maps, {} block, {} B held vs {} B as RGBA8 = {}.{}%",
            self.maps,
            self.block_maps(),
            self.bytes,
            self.rgba8_bytes,
            self.percent().0,
            self.percent().1
        );
        for slice in &self.slices {
            out.push_str(&format!(
                "; {} {} maps {} B",
                slice.shape.label(),
                slice.maps,
                slice.bytes
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::present::cache::TextureUpload;

    /// A block upload whose levels are all-`fill` bytes.
    fn fixture(width: u32, height: u32, format: BlockFormat, fill: u8) -> BlockUpload {
        let mut levels = Vec::new();
        let mut level = 0u32;
        loop {
            let w = (width >> level).max(1);
            let h = (height >> level).max(1);
            let len = BlockLevel::byte_len(format, w, h).expect("fits");
            levels.push(BlockLevel {
                width: w,
                height: h,
                bytes: vec![fill; len],
            });
            if w == 1 && h == 1 {
                break;
            }
            level += 1;
        }
        BlockUpload {
            width,
            height,
            format,
            levels,
        }
    }

    #[test]
    fn block_upload_counts_blocks_and_the_rgba8_twin() {
        let upload = fixture(8, 4, BlockFormat::Bc7Srgb, 0x40);
        assert!(upload.validate().is_ok());
        // 8x4 = 2 blocks a row, 1 down: 32 bytes at the base, 16 at 4x2, 16 at
        // 2x1 and 16 at 1x1 (a block each below 4 texels).
        assert_eq!(upload.levels.len(), 4, "chain runs to 1x1");
        assert_eq!(upload.block_bytes(), 80);
        assert_eq!(upload.rgba8_bytes(), 8 * 4 * 4 + 4 * 2 * 4 + 2 * 4 + 4);
        assert_eq!(upload.base_rgba8_bytes(), 128);
        assert_eq!(BlockFormat::Bc7Srgb.block_bytes(), 16);
        assert_eq!(BlockFormat::Bc7Linear.label(), "bc7-rgba-unorm");
    }

    #[test]
    fn block_upload_refuses_a_broken_chain_or_a_short_level() {
        let mut upload = fixture(8, 8, BlockFormat::Bc7Linear, 0);
        upload.levels[1].width = 8;
        assert!(matches!(
            upload.validate(),
            Err(PresentError::InvalidTexture(_))
        ));
        let mut upload = fixture(8, 8, BlockFormat::Bc7Linear, 0);
        upload.levels[0].bytes.pop();
        assert!(matches!(
            upload.validate(),
            Err(PresentError::InvalidTexture(_))
        ));
        let mut upload = fixture(4, 4, BlockFormat::Bc7Linear, 0);
        upload.levels.clear();
        assert!(matches!(
            upload.validate(),
            Err(PresentError::InvalidTexture(_))
        ));
        let zero = BlockUpload {
            width: 0,
            height: 4,
            format: BlockFormat::Bc7Linear,
            levels: Vec::new(),
        };
        assert!(matches!(
            zero.validate(),
            Err(PresentError::InvalidTexture(_))
        ));
    }

    #[test]
    fn block_upload_caps_to_a_smaller_mip() {
        let upload = fixture(16, 8, BlockFormat::Bc7Srgb, 1);
        let capped = upload.capped_to(4).expect("4x2 mip fits");
        assert_eq!((capped.width, capped.height), (4, 2));
        assert!(capped.validate().is_ok());
        assert_eq!(capped.block_bytes(), 32 + 16);
        assert!(
            upload.capped_to(16).is_none(),
            "a fitting upload is not capped"
        );
        assert!(upload.capped_to(0).is_none(), "zero cap keeps the upload");
    }

    #[test]
    fn texture_source_reads_either_shape() {
        let rgba = TextureUpload {
            width: 4,
            height: 2,
            rgba: vec![7u8; 4 * 2 * 4],
        };
        let source = TextureSource::from(rgba);
        assert_eq!((source.width(), source.height()), (4, 2));
        assert_eq!(source.rgba8_base_bytes(), 32);
        assert!(source.validate().is_ok());
        let block = TextureSource::Block(fixture(4, 2, BlockFormat::Bc7Linear, 3));
        assert_eq!((block.width(), block.height()), (4, 2));
        assert_eq!(block.rgba8_base_bytes(), 32);
        assert!(block.validate().is_ok());
    }

    #[test]
    fn receipt_prints_every_shape_and_the_rgba8_baseline() {
        let mut receipt = TextureReceipt::default();
        receipt.add(TextureShape::Bc7Srgb, 1024, 4096);
        receipt.add(TextureShape::Rgba8Linear, 4096, 4096);
        receipt.add(TextureShape::Bc7Srgb, 1024, 4096);
        assert_eq!(receipt.maps, 3);
        assert_eq!(receipt.block_maps(), 2);
        assert_eq!(receipt.bytes, 6144);
        assert_eq!(receipt.rgba8_bytes, 12288);
        assert_eq!(receipt.percent(), (50, 0));
        let line = receipt.line();
        assert!(
            line.contains("bc7-rgba-unorm-srgb 2 maps 2048 B"),
            "receipt names each shape: {line}"
        );
        assert!(line.contains("= 50.0%"), "receipt prints the drop: {line}");
        assert_eq!(TextureShape::Other.label(), "other");
        assert!(!TextureShape::Other.is_block_compressed());
    }
}
