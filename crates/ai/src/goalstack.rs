//! goalstack: bounded ioq3-style goal memory plus fuzzy intent weights.
//! Owner: `Brain` keeps one stack plus one nerve table per fighter.
//! Units: permille weights, tick costs, integer cells, tick stamps.
//! Determinism: integer math only, stable order, no wall clock.
//! Errors: [`StackError`]; bad weights refuse with `None`.
//! PORT(ioquake/ioq3@87a99d18206e878a20289f05ac3c16005dcae5c3:code/botlib/be_ai_goal.h,be_ai_weight.h): stack discipline plus separator weights in integers, zero donor bytes.

/// Stack cap, donor `MAX_GOALSTACK` shape. Units: goals.
pub const MAX_GOALSTACK: usize = 8;
/// Stamina separator: below reads exhausted. Units: permille.
pub const STAMINA_LO: u32 = 150;
/// Stamina separator: above reads fresh. Units: permille.
pub const STAMINA_HI: u32 = 400;
/// Range separator: inside reads close. Units: mm.
pub const RANGE_CLOSE_MM: u32 = 2_500;
/// Range separator: beyond reads far. Units: mm.
pub const RANGE_FAR_MM: u32 = 8_000;
/// Morale separator: below reads shaken. Units: permille.
pub const MORALE_LO: u32 = 350;
/// Morale separator: above reads bold. Units: permille.
pub const MORALE_HI: u32 = 650;

/// One pursuable goal: id, worth, travel cost, map cell, avoid expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Goal {
    /// Goal id; the NBG split skips the LTG by this id.
    pub id: u32,
    /// Worth `0..=1000`; refused above 1000 at construction.
    pub weight: u32,
    /// Travel cost in ticks; LTG scores `weight * 1000 / (cost + 1)`.
    pub cost: u32,
    /// Map cell x in integer cells.
    pub x: i32,
    /// Map cell y in integer cells.
    pub y: i32,
    /// First tick this goal is pursuable again; `0` means never avoided.
    pub avoid_until: u32,
}

impl Goal {
    /// New goal, no avoidance. `None` when `weight` tops 1000.
    pub const fn new(id: u32, weight: u32, cost: u32, x: i32, y: i32) -> Option<Self> {
        if weight > 1_000 {
            return None;
        }
        Some(Self {
            id,
            weight,
            cost,
            x,
            y,
            avoid_until: 0,
        })
    }
    /// True once `now` reaches `avoid_until`. Units: ticks.
    pub const fn pursuable(&self, now: u32) -> bool {
        now >= self.avoid_until
    }
}

/// Stack failures, never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError {
    /// Stack holds 8 already.
    Full,
}

impl core::fmt::Display for StackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("goal stack: full")
    }
}

/// Bounded LIFO goal memory, newest on top. Errors: [`StackError`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoalStack {
    goals: [Goal; MAX_GOALSTACK],
    len: usize,
}

impl GoalStack {
    /// Empty stack.
    pub const fn new() -> Self {
        const BLANK: Goal = Goal {
            id: 0,
            weight: 0,
            cost: 0,
            x: 0,
            y: 0,
            avoid_until: 0,
        };
        Self {
            goals: [BLANK; MAX_GOALSTACK],
            len: 0,
        }
    }
    /// True when nothing is held.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Push on top; refuses past cap 8.
    pub fn push(&mut self, goal: Goal) -> Result<(), StackError> {
        if self.len >= MAX_GOALSTACK {
            return Err(StackError::Full);
        }
        self.goals[self.len] = goal;
        self.len += 1;
        Ok(())
    }
    /// Pop the top; `None` on empty, never panics.
    pub fn pop(&mut self) -> Option<Goal> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.goals[self.len])
    }
    /// Drop every goal.
    pub fn empty(&mut self) {
        self.len = 0;
    }
    /// Newest goal, if any.
    pub fn top(&self) -> Option<&Goal> {
        if self.len == 0 {
            None
        } else {
            Some(&self.goals[self.len - 1])
        }
    }
    /// Goal below the top, if any.
    pub fn second(&self) -> Option<&Goal> {
        if self.len < 2 {
            None
        } else {
            Some(&self.goals[self.len - 2])
        }
    }
    /// Stamp avoidance on `id`; true when found. Units: ticks.
    pub fn set_avoid(&mut self, id: u32, until: u32) -> bool {
        let mut found = false;
        for g in self.goals[..self.len].iter_mut() {
            if g.id == id {
                g.avoid_until = until;
                found = true;
            }
        }
        found
    }
    /// Best long-term goal: max `weight * 1000 / (cost + 1)` over pursuable
    /// goals, ties keep the earlier push. Units: score.
    pub fn choose_ltg(&self, now: u32) -> Option<&Goal> {
        let mut best: Option<&Goal> = None;
        let mut best_score = 0u32;
        let mut first = true;
        for g in self.goals[..self.len].iter() {
            if g.pursuable(now) {
                let score = g.weight.saturating_mul(1_000) / g.cost.saturating_add(1);
                if first || score > best_score {
                    best = Some(g);
                    best_score = score;
                    first = false;
                }
            }
        }
        best
    }
    /// Best nearby goal apart from the LTG whose detour never lengthens
    /// travel: `to(nbg) + at(nbg, ltg) <= to(ltg)`. Highest weight wins,
    /// ties keep the earlier push; `None` when no safe detour exists.
    pub fn choose_nbg(&self, now: u32, sx: i32, sy: i32) -> Option<&Goal> {
        let ltg = self.choose_ltg(now)?;
        let direct = manhattan(sx, sy, ltg.x, ltg.y);
        let mut best: Option<&Goal> = None;
        for g in self.goals[..self.len].iter() {
            let via = manhattan(sx, sy, g.x, g.y).saturating_add(manhattan(g.x, g.y, ltg.x, ltg.y));
            if g.id != ltg.id
                && g.pursuable(now)
                && via <= direct
                && best.map_or(true, |b| g.weight > b.weight)
            {
                best = Some(g);
            }
        }
        best
    }
}

/// Manhattan distance in cells. Units: cells, saturating.
fn manhattan(ax: i32, ay: i32, bx: i32, by: i32) -> u32 {
    let dx = u32::try_from(ax.saturating_sub(bx).saturating_abs()).unwrap_or(u32::MAX);
    let dy = u32::try_from(ay.saturating_sub(by).saturating_abs()).unwrap_or(u32::MAX);
    dx.saturating_add(dy)
}

/// Per-lane nerve table in lane order. Units: permille 0..=1000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeightTable {
    /// Lane weights.
    pub lanes: [u32; 5],
}

impl WeightTable {
    /// New table; `None` when any lane tops 1000.
    pub const fn new(lanes: [u32; 5]) -> Option<Self> {
        if lanes[0] > 1_000
            || lanes[1] > 1_000
            || lanes[2] > 1_000
            || lanes[3] > 1_000
            || lanes[4] > 1_000
        {
            return None;
        }
        Some(Self { lanes })
    }
    /// Neutral nerve: no lean.
    pub const fn neutral() -> Self {
        Self {
            lanes: [1_000, 1_000, 1_000, 1_000, 1_000],
        }
    }
    /// Bold nerve: strikes and presses.
    pub const fn bold() -> Self {
        Self {
            lanes: [1_000, 200, 200, 800, 200],
        }
    }
    /// Shy nerve: guards and yields.
    pub const fn shy() -> Self {
        Self {
            lanes: [200, 1_000, 900, 200, 1_000],
        }
    }
    /// True for the all-1000 table.
    pub const fn is_neutral(&self) -> bool {
        self.lanes[0] == 1_000
            && self.lanes[1] == 1_000
            && self.lanes[2] == 1_000
            && self.lanes[3] == 1_000
            && self.lanes[4] == 1_000
    }
}

/// Separator-shaped weights over stamina, range and morale: table lanes
/// scaled by one factor per input. Fresh, close, bold briefs keep attack;
/// spent, far, shaken ones lean guard. Three permille factors chain over
/// 1e9 in `u64`, so every lane stays `0..=1000` permille. Units: permille,
/// deterministic.
pub fn weights_for(stamina: u32, range_mm: u32, morale: u32, table: &WeightTable) -> [u32; 5] {
    let stamina_factor: [u32; 5] = if stamina < STAMINA_LO {
        [200, 1_000, 900, 500, 800]
    } else if stamina < STAMINA_HI {
        [600, 800, 700, 800, 600]
    } else {
        [1_000, 500, 500, 800, 400]
    };
    let range_factor: [u32; 5] = if range_mm <= RANGE_CLOSE_MM {
        [1_000, 700, 800, 300, 400]
    } else if range_mm <= RANGE_FAR_MM {
        [600, 600, 500, 800, 500]
    } else {
        [100, 300, 200, 1_000, 500]
    };
    let morale_factor: [u32; 5] = if morale < MORALE_LO {
        [300, 1_000, 900, 400, 900]
    } else if morale < MORALE_HI {
        [700, 700, 700, 700, 600]
    } else {
        [1_000, 400, 400, 900, 300]
    };
    let mut out = [0u32; 5];
    let mut i = 0;
    while i < 5 {
        let scaled = u64::from(table.lanes[i])
            .saturating_mul(u64::from(stamina_factor[i]))
            .saturating_mul(u64::from(range_factor[i]))
            .saturating_mul(u64::from(morale_factor[i]))
            / 1_000_000_000;
        out[i] = u32::try_from(scaled).unwrap_or(u32::MAX).min(1_000);
        i += 1;
    }
    out
}

/// Normalized blend: winning lane of `scores[i] * weights[i]`, ties keep
/// the lower lane index. Units: lane index, deterministic.
pub fn blend_lanes(scores: [i32; 5], weights: [u32; 5]) -> usize {
    let w0 = i32::try_from(weights[0].clamp(0, 1_000)).unwrap_or(1_000);
    let mut best = 0usize;
    let mut best_value = scores[0].saturating_mul(w0);
    let mut i = 1;
    while i < 5 {
        let w = i32::try_from(weights[i].clamp(0, 1_000)).unwrap_or(1_000);
        if scores[i].saturating_mul(w) > best_value {
            best = i;
            best_value = scores[i].saturating_mul(w);
        }
        i += 1;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fnv64;

    fn goal(id: u32, weight: u32, cost: u32, x: i32, y: i32) -> Goal {
        Goal::new(id, weight, cost, x, y).expect("fixture weights are legal")
    }

    fn chase() -> GoalStack {
        let mut stack = GoalStack::new();
        for g in [
            goal(1, 200, 4, 4, 0),
            goal(2, 900, 10, 10, 0),
            goal(3, 800, 9, 0, 9),
        ] {
            stack.push(g).expect("cap 8 holds three");
        }
        stack
    }

    /// Scenario checksum over LTG/NBG choice plus bold/shy blend lanes.
    /// The hash feeds the bold lane then the shy lane in canonical order,
    /// so computing shy-first replays the same digest bit-identically.
    fn scenario_digest(bold_first: bool) -> (u64, usize, usize) {
        let stack = chase();
        let ltg = stack.choose_ltg(50).expect("ltg exists").id;
        let nbg = stack.choose_nbg(50, 0, 0).map_or(0, |g| g.id);
        let scores = [600, 590, 580, 570, 560];
        let (first, second) = if bold_first {
            (WeightTable::bold(), WeightTable::shy())
        } else {
            (WeightTable::shy(), WeightTable::bold())
        };
        let a = blend_lanes(scores, weights_for(250, 5_000, 500, &first));
        let b = blend_lanes(scores, weights_for(250, 5_000, 500, &second));
        let (bold_lane, shy_lane) = if bold_first { (a, b) } else { (b, a) };
        let mut hash = Fnv64::new();
        hash.write_u64(u64::from(ltg));
        hash.write_u64(u64::from(nbg));
        hash.write_u64(u64::try_from(bold_lane).unwrap_or(u64::MAX));
        hash.write_u64(u64::try_from(shy_lane).unwrap_or(u64::MAX));
        (hash.finish(), a, b)
    }

    #[test]
    fn goalstack_pursue_blend() {
        // (a) push 3: top is last-pushed; pop resumes; empty clears.
        let mut stack = GoalStack::new();
        assert!(stack.pop().is_none());
        for g in [
            goal(1, 500, 5, 1, 0),
            goal(2, 500, 5, 2, 0),
            goal(3, 500, 5, 3, 0),
        ] {
            stack.push(g).expect("cap 8 holds three");
        }
        assert_eq!(stack.top().map(|g| g.id), Some(3));
        assert_eq!(stack.second().map(|g| g.id), Some(2));
        assert_eq!(stack.pop().map(|g| g.id), Some(3));
        assert_eq!(stack.top().map(|g| g.id), Some(2));
        stack.empty();
        assert!(stack.is_empty() && stack.top().is_none() && stack.second().is_none());
        let mut full = GoalStack::new();
        for id in 0..8 {
            full.push(goal(id, 100, 1, 0, 0)).expect("first eight fit");
        }
        assert_eq!(full.push(goal(8, 100, 1, 0, 0)), Err(StackError::Full));
        assert!(Goal::new(9, 1_001, 1, 0, 0).is_none());
        assert!(WeightTable::new([1_000, 1_000, 1_000, 1_000, 1_001]).is_none());
        // (b) LTG wins on weight x cost; the NBG detour never lengthens travel.
        let chase = chase();
        let ltg = chase.choose_ltg(50).expect("ltg exists");
        assert_eq!(ltg.id, 2);
        let nbg = chase.choose_nbg(50, 0, 0).expect("safe detour exists");
        assert_eq!(nbg.id, 1);
        let direct = manhattan(0, 0, ltg.x, ltg.y);
        let via =
            manhattan(0, 0, nbg.x, nbg.y).saturating_add(manhattan(nbg.x, nbg.y, ltg.x, ltg.y));
        assert!(via <= direct);
        // (c) avoided goal skipped before expiry, pursuable at expiry tick.
        let mut shy = chase.clone();
        assert!(shy.set_avoid(2, 50));
        assert_ne!(shy.choose_ltg(10).map(|g| g.id), Some(2));
        assert_eq!(shy.choose_ltg(50).map(|g| g.id), Some(2));
        // (d) sensible lane plus bold/shy flip on identical briefs: the
        // contested mid brief (spent neither fresh, mid range, neutral
        // nerve) advances bold but guards shy.
        let scores = [600, 590, 580, 570, 560];
        let bold_lane = blend_lanes(scores, weights_for(250, 5_000, 500, &WeightTable::bold()));
        let shy_lane = blend_lanes(scores, weights_for(250, 5_000, 500, &WeightTable::shy()));
        assert_eq!(bold_lane, 3);
        assert_eq!(shy_lane, 1);
        assert_eq!(
            blend_lanes([5, 5, 5, 5, 5], [1_000, 1_000, 1_000, 1_000, 1_000]),
            0
        );
        for table in [WeightTable::bold(), WeightTable::shy()] {
            for w in weights_for(250, 5_000, 500, &table) {
                assert!(w <= 1_000);
            }
        }
        // (e) digest twice identical.
        let (d1, a1, b1) = scenario_digest(true);
        let (d2, a2, b2) = scenario_digest(true);
        assert_eq!((d1, a1, b1), (d2, a2, b2));
        assert_eq!(d1, scenario_digest(false).0);
    }
}
