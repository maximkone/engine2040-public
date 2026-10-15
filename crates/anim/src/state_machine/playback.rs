//! Run-time half of the state machine: integer tick, fade, travel.
//!
//! One [`Playback`] per fighter; all fighters share one [`CompiledMachine`].
//! `tick` allocates nothing. The whole state is plain integers, so a clone is
//! a snapshot and [`Playback::digest`] is the replay proof.

use std::sync::Arc;

use engine_core::Fnv64;

use super::{
    AdvanceMode, CompiledMachine, Curve, PlayError, Source, StateDef, SwitchMode, RATE_Q8_MAX,
    RATE_Q8_ONE, RATE_Q8_PARAM, WEIGHT_ONE,
};
use crate::graph::{blend_poses, GraphError, Pose};

/// 0.15 s at 60 Hz, inside the row 0.1-0.2 s band.
pub const FIGHTER_CROSSFADE_TICKS: u32 = 9;

/// Fighter crossfade length in ticks, for the game slice to reuse.
pub fn fighter_crossfade_ticks() -> u32 {
    FIGHTER_CROSSFADE_TICKS
}

/// A fade in progress: the state being left keeps playing while it fades.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fade {
    from: usize,
    from_age: u32,
    elapsed: u32,
    total: u32,
    curve: Curve,
}

/// The state being faded out, at its playback position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FadeFrom {
    /// State index.
    pub state: usize,
    /// Playback position inside the state, in ticks.
    pub phase: u32,
}

/// What to show this tick: the current state, optionally a fade from another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StateSample {
    /// Current state index.
    pub state: usize,
    /// Playback position inside the current state, in ticks.
    pub phase: u32,
    /// State being left, while a fade runs.
    pub from: Option<FadeFrom>,
    /// Weight of the current state in `0..=WEIGHT_ONE`.
    pub weight: u32,
}

/// What one tick did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TickReport {
    /// Transition index taken this tick.
    pub taken: Option<usize>,
    /// A fade finished this tick.
    pub fade_done: bool,
}

/// Run-time state of one machine instance.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Playback {
    machine: Arc<CompiledMachine>,
    cur: usize,
    age: u32,
    /// Banked Q8 fraction of a tick toward the next age step: carries the
    /// remainder while `rate_q8` runs below 1x. Reset on every state entry
    /// and folded into [`Playback::digest`], so the bank is replay state.
    frac: u16,
    fade: Option<Fade>,
    travel: Vec<usize>,
    params: Vec<i64>,
    tick: u32,
}

fn phase_of(state: &StateDef, age: u32) -> u32 {
    if state.looping {
        age % state.length_ticks
    } else {
        age.min(state.length_ticks)
    }
}

/// Fade weight for `x = elapsed / total` in `0..=WEIGHT_ONE`, integer only.
pub(super) fn curve_weight(curve: Curve, elapsed: u32, total: u32) -> u32 {
    if total == 0 || elapsed >= total {
        return WEIGHT_ONE;
    }
    let x = u64::from(elapsed) * u64::from(WEIGHT_ONE) / u64::from(total);
    let one = u64::from(WEIGHT_ONE);
    let w = match curve {
        Curve::Linear => x,
        Curve::EaseIn => x * x / one,
        Curve::EaseOut => one - (one - x) * (one - x) / one,
        Curve::EaseInOut => (3 * x * x * one - 2 * x * x * x) / (one * one),
    };
    u32::try_from(w.min(one)).unwrap_or(WEIGHT_ONE)
}

impl Playback {
    /// Start at the machine's start state with default parameters.
    pub fn new(machine: Arc<CompiledMachine>) -> Self {
        let params = machine.def.params.iter().map(|p| p.default).collect();
        let cur = machine.start;
        Self {
            machine,
            cur,
            age: 0,
            frac: 0,
            fade: None,
            travel: Vec::new(),
            params,
            tick: 0,
        }
    }

    /// Current state index.
    pub fn state(&self) -> usize {
        self.cur
    }

    /// Name of the current state.
    pub fn state_name(&self) -> &str {
        &self.machine.def.states[self.cur].name
    }

    /// Ticks run so far.
    pub fn ticks(&self) -> u32 {
        self.tick
    }

    /// True while a travel path still has edges to take.
    pub fn travelling(&self) -> bool {
        !self.travel.is_empty()
    }

    /// Set a parameter by name, inside its range.
    pub fn set_param(&mut self, name: &str, value: i64) -> Result<(), PlayError> {
        let i = self
            .machine
            .def
            .param_index(name)
            .ok_or_else(|| PlayError::UnknownParam(name.to_string()))?;
        let p = &self.machine.def.params[i];
        if value < p.min || value > p.max {
            return Err(PlayError::ParamOutOfRange {
                name: name.to_string(),
                value,
            });
        }
        self.params[i] = value;
        Ok(())
    }

    /// Read a parameter by name.
    pub fn param(&self, name: &str) -> Option<i64> {
        self.machine.def.param_index(name).map(|i| self.params[i])
    }

    /// Jump to `name` at tick zero with no fade; cancels travel.
    pub fn start(&mut self, name: &str) -> Result<(), PlayError> {
        let i = self
            .machine
            .def
            .state_index(name)
            .ok_or_else(|| PlayError::UnknownState(name.to_string()))?;
        self.cur = i;
        self.age = 0;
        self.frac = 0;
        self.fade = None;
        self.travel.clear();
        Ok(())
    }

    /// Plan the shortest legal path to `target` and follow it, one edge each
    /// time its switch mode allows. Starts from where the current fade ends.
    /// Automatic transitions are held back while a path is being followed.
    pub fn travel(&mut self, target: &str) -> Result<(), PlayError> {
        let to = self
            .machine
            .def
            .state_index(target)
            .ok_or_else(|| PlayError::UnknownState(target.to_string()))?;
        self.travel = self.machine.travel_path(self.cur, to)?;
        Ok(())
    }

    /// Swap in a new compiled machine (hot reload). The state is kept by name
    /// when it still exists, else the new start; parameters keep their value
    /// by name, clamped to the new range; fade and travel are dropped.
    pub fn rebind(&mut self, machine: Arc<CompiledMachine>) {
        let name = self.machine.def.states[self.cur].name.clone();
        let params: Vec<i64> = machine
            .def
            .params
            .iter()
            .map(|p| {
                self.param(&p.name)
                    .map_or(p.default, |v| v.clamp(p.min, p.max))
            })
            .collect();
        match machine.def.state_index(&name) {
            Some(i) => self.cur = i,
            None => {
                self.cur = machine.start;
                self.age = 0;
                self.frac = 0;
            }
        }
        self.fade = None;
        self.travel.clear();
        self.params = params;
        self.machine = machine;
    }

    /// Advance one tick: clip ages move at the `rate_q8` parameter when the
    /// machine declares it (Q8 units, 256 reads 1x, absent reads 1x, clamped
    /// to 0..=1024), then at most one transition starts. The fractional bank
    /// keeps 0.5x exact across ticks; fades stay on wall ticks while the
    /// leaving age scales with the rate. Integer math only, no allocation.
    pub fn tick(&mut self) -> TickReport {
        let mut report = TickReport::default();
        self.tick = self.tick.saturating_add(1);
        let rate = self
            .param(RATE_Q8_PARAM)
            .unwrap_or(RATE_Q8_ONE)
            .clamp(0, RATE_Q8_MAX);
        let total = i64::from(self.frac) + rate;
        let step = u32::try_from(total / 256).unwrap_or(u32::MAX);
        self.frac = u16::try_from(total % 256).unwrap_or(u16::MAX);
        self.age = self.age.saturating_add(step);
        if let Some(fade) = &mut self.fade {
            fade.from_age = fade.from_age.saturating_add(step);
            fade.elapsed += 1;
            if fade.elapsed >= fade.total {
                self.fade = None;
                report.fade_done = true;
            }
        }
        if self.fade.is_none() {
            if let Some(e) = self.pick() {
                self.take(e);
                report.taken = Some(e);
            }
        }
        report
    }

    fn at_end(&self, fade: u32) -> bool {
        let state = &self.machine.def.states[self.cur];
        if state.looping {
            let rem = state.length_ticks - self.age % state.length_ticks;
            rem <= fade
                || (fade == 0 && self.age > 0 && self.age.is_multiple_of(state.length_ticks))
        } else {
            state.length_ticks.saturating_sub(self.age) <= fade
        }
    }

    fn permits(&self, e: usize) -> bool {
        let t = &self.machine.trans[e];
        match t.switch {
            SwitchMode::Immediate | SwitchMode::Sync => true,
            SwitchMode::AtEnd => self.at_end(t.fade),
        }
    }

    fn pick(&self) -> Option<usize> {
        if let Some(&e) = self.travel.first() {
            return self.permits(e).then_some(e);
        }
        self.machine.out[self.cur].iter().copied().find(|&e| {
            let t = &self.machine.trans[e];
            t.advance == AdvanceMode::Auto && self.permits(e) && t.cond.eval(&self.params, self.age)
        })
    }

    fn take(&mut self, e: usize) {
        let (from, to, switch, fade, curve, reset) = {
            let t = &self.machine.trans[e];
            (t.from, t.to, t.switch, t.fade, t.curve, t.reset)
        };
        let states = &self.machine.def.states;
        let from_age = self.age;
        let new_age = match switch {
            SwitchMode::Sync => {
                let phase = u64::from(phase_of(&states[from], from_age));
                let scaled = phase * u64::from(states[to].length_ticks)
                    / u64::from(states[from].length_ticks);
                u32::try_from(scaled).unwrap_or(u32::MAX)
            }
            _ if reset => 0,
            _ => from_age.min(states[to].length_ticks.saturating_sub(1)),
        };
        if self.travel.first() == Some(&e) {
            self.travel.remove(0);
        }
        self.cur = to;
        self.age = new_age;
        self.frac = 0;
        self.fade = (fade > 0).then_some(Fade {
            from,
            from_age,
            elapsed: 0,
            total: fade,
            curve,
        });
    }

    /// What to show now.
    pub fn sample(&self) -> StateSample {
        let states = &self.machine.def.states;
        let (from, weight) = match &self.fade {
            Some(f) => (
                Some(FadeFrom {
                    state: f.from,
                    phase: phase_of(&states[f.from], f.from_age),
                }),
                curve_weight(f.curve, f.elapsed, f.total),
            ),
            None => (None, WEIGHT_ONE),
        };
        StateSample {
            state: self.cur,
            phase: phase_of(&states[self.cur], self.age),
            from,
            weight,
        }
    }

    /// Source of state `i`.
    pub fn source(&self, i: usize) -> Option<Source> {
        self.machine.def.states.get(i).map(|s| s.source)
    }

    /// Build the pose for this tick: `eval` fills one state's pose at its
    /// phase; a fade blends `scratch[0]` (leaving) into `scratch[1]` (current)
    /// with the integer weight (exact in `f32`). No allocation.
    pub fn pose(
        &self,
        mut eval: impl FnMut(Source, u32, &mut Pose) -> Result<(), GraphError>,
        scratch: &mut [Pose; 2],
        out: &mut Pose,
    ) -> Result<(), GraphError> {
        let s = self.sample();
        let cur = self.source(s.state).unwrap_or(Source::Clip(0));
        let Some(from) = s.from else {
            return eval(cur, s.phase, out);
        };
        let src = self.source(from.state).unwrap_or(Source::Clip(0));
        let [leaving, current] = scratch;
        eval(src, from.phase, leaving)?;
        eval(cur, s.phase, current)?;
        blend_poses(out, leaving, current, weight_f32(s.weight))
    }

    /// FNV over the whole run-time state plus the machine digest.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(self.machine.digest);
        h.write_u32(u32::try_from(self.cur).unwrap_or(u32::MAX));
        h.write_u32(self.age);
        h.write_u16(self.frac);
        h.write_u32(self.tick);
        match &self.fade {
            None => h.write_u8(0),
            Some(f) => {
                h.write_u8(1);
                h.write_u32(u32::try_from(f.from).unwrap_or(u32::MAX));
                h.write_u32(f.from_age);
                h.write_u32(f.elapsed);
                h.write_u32(f.total);
            }
        }
        h.write_u32(u32::try_from(self.travel.len()).unwrap_or(u32::MAX));
        for e in &self.travel {
            h.write_u32(u32::try_from(*e).unwrap_or(u32::MAX));
        }
        for p in &self.params {
            h.write_i64(*p);
        }
        h.finish()
    }
}

/// Fixed-point weight as `f32`: `w / 4096` is exact.
fn weight_f32(weight: u32) -> f32 {
    let w = u16::try_from(weight).unwrap_or(u16::MAX);
    f32::from(w) / 4096.0
}

#[cfg(test)]
mod rate_tests {
    use super::super::{
        CmpOp, CompiledMachine, CondOf, ParamDef, Source, StateDef, StateMachineDef, TransitionDef,
        RATE_Q8_MAX, RATE_Q8_ONE, RATE_Q8_PARAM,
    };
    use super::*;
    use std::sync::Arc;

    fn rate_machine() -> Arc<CompiledMachine> {
        let mut def = StateMachineDef::new("rate", "run");
        def.params.push(ParamDef {
            name: RATE_Q8_PARAM.into(),
            default: RATE_Q8_ONE,
            min: 0,
            max: RATE_Q8_MAX,
        });
        def.params.push(ParamDef {
            name: "gate".into(),
            default: 0,
            min: 0,
            max: 1,
        });
        def.states.push(StateDef {
            name: "run".into(),
            source: Source::Clip(0),
            length_ticks: 60,
            looping: true,
        });
        def.states.push(StateDef {
            name: "idle".into(),
            source: Source::Clip(1),
            length_ticks: 30,
            looping: true,
        });
        let mut edge = TransitionDef::new("run", "idle");
        edge.condition = CondOf::Param {
            param: "gate".into(),
            op: CmpOp::Eq,
            value: 1,
        };
        edge.fade_ticks = 6;
        edge.priority = 1;
        def.transitions.push(edge);
        Arc::new(CompiledMachine::new(&def).expect("rate fixture validates"))
    }

    fn player() -> Playback {
        Playback::new(rate_machine())
    }

    #[test]
    fn rate_absent_reads_1x() {
        let mut def = StateMachineDef::new("plain", "run");
        def.states.push(StateDef {
            name: "run".into(),
            source: Source::Clip(0),
            length_ticks: 60,
            looping: true,
        });
        let mut play = Playback::new(Arc::new(
            CompiledMachine::new(&def).expect("plain fixture validates"),
        ));
        assert_eq!(play.param(RATE_Q8_PARAM), None);
        for _ in 0..10 {
            play.tick();
        }
        assert_eq!(play.sample().phase, 10);
    }

    #[test]
    fn rate_double_advances_two_per_tick() {
        let mut play = player();
        play.set_param(RATE_Q8_PARAM, 2 * RATE_Q8_ONE)
            .expect("in range");
        for _ in 0..5 {
            play.tick();
        }
        assert_eq!(play.sample().phase, 10);
    }

    #[test]
    fn rate_half_banks_the_fraction() {
        let mut play = player();
        play.set_param(RATE_Q8_PARAM, RATE_Q8_ONE / 2)
            .expect("in range");
        play.tick();
        assert_eq!(play.sample().phase, 0);
        play.tick();
        assert_eq!(play.sample().phase, 1);
        play.tick();
        play.tick();
        assert_eq!(play.sample().phase, 2);
    }

    #[test]
    fn rate_zero_freezes_age_but_not_the_clock() {
        let mut play = player();
        play.set_param(RATE_Q8_PARAM, 0).expect("in range");
        for _ in 0..3 {
            play.tick();
        }
        assert_eq!(play.sample().phase, 0);
        assert_eq!(play.ticks(), 3);
    }

    #[test]
    fn rate_unknown_name_is_refused() {
        let mut play = player();
        assert!(play.set_param("nope", 1).is_err());
        assert_eq!(play.param("nope"), None);
    }

    #[test]
    fn rate_start_clears_the_banked_fraction() {
        let mut play = player();
        play.set_param(RATE_Q8_PARAM, RATE_Q8_ONE / 2)
            .expect("in range");
        play.tick();
        play.start("run").expect("state exists");
        play.tick();
        assert_eq!(play.sample().phase, 0);
    }

    #[test]
    fn rate_take_clears_the_banked_fraction() {
        let mut play = player();
        play.set_param(RATE_Q8_PARAM, RATE_Q8_ONE + RATE_Q8_ONE / 2)
            .expect("in range");
        play.set_param("gate", 1).expect("gate opens");
        play.tick();
        assert_eq!(play.state_name(), "idle");
        play.set_param("gate", 0).expect("gate shuts");
        play.tick();
        assert_eq!(play.sample().phase, 1);
    }

    #[test]
    fn rate_digest_folds_the_fraction() {
        let mut half = player();
        half.set_param(RATE_Q8_PARAM, RATE_Q8_ONE / 2)
            .expect("in range");
        half.tick();
        let mut reset = player();
        reset
            .set_param(RATE_Q8_PARAM, RATE_Q8_ONE / 2)
            .expect("in range");
        reset.tick();
        reset.start("run").expect("state exists");
        // Same tick count, same age, same parameters: only the banked
        // fraction differs, so equal digests would drop replay state.
        assert_ne!(half.digest(), reset.digest());
        assert_eq!(half.digest(), half.digest());
    }

    #[test]
    fn rate_scales_the_leaving_age_inside_a_fade() {
        let mut play = player();
        play.set_param(RATE_Q8_PARAM, 2 * RATE_Q8_ONE)
            .expect("double");
        play.set_param("gate", 1).expect("gate opens");
        play.tick();
        play.tick();
        let sample = play.sample();
        let from = sample.from.expect("fade runs");
        assert_eq!(from.phase, 4);
    }
}
