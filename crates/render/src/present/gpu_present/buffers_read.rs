//! buffers_read: the GPU half of the A3 debug frame. It lives under
//! `present` because it names `wgpu` types and copies real textures; the pure
//! frame, its builder and its helpers stay in `crate::buffers`, which never
//! mentions a device. Same contract as the CPU path: rows are stripped of the
//! 256-byte copy padding and the decoded values go back through
//! `DebugFrame::from_gbuffer`, so a readback and a fixture prove one frame.
//!
//! Determinism: presentation only, never the tick hash. Consumers: the A3
//! debug view and its device test.

use crate::buffers::{decode_normal, BufferError, DebugFrame};

/// Read one GPU frame into a [`DebugFrame`] (gap A3, the GPU half). Copies
/// each present texture into a mapped buffer and feeds the same pure builder
/// the CPU path uses, so a readback and a fixture prove the same frame.
///
/// Formats: depth must already be `R32Float` (a `Depth32Float` target cannot
/// be copied to a buffer; render it into an `R32Float` copy first), normal and
/// albedo are `Rgba8Unorm`, ids are `R16Uint`. Depth values are read as
/// normalized `0..=1`, so the builder quantizes them exactly as the CPU path
/// does.
///
/// # Errors
///
/// [`BufferError::Empty`] on a zero side, [`BufferError::MissingChannel`] when
/// a channel texture is absent (the message names the pass that must exist
/// first) or is a depth format that cannot be copied,
/// [`BufferError::Format`] for any other wrong format.
pub fn read_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    depth_texture: &wgpu::Texture,
    normal: Option<&wgpu::Texture>,
    albedo: Option<&wgpu::Texture>,
    ids: Option<&wgpu::Texture>,
    size: [u32; 2],
) -> Result<DebugFrame, BufferError> {
    let [width, height] = size;
    if width == 0 || height == 0 {
        return Err(BufferError::Empty { width, height });
    }
    expect_format(depth_texture, "depth", wgpu::TextureFormat::R32Float)?;
    let depth_bytes = read_texels(device, queue, depth_texture, width, height, 4, "depth")?;
    read_frame_channels(
        device,
        queue,
        &depth_floats(&depth_bytes),
        normal,
        albedo,
        ids,
        size,
    )
}

/// The copied `R32Float` texels as one `f32` per texel, little-endian like the
/// device wrote them.
fn depth_floats(bytes: &[u8]) -> Vec<f32> {
    let mut depth = Vec::with_capacity(bytes.len() / 4);
    for quad in bytes.chunks_exact(4) {
        depth.push(f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]));
    }
    depth
}

/// The same decode with the depth channel already in hand: the shape a device
/// capture needs, whose depth comes from a `Depth32Float` target copied to a
/// buffer (a depth texture cannot be copied into `read_frame`'s `R32Float`).
///
/// Owns the three colour channels' gates and texel reads and feeds the same
/// [`decode_channels`] the `R32Float` path uses, so both paths answer one
/// frame builder.
///
/// # Errors
///
/// [`BufferError::Empty`] on a zero side, [`BufferError::MissingChannel`] when
/// a channel texture is absent (the message names the pass that must exist
/// first) or is a depth format that cannot be copied, [`BufferError::Format`]
/// for any other wrong format.
pub fn read_frame_channels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    depth: &[f32],
    normal: Option<&wgpu::Texture>,
    albedo: Option<&wgpu::Texture>,
    ids: Option<&wgpu::Texture>,
    size: [u32; 2],
) -> Result<DebugFrame, BufferError> {
    let [width, height] = size;
    if width == 0 || height == 0 {
        return Err(BufferError::Empty { width, height });
    }
    let normal_texture = normal.ok_or(BufferError::MissingChannel {
        channel: "normal",
        why: "the forward pass keeps no normal target; add one and read it back",
    })?;
    let albedo_texture = albedo.ok_or(BufferError::MissingChannel {
        channel: "albedo",
        why: "the forward pass keeps no albedo target; add one and read it back",
    })?;
    let ids_texture = ids.ok_or(BufferError::MissingChannel {
        channel: "material_id",
        why: "the id prepass target does not exist; write ids there and read it back",
    })?;
    expect_format(normal_texture, "normal", wgpu::TextureFormat::Rgba8Unorm)?;
    expect_format(albedo_texture, "albedo", wgpu::TextureFormat::Rgba8Unorm)?;
    expect_format(ids_texture, "material_id", wgpu::TextureFormat::R16Uint)?;

    let normal_bytes = read_texels(device, queue, normal_texture, width, height, 4, "normal")?;
    let albedo_bytes = read_texels(device, queue, albedo_texture, width, height, 4, "albedo")?;
    let id_bytes = read_texels(device, queue, ids_texture, width, height, 2, "material_id")?;
    decode_channels(
        &normal_bytes,
        &albedo_bytes,
        &id_bytes,
        depth,
        width,
        height,
    )
}

/// Decode the channel readbacks into the frame. Not on the tick path: one
/// allocation per channel per readback, which is what a debug frame costs.
fn decode_channels(
    normal_bytes: &[u8],
    albedo_bytes: &[u8],
    id_bytes: &[u8],
    depth: &[f32],
    width: u32,
    height: u32,
) -> Result<DebugFrame, BufferError> {
    let mut normal_dirs = Vec::with_capacity(normal_bytes.len() / 4);
    for (pixel, _) in normal_bytes.chunks_exact(4).enumerate() {
        normal_dirs.push(decode_normal(normal_bytes, pixel));
    }
    let mut albedo_dirs = Vec::with_capacity(albedo_bytes.len() / 4);
    for quad in albedo_bytes.chunks_exact(4) {
        albedo_dirs.push([
            f32::from(quad[0]) / 255.0,
            f32::from(quad[1]) / 255.0,
            f32::from(quad[2]) / 255.0,
        ]);
    }
    let mut materials = Vec::with_capacity(id_bytes.len() / 2);
    for pair in id_bytes.chunks_exact(2) {
        materials.push(u16::from_le_bytes([pair[0], pair[1]]));
    }
    DebugFrame::from_gbuffer(&normal_dirs, depth, &albedo_dirs, &materials, width, height)
}
/// One texture's format gate.
fn expect_format(
    texture: &wgpu::Texture,
    channel: &'static str,
    want: wgpu::TextureFormat,
) -> Result<(), BufferError> {
    if texture.format() == want {
        return Ok(());
    }
    if texture.format() == wgpu::TextureFormat::Depth32Float {
        return Err(BufferError::MissingChannel {
            channel,
            why: "a depth texture cannot be copied to a buffer; render it into an R32Float copy first",
        });
    }
    let want_name = match want {
        wgpu::TextureFormat::Rgba8Unorm => "Rgba8Unorm",
        wgpu::TextureFormat::R32Float => "R32Float",
        wgpu::TextureFormat::R16Uint => "R16Uint",
        _ => "the channel format",
    };
    Err(BufferError::Format {
        channel,
        want: want_name,
    })
}

/// Copy one texture into a mapped buffer and strip the row padding.
fn read_texels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    bytes_per_pixel: usize,
    channel: &'static str,
) -> Result<Vec<u8>, BufferError> {
    let row_bytes = usize::try_from(width)
        .unwrap_or(0)
        .saturating_mul(bytes_per_pixel);
    let padded = (row_bytes + 255) & !255;
    let rows = usize::try_from(height).unwrap_or(0);
    let size = u64::try_from(padded.saturating_mul(rows)).unwrap_or(0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("debug-frame-readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("debug-frame-copy"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(u32::try_from(padded).unwrap_or(0)),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let mapped = slice
        .get_mapped_range()
        .map_err(|_| BufferError::Readback { channel })?;
    let mut out = Vec::with_capacity(row_bytes.saturating_mul(rows));
    for row in 0..rows {
        let start = row.saturating_mul(padded);
        let end = start.saturating_add(row_bytes);
        if let Some(line) = mapped.get(start..end) {
            out.extend_from_slice(line);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffers::{lookup_id, quantize_u16};
    /// A 4x4 texture of one format, copyable, for the readback probe.
    fn textured(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("debug-frame-fixture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// The readback turns known GPU texels into the same frame the CPU path
    /// builds, and an absent channel is a typed error. Needs a device; skips
    /// loudly with no adapter.
    #[test]
    fn buffers_read_frame_matches_known_values() {
        let Some((device, queue, _info)) = crate::test_gpu::require_or_skip(
            "buffers_read_frame_matches_known_values",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let (width, height) = (4u32, 4u32);
        let depth_tex = textured(device, width, height, wgpu::TextureFormat::R32Float);
        let normal_tex = textured(device, width, height, wgpu::TextureFormat::Rgba8Unorm);
        let albedo_tex = textured(device, width, height, wgpu::TextureFormat::Rgba8Unorm);
        let id_tex = textured(device, width, height, wgpu::TextureFormat::R16Uint);

        let mut depth = Vec::new();
        for pixel in 0..(width * height) {
            depth.extend_from_slice(&(pixel as f32 / 15.0).to_le_bytes());
        }
        let mut normal = Vec::new();
        let mut albedo = Vec::new();
        let mut ids = Vec::new();
        for pixel in 0..(width * height) {
            normal.extend_from_slice(&[128, 128, 255, 255]);
            albedo.extend_from_slice(&[255, 128, 0, 255]);
            ids.extend_from_slice(&u16::try_from(pixel).unwrap_or(0).to_le_bytes());
        }
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let write = |texture: &wgpu::Texture, bytes: &[u8], row: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
                extent,
            );
        };
        write(&depth_tex, &depth, width * 4);
        write(&normal_tex, &normal, width * 4);
        write(&albedo_tex, &albedo, width * 4);
        write(&id_tex, &ids, width * 2);

        let frame = read_frame(
            device,
            queue,
            &depth_tex,
            Some(&normal_tex),
            Some(&albedo_tex),
            Some(&id_tex),
            [width, height],
        )
        .expect("the frame reads back");
        assert_eq!(frame.width, width, "width reads back");
        assert_eq!(frame.texels(), 16, "4x4 texels");
        assert_eq!(lookup_id(&frame, 2, 1), 6, "the id at (2, 1)");
        assert_eq!(lookup_id(&frame, 3, 3), 15, "the last id");
        let probe = usize::try_from(5).expect("pixel") * 4;
        assert_eq!(
            &frame.albedo[probe..probe + 4],
            &[255, 128, 0, 255],
            "albedo bytes read back"
        );
        assert_eq!(
            frame.depth[5],
            quantize_u16(5.0 / 15.0),
            "depth quantizes like the CPU path"
        );
        let decoded = decode_normal(&frame.normal, 0);
        assert!(
            (decoded[2] - 1.0).abs() <= 1.0 / 255.0,
            "a (128, 128) normal decodes to +Z, read {decoded:?}"
        );

        match read_frame(
            device,
            queue,
            &depth_tex,
            None,
            Some(&albedo_tex),
            Some(&id_tex),
            [width, height],
        ) {
            Err(error @ BufferError::MissingChannel { channel, .. }) => {
                assert_eq!(channel, "normal", "the message names the missing channel");
                assert!(
                    error.to_string().contains("forward pass"),
                    "the message names the pass that must exist: {error}"
                );
            }
            other => panic!("an absent normal gives {other:?}, not a typed miss"),
        }
        eprintln!("buffers_read_frame_matches_known_values: 4x4 readback matches the fixture");
    }
}
