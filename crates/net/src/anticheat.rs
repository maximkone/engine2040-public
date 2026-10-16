//! anticheat: server guards. Sanitize + speed/teleport caps + Merkle verify + killcam ring.
use engine_core::{hash::GOLDEN_GAMMA, mix64};

/// Button bits the server accepts (the low six); any other bit is masked off
/// and flagged.
pub const ALLOWED_BUTTONS: u16 = 0x003F;
/// Largest look delta per axis; larger values clamp.
pub const MAX_LOOK: f32 = 16.0;
/// Largest legal move per tick in meters; more flags [`F_SPEED`].
pub const MAX_STEP: f32 = 0.25;
/// Move length in meters past which a step counts as a teleport.
pub const TELEPORT_DIST: f32 = 12.0;
/// Flag bits: no violation.
pub const F_OK: u8 = 0;
/// Flag bit: a value was clamped, masked off or non-finite.
pub const F_CLAMP: u8 = 1;
/// Flag bit: the step is longer than [`MAX_STEP`].
pub const F_SPEED: u8 = 2;
/// Flag bit: the step is longer than [`TELEPORT_DIST`] or non-finite.
pub const F_TELEPORT: u8 = 4;
/// Flag bit: the tick did not advance, or jumped more than eight ticks.
pub const F_TICK: u8 = 8;
/// Flag bit: buttons outside [`ALLOWED_BUTTONS`] were set.
pub const F_BUTTON: u8 = 16;
/// Untrusted client input as received, before [`sanitize`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawInput {
    /// Forward axis.
    pub mx: f32,
    /// Strafe axis.
    pub my: f32,
    /// Horizontal look delta.
    pub lx: f32,
    /// Vertical look delta.
    pub ly: f32,
    /// Button bits as sent.
    pub buttons: u16,
    /// Tick the client claims.
    pub tick: u32,
}
/// Sanitized input: axes inside `[-1, 1]`, look inside `+/-MAX_LOOK`, only
/// [`ALLOWED_BUTTONS`] set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Forward axis.
    pub mx: f32,
    /// Strafe axis.
    pub my: f32,
    /// Horizontal look delta.
    pub lx: f32,
    /// Vertical look delta.
    pub ly: f32,
    /// Button bits after masking.
    pub buttons: u16,
    /// Tick the client claimed.
    pub tick: u32,
}
fn clamp_axis(v: f32) -> (f32, bool) {
    if !v.is_finite() {
        return (0.0, true);
    }
    if v > 1.0 {
        (1.0, true)
    } else if v < -1.0 {
        (-1.0, true)
    } else {
        (v, false)
    }
}
fn clamp_look(v: f32) -> (f32, bool) {
    if !v.is_finite() {
        return (0.0, true);
    }
    if v > MAX_LOOK {
        (MAX_LOOK, true)
    } else if v < -MAX_LOOK {
        (-MAX_LOOK, true)
    } else {
        (v, false)
    }
}
/// Clamp axes to [-1,1], look to [-MAX_LOOK,MAX_LOOK], mask buttons, kill NaN/inf. Returns flags.
pub fn sanitize(r: &RawInput) -> (Input, u8) {
    let mut f = F_OK;
    let (mx, c1) = clamp_axis(r.mx);
    let (my, c2) = clamp_axis(r.my);
    let (lx, c3) = clamp_look(r.lx);
    let (ly, c4) = clamp_look(r.ly);
    if c1 || c2 || c3 || c4 {
        f |= F_CLAMP;
    }
    let masked = r.buttons & ALLOWED_BUTTONS;
    if masked != r.buttons {
        f |= F_BUTTON | F_CLAMP;
    }
    (
        Input {
            mx,
            my,
            lx,
            ly,
            buttons: masked,
            tick: r.tick,
        },
        f,
    )
}
/// Squared move check: SPEED if step too big, TELEPORT if dist too big. No sqrt.
pub fn check_move(a: [f32; 3], b: [f32; 3]) -> u8 {
    let (mut dx, mut dy, mut dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    if !dx.is_finite() || !dy.is_finite() || !dz.is_finite() {
        return F_TELEPORT | F_CLAMP;
    }
    dx = dx.clamp(-64.0, 64.0);
    dy = dy.clamp(-64.0, 64.0);
    dz = dz.clamp(-64.0, 64.0);
    let d2 = dx * dx + dy * dy + dz * dz;
    if d2 > TELEPORT_DIST * TELEPORT_DIST {
        F_TELEPORT | F_SPEED
    } else if d2 > MAX_STEP * MAX_STEP {
        F_SPEED
    } else {
        F_OK
    }
}
/// Per-player server guard: tick monotonicity + move caps. Counts strikes.
pub struct PlayerGuard {
    /// Position recorded by the last observation.
    pub last: [f32; 3],
    /// Tick recorded by the last observation.
    pub last_tick: u32,
    /// False until the first observation seeds `last` and `last_tick`.
    pub init: bool,
    /// Observations that raised any flag (saturating).
    pub strikes: u32,
}
impl PlayerGuard {
    /// Fresh guard: the first observation is accepted unconditionally.
    pub fn new() -> Self {
        Self {
            last: [0.0; 3],
            last_tick: 0,
            init: false,
            strikes: 0,
        }
    }
    /// Check `pos` at `tick` against the last observation: flags a tick that
    /// did not advance or jumped more than eight ticks, plus the
    /// [`check_move`] bits. Any flag adds a strike; the new position and tick
    /// are always recorded. Returns the flag bits.
    pub fn observe(&mut self, pos: [f32; 3], tick: u32) -> u8 {
        let mut f = F_OK;
        if !self.init {
            self.init = true;
            self.last = pos;
            self.last_tick = tick;
            return F_OK;
        }
        if tick <= self.last_tick || tick.wrapping_sub(self.last_tick) > 8 {
            f |= F_TICK;
        }
        f |= check_move(self.last, pos);
        if f != F_OK {
            self.strikes = self.strikes.saturating_add(1);
        }
        self.last = pos;
        self.last_tick = tick;
        f
    }
}
impl Default for PlayerGuard {
    fn default() -> Self {
        Self::new()
    }
}
/// Leaf = mixed (tick,input,state) through the shared SplitMix owner.
pub fn leaf_hash(tick: u32, input: u32, state: u64) -> u64 {
    mix64(
        u64::from(tick).wrapping_mul(GOLDEN_GAMMA)
            ^ u64::from(input).wrapping_mul(0xC2B2AE3D27D4EB4F)
            ^ state.wrapping_add(0x165667B19E3779F9),
    )
}
#[inline]
fn pair(l: u64, r: u64) -> u64 {
    mix64(l.wrapping_mul(GOLDEN_GAMMA) ^ r.wrapping_mul(0xC2B2AE3D27D4EB4F) ^ 0xA24BAED4963EE407)
}
/// Merkle root over leaves; odd level duplicates last; empty=0. Deterministic.
pub fn merkle_root(leaves: &[u64]) -> u64 {
    if leaves.is_empty() {
        return 0;
    }
    let mut cur = leaves.to_vec();
    while cur.len() > 1 {
        let mut nxt = Vec::with_capacity(cur.len().div_ceil(2));
        for c in cur.chunks(2) {
            let (l, r) = (c[0], if c.len() > 1 { c[1] } else { c[0] });
            nxt.push(pair(l, r));
        }
        cur = nxt;
    }
    cur[0]
}
/// Sibling path for leaf index (dup-last on odd). Empty vec if OOB/singleton.
pub fn merkle_prove(leaves: &[u64], mut idx: usize) -> Vec<u64> {
    let mut proof = vec![];
    if idx >= leaves.len() || leaves.len() < 2 {
        return proof;
    }
    let mut cur = leaves.to_vec();
    while cur.len() > 1 {
        let sib = if idx ^ 1 < cur.len() {
            cur[idx ^ 1]
        } else {
            cur[idx]
        };
        proof.push(sib);
        let mut nxt = Vec::with_capacity(cur.len().div_ceil(2));
        for c in cur.chunks(2) {
            nxt.push(pair(c[0], if c.len() > 1 { c[1] } else { c[0] }));
        }
        cur = nxt;
        idx /= 2;
    }
    proof
}
/// Recompute from leaf+index+proof and compare to root.
pub fn merkle_verify(leaf: u64, mut idx: usize, proof: &[u64], root: u64) -> bool {
    let mut cur = leaf;
    for s in proof {
        cur = if idx & 1 == 0 {
            pair(cur, *s)
        } else {
            pair(*s, cur)
        };
        idx /= 2;
    }
    cur == root
}
/// Append-only replay log with Merkle root/verify.
#[derive(Clone, Default)]
pub struct Replay {
    /// Tick of each logged step.
    pub ticks: Vec<u32>,
    /// Input word of each logged step.
    pub inputs: Vec<u32>,
    /// State hash of each logged step.
    pub states: Vec<u64>,
}
impl Replay {
    /// Empty log.
    pub fn new() -> Self {
        Self::default()
    }
    /// Append one step: its tick, input word and resulting state hash.
    pub fn push(&mut self, tick: u32, input: u32, state: u64) {
        self.ticks.push(tick);
        self.inputs.push(input);
        self.states.push(state);
    }
    /// Leaf hash of every logged step, in log order.
    pub fn leaves(&self) -> Vec<u64> {
        self.ticks
            .iter()
            .zip(self.inputs.iter())
            .zip(self.states.iter())
            .map(|((t, i), s)| leaf_hash(*t, *i, *s))
            .collect()
    }
    /// Merkle root over the leaves (0 for an empty log).
    pub fn root(&self) -> u64 {
        merkle_root(&self.leaves())
    }
    /// Sibling path for step `idx`; empty when out of range or a single step.
    pub fn prove(&self, idx: usize) -> Vec<u64> {
        merkle_prove(&self.leaves(), idx)
    }
    /// True when step `idx` with `proof` hashes up to `root`; an index past
    /// the log is false.
    pub fn verify(&self, idx: usize, proof: &[u64], root: u64) -> bool {
        if idx >= self.ticks.len() {
            return false;
        }
        merkle_verify(
            leaf_hash(self.ticks[idx], self.inputs[idx], self.states[idx]),
            idx,
            proof,
            root,
        )
    }
    /// Logged step count.
    pub fn len(&self) -> usize {
        self.ticks.len()
    }
    /// True when nothing is logged.
    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }
}
/// One killcam sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    /// Tick the sample was taken on.
    pub tick: u32,
    /// Position in meters.
    pub pos: [f32; 3],
    /// Facing yaw.
    pub yaw: f32,
}
/// Killcam ring: last N snaps, insertion-ordered playback.
pub struct Killcam {
    buf: Vec<Snap>,
    cap: usize,
    head: usize,
}
impl Killcam {
    /// Ring holding at most `cap` snaps (a `cap` of 0 counts as 1).
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            cap: cap.max(1),
            head: 0,
        }
    }
    /// Append a snap; once the ring is full the oldest is overwritten.
    pub fn push(&mut self, s: Snap) {
        if self.buf.len() < self.cap {
            self.buf.push(s);
            self.head = self.buf.len() % self.cap;
        } else {
            self.buf[self.head] = s;
            self.head = (self.head + 1) % self.cap;
        }
    }
    /// Held snaps, oldest first.
    pub fn ordered(&self) -> Vec<Snap> {
        if self.buf.len() < self.cap {
            return self.buf.clone();
        }
        let mut o = Vec::with_capacity(self.buf.len());
        for i in 0..self.buf.len() {
            o.push(self.buf[(self.head + i) % self.cap]);
        }
        o
    }
    /// The newest `n` snaps, oldest first (all of them when `n` exceeds the
    /// count held).
    pub fn last(&self, n: usize) -> Vec<Snap> {
        let o = self.ordered();
        o.into_iter().skip(o_len(n, self.buf.len())).collect()
    }
    /// Snaps currently held.
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    /// True when no snap is held.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}
fn o_len(n: usize, len: usize) -> usize {
    len.saturating_sub(n.min(len))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn lcg(x: &mut u64) -> u64 {
        *x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        (*x >> 33) ^ *x
    }
    fn bits_f32(v: u32) -> f32 {
        f32::from_bits(v)
    }
    #[test]
    fn fuzz_clamped() {
        let mut s = 0x12345678u64;
        for _ in 0..4096 {
            let r = RawInput {
                mx: bits_f32(lcg(&mut s) as u32),
                my: bits_f32(lcg(&mut s) as u32),
                lx: bits_f32(lcg(&mut s) as u32),
                ly: bits_f32(lcg(&mut s) as u32),
                buttons: lcg(&mut s) as u16,
                tick: lcg(&mut s) as u32,
            };
            let (c, f) = sanitize(&r);
            assert!((-1.0..=1.0).contains(&c.mx) && (-1.0..=1.0).contains(&c.my));
            assert!(
                (-MAX_LOOK..=MAX_LOOK).contains(&c.lx) && (-MAX_LOOK..=MAX_LOOK).contains(&c.ly)
            );
            assert_eq!(c.buttons & !ALLOWED_BUTTONS, 0);
            assert!(c.mx.is_finite() && c.my.is_finite() && c.lx.is_finite() && c.ly.is_finite());
            if !r.mx.is_finite()
                || r.mx.abs() > 1.0
                || !r.my.is_finite()
                || r.my.abs() > 1.0
                || !r.lx.is_finite()
                || r.lx.abs() > MAX_LOOK
                || !r.ly.is_finite()
                || r.ly.abs() > MAX_LOOK
                || r.buttons & !ALLOWED_BUTTONS != 0
            {
                assert_ne!(f & F_CLAMP, 0, "{r:?}");
            }
        }
    }
    #[test]
    fn speed_teleport_caps() {
        assert_eq!(check_move([0.0; 3], [0.05, 0.0, 0.0]), F_OK);
        assert_ne!(check_move([0.0; 3], [1.0, 0.0, 0.0]) & F_SPEED, 0);
        assert_ne!(check_move([0.0; 3], [20.0, 0.0, 0.0]) & F_TELEPORT, 0);
        let mut g = PlayerGuard::new();
        assert_eq!(g.observe([0.0; 3], 1), F_OK);
        assert_eq!(g.observe([0.0, 0.0, 0.0], 2), F_OK);
        assert_ne!(g.observe([50.0, 0.0, 0.0], 3) & F_TELEPORT, 0);
        assert_ne!(g.observe([50.0, 0.0, 0.0], 3) & F_TICK, 0);
        assert_eq!(g.strikes, 2);
    }
    #[test]
    fn merkle_verify() {
        let mut log = Replay::new();
        for i in 0..8u32 {
            log.push(
                i,
                i.wrapping_mul(7).wrapping_add(1),
                u64::from(i).wrapping_mul(GOLDEN_GAMMA),
            );
        }
        let root = log.root();
        for i in 0..8 {
            let p = log.prove(i);
            assert!(log.verify(i, &p, root), "idx {i}");
        }
        let mut bad = log.clone();
        bad.states[3] ^= 1;
        assert_ne!(bad.root(), root);
        assert!(!bad.verify(3, &log.prove(3), root));
        assert_eq!(Replay::new().root(), 0);
        assert_eq!(merkle_root(&[42]), 42);
    }
    #[test]
    fn merkle_determinism_x2() {
        let build = || {
            let mut l = Replay::new();
            for i in 0..16u32 {
                l.push(i, i * 3 + 1, i as u64 * 11);
            }
            l.root()
        };
        assert_eq!(build(), build());
    }
    #[test]
    fn killcam_ring_order() {
        let mut k = Killcam::new(4);
        for i in 0..6u32 {
            k.push(Snap {
                tick: i,
                pos: [i as f32, 0.0, 0.0],
                yaw: 0.0,
            });
        }
        assert_eq!(k.len(), 4);
        let o: Vec<u32> = k.ordered().iter().map(|s| s.tick).collect();
        assert_eq!(o, vec![2, 3, 4, 5]);
        let l: Vec<u32> = k.last(2).iter().map(|s| s.tick).collect();
        assert_eq!(l, vec![4, 5]);
        let a = k.ordered();
        let mut k2 = Killcam::new(4);
        for i in 0..6u32 {
            k2.push(Snap {
                tick: i,
                pos: [i as f32, 0.0, 0.0],
                yaw: 0.0,
            });
        }
        assert_eq!(a, k2.ordered());
    }
}
