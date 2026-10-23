//! The after-blit debug line pass: one textured-quad draw over the blit view.
//!
//! The staged [`LineBatch`] becomes thin screen-space quads and rides the
//! overlay pipeline (see [`super::live::live_overlay_pipeline`]); no new render
//! target and no depth attachment. A group's `depth_bias_permille` orders
//! nothing here because the pass draws after the blit with no depth test; the
//! recorder sink keeps it for the export. Presentation only, never hashed.
//!
//! Owner: render lane. Units: 0..1 overlay space; a width is a fraction of the
//! frame height. Determinism: the same batch and width give the same vertex
//! bytes. Budget: one vertex buffer per frame, one draw, allocating while it draws.
//! Callers: `present_frame_live` (window and twin) and
//! `submit::render_offscreen_capture` (the arena and web captures).

use super::live::LiveSink;
use super::presenter::Presenter;
use super::OUTPUT_FORMAT;
use crate::debug_draw::{LineBatch, LineQuad, DEBUG_LINE_MAX_QUADS, DEBUG_LINE_WIDTH};
use crate::overlay::{OverlayDraw, QuadVertex, VERTS_PER_QUAD};

impl Presenter {
    /// Switch the after-blit debug line pass; off by default, so a frame with
    /// nothing to show stays bit-identical. Returns the previous switch. The
    /// staged batch survives either way, so a toggle back on needs no push.
    pub fn debug_line_enable(&mut self, on: bool) -> bool {
        let previous = self.debug_line_on;
        self.debug_line_on = on;
        previous
    }

    /// True when the after-blit line pass draws this frame.
    pub fn debug_lines_enabled(&self) -> bool {
        self.debug_line_on
    }

    /// Output format of the after-blit draws for this sink: the LDR twin or
    /// the attached surface. Presentation only.
    pub(super) fn live_overlay_format(&self, sink: LiveSink<'_>) -> wgpu::TextureFormat {
        match sink {
            LiveSink::Twin => OUTPUT_FORMAT,
            LiveSink::Surface(_) => self
                .surface_config
                .as_ref()
                .map(|config| config.format)
                .unwrap_or(OUTPUT_FORMAT),
        }
    }

    /// Draw the staged debug lines over the blit view: one draw for the whole
    /// batch, no depth. An empty batch or the off switch records nothing, so
    /// the frame stays bit-identical to one without the pass.
    pub(super) fn encode_debug_lines(
        &self,
        enc: &mut wgpu::CommandEncoder,
        aspect: f32,
        format: wgpu::TextureFormat,
        out_view: &wgpu::TextureView,
    ) {
        if !self.debug_line_on || self.debug_lines.is_empty() {
            return;
        }
        let mut quads = [LineQuad::default(); DEBUG_LINE_MAX_QUADS];
        let made = crate::debug_draw::line_quads(&self.debug_lines, DEBUG_LINE_WIDTH, &mut quads);
        if made == 0 {
            return;
        }
        let solid = crate::overlay::text::solid_uv();
        let uv = [
            [solid[0], solid[1]],
            [solid[2], solid[1]],
            [solid[0], solid[3]],
            [solid[2], solid[3]],
        ];
        let mut verts = vec![QuadVertex::default(); DEBUG_LINE_MAX_QUADS * VERTS_PER_QUAD];
        for (index, quad) in quads[..made].iter().enumerate() {
            let col4 = display_colour(quad.colour_rgba);
            let base = index * VERTS_PER_QUAD;
            for (slot, (corner, uv2)) in quad.corners.iter().zip(uv).enumerate() {
                verts[base + slot] = QuadVertex {
                    pos2: *corner,
                    uv2,
                    col4,
                };
            }
        }
        let plan = OverlayDraw {
            quads: made,
            verts: made * VERTS_PER_QUAD,
            mvp: crate::overlay::overlay_mvp(aspect),
        };
        let pipeline = self.overlay_pipeline_for(format);
        self.encode_overlay(
            enc,
            Some(plan),
            &verts[..plan.verts],
            &mvp_bytes(plan.mvp),
            out_view,
            &pipeline,
        );
    }
}

/// The overlay uniform bytes: the 4x4 ortho in column-major order, 64 bytes.
pub(super) fn mvp_bytes(mvp: [[f32; 4]; 4]) -> [u8; 64] {
    let mut out = [0u8; 64];
    let mut at = 0usize;
    for column in mvp {
        for value in column {
            let end = at.saturating_add(4);
            out[at..end].copy_from_slice(&value.to_le_bytes());
            at = end;
        }
    }
    out
}

/// Linear RGBA to the sRGB bytes the after-blit view holds: the blit already
/// encoded the frame, so a linear debug colour takes the same encode. Alpha
/// rides the same curve; the recorder keeps the true linear value.
fn display_colour(colour: [f32; 4]) -> [u8; 4] {
    let rgb = crate::tonemap::linear_to_srgb([colour[0], colour[1], colour[2]]);
    [
        crate::shadow_post::srgb_u8(rgb[0]),
        crate::shadow_post::srgb_u8(rgb[1]),
        crate::shadow_post::srgb_u8(rgb[2]),
        crate::shadow_post::srgb_u8(colour[3]),
    ]
}

/// Copy `src` over the frame's debug line list; the stored buffers keep their
/// capacity, so a same-sized push after the first allocates nothing.
pub(super) fn stage_debug_lines(dst: &mut LineBatch, src: &LineBatch) {
    dst.copy_from(src);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug_draw::{DebugRecorder, LineBatch};

    /// A console box: the twelve edges of one world AABB in a single group.
    fn box_batch() -> LineBatch {
        let mut rec = DebugRecorder::default();
        let g = rec.group("console");
        rec.push_box(g, [1.0, 2.0, 3.0], [0.5, 0.5, 0.5], [0.2, 1.0, 0.2, 1.0])
            .expect("a finite box lands");
        LineBatch::build(rec.frames()).expect("finite lines build")
    }

    #[test]
    fn debug_draw_presenter_stage_stores_a_box_and_an_empty_batch_clears_it() {
        let boxed = box_batch();
        let mut stored = LineBatch::default();
        stage_debug_lines(&mut stored, &boxed);
        assert_eq!(stored.line_count(), 12, "a box lands twelve edges");
        assert_eq!(stored.groups().len(), 1, "one group per recorder group");
        assert_eq!(stored.groups()[0].name, "console");
        assert_eq!(stored.groups()[0].line_count(), 12);
        assert_eq!(stored.verts(), boxed.verts(), "same bytes, copied once");

        let kept = stored.verts().as_ptr();
        stage_debug_lines(&mut stored, &boxed);
        assert_eq!(
            stored.verts().as_ptr(),
            kept,
            "a same-sized push reuses the stored buffer"
        );

        stage_debug_lines(&mut stored, &LineBatch::default());
        assert!(stored.is_empty(), "an empty batch clears the list");
        assert!(stored.groups().is_empty(), "no stale group span survives");
    }

    #[test]
    fn debug_draw_pass_draws_after_the_blit_in_both_frame_paths() {
        // Structural: one after-blit owner per path, each after its blit.
        let live = include_str!("live.rs");
        let submit = include_str!("submit.rs");
        let live_at = live.find("encode_debug_lines").expect("live draws lines");
        let live_blit = live.find("encode_blit").expect("live blits");
        assert!(live_blit < live_at, "lines draw after the live blit");
        let capture_at = submit
            .find("encode_debug_lines")
            .expect("capture draws lines");
        let capture_blit = submit.find("encode_blit(").expect("capture blits");
        assert!(
            capture_blit < capture_at,
            "lines draw after the capture blit"
        );
    }
}
