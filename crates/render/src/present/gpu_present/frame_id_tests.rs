//! frame_id_tests: id-target readback for frame tests (window build).
//!
//! Owner: render lane, consumer `frame::id_tests` only. Test-only helper:
//! copies the id texture to a mapped buffer and decodes little-endian ids.

/// Copy the id texture back to `u32` ids row-major (test-only).
pub(crate) fn read_id_back(
    presenter: &crate::present::Presenter,
    texture: &wgpu::Texture,
) -> Vec<u32> {
    let (w, h) = (texture.width(), texture.height());
    let row = w * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-id-readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = presenter
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    presenter.queue.submit(Some(enc.finish()));
    let mut waits = crate::present::wait_stats::FrameWaits::default();
    let bytes =
        super::super::submit::map_readback(&presenter.device, &mut waits, &buffer, row, padded, h)
            .expect("id readback maps");
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
