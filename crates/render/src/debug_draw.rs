//! Debug line sink contract (RL58): line groups with a depth bias, a per-frame
//! recorder a caller fills after the tick, the pure build of line vertices, and
//! the one-buffer `LineBatch` the live pass draws. Std-only; no GPU pass and
//! no Rapier backend here. The Rapier backend lives in `engine_physics::rapier`;
//! the live sink is one call, `Presenter::push_debug_lines(&batch)`.

/// One debug line segment: endpoints in meters plus a linear RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugLine {
    /// Start point in meters.
    pub a_m: [f32; 3],
    /// End point in meters.
    pub b_m: [f32; 3],
    /// Linear RGBA, each channel 0..=1.
    pub colour_rgba: [f32; 4],
}

/// One named line group; its depth bias rides with the group, never baked into
/// a position.
#[derive(Clone, Debug, PartialEq)]
pub struct LineGroup {
    /// Group name, stable for the run so the recorder finds it again.
    pub name: &'static str,
    /// Depth bias in thousandths of the depth range; negative sits closer.
    pub depth_bias_permille: i32,
    lines: Vec<DebugLine>,
}

impl LineGroup {
    /// An empty group with a stable name and depth bias; callers that do not
    /// own a recorder build their groups this way.
    pub fn new(name: &'static str, depth_bias_permille: i32) -> Self {
        Self {
            name,
            depth_bias_permille,
            lines: Vec::new(),
        }
    }

    /// The group's lines in push order.
    pub fn lines(&self) -> &[DebugLine] {
        &self.lines
    }

    /// Append one line; a non-finite endpoint or colour is refused and nothing
    /// is written.
    pub fn push(&mut self, line: DebugLine) -> Result<(), DebugError> {
        check_line(&line)?;
        self.lines.push(line);
        Ok(())
    }
}

/// The debug-draw contract: both sinks call `collect` the same way.
pub trait DebugDraw {
    /// Push this frame's lines into `out`.
    fn collect(&self, out: &mut DebugRecorder);
}

/// Per-frame recorder: groups are made once per run, lines cleared each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugRecorder {
    groups: Vec<LineGroup>,
}

impl DebugRecorder {
    /// Index of the group named `name`, created on first sight with bias 0.
    pub fn group(&mut self, name: &'static str) -> usize {
        if let Some(i) = self.groups.iter().position(|g| g.name == name) {
            return i;
        }
        self.groups.push(LineGroup::new(name, 0));
        self.groups.len() - 1
    }

    /// Set the depth bias of a group made by [`DebugRecorder::group`]; negative
    /// sits closer. An unknown index is refused and nothing changes.
    pub fn set_depth_bias(
        &mut self,
        group: usize,
        depth_bias_permille: i32,
    ) -> Result<(), DebugError> {
        let g = self
            .groups
            .get_mut(group)
            .ok_or(DebugError::UnknownGroup(group))?;
        g.depth_bias_permille = depth_bias_permille;
        Ok(())
    }

    /// Push one line into an existing group; an unknown index or a non-finite
    /// value is refused and nothing is written.
    pub fn push_line(
        &mut self,
        group: usize,
        a_m: [f32; 3],
        b_m: [f32; 3],
        colour_rgba: [f32; 4],
    ) -> Result<(), DebugError> {
        let g = self.groups.get_mut(group);
        let g = g.ok_or(DebugError::UnknownGroup(group))?;
        g.push(DebugLine {
            a_m,
            b_m,
            colour_rgba,
        })
    }

    /// Push the twelve edges of a world-space axis-aligned box into an
    /// existing group in one go, all in `colour_rgba`. A non-finite centre,
    /// half extent or colour is refused before the first line and writes
    /// nothing.
    pub fn push_box(
        &mut self,
        group: usize,
        centre_m: [f32; 3],
        half_m: [f32; 3],
        colour_rgba: [f32; 4],
    ) -> Result<(), DebugError> {
        let g = self
            .groups
            .get_mut(group)
            .ok_or(DebugError::UnknownGroup(group))?;
        if centre_m.iter().any(|c| !c.is_finite()) {
            return Err(DebugError::BadPoint("centre_m"));
        }
        if half_m.iter().any(|c| !c.is_finite()) {
            return Err(DebugError::BadPoint("half_m"));
        }
        if colour_rgba.iter().any(|c| !c.is_finite()) {
            return Err(DebugError::BadPoint("colour_rgba"));
        }
        // Bit 0 is x, bit 1 y, bit 2 z; the twelve edges each flip one bit.
        const EDGES: [(usize, usize); 12] = [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];
        let corner = |bits: usize| -> [f32; 3] {
            let axis = |i: usize| {
                if bits & (1 << i) == 0 {
                    centre_m[i] - half_m[i]
                } else {
                    centre_m[i] + half_m[i]
                }
            };
            [axis(0), axis(1), axis(2)]
        };
        for (a, b) in EDGES {
            g.push(DebugLine {
                a_m: corner(a),
                b_m: corner(b),
                colour_rgba,
            })?;
        }
        Ok(())
    }

    /// Every group in creation order.
    pub fn frames(&self) -> &[LineGroup] {
        &self.groups
    }

    /// Total lines across all groups.
    pub fn line_count(&self) -> usize {
        self.groups.iter().map(|g| g.lines.len()).sum()
    }

    /// Drop every line but keep the group order and names.
    pub fn clear(&mut self) {
        for g in &mut self.groups {
            g.lines.clear();
        }
    }
}

/// Typed debug-draw failures; every variant names what to fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugError {
    /// An endpoint or colour channel is not finite; names the field.
    BadPoint(&'static str),
    /// A group index that was never created by `DebugRecorder::group`.
    UnknownGroup(usize),
}

impl core::fmt::Display for DebugError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadPoint(what) => write!(f, "debug draw: {what} is not finite"),
            Self::UnknownGroup(index) => write!(f, "debug draw: group {index} missing"),
        }
    }
}

/// Refuse a non-finite endpoint or colour, naming the first bad field.
fn check_line(line: &DebugLine) -> Result<(), DebugError> {
    for (v, name) in [(&line.a_m, "a_m"), (&line.b_m, "b_m")] {
        if v.iter().any(|c| !c.is_finite()) {
            return Err(DebugError::BadPoint(name));
        }
    }
    if line.colour_rgba.iter().all(|c| c.is_finite()) {
        return Ok(());
    }
    Err(DebugError::BadPoint("colour_rgba"))
}

/// Refuse the first stored line that is not finite.
fn check_groups(groups: &[LineGroup]) -> Result<(), DebugError> {
    for group in groups {
        for line in &group.lines {
            check_line(line)?;
        }
    }
    Ok(())
}

/// Append every line's two verts; the caller has checked the groups.
fn fill_verts(groups: &[LineGroup], out: &mut Vec<([f32; 3], [f32; 4])>) {
    for group in groups {
        for line in &group.lines {
            out.push((line.a_m, line.colour_rgba));
            out.push((line.b_m, line.colour_rgba));
        }
    }
}

/// Build this frame's line verts: two per line, groups in declaration order,
/// lines in push order, colour copied to both verts, in the `trail_strip_verts`
/// `(position, colour)` shape; the per-group bias is never baked into a
/// position. A stored non-finite line refuses and writes nothing; `out` needs no
/// new allocation while it has spare capacity. Returns the verts written.
pub fn line_verts(
    groups: &[LineGroup],
    out: &mut Vec<([f32; 3], [f32; 4])>,
) -> Result<usize, DebugError> {
    check_groups(groups)?;
    let start = out.len();
    fill_verts(groups, out);
    Ok(out.len() - start)
}

/// One group's slice of a `LineBatch`: its name, depth bias and vertex range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchGroup {
    /// Group name, stable for the run.
    pub name: &'static str,
    /// Depth bias in thousandths of the depth range; negative sits closer.
    pub depth_bias_permille: i32,
    /// Index of the group's first vertex in [`LineBatch::verts`].
    pub first_vert: usize,
    /// Vertices in the group; two per line.
    pub vert_count: usize,
}

impl BatchGroup {
    /// Lines in this group's span.
    pub fn line_count(self) -> usize {
        self.vert_count / 2
    }
}

/// One frame's whole line list: a single vertex buffer plus the group spans a
/// live pass draws with their own depth bias. Build once after the tick and
/// hand it to the pass; `build_into` reuses its buffers on the next frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineBatch {
    verts: Vec<([f32; 3], [f32; 4])>,
    groups: Vec<BatchGroup>,
}

impl LineBatch {
    /// Build a fresh batch; a stored non-finite line refuses and writes nothing.
    pub fn build(groups: &[LineGroup]) -> Result<Self, DebugError> {
        let mut batch = Self::default();
        batch.build_into(groups)?;
        Ok(batch)
    }

    /// Rebuild in place. On a refusal the previous batch is left untouched, so
    /// the pass can keep drawing last-good lines. No allocation once `self` has
    /// the capacity for this frame's groups and verts.
    pub fn build_into(&mut self, groups: &[LineGroup]) -> Result<(), DebugError> {
        check_groups(groups)?;
        self.verts.clear();
        self.groups.clear();
        fill_verts(groups, &mut self.verts);
        let mut first_vert = 0;
        for group in groups {
            let vert_count = group.lines.len() * 2;
            self.groups.push(BatchGroup {
                name: group.name,
                depth_bias_permille: group.depth_bias_permille,
                first_vert,
                vert_count,
            });
            first_vert += vert_count;
        }
        Ok(())
    }

    /// Every line vertex in group order, in the `(position, colour)` shape.
    pub fn verts(&self) -> &[([f32; 3], [f32; 4])] {
        &self.verts
    }

    /// Copy another batch's bytes into this one, keeping both buffers'
    /// capacity (`Vec::clone_from`), so a per-frame sink reuses them.
    pub fn copy_from(&mut self, source: &LineBatch) {
        self.verts.clone_from(&source.verts);
        self.groups.clone_from(&source.groups);
    }

    /// Group spans in recorder order.
    pub fn groups(&self) -> &[BatchGroup] {
        &self.groups
    }

    /// Total lines across all groups.
    pub fn line_count(&self) -> usize {
        self.verts.len() / 2
    }

    /// True when the batch holds no line.
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty()
    }
}

/// Thickness of one debug line in HUD space: a fraction of the frame height,
/// so a horizontal and a vertical line draw the same pixels wide. Thin at
/// 1080p (about two pixels) and still visible on a 180-row capture.
pub const DEBUG_LINE_WIDTH: f32 = 0.012;

/// Most line quads one frame stages: the pass fills a fixed array and stops at
/// a whole quad, so a huge batch clips instead of growing a buffer.
pub const DEBUG_LINE_MAX_QUADS: usize = 256;

/// One screen-space line quad: four corners in 0..1 overlay space (the
/// after-blit shader's space) plus the segment's colour. Pure data; the live
/// pass turns it into four [`QuadVertex`](crate::overlay::QuadVertex)s.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LineQuad {
    /// Corners round the segment in 0..1 space: a-left, b-left, a-right,
    /// b-right, so the triangle list draws (0, 1, 2) and (2, 1, 3).
    pub corners: [[f32; 2]; 4],
    /// Linear RGBA of the segment.
    pub colour_rgba: [f32; 4],
}

/// HUD space x/y in -1..=1 (y up) to the overlay shader's 0..1 space. One
/// unit in either axis is one frame height in pixels, so a width stays square
/// on screen.
fn hud_to_quad(p: [f32; 3]) -> [f32; 2] {
    [p[0] * 0.5 + 0.5, p[1] * 0.5 + 0.5]
}

/// Build this frame's line quads from a batch: one thin quad per stored
/// segment, groups in recorder order and lines in push order, each in its own
/// colour, `width` being a fraction of the frame height. A non-finite or
/// negative width falls back to [`DEBUG_LINE_WIDTH`]. Stops at a whole quad
/// when `out` is full and returns the quads written. Pure; the same batch and
/// width give the same corners twice; no heap work.
pub fn line_quads(batch: &LineBatch, width: f32, out: &mut [LineQuad]) -> usize {
    let w = if width.is_finite() && width >= 0.0 {
        width
    } else {
        DEBUG_LINE_WIDTH
    };
    let half = w * 0.5;
    let verts = batch.verts();
    let mut written = 0usize;
    for group in batch.groups() {
        let start = group.first_vert.min(verts.len());
        let end = start.saturating_add(group.vert_count).min(verts.len());
        for line in verts[start..end].chunks_exact(2) {
            if written >= out.len() {
                return written;
            }
            let a = hud_to_quad(line[0].0);
            let b = hud_to_quad(line[1].0);
            let d = [b[0] - a[0], b[1] - a[1]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            // The perpendicular runs half a width each way; a degenerate
            // segment keeps a flat quad so the build never divides by zero.
            let perp = if len > f32::EPSILON {
                [-d[1] / len * half, d[0] / len * half]
            } else {
                [half, 0.0]
            };
            out[written] = LineQuad {
                corners: [
                    [a[0] - perp[0], a[1] - perp[1]],
                    [b[0] - perp[0], b[1] - perp[1]],
                    [a[0] + perp[0], a[1] + perp[1]],
                    [b[0] + perp[0], b[1] + perp[1]],
                ],
                colour_rgba: line[0].1,
            };
            written += 1;
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: [f32; 3], b: [f32; 3]) -> DebugLine {
        DebugLine {
            a_m: a,
            b_m: b,
            colour_rgba: [1.0, 0.5, 0.25, 1.0],
        }
    }

    fn push(rec: &mut DebugRecorder, name: &'static str, x: f32) -> usize {
        let g = rec.group(name);
        let made = rec.push_line(g, [x, 0.0, 0.0], [x, 1.0, 0.0], [1.0; 4]);
        assert!(made.is_ok(), "a finite line lands");
        g
    }

    #[test]
    fn debug_draw_four_groups_write_every_line_once() {
        let mut rec = DebugRecorder::default();
        let names = ["colliders", "joints", "sweeps", "nav"];
        for (i, name) in names.iter().enumerate() {
            assert_eq!(rec.group(name), i);
            for _ in 0..3 {
                push(&mut rec, name, 0.0);
            }
            assert_eq!(rec.frames()[i].lines().len(), 3);
        }
        assert_eq!(rec.frames().len(), 4, "one group per name");
        assert_eq!(rec.line_count(), 12, "every line counted once");
        assert_eq!(rec.group("joints"), 1, "a repeat name keeps its index");
        let ((), probe) = engine_core::alloc_probe::measure(|| {
            push(&mut rec, "joints", 0.0);
        });
        assert_eq!(probe.calls, 0, "an existing group never grows: {probe:?}");
    }

    #[test]
    fn debug_draw_refuses_a_nonfinite_point_and_writes_nothing() {
        let mut rec = DebugRecorder::default();
        let g = rec.group("colliders");
        let p = [0.0, 0.0, 0.0];
        let bad = [
            ([f32::NAN, 0.0, 0.0], p, [1.0; 4], "a_m"),
            (p, [0.0, f32::INFINITY, 0.0], [1.0; 4], "b_m"),
            (p, p, [1.0, 1.0, 1.0, f32::NAN], "colour_rgba"),
        ];
        for (a, b, c, field) in bad {
            let got = rec.push_line(g, a, b, c);
            assert_eq!(got, Err(DebugError::BadPoint(field)));
        }
        assert_eq!(rec.line_count(), 0, "a refusal writes nothing");
        let unknown = rec.push_line(9, p, p, [1.0; 4]);
        assert!(matches!(unknown, Err(DebugError::UnknownGroup(9))));
        assert!(DebugError::UnknownGroup(9).to_string().contains('9'));
        let mut one = LineGroup {
            name: "x",
            depth_bias_permille: -1,
            lines: Vec::new(),
        };
        assert!(one.push(seg(p, p)).is_ok());
        assert!(one.push(seg([f32::NAN; 3], p)).is_err());
        assert_eq!(one.lines().len(), 1, "the refused line is not stored");
    }

    #[test]
    fn debug_draw_clear_keeps_group_order() {
        let mut rec = DebugRecorder::default();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            assert_eq!(rec.group(name), i);
            push(&mut rec, name, 0.0);
        }
        rec.clear();
        assert_eq!(rec.line_count(), 0, "clear drops every line");
        let kept: Vec<_> = rec.frames().iter().map(|g| g.name).collect();
        assert_eq!(kept, vec!["a", "b", "c"], "order survives the clear");
        assert_eq!(rec.group("c"), 2, "a cleared group keeps its index");
        assert_eq!(rec.frames().len(), 3, "no group is recreated");
    }

    #[test]
    fn debug_draw_line_verts_replay_same_bytes_twice() {
        let mut rec = DebugRecorder::default();
        push(&mut rec, "colliders", 0.0);
        push(&mut rec, "nav", 2.0);
        let mut first = Vec::new();
        assert_eq!(line_verts(rec.frames(), &mut first), Ok(4));
        assert_eq!(first[0].0, [0.0, 0.0, 0.0], "a leads its line");
        assert_eq!(first[1].0, [0.0, 1.0, 0.0], "b follows its a");
        assert_eq!(first[2].0, [2.0, 0.0, 0.0], "group order leads");
        assert_eq!(first[3].0, [2.0, 1.0, 0.0], "second group follows");
        assert_eq!(first[1].1, first[0].1, "both verts share the colour");
        let mut second = Vec::with_capacity(4);
        assert_eq!(line_verts(rec.frames(), &mut second), Ok(4));
        assert_eq!(first, second, "same groups give the same bytes x2");
        let ((), probe) = engine_core::alloc_probe::measure(|| {
            second.clear();
            line_verts(rec.frames(), &mut second).expect("capacity");
        });
        assert_eq!(probe.calls, 0, "a reserved out never allocates");
    }

    #[test]
    fn debug_draw_bias_is_carried_not_baked() {
        let a = seg([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]);
        let group = |bias: i32| LineGroup {
            name: "grid",
            depth_bias_permille: bias,
            lines: vec![a],
        };
        let (near, far) = ([group(-8)], [group(8)]);
        let (mut lo, mut hi) = (Vec::new(), Vec::new());
        assert_eq!(line_verts(&near, &mut lo), Ok(2));
        assert_eq!(line_verts(&far, &mut hi), Ok(2));
        assert_eq!(lo, hi, "moving the bias moves no vertex");
        let moved = near[0].depth_bias_permille != far[0].depth_bias_permille;
        assert!(moved, "the bias really moved");
        assert_eq!(lo[0].0, [0.0, 0.0, 0.0], "positions stay raw meters");
        assert_eq!(lo[1].0, [1.0, 2.0, 3.0], "endpoints stay raw meters");
    }

    #[test]
    fn debug_draw_group_bias_can_be_set_after_creation() {
        let mut rec = DebugRecorder::default();
        let g = rec.group("sweeps");
        assert_eq!(rec.frames()[g].depth_bias_permille, 0, "fresh group");
        rec.set_depth_bias(g, -3).expect("known group");
        assert_eq!(rec.frames()[g].depth_bias_permille, -3);
        assert_eq!(rec.set_depth_bias(7, 0), Err(DebugError::UnknownGroup(7)));
    }

    /// The four groups the capture leg names, each a dozen screen-space lines.
    fn raster_groups() -> Vec<LineGroup> {
        const SPECS: [(&str, i32); 4] =
            [("colliders", -8), ("joints", -4), ("sweeps", 0), ("nav", 4)];
        let mut groups = Vec::new();
        for (row, (name, bias)) in SPECS.iter().enumerate() {
            let mut group = LineGroup::new(name, *bias);
            for i in 0..12 {
                let t = -0.9 + i as f32 * 0.16;
                let (a, b) = match row {
                    0 => ([t, -0.9, 0.0], [t, 0.9, 0.0]),
                    1 => ([-0.9, t, 0.0], [0.9, t, 0.0]),
                    2 => ([-0.9, t, 0.0], [t, 0.9, 0.0]),
                    _ => ([t, -0.2, 0.0], [t + 0.1, 0.2, 0.0]),
                };
                group.push(seg(a, b)).expect("finite line");
            }
            groups.push(group);
        }
        groups
    }

    const RASTER_W: usize = 320;
    const RASTER_H: usize = 180;

    fn raster_point(p: [f32; 3]) -> (i32, i32) {
        let x = (p[0].clamp(-1.0, 1.0) + 1.0) * 0.5 * (RASTER_W - 1) as f32;
        let y = (p[1].clamp(-1.0, 1.0) + 1.0) * 0.5 * (RASTER_H - 1) as f32;
        (x.round() as i32, y.round() as i32)
    }

    /// CPU stand-in for the live line pass: project x/y, depth at 0.5 plus the
    /// group bias, and light every pixel the line wins.
    fn rasterise(batch: &LineBatch, plane_depth: Option<f32>) -> Vec<bool> {
        let mut depth = vec![1.0f32; RASTER_W * RASTER_H];
        let mut lit = vec![false; RASTER_W * RASTER_H];
        if let Some(d) = plane_depth {
            depth.fill(d);
        }
        for group in batch.groups() {
            let z = 0.5 + group.depth_bias_permille as f32 * 0.0005;
            let verts = &batch.verts()[group.first_vert..group.first_vert + group.vert_count];
            for line in verts.chunks_exact(2) {
                let (x0, y0) = raster_point(line[0].0);
                let (x1, y1) = raster_point(line[1].0);
                let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
                for step in 0..=steps {
                    let x = x0 + (x1 - x0) * step / steps;
                    let y = y0 + (y1 - y0) * step / steps;
                    let i = y as usize * RASTER_W + x as usize;
                    if z <= depth[i] {
                        depth[i] = z;
                        lit[i] = true;
                    }
                }
            }
        }
        lit
    }

    fn lit_pixels(lit: &[bool]) -> usize {
        lit.iter().filter(|p| **p).count()
    }

    #[test]
    fn debug_draw_batch_four_groups_carry_spans_and_bias() {
        let groups = raster_groups();
        let batch = LineBatch::build(&groups).expect("batch");
        assert_eq!(batch.groups().len(), 4, "four groups in order");
        assert_eq!(batch.verts().len(), 4 * 12 * 2, "two verts per line");
        assert_eq!(batch.line_count(), 48);
        let mut first = 0;
        for (row, group) in batch.groups().iter().zip(&groups) {
            assert_eq!(row.name, group.name);
            assert_eq!(row.depth_bias_permille, group.depth_bias_permille);
            assert_eq!(row.first_vert, first, "spans are contiguous");
            assert_eq!(row.vert_count, group.lines().len() * 2);
            assert_eq!(row.line_count(), 12);
            first += row.vert_count;
        }
        assert_eq!(first, batch.verts().len(), "spans tile the batch");
        let mut reused = LineBatch::default();
        reused.build_into(&groups).expect("first fill");
        let ((), probe) = engine_core::alloc_probe::measure(|| {
            reused.build_into(&groups).expect("reuse");
        });
        assert_eq!(
            probe.calls, 0,
            "a reserved batch never allocates: {probe:?}"
        );
        assert_eq!(reused, batch, "same groups give the same batch x2");
    }

    #[test]
    fn debug_draw_batch_refusal_keeps_the_last_good_batch() {
        let groups = raster_groups();
        let mut batch = LineBatch::build(&groups).expect("batch");
        let good = batch.clone();
        let mut bad = LineGroup::new("colliders", -8);
        bad.lines.push(seg([0.0; 3], [f32::NAN, 1.0, 0.0]));
        let refused = batch.build_into(&[bad]);
        assert_eq!(refused, Err(DebugError::BadPoint("b_m")));
        assert_eq!(batch, good, "a refused build writes nothing");
    }

    #[test]
    fn debug_draw_batch_cpu_raster_lights_1000_pixels_across_four_groups() {
        let batch = LineBatch::build(&raster_groups()).expect("batch");
        let lit = lit_pixels(&rasterise(&batch, None));
        assert!(lit >= 1000, "four groups lit {lit} pixels, need >= 1000");
        let off = lit_pixels(&rasterise(&LineBatch::default(), None));
        assert_eq!(off, 0, "groups off leaves the beauty frame untouched");
    }

    #[test]
    fn debug_draw_batch_depth_bias_orders_two_groups() {
        let line = |bias: i32, y: f32| {
            let mut group = LineGroup::new("grid", bias);
            group
                .push(seg([-1.0, y, 0.0], [1.0, y, 0.0]))
                .expect("finite line");
            [group]
        };
        let near = LineBatch::build(&line(-8, -0.5)).expect("near");
        let far = LineBatch::build(&line(8, 0.5)).expect("far");
        let plane = Some(0.5);
        assert!(
            lit_pixels(&rasterise(&near, plane)) > 300,
            "a negative bias sits in front of the beauty depth"
        );
        assert_eq!(
            lit_pixels(&rasterise(&far, plane)),
            0,
            "a positive bias sits behind it"
        );
    }

    #[test]
    fn debug_draw_box_pushes_twelve_edges_once() {
        let mut rec = DebugRecorder::default();
        let g = rec.group("colliders");
        let centre = [3.0, 4.0, 5.0];
        let half = [0.5, 1.0, 2.0];
        rec.push_box(g, centre, half, [1.0, 0.0, 0.0, 1.0])
            .expect("a finite box lands");
        let lines = rec.frames()[g].lines();
        assert_eq!(lines.len(), 12, "a box is twelve edges");
        let corner = |bits: usize| {
            [
                centre[0] + if bits & 1 == 0 { -half[0] } else { half[0] },
                centre[1] + if bits & 2 == 0 { -half[1] } else { half[1] },
                centre[2] + if bits & 4 == 0 { -half[2] } else { half[2] },
            ]
        };
        let mut seen = [0u8; 8];
        let mut pairs = Vec::new();
        for line in lines {
            let a = (0..8).find(|i| corner(*i) == line.a_m).expect("a corner");
            let b = (0..8).find(|i| corner(*i) == line.b_m).expect("b corner");
            assert_ne!(a, b, "an edge joins two corners");
            seen[a] += 1;
            seen[b] += 1;
            pairs.push(if a < b { (a, b) } else { (b, a) });
        }
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), 12, "every edge is distinct");
        assert_eq!(seen, [3; 8], "each corner joins three edges");
        assert_eq!(lines[0].colour_rgba, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn debug_draw_box_refuses_a_nonfinite_field_and_writes_nothing() {
        let mut rec = DebugRecorder::default();
        let g = rec.group("sweeps");
        let p = [0.0; 3];
        let h = [1.0; 3];
        let red = [1.0; 4];
        let bad = [
            ([f32::NAN, 0.0, 0.0], h, red, "centre_m"),
            (p, [1.0, f32::INFINITY, 1.0], red, "half_m"),
            (p, h, [1.0, 1.0, 1.0, f32::NAN], "colour_rgba"),
        ];
        for (centre, half, colour, field) in bad {
            let got = rec.push_box(g, centre, half, colour);
            assert_eq!(got, Err(DebugError::BadPoint(field)));
        }
        assert_eq!(rec.push_box(9, p, h, red), Err(DebugError::UnknownGroup(9)));
        assert_eq!(rec.line_count(), 0, "a refusal writes no edge");
    }

    /// One group holding a horizontal and a vertical segment through the
    /// middle of HUD space, both in the same colour.
    fn cross_batch() -> LineBatch {
        let mut group = LineGroup::new("cross", -4);
        group
            .push(seg([-0.5, 0.0, 0.0], [0.5, 0.0, 0.0]))
            .expect("finite line");
        group
            .push(seg([0.0, -0.5, 0.0], [0.0, 0.5, 0.0]))
            .expect("finite line");
        LineBatch::build(&[group]).expect("finite batch")
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn debug_draw_line_quads_make_a_thin_quad_per_segment() {
        let batch = cross_batch();
        let mut quads = [LineQuad::default(); 8];
        let made = line_quads(&batch, 0.02, &mut quads);
        assert_eq!(made, 2, "one quad per segment");
        let horizontal = quads[0];
        // HUD x/y -0.5..0.5 lands at the frame middle; a 0.02 width sits
        // half either side, so a horizontal line is 1 unit long and 0.02 tall.
        assert!(close(horizontal.corners[0][0], 0.25));
        assert!(close(horizontal.corners[0][1], 0.49));
        assert!(close(horizontal.corners[1][0], 0.75));
        assert!(close(horizontal.corners[1][1], 0.49));
        assert!(close(horizontal.corners[2][0], 0.25));
        assert!(close(horizontal.corners[2][1], 0.51));
        assert!(close(horizontal.corners[3][0], 0.75));
        assert!(close(horizontal.corners[3][1], 0.51));
        let tall = 0.02;
        let wide = horizontal.corners[2][1] - horizontal.corners[0][1];
        assert!(close(tall, wide), "the width is the same on both axes");
        let vertical = quads[1];
        let across = (vertical.corners[2][0] - vertical.corners[0][0]).abs();
        assert!(
            close(across, wide),
            "a vertical line draws the same thickness as a horizontal one"
        );
        assert_eq!(vertical.colour_rgba, [1.0, 0.5, 0.25, 1.0], "colour copied");
        let mut again = [LineQuad::default(); 8];
        assert_eq!(line_quads(&batch, 0.02, &mut again), made);
        assert_eq!(quads, again, "same batch gives the same quads twice");
    }

    #[test]
    fn debug_draw_line_quads_cap_at_a_whole_quad_and_default_a_bad_width() {
        let batch = LineBatch::build(&raster_groups()).expect("batch");
        let mut roomy = [LineQuad::default(); DEBUG_LINE_MAX_QUADS];
        assert_eq!(line_quads(&batch, DEBUG_LINE_WIDTH, &mut roomy), 48);
        let mut short = [LineQuad::default(); 5];
        assert_eq!(line_quads(&batch, DEBUG_LINE_WIDTH, &mut short), 5);
        assert_eq!(line_quads(&batch, DEBUG_LINE_WIDTH, &mut []), 0, "no room");
        let mut fallback = [LineQuad::default(); 8];
        let mut sentinel = [LineQuad::default(); 8];
        let one_line = cross_batch();
        assert_eq!(line_quads(&one_line, f32::NAN, &mut fallback), 2);
        assert_eq!(line_quads(&one_line, DEBUG_LINE_WIDTH, &mut sentinel), 2);
        assert_eq!(fallback, sentinel, "a bad width reads as the default");
        assert_eq!(line_quads(&one_line, -3.0, &mut fallback), 2);
        assert_eq!(fallback, sentinel, "a negative width reads as the default");
        let mut zero = [LineQuad::default(); 8];
        assert_eq!(
            line_quads(&one_line, 0.0, &mut zero),
            2,
            "zero still counts"
        );
        assert!(
            close(zero[0].corners[0][1], zero[0].corners[2][1]),
            "a zero width collapses the quad flat"
        );
    }

    /// Pixels one triangle-list quad covers, sampling pixel centres.
    fn quad_pixels(quads: &[LineQuad]) -> usize {
        let mut lit = vec![false; RASTER_W * RASTER_H];
        let to_px = |p: [f32; 2]| [p[0] * (RASTER_W - 1) as f32, p[1] * (RASTER_H - 1) as f32];
        let edge = |a: [f32; 2], b: [f32; 2], p: [f32; 2]| {
            (p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0])
        };
        for quad in quads {
            let corner = [
                to_px(quad.corners[0]),
                to_px(quad.corners[1]),
                to_px(quad.corners[2]),
                to_px(quad.corners[3]),
            ];
            for tri in [[0usize, 1, 2], [2, 1, 3]] {
                let (a, b, c) = (corner[tri[0]], corner[tri[1]], corner[tri[2]]);
                let lo_x = a[0].min(b[0]).min(c[0]).floor().max(0.0);
                let hi_x = a[0].max(b[0]).max(c[0]).ceil().min((RASTER_W - 1) as f32);
                let lo_y = a[1].min(b[1]).min(c[1]).floor().max(0.0);
                let hi_y = a[1].max(b[1]).max(c[1]).ceil().min((RASTER_H - 1) as f32);
                for y in lo_y as usize..=hi_y as usize {
                    for x in lo_x as usize..=hi_x as usize {
                        let p = [x as f32, y as f32];
                        let (e0, e1, e2) = (edge(a, b, p), edge(b, c, p), edge(c, a, p));
                        let inside = (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0)
                            || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0);
                        if inside {
                            lit[y * RASTER_W + x] = true;
                        }
                    }
                }
            }
        }
        lit.iter().filter(|p| **p).count()
    }

    #[test]
    fn debug_draw_line_quads_four_groups_cover_1000_pixels() {
        let batch = LineBatch::build(&raster_groups()).expect("batch");
        let mut quads = [LineQuad::default(); DEBUG_LINE_MAX_QUADS];
        let made = line_quads(&batch, DEBUG_LINE_WIDTH, &mut quads);
        assert_eq!(made, 48, "four groups of a dozen lines");
        let lit = quad_pixels(&quads[..made]);
        assert!(lit >= 1000, "four groups cover {lit} pixels, need >= 1000");
        assert_eq!(quad_pixels(&[]), 0, "no quad lights no pixel");
    }
}
