//! tests_gpu: the descriptor and llvmpipe legs: uploads, chart submit, budgets.

#![cfg(test)]

use super::tests_look::HDRI_BYTES;
use super::*;

#[cfg(feature = "v1")]
#[test]
fn missing_hdri_refuses_never_silent() {
    // No bytes refuse with MissingHdri (not an empty bake); undecodable
    // bytes wrap the R2 decode error; oversize bytes wrap TooLarge.
    assert_eq!(bake(&[]), Err(IblError::MissingHdri));
    assert!(matches!(
        bake(b"definitely not an image at all ........"),
        Err(IblError::Hdri(_))
    ));
    let huge = vec![0u8; crate::v1_ibl_shadow::MAX_HDRI_BYTES + 1];
    assert!(matches!(bake(&huge), Err(IblError::Hdri(_))));
}

// ------------------------------------------------------------------
// GPU descriptors behind `window`: pure format/extent checks, a real
// cook-bytes upload round-trip, and the live chart submit that skips
// without an adapter (same skip rule as the RA1 presenter tests).
// ------------------------------------------------------------------

#[cfg(feature = "gpu")]
#[test]
fn ibl_descs_are_sampler_readable() {
    let cube = gpu::prefilter_cube_desc(IBL_CUBE_SIZE, IBL_LEVELS as u32);
    assert_eq!(cube.format, wgpu::TextureFormat::Rgba32Float);
    assert_eq!(
        (cube.size.width, cube.size.height),
        (IBL_CUBE_SIZE, IBL_CUBE_SIZE)
    );
    assert_eq!(cube.size.depth_or_array_layers, IBL_FACES as u32);
    assert_eq!(cube.mip_level_count, IBL_LEVELS as u32);
    assert!(cube.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    assert!(cube.usage.contains(wgpu::TextureUsages::COPY_DST));
    assert!(cube.usage.contains(wgpu::TextureUsages::COPY_SRC));
    let array = gpu::prefilter_array_desc(IBL_CUBE_SIZE, IBL_LEVELS as u32);
    assert_eq!(array.format, wgpu::TextureFormat::Rgba32Float);
    assert_eq!(
        array.size.depth_or_array_layers,
        IBL_LEVELS as u32 * IBL_FACES as u32
    );
    assert_eq!(array.mip_level_count, 1);
    assert!(array.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    assert!(array.usage.contains(wgpu::TextureUsages::COPY_DST));
    let lut = gpu::brdf_lut_desc(BRDF_LUT_SIZE);
    assert_eq!(lut.format, wgpu::TextureFormat::Rg32Float);
    assert_eq!(
        (lut.size.width, lut.size.height),
        (BRDF_LUT_SIZE, BRDF_LUT_SIZE)
    );
    assert!(lut.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    assert!(lut.usage.contains(wgpu::TextureUsages::COPY_SRC));
    assert!(!gpu::FORWARD_IBL_WGSL.is_empty(), "forward chunk ships");
    for marker in FORWARD_IBL_MARKERS {
        assert!(
            gpu::FORWARD_IBL_WGSL.contains(marker),
            "forward chunk carries marker '{marker}'"
        );
    }
    assert!(
        gpu::FORWARD_IBL_WGSL.contains("ibl_sh[8]"),
        "forward chunk dots all 9 SH coefficients"
    );
    assert!(
        assert_forward_wgsl_binds_ibl(gpu::FORWARD_IBL_WGSL).is_ok(),
        "the shipped chunk passes its own wiring gate"
    );
    assert!(!gpu::CHART_WGSL.is_empty(), "chart pass ships");
    assert!(
        gpu::CHART_WGSL.contains("textureLoad"),
        "chart pass fetches exact cook texels"
    );
    let sh_bytes = gpu::sh_uniform_bytes(&IrradianceSh::zero());
    assert_eq!(sh_bytes.len(), 9 * 16, "SH uniform is 9 vec4s");
    assert!(
        sh_bytes.iter().all(|b| *b == 0),
        "zero coefficients pack to zero bytes"
    );
    // Chart inputs pin the CPU/GPU contract: columns alternate metal,
    // halves swap albedo, rows sweep roughness, normals stay unit.
    let (n, _, albedo, metal, rough) = gpu::chart_inputs(0, 0, 32, 32);
    assert_eq!((metal, albedo), (0.0, [0.5, 0.5, 0.5]));
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    assert!((l - 1.0).abs() < 1e-6, "chart normal is unit");
    let (_, _, warm, metal_r, rough_top) = gpu::chart_inputs(17, 31, 32, 32);
    assert_eq!((metal_r, warm), (1.0, [0.8, 0.2, 0.1]));
    assert!(
        (rough_top - 31.5 / 32.0).abs() < 1e-6 && rough < rough_top,
        "rows sweep roughness upward"
    );
}

#[cfg(feature = "gpu")]
#[test]
fn brdf_lut_upload_roundtrips_on_llvmpipe() {
    // Cook bytes upload verbatim and read back bit-identical: the GPU
    // bake matches the CPU reference texel-for-texel (max diff 0.0).
    let lut = brdf_lut_build(16).expect("small lut builds");
    crate::test_gpu::hold_gpu_slot();
    let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
    let probe = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|error| {
        crate::test_gpu::NoAdapter(format!("no wgpu adapter on this machine: {error}"))
    });
    let Some(adapter) =
        crate::test_gpu::require_or_skip("brdf_lut_upload_roundtrips_on_llvmpipe", probe)
    else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rl2-ibl-lut"),
        ..Default::default()
    }))
    .expect("device requests succeed when the adapter exists");
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let texture = gpu::upload_brdf_lut(&device, &queue, &lut).expect("lut uploads");
    let size = 16u32;
    let padded: u32 = 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rl2-ibl-lut-readback"),
        size: (padded as u64) * (size as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rl2-ibl-lut-copy"),
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll succeeds");
    rx.recv().expect("map channel").expect("map succeeds");
    let mut max_diff = 0.0f32;
    {
        let mapped = slice.get_mapped_range().expect("mapped range");
        for (row, t_row) in lut.data.chunks_exact(size as usize).enumerate() {
            let off = row * padded as usize;
            for (i, t) in t_row.iter().enumerate() {
                let bits = &mapped[off + i * 8..off + i * 8 + 8];
                let g0 = f32::from_le_bytes([bits[0], bits[1], bits[2], bits[3]]);
                let g1 = f32::from_le_bytes([bits[4], bits[5], bits[6], bits[7]]);
                max_diff = max_diff.max((g0 - t[0]).abs());
                max_diff = max_diff.max((g1 - t[1]).abs());
            }
        }
    }
    readback.unmap();
    let scope_err = pollster::block_on(scope.pop());
    assert!(
        scope_err.is_none(),
        "zero validation errors, got {scope_err:?}"
    );
    assert_eq!(max_diff, 0.0, "GPU LUT matches the CPU bake exactly");
    eprintln!("RL2 GPU LUT round-trip: 16x16 texels, max diff {max_diff}");
}

#[cfg(all(feature = "v1", feature = "gpu"))]
#[test]
fn ibl_chart_submits_with_look_binds_on_llvmpipe() {
    // RL2 live: the full bake binds (prefilter array, BRDF LUT, SH) next
    // to the shared look frame in one real wgpu pass, and the float
    // readback tracks the CPU `shade_ibl` reference inside the posted
    // chart tolerance with zero validation errors.
    let Some((device, queue, info)) = crate::test_gpu::require_or_skip(
        "ibl_chart_submits_with_look_binds_on_llvmpipe",
        crate::graph::gx1_probe(),
    ) else {
        return;
    };
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let mut submitter = gpu::IblSubmitter::new(device);
    let frame = crate::gtao::LookFrame {
        frame_index: 5,
        viewport_px: [32, 32],
        exposure_ev100: 10.0,
        sun_dir: [0.3, 0.8, 0.5],
    };
    let receipt = bake
        .submit_gpu(device, queue, &frame, &mut submitter)
        .expect("chart submits");
    assert_eq!(receipt.passes_submitted, 1, "one chart pass recorded");
    assert!(
        receipt.is_clean(),
        "0 validation errors, got {}",
        receipt.validation_errors
    );
    assert_eq!(
        receipt.covered_pixels,
        32 * 32,
        "every chart pixel shades, got {}",
        receipt.covered_pixels
    );
    assert_eq!(
        (receipt.frame_index, receipt.width_px, receipt.height_px),
        (5, 32, 32),
        "receipt names the submitted frame"
    );
    assert_eq!(
        receipt.cube_texels,
        IBL_LEVELS as u64 * IBL_FACES as u64 * IBL_CUBE_SIZE as u64 * IBL_CUBE_SIZE as u64
    );
    assert_eq!(
        receipt.lut_texels,
        BRDF_LUT_SIZE as u64 * BRDF_LUT_SIZE as u64
    );
    assert_eq!(receipt.sh_coeffs, IBL_SH_COEFFS as u32);
    assert!(
        receipt.within_chart_tolerance(),
        "chart tracks CPU shade: abs {} over {} / rel {:.4}% over {}",
        receipt.max_abs_diff,
        gpu::IBL_CHART_ABS_TOL,
        receipt.max_rel_diff * 100.0,
        gpu::IBL_CHART_REL_TOL * 100.0
    );
    // Wall-clock budget lives in ibl_chart_submit_wall_budget (--perf
    // only): cold submits pay pipeline compile and idle clocks (observed
    // 210MHz vs ~1500MHz boost), never a default gate.
    eprintln!(
        "RL2 chart on {}: covered={} abs={:.6} rel={:.4}% submit_ms={:.2}",
        info.name,
        receipt.covered_pixels,
        receipt.max_abs_diff,
        receipt.max_rel_diff * 100.0,
        receipt.submit_ms
    );
}

#[cfg(all(feature = "v1", feature = "gpu"))]
#[test]
fn ibl_submit_reuses_backing_and_binds() {
    // Steady state creates nothing: the second submit on matching shape
    // reuses every texture and bind group, and both receipts agree
    // exactly (deterministic GPU bytes twice).
    let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
        "ibl_submit_reuses_backing_and_binds",
        crate::graph::gx1_probe(),
    ) else {
        return;
    };
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let mut submitter = gpu::IblSubmitter::new(device);
    assert_eq!(
        (submitter.texture_count(), submitter.bind_group_count()),
        (0, 0),
        "submitter sizes nothing up front"
    );
    let frame = |index: u64| crate::gtao::LookFrame {
        frame_index: index,
        viewport_px: [32, 32],
        exposure_ev100: 10.0,
        sun_dir: [0.3, 0.8, 0.5],
    };
    let first = bake
        .submit_gpu(device, queue, &frame(5), &mut submitter)
        .expect("first submits");
    assert_eq!(
        (submitter.texture_count(), submitter.bind_group_count()),
        (3, 2),
        "one array plus one LUT plus one target, one bind per group"
    );
    let second = bake
        .submit_gpu(device, queue, &frame(6), &mut submitter)
        .expect("second submits");
    assert_eq!(
        (submitter.texture_count(), submitter.bind_group_count()),
        (3, 2),
        "steady-state submit creates no new textures or bind groups"
    );
    assert_eq!(
        (
            second.covered_pixels,
            second.max_abs_diff,
            second.max_rel_diff
        ),
        (first.covered_pixels, first.max_abs_diff, first.max_rel_diff),
        "same bake twice gives identical chart pixels"
    );
    assert!(second.is_clean() && second.within_chart_tolerance());
    // Wall-clock budget lives in ibl_chart_submit_wall_budget (--perf only).
}

#[cfg(all(feature = "v1", feature = "gpu"))]
#[test]
fn ibl_submit_grows_no_host_scratch() {
    // Steady-state submits grow nothing on the host: both scratch
    // capacities stay flat across submits, and the chart still tracks
    // the CPU reference. Sizing happens once, on the first upload.
    let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
        "ibl_submit_grows_no_host_scratch",
        crate::graph::gx1_probe(),
    ) else {
        return;
    };
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let mut submitter = gpu::IblSubmitter::new(device);
    let frame = |index: u64| crate::gtao::LookFrame {
        frame_index: index,
        viewport_px: [32, 32],
        exposure_ev100: 10.0,
        sun_dir: [0.3, 0.8, 0.5],
    };
    bake.submit_gpu(device, queue, &frame(5), &mut submitter)
        .expect("first submits");
    let caps = submitter.scratch_capacities();
    assert_eq!(
        caps,
        (64 * 64 * 16, 64 * 64 * 8),
        "first submit sizes staging exactly, got {caps:?}"
    );
    for index in 6..9 {
        let receipt = bake
            .submit_gpu(device, queue, &frame(index), &mut submitter)
            .expect("steady state submits");
        assert_eq!(
            submitter.scratch_capacities(),
            caps,
            "steady-state submit grows no host scratch"
        );
        assert!(receipt.is_clean() && receipt.within_chart_tolerance());
    }
}

#[cfg(all(feature = "v1", feature = "gpu"))]
#[test]
#[ignore = "timing: chart submit wall-clock needs release --perf on the boosted reference GPU (cold/idle-clock submits miss the budget)"]
fn ibl_chart_submit_wall_budget() {
    let Some((device, queue, _)) =
        crate::test_gpu::require_or_skip("ibl_chart_submit_wall_budget", crate::graph::gx1_probe())
    else {
        return;
    };
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let mut submitter = gpu::IblSubmitter::new(device);
    let frame = |index: u64| crate::gtao::LookFrame {
        frame_index: index,
        viewport_px: [32, 32],
        exposure_ev100: 10.0,
        sun_dir: [0.3, 0.8, 0.5],
    };
    // Warm the pipeline once so the budget reads steady state, not compile.
    let _ = bake
        .submit_gpu(device, queue, &frame(5), &mut submitter)
        .expect("warmup submits");
    let second = bake
        .submit_gpu(device, queue, &frame(6), &mut submitter)
        .expect("budget submit runs");
    // Perf harness line: log the measured submit so --perf medians are readable.
    eprintln!(
        "ibl_chart_submit_wall_budget: steady submit {:.2}ms vs {:.1}ms budget",
        second.submit_ms,
        second.budget_ms()
    );
    assert!(second.is_clean(), "0 validation errors");
    assert!(
        second.within_budget(),
        "submit {:.2}ms over budget {:.1}ms",
        second.submit_ms,
        second.budget_ms()
    );
}

#[cfg(all(feature = "v1", feature = "gpu"))]
#[test]
fn ibl_submit_rejects_bad_inputs() {
    // Bad frames and non-standard bakes refuse before any target sizes;
    // device errors stay loud and typed.
    let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
        "ibl_submit_rejects_bad_inputs",
        crate::graph::gx1_probe(),
    ) else {
        return;
    };
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let mut submitter = gpu::IblSubmitter::new(device);
    let frame = crate::gtao::LookFrame {
        frame_index: 5,
        viewport_px: [32, 32],
        exposure_ev100: 10.0,
        sun_dir: [0.3, 0.8, 0.5],
    };
    let bad_view = crate::gtao::LookFrame {
        viewport_px: [0, 32],
        ..frame
    };
    match bake.submit_gpu(device, queue, &bad_view, &mut submitter) {
        Err(IblError::Invalid(detail)) => assert!(!detail.is_empty(), "never silent"),
        other => panic!("zero viewport must fail Invalid, got {other:?}"),
    }
    // Window viewports downscale into the edge instead of failing the
    // frame: 512x32 submits the 256x16 chart on its own submitter and
    // still tracks the CPU reference.
    let huge = crate::gtao::LookFrame {
        viewport_px: [512, 32],
        ..frame
    };
    let mut wide = gpu::IblSubmitter::new(device);
    let wide_receipt = bake
        .submit_gpu(device, queue, &huge, &mut wide)
        .expect("wide viewport downscales into the edge");
    assert_eq!(
        (wide_receipt.width_px, wide_receipt.height_px),
        (256, 16),
        "512x32 downscales to 256x16"
    );
    assert!(wide_receipt.is_clean(), "0 validation errors");
    assert!(
        wide_receipt.within_chart_tolerance(),
        "downscaled chart tracks CPU shade: abs {} / rel {:.4}%",
        wide_receipt.max_abs_diff,
        wide_receipt.max_rel_diff * 100.0
    );
    // Direct over-edge targets still refuse loudly before sizing.
    match check_chart_target_size(512, 32) {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("exceeds"), "names the cap: {detail}");
        }
        other => panic!("oversize target must fail Invalid, got {other:?}"),
    }
    let small_cube = IblBake {
        cube: PrefilterCube::new(8, IBL_LEVELS),
        sh: IrradianceSh::zero(),
        brdf: BrdfLut::new(8),
        source_average: [0.5, 0.5, 0.5],
    };
    match small_cube.submit_gpu(device, queue, &frame, &mut submitter) {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("standard"), "names the shape: {detail}");
        }
        other => panic!("non-standard cube must fail Invalid, got {other:?}"),
    }
    let small_lut = IblBake {
        brdf: BrdfLut::new(8),
        ..bake.clone()
    };
    match small_lut.submit_gpu(device, queue, &frame, &mut submitter) {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("BRDF"), "names the shape: {detail}");
        }
        other => panic!("non-standard LUT must fail Invalid, got {other:?}"),
    }
    assert_eq!(
        (submitter.texture_count(), submitter.bind_group_count()),
        (0, 0),
        "refused submits size nothing"
    );
    for err in [
        IblError::Device("poll".to_string()),
        IblError::Validation("scope".to_string()),
        IblError::Invalid("bad frame".to_string()),
    ] {
        let text = format!("{err}");
        assert!(!text.is_empty(), "errors are never silent");
    }
}
