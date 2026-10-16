//! crowd: ORCA (RVO2 linear-program school) plus arrive steering with
//! camera-distance LOD. `separation` stays as the measured baseline; the
//! tick path (`steer`, `steer_all`) steers near agents with a
//! single-threaded ascending-`AgentId` 2-D ORCA linear program.
//! Owner: nav crate, consumer arena background agents. Units: meters,
//! meters per second, ticks at 60 Hz. Determinism: neighbor order sorted
//! by agent id, only `+ - * /` and `sqrt`, no wall clock, no threads, no
//! KdTree. Errors: typed [`CrowdError`], invalid input refuses, never panics.

use std::fmt;

use crate::neighbors::{NeighborIndex, Shortlist, NEIGHBOR_CELL_M};

/// Fixed tick rate steering integrates over.
pub const TICK_HZ: f32 = 60.0;
/// Seconds per tick.
pub const TICK_DT_S: f32 = 1.0 / TICK_HZ;
/// Camera distance at or below which agents pay full separation.
pub const LOD_NEAR_M: f32 = 15.0;
/// Camera distance past which agents hold still as impostors.
pub const IMPOSTOR_M: f32 = 60.0;
/// Largest crowd one batch call steers; bounds per-tick work up front.
pub const MAX_AGENTS: usize = 256;
/// Largest separation radius in meters; beyond it refuses.
pub const MAX_SEPARATION_M: f32 = 10.0;
/// Largest speed in meters per second; matches traversal probe cap.
pub const MAX_SPEED_M_S: f32 = 12.0;
/// Largest accepted coordinate lane in meters.
pub const MAX_COORD_M: f32 = 4096.0;
/// ORCA neighbor cap: at most this many nearest agents constrain one solve.
/// Bounds per-agent work up front; extra agents are ignored deterministically
/// (nearest by squared distance, ties by ascending id).
pub const ORCA_MAX_NEIGHBORS: usize = 10;
/// ORCA lookahead in seconds: collisions inside this horizon constrain the
/// solve. Fixed for every agent, so every tick replays identically.
pub const ORCA_TIME_HORIZON_S: f32 = 2.0;
/// Largest ORCA horizon accepted in seconds.
pub const ORCA_MAX_HORIZON_S: f32 = 8.0;

/// Stable agent id; every tie-break orders by this, never by thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentId(pub u32);

/// One steering body on the XZ plane (`y` is up and never steered).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Agent {
    /// Stable id for deterministic tie-breaks.
    pub id: AgentId,
    /// Position in meters `[x, z]`.
    pub pos_m: [f32; 2],
    /// Velocity in meters per second `[x, z]`.
    pub vel_m_s: [f32; 2],
    /// Body radius in meters.
    pub radius_m: f32,
    /// Speed cap in meters per second.
    pub max_speed_m_s: f32,
}

/// Steering detail level, decided by camera distance only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lod {
    /// Within [`LOD_NEAR_M`]: full separation plus arrive.
    Near,
    /// Between near and impostor: flow direction at arrive speed.
    Far,
    /// Past [`IMPOSTOR_M`]: holds still, pays nothing.
    Impostor,
}

/// Steering configuration. Units: meters and meters per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrowdConfig {
    /// Separation falloff radius in meters, within `(0, MAX_SEPARATION_M]`.
    pub separation_radius_m: f32,
    /// Gain on the separation term, within `[0, 4]`.
    pub separation_gain: f32,
    /// Distance where arrival starts slowing, in meters.
    pub arrive_slow_m: f32,
    /// Distance where arrival stops, in meters.
    pub arrive_stop_m: f32,
    /// Near/far switch distance in meters (plan fixes 15).
    pub lod_near_m: f32,
    /// Far/impostor switch distance in meters.
    pub impostor_m: f32,
}

impl Default for CrowdConfig {
    fn default() -> Self {
        Self {
            separation_radius_m: 1.0,
            separation_gain: 1.0,
            arrive_slow_m: 2.0,
            arrive_stop_m: 0.1,
            lod_near_m: LOD_NEAR_M,
            impostor_m: IMPOSTOR_M,
        }
    }
}

/// Typed steering failures; bad input refuses, never guesses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CrowdError {
    /// Too many agents for one batch.
    TooManyAgents {
        /// Agent count offered.
        got: usize,
        /// Batch bound refused past.
        max: usize,
    },
    /// Goals/flows length differs from agents length.
    LengthMismatch {
        /// Agent count.
        agents: usize,
        /// Other lane count.
        other: usize,
    },
    /// Non-finite or out-of-range lane; the lane names it.
    BadValue(&'static str),
    /// Config lane out of range; the lane names it.
    BadConfig(&'static str),
}

impl fmt::Display for CrowdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyAgents { got, max } => {
                write!(f, "crowd: {got} agents over bound {max}")
            }
            Self::LengthMismatch { agents, other } => {
                write!(f, "crowd: {agents} agents but {other} lane entries")
            }
            Self::BadValue(lane) => write!(f, "crowd: bad value ({lane})"),
            Self::BadConfig(lane) => write!(f, "crowd: bad config ({lane})"),
        }
    }
}

impl std::error::Error for CrowdError {}

/// Committed per-agent command plus its LOD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteerOutput {
    /// Commanded velocity in meters per second `[x, z]`.
    pub vel_m_s: [f32; 2],
    /// LOD this command was steered at.
    pub lod: Lod,
}

/// Batch cost proof: directed neighbor checks actually evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SteerStats {
    /// Directed near-agent neighbor checks run this batch. The tick path
    /// runs ORCA, so this counts capped ORCA half-planes evaluated
    /// (at most `near * ORCA_MAX_NEIGHBORS`); `separation` stays as the
    /// offline baseline and does not feed this counter.
    pub separation_pairs: usize,
    /// Agents steered per LOD.
    pub near: usize,
    /// Agents steered per LOD.
    pub far: usize,
    /// Agents steered per LOD.
    pub impostor: usize,
}

/// LOD from camera distance only; frame timing never feeds it.
pub fn lod_for(dist_m: f32, config: &CrowdConfig) -> Result<Lod, CrowdError> {
    check_config(config)?;
    if !dist_m.is_finite() || dist_m < 0.0 {
        return Err(CrowdError::BadValue("camera distance"));
    }
    if dist_m <= config.lod_near_m {
        Ok(Lod::Near)
    } else if dist_m <= config.impostor_m {
        Ok(Lod::Far)
    } else {
        Ok(Lod::Impostor)
    }
}

/// Separation velocity for one agent over id-sorted neighbors.
///
/// Linear falloff: full push at contact, zero at the radius edge. The
/// output is a velocity offset capped at the agent cap, summed in
/// ascending neighbor-id order so shuffles replay identically.
pub fn separation(
    agent: &Agent,
    neighbors: &[Agent],
    config: &CrowdConfig,
) -> Result<[f32; 2], CrowdError> {
    check_config(config)?;
    check_agent(agent)?;
    if neighbors.len() > MAX_AGENTS {
        return Err(CrowdError::TooManyAgents {
            got: neighbors.len(),
            max: MAX_AGENTS,
        });
    }
    check_unique_ids(neighbors)?;
    let mut order: Vec<usize> = (0..neighbors.len()).collect();
    order.sort_by_key(|i| neighbors[*i].id);
    let mut push = [0.0f32; 2];
    let radius = config.separation_radius_m;
    for i in order {
        let other = &neighbors[i];
        check_agent(other)?;
        if other.id == agent.id {
            continue;
        }
        let dx = agent.pos_m[0] - other.pos_m[0];
        let dz = agent.pos_m[1] - other.pos_m[1];
        if !dx.is_finite() || !dz.is_finite() {
            return Err(CrowdError::BadValue("neighbor position"));
        }
        let dist2 = dx * dx + dz * dz;
        if dist2 <= 0.0 || dist2 >= radius * radius {
            continue;
        }
        let dist = dist2.sqrt();
        let weight = (radius - dist) / radius;
        push[0] += dx / dist * weight;
        push[1] += dz / dist * weight;
    }
    push[0] *= config.separation_gain * agent.max_speed_m_s;
    push[1] *= config.separation_gain * agent.max_speed_m_s;
    Ok(clamp_speed(push, agent.max_speed_m_s))
}

/// Reciprocal collision-avoidance velocity for one agent.
///
/// Builds at most [`ORCA_MAX_NEIGHBORS`] half-planes over the nearest
/// neighbors (squared distance, ties by ascending id), ordered by ascending
/// id, then solves the 2-D linear program for the velocity nearest
/// `preferred_m_s` inside the speed cap. Single-threaded, stack scratch
/// only (no heap after the caller's reserve), only `+ - * /` and `sqrt`.
///
/// Units: meters, meters per second. Determinism: same agents plus same
/// preferred velocity give bit-identical output. Errors:
/// [`CrowdError::BadValue`] on non-finite lanes, [`CrowdError::TooManyAgents`]
/// past [`MAX_AGENTS`], [`CrowdError::BadValue`] on a bad horizon.
/// A zero preferred velocity holds still (arrive starve guard: agents at
/// their goal never drift).
///
/// ```rust
/// use engine_nav::crowd::{Agent, AgentId, CrowdConfig, orca_velocity};
/// let cfg = CrowdConfig::default();
/// let me = Agent { id: AgentId(1), pos_m: [0.0, 0.0], vel_m_s: [1.0, 0.0], radius_m: 0.3, max_speed_m_s: 3.0 };
/// let other = Agent { id: AgentId(2), pos_m: [4.0, 0.5], vel_m_s: [-1.0, 0.0], radius_m: 0.3, max_speed_m_s: 3.0 };
/// let out = orca_velocity(&me, &[me, other], [1.0, 0.0], 2.0).expect("solves");
/// assert!(out[0] > 0.0, "keeps forward progress: {out:?}");
/// ```
pub fn orca_velocity(
    agent: &Agent,
    neighbors: &[Agent],
    preferred_m_s: [f32; 2],
    time_horizon_s: f32,
) -> Result<[f32; 2], CrowdError> {
    check_agent(agent)?;
    if neighbors.len() > MAX_AGENTS {
        return Err(CrowdError::TooManyAgents {
            got: neighbors.len(),
            max: MAX_AGENTS,
        });
    }
    if !preferred_m_s.iter().all(|v| v.is_finite()) {
        return Err(CrowdError::BadValue("preferred velocity"));
    }
    if !time_horizon_s.is_finite() || time_horizon_s <= 0.0 || time_horizon_s > ORCA_MAX_HORIZON_S {
        return Err(CrowdError::BadValue("time horizon"));
    }
    check_unique_ids(neighbors)?;
    for other in neighbors {
        check_agent(other)?;
    }
    let pref2 = preferred_m_s[0] * preferred_m_s[0] + preferred_m_s[1] * preferred_m_s[1];
    if pref2 <= 1e-12 {
        return Ok([0.0, 0.0]);
    }
    let picked = pick_nearest(agent, neighbors);
    let mut lines = [OrcaLine {
        point_m_s: [0.0, 0.0],
        dir_m_s: [1.0, 0.0],
    }; ORCA_MAX_NEIGHBORS];
    let mut count = 0usize;
    for maybe in picked {
        let Some(other) = maybe else {
            break;
        };
        check_agent(&other)?;
        if let Some(line) = orca_line_for(agent, &other, time_horizon_s) {
            lines[count] = line;
            count += 1;
        }
    }
    Ok(clamp_speed(
        solve_orca(preferred_m_s, &lines[..count], agent.max_speed_m_s),
        agent.max_speed_m_s,
    ))
}

/// One ORCA half-plane: feasible velocities satisfy
/// `det(dir, point - v) <= 0`. `point` sits on the line, `dir` runs along it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct OrcaLine {
    /// A point on the constraint line in velocity space, meters per second.
    point_m_s: [f32; 2],
    /// Unit direction along the constraint line.
    dir_m_s: [f32; 2],
}

/// Nearest capped neighbors in ascending-id order for line building.
///
/// Scans every lane once into a proximity-ordered stack shortlist (squared
/// distance, ties by id), then reorders the shortlist by id so the linear
/// program sees lines in a stable order whatever the input shuffle.
/// Stack scratch only; never allocates.
fn pick_nearest(agent: &Agent, neighbors: &[Agent]) -> [Option<Agent>; ORCA_MAX_NEIGHBORS] {
    let mut short = Shortlist::new();
    for other in neighbors.iter() {
        if other.id == agent.id {
            continue;
        }
        let dx = agent.pos_m[0] - other.pos_m[0];
        let dz = agent.pos_m[1] - other.pos_m[1];
        short.rank(dx * dx + dz * dz, other.id.0, *other);
    }
    short.finish()
}

/// Half-plane of `agent` against `other` over `tau` seconds.
///
/// Reciprocal (RVO2) form: each side takes half the correction `u`, so two
/// head-on agents share the dodge instead of oscillating past each other.
/// Returns `None` when the pair needs no constraint (far apart and
/// separating). Only `+ - * /` and `sqrt`; never allocates.
fn orca_line_for(agent: &Agent, other: &Agent, tau_s: f32) -> Option<OrcaLine> {
    let rel_pos = [
        other.pos_m[0] - agent.pos_m[0],
        other.pos_m[1] - agent.pos_m[1],
    ];
    let rel_vel = [
        agent.vel_m_s[0] - other.vel_m_s[0],
        agent.vel_m_s[1] - other.vel_m_s[1],
    ];
    let combined = agent.radius_m + other.radius_m;
    let dist2 = rel_pos[0] * rel_pos[0] + rel_pos[1] * rel_pos[1];
    let combined2 = combined * combined;
    if dist2 > combined2 {
        let inv_tau = 1.0 / tau_s;
        let cutoff = [rel_pos[0] * inv_tau, rel_pos[1] * inv_tau];
        let w = [rel_vel[0] - cutoff[0], rel_vel[1] - cutoff[1]];
        let w2 = w[0] * w[0] + w[1] * w[1];
        let dot1 = w[0] * rel_pos[0] + w[1] * rel_pos[1];
        if dot1 < 0.0 && dot1 * dot1 > combined2 * w2 {
            let wlen2 = w2;
            if wlen2 <= 1e-12 {
                return None;
            }
            let wlen = wlen2.sqrt();
            let unit_w = [w[0] / wlen, w[1] / wlen];
            let dir = [unit_w[1], -unit_w[0]];
            let push = combined * inv_tau - wlen;
            let u = [push * unit_w[0], push * unit_w[1]];
            return Some(OrcaLine {
                point_m_s: [agent.vel_m_s[0] + 0.5 * u[0], agent.vel_m_s[1] + 0.5 * u[1]],
                dir_m_s: dir,
            });
        }
        let leg2 = dist2 - combined2;
        if leg2 <= 0.0 {
            return None;
        }
        let leg = leg2.sqrt();
        if dist2 <= 1e-12 {
            return None;
        }
        let dir = if rel_pos[0] * w[1] - rel_pos[1] * w[0] > 0.0 {
            [
                (rel_pos[0] * leg - rel_pos[1] * combined) / dist2,
                (rel_pos[0] * combined + rel_pos[1] * leg) / dist2,
            ]
        } else {
            [
                -(rel_pos[0] * leg + rel_pos[1] * combined) / dist2,
                -(-rel_pos[0] * combined + rel_pos[1] * leg) / dist2,
            ]
        };
        let along = rel_vel[0] * dir[0] + rel_vel[1] * dir[1];
        let u = [along * dir[0] - rel_vel[0], along * dir[1] - rel_vel[1]];
        Some(OrcaLine {
            point_m_s: [agent.vel_m_s[0] + 0.5 * u[0], agent.vel_m_s[1] + 0.5 * u[1]],
            dir_m_s: dir,
        })
    } else {
        let inv_dt = 1.0 / TICK_DT_S;
        let push_pos = [rel_pos[0] * inv_dt, rel_pos[1] * inv_dt];
        let w = [rel_vel[0] - push_pos[0], rel_vel[1] - push_pos[1]];
        let w2 = w[0] * w[0] + w[1] * w[1];
        if w2 <= 1e-12 {
            let u = [combined * inv_dt, 0.0];
            return Some(OrcaLine {
                point_m_s: [agent.vel_m_s[0] + 0.5 * u[0], agent.vel_m_s[1] + 0.5 * u[1]],
                dir_m_s: [0.0, 1.0],
            });
        }
        let wlen = w2.sqrt();
        let unit_w = [w[0] / wlen, w[1] / wlen];
        let dir = [unit_w[1], -unit_w[0]];
        let push = combined * inv_dt - wlen;
        let u = [push * unit_w[0], push * unit_w[1]];
        Some(OrcaLine {
            point_m_s: [agent.vel_m_s[0] + 0.5 * u[0], agent.vel_m_s[1] + 0.5 * u[1]],
            dir_m_s: dir,
        })
    }
}

/// Closest velocity to `preferred` inside every half-plane and the cap.
///
/// Two-pass linear program after the RVO2 school: `program2` walks the
/// lines and repairs each violation with `program1`; when the lines plus
/// the cap admit nothing, `program3` keeps the feasible prefix and projects
/// the rest, so the tick always returns a capped velocity. Line count is
/// bounded by [`ORCA_MAX_NEIGHBORS`], so iterations are fixed; only
/// `+ - * /` and `sqrt`.
fn solve_orca(preferred_m_s: [f32; 2], lines: &[OrcaLine], cap_m_s: f32) -> [f32; 2] {
    let mut result = clamp_speed(preferred_m_s, cap_m_s);
    for (i, line) in lines.iter().enumerate() {
        if det2(line.dir_m_s, sub2(line.point_m_s, result)) > 0.0
            && !project_line(lines, i, cap_m_s, preferred_m_s, &mut result)
        {
            result = fallback_orca(lines, i, cap_m_s, preferred_m_s);
            break;
        }
    }
    result
}

/// Repair one violated line against all earlier lines plus the cap circle.
///
/// Returns false when line `line_no` admits nothing with its prefix, which
/// is what sends the solve into the `program3` fallback. Fixed iterations
/// over at most [`ORCA_MAX_NEIGHBORS`] earlier lines.
fn project_line(
    lines: &[OrcaLine],
    line_no: usize,
    cap_m_s: f32,
    preferred_m_s: [f32; 2],
    result: &mut [f32; 2],
) -> bool {
    let line = lines[line_no];
    let dot = line.point_m_s[0] * line.dir_m_s[0] + line.point_m_s[1] * line.dir_m_s[1];
    let disc = dot * dot + cap_m_s * cap_m_s
        - (line.point_m_s[0] * line.point_m_s[0] + line.point_m_s[1] * line.point_m_s[1]);
    if disc < 0.0 {
        return false;
    }
    let root = disc.sqrt();
    let mut left = -dot - root;
    let mut right = -dot + root;
    for other in lines.iter().take(line_no) {
        let denom = det2(line.dir_m_s, other.dir_m_s);
        let numer = det2(other.dir_m_s, sub2(line.point_m_s, other.point_m_s));
        if denom * denom <= 1e-12 {
            if numer < 0.0 {
                return false;
            }
            continue;
        }
        let t = numer / denom;
        if denom >= 0.0 {
            if t < right {
                right = t;
            }
        } else if t > left {
            left = t;
        }
        if left > right {
            return false;
        }
    }
    let off = sub2(preferred_m_s, line.point_m_s);
    let mut t = off[0] * line.dir_m_s[0] + off[1] * line.dir_m_s[1];
    if t < left {
        t = left;
    } else if t > right {
        t = right;
    }
    *result = [
        line.point_m_s[0] + line.dir_m_s[0] * t,
        line.point_m_s[1] + line.dir_m_s[1] * t,
    ];
    true
}

/// Fallback when the full line set plus the cap admit nothing.
///
/// Keeps the feasible prefix before `failed_at` (repaired with the same
/// projection), then projects each later violated line in id order. Fixed
/// iterations, always returns a capped velocity, so dense packs still move.
fn fallback_orca(
    lines: &[OrcaLine],
    failed_at: usize,
    cap_m_s: f32,
    preferred_m_s: [f32; 2],
) -> [f32; 2] {
    let mut result = clamp_speed(preferred_m_s, cap_m_s);
    for (i, line) in lines.iter().enumerate() {
        if i < failed_at {
            continue;
        }
        if det2(line.dir_m_s, sub2(line.point_m_s, result)) > 0.0 {
            let mut fixed = result;
            if project_line(lines, i, cap_m_s, preferred_m_s, &mut fixed) {
                result = fixed;
            } else {
                result = clamp_speed(line.point_m_s, cap_m_s);
            }
        }
    }
    result
}

/// 2-D determinant (signed area); sign picks the ORCA leg and side.
fn det2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

/// Component-wise difference; keeps the solver free of temporaries.
fn sub2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

/// Near-tick solver: arrive as the preferred velocity, ORCA for the dodge.
///
/// Holds still inside the arrive stop radius (starve guard), otherwise
/// returns the ORCA velocity capped to the agent cap. Reports how many
/// half-planes constrained the solve for the batch cost counter.
fn steer_near(
    agent: &Agent,
    neighbors: &[Agent],
    goal_m: [f32; 2],
    config: &CrowdConfig,
) -> Result<([f32; 2], usize), CrowdError> {
    let preferred = arrive(agent.pos_m, goal_m, agent.max_speed_m_s, config)?;
    if preferred[0] == 0.0 && preferred[1] == 0.0 {
        return Ok(([0.0, 0.0], 0));
    }
    if neighbors.len() > MAX_AGENTS {
        return Err(CrowdError::TooManyAgents {
            got: neighbors.len(),
            max: MAX_AGENTS,
        });
    }
    for other in neighbors {
        check_agent(other)?;
    }
    let picked = pick_nearest(agent, neighbors);
    finish_near(agent, preferred, &picked)
}
fn finish_near(
    agent: &Agent,
    preferred: [f32; 2],
    picked: &[Option<Agent>; ORCA_MAX_NEIGHBORS],
) -> Result<([f32; 2], usize), CrowdError> {
    let mut lines = [OrcaLine {
        point_m_s: [0.0, 0.0],
        dir_m_s: [1.0, 0.0],
    }; ORCA_MAX_NEIGHBORS];
    let mut count = 0usize;
    for maybe in picked.iter().copied() {
        let Some(other) = maybe else {
            break;
        };
        check_agent(&other)?;
        if let Some(line) = orca_line_for(agent, &other, ORCA_TIME_HORIZON_S) {
            lines[count] = line;
            count += 1;
        }
    }
    let vel = clamp_speed(
        solve_orca(preferred, &lines[..count], agent.max_speed_m_s),
        agent.max_speed_m_s,
    );
    Ok((vel, count))
}

/// Arrive velocity toward a goal: full speed outside the slow radius,
/// linear ramp inside, zero within the stop radius.
pub fn arrive(
    pos_m: [f32; 2],
    goal_m: [f32; 2],
    max_speed_m_s: f32,
    config: &CrowdConfig,
) -> Result<[f32; 2], CrowdError> {
    check_config(config)?;
    check_point(pos_m, "position")?;
    check_point(goal_m, "goal")?;
    check_speed(max_speed_m_s)?;
    let dx = goal_m[0] - pos_m[0];
    let dz = goal_m[1] - pos_m[1];
    let dist = (dx * dx + dz * dz).sqrt();
    if dist <= config.arrive_stop_m {
        return Ok([0.0, 0.0]);
    }
    let speed = if dist >= config.arrive_slow_m {
        max_speed_m_s
    } else {
        let span = config.arrive_slow_m - config.arrive_stop_m;
        max_speed_m_s * (dist - config.arrive_stop_m) / span
    };
    Ok([dx / dist * speed, dz / dist * speed])
}

/// One Euler tick: `pos + vel * dt`, refused on bad lanes.
pub fn integrate(pos_m: [f32; 2], vel_m_s: [f32; 2]) -> Result<[f32; 2], CrowdError> {
    check_point(pos_m, "position")?;
    check_point(vel_m_s, "velocity")?;
    Ok([
        pos_m[0] + vel_m_s[0] * TICK_DT_S,
        pos_m[1] + vel_m_s[1] * TICK_DT_S,
    ])
}

/// Far LOD velocity: flow direction at arrive speed, still fallback.
///
/// Pure helper with no behavior change: holds still inside the arrive stop
/// radius, otherwise scales the flow-field direction to the arrive speed
/// (arrive direction when the flow is still), clamped to the agent cap.
/// Squad commanders share one call per far squad; [`steer`] reuses it for
/// every Far agent so shared and per-agent paths stay bit-identical.
/// Units: meters, meters per second. Determinism: same float order as the
/// tick path (`+ - * /` and `sqrt` only, ascending-id order kept by the
/// caller). Errors: [`CrowdError::BadValue`] and [`CrowdError::BadConfig`]
/// from [`arrive`], never panics.
pub fn far_velocity(
    pos_m: [f32; 2],
    goal_m: [f32; 2],
    max_speed_m_s: f32,
    flow_m_s: [f32; 2],
    config: &CrowdConfig,
) -> Result<[f32; 2], CrowdError> {
    check_config(config)?;
    check_point(pos_m, "position")?;
    check_point(goal_m, "goal")?;
    check_point(flow_m_s, "flow")?;
    check_speed(max_speed_m_s)?;
    let dx = goal_m[0] - pos_m[0];
    let dz = goal_m[1] - pos_m[1];
    let dist = (dx * dx + dz * dz).sqrt();
    if dist <= config.arrive_stop_m {
        return Ok([0.0, 0.0]);
    }
    let speed = arrive(pos_m, goal_m, max_speed_m_s, config)?;
    let speed_mag = (speed[0] * speed[0] + speed[1] * speed[1]).sqrt();
    let flow_mag = (flow_m_s[0] * flow_m_s[0] + flow_m_s[1] * flow_m_s[1]).sqrt();
    let vel_m_s = if flow_mag > 0.0 {
        [
            flow_m_s[0] / flow_mag * speed_mag,
            flow_m_s[1] / flow_mag * speed_mag,
        ]
    } else {
        speed
    };
    Ok(clamp_speed(vel_m_s, max_speed_m_s))
}

/// Steer one agent: LOD from camera distance, then the LOD path.
///
/// Near solves arrive as the preferred velocity through the capped ORCA
/// linear program and clamps to the cap. Far moves
/// along the flow-field direction at arrive speed (arrive direction when
/// the flow is still). Impostors hold with zero velocity and read no
/// neighbors, which is what keeps per-tick cost flat past the switch.
pub fn steer(
    agent: &Agent,
    neighbors: &[Agent],
    goal_m: [f32; 2],
    camera_m: [f32; 2],
    flow_m_s: [f32; 2],
    config: &CrowdConfig,
) -> Result<SteerOutput, CrowdError> {
    check_agent(agent)?;
    check_point(camera_m, "camera")?;
    check_point(flow_m_s, "flow")?;
    check_unique_ids(neighbors)?;
    let cdx = agent.pos_m[0] - camera_m[0];
    let cdz = agent.pos_m[1] - camera_m[1];
    let lod = lod_for((cdx * cdx + cdz * cdz).sqrt(), config)?;
    match lod {
        Lod::Near => {
            let (vel_m_s, _) = steer_near(agent, neighbors, goal_m, config)?;
            Ok(SteerOutput { vel_m_s, lod })
        }
        Lod::Far => {
            let vel_m_s = far_velocity(agent.pos_m, goal_m, agent.max_speed_m_s, flow_m_s, config)?;
            Ok(SteerOutput { vel_m_s, lod })
        }
        Lod::Impostor => Ok(SteerOutput {
            vel_m_s: [0.0, 0.0],
            lod,
        }),
    }
}

/// Steer a whole crowd in ascending id order with cost stats.
///
/// Lanes must align (`goals[i]`, `flows[i]` belong to `agents[i]`);
/// outputs come back in the same lane order. The output lane is reserved
/// once; per-agent ORCA work uses stack scratch only, so nothing allocates
/// after that reserve. Far and impostor agents
/// never run the ORCA loop, so density past the LOD switch adds
/// no pair checks. Near agents evaluate at most `ORCA_MAX_NEIGHBORS`
/// half-planes each, so the check count stays flat as crowds densify.
pub fn steer_all(
    agents: &[Agent],
    goals_m: &[[f32; 2]],
    camera_m: [f32; 2],
    flows_m_s: &[[f32; 2]],
    config: &CrowdConfig,
) -> Result<(Vec<SteerOutput>, SteerStats), CrowdError> {
    check_config(config)?;
    if agents.len() > MAX_AGENTS {
        return Err(CrowdError::TooManyAgents {
            got: agents.len(),
            max: MAX_AGENTS,
        });
    }
    if goals_m.len() != agents.len() || flows_m_s.len() != agents.len() {
        return Err(CrowdError::LengthMismatch {
            agents: agents.len(),
            other: goals_m.len().min(flows_m_s.len()),
        });
    }
    check_point(camera_m, "camera")?;
    for agent in agents {
        check_agent(agent)?;
    }
    check_unique_ids(agents)?;
    let mut order: Vec<usize> = (0..agents.len()).collect();
    order.sort_by_key(|i| agents[*i].id);
    let mut out: Vec<Option<SteerOutput>> = vec![None; agents.len()];
    let mut stats = SteerStats::default();
    let index = NeighborIndex::build(agents, NEIGHBOR_CELL_M);
    for i in order {
        let cdx = agents[i].pos_m[0] - camera_m[0];
        let cdz = agents[i].pos_m[1] - camera_m[1];
        let lod = lod_for((cdx * cdx + cdz * cdz).sqrt(), config)?;
        match lod {
            Lod::Near => {
                stats.near += 1;
                let preferred =
                    arrive(agents[i].pos_m, goals_m[i], agents[i].max_speed_m_s, config)?;
                let (vel_m_s, used) = if preferred == [0.0, 0.0] {
                    ([0.0, 0.0], 0)
                } else {
                    let short = index.shortlist_for(agents, &agents[i]);
                    finish_near(&agents[i], preferred, &short)?
                };
                stats.separation_pairs += used;
                out[i] = Some(SteerOutput {
                    vel_m_s,
                    lod: Lod::Near,
                });
                continue;
            }
            Lod::Far => stats.far += 1,
            Lod::Impostor => stats.impostor += 1,
        }
        // Neighbors still pass for signature stability; far/impostor paths
        // never read them, so their cost stays zero by construction.
        let cmd = steer(
            &agents[i],
            agents,
            goals_m[i],
            camera_m,
            flows_m_s[i],
            config,
        )?;
        out[i] = Some(cmd);
    }
    Ok((
        out.into_iter()
            .map(|o| o.expect("every lane steered"))
            .collect(),
        stats,
    ))
}

/// Clamp a velocity vector to a speed cap; exact zero passes through.
fn clamp_speed(vel: [f32; 2], cap: f32) -> [f32; 2] {
    let mag2 = vel[0] * vel[0] + vel[1] * vel[1];
    if mag2 <= cap * cap || mag2 <= 0.0 {
        return vel;
    }
    let mag = mag2.sqrt();
    [vel[0] / mag * cap, vel[1] / mag * cap]
}

/// Refuse bad configs before any steering work.
fn check_config(config: &CrowdConfig) -> Result<(), CrowdError> {
    if !config.separation_radius_m.is_finite()
        || !(0.0..=MAX_SEPARATION_M).contains(&config.separation_radius_m)
        || config.separation_radius_m <= 0.0
    {
        return Err(CrowdError::BadConfig("separation_radius_m"));
    }
    if !config.separation_gain.is_finite() || !(0.0..=4.0).contains(&config.separation_gain) {
        return Err(CrowdError::BadConfig("separation_gain"));
    }
    if !config.arrive_slow_m.is_finite() || !config.arrive_stop_m.is_finite() {
        return Err(CrowdError::BadConfig("arrive"));
    }
    if !(0.0..=MAX_COORD_M).contains(&config.arrive_slow_m)
        || !(0.0..=MAX_COORD_M).contains(&config.arrive_stop_m)
        || config.arrive_slow_m <= config.arrive_stop_m
    {
        return Err(CrowdError::BadConfig("arrive"));
    }
    if !config.lod_near_m.is_finite()
        || !config.impostor_m.is_finite()
        || config.lod_near_m <= 0.0
        || config.impostor_m <= config.lod_near_m
    {
        return Err(CrowdError::BadConfig("lod"));
    }
    Ok(())
}

/// Refuse non-finite or out-of-range agent lanes.
fn check_agent(agent: &Agent) -> Result<(), CrowdError> {
    check_point(agent.pos_m, "agent position")?;
    check_point(agent.vel_m_s, "agent velocity")?;
    if !agent.radius_m.is_finite() || !(0.0..=MAX_SEPARATION_M).contains(&agent.radius_m) {
        return Err(CrowdError::BadValue("agent radius"));
    }
    check_speed(agent.max_speed_m_s)?;
    Ok(())
}

/// Refuse non-finite or out-of-range 2D points.
fn check_point(point: [f32; 2], lane: &'static str) -> Result<(), CrowdError> {
    if !point.iter().all(|v| v.is_finite()) {
        return Err(CrowdError::BadValue(lane));
    }
    if !point.iter().all(|v| v.abs() <= MAX_COORD_M) {
        return Err(CrowdError::BadValue(lane));
    }
    Ok(())
}

/// Refuse non-finite or out-of-range speeds.
fn check_speed(speed: f32) -> Result<(), CrowdError> {
    if !speed.is_finite() || !(0.0..=MAX_SPEED_M_S).contains(&speed) || speed <= 0.0 {
        return Err(CrowdError::BadValue("speed"));
    }
    Ok(())
}

/// Refuse duplicate agent ids: stable ids must be unique, otherwise the
/// self-skip (`other.id == agent.id`) drops real neighbors and id-ordered
/// tie-breaks read lane order. Stack scratch only, no heap.
fn check_unique_ids(agents: &[Agent]) -> Result<(), CrowdError> {
    for (i, a) in agents.iter().enumerate() {
        for b in &agents[i + 1..] {
            if a.id == b.id {
                return Err(CrowdError::BadValue("duplicate id"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: u32, x: f32, z: f32) -> Agent {
        Agent {
            id: AgentId(id),
            pos_m: [x, z],
            vel_m_s: [0.0, 0.0],
            radius_m: 0.3,
            max_speed_m_s: 3.0,
        }
    }

    fn config() -> CrowdConfig {
        CrowdConfig::default()
    }

    #[test]
    fn separation_pushes_close_pair_apart() {
        let left = agent(1, 0.0, 0.0);
        let right = agent(2, 0.5, 0.0);
        let crowd = [left, right];
        let push_left = separation(&left, &crowd, &config()).expect("pair separates");
        let push_right = separation(&right, &crowd, &config()).expect("pair separates");
        assert!(push_left[0] < 0.0, "left moves left: {push_left:?}");
        assert!(push_right[0] > 0.0, "right moves right: {push_right:?}");
        assert!((push_left[0] + push_right[0]).abs() < 1e-5, "symmetric");
        assert!(push_left[1].abs() < 1e-5 && push_right[1].abs() < 1e-5);
    }

    #[test]
    fn separation_ignores_agents_past_radius() {
        let me = agent(1, 0.0, 0.0);
        let far = agent(2, 5.0, 0.0);
        let push = separation(&me, &[me, far], &config()).expect("far ignored");
        assert_eq!(push, [0.0, 0.0]);
    }

    #[test]
    fn separation_order_does_not_change_output() {
        let me = agent(1, 0.0, 0.0);
        let a = agent(2, 0.5, 0.1);
        let b = agent(3, -0.4, 0.2);
        let forward = separation(&me, &[a, b], &config()).expect("separates");
        let backward = separation(&me, &[b, a], &config()).expect("separates");
        assert_eq!(forward, backward);
    }

    #[test]
    fn arrive_runs_full_far_ramps_near_stops_at_goal() {
        let cfg = config();
        let full = arrive([0.0, 0.0], [10.0, 0.0], 3.0, &cfg).expect("arrives");
        assert!((full[0] - 3.0).abs() < 1e-5 && full[1].abs() < 1e-5);
        let slow = arrive([0.0, 0.0], [1.0, 0.0], 3.0, &cfg).expect("arrives");
        let slow_mag = (slow[0] * slow[0] + slow[1] * slow[1]).sqrt();
        assert!(slow_mag > 0.0 && slow_mag < 3.0, "ramps: {slow:?}");
        assert_eq!(arrive([1.0, 0.0], [1.05, 0.0], 3.0, &cfg), Ok([0.0, 0.0]));
    }

    #[test]
    fn lod_switches_on_camera_distance_only() {
        let cfg = config();
        assert_eq!(lod_for(0.0, &cfg), Ok(Lod::Near));
        assert_eq!(lod_for(14.9, &cfg), Ok(Lod::Near));
        assert_eq!(lod_for(15.0, &cfg), Ok(Lod::Near));
        assert_eq!(lod_for(15.1, &cfg), Ok(Lod::Far));
        assert_eq!(lod_for(60.0, &cfg), Ok(Lod::Far));
        assert_eq!(lod_for(60.1, &cfg), Ok(Lod::Impostor));
        assert_eq!(
            lod_for(f32::NAN, &cfg),
            Err(CrowdError::BadValue("camera distance"))
        );
    }

    #[test]
    fn far_agents_follow_flow_and_impostors_hold() {
        let cfg = config();
        let me = agent(1, 30.0, 0.0);
        let goal = [40.0, 0.0];
        let flow = [0.0, 2.0];
        let cmd = steer(&me, &[], goal, [0.0, 0.0], flow, &cfg).expect("far steers");
        assert_eq!(cmd.lod, Lod::Far);
        assert!(
            cmd.vel_m_s[1] > 0.0 && cmd.vel_m_s[0].abs() < 1e-5,
            "{cmd:?}"
        );
        let held = steer(&me, &[], goal, [200.0, 0.0], flow, &cfg).expect("impostor holds");
        assert_eq!(
            held,
            SteerOutput {
                vel_m_s: [0.0, 0.0],
                lod: Lod::Impostor
            }
        );
    }

    #[test]
    fn far_velocity_refuses_bad_flow_even_at_goal() {
        let cfg = config();
        assert_eq!(
            far_velocity([0.0, 0.0], [10.0, 0.0], 3.0, [f32::NAN, 0.0], &cfg),
            Err(CrowdError::BadValue("flow"))
        );
        assert_eq!(
            far_velocity([0.0, 0.0], [0.05, 0.0], 3.0, [f32::NAN, 0.0], &cfg),
            Err(CrowdError::BadValue("flow"))
        );
        assert!(far_velocity([0.0, 0.0], [10.0, 0.0], 3.0, [1.0, 0.0], &cfg).is_ok());
    }

    #[test]
    fn density_8_plus_64_stays_flat_past_the_switch() {
        // Plan density spec: 8 fighters plus 64 background agents. The 8
        // near agents pay capped ORCA half-planes each; the 64 far agents
        // pay none, so per-tick cost does not grow with crowd size past 15 m.
        let cfg = config();
        let camera = [0.0, 0.0];
        let mut agents = Vec::new();
        let mut goals = Vec::new();
        let mut flows = Vec::new();
        for i in 0..8u32 {
            agents.push(agent(i, 1.0 + i as f32 * 0.4, 0.0));
            goals.push([10.0, 0.0]);
            flows.push([1.0, 0.0]);
        }
        for i in 8..72u32 {
            agents.push(agent(i, 30.0 + i as f32 * 0.1, 5.0));
            goals.push([45.0, 5.0]);
            flows.push([1.0, 0.0]);
        }
        let (cmds, stats) = steer_all(&agents, &goals, camera, &flows, &cfg).expect("crowd steers");
        assert_eq!(cmds.len(), 72);
        assert_eq!((stats.near, stats.far, stats.impostor), (8, 64, 0));
        // Capped checks: each near agent constrains at most ORCA_MAX_NEIGHBORS
        // half-planes, far agents pay none.
        assert!(
            stats.separation_pairs <= 8 * ORCA_MAX_NEIGHBORS,
            "capped: {}",
            stats.separation_pairs
        );
        assert_eq!(stats.separation_pairs, 8 * ORCA_MAX_NEIGHBORS);
        // Doubling the far crowd adds outputs but zero pair checks per agent.
        let mut wide = agents.clone();
        let mut wide_goals = goals.clone();
        let mut wide_flows = flows.clone();
        for i in 72..136u32 {
            wide.push(agent(i, 31.0 + i as f32 * 0.05, 6.0));
            wide_goals.push([45.0, 6.0]);
            wide_flows.push([1.0, 0.0]);
        }
        let (_, wide_stats) =
            steer_all(&wide, &wide_goals, camera, &wide_flows, &cfg).expect("steers");
        assert_eq!(wide_stats.far, 128);
        assert_eq!(wide_stats.separation_pairs, stats.separation_pairs);
        // Twice must replay identically.
        let (again, again_stats) =
            steer_all(&agents, &goals, camera, &flows, &cfg).expect("crowd steers");
        assert_eq!(cmds, again);
        assert_eq!(stats, again_stats);
    }

    #[test]
    fn far_shared_matches_per_agent_bit_identical() {
        // Squad-commander share: one `far_velocity` call per far squad with
        // the squad flow must equal the per-agent Far path bit-for-bit.
        // 8 near plus 64 far through the shared path, digest x2 identical.
        let cfg = config();
        let camera = [0.0, 0.0];
        let mut agents = Vec::new();
        let mut goals = Vec::new();
        let mut flows = Vec::new();
        for i in 0..8u32 {
            agents.push(agent(i, 1.0 + i as f32 * 0.4, 0.0));
            goals.push([10.0, 0.0]);
            flows.push([1.0, 0.0]);
        }
        for i in 8..72u32 {
            agents.push(agent(i, 30.0 + i as f32 * 0.1, 5.0));
            goals.push([45.0, 5.0]);
            flows.push([1.0, 0.0]);
        }
        let (cmds, _) = steer_all(&agents, &goals, camera, &flows, &cfg).expect("crowd steers");
        // Shared path: one flow per 8-agent far squad, broadcast to members.
        for squad in 0..8usize {
            let flow = flows[8 + squad * 8];
            for k in 0..8usize {
                let idx = 8 + squad * 8 + k;
                let shared = far_velocity(agents[idx].pos_m, goals[idx], 3.0, flow, &cfg)
                    .expect("shared far");
                assert_eq!(
                    shared, cmds[idx].vel_m_s,
                    "far-shared differs at lane {idx}"
                );
                // Still fallback: zero flow equals arrive direction.
                let still = far_velocity(agents[idx].pos_m, goals[idx], 3.0, [0.0, 0.0], &cfg)
                    .expect("still far");
                let expect = arrive(agents[idx].pos_m, goals[idx], 3.0, &cfg).expect("arrives");
                assert_eq!(still, expect, "still fallback differs at lane {idx}");
            }
        }
        // x2 identical.
        let (again, _) = steer_all(&agents, &goals, camera, &flows, &cfg).expect("crowd steers");
        assert_eq!(cmds, again);
    }

    #[test]
    fn orca_head_on_dodges_earlier_and_smoother_than_baseline() {
        // 256 head-on agents (128 per side, slight lateral offsets): ORCA
        // anticipates with its horizon while separation only pushes inside
        // 1 m, so ORCA must keep at least as much clearance with no more
        // lateral jerk (sum of |dvz| per tick) than the baseline.
        let cfg = config();
        let camera = [0.0, 0.0];
        let mut agents = Vec::new();
        let mut goals = Vec::new();
        let mut flows = Vec::new();
        for i in 0..128u32 {
            let lane = ((i % 8) as f32 - 3.5) * 0.15;
            agents.push(agent(i, -6.0, lane));
            goals.push([6.0, -lane]);
            flows.push([1.0, 0.0]);
        }
        for i in 128..256u32 {
            let lane = ((i % 8) as f32 - 3.5) * 0.15;
            agents.push(agent(i, 6.0, lane));
            goals.push([-6.0, -lane]);
            flows.push([-1.0, 0.0]);
        }
        let (jerk_orca, min_orca, prog_orca) =
            roll_crowd(&agents, &goals, &flows, &cfg, camera, true);
        let (jerk_base, min_base, prog_base) =
            roll_crowd(&agents, &goals, &flows, &cfg, camera, false);
        assert!(
            jerk_orca <= jerk_base,
            "orca jerk {jerk_orca} over baseline {jerk_base}"
        );
        assert!(
            min_orca + 1e-4 >= min_base,
            "orca clearance {min_orca} under baseline {min_base}"
        );
        assert!(
            prog_orca >= prog_base,
            "orca progress {prog_orca} under baseline {prog_base}"
        );
    }

    #[test]
    fn orca_crossing_makes_progress_without_deadlock() {
        // Crossing flows through the origin: every agent must close distance
        // (starve guard) and stay finite, and the batch must replay exactly.
        let cfg = config();
        let camera = [0.0, 0.0];
        let mut agents = Vec::new();
        let mut goals = Vec::new();
        let mut flows = Vec::new();
        for i in 0..32u32 {
            let lane = ((i % 8) as f32 - 3.5) * 0.3;
            agents.push(agent(i, -5.0, lane));
            goals.push([5.0, lane]);
            flows.push([1.0, 0.0]);
        }
        for i in 32..64u32 {
            let lane = (((i - 32) % 8) as f32 - 3.5) * 0.3;
            agents.push(agent(i, lane, -5.0));
            goals.push([lane, 5.0]);
            flows.push([0.0, 1.0]);
        }
        let mut live = agents.clone();
        let mut start_left = 0.0f32;
        for (a, g) in agents.iter().zip(goals.iter()) {
            let dx = g[0] - a.pos_m[0];
            let dz = g[1] - a.pos_m[1];
            start_left += (dx * dx + dz * dz).sqrt();
        }
        for _ in 0..90 {
            let (cmds, _) =
                steer_all(&live, &goals, camera, &flows, &cfg).expect("crossing steers");
            for (a, cmd) in live.iter_mut().zip(cmds.iter()) {
                assert!(cmd.vel_m_s.iter().all(|v| v.is_finite()));
                let mag2 = cmd.vel_m_s[0] * cmd.vel_m_s[0] + cmd.vel_m_s[1] * cmd.vel_m_s[1];
                assert!(mag2 <= a.max_speed_m_s * a.max_speed_m_s + 1e-4);
                a.vel_m_s = cmd.vel_m_s;
                a.pos_m = integrate(a.pos_m, cmd.vel_m_s).expect("integrates");
            }
        }
        let mut end_left = 0.0f32;
        for (a, g) in live.iter().zip(goals.iter()) {
            let dx = g[0] - a.pos_m[0];
            let dz = g[1] - a.pos_m[1];
            end_left += (dx * dx + dz * dz).sqrt();
        }
        assert!(end_left < start_left, "crossing closes distance");
        let (first, _) = steer_all(&live, &goals, camera, &flows, &cfg).expect("steers");
        let (second, _) = steer_all(&live, &goals, camera, &flows, &cfg).expect("steers");
        assert_eq!(first, second, "hash x2 identical");
    }

    #[test]
    fn orca_ignores_input_shuffle_and_caps_neighbors() {
        // Same agents in two lane orders steer the same per-id velocities;
        // dense packs never exceed the neighbor cap per agent.
        let cfg = config();
        let camera = [0.0, 0.0];
        let mut agents = Vec::new();
        let mut goals = Vec::new();
        let mut flows = Vec::new();
        for i in 0..64u32 {
            let x = ((i % 8) as f32 - 3.5) * 0.25;
            let z = ((i / 8) as f32 - 3.5) * 0.25;
            agents.push(agent(i, x, z));
            goals.push([8.0, z]);
            flows.push([1.0, 0.0]);
        }
        let (fwd, fwd_stats) = steer_all(&agents, &goals, camera, &flows, &cfg).expect("steers");
        assert!(fwd_stats.separation_pairs <= 64 * ORCA_MAX_NEIGHBORS);
        let mut rev_agents = agents.clone();
        let mut rev_goals = goals.clone();
        let mut rev_flows = flows.clone();
        rev_agents.reverse();
        rev_goals.reverse();
        rev_flows.reverse();
        let (rev, _) =
            steer_all(&rev_agents, &rev_goals, camera, &rev_flows, &cfg).expect("steers");
        for (id, want) in agents.iter().zip(fwd.iter()).map(|(a, c)| (a.id, c)) {
            let slot = rev_agents
                .iter()
                .position(|a| a.id == id)
                .expect("id present");
            assert_eq!(&rev[slot], want, "id {id:?} replays");
        }
    }

    #[test]
    fn grid_shortlist_matches_brute_force_200_seeded_layouts() {
        use {crate::neighbors::NeighborIndex, engine_core::Rng};
        fn unit(rng: &mut Rng) -> f32 {
            (rng.next_u32() >> 8) as f32 / 16_777_216.0
        }
        let mut rng = Rng::new(0x2040);
        for layout in 0..200 {
            let cell_m = [1.0f32, 2.0, 4.0][layout % 3];
            let count = 2 + ((rng.next_u64() % 255) as usize);
            let mut agents = Vec::with_capacity(count);
            for i in 0..count {
                let (x, z) = if layout % 10 == 9 {
                    ((i % 16) as f32, (i / 16) as f32)
                } else if layout % 2 == 1 {
                    (unit(&mut rng) * 3.0, unit(&mut rng) * 3.0)
                } else {
                    (unit(&mut rng) * 40.0 - 20.0, unit(&mut rng) * 40.0 - 20.0)
                };
                agents.push(agent(i as u32, x, z));
            }
            let index = NeighborIndex::build(&agents, cell_m);
            for lane in 0..count {
                assert_eq!(
                    pick_nearest(&agents[lane], &agents),
                    index.shortlist_for(&agents, &agents[lane]),
                    "layout {layout} lane {lane} cell {cell_m}"
                );
            }
            let again = NeighborIndex::build(&agents, cell_m);
            assert_eq!(
                index.shortlist_for(&agents, &agents[0]),
                again.shortlist_for(&agents, &agents[0]),
                "layout {layout} rebuilds identically"
            );
        }
    }

    #[test]
    fn orca_sharded_1000_stays_capped_and_deterministic() {
        // 1000 agents steer as four 250-wide shards under MAX_AGENTS: each
        // shard stays capped per agent and replays identically.
        let cfg = config();
        let camera = [0.0, 0.0];
        let mut total_pairs = 0usize;
        for shard in 0..4u32 {
            let mut agents = Vec::new();
            let mut goals = Vec::new();
            let mut flows = Vec::new();
            for i in 0..250u32 {
                let id = shard * 250 + i;
                let x = ((i % 25) as f32 - 12.0) * 0.3;
                let z = ((i / 25) as f32 - 4.5) * 0.4 + (shard as f32) * 2.0;
                agents.push(agent(id, x, z));
                goals.push([10.0, z]);
                flows.push([1.0, 0.0]);
            }
            let (cmds, stats) =
                steer_all(&agents, &goals, camera, &flows, &cfg).expect("shard steers");
            assert_eq!(cmds.len(), 250);
            assert!(stats.separation_pairs <= stats.near * ORCA_MAX_NEIGHBORS);
            total_pairs += stats.separation_pairs;
            let (again, again_stats) =
                steer_all(&agents, &goals, camera, &flows, &cfg).expect("shard steers");
            assert_eq!(cmds, again);
            assert_eq!(stats, again_stats);
        }
        assert!(total_pairs <= 1000 * ORCA_MAX_NEIGHBORS);
    }

    #[test]
    fn orca_refuses_bad_horizon_and_holds_at_goal() {
        let me = agent(1, 0.0, 0.0);
        assert_eq!(
            orca_velocity(&me, &[], [1.0, 0.0], f32::NAN),
            Err(CrowdError::BadValue("time horizon"))
        );
        assert_eq!(
            orca_velocity(&me, &[], [1.0, 0.0], 99.0),
            Err(CrowdError::BadValue("time horizon"))
        );
        assert_eq!(
            orca_velocity(&me, &[], [f32::INFINITY, 0.0], ORCA_TIME_HORIZON_S),
            Err(CrowdError::BadValue("preferred velocity"))
        );
        // At the goal the preferred velocity is zero, so ORCA holds still
        // even shoulder to shoulder: the starve guard never drifts.
        let packed = agent(2, 0.05, 0.0);
        let held =
            orca_velocity(&me, &[me, packed], [0.0, 0.0], ORCA_TIME_HORIZON_S).expect("holds");
        assert_eq!(held, [0.0, 0.0]);
    }

    /// Rolls 90 ticks under ORCA (`use_orca`) or the separation baseline and
    /// reports `(lateral_jerk, min_pair_gap, forward_progress)`.
    fn roll_crowd(
        agents: &[Agent],
        goals: &[[f32; 2]],
        flows: &[[f32; 2]],
        cfg: &CrowdConfig,
        camera: [f32; 2],
        use_orca: bool,
    ) -> (f32, f32, f32) {
        let mut live = agents.to_vec();
        for a in live.iter_mut() {
            a.vel_m_s = [0.0, 0.0];
        }
        let mut jerk = 0.0f32;
        let mut prev_vz = vec![0.0f32; live.len()];
        let mut min_gap = f32::INFINITY;
        for _ in 0..90 {
            let mut cmds: Vec<[f32; 2]> = Vec::with_capacity(live.len());
            if use_orca {
                let (out, _) = steer_all(&live, goals, camera, flows, cfg).expect("orca steers");
                for c in out {
                    cmds.push(c.vel_m_s);
                }
            } else {
                for (a, g) in live.iter().zip(goals.iter()) {
                    let pref = arrive(a.pos_m, *g, a.max_speed_m_s, cfg).expect("arrives");
                    let sep = separation(a, &live, cfg).expect("separates");
                    cmds.push(clamp_speed(
                        [pref[0] + sep[0], pref[1] + sep[1]],
                        a.max_speed_m_s,
                    ));
                }
            }
            for (k, vz) in cmds.iter().map(|v| v[1]).enumerate() {
                jerk += (vz - prev_vz[k]).abs();
                prev_vz[k] = vz;
            }
            for (a, cmd) in live.iter_mut().zip(cmds.iter()) {
                a.vel_m_s = *cmd;
                a.pos_m = integrate(a.pos_m, *cmd).expect("integrates");
            }
            for i in 0..live.len() {
                for other in live.iter().skip(i + 1) {
                    let dx = live[i].pos_m[0] - other.pos_m[0];
                    let dz = live[i].pos_m[1] - other.pos_m[1];
                    let gap = (dx * dx + dz * dz).sqrt();
                    if gap < min_gap {
                        min_gap = gap;
                    }
                }
            }
        }
        let mut progress = 0.0f32;
        for (start, (end, goal)) in agents.iter().zip(live.iter().zip(goals.iter())) {
            let before = (goal[0] - start.pos_m[0]) * (goal[0] - start.pos_m[0])
                + (goal[1] - start.pos_m[1]) * (goal[1] - start.pos_m[1]);
            let after = (goal[0] - end.pos_m[0]) * (goal[0] - end.pos_m[0])
                + (goal[1] - end.pos_m[1]) * (goal[1] - end.pos_m[1]);
            progress += before.sqrt() - after.sqrt();
        }
        (jerk, min_gap, progress)
    }

    #[test]
    fn integrate_moves_one_tick() {
        let next = integrate([1.0, 2.0], [3.0, -6.0]).expect("integrates");
        assert!((next[0] - (1.0 + 3.0 * TICK_DT_S)).abs() < 1e-6);
        assert!((next[1] - (2.0 - 6.0 * TICK_DT_S)).abs() < 1e-6);
    }

    #[test]
    fn bad_inputs_refuse_loudly() {
        let cfg = config();
        let mut bad = agent(1, f32::NAN, 0.0);
        assert_eq!(
            separation(&bad, &[], &cfg),
            Err(CrowdError::BadValue("agent position"))
        );
        bad = agent(1, 0.0, 0.0);
        bad.max_speed_m_s = 99.0;
        assert_eq!(
            separation(&bad, &[], &cfg),
            Err(CrowdError::BadValue("speed"))
        );
        assert_eq!(
            arrive([0.0, 0.0], [f32::INFINITY, 0.0], 3.0, &cfg),
            Err(CrowdError::BadValue("goal"))
        );
        let many = vec![agent(0, 0.0, 0.0); MAX_AGENTS + 1];
        let goals = vec![[1.0, 0.0]; MAX_AGENTS + 1];
        let flows = vec![[1.0, 0.0]; MAX_AGENTS + 1];
        assert_eq!(
            steer_all(&many, &goals, [0.0, 0.0], &flows, &cfg),
            Err(CrowdError::TooManyAgents {
                got: MAX_AGENTS + 1,
                max: MAX_AGENTS
            })
        );
        let two = [agent(1, 0.0, 0.0), agent(2, 1.0, 0.0)];
        assert_eq!(
            steer_all(
                &two,
                &[[1.0, 0.0]],
                [0.0, 0.0],
                &[[1.0, 0.0], [1.0, 0.0]],
                &cfg
            ),
            Err(CrowdError::LengthMismatch {
                agents: 2,
                other: 1
            })
        );
        let bad_cfg = CrowdConfig {
            separation_radius_m: -1.0,
            ..cfg
        };
        assert_eq!(
            lod_for(5.0, &bad_cfg),
            Err(CrowdError::BadConfig("separation_radius_m"))
        );
        let bad_arrive = CrowdConfig {
            arrive_slow_m: 0.1,
            arrive_stop_m: 0.5,
            ..cfg
        };
        assert_eq!(
            lod_for(5.0, &bad_arrive),
            Err(CrowdError::BadConfig("arrive"))
        );
    }

    #[test]
    fn duplicate_ids_refuse_loudly() {
        let cfg = config();
        let dup = [agent(1, 0.0, 0.0), agent(1, 0.5, 0.0)];
        assert_eq!(
            separation(&dup[0], &dup, &cfg),
            Err(CrowdError::BadValue("duplicate id"))
        );
        assert_eq!(
            orca_velocity(&dup[0], &dup, [1.0, 0.0], ORCA_TIME_HORIZON_S),
            Err(CrowdError::BadValue("duplicate id"))
        );
        assert_eq!(
            steer(&dup[0], &dup, [10.0, 0.0], [0.0, 0.0], [1.0, 0.0], &cfg),
            Err(CrowdError::BadValue("duplicate id"))
        );
        let goals = [[10.0, 0.0], [10.0, 0.0]];
        let flows = [[1.0, 0.0], [1.0, 0.0]];
        assert_eq!(
            steer_all(&dup, &goals, [0.0, 0.0], &flows, &cfg),
            Err(CrowdError::BadValue("duplicate id"))
        );
    }

    #[test]
    fn orca_velocity_refuses_far_nan_neighbor() {
        let me = agent(1, 0.0, 0.0);
        let mut neighbors = vec![me];
        for id in 2..12u32 {
            neighbors.push(agent(id, 0.5 + (id as f32) * 0.01, 0.0));
        }
        neighbors.push(agent(99, f32::NAN, 0.0));
        assert_eq!(
            orca_velocity(&me, &neighbors, [1.0, 0.0], ORCA_TIME_HORIZON_S),
            Err(CrowdError::BadValue("agent position"))
        );
        assert_eq!(
            separation(&me, &neighbors, &config()),
            Err(CrowdError::BadValue("agent position"))
        );
    }
}
