//! par: per-character animation eval on the RC3 job pool.
//!
//! Owner: this module; presentation and bake callers fan a crowd of
//! [`Character`]s out in index-ordered chunks, the sim tick never calls in.
//! Units: ticks (`u32`), joints as `[f32; 3]` positions. Determinism: every
//! character owns its machine, cursors and pose, so a job touches nothing
//! shared but the read-only bank; the pool decides where a chunk runs,
//! never what it writes. Same input gives bit-identical poses for any
//! worker count and any chunking (proven below against the serial path).
//!
//! Two samplers live here. [`eval_serial`] runs the crate's own scalar
//! sampler and is the equality oracle. [`eval_simd_serial`] and
//! [`eval_simd_parallel`] run the SoA bank ([`SoaBank`]): the keys are
//! dequantized once when the bank is built, so the hot loop never divides.
//! A clip whose tracks share one clock (the bake's shape) is stored key
//! major, so a tick seeks once and streams the whole pose; a clip with
//! staggered clocks keeps the per-track path. With the `simd` feature the
//! three channels of a joint ride one `wide` vector; without it the same
//! bank is read one channel at a time. Both paths give the same poses bit
//! for bit, so the fast path never moves a fighter.

use engine_core::checked::f32_from_u64;
use engine_core::Fnv64;
use engine_ecs::jobs::ChunkSize;
use engine_ecs::JobPool;
#[cfg(feature = "simd")]
use wide::f32x4;

use crate::blend::{lerp3, Machine, Trans};
use crate::clip::{clamp01, Cursor, QClip, QTrack};

/// One animated character: its own machine, per-clip cursors and pose.
#[derive(Clone, Debug)]
pub struct Character {
    machine: Machine,
    cursors: Vec<Cursor>,
    pose: Vec<[f32; 3]>,
}

impl Character {
    /// Character at the start of `machine` over `clips`, `joints` wide.
    pub fn new(machine: Machine, clips: &[QClip], joints: usize) -> Self {
        Self {
            machine,
            cursors: clips.iter().map(|c| Cursor::new(c.tracks.len())).collect(),
            pose: vec![[0.0; 3]; joints],
        }
    }

    /// The pose after the last evaluated tick.
    pub fn pose(&self) -> &[[f32; 3]] {
        &self.pose
    }

    /// Advance `ticks` fixed ticks; the pose holds the last one.
    fn run(&mut self, clips: &[QClip], ticks: u32) {
        for _ in 0..ticks {
            self.machine.tick(clips, &mut self.cursors, &mut self.pose);
        }
    }

    /// Advance `ticks` fixed ticks through the bank; the pose holds the last
    /// one.
    fn run_simd(&mut self, bank: &SoaBank, ticks: u32) {
        for _ in 0..ticks {
            self.tick_simd(bank);
        }
    }

    /// One tick through the bank.
    ///
    /// The machine owns the state and the bank owns the pose. The machine
    /// advances on the durations-only clips (it reads the clip lengths and
    /// the transition table) and leaves the pose to the bank, so the bank
    /// has to make the same choice the machine would make: which clip
    /// plays, and whether the tick blends over the outgoing clip.
    fn tick_simd(&mut self, bank: &SoaBank) {
        // What the machine does this tick: a transition fires or it does
        // not. Read before the tick, because the tick consumes it.
        let fired = fired_transition(&self.machine);
        // The blend the machine runs, if any. The tick takes one off the
        // count, so the factor is read before it.
        let blend_left = self.machine.blend_left;
        let no_cursors: &mut [Cursor] = &mut [];
        self.machine
            .tick(&bank.durations, no_cursors, &mut self.pose);
        if self.machine.states.is_empty() {
            // No state: the machine zeroed the pose and stopped.
            return;
        }
        let cur = usize::try_from(self.machine.cur).unwrap_or(usize::MAX);
        let Some(state) = self.machine.states.get(cur) else {
            return;
        };
        let clip = state.clip;
        let t = self.machine.cur_t;
        match fired {
            // A transition without blend time: the new clip plays alone.
            Some(trans) if trans.dur == 0 => self.sample_simd(bank, clip, t),
            // A transition with blend time: the factor is zero on the
            // transition tick, so the outgoing clip holds the pose.
            Some(_) => {
                self.sample_simd(bank, self.machine.from_clip, self.machine.from_t);
                self.blend_simd(bank, clip, t, 0.0);
            }
            // A plain blend tick: the incoming clip fades over the outgoing.
            None if blend_left > 0 => {
                let dur = self.machine.blend_dur;
                let f = if dur == 0 {
                    1.0
                } else {
                    1.0 - f32_from_u64(u64::from(blend_left)) / f32_from_u64(u64::from(dur))
                };
                self.sample_simd(bank, self.machine.from_clip, self.machine.from_t);
                self.blend_simd(bank, clip, t, f);
            }
            // No blend: the current clip plays alone.
            None => self.sample_simd(bank, clip, t),
        }
    }

    /// Write the pose of clip `clip` at `t` into this character's pose.
    fn sample_simd(&mut self, bank: &SoaBank, clip: u32, t: f32) {
        let clip = usize::try_from(clip).unwrap_or(usize::MAX);
        let Some(cur) = self.cursors.get_mut(clip) else {
            for slot in self.pose.iter_mut() {
                *slot = [0.0; 3];
            }
            return;
        };
        bank.sample(clip, t, &mut self.pose, cur);
    }

    /// One joint of clip `clip` at `t`; a joint past the clip's tracks reads
    /// zero.
    fn sample_simd_joint(&mut self, bank: &SoaBank, clip: u32, t: f32, joint: usize) -> [f32; 3] {
        let clip = usize::try_from(clip).unwrap_or(usize::MAX);
        let Some(cur) = self.cursors.get_mut(clip) else {
            return [0.0; 3];
        };
        bank.joint_at(clip, t, joint, cur)
    }

    /// Blend the clip over the pose, joint by joint: the same loop and the
    /// same factor the machine runs.
    fn blend_simd(&mut self, bank: &SoaBank, clip: u32, t: f32, f: f32) {
        for joint in 0..self.pose.len() {
            let incoming = self.sample_simd_joint(bank, clip, t, joint);
            self.pose[joint] = lerp3(self.pose[joint], incoming, f);
        }
    }
}

/// The transition the machine fires on the next tick, if any.
///
/// Mirrors the rule in `Machine::tick`: the first transition whose `from`
/// is the current state and whose `after` age has passed. The machine
/// resets an out-of-range state to zero before it looks, so the mirror
/// does too. The test below compares both paths tick by tick, so a change
/// in the machine that this mirror misses fails that test.
fn fired_transition(machine: &Machine) -> Option<Trans> {
    let cur = if usize::try_from(machine.cur).unwrap_or(usize::MAX) >= machine.states.len() {
        0
    } else {
        machine.cur
    };
    machine
        .trans
        .iter()
        .copied()
        .find(|tr| tr.from == cur && machine.age >= tr.after)
}

/// The SoA clip bank the fast path samples.
///
/// One array per track: the key times in one array and the key values in
/// another, the values already dequantized, so the hot loop never divides.
/// A clip with one shared clock also keeps its keys key-major, so one tick
/// touches two contiguous runs. The bank is read-only once built, so every
/// job shares it.
#[derive(Clone, Debug)]
pub struct SoaBank {
    clips: Vec<SoaClip>,
    /// The same clips with empty tracks: the state machine advances on these
    /// (it reads the clip lengths only) while the pose comes from the bank.
    durations: Vec<QClip>,
}

/// One clip of the bank: its tracks, in joint order.
///
/// A clip whose tracks all share one key clock (the bake writes them that
/// way) also carries the flat [`UniformBank`]: one seek per tick for the
/// whole pose, then a straight stream over the two keys. A clip with a
/// mixed clock keeps the per-track path, and both give the scalar
/// sampler's values bit for bit.
#[derive(Clone, Debug)]
struct SoaClip {
    tracks: Vec<SoaTrack>,
    uniform: Option<UniformBank>,
}

/// The whole clip in one key-major stream: key `k` covers
/// `keys[k * joints ..][..joints]`, so one interval serves every joint.
#[derive(Clone, Debug)]
struct UniformBank {
    times: Vec<f32>,
    keys: Vec<[f32; 4]>,
    joints: usize,
}

impl UniformBank {
    /// `None` unless every track is valid and carries the first track's
    /// clock. An empty clip has no clock.
    fn build(tracks: &[SoaTrack]) -> Option<Self> {
        let first = tracks.first()?;
        if first.times.is_empty() || tracks.iter().any(|tr| tr.times != first.times) {
            return None;
        }
        let mut keys = Vec::with_capacity(first.times.len() * tracks.len());
        for k in 0..first.times.len() {
            for tr in tracks {
                keys.push(tr.values[k]);
            }
        }
        Some(Self {
            times: first.times.clone(),
            keys,
            joints: tracks.len(),
        })
    }

    /// Write the whole pose: one seek, then a straight lerp over the two
    /// keys. The expression is the scalar sampler's, lane for lane.
    fn sample(&self, t: f32, out: &mut [[f32; 3]], cur: &mut Cursor) {
        let n = out.len().min(self.joints);
        let Some((lo, hi, f)) = seek_times(&self.times, t, &mut cur.pos[0]) else {
            for slot in out.iter_mut() {
                *slot = [0.0; 3];
            }
            return;
        };
        let a = &self.keys[lo * self.joints..lo * self.joints + n];
        // A clamped key is copied, never lerped: the scalar sampler returns
        // its stored value there, sign of zero included.
        if lo == hi {
            for (slot, a) in out.iter_mut().zip(a.iter()) {
                *slot = [a[0], a[1], a[2]];
            }
        } else {
            let b = &self.keys[hi * self.joints..hi * self.joints + n];
            for (slot, (a, b)) in out.iter_mut().zip(a.iter().zip(b.iter())) {
                *slot = lerp4(*a, *b, f);
            }
        }
        for slot in out.iter_mut().skip(n) {
            *slot = [0.0; 3];
        }
    }

    /// One joint of the clip; a joint past the clip's tracks reads zero.
    fn joint_at(&self, t: f32, joint: usize, cur: &mut Cursor) -> [f32; 3] {
        if joint >= self.joints {
            return [0.0; 3];
        }
        let Some((lo, hi, f)) = seek_times(&self.times, t, &mut cur.pos[0]) else {
            return [0.0; 3];
        };
        let a = self.keys[lo * self.joints + joint];
        if lo == hi {
            return [a[0], a[1], a[2]];
        }
        lerp4(a, self.keys[hi * self.joints + joint], f)
    }
}

/// One track of the bank: the key times and the dequantized key values.
///
/// The values are one flat array, four floats per key: x, y, z and a zero
/// lane. The zero lane carries the vector lerp to four lanes, so one wide
/// lerp gives the whole joint.
#[derive(Clone, Debug)]
struct SoaTrack {
    times: Vec<f32>,
    values: Vec<[f32; 4]>,
}

impl SoaBank {
    /// Build the bank from baked clips. The keys are dequantized once, here.
    pub fn new(clips: &[QClip]) -> Self {
        let mut bank = Self {
            clips: Vec::with_capacity(clips.len()),
            durations: Vec::with_capacity(clips.len()),
        };
        for clip in clips {
            let tracks: Vec<SoaTrack> = clip.tracks.iter().map(SoaTrack::new).collect();
            let uniform = UniformBank::build(&tracks);
            bank.clips.push(SoaClip { tracks, uniform });
            bank.durations.push(QClip {
                tracks: Vec::new(),
                dur: clip.dur,
            });
        }
        bank
    }

    /// The pose of clip `clip` at `t`, joint by joint. Joints past the
    /// clip's tracks read zero, like the crate's scalar sampler.
    fn sample(&self, clip: usize, t: f32, out: &mut [[f32; 3]], cur: &mut Cursor) {
        let Some(entry) = self.clips.get(clip) else {
            for slot in out.iter_mut() {
                *slot = [0.0; 3];
            }
            return;
        };
        let tracks = &entry.tracks;
        if cur.pos.len() != tracks.len() {
            cur.pos.resize(tracks.len(), 0);
        }
        if let Some(uniform) = &entry.uniform {
            return uniform.sample(t, out, cur);
        }
        let count = tracks.len().min(out.len());
        for (joint, track) in tracks.iter().enumerate().take(count) {
            out[joint] = sample_track(track, t, &mut cur.pos[joint]);
        }
        for slot in out.iter_mut().skip(count) {
            *slot = [0.0; 3];
        }
    }

    /// One joint of clip `clip` at `t`; a joint past the clip's tracks reads
    /// zero.
    fn joint_at(&self, clip: usize, t: f32, joint: usize, cur: &mut Cursor) -> [f32; 3] {
        let Some(entry) = self.clips.get(clip) else {
            return [0.0; 3];
        };
        if cur.pos.len() != entry.tracks.len() {
            cur.pos.resize(entry.tracks.len(), 0);
        }
        if let Some(uniform) = &entry.uniform {
            return uniform.joint_at(t, joint, cur);
        }
        let Some(track) = entry.tracks.get(joint) else {
            return [0.0; 3];
        };
        sample_track(track, t, &mut cur.pos[joint])
    }
}

impl SoaTrack {
    /// The track in bank form: the key times and the dequantized key
    /// values. A track the crate's sampler refuses (empty or mismatched
    /// keys) stays empty here, so it reads zero the same way.
    fn new(track: &QTrack) -> Self {
        let mut values = Vec::with_capacity(track.keys.len());
        if track.times.len() != track.keys.len() || track.times.is_empty() {
            return Self {
                times: Vec::new(),
                values,
            };
        }
        for key in &track.keys {
            values.push([
                dequantize(key[0], track.min[0], track.max[0]),
                dequantize(key[1], track.min[1], track.max[1]),
                dequantize(key[2], track.min[2], track.max[2]),
                0.0,
            ]);
        }
        Self {
            times: track.times.clone(),
            values,
        }
    }
}

/// The two key indices to lerp and the fraction between them, or `None` for a
/// clock the crate's sampler refuses. The hint is the previous index: the scan
/// walks forward from it. One clock serves a whole uniform clip.
fn seek_times(times: &[f32], t: f32, hint: &mut usize) -> Option<(usize, usize, f32)> {
    if times.is_empty() {
        return None;
    }
    let last = times.len() - 1;
    // A NaN time maps to the first key, like the crate's sampler.
    let t = if t.is_nan() { times[0] } else { t };
    if t <= times[0] {
        *hint = 0;
        return Some((0, 0, 0.0));
    }
    if t >= times[last] {
        *hint = last;
        return Some((last, last, 0.0));
    }
    let lo = find(times, t, hint);
    let (t0, t1) = (times[lo], times[lo + 1]);
    let span = t1 - t0;
    let f = if span <= 0.0 {
        0.0
    } else {
        clamp01((t - t0) / span)
    };
    *hint = lo;
    Some((lo, lo + 1, f))
}

/// The last key index at or before `t`, walking forward from the hint.
fn find(times: &[f32], t: f32, hint: &usize) -> usize {
    let last = times.len() - 1;
    let mut lo = (*hint).min(last);
    if times[lo] <= t {
        while lo < last && times[lo + 1] <= t {
            lo += 1;
        }
        return lo;
    }
    // The hint overshot. The keys are sorted, so the count of keys at or
    // before `t` names the interval.
    times.partition_point(|time| *time <= t).saturating_sub(1)
}

/// One joint from two bank keys: the scalar sampler's expression, lane for
/// lane.
#[cfg(feature = "simd")]
fn lerp4(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 3] {
    let a = f32x4::new(a);
    let b = f32x4::new(b);
    let out = (a + (b - a) * f32x4::splat(f)).to_array();
    [out[0], out[1], out[2]]
}

/// One joint from two bank keys, one channel at a time: the build without the
/// `simd` feature writes the same values.
#[cfg(not(feature = "simd"))]
fn lerp4(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 3] {
    lerp3([a[0], a[1], a[2]], [b[0], b[1], b[2]], f)
}

/// One dequantized channel: the same expression the crate's sampler uses, so
/// the bank and the scalar path agree bit for bit.
fn dequantize(key: u16, min: f32, max: f32) -> f32 {
    min + (max - min) * (f32::from(key) / 65535.0)
}

/// One joint of one track: the value the crate's scalar sampler gives. The
/// three channels ride one wide vector under the `simd` feature.
fn sample_track(track: &SoaTrack, t: f32, hint: &mut usize) -> [f32; 3] {
    let Some((lo, hi, f)) = seek_times(&track.times, t, hint) else {
        return [0.0; 3];
    };
    let a = track.values[lo];
    if lo == hi {
        return [a[0], a[1], a[2]];
    }
    lerp4(a, track.values[hi], f)
}

/// Advance every character `ticks` ticks on the caller, in index order.
pub fn eval_serial(chars: &mut [Character], clips: &[QClip], ticks: u32) {
    for c in chars {
        c.run(clips, ticks);
    }
}

/// Advance every character `ticks` ticks on `pool`, one job per chunk of at
/// most `chunk` characters. A crowd that fits one chunk runs inline. Equal
/// to [`eval_serial`] bit for bit for every worker count and chunking.
pub fn eval_parallel(
    pool: &JobPool,
    chars: &mut [Character],
    clips: &[QClip],
    ticks: u32,
    chunk: ChunkSize,
) {
    let size = chunk.value();
    if chars.len() <= size {
        return eval_serial(chars, clips, ticks);
    }
    pool.scope(|s| {
        for part in chars.chunks_mut(size) {
            s.spawn(move || eval_serial(part, clips, ticks));
        }
    });
}

/// Advance every character `ticks` ticks through the bank on the caller, in
/// index order. The poses equal [`eval_serial`] bit for bit.
pub fn eval_simd_serial(chars: &mut [Character], bank: &SoaBank, ticks: u32) {
    for c in chars {
        c.run_simd(bank, ticks);
    }
}

/// Advance every character `ticks` ticks through the bank on `pool`, one job
/// per chunk of at most `chunk` characters. A crowd that fits one chunk runs
/// inline. Equal to [`eval_simd_serial`] bit for bit for every worker count
/// and chunking.
pub fn eval_simd_parallel(
    pool: &JobPool,
    chars: &mut [Character],
    bank: &SoaBank,
    ticks: u32,
    chunk: ChunkSize,
) {
    let size = chunk.value();
    if chars.len() <= size {
        return eval_simd_serial(chars, bank, ticks);
    }
    pool.scope(|s| {
        for part in chars.chunks_mut(size) {
            s.spawn(move || eval_simd_serial(part, bank, ticks));
        }
    });
}

/// FNV-1a64 over every character's pose bits in index order.
pub fn poses_hash(chars: &[Character]) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(chars.len());
    for c in chars {
        for p in &c.pose {
            for x in p {
                h.write_f32(*x);
            }
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{bake_clip, bake_track};

    const JOINTS: usize = 40;

    /// Three clips, `JOINTS` tracks of 32 keys each, shaped per clip.
    fn clips() -> Vec<QClip> {
        (0..3u32)
            .map(|c| {
                let tracks = (0..JOINTS)
                    .map(|j| {
                        let times: Vec<f32> = (0..32).map(|k| k as f32 * 0.05).collect();
                        let values = (0..32)
                            .map(|k| {
                                let a = (k as f32 + j as f32 * 0.37) * (c + 1) as f32;
                                [a % 3.0, (a * 0.5) % 2.0, (a * 0.25) % 1.5]
                            })
                            .collect();
                        bake_track(times, values)
                    })
                    .collect();
                bake_clip(tracks)
            })
            .collect()
    }

    /// Crowd of `n` characters; machines differ by speed and switch time.
    fn crowd(n: usize, clips: &[QClip]) -> Vec<Character> {
        (0..n)
            .map(|i| {
                let mut m = Machine::new();
                m.add_state(0, true, 0.5 + (i % 5) as f32 * 0.25);
                m.add_state(1, true, 1.0);
                m.add_state(2, false, 1.5);
                m.add_trans(0, 1, 7 + (i % 11) as u32, 4 + (i % 3) as u32);
                m.add_trans(1, 2, 40 + (i % 13) as u32, 6);
                Character::new(m, clips, JOINTS)
            })
            .collect()
    }

    #[test]
    fn identical_poses_under_every_chunking_and_worker_count() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        let mut want = crowd(64, &cs);
        eval_simd_serial(&mut want, &bank, 90);
        let want = poses_hash(&want);
        for workers in [0, 1, 2, 4, 8] {
            let pool = JobPool::new(workers).expect("pool");
            for chunk in [1, 3, 7, 16, 63, 64, 100] {
                let mut got = crowd(64, &cs);
                eval_simd_parallel(&pool, &mut got, &bank, 90, ChunkSize::new(chunk));
                assert_eq!(poses_hash(&got), want, "workers {workers} chunk {chunk}");
            }
        }
    }

    #[test]
    fn identical_poses_under_every_chunking_for_the_crate_sampler() {
        let cs = clips();
        let mut want = crowd(64, &cs);
        eval_serial(&mut want, &cs, 90);
        let want = poses_hash(&want);
        for workers in [0, 1, 2, 4, 8] {
            let pool = JobPool::new(workers).expect("pool");
            for chunk in [1, 3, 7, 16, 63, 64, 100] {
                let mut got = crowd(64, &cs);
                eval_parallel(&pool, &mut got, &cs, 90, ChunkSize::new(chunk));
                assert_eq!(poses_hash(&got), want, "workers {workers} chunk {chunk}");
            }
        }
    }

    #[test]
    fn simd_poses_match_the_crate_sampler_bit_for_bit() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        let mut scalar = crowd(64, &cs);
        let mut simd = crowd(64, &cs);
        // One tick at a time: the blend ticks are the hard ones, and they
        // land inside the first hundred ticks.
        for _ in 0..240 {
            eval_serial(&mut scalar, &cs, 1);
            eval_simd_serial(&mut simd, &bank, 1);
        }
        assert_eq!(poses_hash(&simd), poses_hash(&scalar));
    }

    #[test]
    fn simd_poses_match_the_crate_sampler_over_transitions() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        // One machine, one transition, one blend: the tick that fires the
        // transition and the ticks that blend it are the ones the two paths
        // could disagree on.
        let mut machine = Machine::new();
        machine.add_state(0, true, 1.0);
        machine.add_state(1, true, 1.0);
        machine.add_trans(0, 1, 3, 5);
        let mut scalar = vec![Character::new(machine.clone(), &cs, JOINTS)];
        let mut simd = vec![Character::new(machine, &cs, JOINTS)];
        for _ in 0..40 {
            eval_serial(&mut scalar, &cs, 1);
            eval_simd_serial(&mut simd, &bank, 1);
            assert_eq!(poses_hash(&simd), poses_hash(&scalar));
        }
    }

    #[test]
    fn a_shared_clock_builds_the_flat_bank_and_a_mixed_clock_does_not() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        assert!(
            bank.clips.iter().all(|c| c.uniform.is_some()),
            "the bake writes one clock per clip"
        );
        // Tracks with their own key counts and steps: no shared clock, so the
        // per-track path carries the clip and must still equal the crate
        // sampler bit for bit.
        let mixed: Vec<QClip> = (0..2u32)
            .map(|c| {
                let tracks = (0..JOINTS)
                    .map(|j| {
                        let n = 8 + (j % 5);
                        let step = 0.04 + (j % 3) as f32 * 0.03;
                        let times: Vec<f32> = (0..n).map(|k| k as f32 * step).collect();
                        let values = (0..n)
                            .map(|k| {
                                let a = (k as f32 + j as f32 * 0.41) * (c + 1) as f32;
                                [a % 3.0, (a * 0.5) % 2.0, (a * 0.25) % 1.5]
                            })
                            .collect();
                        bake_track(times, values)
                    })
                    .collect();
                bake_clip(tracks)
            })
            .collect();
        let mixed_bank = SoaBank::new(&mixed);
        assert!(
            mixed_bank.clips.iter().all(|c| c.uniform.is_none()),
            "staggered clocks keep the per-track path"
        );
        let mut scalar = crowd(16, &mixed);
        let mut simd = crowd(16, &mixed);
        for _ in 0..120 {
            eval_serial(&mut scalar, &mixed, 1);
            eval_simd_serial(&mut simd, &mixed_bank, 1);
            assert_eq!(poses_hash(&simd), poses_hash(&scalar));
        }
    }

    #[test]
    fn a_machine_without_states_zeroes_the_pose() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        let mut chars = vec![Character::new(Machine::new(), &cs, JOINTS)];
        eval_simd_serial(&mut chars, &bank, 5);
        assert_eq!(chars[0].pose(), vec![[0.0; 3]; JOINTS]);
    }

    #[test]
    fn joints_past_the_tracks_read_zero() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        // More joints than the clip has tracks: the extra joints read zero.
        let mut machine = Machine::new();
        machine.add_state(0, true, 1.0);
        let mut scalar = vec![Character::new(machine.clone(), &cs, JOINTS + 7)];
        let mut simd = vec![Character::new(machine, &cs, JOINTS + 7)];
        eval_serial(&mut scalar, &cs, 10);
        eval_simd_serial(&mut simd, &bank, 10);
        assert_eq!(poses_hash(&simd), poses_hash(&scalar));
        assert_eq!(simd[0].pose()[JOINTS], [0.0; 3]);
        assert_ne!(simd[0].pose()[0], [0.0; 3]);
    }

    #[test]
    fn replay_is_exact_x2_and_characters_differ() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        let pool = JobPool::new(4).expect("pool");
        let run = || {
            let mut chars = crowd(64, &cs);
            eval_simd_parallel(&pool, &mut chars, &bank, 60, ChunkSize::new(8));
            chars
        };
        let (a, b) = (run(), run());
        assert_eq!(poses_hash(&a), poses_hash(&b));
        assert_ne!(
            a[0].pose(),
            a[1].pose(),
            "per-character state is not shared"
        );
    }

    #[test]
    fn eval_advances_the_pose() {
        let cs = clips();
        let bank = SoaBank::new(&cs);
        let mut chars = crowd(2, &cs);
        let before = chars[0].pose().to_vec();
        eval_serial(&mut chars, &cs, 5);
        assert_ne!(chars[0].pose(), before.as_slice());
        let mut simd = crowd(2, &cs);
        let before = simd[0].pose().to_vec();
        eval_simd_serial(&mut simd, &bank, 5);
        assert_ne!(simd[0].pose(), before.as_slice());
    }

    /// Speed report for the bar: `cargo test -p engine-anim --release --lib
    /// --features simd par_speedup -- --ignored --nocapture`. Machine
    /// dependent, so it prints the box, the crate's own serial path (the
    /// baseline), the SoA serial path and both parallel paths. The bar is
    /// read from the best chunk size; every size is printed beside it.
    #[test]
    #[ignore = "timing report, run in release on the reference CPU"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "wall clock in a test-only timing report, never in the tick"
    )]
    fn par_speedup_64_fighters() {
        use std::time::Instant;
        println!(
            "par: available_parallelism {:?}",
            std::thread::available_parallelism()
        );
        let cs = clips();
        let bank = SoaBank::new(&cs);
        let ticks = 240;
        // The fastest of nine runs. Every run starts from a fresh crowd, so
        // the workload never changes, and the hash keeps the work alive.
        let timed = |run: &mut dyn FnMut() -> u64| {
            let mut best = u64::MAX;
            let mut hash = 0;
            for _ in 0..9 {
                let start = Instant::now();
                hash = run();
                let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                best = best.min(ns);
            }
            (best, hash)
        };
        let (scalar, scalar_hash) = timed(&mut || {
            let mut chars = crowd(64, &cs);
            eval_serial(&mut chars, &cs, ticks);
            poses_hash(&chars)
        });
        println!("par: crate sampler serial {scalar} ns for 64 fighters x {ticks} ticks");
        let (soa, soa_hash) = timed(&mut || {
            let mut chars = crowd(64, &cs);
            eval_simd_serial(&mut chars, &bank, ticks);
            poses_hash(&chars)
        });
        println!(
            "par: soa serial {soa} ns, the soa sampler alone {:.2}x of the crate sampler",
            scalar as f64 / soa.max(1) as f64
        );
        assert_eq!(soa_hash, scalar_hash, "the two samplers must agree");
        let pool = JobPool::new(4).expect("pool");
        let (scalar_par, _) = timed(&mut || {
            let mut chars = crowd(64, &cs);
            eval_parallel(&pool, &mut chars, &cs, ticks, ChunkSize::new(8));
            poses_hash(&chars)
        });
        println!(
            "par: crate sampler parallel 4 workers: {scalar_par} ns, {:.2}x of its serial path",
            scalar as f64 / scalar_par.max(1) as f64
        );
        let mut bar = 0.0f64;
        for chunk in [4usize, 8, 16] {
            let (par, hash) = timed(&mut || {
                let mut chars = crowd(64, &cs);
                eval_simd_parallel(&pool, &mut chars, &bank, ticks, ChunkSize::new(chunk));
                poses_hash(&chars)
            });
            assert_eq!(hash, scalar_hash, "the parallel path must agree");
            let total = scalar as f64 / par.max(1) as f64;
            bar = bar.max(total);
            println!(
                "par: soa parallel 4 workers chunk {chunk}: {par} ns, \
                 {total:.2}x of the crate sampler serial path, the parallel part alone {:.2}x",
                soa as f64 / par.max(1) as f64
            );
        }
        assert!(
            bar >= 3.0,
            "the bar: 64 fighters on 4 workers must be at least 3x the crate sampler serial path, \
             got {bar:.2}x"
        );
    }
}
