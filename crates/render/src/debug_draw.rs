//! Debug line sink contract (RL58 slice 1): line groups with a depth bias, a
//! per-frame recorder a caller fills after the tick, and the pure build of line
//! vertices a line pass consumes. Std-only; no GPU pass and no Rapier backend
//! here. Consumer: the RL58 line pass reads `line_verts`, the Rapier backend and
//! the arena binary call `DebugDraw::collect` (later slices).

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
        let made = LineGroup {
            name,
            depth_bias_permille: 0,
            lines: Vec::new(),
        };
        self.groups.push(made);
        self.groups.len() - 1
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

/// Build this frame's line verts: two per line, groups in declaration order,
/// lines in push order, colour copied to both verts, in the `trail_strip_verts`
/// `(position, colour)` shape; the per-group bias is never baked into a
/// position. A stored non-finite line refuses and writes nothing; `out` needs no
/// new allocation while it has spare capacity. Returns the verts written.
pub fn line_verts(
    groups: &[LineGroup],
    out: &mut Vec<([f32; 3], [f32; 4])>,
) -> Result<usize, DebugError> {
    for group in groups {
        for line in &group.lines {
            check_line(line)?;
        }
    }
    let start = out.len();
    for group in groups {
        for line in &group.lines {
            out.push((line.a_m, line.colour_rgba));
            out.push((line.b_m, line.colour_rgba));
        }
    }
    Ok(out.len() - start)
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
}
