//! debug_gbuffer: the GPU half of the A3 debug view that was missing. The
//! default forward pass keeps no normal, albedo or material channel, so
//! `buffers::read_frame` had nothing to read. This module owns the second
//! forward pipeline (`fs_gbuffer`), the three channel targets plus their own
//! depth, and `Presenter::debug_capture`, which re-renders one staged draw
//! list into them and hands the result to `read_frame_channels`.
//!
//! Nothing here runs on a live frame: the targets are built on the first
//! capture, the live forward pipeline, targets and pixels are never touched,
//! and `clear_debug_targets` gives the memory back. Presentation only, never
//! the tick hash.
//!
//! Owner: render lane. Callers: the A3 debug view, its device test and any
//! tool that asks `Presenter::debug_capture` for a frame.

use super::buffers_read::read_frame_channels;
use super::presenter::Presenter;
use super::submit::{effective_camera, padded_depth_row, ForwardLook};
use super::DEPTH_FORMAT;
use crate::buffers::DebugFrame;
use crate::present::{DrawList, PresentError};

/// One frame of debug channels, sized to the presenter's frame.
///
/// The three colour channels are the order `buffers::read_frame` reads
/// (normal, albedo, id); the depth is this capture's own `Depth32Float`
/// target, copied to a buffer after the pass because a depth texture cannot
/// be copied straight into the `R32Float` readback `read_frame` expects.
pub(crate) struct DebugTargets {
    /// Frame width these targets were built for.
    pub(super) width: u32,
    /// Frame height these targets were built for.
    pub(super) height: u32,
    pub(super) normal_texture: wgpu::Texture,
    pub(super) normal_view: wgpu::TextureView,
    pub(super) albedo_texture: wgpu::Texture,
    pub(super) albedo_view: wgpu::TextureView,
    pub(super) ids_texture: wgpu::Texture,
    pub(super) ids_view: wgpu::TextureView,
    pub(super) depth_texture: wgpu::Texture,
    pub(super) depth_view: wgpu::TextureView,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) skinned: wgpu::RenderPipeline,
}

/// One colour attachment that clears its view and stores every fragment.
/// A3 uses it for the three debug channels, whose clear values say "empty"
/// (`+Z` normal, black albedo, id 0). Caller: `encode_forward`'s debug arm.
pub(super) fn clear_attachment<'a>(
    view: &'a wgpu::TextureView,
    color: wgpu::Color,
) -> wgpu::RenderPassColorAttachment<'a> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(color),
            store: wgpu::StoreOp::Store,
        },
    }
}

/// A3: the two debug G-buffer pipelines. Same vertex entries, groups and
/// depth state as the window forward pass, but the `fs_gbuffer` fragment entry
/// and three channels in the order `buffers::read_frame` reads them: normal and
/// albedo `Rgba8Unorm`, the material id `R16Uint`. No blend: nothing
/// accumulates into a debug channel. The debug view runs the shader's default
/// look guard, so it shows the frame as staged, never a tier's approximation.
/// Cold path only. Caller: `DebugTargets::new`.
fn debug_pipelines(presenter: &Presenter) -> [wgpu::RenderPipeline; 2] {
    let formats = [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::R16Uint,
    ];
    let multisample = wgpu::MultisampleState::default();
    let constants: [(&str, f64); 0] = [];
    let build = |skinned: bool| {
        super::submit::forward_pipeline_for(
            &presenter.device,
            &presenter.forward_shader,
            &presenter.forward_layout,
            &presenter.ao_layout,
            &formats,
            multisample,
            &constants,
            skinned,
            true,
        )
    };
    [build(false), build(true)]
}

impl DebugTargets {
    /// Build the channels, their depth and the two debug pipelines for one
    /// size. Cold path: only a size change or the first capture reaches it.
    fn new(presenter: &Presenter, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let colour = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            let texture = presenter.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (normal_texture, normal_view) = colour(
            "ra1-debug-normal",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let (albedo_texture, albedo_view) = colour(
            "ra1-debug-albedo",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let (ids_texture, ids_view) = colour(
            "ra1-debug-ids",
            wgpu::TextureFormat::R16Uint,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = colour(
            "ra1-debug-depth",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let [pipeline, skinned] = debug_pipelines(presenter);
        Self {
            width,
            height,
            normal_texture,
            normal_view,
            albedo_texture,
            albedo_view,
            ids_texture,
            ids_view,
            depth_texture,
            depth_view,
            pipeline,
            skinned,
        }
    }
}

impl Presenter {
    /// Whether a debug G-buffer exists right now. `false` before the first
    /// [`Self::debug_capture`] and after [`Self::clear_debug_targets`].
    #[must_use]
    pub fn debug_targets_on(&self) -> bool {
        self.debug_targets.is_some()
    }

    /// Release the debug channels and their depth. The next capture rebuilds
    /// them; a live frame never notices.
    pub fn clear_debug_targets(&mut self) {
        self.debug_targets = None;
    }

    /// Render one staged draw list into the debug G-buffer and return it as a
    /// [`DebugFrame`]: world-space normal (octahedral), linear albedo, the
    /// material id and this capture's depth.
    ///
    /// The list is the caller's because the presenter keeps no staged list;
    /// two calls with the same list re-render the same frame and give equal
    /// digests. The live frame's targets, pixels and depth are untouched, so
    /// a capture between two [`Self::render_offscreen`] calls leaves their
    /// bytes equal. Presentation only, never hashed.
    ///
    /// # Errors
    ///
    /// [`PresentError`] when the presenter has no frame size, the draw list
    /// cannot be staged, the device is lost, or a channel readback fails.
    pub fn debug_capture(&mut self, list: &DrawList) -> Result<DebugFrame, PresentError> {
        self.check_device()?;
        if self.suspended() {
            return Err(PresentError::Device(
                "a suspended presenter renders no debug frame".into(),
            ));
        }
        let (width, height) = self.swap.size();
        let frame = effective_camera(list, width, height)?;
        let staged = self.stage_frame(list, &frame)?;
        let (cam_bytes, light_bytes) = (staged.cam_bytes, staged.light_bytes);
        // The forward group's second binding is the frame's occlusion field;
        // the debug pass shades nothing with it, but the layout is shared.
        self.ensure_targets(width, height);
        self.write_frame_uniforms(&cam_bytes, &light_bytes, true);
        self.write_fog_forward(&frame, false)?;
        self.resolve_draw_binds(list)?;
        match self.debug_targets.as_ref() {
            Some(targets) if targets.width == width && targets.height == height => {}
            _ => {
                self.debug_targets = Some(DebugTargets::new(self, width, height));
            }
        }

        let padded = padded_depth_row(width);
        let depth_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-debug-depth-readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-debug-frame"),
            });
        {
            let targets = self.targets.as_ref().ok_or_else(|| {
                PresentError::Device("the debug capture has no frame targets".into())
            })?;
            let dbg = self
                .debug_targets
                .as_ref()
                .ok_or_else(|| PresentError::Device("the debug capture has no channels".into()))?;
            self.encode_forward(
                &mut enc,
                targets,
                list,
                ForwardLook::blocking(false),
                staged.order(),
                Some(dbg),
            );
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &dbg.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &depth_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit(Some(enc.finish()));
        let depth = map_depth(&self.device, &depth_buffer, width, height, padded)?;
        let dbg = self
            .debug_targets
            .as_ref()
            .ok_or_else(|| PresentError::Device("the debug capture has no channels".into()))?;
        read_frame_channels(
            &self.device,
            &self.queue,
            &depth,
            Some(&dbg.normal_texture),
            Some(&dbg.albedo_texture),
            Some(&dbg.ids_texture),
            [width, height],
        )
        .map_err(|error| PresentError::Device(error.to_string()))
    }
}

/// Map the copied depth rows and strip the copy padding into one `f32` per
/// texel, the shape `DebugFrame::from_gbuffer` checks.
fn map_depth(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    width: u32,
    height: u32,
    padded: u32,
) -> Result<Vec<f32>, PresentError> {
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    // Native waits here; a browser cannot block the main thread on the GPU, and
    // its map is answered by a later frame, so the wasm path reports the miss
    // from `get_mapped_range` below instead of waiting for it.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    #[cfg(target_arch = "wasm32")]
    {
        let _ = device;
        return Err(PresentError::Device(
            "the debug G-buffer readback waits on the GPU map, which a browser cannot block on"
                .into(),
        ));
    }
    let row_bytes = usize::try_from(width).unwrap_or(0).saturating_mul(4);
    let stride = usize::try_from(padded).unwrap_or(0);
    let rows = usize::try_from(height).unwrap_or(0);
    let mut depth = Vec::with_capacity(row_bytes.saturating_mul(rows) / 4);
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|_| PresentError::Device("the debug depth readback did not map".into()))?;
        for row in 0..rows {
            let start = row.saturating_mul(stride);
            let Some(line) = mapped.get(start..start.saturating_add(row_bytes)) else {
                continue;
            };
            for quad in line.chunks_exact(4) {
                depth.push(f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]));
            }
        }
    }
    buffer.unmap();
    Ok(depth)
}

// A copy of the readback row alignment lives in `submit`; padding here comes
// from `padded_depth_row`, so no second constant is needed.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffers::{decode_normal, lookup_id, quantize_u16};
    use crate::material_kind::MaterialKind;
    use crate::present::{DrawItem, MeshKey, Presenter};
    use crate::v1;

    fn presenter_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(mut presenter) => {
                presenter.resize(128, 128).expect("valid size applies");
                Ok(presenter)
            }
            Err(PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        crate::test_gpu::require_or_skip(test, probe)
    }

    fn list_with(scene: &v1::Scene, keys: &[MeshKey]) -> DrawList {
        DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: keys
                .iter()
                .map(|key| DrawItem {
                    mesh: *key,
                    material: scene.material.clone(),
                    base_color: None,
                    normal_map: None,
                    orm: None,
                    emissive: None,
                })
                .collect(),
        }
    }

    fn pixels(presenter: &mut Presenter, list: &DrawList) -> Vec<u8> {
        let out = presenter.render_offscreen(list).expect("frame renders");
        assert!(
            out.receipt.is_clean(),
            "no validation errors: {:?}",
            out.receipt
        );
        let png = out.png.expect("capture has pixels");
        image::load_from_memory(&png)
            .expect("capture is a PNG")
            .to_rgba8()
            .into_raw()
    }

    /// (largest channel difference, pixels differing by more than `slack`).
    fn compare(a: &[u8], b: &[u8], slack: u8) -> (u8, usize) {
        assert_eq!(a.len(), b.len(), "same frame size");
        let mut worst = 0u8;
        let mut over = 0usize;
        for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let d = pa
                .iter()
                .zip(pb)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            worst = worst.max(d);
            over += usize::from(d > slack);
        }
        (worst, over)
    }

    /// Nearest-depth pixel of a frame: the one texel a draw is sure to cover,
    /// so the "known position" needs no camera math.
    fn nearest(frame: &DebugFrame) -> (u32, u32) {
        let mut best = (0u32, 0u32, u16::MAX);
        for y in 0..frame.height {
            for x in 0..frame.width {
                let index = usize::try_from(y)
                    .unwrap_or(0)
                    .saturating_mul(usize::try_from(frame.width).unwrap_or(0))
                    .saturating_add(usize::try_from(x).unwrap_or(0));
                let depth = frame.depth.get(index).copied().unwrap_or(u16::MAX);
                if depth < best.2 {
                    best = (x, y, depth);
                }
            }
        }
        (best.0, best.1)
    }

    /// The debug G-buffer answers real channels, clears on demand and leaves
    /// the live capture bytes alone. Needs a device; skips loudly with no
    /// adapter.
    #[test]
    fn debug_capture_answers_normal_albedo_id_and_depth() {
        let Some(mut presenter) =
            presenter_or_skip("debug_capture_answers_normal_albedo_id_and_depth")
        else {
            return;
        };
        assert!(
            !presenter.debug_targets_on(),
            "no debug targets before the first capture"
        );
        let scene = v1::default_scene(128, 128).expect("default scene is valid");
        let cube = v1::cube();
        let skin_key = MeshKey::new(0x5C1F, [0x5C; 32]);
        let metal_key = MeshKey::new(0x9E7A, [0x9E; 32]);
        presenter
            .upload_mesh(skin_key, &cube)
            .expect("cube uploads");
        presenter
            .upload_mesh(metal_key, &cube)
            .expect("cube uploads");
        presenter.set_material_kind(skin_key, MaterialKind::Skin);
        presenter.set_material_kind(metal_key, MaterialKind::Metal);
        let skin_list = list_with(&scene, &[skin_key]);
        let metal_list = list_with(&scene, &[metal_key]);

        let before = pixels(&mut presenter, &skin_list);
        // One plain repeat first: the first capture of a fresh presenter may
        // warm pipelines and history, so the steady frame is the pair to
        // compare across the debug capture.
        let steady = pixels(&mut presenter, &skin_list);
        let skin = presenter.debug_capture(&skin_list).expect("skin frame");
        assert!(
            presenter.debug_targets_on(),
            "the first capture builds the targets"
        );
        let again = presenter.debug_capture(&skin_list).expect("second frame");
        for channel in 0..4 {
            assert_eq!(
                skin.digest(channel),
                again.digest(channel),
                "channel {channel} repeats for the same list"
            );
        }
        let metal = presenter.debug_capture(&metal_list).expect("metal frame");
        let after = pixels(&mut presenter, &skin_list);
        // The capture path keeps per-frame history (exposure and dither), so
        // the pair is compared the way the capture tests compare two frames:
        // no pixel may move by more than a dither step. A debug capture that
        // wrote the HDR, id or motion target would move whole regions.
        let (worst, over) = compare(&steady, &after, 8);
        assert_eq!(
            over, 0,
            "a debug capture leaves the live capture pixels unchanged: worst {worst}"
        );
        if before != steady {
            eprintln!("note: the first capture of a fresh presenter differs from the steady one");
        }

        // The nearest texel is on the cube: both materials cover it, and the
        // far corner keeps the cleared far depth.
        let (x, y) = nearest(&skin);
        assert_eq!(nearest(&metal), (x, y), "both cubes cover one texel");
        let index = usize::try_from(y)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(skin.width).unwrap_or(0))
            .saturating_add(usize::try_from(x).unwrap_or(0));
        assert_eq!(
            lookup_id(&skin, x, y),
            1,
            "the skin draw's family at ({x}, {y})"
        );
        assert_eq!(
            lookup_id(&metal, x, y),
            2,
            "the metal draw's family at ({x}, {y})"
        );
        assert_eq!(
            lookup_id(&skin, 0, 0).min(lookup_id(&metal, 0, 0)),
            0,
            "the empty corner claims no material"
        );
        let far = quantize_u16(1.0);
        assert!(
            skin.depth.contains(&far),
            "uncovered texels keep the cleared far depth"
        );
        let covered = skin.depth.get(index).copied().unwrap_or(u16::MAX);
        assert!(
            covered < far,
            "the covered texel is nearer than the far clear: {covered}"
        );
        assert_eq!(
            skin.depth.get(index),
            metal.depth.get(index),
            "the same geometry copies the same depth"
        );

        // Normal: a unit direction within one byte step after decode.
        let normal = decode_normal(&skin.normal, index);
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!(
            (length - 1.0).abs() <= 1.0 / 255.0,
            "the decoded normal is unit length, read {length} from {normal:?}"
        );
        // Albedo: the linear material colour within two byte steps.
        let base = scene.material.base_color;
        let albedo = skin
            .albedo
            .get(index.saturating_mul(4)..index.saturating_mul(4) + 3)
            .expect("three albedo bytes");
        for (channel, want) in base.iter().enumerate() {
            let got = f32::from(albedo[channel]) / 255.0;
            assert!(
                (got - want).abs() <= 2.0 / 255.0,
                "albedo channel {channel} reads {got}, want {want}"
            );
        }

        presenter.clear_debug_targets();
        assert!(
            !presenter.debug_targets_on(),
            "clear releases the debug targets"
        );
        eprintln!(
            "debug_capture_answers_normal_albedo_id_and_depth: 128x128, id 1 then 2 at ({x}, {y}), normal length {length:.5}, depth {covered}, two captures equal, targets cleared"
        );
    }
}
