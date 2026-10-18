//! SPR4 real-model spatial A/B on cassettes (the missing SP20 number).
//!
//! Consumer: `cargo test -p engine-playtest --lib spatial_llm_replay` and the
//! operator who records a real model (see `docs/plan/spatial-llm-ab.md`).
//! Data owner: this module owns the 50 seeded questions, the prompts, the
//! cassette text format and the table; `world.rs` owns the geometry oracle
//! that makes the gold. Two arms answer the same questions: `raw` reads only
//! today's `scene.brief` text, `verb` may call `scene.query` / `scene.place`.
//! Determinism: questions come from one splitmix64 seed, prompts from the
//! brief text, keys from FNV-1a over (model, arm, id, prompt), so a replay
//! prints the same bytes twice. Failure mode: a changed question or brief
//! misses its cassette row with a typed [`LlmError::Miss`] (never reused),
//! and an empty answer scores wrong, so a do-nothing arm scores 0.
//!
//! The harness never calls a model. A live recorder drives the agent loop
//! (tool calls included), then writes one aggregate row per question and
//! arm: calls, tokens and the final answer. `echo` cassettes answer from
//! gold and only prove the plumbing; they carry no model figure.

use super::world::{overlaps, scene_of, seg_blocked, Rect, Scene, SCENES};
use crate::eval::mix_text;
use engine_core::Fnv64;
use engine_spatial::{brief, brief_text, build, estimate_tokens, run, Actor as SceneActor};
use std::collections::BTreeMap;
use std::fmt;

/// Seeded questions in the set.
pub const QUESTION_COUNT: usize = 50;
/// Questions per family: left_of, can_see, where_fits, why_failed.
pub const FAMILY_COUNTS: [usize; 4] = [15, 15, 10, 10];
/// Points the verb arm must win by for the verbs to earn their keep.
pub const WIN_POINTS: u32 = 20;
/// Seed of the question generator.
const SEED: u64 = 0x5eed_2040_a0b0;
/// Cassette file format tag (first line).
const FORMAT: &str = "# spatial-llm-cassette v1";
/// Largest cassette file read, in bytes.
pub const MAX_CASSETTE_BYTES: usize = 256 * 1024;
/// Couch footprint half extents in cm (200 x 90).
const COUCH: (i32, i32) = (100, 45);
/// Budget of the raw brief the raw arm reads, in bytes.
const BRIEF_BYTES: usize = 4096;

const SYSTEM: &str = "You answer one spatial question about a game scene. Reply with the answer only, no explanation. Coordinates are cm on a floor plan: +x is east, +z is north.";
const VERB_TOOLS: &str = "Tools: scene.query kind=relation|near|between|visible|reachable|cover_near|largest_open|free_space|cone; scene.place with wants and dry_run. Call them, then answer.";

/// Question family, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    LeftOf,
    CanSee,
    WhereFits,
    WhyFailed,
}

impl Family {
    const ALL: [Self; 4] = [Self::LeftOf, Self::CanSee, Self::WhereFits, Self::WhyFailed];

    /// Report column name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LeftOf => "left_of",
            Self::CanSee => "can_see",
            Self::WhereFits => "where_fits",
            Self::WhyFailed => "why_failed",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::LeftOf => 0,
            Self::CanSee => 1,
            Self::WhereFits => 2,
            Self::WhyFailed => 3,
        }
    }
}

/// Which information the answering model gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// Only the raw `scene.brief` text.
    Raw,
    /// The door tools (`scene.query`, `scene.place`).
    Verb,
}

impl Arm {
    /// Cassette and table name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Verb => "verb",
        }
    }
}

/// One seeded question with its gold answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Stable id `sq-001`..`sq-050`.
    pub id: String,
    /// Family.
    pub family: Family,
    /// Scene name from `world.rs`.
    pub scene: &'static str,
    /// Question text, convention included.
    pub text: String,
    /// Gold answer: `yes`/`no`, or ascending labels or letters, comma separated.
    pub gold: String,
}

/// Typed failure of cassette parse or replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// Cassette text is malformed.
    Parse(String),
    /// No usable row for this arm and question.
    Miss {
        arm: &'static str,
        id: String,
        why: &'static str,
    },
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(why) => write!(f, "bad spatial-llm cassette: {why}"),
            Self::Miss { arm, id, why } => write!(
                f,
                "cassette miss: arm {arm} question {id} is {why}; re-record this model"
            ),
        }
    }
}

impl std::error::Error for LlmError {}

// ---- questions ---------------------------------------------------------------

struct Rng(engine_core::Rng);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n.max(1)).unwrap_or(1);
        usize::try_from(self.0.next_u64() % n).unwrap_or(0)
    }

    /// Multiple of 10 in `lo..=hi`.
    fn tens(&mut self, lo: i32, hi: i32) -> i32 {
        let steps = usize::try_from((hi - lo) / 10 + 1).unwrap_or(1);
        lo + 10 * i32::try_from(self.below(steps)).unwrap_or(0)
    }
}

/// Door-style label: scene index order, like the `prop_NNNN` names.
fn label(scene: &Scene, id: u32) -> String {
    let at = scene.actors.iter().position(|a| a.id == id).unwrap_or(0);
    format!("prop_{at:04}")
}

fn labels(scene: &Scene, ids: &[u32]) -> String {
    ids.iter()
        .map(|id| label(scene, *id))
        .collect::<Vec<_>>()
        .join(",")
}

/// Crates: everything but the four boundary walls (ids 0..=3 in every scene).
fn crates_of(scene: &Scene) -> Vec<u32> {
    scene
        .actors
        .iter()
        .map(|a| a.id)
        .filter(|id| *id >= 10)
        .collect()
}

const COMPASS: [(i32, i32, &str); 8] = [
    (1, 0, "east"),
    (1, 1, "north-east"),
    (0, 1, "north"),
    (-1, 1, "north-west"),
    (-1, 0, "west"),
    (-1, -1, "south-west"),
    (0, -1, "south"),
    (1, -1, "south-east"),
];

/// Ids strictly on the left of `subject` facing `(fx, fz)`: positive cross
/// product with x east and z north (facing north, west is left).
fn left_of(scene: &Scene, subject: u32, face: (i32, i32)) -> Vec<u32> {
    let me = scene.actors.iter().find(|a| a.id == subject);
    let Some(me) = me else { return Vec::new() };
    crates_of(scene)
        .into_iter()
        .filter(|id| *id != subject)
        .filter(|id| {
            let o = scene.actors.iter().find(|a| a.id == *id);
            o.is_some_and(|o| face.0 * (o.z_cm - me.z_cm) - face.1 * (o.x_cm - me.x_cm) > 0)
        })
        .collect()
}

fn sees(scene: &Scene, a: u32, b: u32) -> bool {
    let at = |id: u32| scene.actors.iter().find(|x| x.id == id);
    let (Some(pa), Some(pb)) = (at(a), at(b)) else {
        return false;
    };
    let blockers: Vec<Rect> = scene
        .actors
        .iter()
        .filter(|x| x.id != a && x.id != b)
        .map(|x| x.rect())
        .collect();
    !seg_blocked((pa.x_cm, pa.z_cm), (pb.x_cm, pb.z_cm), &blockers)
}

fn couch(x: i32, z: i32) -> Rect {
    Rect {
        min_x: x - COUCH.0,
        max_x: x + COUCH.0,
        min_z: z - COUCH.1,
        max_z: z + COUCH.1,
    }
}

/// Ids of every actor (walls included) whose footprint the couch overlaps.
fn collides(scene: &Scene, x: i32, z: i32) -> Vec<u32> {
    let c = couch(x, z);
    scene
        .actors
        .iter()
        .filter(|a| overlaps(&c, &a.rect()))
        .map(|a| a.id)
        .collect()
}

fn pick_scene(rng: &mut Rng) -> &'static Scene {
    &SCENES[rng.below(SCENES.len())]
}

fn left_question(rng: &mut Rng) -> (String, &'static str, String) {
    loop {
        let scene = pick_scene(rng);
        let crates = crates_of(scene);
        let subject = crates[rng.below(crates.len())];
        let (fx, fz, dir) = COMPASS[rng.below(8)];
        let ids = left_of(scene, subject, (fx, fz));
        if ids.is_empty() || ids.len() + 1 >= crates.len() {
            continue;
        }
        let text = format!(
            "{} stands facing {dir}. Which crates are strictly on its left side? Crates are every object except the four boundary walls; an object straight ahead or behind counts as neither side. Answer with labels, comma separated.",
            label(scene, subject)
        );
        return (text, scene.name, labels(scene, &ids));
    }
}

fn see_question(rng: &mut Rng, want: bool) -> (String, &'static str, String) {
    loop {
        let scene = pick_scene(rng);
        let crates = crates_of(scene);
        let (a, b) = (
            crates[rng.below(crates.len())],
            crates[rng.below(crates.len())],
        );
        if a == b || sees(scene, a, b) != want {
            continue;
        }
        let text = format!(
            "Can {} see {}? A straight line between their centres must not pass through any other object or wall. Answer yes or no.",
            label(scene, a),
            label(scene, b)
        );
        return (text, scene.name, if want { "yes" } else { "no" }.to_owned());
    }
}

fn fit_question(rng: &mut Rng) -> (String, &'static str, String) {
    loop {
        let scene = pick_scene(rng);
        let spots: Vec<(i32, i32)> = (0..4)
            .map(|_| (rng.tens(-400, 400), rng.tens(-440, 440)))
            .collect();
        let fits: Vec<&str> = ["A", "B", "C", "D"]
            .into_iter()
            .zip(&spots)
            .filter(|(_, (x, z))| collides(scene, *x, *z).is_empty())
            .map(|(l, _)| l)
            .collect();
        if fits.is_empty() || fits.len() == 4 {
            continue;
        }
        let list = ["A", "B", "C", "D"]
            .iter()
            .zip(&spots)
            .map(|(l, (x, z))| format!("{l}=({x},{z})"))
            .collect::<Vec<_>>()
            .join(" ");
        let text = format!(
            "A couch is 200 cm along x and 90 cm along z. Which of these centre points can hold it without touching any object or wall? {list}. Answer with letters, comma separated."
        );
        return (text, scene.name, fits.join(","));
    }
}

fn why_question(rng: &mut Rng) -> (String, &'static str, String) {
    loop {
        let scene = pick_scene(rng);
        let (x, z) = (rng.tens(-400, 400), rng.tens(-440, 440));
        let hit = collides(scene, x, z);
        if hit.is_empty() || hit.len() > 2 {
            continue;
        }
        let text = format!(
            "A couch 200 cm along x and 90 cm along z, centred at ({x},{z}), was refused. Which objects or walls does it collide with? Answer with labels, comma separated."
        );
        return (text, scene.name, labels(scene, &hit));
    }
}

/// The 50 seeded questions, family blocks in report order.
pub fn questions() -> Vec<Question> {
    let mut rng = Rng(engine_core::Rng::new(SEED));
    let mut out = Vec::new();
    for family in Family::ALL {
        for n in 0..FAMILY_COUNTS[family.index()] {
            let (text, scene, gold) = match family {
                Family::LeftOf => left_question(&mut rng),
                Family::CanSee => see_question(&mut rng, n % 2 == 0),
                Family::WhereFits => fit_question(&mut rng),
                Family::WhyFailed => why_question(&mut rng),
            };
            out.push(Question {
                id: format!("sq-{:03}", out.len() + 1),
                family,
                scene,
                text,
                gold,
            });
        }
    }
    out
}

// ---- prompts and keys --------------------------------------------------------

/// Today's `scene.brief` text for a scene: door-style `prop_NNNN` names,
/// real footprints, the same `brief` and checks the door runs.
/// Scene centimeters (room sized, far under 32k) as meters.
fn meters(cm: i32) -> f32 {
    f32::from(i16::try_from(cm).unwrap_or(0)) / 100.0
}

fn raw_brief(scene: &Scene) -> String {
    let actors: Vec<SceneActor> = scene
        .actors
        .iter()
        .enumerate()
        .map(|(i, a)| SceneActor {
            id: a.id,
            name: format!("prop_{i:04}"),
            x: meters(a.x_cm),
            y: 0.5,
            z: meters(a.z_cm),
            sx: 2.0 * meters(a.hx_cm),
            sy: 1.0,
            sz: 2.0 * meters(a.hz_cm),
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        })
        .collect();
    brief_text(&brief(&actors, &build(&actors), &run(&actors), BRIEF_BYTES))
}

/// The full prompt one arm gets for one question.
pub fn prompt(arm: Arm, q: &Question) -> String {
    match (arm, scene_of(q.scene)) {
        (Arm::Raw, Some(scene)) => format!(
            "{SYSTEM}\nSCENE BRIEF\n{}QUESTION\n{}\n",
            raw_brief(scene),
            q.text
        ),
        _ => format!(
            "{SYSTEM}\n{VERB_TOOLS}\nSCENE {}\nQUESTION\n{}\n",
            q.scene, q.text
        ),
    }
}

fn key_of(model: &str, arm: Arm, q: &Question) -> u64 {
    let mut h = Fnv64::new();
    h.write(b"spatial-llm:1");
    for part in [model, arm.as_str(), q.id.as_str(), prompt(arm, q).as_str()] {
        mix_text(&mut h, part);
    }
    h.finish()
}

// ---- model and cassette ------------------------------------------------------

/// What one model run produced for one question and arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// Tool calls made (the final answer is not a call).
    pub calls: u32,
    /// Prompt tokens, tool results included.
    pub tokens_in: u32,
    /// Completion tokens.
    pub tokens_out: u32,
    /// Final answer text.
    pub answer: String,
}

/// An answering agent. A live recorder implements this over a provider.
pub trait Model {
    /// `echo` for harness stand-ins, `model` for a real recording.
    fn source(&self) -> &'static str;
    /// Answer one question under one arm.
    fn ask(&self, arm: Arm, q: &Question) -> Reply;
}

/// Answers from gold in both arms: proves cassette and scoring plumbing,
/// says nothing about any model.
pub struct EchoGold;

impl Model for EchoGold {
    fn source(&self) -> &'static str {
        "echo"
    }

    fn ask(&self, arm: Arm, q: &Question) -> Reply {
        let out = estimate_tokens(&q.gold);
        let tool = if arm == Arm::Verb { out } else { 0 };
        Reply {
            calls: u32::from(arm == Arm::Verb),
            tokens_in: estimate_tokens(&prompt(arm, q)) + tool,
            tokens_out: out,
            answer: q.gold.clone(),
        }
    }
}

/// Never answers: must score 0 in both arms.
pub struct DoNothing;

impl Model for DoNothing {
    fn source(&self) -> &'static str {
        "echo"
    }

    fn ask(&self, _arm: Arm, _q: &Question) -> Reply {
        Reply {
            calls: 0,
            tokens_in: 0,
            tokens_out: 0,
            answer: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    key: u64,
    reply: Reply,
}

/// Recorded answers of one model, keyed by arm and question id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cassette {
    /// Model slug.
    pub model: String,
    /// `echo` or `model`.
    pub source: String,
    rows: BTreeMap<(String, String), Row>,
}

/// Record `agent` over all questions and both arms.
pub fn record(model: &str, agent: &dyn Model, qs: &[Question]) -> Cassette {
    let mut rows = BTreeMap::new();
    for arm in [Arm::Raw, Arm::Verb] {
        for q in qs {
            let mut reply = agent.ask(arm, q);
            reply.answer = reply.answer.replace(['\t', '\n', '\r'], " ");
            let key = key_of(model, arm, q);
            rows.insert((arm.as_str().to_owned(), q.id.clone()), Row { key, reply });
        }
    }
    Cassette {
        model: model.to_owned(),
        source: agent.source().to_owned(),
        rows,
    }
}

impl Cassette {
    /// Canonical text: header, then one tab-separated row per arm and question.
    pub fn to_text(&self) -> String {
        let mut out = format!("{FORMAT}\nmodel\t{}\nsource\t{}\n", self.model, self.source);
        out.push_str("arm\tid\tkey\tcalls\ttok_in\ttok_out\tanswer\n");
        for ((arm, id), row) in &self.rows {
            let r = &row.reply;
            out.push_str(&format!(
                "{arm}\t{id}\t{:016x}\t{}\t{}\t{}\t{}\n",
                row.key, r.calls, r.tokens_in, r.tokens_out, r.answer
            ));
        }
        out
    }

    /// Parse [`Self::to_text`] output; anything else is a typed error.
    pub fn from_text(text: &str) -> Result<Self, LlmError> {
        let bad = |why: &str| LlmError::Parse(why.to_owned());
        if text.len() > MAX_CASSETTE_BYTES {
            return Err(bad("file over the size cap"));
        }
        let mut lines = text.lines();
        if lines.next() != Some(FORMAT) {
            return Err(bad("first line is not the v1 format tag"));
        }
        let mut head = |name: &str| {
            lines
                .next()
                .and_then(|l| l.strip_prefix(name))
                .and_then(|l| l.strip_prefix('\t'))
                .map(str::to_owned)
        };
        let model = head("model").ok_or_else(|| bad("missing model line"))?;
        let source = head("source").ok_or_else(|| bad("missing source line"))?;
        if lines.next() != Some("arm\tid\tkey\tcalls\ttok_in\ttok_out\tanswer") {
            return Err(bad("missing column header"));
        }
        let mut rows = BTreeMap::new();
        for line in lines {
            let f: Vec<&str> = line.splitn(7, '\t').collect();
            let [arm, id, key, calls, tin, tout, answer] = f[..] else {
                return Err(bad("row needs 7 tab-separated fields"));
            };
            let num = |s: &str| {
                s.parse::<u32>()
                    .map_err(|_| bad("row number is not a count"))
            };
            let row = Row {
                key: u64::from_str_radix(key, 16).map_err(|_| bad("row key is not hex"))?,
                reply: Reply {
                    calls: num(calls)?,
                    tokens_in: num(tin)?,
                    tokens_out: num(tout)?,
                    answer: answer.to_owned(),
                },
            };
            if rows.insert((arm.to_owned(), id.to_owned()), row).is_some() {
                return Err(bad("duplicate arm and question row"));
            }
        }
        Ok(Self {
            model,
            source,
            rows,
        })
    }
}

// ---- replay and report -------------------------------------------------------

/// Sorted, spaceless, lowercase answer set: order and spacing never matter.
fn canon(answer: &str) -> String {
    let flat: String = answer
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let mut parts: Vec<&str> = flat
        .split(',')
        .filter(|p| !p.is_empty() && *p != "none")
        .collect();
    parts.sort_unstable();
    parts.dedup();
    parts.join(",")
}

/// Exact match; an empty answer is never right.
fn is_right(answer: &str, gold: &str) -> bool {
    !canon(answer).is_empty() && canon(answer) == canon(gold)
}

/// One arm's score.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArmScore {
    /// Exact matches.
    pub exact: u32,
    /// Exact matches per family.
    pub by_family: [u32; 4],
    /// Tool calls over all questions.
    pub calls: u32,
    /// Estimated tokens in and out over all questions.
    pub tokens: u32,
}

/// Both arms of one cassette replayed over the questions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    /// Model slug.
    pub model: String,
    /// `echo` or `model`.
    pub source: String,
    /// Questions asked per arm.
    pub total: u32,
    /// Questions per family.
    pub family_totals: [u32; 4],
    /// Raw-brief arm.
    pub raw: ArmScore,
    /// Door-verb arm.
    pub verb: ArmScore,
}

/// Score a cassette offline. A missing or stale row refuses the replay.
pub fn replay(c: &Cassette, qs: &[Question]) -> Result<Replay, LlmError> {
    let mut out = Replay {
        model: c.model.clone(),
        source: c.source.clone(),
        total: u32::try_from(qs.len()).unwrap_or(u32::MAX),
        family_totals: [0; 4],
        raw: ArmScore::default(),
        verb: ArmScore::default(),
    };
    for q in qs {
        out.family_totals[q.family.index()] += 1;
    }
    for arm in [Arm::Raw, Arm::Verb] {
        let mut score = ArmScore::default();
        for q in qs {
            let miss = |why| LlmError::Miss {
                arm: arm.as_str(),
                id: q.id.clone(),
                why,
            };
            let row = c
                .rows
                .get(&(arm.as_str().to_owned(), q.id.clone()))
                .ok_or_else(|| miss("absent"))?;
            if row.key != key_of(&c.model, arm, q) {
                return Err(miss("stale (question or brief changed)"));
            }
            if is_right(&row.reply.answer, &q.gold) {
                score.exact += 1;
                score.by_family[q.family.index()] += 1;
            }
            score.calls += row.reply.calls;
            score.tokens += row.reply.tokens_in + row.reply.tokens_out;
        }
        match arm {
            Arm::Raw => out.raw = score,
            Arm::Verb => out.verb = score,
        }
    }
    Ok(out)
}

impl Replay {
    fn pct(&self, s: &ArmScore) -> u32 {
        s.exact * 100 / self.total.max(1)
    }

    /// The decision line: no figure from an echo, kill or keep from a model.
    pub fn verdict(&self) -> String {
        if self.source != "model" {
            return "NO MODEL FIGURE: echo cassette proves plumbing only; record a real model"
                .to_owned();
        }
        let (raw, verb) = (self.pct(&self.raw), self.pct(&self.verb));
        if verb >= raw + WIN_POINTS {
            format!(
                "KEEP: verbs win by {} points (need {WIN_POINTS})",
                verb - raw
            )
        } else {
            format!(
                "KILL: verbs win by {} points, under {WIN_POINTS}; stop adding verbs on this evidence",
                verb.saturating_sub(raw)
            )
        }
    }

    /// Train-or-not note from the verb arm score.
    pub fn train_note(&self) -> &'static str {
        if self.source != "model" {
            "train: unknown until a real model is recorded"
        } else if self.pct(&self.verb) >= 90 {
            "train: no, the verb arm already answers 90 percent or more"
        } else {
            "train: not yet; read the misses first (wrong ids = name grounding, SP67; wrong yes/no = verb or schema text)"
        }
    }

    /// One table, byte-stable.
    pub fn to_table(&self) -> String {
        let mut out = format!(
            "spatial-llm replay: model={} source={} questions={}\n",
            self.model, self.source, self.total
        );
        out.push_str("arm   exact   pct");
        for f in Family::ALL {
            out.push_str(&format!("  {}", f.as_str()));
        }
        out.push_str("  calls  tokens\n");
        for (name, s) in [("raw", &self.raw), ("verb", &self.verb)] {
            out.push_str(&format!(
                "{name:<5} {:>2}/{:<3} {:>3}",
                s.exact,
                self.total,
                self.pct(s)
            ));
            for f in Family::ALL {
                let w = f.as_str().len();
                let cell = format!(
                    "{}/{}",
                    s.by_family[f.index()],
                    self.family_totals[f.index()]
                );
                out.push_str(&format!("  {cell:>w$}"));
            }
            out.push_str(&format!("  {:>5}  {:>6}\n", s.calls, s.tokens));
        }
        out.push_str(&format!(
            "verdict: {}\n{}\n",
            self.verdict(),
            self.train_note()
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo() -> Cassette {
        record("echo-gold", &EchoGold, &questions())
    }

    #[test]
    fn spatial_llm_questions_are_fifty_seeded_and_balanced() {
        let qs = questions();
        assert_eq!(qs.len(), QUESTION_COUNT);
        assert_eq!(qs, questions(), "same seed, same questions");
        for f in Family::ALL {
            let n = qs.iter().filter(|q| q.family == f).count();
            assert_eq!(n, FAMILY_COUNTS[f.index()], "{}", f.as_str());
        }
        let yes = qs
            .iter()
            .filter(|q| q.family == Family::CanSee && q.gold == "yes")
            .count();
        assert_eq!(yes, 8, "can_see is 8 yes and 7 no");
        assert!(qs.iter().all(|q| !canon(&q.gold).is_empty()));
    }

    #[test]
    fn spatial_llm_left_follows_the_floor_plan_convention() {
        // Pillar room: crate 21 at the origin, 20 west of it, 10 east of it.
        let scene = scene_of("room_pillar").expect("scene");
        let ids = left_of(scene, 21, (0, 1));
        assert!(
            ids.contains(&20) && !ids.contains(&10),
            "facing north, west is left: {ids:?}"
        );
        let ids = left_of(scene, 21, (0, -1));
        assert!(
            ids.contains(&10) && !ids.contains(&20),
            "facing south, east is left: {ids:?}"
        );
    }

    #[test]
    fn spatial_llm_replay_prints_one_table_x2() {
        let qs = questions();
        let first = replay(&echo(), &qs).expect("echo replays").to_table();
        let second = replay(&echo(), &qs).expect("echo replays").to_table();
        assert_eq!(first, second, "bytes x2");
        println!("{first}");
        assert!(first.contains("raw   50/50  100"), "{first}");
        assert!(
            first.contains("NO MODEL FIGURE"),
            "echo must not claim a number"
        );
    }

    #[test]
    fn spatial_llm_replay_do_nothing_scores_zero() {
        let qs = questions();
        let r = replay(&record("do-nothing", &DoNothing, &qs), &qs).expect("replays");
        assert_eq!((r.raw.exact, r.verb.exact), (0, 0));
    }

    #[test]
    fn spatial_llm_replay_text_round_trips_and_refuses_stale_rows() {
        let qs = questions();
        let c = echo();
        assert_eq!(Cassette::from_text(&c.to_text()), Ok(c.clone()));
        let mut changed = qs.clone();
        changed[3].text.push_str(" Think twice.");
        assert!(
            matches!(replay(&c, &changed), Err(LlmError::Miss { why, .. }) if why.starts_with("stale"))
        );
        let cut = c.to_text().lines().take(8).collect::<Vec<_>>().join("\n");
        let short = Cassette::from_text(&cut).expect("parses");
        assert!(matches!(
            replay(&short, &qs),
            Err(LlmError::Miss { why: "absent", .. })
        ));
        assert!(Cassette::from_text("junk").is_err());
    }

    #[test]
    fn spatial_llm_replay_verdict_kills_a_small_win() {
        let mut r = replay(&echo(), &questions()).expect("replays");
        r.source = "model".to_owned();
        r.raw.exact = 30;
        r.verb.exact = 39;
        assert!(r.verdict().starts_with("KILL"), "{}", r.verdict());
        r.verb.exact = 40;
        assert!(r.verdict().starts_with("KEEP"), "{}", r.verdict());
    }

    #[test]
    fn spatial_llm_replay_serves_every_shipped_cassette() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/spatial-gold/llm");
        let qs = questions();
        for entry in std::fs::read_dir(&dir).expect("llm fixture dir") {
            let path = entry.expect("entry").path();
            if path.extension().is_some_and(|e| e == "tsv") {
                let text = std::fs::read_to_string(&path).expect("read");
                let c = Cassette::from_text(&text).expect("parses");
                println!("{}", replay(&c, &qs).expect("replays").to_table());
            }
        }
    }
}
