//! capture: the presented image copied into a buffer and read back without waiting.
//!
//! A headless browser canvas screenshot comes out black, so a check of a browser frame asks the
//! GPU instead: the swapchain image is copied into a staging buffer right after the frame's own
//! submit, and the buffer is mapped with a callback. Nothing here blocks. The caller asks for a
//! copy ([`Presenter::request_capture`]), keeps rendering, and polls [`Presenter::take_capture`]
//! on later frames; on a browser the map callback runs from the event loop, natively the poll in
//! `take_capture` lets it finish.
//!
//! Units: pixels; rows are 256-byte padded in the staging buffer and tight in the result, in RGBA
//! order whatever the swapchain order was. Determinism: pixels are presentation and never enter
//! a simulation hash. The digest below is FNV-1a over the RGBA bytes, so any other tool can
//! recompute it. Caller: `Presenter::present_frame` (the copy) and the web shell (the poll).
use super::presenter::Presenter;
use super::ROW_ALIGN;
use crate::present::PresentError;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Exact colours counted before new ones stop being added to the tally. The most common colour of
/// a frame is almost always among the first few thousand seen, so the share stays a fair bound.
pub const DISTINCT_CAP: usize = 4096;

/// One captured frame: tight RGBA8 rows, row 0 at the top.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameCapture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, R G B A per pixel.
    pub rgba: Vec<u8>,
}

/// What a captured frame shows, enough to tell a drawn frame from a blank one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureStats {
    /// Distinct RGB values, counted up to [`DISTINCT_CAP`].
    pub distinct: usize,
    /// Share of all pixels held by the most common RGB value, `0..=1`.
    pub dominant_share: f64,
    /// Mean luma of the RGB channels, `0..=255`.
    pub mean_luma: f64,
}

impl CaptureStats {
    /// A frame is blank when it is one flat colour or all but black: the black canvas of a
    /// headless screenshot, a failed present, a cleared target nothing drew into.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.distinct <= 4 || self.dominant_share >= 0.98 || self.mean_luma < 2.0
    }
}

impl FrameCapture {
    /// FNV-1a 64 over the RGBA bytes.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h = engine_core::Fnv64::new();
        h.write(&self.rgba);
        h.finish()
    }

    /// Colour statistics of the frame.
    #[must_use]
    pub fn stats(&self) -> CaptureStats {
        let pixels = self.rgba.len() / 4;
        let pixel_count = f64::from(u32::try_from(pixels).unwrap_or(u32::MAX));
        let mut counts: BTreeMap<u32, u32> = BTreeMap::new();
        let mut luma = 0.0f64;
        for px in self.rgba.chunks_exact(4) {
            luma +=
                0.2126 * f64::from(px[0]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[2]);
            let key = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
            if let Some(n) = counts.get_mut(&key) {
                *n += 1;
            } else if counts.len() < DISTINCT_CAP {
                counts.insert(key, 1);
            }
        }
        let top = counts.values().copied().max().unwrap_or(0);
        CaptureStats {
            distinct: counts.len(),
            dominant_share: if pixels == 0 {
                1.0
            } else {
                f64::from(top) / pixel_count
            },
            mean_luma: if pixels == 0 { 0.0 } else { luma / pixel_count },
        }
    }
}

/// Bytes per padded staging row for a frame `width` pixels wide.
#[must_use]
pub(crate) fn padded_row(width: u32) -> u32 {
    (width * 4).div_ceil(ROW_ALIGN) * ROW_ALIGN
}

/// Whether a swapchain format can be captured, and if so whether its bytes are blue-first.
/// `None` for formats this readback does not convert (float and packed formats).
#[must_use]
pub(crate) fn capture_order(format: wgpu::TextureFormat) -> Option<bool> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => Some(false),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => Some(true),
        _ => None,
    }
}

/// Padded staging rows to tight RGBA rows, swapping blue and red when the swapchain is BGRA.
/// A staging image shorter than its shape is a typed error, never a slice panic.
pub(crate) fn rows_to_rgba(
    mapped: &[u8],
    width: u32,
    height: u32,
    padded: u32,
    bgra: bool,
) -> Result<Vec<u8>, PresentError> {
    let row = usize::try_from(width).unwrap_or(0) * 4;
    let stride = usize::try_from(padded).unwrap_or(0);
    let rows = usize::try_from(height).unwrap_or(0);
    let need = rows
        .saturating_sub(1)
        .saturating_mul(stride)
        .saturating_add(row);
    if row == 0 || rows == 0 || stride < row || mapped.len() < need {
        return Err(PresentError::Readback(format!(
            "capture staging holds {} bytes, a {width}x{height} frame at row pitch {padded} needs {need}",
            mapped.len()
        )));
    }
    let mut out = Vec::with_capacity(row * rows);
    for y in 0..rows {
        let src = &mapped[y * stride..y * stride + row];
        if bgra {
            for px in src.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            out.extend_from_slice(src);
        }
    }
    Ok(out)
}

const MAPPING: u8 = 0;
const MAPPED: u8 = 1;
const MAP_FAILED: u8 = 2;

/// A copy in flight: the staging buffer and what the map callback said so far.
pub(crate) struct PendingCapture {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded: u32,
    bgra: bool,
    state: Arc<AtomicU8>,
}

/// Where the presenter is in taking a capture.
#[derive(Default)]
pub(crate) enum CaptureSlot {
    /// Nothing asked for.
    #[default]
    Idle,
    /// The next presented frame is copied.
    Armed,
    /// The copy was submitted; the map has not finished.
    Pending(PendingCapture),
}

impl Presenter {
    /// Ask for the next presented surface frame to be copied and read back. The swapchain is
    /// reconfigured once with copy-source usage if it lacks it. `Ok(false)` when a capture is
    /// already waiting or in flight (nothing changes); an error when there is no surface or its
    /// format is not an 8-bit RGBA or BGRA one. Cold path: call it once per check, not per frame.
    pub fn request_capture(&mut self) -> Result<bool, PresentError> {
        if !matches!(self.capture, CaptureSlot::Idle) {
            return Ok(false);
        }
        let (Some(surface), Some(config)) = (self.surface.as_ref(), self.surface_config.as_mut())
        else {
            return Err(PresentError::NoSurface);
        };
        if capture_order(config.format).is_none() {
            return Err(PresentError::Readback(format!(
                "capture reads 8-bit RGBA or BGRA surfaces, this one is {:?}",
                config.format
            )));
        }
        if !config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
            surface.configure(&self.device, config);
        }
        self.capture = CaptureSlot::Armed;
        Ok(true)
    }

    /// The captured frame once its map has finished, `Ok(None)` while it is still on its way or
    /// when none was asked for. Never waits.
    pub fn take_capture(&mut self) -> Result<Option<FrameCapture>, PresentError> {
        // Native maps finish inside a poll; a browser maps from its event loop and the poll is a
        // no-op there. Either way this returns at once.
        let _ = self.device.poll(wgpu::PollType::Poll);
        let CaptureSlot::Pending(pending) = &self.capture else {
            return Ok(None);
        };
        match pending.state.load(Ordering::Acquire) {
            MAPPING => return Ok(None),
            MAP_FAILED => {
                self.capture = CaptureSlot::Idle;
                return Err(PresentError::Readback("capture map failed".into()));
            }
            _ => {}
        }
        let rgba = {
            let mapped = pending
                .buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| PresentError::Readback(format!("capture mapped range: {e}")))?;
            rows_to_rgba(
                &mapped,
                pending.width,
                pending.height,
                pending.padded,
                pending.bgra,
            )?
        };
        pending.buffer.unmap();
        let (width, height) = (pending.width, pending.height);
        self.capture = CaptureSlot::Idle;
        Ok(Some(FrameCapture {
            width,
            height,
            rgba,
        }))
    }

    /// Whether a capture is waiting for its frame or for its map.
    #[must_use]
    pub fn capture_busy(&self) -> bool {
        !matches!(self.capture, CaptureSlot::Idle)
    }

    /// Copy `texture` (the frame just rendered, `width` x `height`, `format`) when a capture
    /// is armed. Called by `present_frame` after the frame's submit and before the present.
    pub(super) fn capture_after_frame(
        &mut self,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        if matches!(self.capture, CaptureSlot::Armed) {
            self.start_capture(texture, format, width, height);
        }
    }

    /// Copy `texture` into a fresh staging buffer and start mapping it. The texture needs
    /// copy-source usage; a format this readback cannot convert drops the capture.
    fn start_capture(
        &mut self,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let Some(bgra) = capture_order(format) else {
            self.capture = CaptureSlot::Idle;
            return;
        };
        let padded = padded_row(width);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture-staging"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("capture-copy"),
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
        self.queue.submit(Some(enc.finish()));
        let state = Arc::new(AtomicU8::new(MAPPING));
        let signal = Arc::clone(&state);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                signal.store(
                    if result.is_ok() { MAPPED } else { MAP_FAILED },
                    Ordering::Release,
                );
            });
        self.capture = CaptureSlot::Pending(PendingCapture {
            buffer,
            width,
            height,
            padded,
            bgra,
            state,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> FrameCapture {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&px(x, y));
            }
        }
        FrameCapture {
            width,
            height,
            rgba,
        }
    }

    #[test]
    fn a_flat_black_frame_is_blank_and_a_gradient_is_not() {
        let black = frame(32, 32, |_, _| [0, 0, 0, 255]).stats();
        assert_eq!(black.distinct, 1);
        assert!(black.is_blank(), "{black:?}");
        let flat = frame(32, 32, |_, _| [200, 180, 90, 255]).stats();
        assert!(flat.is_blank(), "one flat colour is blank: {flat:?}");
        let drawn = frame(32, 32, |x, y| [(x * 8) as u8, (y * 8) as u8, 120, 255]).stats();
        assert!(!drawn.is_blank(), "{drawn:?}");
        assert!(drawn.distinct > 500);
        assert!(drawn.dominant_share < 0.1);
    }

    #[test]
    fn a_mostly_flat_frame_with_a_small_drawn_part_is_still_blank() {
        // 99% one colour: a clear colour with a speck of drawing is not an arena frame.
        let speck = frame(100, 100, |x, y| {
            if x < 10 && y < 10 {
                [x as u8 * 20, y as u8 * 20, 40, 255]
            } else {
                [20, 30, 60, 255]
            }
        })
        .stats();
        assert!(speck.dominant_share > 0.98, "{speck:?}");
        assert!(speck.is_blank());
    }

    #[test]
    fn the_digest_is_fnv1a_over_the_rgba_bytes_and_moves_with_every_pixel() {
        // FNV-1a 64 of "a" is af63dc4c8601ec8c (the published test vector), so a tool outside
        // this crate can recompute the digest of a capture.
        let a = FrameCapture {
            width: 1,
            height: 1,
            rgba: b"a".to_vec(),
        };
        assert_eq!(a.hash(), 0xaf63_dc4c_8601_ec8c);
        let one = FrameCapture {
            width: 1,
            height: 1,
            rgba: vec![9, 8, 7, 255],
        };
        // The same four bytes are a vector in tools/web-browse.test.mjs, whose Node digest the
        // headless check compares with the one the page prints.
        assert_eq!(one.hash(), 0xf247_a9ba_2081_b132);
        let mut other = one.clone();
        other.rgba[3] = 254;
        assert_ne!(one.hash(), other.hash(), "the alpha byte is hashed too");
    }

    #[test]
    fn padded_rows_become_tight_rgba_and_bgra_is_swapped() {
        // 3 pixels wide: 12 data bytes, padded to 256.
        let padded = padded_row(3);
        assert_eq!(padded, 256);
        let mut staging = vec![0xEEu8; padded as usize * 2];
        for (i, byte) in (1u8..=12).enumerate() {
            staging[i] = byte;
            staging[padded as usize + i] = byte + 100;
        }
        let rgba = rows_to_rgba(&staging, 3, 2, padded, false).expect("rows unpad");
        assert_eq!(rgba.len(), 24, "padding bytes are dropped");
        assert_eq!(rgba[..12], (1u8..=12).collect::<Vec<_>>());
        assert_eq!(rgba[12], 101);
        let swapped = rows_to_rgba(&staging, 3, 2, padded, true).expect("rows swap");
        assert_eq!(
            &swapped[..4],
            &[3, 2, 1, 4],
            "B and R trade places, G and A stay"
        );
        assert_eq!(&swapped[4..8], &[7, 6, 5, 8]);
    }

    #[test]
    fn a_short_staging_image_is_a_typed_error_not_a_panic() {
        let padded = padded_row(4);
        let short = vec![0u8; 100];
        assert!(matches!(
            rows_to_rgba(&short, 4, 2, padded, false),
            Err(PresentError::Readback(_))
        ));
        assert!(
            rows_to_rgba(&[], 0, 0, 0, false).is_err(),
            "empty shape refused"
        );
        let tight = vec![0u8; padded as usize + 16];
        assert!(
            rows_to_rgba(&tight, 4, 2, padded, false).is_ok(),
            "last row needs no padding"
        );
    }

    fn twin_presenter(test: &str) -> Option<Presenter> {
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

    fn cube_list(presenter: &mut Presenter) -> crate::present::DrawList {
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        let key = crate::present::MeshKey::new(0xCA00_0001, [7; 32]);
        presenter
            .upload_mesh(key, &scene.mesh)
            .expect("cube uploads");
        crate::present::DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![crate::present::DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        }
    }

    /// The same texture read the blocking way: the oracle for the non-blocking capture.
    fn blocking_rgba(presenter: &Presenter, texture: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        let padded = padded_row(w);
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-capture-oracle"),
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
        super::super::submit::map_readback(
            &presenter.device,
            &mut crate::present::FrameWaits::default(),
            &buffer,
            w * 4,
            padded,
            h,
        )
        .expect("oracle readback maps")
    }

    #[test]
    fn the_nonblocking_capture_returns_the_bytes_a_blocking_readback_returns() {
        let Some(mut presenter) = twin_presenter("nonblocking capture") else {
            return;
        };
        let list = cube_list(&mut presenter);
        presenter
            .render_live_offscreen(&list)
            .expect("live frame renders");
        let (texture, w, h) = {
            let targets = presenter.targets.as_ref().expect("twin targets exist");
            (targets.out_texture.clone(), targets.width, targets.height)
        };
        assert!(!presenter.capture_busy());
        // Not armed: the hook after a frame does nothing.
        presenter.capture_after_frame(&texture, super::super::OUTPUT_FORMAT, w, h);
        assert!(
            !presenter.capture_busy(),
            "an unarmed presenter copies nothing"
        );
        presenter.capture = CaptureSlot::Armed;
        presenter.capture_after_frame(&texture, super::super::OUTPUT_FORMAT, w, h);
        assert!(presenter.capture_busy(), "the copy is in flight");
        let mut got = None;
        for _ in 0..2000 {
            if let Some(frame) = presenter.take_capture().expect("map succeeds") {
                got = Some(frame);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let got = got.expect("the map finishes after polls, without a wait");
        assert!(!presenter.capture_busy(), "the slot is free again");
        assert_eq!((got.width, got.height), (w, h));
        let oracle = blocking_rgba(&presenter, &texture, w, h);
        assert_eq!(got.rgba, oracle, "the same bytes as the blocking readback");
        let stats = got.stats();
        assert!(
            !stats.is_blank(),
            "the lit cube frame is not blank: {stats:?}"
        );
    }

    #[test]
    fn a_capture_without_a_surface_or_with_one_in_flight_is_refused_loudly() {
        let Some(mut presenter) = twin_presenter("capture refusals") else {
            return;
        };
        assert!(
            matches!(presenter.request_capture(), Err(PresentError::NoSurface)),
            "the offscreen twin has no swapchain to copy"
        );
        assert!(
            presenter.take_capture().expect("nothing asked").is_none(),
            "nothing asked for, nothing returned"
        );
        presenter.capture = CaptureSlot::Armed;
        assert!(
            !presenter.request_capture().expect("busy is not an error"),
            "a second ask while one waits changes nothing"
        );
    }

    #[test]
    fn only_eight_bit_rgba_and_bgra_surfaces_are_captured() {
        assert_eq!(capture_order(wgpu::TextureFormat::Rgba8Unorm), Some(false));
        assert_eq!(
            capture_order(wgpu::TextureFormat::Bgra8UnormSrgb),
            Some(true)
        );
        assert_eq!(capture_order(wgpu::TextureFormat::Rgba16Float), None);
        assert_eq!(capture_order(wgpu::TextureFormat::Rgb10a2Unorm), None);
    }
}
