//! showcase: a scripted tour of the systems the arena already plays, one station at a time.
//!
//! Owner 2026-10-24: "how can we show all of our shits we made on a demo ... i afraid we can
//! forget these cool stuff". `arena.exe --showcase` (and `play-showcase.bat`) hands slot 0 to
//! [`Showcase`]: it walks the human through the stations in order, each with a caption for the
//! window title, and grades each one from what the match reports (spear phases, spear events,
//! fight cues). The board that lists every system and whether it is in the tour yet is
//! `docs/plan/showcase.md`; a system joins the tour as one more [`Station`].
//!
//! Stations, in order: the yank (hold back as the spear sticks, SG9), the carry (the spear
//! drags both fighters on, SG1), the rope cut (the dummy's blade cuts the carry's rope, SG13),
//! the noose (a loop over the dummy, then the shove with the scutum, SG11), the ruins (walk the
//! breach the `world.rewrite` generator cut in a ruined wall, MJ2), the wall swing (a
//! world-aimed throw on the rope, SG4/SG5), three clean sword hits (combat, the toned hit
//! reactions), the finisher (a knockdown into the toned ragdoll), the trace (the bot, not
//! the dummy, fights a round with its decision ring on, SHOW7), and the props (the tour walks
//! past the three machine props the kit stages, SHOW4). A station
//! that cannot pass inside [`STATION_TICKS`] is marked missed and the tour moves on, so one bad
//! seed never stalls the demo.
//!
//! Consumer: the arena window (`--showcase`) and the headless receipt test. Data owner: this
//! module owns only the script state; the match owns everything it reads. Units: meters,
//! ticks at 60 Hz, facing in centidegrees (0 is +z, 9000 is +x), move wishes in percent of
//! top speed on world axes. Determinism: pure over the match's views, so the same seed plays
//! the same tour. Budget: no allocation per tick.

use crate::FighterView;
use engine_arena::spear::{EndWhy, Phase, SpearEvent, SpearSlot};
use engine_audio::FightCue;
use engine_input::fighter::{ATTACK, BLOCK, THROW_NOOSE, THROW_SPEAR, THROW_SPEAR_WORLD};
use engine_input::{Buttons, FighterId, FighterInput};

mod props;
mod ruins;
mod trace;

/// Longest a station may take before it is marked missed, ticks (20 s).
pub const STATION_TICKS: u32 = 1200;
/// Ticks the tour waits after a station passes, so the viewer sees the landing.
const BREATHER_TICKS: u32 = 90;
/// Throw range the script walks into, meters (the spear flies 12 m).
const THROW_FAR_M: f32 = 8.0;
/// Closest the script throws from, meters: nearer, it steps back first.
const THROW_NEAR_M: f32 = 3.5;
/// How long the back-step of a yank is held after the press, ticks (the spear sticks well
/// inside it at 8 m).
const YANK_HOLD_TICKS: u32 = 24;
/// The back-step's share of top speed, percent: enough to read as a yank, gentle enough that
/// the follow camera never ends up inside the body.
const YANK_BACK_PERCENT: f32 = 30.0;
/// Most distance from the arena wall a wall throw starts at, meters.
const WALL_FAR_M: f32 = 8.0;
/// Least distance from the arena wall a wall throw starts at, meters.
const WALL_NEAR_M: f32 = 2.5;
/// Ticks the script swings on the rope (strafing) before it lets go.
const SWING_TICKS: u32 = 50;
/// Sword reach the melee stations close to, meters.
const MELEE_M: f32 = 1.4;
/// Clean hits the melee station asks for.
const MELEE_HITS: u32 = 3;
/// A swing is pressed on one tick of every this many, so each press is a fresh edge.
const SWING_PERIOD: u32 = 6;
/// Ticks a walk may go nowhere before the script sidesteps.
const STUCK_TICKS: u32 = 15;
/// Less than this per tick, meters, counts as not moving (a walk is 83 mm a tick).
const STUCK_MOVE_M: f32 = 0.005;
/// Ticks a sidestep lasts.
const DETOUR_TICKS: u32 = 45;
/// Nearest the noose station throws from, meters (the loop flies 6 m).
const NOOSE_NEAR_M: f32 = 3.0;
/// Farthest the noose station throws from, meters.
const NOOSE_FAR_M: f32 = 5.0;
/// Distance at which the noose station stops walking in and shoves, meters (the push reaches
/// 0.7 m past a body's skin).
const SHOVE_M: f32 = 1.1;
/// The shove chord is pressed on one tick of every this many; the guard is held between.
const SHOVE_PERIOD: u32 = 20;
/// How close the ruins station must get to a route waypoint before it walks to the next, meters.
const REACHED_M: f32 = 0.4;
/// Distance the rope-cut station stands off the dummy along an axis, meters: inside the throw band.
const CUT_STAND_M: f32 = 5.0;
/// Least cosine of the angle off the dummy's axis the rope-cut station throws from (10 degrees).
/// The blade starts 200 mm out and cuts within 70 mm of rope: a rope at 10 degrees passes 35 mm
/// from the blade's start, inside the cut.
const CUT_CONE_COS: f32 = 0.984_808;

/// One stop of the tour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Station {
    /// Hold back as the spear sticks: the enemy is reeled in (SG9).
    Yank,
    /// Throw without holding back: the spear drags both fighters on (SG1).
    Carry,
    /// The carry's rope is cut by the dummy's blade: the spear drops (SG13).
    Cut,
    /// A loop over the dummy, then the shove with the scutum (SG11).
    Noose,
    /// Walk the breach the `world.rewrite` generator cut in a ruined wall (MJ2, SHOW5).
    Ruins,
    /// The bot fights a round with the AIN8 decision ring on (SHOW7).
    Trace,
    /// Walk past the three machine props the kit stages in the ring (SHOW4).
    Props,
    /// A world-aimed throw: the rope swings the thrower (SG4, SG5, SG7).
    Swing,
    /// Three clean sword hits.
    Melee,
    /// Keep at it until the rival goes down as a ragdoll.
    Finisher,
}

/// Every station, in tour order.
pub const STATIONS: [Station; 10] = [
    Station::Yank,
    Station::Carry,
    Station::Cut,
    Station::Noose,
    Station::Ruins,
    Station::Swing,
    Station::Melee,
    Station::Finisher,
    Station::Trace,
    Station::Props,
];

impl Station {
    /// The receipt key and the window caption, in one table so the two stay in step.
    const fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Yank => (
                "yank",
                "showcase 1/10: the yank - hold back as the spear sticks",
            ),
            Self::Carry => (
                "carry",
                "showcase 2/10: the carry - the spear drags you both",
            ),
            Self::Cut => (
                "cut",
                "showcase 3/10: the rope cut - the dummy's blade cuts the carry",
            ),
            Self::Noose => (
                "noose",
                "showcase 4/10: the noose - Ctrl throws a loop, then shove",
            ),
            Self::Ruins => (
                "ruins",
                "showcase 5/10: the ruins - the tour walks the breach",
            ),
            Self::Trace => (
                "trace",
                "showcase 9/10: the trace - the bot fights and prints its last decision",
            ),
            Self::Props => (
                "props",
                "showcase 10/10: the machine props - the tour walks past them",
            ),
            Self::Swing => (
                "swing",
                "showcase 6/10: the wall swing - Q+E hooks the wall",
            ),
            Self::Melee => ("melee", "showcase 7/10: sword hits on a toned body"),
            Self::Finisher => (
                "finisher",
                "showcase 8/10: the finisher - a ragdoll with muscle",
            ),
        }
    }

    /// The receipt key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.label().0
    }

    /// The caption the window shows while the station plays.
    #[must_use]
    pub const fn caption(self) -> &'static str {
        self.label().1
    }
}

/// How a station went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Grade {
    /// Not reached yet.
    #[default]
    Pending,
    /// The system did what the station shows.
    Ok,
    /// The station ran out of time.
    Missed,
}

impl Grade {
    const fn word(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ok => "ok",
            Self::Missed => "missed",
        }
    }
}

/// The tour script for slot 0. See the module docs.
#[derive(Clone, Debug)]
pub struct Showcase {
    /// Index into [`STATIONS`]; `STATIONS.len()` once the tour is over.
    at: usize,
    /// Tick the current station started.
    started: u32,
    /// Tick the current station passed, while the breather runs.
    passed: Option<u32>,
    /// Grades in [`STATIONS`] order.
    grades: [Grade; STATIONS.len()],
    /// The current spear station has seen its own kind of tether.
    hooked: bool,
    /// Tick of the last throw press.
    pressed: Option<u32>,
    /// Ticks the current tether has held.
    tethered: u32,
    /// Clean hits the human landed in the current station.
    hits: u32,
    /// Arena half extent, meters, for the wall station.
    half_extent_m: f32,
    /// Where the human stood last tick, `[x, z]` meters.
    last: [f32; 2],
    /// Ticks the human wished to walk and did not move.
    still: u32,
    /// Ticks of sidestep left round whatever blocks the walk.
    detour: u32,
    /// Which way the next sidestep goes, +1 or -1.
    side: f32,
    /// Tick the dummy presses its blade in the rope-cut station: the tick after the spear sticks.
    swing_at: Option<u32>,
    /// The ruins station's walk through the breach, meters; empty until the station starts.
    route: Vec<ruins::Waypoint>,
    /// Waypoint the human is walking to in the ruins station.
    route_at: usize,
    /// The walking station's route is finished (the breach crossed, the machines passed).
    route_done: bool,
    /// The decision-trace station's bot, once the station starts.
    bot: Option<trace::TraceBot>,
    /// The bot's last decision line, kept after the station so the receipt still shows it.
    trace_line: Option<String>,
}

impl Showcase {
    /// A tour at its first station, for an arena of `half_extent_m`.
    #[must_use]
    pub fn new(half_extent_m: f32) -> Self {
        Self {
            at: 0,
            started: 0,
            passed: None,
            grades: [Grade::Pending; STATIONS.len()],
            hooked: false,
            pressed: None,
            tethered: 0,
            hits: 0,
            half_extent_m,
            last: [0.0; 2],
            still: 0,
            detour: 0,
            side: 1.0,
            swing_at: None,
            route: Vec::new(),
            route_at: 0,
            route_done: false,
            bot: None,
            trace_line: None,
        }
    }

    /// The station playing now, `None` once the tour is over.
    #[must_use]
    pub fn station(&self) -> Option<Station> {
        STATIONS.get(self.at).copied()
    }

    /// The window caption now.
    #[must_use]
    pub fn caption(&self) -> &'static str {
        self.station()
            .map_or("showcase: the tour is over", Station::caption)
    }

    /// True once every station is graded.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.at >= STATIONS.len()
    }

    /// The grades so far, in [`STATIONS`] order.
    #[must_use]
    pub fn grades(&self) -> [Grade; STATIONS.len()] {
        self.grades
    }

    /// One receipt line: `showcase: yank=ok carry=ok swing=missed melee=ok finisher=pending`.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        let parts: Vec<String> = STATIONS
            .iter()
            .zip(self.grades)
            .map(|(station, grade)| format!("{}={}", station.key(), grade.word()))
            .collect();
        format!("showcase: {}", parts.join(" "))
    }

    /// Slot 0's input for `tick`, from the fighters as drawn and the spear view. Call it once
    /// per tick: it remembers where the human stood, so a walk blocked by an obstacle turns
    /// into a sidestep round it.
    #[must_use]
    pub fn input(
        &mut self,
        tick: u32,
        human: FighterId,
        fighters: &[FighterView],
        spear: &[SpearSlot],
    ) -> FighterInput {
        let idle = FighterInput {
            tick,
            fighter: human,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        let (Some(station), Some((me, rival))) = (self.station(), pair(fighters, human.0)) else {
            return idle;
        };
        let to = [rival.pos_m[0] - me.pos_m[0], rival.pos_m[2] - me.pos_m[2]];
        let dist = to[0].hypot(to[1]);
        let facing = bearing(to);
        let mine = spear.get(usize::from(human.0)).copied().unwrap_or_default();
        let ready = mine.phase == Phase::Idle && mine.cooldown == 0;
        let breathing = self.passed.is_some();
        let mut out = FighterInput { facing, ..idle };
        if breathing {
            out.buttons = Buttons(BLOCK);
            return out;
        }
        match station {
            Station::Yank | Station::Carry => {
                self.throw_at_rival(station, tick, to, dist, mine, ready, &mut out);
            }
            Station::Cut => throw_down_axis(to, dist, mine, ready, &mut out),
            Station::Noose => noose_and_shove(tick, to, dist, mine, ready, &mut out),
            Station::Ruins => self.walk_breach([me.pos_m[0], me.pos_m[2]], &mut out),
            Station::Trace => self.trace_round(tick, human, me, rival, &mut out),
            Station::Props => self.walk_props([me.pos_m[0], me.pos_m[2]], &mut out),
            Station::Swing => self.throw_at_wall(me.pos_m, mine, ready, &mut out),
            Station::Melee | Station::Finisher => {
                if dist > MELEE_M {
                    let walk = unit(to).map(|c| c * 100.0);
                    (out.mx, out.mz) = (percent(walk[0]), percent(walk[1]));
                } else if tick.is_multiple_of(SWING_PERIOD) {
                    out.buttons = Buttons(ATTACK);
                }
            }
        }
        self.unstick(&mut out, [me.pos_m[0], me.pos_m[2]], to);
        out
    }

    /// The yank and the carry: walk into throw range, throw at the rival, and for the yank
    /// hold the back-step through the stick.
    #[expect(
        clippy::too_many_arguments,
        reason = "one station's whole read of the tick"
    )]
    fn throw_at_rival(
        &self,
        station: Station,
        tick: u32,
        to: [f32; 2],
        dist: f32,
        mine: SpearSlot,
        ready: bool,
        out: &mut FighterInput,
    ) {
        let held = self
            .pressed
            .is_some_and(|at| tick.saturating_sub(at) < YANK_HOLD_TICKS);
        let back = unit(to).map(|c| -c * YANK_BACK_PERCENT);
        if station == Station::Yank && held {
            (out.mx, out.mz) = (percent(back[0]), percent(back[1]));
        } else if ready && (THROW_NEAR_M..=THROW_FAR_M).contains(&dist) {
            out.buttons = Buttons(THROW_SPEAR);
            if station == Station::Yank {
                (out.mx, out.mz) = (percent(back[0]), percent(back[1]));
            }
        } else if mine.phase == Phase::Idle {
            (out.mx, out.mz) = approach(to, dist, THROW_NEAR_M + 0.5, THROW_FAR_M - 0.5);
            out.buttons = Buttons(BLOCK);
        }
    }

    /// The wall swing: walk into range of the nearest arena wall, hook it with a world throw,
    /// strafe across the rope so it swings, then let go.
    fn throw_at_wall(&self, pos: [f32; 3], mine: SpearSlot, ready: bool, out: &mut FighterInput) {
        let (outward, wall_dist) = nearest_wall(pos, self.half_extent_m);
        out.facing = bearing(outward);
        match mine.phase {
            Phase::Tether(_) if self.tethered >= SWING_TICKS => {
                out.buttons = Buttons(THROW_SPEAR);
            }
            Phase::Tether(_) => {
                (out.mx, out.mz) = (percent(-outward[1] * 100.0), percent(outward[0] * 100.0));
            }
            Phase::Flight(_) => {}
            Phase::Idle if ready && (WALL_NEAR_M..=WALL_FAR_M).contains(&wall_dist) => {
                out.buttons = Buttons(THROW_SPEAR | THROW_SPEAR_WORLD);
            }
            Phase::Idle => {
                let sign = if wall_dist > WALL_FAR_M { 1.0 } else { -1.0 };
                let walk = outward.map(|c| c * sign * 100.0);
                (out.mx, out.mz) = (percent(walk[0]), percent(walk[1]));
            }
        }
    }

    /// The rival's input for `tick`: the tour's training dummy stands and faces the human. In
    /// the rope-cut station it swings once, the tick after the spear sticks in it: its facing
    /// then sits on the rope's axis, so its blade lies along the carry (`rope_cut`). `None` when
    /// there is no rival to drive. The caller stages it as the remote slot (`App::offer_remote`).
    #[must_use]
    pub fn rival_input(
        &self,
        tick: u32,
        human: u8,
        fighters: &[FighterView],
    ) -> Option<FighterInput> {
        let (me, rival) = pair(fighters, human)?;
        let to_human = [me.pos_m[0] - rival.pos_m[0], me.pos_m[2] - rival.pos_m[2]];
        let buttons = if self.swing_at == Some(tick) {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        };
        Some(FighterInput {
            tick,
            fighter: FighterId::new(rival.slot)?,
            mx: 0,
            mz: 0,
            facing: bearing(to_human),
            buttons,
        })
    }

    /// A walk that has not moved the human for [`STUCK_TICKS`] is blocked (a pillar, a wall
    /// prop between the two): sidestep across the line to the rival for [`DETOUR_TICKS`],
    /// the other way each time, then walk on.
    fn unstick(&mut self, out: &mut FighterInput, at: [f32; 2], to: [f32; 2]) {
        let wished = out.mx != 0 || out.mz != 0;
        let moved = (at[0] - self.last[0]).hypot(at[1] - self.last[1]);
        self.last = at;
        if self.detour > 0 {
            self.detour -= 1;
            let dir = unit(to);
            let across = [-dir[1] * self.side, dir[0] * self.side];
            (out.mx, out.mz) = (percent(across[0] * 100.0), percent(across[1] * 100.0));
            return;
        }
        if wished && moved < STUCK_MOVE_M {
            self.still += 1;
        } else {
            self.still = 0;
        }
        if self.still >= STUCK_TICKS {
            self.still = 0;
            self.detour = DETOUR_TICKS;
            self.side = -self.side;
        }
    }

    /// Grades the tick that just ran from what the match reported, and moves the tour on.
    pub fn observe(
        &mut self,
        tick: u32,
        human: u8,
        spear: &[SpearSlot],
        events: &[SpearEvent],
        cues: &[FightCue],
    ) {
        let Some(station) = self.station() else {
            return;
        };
        if let Some(at) = self.passed {
            if tick.saturating_sub(at) >= BREATHER_TICKS {
                self.advance(tick);
            }
            return;
        }
        let mine = spear.get(usize::from(human)).copied().unwrap_or_default();
        if events
            .iter()
            .any(|e| matches!(*e, SpearEvent::Thrown { thrower, .. } if thrower == human))
        {
            self.pressed = Some(tick);
        }
        if station == Station::Cut
            && events.iter().any(|e| {
                matches!(*e, SpearEvent::Hit { thrower, victim: Some(_), .. } if thrower == human)
            })
        {
            self.swing_at = Some(tick + 1);
        }
        let kind = match mine.phase {
            Phase::Tether(t) => match station {
                Station::Yank => t.victim.is_some() && t.yank,
                Station::Carry | Station::Cut => t.victim.is_some() && !t.yank && !t.noose,
                Station::Noose => t.victim.is_some() && t.noose,
                Station::Swing => t.victim.is_none(),
                Station::Melee
                | Station::Finisher
                | Station::Ruins
                | Station::Trace
                | Station::Props => false,
            },
            _ => false,
        };
        if matches!(mine.phase, Phase::Tether(_)) {
            self.tethered = self.tethered.saturating_add(1);
        } else {
            self.tethered = 0;
        }
        self.hooked |= kind;
        self.hits += u32::try_from(
            cues.iter()
                .filter(|c| matches!(**c, FightCue::Hit { attacker, .. } if attacker == human))
                .count(),
        )
        .unwrap_or(0);
        let down = cues.iter().any(|c| {
            matches!(*c, FightCue::Knockdown { attacker, victim } if attacker == human && victim != human)
        });
        let pass = match station {
            Station::Yank | Station::Carry | Station::Noose | Station::Swing => {
                self.hooked && mine.phase == Phase::Idle
            }
            Station::Cut => events.iter().any(|e| {
                matches!(*e, SpearEvent::TetherEnd { thrower, why: EndWhy::Cut, .. } if thrower == human)
            }),
            Station::Melee => self.hits >= MELEE_HITS,
            Station::Ruins | Station::Props => self.route_done,
            Station::Trace => self.trace_line().is_some(),
            Station::Finisher => down,
        };
        if pass {
            self.grades[self.at] = Grade::Ok;
            self.passed = Some(tick);
        } else if tick.saturating_sub(self.started) >= STATION_TICKS {
            self.grades[self.at] = Grade::Missed;
            self.advance(tick);
        }
    }

    fn advance(&mut self, tick: u32) {
        self.at += 1;
        self.started = tick;
        self.passed = None;
        self.hooked = false;
        self.pressed = None;
        self.tethered = 0;
        self.hits = 0;
        self.swing_at = None;
        self.route.clear();
        self.route_at = 0;
        self.route_done = false;
        self.bot = None;
    }

    /// The ruins station: walk the breach the `world.rewrite` generator cut in the ruined wall,
    /// waypoint by waypoint, and mark the wall crossed at the last one. The route is built once
    /// on the station's first tick; a generator refusal leaves the route empty, so the station
    /// stands (and grades missed) rather than stalling the tour.
    fn walk_breach(&mut self, at: [f32; 2], out: &mut FighterInput) {
        if self.route.is_empty() {
            self.route = ruins::ruined_grid(ruins::RUINS_SEED)
                .and_then(|grid| ruins::breach_route(&grid, ruins::CELL_M))
                .unwrap_or_default();
        }
        self.walk_route(at, out);
    }

    /// The machine-props station: walk past the three props the kit stages, nearest first, so the
    /// tour shows the props cook. The machines are dressing, so the walk is never blocked; a
    /// failed route leaves the station standing (missed).
    fn walk_props(&mut self, at: [f32; 2], out: &mut FighterInput) {
        if self.route.is_empty() {
            self.route = props::machine_walk(at);
        }
        self.walk_route(at, out);
    }

    /// One tick of a walking station: steer at the next waypoint, take the next when reached, and
    /// mark the route done at its last one.
    fn walk_route(&mut self, at: [f32; 2], out: &mut FighterInput) {
        if self.route_at >= self.route.len() {
            self.route_done = true;
            out.buttons = Buttons(BLOCK);
            return;
        }
        let goal = self.route[self.route_at];
        let to = [goal[0] - at[0], goal[1] - at[1]];
        let dist = to[0].hypot(to[1]);
        if dist < REACHED_M {
            self.route_at += 1;
            if self.route_at >= self.route.len() {
                self.route_done = true;
            }
        }
        out.facing = bearing(to);
        (out.mx, out.mz) = approach(to, dist, 0.0, REACHED_M * 0.5);
        if dist <= REACHED_M {
            out.buttons = Buttons(BLOCK);
        }
    }

    /// The decision-trace station: slot 0 is fought by a real gladiator brain with the AIN8
    /// decision ring on, so the tour shows the bot's own decisions instead of the dummy's script.
    /// The line refreshes once a second ([`trace::LINE_PERIOD`]) and is read by the receipt.
    fn trace_round(
        &mut self,
        tick: u32,
        human: FighterId,
        me: &FighterView,
        rival: &FighterView,
        out: &mut FighterInput,
    ) {
        let bot = self.bot.get_or_insert_with(|| trace::TraceBot::new(human));
        *out = bot.input(tick, me, rival);
        if let Some(line) = bot.line() {
            self.trace_line = Some(line.to_owned());
        }
    }

    /// The bot's last decision line in the decision-trace station, once it has one: the window
    /// prints it each second and the receipt grades it. Kept after the station ends.
    #[must_use]
    pub fn trace_line(&self) -> Option<&str> {
        self.trace_line.as_deref()
    }
}

/// The human's view and the nearest other fighter's.
fn pair(fighters: &[FighterView], human: u8) -> Option<(&FighterView, &FighterView)> {
    let me = fighters.iter().find(|f| f.slot == human)?;
    let rival = fighters.iter().filter(|f| f.slot != human).min_by(|a, b| {
        let d = |f: &FighterView| (f.pos_m[0] - me.pos_m[0]).hypot(f.pos_m[2] - me.pos_m[2]);
        d(a).total_cmp(&d(b))
    })?;
    Some((me, rival))
}

/// Centidegree facing along a planar `[x, z]` direction (0 is +z, 9000 is +x).
#[expect(
    clippy::cast_possible_truncation,
    reason = "a bearing is within +-18000 centidegrees, inside i16; f32 has no TryFrom"
)]
fn bearing(dir: [f32; 2]) -> i16 {
    let deg = dir[0].atan2(dir[1]).to_degrees();
    if deg.is_finite() {
        (deg * 100.0).round() as i16
    } else {
        0
    }
}

/// The planar unit vector along `v` (zero stays zero).
fn unit(v: [f32; 2]) -> [f32; 2] {
    let len = v[0].hypot(v[1]);
    if len > 1.0e-6 {
        [v[0] / len, v[1] / len]
    } else {
        [0.0, 0.0]
    }
}

/// A move wish component in percent, clamped to the legal range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "clamped to +-100 first, inside i8; f32 has no TryFrom"
)]
fn percent(v: f32) -> i8 {
    if v.is_finite() {
        v.clamp(-100.0, 100.0).round() as i8
    } else {
        0
    }
}

/// Walk into the band `[near, far]` of distance from the rival.
fn approach(to: [f32; 2], dist: f32, near: f32, far: f32) -> (i8, i8) {
    let dir = unit(to);
    let sign = if dist > far {
        100.0
    } else if dist < near {
        -100.0
    } else {
        0.0
    };
    (percent(dir[0] * sign), percent(dir[1] * sign))
}

/// The outward unit direction to the nearest arena wall from `pos`, and the distance to it.
fn nearest_wall(pos: [f32; 3], half_extent_m: f32) -> ([f32; 2], f32) {
    let (x, z) = (pos[0], pos[2]);
    if x.abs() >= z.abs() {
        let sign = if x >= 0.0 { 1.0 } else { -1.0 };
        ([sign, 0.0], half_extent_m - x.abs())
    } else {
        let sign = if z >= 0.0 { 1.0 } else { -1.0 };
        ([0.0, sign], half_extent_m - z.abs())
    }
}

/// The rope-cut throw. The human stands `CUT_STAND_M` off the dummy along the axis nearest to
/// it, walks onto that axis and throws down it once within `CUT_CONE_COS` of it. The dummy's
/// facing quantizes to the same axis (`facing_vector`), so its blade starts on the rope and
/// `rope_cut` cuts it. Off the axis the rope would pass the blade's start too far away.
fn throw_down_axis(to: [f32; 2], dist: f32, mine: SpearSlot, ready: bool, out: &mut FighterInput) {
    let away = [-to[0], -to[1]];
    let axis = nearest_axis(away);
    let along = away[0] * axis[0] + away[1] * axis[1];
    let aligned = along >= CUT_CONE_COS * away[0].hypot(away[1]);
    if ready && aligned && (THROW_NEAR_M..=THROW_FAR_M).contains(&dist) {
        out.buttons = Buttons(THROW_SPEAR);
    } else if mine.phase == Phase::Idle {
        let stand = [to[0] + axis[0] * CUT_STAND_M, to[1] + axis[1] * CUT_STAND_M];
        let stand_len = stand[0].hypot(stand[1]);
        (out.mx, out.mz) = approach(stand, stand_len, 0.3, 0.5);
        out.buttons = Buttons(BLOCK);
    }
}

/// The noose: walk into the loop's range, throw it over the dummy, then walk in on the cord and
/// shove with the scutum once inside reach (the attack and block chord, the guard held between
/// presses). The dummy cannot dodge or walk off, so the loop lands; the shove is a knockdown.
fn noose_and_shove(
    tick: u32,
    to: [f32; 2],
    dist: f32,
    mine: SpearSlot,
    ready: bool,
    out: &mut FighterInput,
) {
    match mine.phase {
        Phase::Flight(_) => {}
        Phase::Tether(_) if dist > SHOVE_M => {
            let walk = unit(to).map(|c| c * 100.0);
            (out.mx, out.mz) = (percent(walk[0]), percent(walk[1]));
        }
        Phase::Tether(_) => {
            let chord = tick.is_multiple_of(SHOVE_PERIOD);
            out.buttons = Buttons(if chord { ATTACK | BLOCK } else { BLOCK });
        }
        Phase::Idle if ready && (NOOSE_NEAR_M..=NOOSE_FAR_M).contains(&dist) => {
            out.buttons = Buttons(THROW_NOOSE);
        }
        Phase::Idle => {
            (out.mx, out.mz) = approach(to, dist, NOOSE_NEAR_M + 0.5, NOOSE_FAR_M - 0.5);
            out.buttons = Buttons(BLOCK);
        }
    }
}

/// The arena axis (`+-x` or `+-z`) nearest the planar direction `v`; a tie goes to x.
fn nearest_axis(v: [f32; 2]) -> [f32; 2] {
    if v[0].abs() >= v[1].abs() {
        [if v[0] >= 0.0 { 1.0 } else { -1.0 }, 0.0]
    } else {
        [0.0, if v[1] >= 0.0 { 1.0 } else { -1.0 }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};

    /// The tour played headless on the window's own session: spear, noose and height armed.
    fn tour(seed: u64, ticks: u32) -> (Showcase, App) {
        let config = GameConfig {
            seed,
            ticks,
            fighters: 2,
            half_extent_m: 12.0,
        };
        let mut app = App::new(config)
            .expect("session")
            .into_spear()
            .expect("spear")
            .into_noose()
            .expect("noose")
            .into_height()
            .expect("height");
        let mut show = Showcase::new(config.half_extent_m);
        let mut out = TickOutcome::default();
        let human = FighterId::new(HUMAN_SLOT).expect("slot");
        for tick in 0..ticks {
            if show.finished() {
                break;
            }
            let live = app.live_match();
            let input = show.input(tick, human, app.fighter_views(), live.spear_view());
            if let Some(dummy) = show.rival_input(tick, HUMAN_SLOT, app.fighter_views()) {
                app.offer_remote(dummy)
                    .expect("the dummy stages as the remote slot");
            }
            app.tick_into(input, &mut out);
            if !out.input_accepted {
                break;
            }
            let live = app.live_match();
            show.observe(
                tick,
                HUMAN_SLOT,
                live.spear_view(),
                live.spear_events(),
                &out.cues,
            );
        }
        (show, app)
    }

    #[test]
    fn the_tour_passes_every_station_against_the_dummy() {
        let (show, app) = tour(2040, 9000);
        println!("{}", show.receipt_line());
        assert!(
            show.finished(),
            "the tour ran to its end: {}",
            show.receipt_line()
        );
        assert!(
            show.grades().iter().all(|g| *g == Grade::Ok),
            "every station passes: {}",
            show.receipt_line()
        );
        assert!(
            show.receipt_line().contains("cut=ok"),
            "the rope-cut station passes: {}",
            show.receipt_line()
        );
        assert!(
            show.receipt_line().contains("noose=ok"),
            "the noose station passes: {}",
            show.receipt_line()
        );
        assert!(
            show.receipt_line().contains("ruins=ok"),
            "the ruins station passes: {}",
            show.receipt_line()
        );
        assert!(
            show.receipt_line().contains("trace=ok"),
            "the trace station passes: {}",
            show.receipt_line()
        );
        assert!(
            show.trace_line().is_some(),
            "the bot printed its last decision: {}",
            show.trace_line().unwrap_or("none")
        );
        assert!(
            show.receipt_line().contains("props=ok"),
            "the machine-props station passes: {}",
            show.receipt_line()
        );
        assert!(
            app.hp()[usize::from(HUMAN_SLOT)] > 0,
            "the dummy never hurts the player"
        );
    }

    #[test]
    fn the_same_seed_plays_the_same_tour() {
        let (a, _) = tour(7, 3000);
        let (b, _) = tour(7, 3000);
        assert_eq!(a.receipt_line(), b.receipt_line());
        assert_eq!(a.station(), b.station());
    }

    #[test]
    fn a_station_that_cannot_pass_is_missed_and_the_tour_moves_on() {
        let mut show = Showcase::new(12.0);
        for tick in 0..=STATION_TICKS {
            show.observe(tick, 0, &[], &[], &[]);
        }
        assert_eq!(show.grades()[0], Grade::Missed);
        assert_eq!(show.station(), Some(Station::Carry));
        assert!(show
            .receipt_line()
            .starts_with("showcase: yank=missed carry=pending"));
    }

    #[test]
    fn captions_name_every_station_and_the_end() {
        for (n, station) in STATIONS.iter().enumerate() {
            assert!(station
                .caption()
                .contains(&format!("{}/{}", n + 1, STATIONS.len())));
        }
        let mut show = Showcase::new(12.0);
        show.at = STATIONS.len();
        assert!(show.finished());
        assert_eq!(show.caption(), "showcase: the tour is over");
    }
}
