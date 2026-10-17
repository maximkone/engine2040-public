//! par: per-character animation eval on the RC3 job pool.
//!
//! Owner: this module; presentation and bake callers fan a crowd of
//! [`Character`]s out in index-ordered chunks, the sim tick never calls in.
//! Units: ticks (`u32`), joints as `[f32; 3]` positions. Determinism: every
//! character owns its machine, cursors and pose, so a job touches nothing
//! shared but the read-only clip bank; the pool decides where a chunk runs,
//! never what it writes. Same input gives bit-identical poses for any worker
//! count and any chunking (proven below against the serial path).

use engine_core::Fnv64;
use engine_ecs::jobs::ChunkSize;
use engine_ecs::JobPool;

use crate::blend::Machine;
use crate::clip::{Cursor, QClip};

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
    fn replay_is_exact_x2_and_characters_differ() {
        let cs = clips();
        let pool = JobPool::new(4).expect("pool");
        let run = || {
            let mut chars = crowd(64, &cs);
            eval_parallel(&pool, &mut chars, &cs, 60, ChunkSize::new(8));
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
        let mut chars = crowd(2, &cs);
        let before = chars[0].pose().to_vec();
        eval_serial(&mut chars, &cs, 5);
        assert_ne!(chars[0].pose(), before.as_slice());
    }

    /// Speed report for the AM14 bar: `cargo test -p engine-anim --release
    /// --lib par_speedup -- --ignored --nocapture`. Machine dependent, so it
    /// prints the ratio and asserts only that the parallel path is no slower
    /// than half the serial speed.
    #[test]
    #[ignore = "timing report, run in release on the reference CPU"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "wall clock in a test-only timing report, never in the tick"
    )]
    fn par_speedup_64_fighters() {
        use std::time::Instant;
        let cs = clips();
        let ticks = 240;
        let best = |f: &mut dyn FnMut()| {
            (0..7)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_nanos()
                })
                .min()
                .unwrap_or(0)
        };
        let mut chars = crowd(64, &cs);
        let serial = best(&mut || eval_serial(&mut chars, &cs, ticks));
        println!("par: serial {serial} ns for 64 fighters x {ticks} ticks");
        let mut ratio = 0.0;
        for workers in [3, 4] {
            let pool = JobPool::new(workers).expect("pool");
            for chunk in [4, 8, 16] {
                let mut chars = crowd(64, &cs);
                let par = best(&mut || {
                    eval_parallel(&pool, &mut chars, &cs, ticks, ChunkSize::new(chunk));
                });
                ratio = serial as f64 / par.max(1) as f64;
                println!("par: workers {workers} chunk {chunk}: {par} ns, speedup {ratio:.2}x");
            }
        }
        assert!(
            ratio > 0.5,
            "parallel eval slower than half serial: {ratio:.2}x"
        );
    }
}
