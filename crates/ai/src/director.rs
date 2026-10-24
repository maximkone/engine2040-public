//! director: pacing as data. Tables + tension scalar, never hardcoded spawns.

/// Intensity budget: accumulates pressure, spends on spawns.
#[derive(Clone, Debug)]
pub struct Budget {
    pub intensity: f32,
    pub cap: f32,
}

impl Budget {
    pub fn new(cap: f32) -> Self {
        Self {
            intensity: 0.0,
            cap: cap.max(1.0),
        }
    }
    /// Add pressure (clamped to cap). Pure, deterministic.
    pub fn add(&mut self, x: f32) {
        self.intensity = (self.intensity + x.max(0.0)).min(self.cap);
    }
    /// Spend if affordable. Returns true on success.
    pub fn spend(&mut self, cost: f32) -> bool {
        if cost <= self.intensity {
            self.intensity -= cost;
            true
        } else {
            false
        }
    }
    /// Tension scalar 0..1 = intensity / cap. Never hardcoded spawns.
    pub fn tension(&self) -> f32 {
        (self.intensity / self.cap).clamp(0.0, 1.0)
    }
}

/// Spawn table row: min tension gate + cost + label. Data, not code.
#[derive(Clone, Debug)]
pub struct SpawnRow {
    pub at_tension: f32,
    pub cost: f32,
    pub label: String,
}

/// Situation-placed spawn: first affordable row whose gate <= tension, in table order.
pub fn pick_spawn(budget: &mut Budget, table: &[SpawnRow]) -> Option<String> {
    let t = budget.tension();
    for row in table {
        if t >= row.at_tension && budget.spend(row.cost) {
            return Some(row.label.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// PressureGauge (SW130): live pacing as integer permille, not a static ladder.
//
// PORT (ideas reimplemented, zero bytes vendored): zeljkovranjes
// sbox-two-brain-director `Core/PressureDirector.cs` (Tick steps 4-6: gauge
// fill/decrease, Normal->Aggressive, sweep end), `Core/PressureState.cs`
// (FillSeconds 0.5..=600, DecreaseDelaySeconds 0..=600,
// AggressiveThresholdProgression (0,1], MaxOpportunities 1..=100) and
// `tests/.../GaugeFormulaTests.cs` (fill, delay, freeze, cooldown). MIT.
//
// Units: permille progression 0..=1000, milliseconds for every duration.
// Determinism: integers only, no clock, no float, no allocation per tick, so
// the same (latch, dt) tape replays the same mode, progression and reasons on
// every machine. No shipped call site yet: a bout's director drives it (SW131).
// ---------------------------------------------------------------------------

/// Full progression, permille.
pub const GAUGE_MAX: u32 = 1000;

/// Most reasons the gauge remembers; older ones fall off the ring.
pub const REASON_RING: usize = 8;

/// Longest single step the gauge accepts, ms: a stalled frame never jumps the
/// whole fill curve in one tick.
pub const MAX_STEP_MS: u32 = 250;

/// Where the gauge is in one bout, as a four-mode cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No viable target and the gauge is at rest.
    Relax,
    /// A target is latched and the gauge is filling.
    Build,
    /// The aggressive threshold was reached: the gauge holds.
    Peak,
    /// The peak is over and the gauge decays through its cooldown.
    Fade,
}

impl Mode {
    /// Short name for reasons, logs and digests.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Relax => "relax",
            Self::Build => "build",
            Self::Peak => "peak",
            Self::Fade => "fade",
        }
    }
}

/// Gauge knobs, clamped to the donor's ranges at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaugeConfig {
    /// Time to approach full progression while latched, ms (500..=600_000).
    pub fill_ms: u32,
    /// Time to drain the whole gauge once past the grace, ms (1..=600_000).
    pub decrease_ms: u32,
    /// Grace after the latch is lost before any decay, ms (0..=600_000).
    pub decrease_delay_ms: u32,
    /// Progression that turns Build into Peak, permille (1..=1000).
    pub aggressive_threshold: u32,
    /// Most peaks one bout may have (1..=100).
    pub max_peaks: u32,
    /// Time after a peak during which the gauge never fills, ms (0..=600_000).
    pub cooldown_ms: u32,
    /// True when a peak holds the gauge instead of letting it keep filling.
    pub freeze_during_peak: bool,
}

impl Default for GaugeConfig {
    fn default() -> Self {
        Self {
            fill_ms: 2_000,
            decrease_ms: 8_000,
            decrease_delay_ms: 750,
            aggressive_threshold: 700,
            max_peaks: 3,
            cooldown_ms: 4_000,
            freeze_during_peak: true,
        }
    }
}

impl GaugeConfig {
    /// The same knobs with every field inside the donor's range, so a caller
    /// cannot ask for a zero-length fill or a hundred peaks.
    pub fn clamped(self) -> Self {
        Self {
            fill_ms: self.fill_ms.clamp(500, 600_000),
            decrease_ms: self.decrease_ms.clamp(1, 600_000),
            decrease_delay_ms: self.decrease_delay_ms.clamp(0, 600_000),
            aggressive_threshold: self.aggressive_threshold.clamp(1, GAUGE_MAX),
            max_peaks: self.max_peaks.clamp(1, 100),
            cooldown_ms: self.cooldown_ms.clamp(0, 600_000),
            freeze_during_peak: self.freeze_during_peak,
        }
    }
}

/// Bounded ring of the gauge's own reason codes, oldest first.
#[derive(Clone, Debug)]
pub struct ReasonRing {
    slots: [&'static str; REASON_RING],
    len: usize,
    next: usize,
}

impl Default for ReasonRing {
    fn default() -> Self {
        Self {
            slots: [""; REASON_RING],
            len: 0,
            next: 0,
        }
    }
}

impl ReasonRing {
    /// Every reason the ring still holds, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        let len = self.len;
        let start = if len == REASON_RING { self.next } else { 0 };
        (0..len).map(move |offset| self.slots[(start + offset) % REASON_RING])
    }

    /// Newest reason, when the ring holds one.
    pub fn newest(&self) -> Option<&'static str> {
        if self.len == 0 {
            return None;
        }
        Some(self.slots[(self.next + REASON_RING - 1) % REASON_RING])
    }

    /// True when the ring still holds this reason.
    pub fn contains(&self, reason: &str) -> bool {
        self.iter().any(|held| held == reason)
    }

    /// Reasons held, at most [`REASON_RING`].
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when nothing was noted yet.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push(&mut self, reason: &'static str) {
        // A repeated reason adds no line twice in a row: a stuck latch must
        // not flush a quota refusal out of the ring.
        if self.newest() == Some(reason) {
            return;
        }
        self.slots[self.next] = reason;
        self.next = (self.next + 1) % REASON_RING;
        self.len = (self.len + 1).min(REASON_RING);
    }
}

/// Live pacing: integer permille progression that fills while a viable target
/// is latched, holds through its grace, decays afterwards, freezes during a
/// peak and caps how many peaks one bout may have.
#[derive(Clone, Debug)]
pub struct PressureGauge {
    cfg: GaugeConfig,
    progression: u32,
    mode: Mode,
    /// Ms since the latch was lost; 0 while latched.
    since_lost_ms: u32,
    peaks: u32,
    cooldown_left_ms: u32,
    reasons: ReasonRing,
}

impl PressureGauge {
    /// Fresh gauge at rest under these knobs (clamped to their ranges).
    pub fn new(cfg: GaugeConfig) -> Self {
        Self {
            cfg: cfg.clamped(),
            progression: 0,
            mode: Mode::Relax,
            since_lost_ms: 0,
            peaks: 0,
            cooldown_left_ms: 0,
            reasons: ReasonRing::default(),
        }
    }

    /// Progression now, permille `0..=1000`.
    pub fn progression(&self) -> u32 {
        self.progression
    }

    /// Mode now.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Peaks this bout has spent from its quota.
    pub fn peaks(&self) -> u32 {
        self.peaks
    }

    /// Peaks the quota allows.
    pub fn max_peaks(&self) -> u32 {
        self.cfg.max_peaks
    }

    /// The bounded reason ring, oldest first.
    pub fn reasons(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.reasons.iter()
    }

    /// Advance one step: `latched` says a viable target is in play now, `dt_ms`
    /// is the step length. Integer permille only.
    pub fn tick(&mut self, latched: bool, dt_ms: u32) {
        let dt = dt_ms.min(MAX_STEP_MS);
        if latched {
            self.since_lost_ms = 0;
        } else if self.since_lost_ms == 0 {
            self.reasons.push("latch_lost");
            self.since_lost_ms = self.since_lost_ms.saturating_add(dt);
        } else {
            self.since_lost_ms = self.since_lost_ms.saturating_add(dt);
        }
        match self.mode {
            Mode::Peak => self.tick_peak(latched, dt),
            Mode::Fade => self.tick_fade(latched, dt),
            Mode::Relax | Mode::Build => self.tick_build(latched, dt),
        }
    }

    /// Peak: hold (or keep filling when the freeze is off) until the target is
    /// lost, then hand over to the cooldown.
    fn tick_peak(&mut self, latched: bool, dt: u32) {
        if self.cfg.freeze_during_peak {
            self.reasons.push("peak_frozen");
        } else {
            self.fill(dt);
        }
        if !latched {
            self.mode = Mode::Fade;
            self.cooldown_left_ms = self.cfg.cooldown_ms;
            self.reasons.push("peak_end");
        }
    }

    /// Fade: never fills while the cooldown runs; decays once past the grace.
    fn tick_fade(&mut self, latched: bool, dt: u32) {
        self.cooldown_left_ms = self.cooldown_left_ms.saturating_sub(dt);
        self.hold_or_decay(dt);
        if self.cooldown_left_ms > 0 {
            self.reasons.push("cooldown");
            return;
        }
        if latched {
            self.mode = Mode::Build;
            self.reasons.push("latched");
            self.fill(dt);
            self.maybe_peak();
        } else if self.progression == 0 {
            self.mode = Mode::Relax;
            self.reasons.push("relaxed");
        }
    }

    /// Relax/Build: fill while latched, hold through the grace, decay after it.
    fn tick_build(&mut self, latched: bool, dt: u32) {
        if latched {
            if self.mode == Mode::Relax {
                self.mode = Mode::Build;
                self.reasons.push("latched");
            }
            self.fill(dt);
            self.maybe_peak();
            return;
        }
        self.hold_or_decay(dt);
        if self.mode == Mode::Build && self.progression < self.cfg.aggressive_threshold {
            self.mode = Mode::Fade;
        }
        if self.progression == 0 {
            self.mode = Mode::Relax;
            self.reasons.push("relaxed");
        }
    }

    /// Fill curve: `p + (1000 - p) * dt / fill_ms`, asymptotic, never past
    /// [`GAUGE_MAX`].
    fn fill(&mut self, dt: u32) {
        let room = GAUGE_MAX - self.progression;
        let step = u64::from(room) * u64::from(dt) / u64::from(self.cfg.fill_ms);
        let step = u32::try_from(step).unwrap_or(room);
        self.progression = (self.progression + step).min(GAUGE_MAX);
    }

    /// Grace then decay: the gauge holds for [`GaugeConfig::decrease_delay_ms`]
    /// and drains `1000 * dt / decrease_ms` permille a step after it, never
    /// below zero.
    fn hold_or_decay(&mut self, dt: u32) {
        if self.since_lost_ms <= self.cfg.decrease_delay_ms {
            self.reasons.push("grace");
            return;
        }
        let step = 1000u64 * u64::from(dt) / u64::from(self.cfg.decrease_ms);
        let step = u32::try_from(step).unwrap_or(GAUGE_MAX);
        self.progression = self.progression.saturating_sub(step);
        self.reasons.push("decay");
    }

    /// Build -> Peak once the threshold is reached, while the quota allows it;
    /// a spent quota notes `quota_blocked` and leaves the gauge in Build.
    fn maybe_peak(&mut self) {
        if self.mode != Mode::Build || self.progression < self.cfg.aggressive_threshold {
            return;
        }
        if self.peaks >= self.cfg.max_peaks {
            self.reasons.push("quota_blocked");
            return;
        }
        self.peaks += 1;
        self.mode = Mode::Peak;
        self.reasons.push("peak");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_clamps_and_tension() {
        let mut b = Budget::new(100.0);
        b.add(40.0);
        assert_eq!(b.tension(), 0.4);
        b.add(999.0);
        assert_eq!(b.tension(), 1.0);
        assert!(b.spend(60.0));
        assert!(!b.spend(60.0));
    }

    #[test]
    fn spawn_tables_not_hardcode() {
        let table = vec![
            SpawnRow {
                at_tension: 0.8,
                cost: 60.0,
                label: "horde".into(),
            },
            SpawnRow {
                at_tension: 0.3,
                cost: 20.0,
                label: "scout".into(),
            },
        ];
        let mut b = Budget::new(100.0);
        b.add(35.0);
        assert_eq!(pick_spawn(&mut b, &table), Some("scout".into()));
        assert_eq!(pick_spawn(&mut b, &table), None); // spent, tension dropped
        b.add(90.0);
        assert_eq!(pick_spawn(&mut b, &table), Some("horde".into()));
    }

    #[test]
    fn determinism_x2() {
        let run = || {
            let mut b = Budget::new(100.0);
            b.add(50.0);
            (b.tension(), b.spend(20.0), b.tension())
        };
        assert_eq!(format!("{:?}", run()), format!("{:?}", run()));
    }

    /// Config for the two Accept cases: a clean fill, a grace, a quota.
    fn gauge_config() -> GaugeConfig {
        GaugeConfig {
            fill_ms: 2_000,
            decrease_ms: 2_000,
            decrease_delay_ms: 500,
            aggressive_threshold: GAUGE_MAX,
            max_peaks: 2,
            cooldown_ms: 400,
            freeze_during_peak: true,
        }
    }

    /// SW130 Accept 1: the fill is asymptotic (monotone, always under 1000),
    /// a lost latch holds through its grace, then decays to 0 and never below.
    #[test]
    fn pressure_gauge_fills_asymptotically_clamps_and_decays_after_grace() {
        let mut gauge = PressureGauge::new(gauge_config());
        let mut last = 0;
        for _ in 0..120 {
            gauge.tick(true, 16);
            let now = gauge.progression();
            assert!(now > last, "the fill is monotone: {last} -> {now}");
            assert!(
                now < GAUGE_MAX,
                "the fill approaches 1000, never reaches it"
            );
            last = now;
        }
        let filled = gauge.progression();
        assert!(
            filled > 400,
            "120 ticks of a 2 s fill build real pressure: {filled}"
        );
        assert_eq!(gauge.mode(), Mode::Build);
        assert_eq!(gauge.peaks(), 0, "the threshold is 1000, so no peak yet");

        // Grace: the gauge holds while the latch is recently lost (500 ms).
        gauge.tick(false, 16);
        assert_eq!(gauge.progression(), filled, "the first grace step holds");
        for _ in 0..30 {
            gauge.tick(false, 16);
            assert_eq!(
                gauge.progression(),
                filled,
                "the whole grace delay holds the gauge"
            );
        }
        // Past the grace the gauge drains, and never below zero.
        let mut seen = filled;
        for _ in 0..200 {
            gauge.tick(false, 16);
            assert!(gauge.progression() <= seen, "the decay never rises");
            seen = gauge.progression();
        }
        assert_eq!(gauge.progression(), 0, "the gauge reaches zero");
        assert_eq!(gauge.mode(), Mode::Relax);
        assert!(
            gauge.reasons().any(|reason| reason == "decay"),
            "the ring names the decay"
        );
        assert!(
            gauge.reasons().count() <= REASON_RING,
            "the ring is bounded"
        );
    }

    /// SW130 Accept 2: a peak freezes the gauge, the quota stops the third
    /// peak, and the reason ring names `quota_blocked`.
    #[test]
    fn peak_freezes_gauge_and_quota_caps_cycles() {
        let cfg = GaugeConfig {
            aggressive_threshold: 200,
            fill_ms: 500,
            decrease_ms: 200,
            decrease_delay_ms: 0,
            ..gauge_config()
        };
        let mut gauge = PressureGauge::new(cfg);
        // Latch until the first peak: the gauge reaches the threshold and stops.
        let mut guard = 0;
        while gauge.mode() != Mode::Peak {
            gauge.tick(true, 16);
            guard += 1;
            assert!(guard < 200, "the first peak arrives");
        }
        assert_eq!(gauge.peaks(), 1);
        let at_peak = gauge.progression();
        for _ in 0..20 {
            gauge.tick(true, 16);
            assert_eq!(gauge.progression(), at_peak, "a peak freezes the gauge");
        }
        // Peak over: cooldown holds the gauge down even while latched.
        gauge.tick(false, 16);
        assert_eq!(gauge.mode(), Mode::Fade);
        let cooled = gauge.progression();
        for _ in 0..10 {
            gauge.tick(true, 16);
            assert!(
                gauge.progression() <= cooled,
                "the cooldown never fills the gauge"
            );
        }
        // Second cycle: latch through the cooldown into the second peak.
        let mut guard = 0;
        while gauge.peaks() < 2 {
            gauge.tick(true, 16);
            guard += 1;
            assert!(guard < 400, "the second peak arrives");
        }
        assert_eq!(gauge.mode(), Mode::Peak);
        gauge.tick(false, 16);
        for _ in 0..40 {
            gauge.tick(false, 16);
        }
        // Third cycle: the quota is spent, so the gauge builds but never peaks.
        let mut guard = 0;
        while gauge.progression() < cfg.aggressive_threshold && guard < 200 {
            gauge.tick(true, 16);
            guard += 1;
        }
        assert_eq!(gauge.mode(), Mode::Build, "no third peak past the quota");
        assert_eq!(gauge.peaks(), 2);
        assert!(
            gauge.reasons().any(|reason| reason == "quota_blocked"),
            "the ring names the quota refusal: {:?}",
            gauge.reasons().collect::<Vec<_>>()
        );
    }

    /// The gauge is integer-only, bounded and clamps hostile knobs: no float,
    /// no unbounded history, no zero-length fill.
    #[test]
    fn pressure_gauge_knobs_clamp_and_the_ring_stays_bounded() {
        let wild = GaugeConfig {
            fill_ms: 0,
            decrease_ms: 0,
            decrease_delay_ms: u32::MAX,
            aggressive_threshold: 0,
            max_peaks: 0,
            cooldown_ms: u32::MAX,
            freeze_during_peak: true,
        }
        .clamped();
        assert_eq!(wild.fill_ms, 500, "a zero fill clamps up");
        assert_eq!(wild.decrease_ms, 1);
        assert_eq!(wild.decrease_delay_ms, 600_000);
        assert_eq!(wild.aggressive_threshold, 1, "(0, 1] keeps one permille");
        assert_eq!(wild.max_peaks, 1, "a bout always allows one peak");

        let mut gauge = PressureGauge::new(GaugeConfig::default());
        for step in 0..40 {
            gauge.tick(step % 3 != 0, 16);
        }
        assert_eq!(gauge.reasons().count(), REASON_RING, "the ring is bounded");
        assert!(gauge.progression() <= GAUGE_MAX);
        // Same tape twice: same progression, mode and reasons.
        let tape = || {
            let mut gauge = PressureGauge::new(GaugeConfig::default());
            for step in 0..40 {
                gauge.tick(step % 3 != 0, 16);
            }
            (
                gauge.progression(),
                gauge.mode(),
                gauge.reasons().collect::<Vec<_>>(),
            )
        };
        assert_eq!(tape(), tape(), "a replay without a model drifts");
    }
}
