//! Tests of the live upscale pass: the uniform bytes and the shader text on any machine, the
//! device passes (parity with the CPU port, the sharpen, the sRGB target) where an adapter exists.

use super::*;
use crate::post::{
    fsr1_easu, fsr_easu_con, fsr_edge_lowres, fsr_edge_width_10_90, fsr_mtf50_lift,
    fsr_rcas_sharpen, taa_upsample, FSR_MTF50_LIFT_MIN,
};

/// A deterministic RGBA8 image: ramps, a diagonal edge, a checker patch and grain.
fn fixture_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut state = 0x2040_u32;
    let mut grain = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        u8::try_from(state >> 24).unwrap_or(0)
    };
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let u =
                f32::from(u16::try_from(x).unwrap_or(0)) / f32::from(u16::try_from(w).unwrap_or(1));
            let v =
                f32::from(u16::try_from(y).unwrap_or(0)) / f32::from(u16::try_from(h).unwrap_or(1));
            let diag = if u + v * 0.7 > 0.9 { 0.35 } else { 0.0 };
            let check = if x < w / 4 && y < h / 4 && (x / 2 + y / 2) % 2 == 0 {
                0.4
            } else {
                0.0
            };
            let noise = f32::from(grain()) / 255.0 * 0.04;
            let px = [
                0.15 + 0.5 * u + diag + noise,
                0.2 + 0.4 * v + check,
                0.1 + 0.3 * (1.0 - u) + diag * 0.5,
            ];
            for c in px {
                out.push(unit_to_byte(c));
            }
            out.push(255);
        }
    }
    out
}

fn unit_to_byte(c: f32) -> u8 {
    u8::try_from(round_to_u32(c.clamp(0.0, 1.0) * 255.0)).unwrap_or(255)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test rounding of a value in 0..=255"
)]
fn round_to_u32(v: f32) -> u32 {
    (v + 0.5).floor() as u32
}

fn bytes_to_rgb(bytes: &[u8]) -> Vec<[f32; 3]> {
    bytes
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]].map(|b| f32::from(b) / 255.0))
        .collect()
}

#[test]
fn uniform_bytes_are_the_cpu_easu_constants() {
    for (src, dst) in [
        ((160, 90), (240, 135)),
        ((1280, 720), (1920, 1080)),
        ((99, 64), (150, 96)),
    ] {
        let bytes = uniform_bytes_for_tests(src, dst, 0.4);
        let word = |i: usize| {
            let mut w = [0u8; 4];
            w.copy_from_slice(&bytes[i * 4..i * 4 + 4]);
            f32::from_le_bytes(w)
        };
        let con = fsr_easu_con(src.0, src.1, dst.0, dst.1).expect("sizes in range");
        assert_eq!(
            [word(0), word(1)],
            con.scale,
            "scale for {src:?} -> {dst:?}"
        );
        assert_eq!(
            [word(2), word(3)],
            con.offset,
            "offset for {src:?} -> {dst:?}"
        );
        assert_eq!(word(4), 0.4, "sharpness word");
    }
}

#[test]
fn the_shader_declares_the_vertex_stage_and_all_four_fragment_entries() {
    for needle in [
        "fn vs(",
        "fn fs_easu(",
        "fn fs_easu_hw_srgb(",
        "fn fs_rcas(",
        "fn fs_rcas_hw_srgb(",
    ] {
        assert!(FSR1_UPSCALE_WGSL.contains(needle), "missing {needle}");
    }
}

/// Run the pass chain over `input` into a `format` target of `out_size` and read it back.
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    input: &[u8],
    in_size: (u32, u32),
    out_size: (u32, u32),
    sharpness: f32,
    format: wgpu::TextureFormat,
) -> Vec<u8> {
    let pass = UpscalePass::new(device);
    let sized = pass.sized(device, queue, in_size, out_size, sharpness, false);
    assert!(sized.matches(in_size, out_size, sharpness.clamp(0.0, 1.0), false));
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: sized.lo_texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        input,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(in_size.0 * 4),
            rows_per_image: Some(in_size.1),
        },
        wgpu::Extent3d {
            width: in_size.0,
            height: in_size.1,
            depth_or_array_layers: 1,
        },
    );
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fsr1-test-target"),
        size: wgpu::Extent3d {
            width: out_size.0,
            height: out_size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fsr1-test"),
    });
    sized.encode(
        &pass,
        device,
        &mut enc,
        &view,
        format,
        SpanStamps::default(),
    );
    queue.submit(Some(enc.finish()));
    readback(device, queue, &target, out_size, format)
}

/// Copy a target back as tightly packed texel bytes.
fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    (w, h): (u32, u32),
    format: wgpu::TextureFormat,
) -> Vec<u8> {
    let texel = format
        .block_copy_size(None)
        .expect("a color format has a texel size");
    let tight = w * texel;
    let row = tight.next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fsr1-test-readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fsr1-test-copy"),
    });
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
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let mapped = slice.get_mapped_range().expect("mapped range");
    let mut out = Vec::new();
    for r in 0..h {
        let start = usize::try_from(r * row).expect("row offset");
        out.extend_from_slice(&mapped[start..start + usize::try_from(tight).expect("row bytes")]);
    }
    out
}

/// One f16 bit pattern back to f32.
fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (bits >> 10) & 0x1f;
    let mantissa = f32::from(bits & 0x03ff);
    if exp == 0 {
        return sign * mantissa * 2f32.powi(-24);
    }
    sign * (1.0 + mantissa / 1024.0) * 2f32.powi(i32::from(exp) - 15)
}

fn halves_to_rgb(bytes: &[u8]) -> Vec<[f32; 3]> {
    bytes
        .chunks_exact(8)
        .map(|p| {
            let h = |i: usize| half_to_f32(u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]));
            [h(0), h(1), h(2)]
        })
        .collect()
}

fn gap(a: &[[f32; 3]], b: &[[f32; 3]]) -> (f32, f32, usize) {
    let mut worst = 0.0f32;
    let mut sum = 0.0f32;
    let mut over = 0usize;
    for (p, q) in a.iter().zip(b) {
        for c in 0..3 {
            let d = (p[c] - q[c]).abs();
            worst = worst.max(d);
            sum += d;
            over += usize::from(d > 2.0e-3);
        }
    }
    (
        worst,
        sum / f32::from(u16::try_from(a.len() * 3).unwrap_or(u16::MAX)),
        over,
    )
}

fn device_or_skip(test: &str) -> Option<(&'static wgpu::Device, &'static wgpu::Queue)> {
    let (device, queue, _info) = crate::test_gpu::require_or_skip(test, crate::graph::gx1_probe())?;
    Some((device, queue))
}

#[test]
fn easu_pass_lands_on_the_cpu_port_in_float() {
    let Some((device, queue)) = device_or_skip("easu_pass_lands_on_the_cpu_port_in_float") else {
        return;
    };
    let (src, dst) = ((160, 90), (240, 135));
    let input = fixture_bytes(src.0, src.1);
    let gpu = halves_to_rgb(&run(
        device,
        queue,
        &input,
        src,
        dst,
        0.0,
        wgpu::TextureFormat::Rgba16Float,
    ));
    let cpu = fsr1_easu(&bytes_to_rgb(&input), src.0, src.1, dst.0, dst.1).expect("cpu easu");
    assert_eq!(gpu.len(), cpu.len());
    let (worst, mean, over) = gap(&gpu, &cpu);
    eprintln!(
        "easu parity f16 {src:?} -> {dst:?}: worst {worst:.6} mean {mean:.7} over 2e-3: {over} of {}",
        cpu.len() * 3
    );
    assert!(worst <= 8.0e-3, "worst channel gap {worst}");
    assert!(mean <= 3.0e-4, "mean channel gap {mean}");
    assert!(
        over * 500 <= cpu.len() * 3,
        "{over} channel values past 2e-3"
    );
}

#[test]
fn easu_pass_lands_on_the_cpu_port_in_bytes() {
    let Some((device, queue)) = device_or_skip("easu_pass_lands_on_the_cpu_port_in_bytes") else {
        return;
    };
    let (src, dst) = ((128, 72), (192, 108));
    let input = fixture_bytes(src.0, src.1);
    let gpu = run(
        device,
        queue,
        &input,
        src,
        dst,
        0.0,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let cpu = fsr1_easu(&bytes_to_rgb(&input), src.0, src.1, dst.0, dst.1).expect("cpu easu");
    let mut worst = 0u8;
    let mut off_by_more = 0usize;
    for (px, want) in gpu.chunks_exact(4).zip(&cpu) {
        for c in 0..3 {
            let d = px[c].abs_diff(unit_to_byte(want[c]));
            worst = worst.max(d);
            off_by_more += usize::from(d > 1);
        }
        assert_eq!(px[3], 255, "alpha is opaque");
    }
    eprintln!(
        "easu parity bytes {src:?} -> {dst:?}: worst {worst} LSB, {off_by_more} values over 1 LSB"
    );
    assert!(worst <= 3, "worst byte gap {worst}");
    assert!(
        off_by_more * 200 <= cpu.len() * 3,
        "{off_by_more} byte values over 1 LSB"
    );
}

#[test]
fn flat_fields_come_back_byte_exact() {
    let Some((device, queue)) = device_or_skip("flat_fields_come_back_byte_exact") else {
        return;
    };
    let src = (64, 36);
    let mut input = Vec::new();
    for _ in 0..src.0 * src.1 {
        input.extend_from_slice(&[102, 40, 230, 255]);
    }
    let out = run(
        device,
        queue,
        &input,
        src,
        (96, 54),
        0.0,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    assert!(
        out.chunks_exact(4).all(|p| p == [102, 40, 230, 255]),
        "a flat field must not move"
    );
}

#[test]
fn rcas_stage_follows_the_landed_cpu_sharpen() {
    let Some((device, queue)) = device_or_skip("rcas_stage_follows_the_landed_cpu_sharpen") else {
        return;
    };
    let (src, dst, sharp) = ((96, 54), (144, 81), 0.5);
    let input = fixture_bytes(src.0, src.1);
    let gpu = run(
        device,
        queue,
        &input,
        src,
        dst,
        sharp,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    // The mid target is 8-bit on the device, so the CPU statement quantizes between the passes.
    let easu = fsr1_easu(&bytes_to_rgb(&input), src.0, src.1, dst.0, dst.1).expect("cpu easu");
    let mid: Vec<[f32; 3]> = easu
        .iter()
        .map(|p| p.map(|c| f32::from(unit_to_byte(c)) / 255.0))
        .collect();
    let cpu = fsr_rcas_sharpen(&mid, dst.0, dst.1, sharp).expect("cpu rcas");
    let mut worst = 0u8;
    let mut off_by_more = 0usize;
    for (px, want) in gpu.chunks_exact(4).zip(&cpu) {
        for c in 0..3 {
            let d = px[c].abs_diff(unit_to_byte(want[c]));
            worst = worst.max(d);
            off_by_more += usize::from(d > 2);
        }
    }
    eprintln!("easu + rcas {src:?} -> {dst:?} at {sharp}: worst {worst} LSB, {off_by_more} values over 2 LSB");
    assert!(worst <= 6, "worst byte gap {worst}");
    assert!(
        off_by_more * 100 <= cpu.len() * 3,
        "{off_by_more} byte values over 2 LSB"
    );
    // And the sharpen is real: the result differs from the unsharpened chain.
    let plain = run(
        device,
        queue,
        &input,
        src,
        dst,
        0.0,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    assert_ne!(plain, gpu, "a sharpness of 0.5 must move the picture");
}

#[test]
fn a_srgb_target_gets_the_same_bytes() {
    let Some((device, queue)) = device_or_skip("a_srgb_target_gets_the_same_bytes") else {
        return;
    };
    let (src, dst) = ((64, 36), (96, 54));
    let input = fixture_bytes(src.0, src.1);
    let plain = run(
        device,
        queue,
        &input,
        src,
        dst,
        0.0,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let hw = run(
        device,
        queue,
        &input,
        src,
        dst,
        0.0,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    let worst = plain
        .chunks_exact(4)
        .zip(hw.chunks_exact(4))
        .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
        .max()
        .unwrap_or(0);
    eprintln!("hw srgb target against the plain target: worst {worst} LSB");
    assert!(
        worst <= 2,
        "the *_hw_srgb entry must hand the hardware encode the same bytes: {worst}"
    );
}

#[test]
fn a_step_edge_is_sharper_than_bilinear_on_the_device() {
    let Some((device, queue)) =
        device_or_skip("a_step_edge_is_sharper_than_bilinear_on_the_device")
    else {
        return;
    };
    let (src, dst) = ((64, 32), (96, 48));
    let edge = fsr_edge_lowres(src.0, src.1).expect("edge");
    let input: Vec<u8> = edge
        .iter()
        .flat_map(|p| {
            [
                unit_to_byte(p[0]),
                unit_to_byte(p[1]),
                unit_to_byte(p[2]),
                255,
            ]
        })
        .collect();
    let gpu = bytes_to_rgb(&run(
        device,
        queue,
        &input,
        src,
        dst,
        0.0,
        wgpu::TextureFormat::Rgba8Unorm,
    ));
    let bilinear =
        taa_upsample(&bytes_to_rgb(&input), src.0, src.1, dst.0, dst.1).expect("bilinear");
    let wb = fsr_edge_width_10_90(&bilinear, dst.0, dst.1).expect("bilinear width");
    let wg = fsr_edge_width_10_90(&gpu, dst.0, dst.1).expect("gpu width");
    let lift = fsr_mtf50_lift(wb, wg).expect("lift");
    eprintln!(
        "step edge 10-90 width: bilinear {wb} px, easu on the device {wg} px, mtf50 lift {lift:.3}"
    );
    assert!(
        lift >= FSR_MTF50_LIFT_MIN,
        "lift {lift} under {FSR_MTF50_LIFT_MIN}"
    );
}
