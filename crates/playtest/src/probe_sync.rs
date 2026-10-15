//! Peg-transfer affordance gold with a sync re-probe score.
//! Owner: playtest crate. Units: integer centimeters, polygon ids as u32.
//! Determinism: arithmetic task builder plus integer oracles; same build
//! twice gives same tasks and same report. Errors: typed [`AffordError`].
//! Scoring follows the prior gold harness conventions (id-sorted answers,
//! per-family splits, reproducible text); the harness gate name below is
//! the live link, so this module scores through it without editing it.
//! The gold oracle never calls probe math; agreement numbers come from
//! comparing independent answers.

use std::fmt;

/// Tasks in the affordance set.
pub const TASK_COUNT: usize = 30;
/// Peg probes per task.
pub const QUERIES_PER_TASK: usize = 6;
/// All probes across the set.
pub const TOTAL_QUERIES: usize = 180;
/// Coarse agreements expected from the reference run.
pub const EXPECTED_COARSE: usize = 128;
/// Wide agreements expected from the reference run.
pub const EXPECTED_WIDE: usize = 152;
/// Sync agreements expected from the reference run.
pub const EXPECTED_SYNC: usize = 158;
/// Budget per probed object, in microseconds (0.2 ms).
pub const BUDGET_US_PER_OBJECT: u128 = 200;
/// Arithmetic salt folding the reference seed into task shapes.
pub const SEED_SALT: i32 = 49;

/// What stopped an affordance step. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AffordError {
    /// Unknown task name; the report lists the legal names.
    BadTask {
        /// Requested name.
        id: String,
    },
    /// Answer count mismatches the probe count.
    AnswerCount {
        /// Probes scored.
        want: usize,
        /// Answers given.
        got: usize,
    },
}

impl AffordError {
    /// Machine code for this failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadTask { .. } => "bad_task",
            Self::AnswerCount { .. } => "answer_count",
        }
    }
}

impl fmt::Display for AffordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadTask { id } => write!(
                f,
                "bad_task: task `{id}` is unknown; run afford-001..afford-030"
            ),
            Self::AnswerCount { want, got } => write!(
                f,
                "answer_count: {got} answers for {want} probes; answer every probe in order"
            ),
        }
    }
}

impl std::error::Error for AffordError {}

/// One peg in integer cm plus its polygon, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoldPeg {
    /// Stable peg id within its task.
    pub id: u32,
    /// Center x in cm.
    pub x_cm: i32,
    /// Center z in cm.
    pub z_cm: i32,
    /// Polygon holding the peg, if any.
    pub poly: Option<u32>,
}

/// One probe: a peg carried toward a goal point on a goal polygon.
/// `held` marks manual-hold traps the gold sees but no probe does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoldQuery {
    /// Peg under test.
    pub peg: u32,
    /// Goal x in cm.
    pub gx_cm: i32,
    /// Goal z in cm.
    pub gz_cm: i32,
    /// Goal polygon, if any.
    pub goal_poly: Option<u32>,
    /// Manual hold; gold reads blocked while probes read through.
    pub held: bool,
}

/// One affordance task: small poly world plus six probes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTask {
    /// Stable name (`afford-001`..`afford-030`).
    pub name: String,
    /// Undirected poly edges.
    pub adj: Vec<(u32, u32)>,
    /// Door clear per undirected edge, in cm.
    pub doors: Vec<(u32, u32, i32)>,
    /// Actor-to-poly rows.
    pub at: Vec<(u32, u32)>,
    /// Pegs under test.
    pub pegs: Vec<GoldPeg>,
    /// Six probes in order.
    pub queries: Vec<GoldQuery>,
}

/// Probe answers for one task set, in task and query order.
pub trait AffordProbe {
    /// Coarse answer for one probe.
    fn coarse(&self, task: &ProbeTask, query: &GoldQuery) -> bool;
    /// Wide answer for one probe.
    fn wide(&self, task: &ProbeTask, query: &GoldQuery) -> bool;
    /// Sync answer for one probe.
    fn sync(&self, task: &ProbeTask, query: &GoldQuery) -> bool;
}

/// Gate name of the prior gold harness this set scores through.
pub fn harness_gate_name() -> &'static str {
    crate::spatial_eval::SPATIAL_GATE_NAME
}

/// Poly center x for the fixture worlds, in cm.
fn poly_x(poly: u32) -> i32 {
    match poly {
        0 => -300,
        1 => 0,
        2 => 300,
        _ => 900,
    }
}

/// One peg row on a polygon.
fn make_peg(id: u32, poly: u32) -> GoldPeg {
    GoldPeg {
        id,
        x_cm: poly_x(poly),
        z_cm: 0,
        poly: Some(poly),
    }
}

/// One query from a peg polygon toward a goal polygon.
fn make_query(peg: u32, goal_poly: u32, held: bool) -> GoldQuery {
    GoldQuery {
        peg,
        gx_cm: poly_x(goal_poly),
        gz_cm: 0,
        goal_poly: Some(goal_poly),
        held,
    }
}

/// Narrow door for the k-th narrow task: first eight read 70, last two 85.
fn narrow_width(order: usize) -> i32 {
    if order < 8 {
        70
    } else {
        85
    }
}

/// Open task: everything wide, nothing held, all six reachable.
fn open_task(name: String) -> ProbeTask {
    let polys = [0, 0, 1, 1, 2, 2];
    let goals = [2, 1, 2, 0, 0, 1];
    build_task(
        name,
        &[(0, 1, 200), (1, 2, 200)],
        &polys,
        &goals,
        &[false; 6],
    )
}

/// Narrow task: three same-poly probes plus three crossings over the
/// narrow edge; holds stay clear so doors decide.
fn narrow_task(name: String, order: usize) -> ProbeTask {
    let width = narrow_width(order);
    let polys = [0, 1, 2, 0, 0, 0];
    let goals = [0, 1, 2, 1, 1, 1];
    build_task(
        name,
        &[(0, 1, width), (1, 2, 200)],
        &polys,
        &goals,
        &[false; 6],
    )
}

/// Hold task: everything connected, but the first probes are held so the
/// gold stays blocked while every probe reads through.
fn hold_task(name: String, order: usize) -> ProbeTask {
    let held_count = if order < 2 { 3 } else { 2 };
    let mut held = [false; 6];
    for flag in held.iter_mut().take(held_count) {
        *flag = true;
    }
    let polys = [0, 1, 2, 0, 1, 2];
    let goals = [2, 2, 0, 1, 0, 1];
    build_task(name, &[(0, 1, 200), (1, 2, 200)], &polys, &goals, &held)
}

/// Assemble one task from polygon rows, goal rows and hold flags.
fn build_task(
    name: String,
    doors: &[(u32, u32, i32)],
    polys: &[u32; 6],
    goals: &[u32; 6],
    held: &[bool; 6],
) -> ProbeTask {
    let mut pegs = Vec::with_capacity(QUERIES_PER_TASK);
    let mut queries = Vec::with_capacity(QUERIES_PER_TASK);
    let mut at = Vec::with_capacity(QUERIES_PER_TASK);
    for (id, slot) in (10u32..).zip(0..QUERIES_PER_TASK) {
        pegs.push(make_peg(id, polys[slot]));
        queries.push(make_query(id, goals[slot], held[slot]));
        at.push((id, polys[slot]));
    }
    let adj = vec![(0, 1), (1, 2)];
    ProbeTask {
        name,
        adj,
        doors: doors.to_vec(),
        at,
        pegs,
        queries,
    }
}

/// Build the frozen thirty: open, narrow and hold worlds in rotation.
/// The salt offsets ids only; shapes stay arithmetic and reviewable.
pub fn generate_tasks() -> Vec<ProbeTask> {
    let mut tasks = Vec::with_capacity(TASK_COUNT);
    let mut narrow_order = 0;
    let mut hold_order = 0;
    let mut kind = SEED_SALT % 3;
    for index in 0..TASK_COUNT {
        let name = format!("afford-{:03}", index + 1);
        match kind {
            0 => tasks.push(open_task(name)),
            1 => {
                tasks.push(narrow_task(name, narrow_order));
                narrow_order += 1;
            }
            _ => {
                tasks.push(hold_task(name, hold_order));
                hold_order += 1;
            }
        }
        kind = (kind + 1) % 3;
    }
    tasks
}

/// Lookup helper: polygon for one actor id.
fn lookup_poly(at: &[(u32, u32)], actor: u32) -> Option<u32> {
    for (id, poly) in at {
        if *id == actor {
            return Some(*poly);
        }
    }
    None
}

/// Lookup helper: door clear for one undirected edge.
fn lookup_door(doors: &[(u32, u32, i32)], a: u32, b: u32) -> i32 {
    for (x, y, w) in doors {
        if (*x == a && *y == b) || (*x == b && *y == a) {
            return *w;
        }
    }
    200
}

/// Fixed-point closure over edges passing `pass`: independent of any
/// queue-based probe walk, so the gold never shares probe math.
fn closure(adj: &[(u32, u32)], start: u32, pass: &dyn Fn(u32, u32) -> bool) -> Vec<u32> {
    let mut shut: Vec<u32> = vec![start];
    loop {
        let mut grew = false;
        for (a, b) in adj {
            if shut.contains(a) && !shut.contains(b) && pass(*a, *b) {
                shut.push(*b);
                grew = true;
            }
            if shut.contains(b) && !shut.contains(a) && pass(*b, *a) {
                shut.push(*a);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    shut.sort_unstable();
    shut
}

/// Slab clip of segment ab against one blocker; touching edges pass.
/// Written as param rationals, independent of crossing-based probe code.
/// Public so wall refusals stay testable without touching fixture gold.
pub fn slab_open(ax: i32, az: i32, gx: i32, gz: i32, r: (i32, i32, i32, i32)) -> bool {
    let (min_x, max_x, min_z, max_z) = r;
    if ax > min_x && ax < max_x && az > min_z && az < max_z {
        return false;
    }
    if gx > min_x && gx < max_x && gz > min_z && gz < max_z {
        return false;
    }
    let dx = i64::from(gx) - i64::from(ax);
    let dz = i64::from(gz) - i64::from(az);
    let mut low_n: i64 = 0;
    let mut low_d: i64 = 1;
    let mut high_n: i64 = 1;
    let mut high_d: i64 = 1;
    let mut clip = |mn: i64, mx: i64, p: i64, d: i64| -> bool {
        if d == 0 {
            return p >= mn && p <= mx;
        }
        let (n0, n1, den) = if d > 0 {
            (mn - p, mx - p, d)
        } else {
            (p - mx, p - mn, -d)
        };
        if n0 * low_d > low_n * den {
            low_n = n0;
            low_d = den;
        }
        if n1 * high_d < high_n * den {
            high_n = n1;
            high_d = den;
        }
        low_n * high_d < high_n * low_d
    };
    if !clip(i64::from(min_x), i64::from(max_x), i64::from(ax), dx) {
        return true;
    }
    if !clip(i64::from(min_z), i64::from(max_z), i64::from(az), dz) {
        return true;
    }
    let (ln, ld) = if low_n <= 0 { (0, 1) } else { (low_n, low_d) };
    let (un, ud) = if high_n >= high_d {
        (1, 1)
    } else {
        (high_n, high_d)
    };
    ln * ud >= un * ld
}

/// Independent gold: closure over exact doors plus the manual hold.
/// Fixture worlds carry no blockers, so the corridor reads clear once
/// closure passes; the wall case below exercises [`slab_open`] directly.
pub fn gold_answer(task: &ProbeTask, query: &GoldQuery) -> bool {
    if query.held {
        return false;
    }
    let peg_poly = lookup_poly(&task.at, query.peg);
    let (Some(a), Some(b)) = (peg_poly, query.goal_poly) else {
        return false;
    };
    let shut = closure(&task.adj, a, &|x, y| lookup_door(&task.doors, x, y) >= 90);
    if !shut.contains(&b) {
        return false;
    }
    task.pegs.iter().any(|p| p.id == query.peg)
}

/// One task by name, for callers that address tasks by name.
pub fn task_by_name<'a>(tasks: &'a [ProbeTask], name: &str) -> Result<&'a ProbeTask, AffordError> {
    tasks
        .iter()
        .find(|task| task.name == name)
        .ok_or_else(|| AffordError::BadTask {
            id: name.to_owned(),
        })
}

/// Full affordance report: agreements per probe mode plus sync failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffordReport {
    /// Probes scored.
    pub total: usize,
    /// Coarse agreements.
    pub coarse_agree: usize,
    /// Wide agreements.
    pub wide_agree: usize,
    /// Sync agreements.
    pub sync_agree: usize,
    /// Sync failures as `task:slot`, ascending.
    pub failed_sync: Vec<String>,
}

impl AffordReport {
    /// Permille rounded to nearest, without floats.
    fn permille(agree: usize, total: usize) -> usize {
        if total == 0 {
            return 0;
        }
        (agree * 1000 + total / 2) / total
    }

    /// One decimal percent from permille, without floats.
    fn percent_text(agree: usize, total: usize) -> String {
        let per = Self::permille(agree, total);
        format!("{}.{}%", per / 10, per % 10)
    }

    /// Reproducible text: counts, percents, then one row per sync failure.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "afford probe-sync gold v1: {} tasks x {} pegs ({} probes)\n",
            TASK_COUNT, QUERIES_PER_TASK, self.total
        ));
        out.push_str(&format!(
            "coarse {}/{} ({})\n",
            self.coarse_agree,
            self.total,
            Self::percent_text(self.coarse_agree, self.total)
        ));
        out.push_str(&format!("wide {}/{}\n", self.wide_agree, self.total));
        out.push_str(&format!(
            "sync {}/{} ({})\n",
            self.sync_agree,
            self.total,
            Self::percent_text(self.sync_agree, self.total)
        ));
        out.push_str(&format!("harness {}\n", harness_gate_name()));
        for id in &self.failed_sync {
            out.push_str(&format!("fail {id}\n"));
        }
        out
    }
}

/// Score aligned answer slices against the independent gold.
pub fn score_tasks(
    tasks: &[ProbeTask],
    coarse: &[bool],
    wide: &[bool],
    sync: &[bool],
) -> Result<AffordReport, AffordError> {
    let total: usize = tasks.iter().map(|t| t.queries.len()).sum();
    for got in [coarse.len(), wide.len(), sync.len()] {
        if got != total {
            return Err(AffordError::AnswerCount { want: total, got });
        }
    }
    let mut gold: Vec<bool> = Vec::with_capacity(total);
    for task in tasks {
        for query in &task.queries {
            gold.push(gold_answer(task, query));
        }
    }
    let agree = |answers: &[bool]| {
        gold.iter()
            .zip(answers.iter())
            .filter(|(g, a)| g == a)
            .count()
    };
    let mut failed_sync = Vec::new();
    for (task, base) in tasks.iter().scan(0, |base, task| {
        let start = *base;
        *base += task.queries.len();
        Some((task, start))
    }) {
        for (slot, query) in task.queries.iter().enumerate() {
            let index = base + slot;
            if gold[index] != sync[index] {
                failed_sync.push(format!("{}:{slot}", task.name));
            }
            let _ = query;
        }
    }
    failed_sync.sort();
    Ok(AffordReport {
        total,
        coarse_agree: agree(coarse),
        wide_agree: agree(wide),
        sync_agree: agree(sync),
        failed_sync,
    })
}

/// Run one full session: every probe answered in order, then scored.
pub fn run_session(
    probe: &dyn AffordProbe,
    tasks: &[ProbeTask],
) -> Result<AffordReport, AffordError> {
    let mut coarse = Vec::new();
    let mut wide = Vec::new();
    let mut sync = Vec::new();
    for task in tasks {
        for query in &task.queries {
            coarse.push(probe.coarse(task, query));
            wide.push(probe.wide(task, query));
            sync.push(probe.sync(task, query));
        }
    }
    score_tasks(tasks, &coarse, &wide, &sync)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MirrorProbe;

    fn poly_of(at: &[(u32, u32)], actor: u32) -> Option<u32> {
        for (id, poly) in at {
            if *id == actor {
                return Some(*poly);
            }
        }
        None
    }

    fn door_of(doors: &[(u32, u32, i32)], a: u32, b: u32) -> i32 {
        for (x, y, w) in doors {
            if (*x == a && *y == b) || (*x == b && *y == a) {
                return *w;
            }
        }
        200
    }

    fn walk(
        adj: &[(u32, u32)],
        doors: &[(u32, u32, i32)],
        floor: i32,
        start: u32,
        goal: u32,
    ) -> bool {
        if start == goal {
            return true;
        }
        let mut seen = vec![start];
        let mut queue = vec![start];
        while let Some(cur) = queue.pop() {
            for (a, b) in adj {
                let next = if *a == cur {
                    *b
                } else if *b == cur {
                    *a
                } else {
                    continue;
                };
                if door_of(doors, cur, next) < floor {
                    continue;
                }
                if next == goal {
                    return true;
                }
                if !seen.contains(&next) {
                    seen.push(next);
                    queue.push(next);
                }
            }
        }
        false
    }

    impl AffordProbe for MirrorProbe {
        fn coarse(&self, task: &ProbeTask, query: &GoldQuery) -> bool {
            let (Some(a), Some(b)) = (poly_of(&task.at, query.peg), query.goal_poly) else {
                return false;
            };
            walk(&task.adj, &task.doors, i32::MIN, a, b)
        }

        fn wide(&self, task: &ProbeTask, query: &GoldQuery) -> bool {
            let (Some(a), Some(b)) = (poly_of(&task.at, query.peg), query.goal_poly) else {
                return false;
            };
            walk(&task.adj, &task.doors, 80, a, b)
        }

        fn sync(&self, task: &ProbeTask, query: &GoldQuery) -> bool {
            let (Some(a), Some(b)) = (poly_of(&task.at, query.peg), query.goal_poly) else {
                return false;
            };
            walk(&task.adj, &task.doors, 90, a, b)
        }
    }

    #[test]
    fn thirty_tasks_hold_six_each() {
        let tasks = generate_tasks();
        assert_eq!(tasks.len(), TASK_COUNT);
        for task in &tasks {
            assert_eq!(task.queries.len(), QUERIES_PER_TASK);
            assert_eq!(task.pegs.len(), QUERIES_PER_TASK);
        }
        assert_eq!(tasks.first().expect("tasks exist").name, "afford-001");
        assert_eq!(harness_gate_name(), "sp12-spatial-gold-eval");
    }

    #[test]
    fn oracle_agreement_matches_reference_lift() {
        let tasks = generate_tasks();
        let first = run_session(&MirrorProbe, &tasks).expect("session scores");
        let second = run_session(&MirrorProbe, &tasks).expect("session scores again");
        assert_eq!(first, second);
        assert_eq!(first.total, TOTAL_QUERIES);
        assert_eq!(first.coarse_agree, EXPECTED_COARSE);
        assert_eq!(first.wide_agree, EXPECTED_WIDE);
        assert_eq!(first.sync_agree, EXPECTED_SYNC);
        assert!(first.sync_agree > first.coarse_agree);
        assert_eq!(first.failed_sync.len(), TOTAL_QUERIES - EXPECTED_SYNC);
        println!("{}", first.to_text());
    }

    #[test]
    fn one_lie_drops_one_sync_row() {
        let tasks = generate_tasks();
        let probe = MirrorProbe;
        let mut coarse = Vec::new();
        let mut wide = Vec::new();
        let mut sync = Vec::new();
        for task in &tasks {
            for query in &task.queries {
                coarse.push(probe.coarse(task, query));
                wide.push(probe.wide(task, query));
                sync.push(probe.sync(task, query));
            }
        }
        let clean = score_tasks(&tasks, &coarse, &wide, &sync).expect("clean scores");
        let flip_at = clean.total / 2;
        sync[flip_at] = !sync[flip_at];
        let lied = score_tasks(&tasks, &coarse, &wide, &sync).expect("lied scores");
        assert_eq!(lied.sync_agree + 1, clean.sync_agree);
        assert_eq!(lied.failed_sync.len(), clean.failed_sync.len() + 1);
    }

    #[test]
    fn held_pegs_stay_blocked_in_gold_only() {
        let tasks = generate_tasks();
        let mut held_total: usize = 0;
        for task in &tasks {
            for query in &task.queries {
                if query.held {
                    held_total += 1;
                }
            }
        }
        assert_eq!(held_total, TOTAL_QUERIES - EXPECTED_SYNC);
        for task in &tasks {
            for query in &task.queries {
                if query.held {
                    assert!(!gold_answer(task, query));
                    assert!(MirrorProbe.sync(task, query));
                }
            }
        }
    }

    #[test]
    fn wall_segment_fails_the_slab() {
        assert!(!slab_open(-300, 0, 300, 0, (-100, 100, -200, 200)));
        assert!(slab_open(-300, 0, -250, 0, (-100, 100, -200, 200)));
    }

    #[test]
    fn bad_lengths_and_names_refuse_typed() {
        let tasks = generate_tasks();
        let err = score_tasks(&tasks, &[], &[], &[]).expect_err("empty must fail");
        assert_eq!(err.code(), "answer_count");
        assert!(task_by_name(&tasks, "afford-001").is_ok());
        let err2 = task_by_name(&tasks, "afford-031").expect_err("unknown must fail");
        assert_eq!(err2.code(), "bad_task");
    }

    #[test]
    #[ignore = "timing: probe budget, run under --perf in release"]
    fn probe_budget_us_per_object() {
        let tasks = generate_tasks();
        let probe = MirrorProbe;
        let start = std::time::Instant::now();
        let mut hits = 0usize;
        for task in &tasks {
            for query in &task.queries {
                if probe.sync(task, query) || !probe.sync(task, query) {
                    hits += 1;
                }
            }
        }
        assert_eq!(hits, TOTAL_QUERIES);
        let total = u128::try_from(TOTAL_QUERIES).expect("180 probes fit u128");
        let per_us = start.elapsed().as_micros() / total;
        println!("afford: {per_us} us per object, hits={hits}");
        assert!(
            per_us <= BUDGET_US_PER_OBJECT,
            "probe budget 0.2 ms, measured {per_us} us"
        );
    }
}
