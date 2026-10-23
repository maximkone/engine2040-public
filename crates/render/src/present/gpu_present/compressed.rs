//! compressed: block-compressed (BC7) texture uploads for the presenter.
//!
//! Owner: the render texture upload path. Consumers: `upload_texture_source`
//! and `upload_material_set_source`, which the game arena kit loader calls with
//! the arena set's cooked KTX2 blocks (VR2). Units: pixels for sizes, bytes for
//! the receipt. Determinism: the block bytes reach the GPU unchanged, so the
//! same upload allocates the same texture.
//!
//! Gate: [`wgpu::Features::TEXTURE_COMPRESSION_BC`]. An adapter without it (a
//! browser page without the BC extension, say) is not an error: the block
//! upload decodes to RGBA8 and takes the original pixel path, so a caller hands
//! the same shape everywhere and [`Presenter::texture_receipt`] reports what the
//! GPU actually holds.
//!
//! BC5 is not uploaded here: the forward shader normalizes the texel it samples
//! (`eg12_decode_normal`), so a normal map has to carry Z, and BC5 holds two
//! channels. A material set therefore mixes shapes: its normal map stays RGBA8
//! linear while its BC7 ORM map arrives as blocks.

use super::frame::GpuTexture;
use super::presenter::{MapSet, Presenter};
use crate::present::cache::{TextureKey, TextureUpload};
use crate::present::surface::PresentError;
use crate::present::texture_source::{
    BlockFormat, BlockUpload, TextureReceipt, TextureShape, TextureSource,
};

/// The GPU format a block shape allocates.
#[must_use]
pub fn texture_format(format: BlockFormat) -> wgpu::TextureFormat {
    match format {
        BlockFormat::Bc7Srgb => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
        BlockFormat::Bc7Linear => wgpu::TextureFormat::Bc7RgbaUnorm,
    }
}

/// The shape a resident texture format holds, for the receipt.
#[must_use]
pub fn texture_shape_of(format: wgpu::TextureFormat) -> TextureShape {
    match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => TextureShape::Rgba8Srgb,
        wgpu::TextureFormat::Rgba8Unorm => TextureShape::Rgba8Linear,
        wgpu::TextureFormat::Bc7RgbaUnormSrgb => TextureShape::Bc7Srgb,
        wgpu::TextureFormat::Bc7RgbaUnorm => TextureShape::Bc7Linear,
        _ => TextureShape::Other,
    }
}

/// One level's copy layout in `format`: bytes per row, rows, and the physical
/// extent the copy covers. A compressed level smaller than a block still copies
/// a whole block (a 2x2 mip of a BC format is one 4x4 block on the GPU).
fn level_copy(format: wgpu::TextureFormat, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let (block_w, block_h) = format.block_dimensions();
    let block_w = block_w.max(1);
    let block_h = block_h.max(1);
    let block_bytes = format.block_copy_size(None).unwrap_or(0);
    let columns = width.div_ceil(block_w);
    let rows = height.div_ceil(block_h);
    (
        columns.saturating_mul(block_bytes),
        rows,
        columns.saturating_mul(block_w),
        rows.saturating_mul(block_h),
    )
}

/// Byte length of one level of a resident texture, and of the same level as
/// RGBA8. Blocks count from the format, so the numbers are the GPU's own.
fn level_bytes(format: wgpu::TextureFormat, width: u32, height: u32) -> (u64, u64) {
    let (row_bytes, rows, _, _) = level_copy(format, width, height);
    let held = u64::from(row_bytes) * u64::from(rows);
    let rgba8 = u64::from(width) * u64::from(height) * 4;
    (held, rgba8)
}

/// Record one resident texture in a receipt: every mip level, as allocated.
fn record(receipt: &mut TextureReceipt, texture: &wgpu::Texture) {
    let format = texture.format();
    let (mut held, mut rgba8) = (0u64, 0u64);
    for level in 0..texture.mip_level_count() {
        let width = (texture.width() >> level).max(1);
        let height = (texture.height() >> level).max(1);
        let (bytes, pixels) = level_bytes(format, width, height);
        held = held.saturating_add(bytes);
        rgba8 = rgba8.saturating_add(pixels);
    }
    receipt.add(texture_shape_of(format), held, rgba8);
}

/// Decode a block upload's base level (after the tier cap) to a validated RGBA8
/// upload: the path an adapter without BC support takes.
fn decode_block_upload(upload: &BlockUpload, max_edge: u32) -> Result<TextureUpload, PresentError> {
    upload.validate()?;
    let capped = upload.capped_to(max_edge);
    let upload = capped.as_ref().unwrap_or(upload);
    let base = upload
        .levels
        .first()
        .ok_or_else(|| PresentError::InvalidTexture("block upload carries no level".to_string()))?;
    let rgba = engine_assets::ktx2_cook::decode_blocks_rgba(
        engine_assets::bc::BcFormat::Bc7,
        base.width,
        base.height,
        &base.bytes,
    )
    .map_err(|err| PresentError::InvalidTexture(format!("block decode: {err}")))?;
    let pixels = TextureUpload {
        width: base.width,
        height: base.height,
        rgba,
    };
    pixels.validate()?;
    Ok(pixels)
}

impl Presenter {
    /// True when this device decodes BC block textures, the gate every block
    /// upload reads before it allocates one.
    #[must_use]
    pub fn block_textures_supported(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }

    /// Upload one texture in either shape under `key`. A block upload whose
    /// adapter has no BC support decodes to RGBA8 and takes the pixel path.
    pub fn upload_texture_source(
        &mut self,
        key: TextureKey,
        source: &TextureSource,
    ) -> Result<(), PresentError> {
        match source {
            TextureSource::Rgba8(upload) => self.upload_texture(key, upload),
            TextureSource::Block(upload) if self.block_textures_supported() => {
                let gpu = self.pack_block_texture(upload, "ra1-texture")?;
                let bind = self.forward_bind(&gpu.view);
                self.creates.bind_group();
                self.insert_texture(key, gpu, bind);
                Ok(())
            }
            TextureSource::Block(upload) => {
                let pixels = decode_block_upload(upload, u32::from(self.quality.texture_max_px))?;
                self.upload_texture(key, &pixels)
            }
        }
    }

    /// Register a material set (VR2) from either shape: a normal map plus an
    /// optional ORM map over the resident base-color `base`. The block path
    /// runs when the adapter decodes BC, a base is resident and an ORM map is
    /// named, and each map keeps its own shape: a BC7 ORM uploads as blocks
    /// over an RGBA8 normal map. Any other case takes `upload_material_set`,
    /// whose white base and ORM fallbacks the pixel path binds.
    pub fn upload_material_set_source(
        &mut self,
        key: TextureKey,
        base: Option<TextureKey>,
        normal: &TextureSource,
        orm: Option<&TextureSource>,
    ) -> Result<(), PresentError> {
        if self.block_textures_supported() {
            if let (Some(base_key), Some(orm)) = (base, orm) {
                let any_block = matches!(normal, TextureSource::Block(_))
                    || matches!(orm, TextureSource::Block(_));
                if any_block {
                    return self.upload_block_material_set(key, base_key, normal, orm);
                }
            }
        }
        let normal_pixels = self.source_pixels(normal)?;
        let orm_pixels = match orm {
            Some(source) => Some(self.source_pixels(source)?),
            None => None,
        };
        self.upload_material_set(key, base, &normal_pixels, orm_pixels.as_ref())
    }

    /// Twin of `upload_material_set` that keeps a block map's own blocks: each
    /// map allocates one texture in its own shape and the set binds them once.
    fn upload_block_material_set(
        &mut self,
        key: TextureKey,
        base: TextureKey,
        normal: &TextureSource,
        orm: &TextureSource,
    ) -> Result<(), PresentError> {
        let normal_gpu = self.pack_source_texture(normal, "ra1-set-normal")?;
        let orm_gpu = self.pack_source_texture(orm, "ra1-set-orm")?;
        let base_view = self.require_texture_view(&base)?.0;
        let bind = self.forward_bind_maps(
            base_view,
            &normal_gpu.view,
            &orm_gpu.view,
            &self.repeat_sampler,
        );
        self.creates.bind_group();
        self.map_sets.retain(|set| set.key != key);
        self.map_sets.push(MapSet {
            key,
            _textures: [normal_gpu, orm_gpu],
            bind,
        });
        Ok(())
    }

    /// Either shape as RGBA8 pixels (a block upload decodes its base level).
    fn source_pixels(&self, source: &TextureSource) -> Result<TextureUpload, PresentError> {
        match source {
            TextureSource::Rgba8(upload) => {
                upload.validate()?;
                Ok(upload.clone())
            }
            TextureSource::Block(upload) => {
                decode_block_upload(upload, u32::from(self.quality.texture_max_px))
            }
        }
    }

    /// One map of a material set in its own shape. A block upload allocates
    /// BC7 with the whole cooked chain; pixels take the linear RGBA8 pack the
    /// pixel path uses for set maps (normals and ORM are data, never sRGB).
    fn pack_source_texture(
        &self,
        source: &TextureSource,
        label: &'static str,
    ) -> Result<GpuTexture, PresentError> {
        match source {
            TextureSource::Block(upload) => self.pack_block_texture(upload, label),
            TextureSource::Rgba8(upload) => self.pack_linear_rgba8(upload, label),
        }
    }

    /// Allocate one BC7 texture with the upload's whole mip chain, every level
    /// written as blocks.
    fn pack_block_texture(
        &self,
        upload: &BlockUpload,
        label: &'static str,
    ) -> Result<GpuTexture, PresentError> {
        upload.validate()?;
        let capped = upload.capped_to(u32::from(self.quality.texture_max_px));
        let upload = capped.as_ref().unwrap_or(upload);
        let levels: Vec<(u32, u32, &[u8])> = upload
            .levels
            .iter()
            .map(|level| (level.width, level.height, level.bytes.as_slice()))
            .collect();
        Ok(self.pack_texture_levels(
            label,
            texture_format(upload.format),
            upload.width,
            upload.height,
            &levels,
        ))
    }

    /// Allocate one linear RGBA8 texture with the upload's box-filtered mip
    /// chain: the pixel shape a set map takes (no sRGB decode on read).
    fn pack_linear_rgba8(
        &self,
        upload: &TextureUpload,
        label: &'static str,
    ) -> Result<GpuTexture, PresentError> {
        upload.validate()?;
        let capped = upload.capped_to(u32::from(self.quality.texture_max_px));
        let upload = capped.as_ref().unwrap_or(upload);
        let mips = upload.mip_chain();
        let mut levels: Vec<(u32, u32, &[u8])> = Vec::with_capacity(mips.len() + 1);
        levels.push((upload.width, upload.height, upload.rgba.as_slice()));
        levels.extend(
            mips.iter()
                .map(|(width, height, bytes)| (*width, *height, bytes.as_slice())),
        );
        Ok(self.pack_texture_levels(
            label,
            wgpu::TextureFormat::Rgba8Unorm,
            upload.width,
            upload.height,
            &levels,
        ))
    }

    /// Allocate one texture of `format` and write every level in order, base
    /// first. Each level's row stride comes from the format, so blocks and
    /// pixels share one path.
    fn pack_texture_levels(
        &self,
        label: &'static str,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        levels: &[(u32, u32, &[u8])],
    ) -> GpuTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: u32::try_from(levels.len()).unwrap_or(1).max(1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: texture_usage(),
            view_formats: &[],
        });
        for (level, (level_width, level_height, bytes)) in levels.iter().enumerate() {
            let (bytes_per_row, rows_per_image, copy_width, copy_height) =
                level_copy(format, *level_width, *level_height);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: u32::try_from(level).unwrap_or(0),
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // Tightly packed rows: the queue stages its own copy, so no
                    // 256-byte row padding is needed here.
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(rows_per_image),
                },
                wgpu::Extent3d {
                    width: copy_width,
                    height: copy_height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        GpuTexture {
            texture,
            view,
            width,
            height,
        }
    }

    /// Cache an already-packed texture under `key`, dropping an older bind
    /// group for it. `upload_texture` packs pixels first; this takes the packed
    /// pair a block upload built.
    fn insert_texture(&mut self, key: TextureKey, gpu: GpuTexture, bind: wgpu::BindGroup) {
        self.texture_binds.retain(|(known, _)| *known != key);
        self.texture_binds.push((key, bind));
        if let Some(evicted) = self.textures.insert(key, gpu) {
            debug_assert_ne!(evicted, key, "reinsert never evicts itself");
            self.texture_binds.retain(|(known, _)| *known != evicted);
        }
    }

    /// The shape one resident base-color texture actually holds; `None` when it
    /// is not resident.
    #[must_use]
    pub fn texture_shape(&self, key: &TextureKey) -> Option<TextureShape> {
        self.textures
            .get(key)
            .map(|gpu| texture_shape_of(gpu.texture.format()))
    }

    /// The shapes a resident material set's normal and ORM maps actually hold,
    /// in that order; `None` when the set is not resident.
    #[must_use]
    pub fn material_set_shapes(&self, key: &TextureKey) -> Option<[TextureShape; 2]> {
        self.map_sets.iter().find(|set| set.key == *key).map(|set| {
            let [normal, orm] = &set._textures;
            [
                texture_shape_of(normal.texture.format()),
                texture_shape_of(orm.texture.format()),
            ]
        })
    }

    /// What the GPU holds for every resident material texture: each map's
    /// allocated shape, its bytes, and the bytes the RGBA8 shape would take.
    /// Read from the textures themselves, never from the upload a caller asked
    /// for, so a gate that fell back shows up here.
    #[must_use]
    pub fn texture_receipt(&self) -> TextureReceipt {
        let mut receipt = TextureReceipt::default();
        for key in self.textures.sorted_keys() {
            if let Some(gpu) = self.textures.get(&key) {
                record(&mut receipt, &gpu.texture);
            }
        }
        for set in &self.map_sets {
            for gpu in &set._textures {
                record(&mut receipt, &gpu.texture);
            }
        }
        receipt
    }
}

/// Usage every material texture allocates: sampled and written at upload, plus
/// `COPY_SRC` in test builds so a test reads the blocks back.
fn texture_usage() -> wgpu::TextureUsages {
    if cfg!(test) {
        wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
    } else {
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::present::texture_source::{BlockLevel, BLOCK_EDGE};

    /// One solid colour per block, so a BC7 mode-6 encode is near exact and the
    /// decode test can check the pixels it reads back.
    fn block_colour(level: u32, bx: u32, by: u32) -> [u8; 4] {
        [
            u8::try_from((bx * 37 + level * 11) % 256).unwrap(),
            u8::try_from((by * 53 + level * 5) % 256).unwrap(),
            u8::try_from(((bx + by) * 29 + level * 7) % 256).unwrap(),
            255,
        ]
    }

    /// A real BC7 mode-6 block upload: the blocks decode, so the fallback test
    /// reads back the pixels the encoder wrote.
    fn fixture(width: u32, height: u32, format: BlockFormat) -> BlockUpload {
        let mut levels = Vec::new();
        let mut level = 0u32;
        loop {
            let w = (width >> level).max(1);
            let h = (height >> level).max(1);
            let mut bytes = Vec::new();
            for by in 0..h.div_ceil(BLOCK_EDGE) {
                for bx in 0..w.div_ceil(BLOCK_EDGE) {
                    let block = [block_colour(level, bx, by); 16];
                    bytes.extend_from_slice(&engine_assets::bc::encode_bc7_block(
                        &block,
                        engine_assets::bc::BcSlot::BaseColor,
                    ));
                }
            }
            levels.push(BlockLevel {
                width: w,
                height: h,
                bytes,
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

    fn pixels(width: u32, height: u32, texel: [u8; 4]) -> TextureUpload {
        TextureUpload {
            width,
            height,
            rgba: texel.repeat((width * height) as usize),
        }
    }

    #[test]
    fn block_format_maps_to_the_formats_the_gpu_allocates() {
        assert_eq!(
            texture_format(BlockFormat::Bc7Srgb),
            wgpu::TextureFormat::Bc7RgbaUnormSrgb
        );
        assert_eq!(
            texture_format(BlockFormat::Bc7Linear),
            wgpu::TextureFormat::Bc7RgbaUnorm
        );
        for (format, shape) in [
            (wgpu::TextureFormat::Bc7RgbaUnormSrgb, TextureShape::Bc7Srgb),
            (wgpu::TextureFormat::Bc7RgbaUnorm, TextureShape::Bc7Linear),
            (wgpu::TextureFormat::Rgba8UnormSrgb, TextureShape::Rgba8Srgb),
            (wgpu::TextureFormat::Rgba8Unorm, TextureShape::Rgba8Linear),
            (wgpu::TextureFormat::R32Float, TextureShape::Other),
        ] {
            assert_eq!(texture_shape_of(format), shape, "{format:?}");
        }
        // 8x4 in BC7: two blocks a row, one down.
        assert_eq!(
            level_bytes(wgpu::TextureFormat::Bc7RgbaUnorm, 8, 4),
            (32, 128)
        );
        assert_eq!(
            level_bytes(wgpu::TextureFormat::Rgba8Unorm, 8, 4),
            (128, 128)
        );
        assert_eq!(BLOCK_EDGE, 4);
    }

    #[test]
    fn a_block_upload_decodes_to_its_rgba8_twin() {
        let upload = fixture(8, 4, BlockFormat::Bc7Srgb);
        let decoded = decode_block_upload(&upload, 2048).expect("decodes");
        assert_eq!((decoded.width, decoded.height), (8, 4));
        assert_eq!(decoded.rgba.len(), 8 * 4 * 4);
        assert!(decoded.validate().is_ok());
        for y in 0..4u32 {
            for x in 0..8u32 {
                let want = block_colour(0, x / BLOCK_EDGE, y / BLOCK_EDGE);
                let at = ((y * 8 + x) * 4) as usize;
                for channel in 0..3 {
                    let got = u32::from(decoded.rgba[at + channel]);
                    let want = u32::from(want[channel]);
                    assert!(
                        got.abs_diff(want) <= 3,
                        "texel ({x},{y}) channel {channel}: {got} vs {want}"
                    );
                }
                let alpha = u32::from(decoded.rgba[at + 3]);
                assert!(
                    alpha.abs_diff(255) <= 2,
                    "texel ({x},{y}) alpha {alpha} vs 255"
                );
            }
        }
        // The tier cap takes the largest mip that fits, like the pixel path.
        let capped = decode_block_upload(&upload, 4).expect("capped decodes");
        assert_eq!((capped.width, capped.height), (4, 2));
        let zero = BlockUpload {
            width: 4,
            height: 4,
            format: BlockFormat::Bc7Linear,
            levels: Vec::new(),
        };
        assert!(decode_block_upload(&zero, 2048).is_err());
    }

    #[cfg(feature = "window")]
    fn init_or_skip(test: &str) -> Option<Presenter> {
        use crate::test_gpu::{require_or_skip, NoAdapter};
        let probe = match Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        require_or_skip(test, probe)
    }

    /// VR2 Accept: a BC7 upload reaches the GPU as blocks with its whole mip
    /// chain, the base level reads back byte for byte, and the receipt reports
    /// the allocated format and the bytes against the RGBA8 twin.
    #[cfg(feature = "window")]
    #[test]
    fn block_upload_holds_bc7_blocks_and_reads_them_back() {
        let Some(mut presenter) = init_or_skip("vr2_block_upload_readback") else {
            return;
        };
        if !presenter.block_textures_supported() {
            eprintln!("SKIPPED: adapter reports no TEXTURE_COMPRESSION_BC");
            return;
        }
        let upload = fixture(8, 8, BlockFormat::Bc7Srgb);
        let key = TextureKey::new(0xBC07, [7u8; 32]);
        presenter
            .upload_texture_source(key, &TextureSource::Block(upload.clone()))
            .expect("block texture uploads");
        assert_eq!(
            presenter.texture_shape(&key),
            Some(TextureShape::Bc7Srgb),
            "the GPU holds colour blocks"
        );
        let handle = presenter.texture_handle(&key).expect("resident");
        assert_eq!(handle.format(), wgpu::TextureFormat::Bc7RgbaUnormSrgb);
        assert_eq!(handle.mip_level_count(), 4, "8x8 down to 1x1");
        // Read level 0 back: the block bytes must be the cooked ones, not a
        // decode and re-encode.
        let base = &upload.levels[0];
        let row = base.blocks_wide() * u32::try_from(BlockFormat::Bc7Srgb.block_bytes()).unwrap();
        let padded = row.div_ceil(256) * 256;
        let readback = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vr2-block-readback"),
            size: u64::from(padded) * u64::from(base.blocks_high()),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vr2-block-copy"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: handle,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(base.blocks_high()),
                },
            },
            wgpu::Extent3d {
                width: base.width,
                height: base.height,
                depth_or_array_layers: 1,
            },
        );
        presenter.queue.submit(Some(enc.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll succeeds");
        rx.recv().expect("map channel").expect("map succeeds");
        let mapped = slice.get_mapped_range().expect("mapped range");
        let mut read = Vec::with_capacity(base.bytes.len());
        for row_index in 0..base.blocks_high() {
            let at = (row_index * padded) as usize;
            read.extend_from_slice(&mapped[at..at + row as usize]);
        }
        assert_eq!(read, base.bytes, "level 0 blocks read back unchanged");
        drop(mapped);
        readback.unmap();

        let receipt = presenter.texture_receipt();
        eprintln!("VR2 block upload: {}", receipt.line());
        assert_eq!(receipt.maps, 1);
        assert_eq!(receipt.block_maps(), 1);
        assert_eq!(receipt.bytes, upload.block_bytes() as u64);
        assert_eq!(receipt.rgba8_bytes, upload.rgba8_bytes() as u64);
    }

    /// The shipping arena shape: a BC7 linear ORM map over an RGBA8 normal map
    /// (the shader needs Z, which BC5 lacks) allocates each map in its own
    /// shape, and the set's two views report exactly that.
    #[cfg(feature = "window")]
    #[test]
    fn block_material_set_keeps_the_orm_blocks() {
        let Some(mut presenter) = init_or_skip("vr2_block_material_set") else {
            return;
        };
        let base_key = TextureKey::new(0xBC08, [8u8; 32]);
        presenter
            .upload_texture_source(
                base_key,
                &TextureSource::Rgba8(pixels(8, 8, [200, 120, 60, 255])),
            )
            .expect("base uploads");
        let set_key = TextureKey::new(0xBC09, [9u8; 32]);
        let normal = TextureSource::Rgba8(pixels(4, 4, [128, 128, 255, 255]));
        let orm = TextureSource::Block(fixture(4, 4, BlockFormat::Bc7Linear));
        presenter
            .upload_material_set_source(set_key, Some(base_key), &normal, Some(&orm))
            .expect("set registers");
        let supported = presenter.block_textures_supported();
        let expected = if supported {
            [TextureShape::Rgba8Linear, TextureShape::Bc7Linear]
        } else {
            [TextureShape::Rgba8Linear, TextureShape::Rgba8Linear]
        };
        assert_eq!(presenter.material_set_shapes(&set_key), Some(expected));
        let receipt = presenter.texture_receipt();
        eprintln!("VR2 material set: {}", receipt.line());
        assert_eq!(receipt.maps, 3, "base + normal + orm");
        assert_eq!(receipt.block_maps(), usize::from(supported), "ORM only");
    }
}
