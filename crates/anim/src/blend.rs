//! blend: blend graph as data + fixed-tick state machine over baked clips.
use super::clip::{clamp01, sample_all, sample_track, slip, Cursor, QClip, TICK_DT};
use engine_core::Fnv64;
pub fn lerp3(a: [f32; 3], b: [f32; 3], f: f32) -> [f32; 3] {
    let w = clamp01(f);
    [
        a[0] + (b[0] - a[0]) * w,
        a[1] + (b[1] - a[1]) * w,
        a[2] + (b[2] - a[2]) * w,
    ]
}
#[derive(Clone, Copy, Debug)]
pub enum Node {
    Clip(u32),
    Mix(u32, u32),
    Add(u32, u32),
}
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
}
impl Graph {
    /// Eval one node into `out` reusing caller `scratch`, never allocating.
    ///
    /// Caller owns `scratch` and reuses it across ticks; size at least
    /// `nodes.len() * out.len()` poses covers the deepest chain. Short
    /// scratch zeroes `out` and refuses. Units: poses are joint `[f32; 3]`
    /// triples. Determinism: same graph plus same clips give same poses.
    #[expect(
        clippy::too_many_arguments,
        reason = "alloc-free eval keeps 8 args until a params struct lands as a follow-up"
    )]
    pub fn eval(
        &self,
        clips: &[QClip],
        curs: &mut [Cursor],
        node: usize,
        t: f32,
        mix: f32,
        out: &mut [[f32; 3]],
        scratch: &mut [[f32; 3]],
    ) -> bool {
        match self.nodes.get(node).copied() {
            Some(Node::Clip(i)) => {
                let ci = usize::try_from(i).unwrap_or(usize::MAX);
                if ci >= clips.len() || ci >= curs.len() {
                    for p in out.iter_mut() {
                        *p = [0.0; 3];
                    }
                    return false;
                }
                sample_all(&clips[ci], t, &mut curs[ci], out);
                true
            }
            Some(Node::Mix(a, b)) => {
                let a = usize::try_from(a).unwrap_or(usize::MAX);
                let b = usize::try_from(b).unwrap_or(usize::MAX);
                let n = out.len();
                if scratch.len() < n {
                    for p in out.iter_mut() {
                        *p = [0.0; 3];
                    }
                    return false;
                }
                let (tmp, rest) = scratch.split_at_mut(n);
                if !self.eval(clips, curs, a, t, mix, out, rest) {
                    return false;
                }
                if !self.eval(clips, curs, b, t, mix, tmp, rest) {
                    return false;
                }
                let w = clamp01(mix);
                for (slot, other) in out.iter_mut().zip(tmp.iter()) {
                    *slot = lerp3(*slot, *other, w);
                }
                true
            }
            Some(Node::Add(a, b)) => {
                let a = usize::try_from(a).unwrap_or(usize::MAX);
                let b = usize::try_from(b).unwrap_or(usize::MAX);
                let n = out.len();
                if scratch.len() < n {
                    for p in out.iter_mut() {
                        *p = [0.0; 3];
                    }
                    return false;
                }
                let (tmp, rest) = scratch.split_at_mut(n);
                if !self.eval(clips, curs, a, t, mix, out, rest) {
                    return false;
                }
                if !self.eval(clips, curs, b, t, mix, tmp, rest) {
                    return false;
                }
                let w = clamp01(mix);
                for (slot, add) in out.iter_mut().zip(tmp.iter()) {
                    *slot = [
                        slot[0] + add[0] * w,
                        slot[1] + add[1] * w,
                        slot[2] + add[2] * w,
                    ];
                }
                true
            }
            None => {
                for p in out.iter_mut() {
                    *p = [0.0; 3];
                }
                false
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub clip: u32,
    pub looped: bool,
    pub speed: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Trans {
    pub from: u32,
    pub to: u32,
    pub after: u32,
    pub dur: u32,
}
#[derive(Clone, Debug)]
pub struct Machine {
    pub states: Vec<State>,
    pub trans: Vec<Trans>,
    pub cur: u32,
    pub age: u32,
    pub from_clip: u32,
    pub from_t: f32,
    pub cur_t: f32,
    pub from_spd: f32,
    pub from_loop: bool,
    pub blend_left: u32,
    pub blend_dur: u32,
}
impl Machine {
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            trans: Vec::new(),
            cur: 0,
            age: 0,
            from_clip: 0,
            from_t: 0.0,
            cur_t: 0.0,
            from_spd: 1.0,
            from_loop: true,
            blend_left: 0,
            blend_dur: 0,
        }
    }
    pub fn add_state(&mut self, clip: u32, looped: bool, speed: f32) -> u32 {
        self.states.push(State {
            clip,
            looped,
            speed,
        });
        (self.states.len() - 1) as u32
    }
    pub fn add_trans(&mut self, from: u32, to: u32, after: u32, dur: u32) {
        self.trans.push(Trans {
            from,
            to,
            after,
            dur,
        });
    }
    pub fn reset(&mut self) {
        self.cur = 0;
        self.age = 0;
        self.from_t = 0.0;
        self.cur_t = 0.0;
        self.blend_left = 0;
        self.blend_dur = 0;
        self.from_spd = 1.0;
        self.from_loop = true;
    }
    fn adv(t: f32, dt: f32, dur: f32, looped: bool) -> f32 {
        if dt <= 0.0 {
            return t;
        }
        if dur <= 0.0 {
            return 0.0;
        }
        let nt = t + dt;
        if looped {
            nt % dur
        } else if nt > dur {
            dur
        } else {
            nt
        }
    }
    pub fn tick(&mut self, clips: &[QClip], curs: &mut [Cursor], out: &mut [[f32; 3]]) {
        if self.states.is_empty() {
            for p in out.iter_mut() {
                *p = [0.0; 3];
            }
            return;
        }
        if (self.cur as usize) >= self.states.len() {
            self.cur = 0;
        }
        let mut fire: Option<Trans> = None;
        for tr in &self.trans {
            if tr.from == self.cur && self.age >= tr.after {
                fire = Some(*tr);
                break;
            }
        }
        if let Some(tr) = fire {
            let old = self.cur as usize;
            self.from_clip = self.states[old].clip;
            self.from_t = self.cur_t;
            let s = self.states[old].speed;
            self.from_spd = if s.is_finite() { s } else { 1.0 };
            self.from_loop = self.states[old].looped;
            self.cur = if (tr.to as usize) < self.states.len() {
                tr.to
            } else {
                0
            };
            self.age = 0;
            self.cur_t = 0.0;
            self.blend_dur = tr.dur;
            self.blend_left = tr.dur;
        }
        let ci = self.cur as usize;
        let st = self.states[ci];
        let spd = if st.speed.is_finite() { st.speed } else { 1.0 };
        let dur_b = clips.get(st.clip as usize).map(|c| c.dur).unwrap_or(0.0);
        if self.blend_left > 0 {
            let dur_a = clips
                .get(self.from_clip as usize)
                .map(|c| c.dur)
                .unwrap_or(0.0);
            self.from_t = Self::adv(self.from_t, TICK_DT * self.from_spd, dur_a, self.from_loop);
            self.cur_t = Self::adv(self.cur_t, TICK_DT * spd, dur_b, st.looped);
            // The outgoing clip lands in `out`; each joint of the incoming clip
            // then blends over it, so the tick needs no scratch pose.
            sample_into(clips, curs, self.from_clip, self.from_t, out);
            let f = if self.blend_dur == 0 {
                1.0
            } else {
                1.0 - (self.blend_left as f32) / (self.blend_dur as f32)
            };
            for (joint, slot) in out.iter_mut().enumerate() {
                let incoming = sample_joint(clips, curs, st.clip, self.cur_t, joint);
                *slot = lerp3(*slot, incoming, f);
            }
            self.blend_left -= 1;
        } else {
            self.cur_t = Self::adv(self.cur_t, TICK_DT * spd, dur_b, st.looped);
            sample_into(clips, curs, st.clip, self.cur_t, out);
        }
        self.age += 1;
    }
}
impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}
/// One joint of `clip` at `t`; an unknown clip or a joint past its tracks
/// reads zero, lane for lane what [`sample_into`] writes for that joint.
fn sample_joint(clips: &[QClip], curs: &mut [Cursor], clip: u32, t: f32, joint: usize) -> [f32; 3] {
    let ci = clip as usize;
    if ci >= clips.len() || ci >= curs.len() {
        return [0.0; 3];
    }
    let (track_count, cur) = (clips[ci].tracks.len(), &mut curs[ci]);
    if cur.pos.len() != track_count {
        cur.pos.resize(track_count, 0);
    }
    match clips[ci].tracks.get(joint) {
        Some(track) => sample_track(track, t, &mut cur.pos[joint]),
        None => [0.0; 3],
    }
}
fn sample_into(clips: &[QClip], curs: &mut [Cursor], clip: u32, t: f32, out: &mut [[f32; 3]]) {
    let ci = clip as usize;
    if ci >= clips.len() || ci >= curs.len() {
        for p in out.iter_mut() {
            *p = [0.0; 3];
        }
        return;
    }
    sample_all(&clips[ci], t, &mut curs[ci], out);
}
pub fn traj_hash_mc(m: &Machine, clips: &[QClip], joints: usize, ticks: u32) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(ticks);
    h.write_u64(joints as u64);
    let mut mc = m.clone();
    let mut curs: Vec<Cursor> = clips.iter().map(|c| Cursor::new(c.tracks.len())).collect();
    let mut pose = vec![[0.0f32; 3]; joints];
    for _ in 0..ticks {
        mc.tick(clips, &mut curs, &mut pose);
        for p in &pose {
            for x in p {
                h.write_f32(*x);
            }
        }
    }
    h.finish()
}
pub fn machine_slip(m: &Machine, clips: &[QClip], foot: usize, joints: usize, ticks: u32) -> f32 {
    if ticks < 2 || joints == 0 {
        return 0.0;
    }
    let mut mc = m.clone();
    let mut curs: Vec<Cursor> = clips.iter().map(|c| Cursor::new(c.tracks.len())).collect();
    let mut pose = vec![[0.0f32; 3]; joints];
    mc.tick(clips, &mut curs, &mut pose);
    let mut prev = if foot < joints { pose[foot] } else { [0.0; 3] };
    let mut acc = 0.0f32;
    for _ in 1..ticks {
        mc.tick(clips, &mut curs, &mut pose);
        let q = if foot < joints { pose[foot] } else { [0.0; 3] };
        acc += slip(prev, q, TICK_DT);
        prev = q;
    }
    acc / (ticks - 1) as f32
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{bake_clip, bake_track};
    use engine_core::alloc_probe::measure;
    fn clips() -> Vec<QClip> {
        vec![
            bake_clip(vec![bake_track(
                vec![0.0, 1.0],
                vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
            )]),
            bake_clip(vec![bake_track(
                vec![0.0, 1.0],
                vec![[10.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            )]),
        ]
    }
    fn curs_for(cs: &[QClip]) -> Vec<Cursor> {
        cs.iter().map(|c| Cursor::new(c.tracks.len())).collect()
    }
    #[test]
    fn lerp_mid() {
        assert_eq!(
            lerp3([0.0, 0.0, 0.0], [2.0, 4.0, 6.0], 0.5),
            [1.0, 2.0, 3.0]
        );
    }
    #[test]
    fn graph_mix_weights() {
        let cs = clips();
        let mut cu = curs_for(&cs);
        let g = Graph {
            nodes: vec![Node::Clip(0), Node::Clip(1), Node::Mix(0, 1)],
        };
        let mut o = [[0.0; 3]; 1];
        let mut scratch = [[0.0; 3]; 8];
        assert!(g.eval(&cs, &mut cu, 2, 0.0, 0.0, &mut o, &mut scratch));
        assert!(o[0][0].abs() < 1e-3, "{o:?}");
        assert!(g.eval(&cs, &mut cu, 2, 0.0, 1.0, &mut o, &mut scratch));
        assert!((o[0][0] - 10.0).abs() < 1e-3, "{o:?}");
        assert!(g.eval(&cs, &mut cu, 2, 0.0, 0.5, &mut o, &mut scratch));
        assert!((o[0][0] - 5.0).abs() < 0.05, "{o:?}");
        assert!(!g.eval(&cs, &mut cu, 99, 0.0, 0.0, &mut o, &mut scratch));
    }
    #[test]
    fn graph_add_and_short_scratch() {
        let cs = clips();
        let mut cu = curs_for(&cs);
        let g = Graph {
            nodes: vec![Node::Clip(0), Node::Clip(1), Node::Add(0, 1)],
        };
        let mut o = [[0.0; 3]; 1];
        let mut scratch = [[0.0; 3]; 8];
        assert!(g.eval(&cs, &mut cu, 2, 0.0, 0.5, &mut o, &mut scratch));
        assert!((o[0][0] - 5.0).abs() < 0.05, "{o:?}");
        let tiny: &mut [[f32; 3]] = &mut [];
        assert!(!g.eval(&cs, &mut cu, 2, 0.0, 0.5, &mut o, tiny));
        assert_eq!(o, [[0.0; 3]; 1], "short scratch zeroes the pose");
    }
    #[test]
    fn graph_eval_reuses_scratch_without_alloc() {
        let cs = clips();
        let mut cu = curs_for(&cs);
        let g = Graph {
            nodes: vec![
                Node::Clip(0),
                Node::Clip(1),
                Node::Mix(0, 1),
                Node::Add(2, 2),
            ],
        };
        let mut o = [[0.0; 3]; 2];
        let mut scratch = [[0.0; 3]; 16];
        assert!(g.eval(&cs, &mut cu, 3, 0.0, 0.5, &mut o, &mut scratch));
        let ((), heap) = measure(|| {
            for _ in 0..40 {
                assert!(g.eval(&cs, &mut cu, 3, 0.0, 0.5, &mut o, &mut scratch));
            }
        });
        assert_eq!(heap.calls, 0, "graph eval must not allocate");
    }
    #[test]
    fn machine_fires_and_settles() {
        let cs = clips();
        let mut cu = curs_for(&cs);
        let mut m = Machine::new();
        m.add_state(0, true, 1.0);
        m.add_state(1, true, 1.0);
        m.add_trans(0, 1, 5, 5);
        let mut o = [[0.0; 3]; 1];
        for _ in 0..4 {
            m.tick(&cs, &mut cu, &mut o);
        }
        assert!(o[0][0].abs() < 0.1, "{o:?}");
        for _ in 0..20 {
            m.tick(&cs, &mut cu, &mut o);
        }
        assert!((o[0][0] - 10.0).abs() < 0.1, "{o:?}");
        assert_eq!(m.cur, 1);
    }
    #[test]
    fn sample_joint_matches_sample_into_lane_for_lane() {
        let cs = vec![
            bake_clip(vec![
                bake_track(vec![0.0, 1.0], vec![[0.0, 1.0, 2.0], [3.0, 5.0, 8.0]]),
                bake_track(vec![0.0, 0.5, 1.0], vec![[1.0; 3], [4.0; 3], [2.0; 3]]),
            ]),
            bake_clip(vec![bake_track(vec![0.0, 2.0], vec![[9.0; 3], [-9.0; 3]])]),
        ];
        // Unknown clip 5 and joints 2..4 past the tracks read zero.
        for clip in [0u32, 1, 5] {
            for t in [0.0f32, 0.3, 0.5, 1.0, 1.7, 9.0] {
                let (mut by_joint, mut whole) = (curs_for(&cs), curs_for(&cs));
                let mut want = [[7.0f32; 3]; 4];
                sample_into(&cs, &mut whole, clip, t, &mut want);
                for (joint, lane) in want.iter().enumerate() {
                    let got = sample_joint(&cs, &mut by_joint, clip, t, joint);
                    assert_eq!(got, *lane, "clip {clip} t {t} joint {joint}");
                }
            }
        }
    }

    #[test]
    fn mid_transition_output_is_the_blend_factor_lerp() {
        // Clip 0 is constant 0 and clip 1 constant 10, so tick k of a ten
        // tick blend must read exactly `10 * k / 10`.
        let cs = clips();
        let mut cu = curs_for(&cs);
        let mut m = Machine::new();
        m.add_state(0, true, 1.0);
        m.add_state(1, true, 1.0);
        m.add_trans(0, 1, 0, 10);
        let mut o = [[0.0; 3]; 2];
        for k in 0..10u8 {
            m.tick(&cs, &mut cu, &mut o);
            let want = f32::from(k);
            assert!((o[0][0] - want).abs() < 1e-3, "tick {k}: {o:?}");
            assert_eq!(o[1], [0.0; 3], "tick {k}: joint past the tracks stays zero");
        }
    }

    #[test]
    fn blending_ticks_allocate_nothing_after_warmup() {
        let cs = clips();
        let mut cu = curs_for(&cs);
        let mut m = Machine::new();
        m.add_state(0, true, 1.0);
        m.add_state(1, true, 1.0);
        m.add_trans(0, 1, 2, 12);
        let mut o = [[0.0; 3]; 4];
        m.tick(&cs, &mut cu, &mut o);
        let ((), heap) = measure(|| {
            for _ in 0..40 {
                m.tick(&cs, &mut cu, &mut o);
            }
        });
        assert_eq!(m.cur, 1, "the script must cross the transition");
        assert_eq!(heap.calls, 0, "machine tick must not allocate");
    }

    #[test]
    fn traj_x2() {
        let cs = clips();
        let mut m = Machine::new();
        m.add_state(0, true, 1.0);
        m.add_state(1, true, 1.0);
        m.add_trans(0, 1, 30, 10);
        assert_eq!(traj_hash_mc(&m, &cs, 1, 600), traj_hash_mc(&m, &cs, 1, 600));
    }
    #[test]
    fn slip_small() {
        let cs = clips();
        let mut m = Machine::new();
        m.add_state(0, true, 1.0);
        assert!(
            machine_slip(&m, &cs, 0, 1, 120) < 0.08,
            "{}",
            machine_slip(&m, &cs, 0, 1, 120)
        );
    }
}
