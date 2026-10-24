//! servo_calib: the PX22 offline calibration corpus.
//!
//! It runs the PX11 servo bench's three motion cases (a hit, a partial
//! ragdoll, a 2 s get-up) on the three rigs over a seeded integer sweep of
//! the tune row, and picks one row per rig: the least rest jitter whose
//! overshoot does not grow past today's `ServoTuning::TABLE` row.
//!
//! Bake only, never the tick. The sweep is integer and seeded (the joint
//! noise is the bench's own LCG), so it reads the same numbers on every run
//! and every machine; the digest at the end proves the replay. No clock and
//! no float enters the loop or the digest: the only float is the boundary
//! `Gains` a caller already owns, and `Servo::step` converts it once.
//!
//! Metric parity with `servo::tests`: same step, same cap, same gravity and
//! contact noise, same joint counts, and jitter is `jitter_rms` over the
//! velocity deltas of a 60-tick rest window, pooled over the joints. The
//! corpus adds the overshoot the PX11 bench never read: the longest travel
//! past the target after the disturbance, in µm.
#![forbid(unsafe_code)]

use engine_core::hash::hex16;
use engine_core::Fnv64;
use engine_physics::pd;
use engine_physics::servo::{jitter_rms, Gains, Rig, Servo, ServoTuning, MICRO};

/// One 60 Hz step, µs: the bench's step.
const DT_US: i64 = 16_667;
/// Ticks per scenario: the bench's length, so the numbers compare.
const TICKS: usize = 360;
/// The disturbance lands here (the hit and the fling).
const HIT_TICK: usize = 60;
/// The get-up drives the chain again here.
const GETUP_TICK: usize = 120;
/// The disturbance's speed kick at the tip, µm/s (20 contact-noise steps).
const KICK: i32 = 60_000 * 20;
/// Per-joint displacement at the hit, µm per joint index.
const SPREAD_UM: i64 = 20_000;
/// Gravity per tick, µm/s: 9.81 m/s² / 60.
const GRAVITY_TICK: i32 = 163_500;
/// A loaded rig's contact jitter amplitude per tick, µm/s.
const LOAD_NOISE: i32 = 60_000;
/// Speed cap, µm/s.
const CAP: i32 = 8_000_000;
/// Shipped bone gains: P 12/s, D clamped inside the stability edge.
const P: f32 = 12.0;
const D: f32 = 1.0;

/// The rig, its joint count and its name for the table.
const RIGS: [(Rig, usize, &str); 3] = [
    (Rig::Standard, 4, "Standard"),
    (Rig::Heavy, 6, "Heavy"),
    (Rig::Tall, 8, "Tall"),
];

/// One motion case: which joints the servo drives, and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    /// Everything driven; the tip is struck at [`HIT_TICK`].
    Hit,
    /// Every other joint free: the partial ragdoll.
    Partial,
    /// All free, then all driven again at [`GETUP_TICK`].
    Getup,
}

impl Case {
    /// The three cases, in table order.
    const ALL: [Case; 3] = [Case::Hit, Case::Partial, Case::Getup];

    /// The case's label.
    const fn name(self) -> &'static str {
        match self {
            Case::Hit => "hit",
            Case::Partial => "partial",
            Case::Getup => "getup",
        }
    }

    /// Whether joint `i` is driven on `tick`.
    fn driven(self, tick: usize, i: usize) -> bool {
        match self {
            Case::Hit => true,
            Case::Partial => i.is_multiple_of(2),
            Case::Getup => tick >= GETUP_TICK,
        }
    }

    /// First tick of the 60-tick rest window read for jitter.
    const fn rest_from(self) -> usize {
        match self {
            Case::Hit | Case::Partial => 240,
            Case::Getup => 300,
        }
    }

    /// First tick the overshoot is read from: one past the disturbance.
    const fn over_from(self) -> usize {
        match self {
            Case::Hit | Case::Partial => HIT_TICK + 1,
            Case::Getup => GETUP_TICK + 1,
        }
    }
}

/// What one scenario run measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Reading {
    /// Rest jitter, RMS µm/s per tick per joint.
    jitter: i32,
    /// Travel past the target after the disturbance, µm.
    over_um: i64,
    /// Mean distance from the target in the rest window, µm: the droop the
    /// integral exists to remove.
    droop_um: i64,
}

impl Reading {
    /// Add another case's reading: jitter sums, the magnitudes keep their worst.
    fn add(self, other: Self) -> Self {
        Self {
            jitter: self.jitter.saturating_add(other.jitter),
            over_um: self.over_um.max(other.over_um),
            droop_um: self.droop_um.max(other.droop_um),
        }
    }

    /// Keep this reading but read the droop of `droop_um`.
    fn with_droop(self, droop_um: i64) -> Self {
        Self { droop_um, ..self }
    }
}

/// The deterministic tick noise, ±[`LOAD_NOISE`] µm/s: the bench's LCG.
fn noise(tick: usize, joint: usize) -> i32 {
    let mut x = u32::try_from(tick)
        .unwrap_or(0)
        .wrapping_mul(1_664_525)
        .wrapping_add(
            u32::try_from(joint)
                .unwrap_or(0)
                .wrapping_mul(1_013_904_223),
        )
        .wrapping_add(12_345);
    x ^= x >> 13;
    x = x.wrapping_mul(2_654_435_761);
    let span = u32::try_from(LOAD_NOISE * 2 + 1).unwrap_or(1);
    i32::try_from(x % span).unwrap_or(0) - LOAD_NOISE
}

/// The hit: the tip takes a speed kick and the chain is pushed out of line.
fn strike(pos: &mut [i64], vel: &mut [i32]) {
    if let Some(tip) = vel.last_mut() {
        *tip = tip.saturating_add(KICK);
    }
    for (i, p) in pos.iter_mut().enumerate() {
        *p += SPREAD_UM * i64::try_from(i).unwrap_or(0);
    }
}

/// One driven joint's answer for this tick: the integer servo, µ/s.
fn drive(s: &mut Servo, pos: i64, measured: i32, gains: Gains, tune: &ServoTuning) -> i32 {
    s.step(
        [-pos, 0, 0],
        [i64::from(measured), 0, 0],
        DT_US,
        gains,
        tune,
        CAP,
    )[0]
}

/// One free joint's answer for this tick: 63/64 of its rate, like an
/// articulated free node.
fn free(measured: i32) -> i32 {
    (measured * 63) / 64
}

/// Run one scenario and answer what it measured.
fn run(tune: &ServoTuning, joints: usize, gains: Gains, case: Case) -> Reading {
    let mut servos = vec![Servo::new(); joints];
    let mut pos = vec![0i64; joints];
    let mut vel = vec![0i32; joints];
    let mut last = vec![0i32; joints];
    let mut dir = vec![1i64; joints];
    let (rest, over_from) = (case.rest_from(), case.over_from());
    let mut before = Vec::with_capacity(joints * 60);
    let mut after = Vec::with_capacity(joints * 60);
    let mut over: i64 = 0;
    let (mut droop, mut samples) = (0i64, 0i64);
    for tick in 0..TICKS {
        if tick == HIT_TICK {
            strike(&mut pos, &mut vel);
        }
        if tick == over_from {
            for (d, p) in dir.iter_mut().zip(&pos) {
                *d = if *p < 0 { -1 } else { 1 };
            }
        }
        for (i, ((s, p), v)) in servos
            .iter_mut()
            .zip(pos.iter_mut())
            .zip(vel.iter_mut())
            .enumerate()
        {
            let measured = v
                .saturating_add(GRAVITY_TICK)
                .saturating_add(noise(tick, i));
            let commanded = if case.driven(tick, i) {
                drive(s, *p, measured, gains, tune)
            } else {
                free(measured)
            };
            *v = commanded.saturating_add(noise(tick, i) / 8);
            *p += i64::from(*v) * DT_US / MICRO;
            if tick > rest && tick <= rest + 60 {
                before.push(last[i]);
                after.push(*v);
                droop += p.abs();
                samples += 1;
            }
            last[i] = *v;
            if tick > over_from {
                over = over.max((-dir[i] * *p).max(0));
            }
        }
    }
    Reading {
        jitter: jitter_rms(&before, &after),
        over_um: over,
        droop_um: droop / samples.max(1),
    }
}

/// A rig's total over the three cases: jitter and overshoot as the bench
/// reads them, droop from the all-driven case, where the rest window is a
/// true steady state (the get-up's window is still recovering from the
/// fling, so its distance from the target is not a tuning error).
fn measure(tune: &ServoTuning, joints: usize, gains: Gains) -> Reading {
    let hit = run(tune, joints, gains, Case::Hit);
    let all = [Case::Partial, Case::Getup]
        .iter()
        .fold(hit, |acc, case| acc.add(run(tune, joints, gains, *case)));
    all.with_droop(hit.droop_um)
}

/// One sweep row: a tune row, the bone gains it was measured at and its reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Candidate {
    tune: ServoTuning,
    p_bits: u32,
    d_bits: u32,
    /// The bone gains are the shipped ones, so this row's tune alone carries
    /// the winner (the drive table owns P and D, not this corpus).
    shipped: bool,
    reading: Reading,
}

impl Candidate {
    /// Distance from today's row, so a tie keeps the table still.
    fn drift(&self, base: &ServoTuning) -> i64 {
        let t = self.tune;
        (i64::from(t.i_q8) - i64::from(base.i_q8)).abs()
            + (t.integ_clamp_um_s - base.integ_clamp_um_s).abs()
            + (t.integ_band_um - base.integ_band_um).abs()
            + (i64::from(t.slew_um_s_tick) - i64::from(base.slew_um_s_tick)).abs()
    }
}

/// The grid: I, the integral bound, the band, the slew and (diagnostically)
/// a P and a D scale, all around today's row. 432 rows per rig.
fn grid(base: &ServoTuning, kd: f32) -> Vec<Candidate> {
    let i = [0, base.i_q8 / 2, base.i_q8, base.i_q8 * 2];
    let clamp = [
        base.integ_clamp_um_s / 2,
        base.integ_clamp_um_s,
        base.integ_clamp_um_s * 2,
    ];
    let band = [
        base.integ_band_um / 2,
        base.integ_band_um,
        base.integ_band_um * 2,
    ];
    let slew = [
        base.slew_um_s_tick / 2,
        base.slew_um_s_tick,
        base.slew_um_s_tick * 3 / 2,
    ];
    let ps = [P.to_bits(), (P * 3.0 / 2.0).to_bits()];
    let ds = [kd.to_bits(), (kd * 5.0 / 4.0).to_bits()];
    let (p_ship, d_ship) = (P.to_bits(), kd.to_bits());
    let mut out = Vec::with_capacity(i.len() * clamp.len() * band.len() * slew.len() * 4);
    for i_q8 in i {
        for integ_clamp_um_s in clamp {
            for integ_band_um in band {
                for slew_um_s_tick in slew {
                    for p_bits in ps {
                        for d_bits in ds {
                            out.push(Candidate {
                                tune: ServoTuning {
                                    i_q8,
                                    integ_clamp_um_s,
                                    integ_band_um,
                                    slew_um_s_tick,
                                },
                                p_bits,
                                d_bits,
                                shipped: p_bits == p_ship && d_bits == d_ship,
                                reading: Reading::default(),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// Least jitter whose overshoot and droop are at or below the baseline's,
/// ties going to the row nearest today's tune.
fn pick(rows: &[Candidate], base: &ServoTuning, limit: Reading) -> Candidate {
    rows.iter()
        .filter(|r| r.reading.over_um <= limit.over_um && r.reading.droop_um <= limit.droop_um)
        .min_by_key(|r| (r.reading.jitter, r.drift(base)))
        .copied()
        .unwrap_or(rows[0])
}

/// One rig's sweep: today's reading, the best row that the rig table can
/// carry, and the best row of the whole grid (the P/D diagnostic).
fn sweep(rig: Rig, joints: usize, kd: f32) -> (Candidate, Candidate, Reading) {
    let base = ServoTuning::for_rig(rig);
    let base_reading = measure(&base, joints, Gains { p: P, d: kd });
    let mut rows = grid(&base, kd);
    for row in &mut rows {
        let gains = Gains {
            p: f32::from_bits(row.p_bits),
            d: f32::from_bits(row.d_bits),
        };
        row.reading = measure(&row.tune, joints, gains);
    }
    let free = rows
        .iter()
        .min_by_key(|r| (r.reading.jitter, r.drift(&base)))
        .copied()
        .unwrap_or(rows[0]);
    let owned: Vec<Candidate> = rows.iter().filter(|r| r.shipped).copied().collect();
    (free, pick(&owned, &base, base_reading), base_reading)
}

/// µm as mm with three decimals, integer only.
fn mm(um: i64) -> String {
    format!("{}.{:03}", um / 1000, (um % 1000).abs())
}

/// Signed change as an integer percentage of `base` (a zero base reads 0).
fn pct(now: i64, base: i64) -> i64 {
    if base == 0 {
        return 0;
    }
    ((now - base) * 100) / base
}

/// One rig's result: today's reading and the two winners.
struct RigResult {
    name: &'static str,
    base: Reading,
    owned: Candidate,
    free: Candidate,
}

impl RigResult {
    /// The emitted row's reading.
    fn landed(&self) -> Reading {
        self.owned.reading
    }

    /// Print the rig's before/after line, its row as Rust consts, and the
    /// P/D diagnostic when it beat the shipped bone gains.
    fn print(&self) {
        let (b, w, t) = (self.base, self.landed(), self.owned.tune);
        println!(
            "{}: jitter {} -> {} µm/s ({}%), overshoot {} -> {} mm ({}%), droop {} -> {} mm",
            self.name,
            b.jitter,
            w.jitter,
            pct(i64::from(w.jitter), i64::from(b.jitter)),
            mm(b.over_um),
            mm(w.over_um),
            pct(w.over_um, b.over_um),
            mm(b.droop_um),
            mm(w.droop_um)
        );
        println!(
            "  row i_q8 {} integ_clamp_um_s {} integ_band_um {} slew_um_s_tick {}",
            t.i_q8, t.integ_clamp_um_s, t.integ_band_um, t.slew_um_s_tick
        );
        let f = self.free;
        if f.reading.jitter < w.jitter {
            println!(
                "  least jitter anywhere: {} µm/s at i_q8 {} clamp {} band {} slew {} P {} D {} \
                 — droop {} mm, overshoot {} mm (rejected: it trades offset for buzz)",
                f.reading.jitter,
                f.tune.i_q8,
                f.tune.integ_clamp_um_s,
                f.tune.integ_band_um,
                f.tune.slew_um_s_tick,
                f32::from_bits(f.p_bits),
                f32::from_bits(f.d_bits),
                mm(f.reading.droop_um),
                mm(f.reading.over_um),
            );
        }
    }
}

/// The corpus: every rig swept, in table order.
fn corpus() -> Vec<RigResult> {
    let (_, kd) = pd::stable_gains(P, D, 1.0 / 60.0);
    RIGS.iter()
        .map(|(rig, joints, name)| {
            let (free, owned, base) = sweep(*rig, *joints, kd);
            RigResult {
                name,
                base,
                owned,
                free,
            }
        })
        .collect()
}

/// A digest of the emitted table and its readings, so a replay can prove it.
fn digest(rows: &[RigResult]) -> String {
    let mut h = Fnv64::new();
    for r in rows {
        let t = r.owned.tune;
        h.write_str(r.name);
        h.write_i64(i64::from(t.i_q8));
        h.write_i64(t.integ_clamp_um_s);
        h.write_i64(t.integ_band_um);
        h.write_i64(i64::from(t.slew_um_s_tick));
        h.write_i64(i64::from(r.landed().jitter));
        h.write_i64(r.landed().over_um);
        h.write_i64(r.landed().droop_um);
    }
    hex16(h.finish())
}

/// PX22's accept: the corpus replays twice to the same table, one row per
/// rig, and the picked row's jitter and overshoot are at or below today's.
#[test]
fn servo_corpus_replays_and_emits_one_row_per_rig() {
    let first = corpus();
    let one = digest(&first);
    println!("PX22 servo calibration corpus: 432 rows per rig x 3 cases x 360 ticks");
    for r in &first {
        r.print();
    }
    println!("corpus digest    {one}");
    let second = corpus();
    let two = digest(&second);
    println!("replay digest    {two}");
    assert_eq!(one, two, "the corpus must replay to the same table");
    for r in &first {
        let (b, w) = (r.base, r.landed());
        assert!(
            w.jitter <= b.jitter,
            "{}: jitter {} above today's {}",
            r.name,
            w.jitter,
            b.jitter
        );
        assert!(
            w.over_um <= b.over_um,
            "{}: overshoot {} µm above today's {} µm",
            r.name,
            w.over_um,
            b.over_um
        );
        assert!(
            w.droop_um <= b.droop_um,
            "{}: droop {} µm above today's {} µm",
            r.name,
            w.droop_um,
            b.droop_um
        );
    }
}

/// The metric the corpus reads: the bench's jitter on one case, per joint
/// count, must be a plain RMS and not a float.
#[test]
fn servo_corpus_jitter_is_the_bench_metric() {
    let t = ServoTuning::for_rig(Rig::Standard);
    let gains = Gains { p: P, d: 1.0 };
    for case in Case::ALL {
        let r = run(&t, 4, gains, case);
        assert!(r.jitter > 0, "{}: a loaded rig buzzes", case.name());
        assert!(r.over_um >= 0, "{}: overshoot is a magnitude", case.name());
    }
    // Same input, same reading: no clock and no float in the loop.
    assert_eq!(
        run(&t, 4, gains, Case::Hit),
        run(&t, 4, gains, Case::Hit),
        "a rerun reads the same"
    );
}
