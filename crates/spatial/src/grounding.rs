//! Grounding: fuzzy referring-expression grounding over the scene graph.
//!
//! A phrase like "the red shield left of the fire" is parsed into terms and
//! scored against every node of the graph. The answer is a ranked list of
//! candidates: the module reads actors and returns names, it never writes a
//! node and never mutates the scene.
//!
//! Owner: spatial crate. Consumer: the door asks for candidates and decides
//! what to do with them. Units: meters in, integer cm inside. Determinism:
//! candidates sort by score then id, every score is an integer, and the digest
//! is FNV-1a over the canonical bytes, so the same actors in any input order
//! give the same answer.
//!
//! Vocabulary: the class phrase reads a node by its own label words, by the
//! class tags the open-vocabulary lexicon lifts from that label, and by the
//! names of the groups the node sits in. Spatial terms read the support graph
//! and the relation graph. A containment term also accepts the group tree, so
//! "the chest in the armory" grounds when the armory is a group and not a box.

use std::collections::{BTreeMap, BTreeSet};

use engine_core::hash_bytes;

use super::fold;
use super::relation::{build, rel_name, subjects_of, Frame, Graph, Rel, Relation};
use super::tag_lift;
use super::Actor;

/// The smallest candidate list a grounding answer carries.
pub const MIN_TOP_K: usize = 3;

/// The candidate count a query keeps when the caller names none.
pub const DEFAULT_TOP_K: usize = 5;

/// The points a spatial term earns a candidate that holds it.
const SPATIAL_WEIGHT: i32 = 20_000;

/// The points the nearest candidate of an order term earns.
const ORDER_BASE: i32 = 10_000;

/// The points each step down the order ranking takes away.
const ORDER_STEP: i32 = 100;

/// The points an exact label word earns.
const MATCH_EXACT: i32 = 100;

/// The points a plural variant earns.
const MATCH_PLURAL: i32 = 95;

/// The points a class tag on the node earns.
const MATCH_TAG: i32 = 90;

/// The points a class tag shared with the word earns.
const MATCH_TAG_SHARED: i32 = 85;

/// The points a four-letter prefix earns.
const MATCH_PREFIX: i32 = 70;

/// The points one wrong letter earns.
const MATCH_TYPO: i32 = 60;

/// The points a group name earns.
const MATCH_GROUP: i32 = 50;

/// A spatial relation a term asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialRel {
    /// A directed relation of the relation graph.
    Directed(Rel),
    /// A contact or support edge of the support graph.
    Edge(Relation),
}

impl SpatialRel {
    /// The stable display name of the relation.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Directed(rel) => rel_name(rel),
            Self::Edge(rel) => rel.name(),
        }
    }
}

/// One parsed term of a referring expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// The class phrase: the head noun and the modifiers that narrow it.
    Class {
        /// The head noun: the word that names the kind of thing.
        head: String,
        /// The modifiers in front of the head, in expression order.
        modifiers: Vec<String>,
    },
    /// A spatial term: the relation and the anchor it is measured from.
    Spatial {
        /// The relation the candidate must hold of the anchor.
        rel: SpatialRel,
        /// The words that name the anchor.
        anchor: Vec<String>,
    },
    /// A containment term: the candidate sits inside the anchor, or inside
    /// the group the anchor names.
    Contain {
        /// The words that name the container.
        anchor: Vec<String>,
    },
    /// An order term: the candidate is the nearest or the farthest one.
    Order {
        /// True for nearest, false for farthest.
        nearest: bool,
        /// The words that name the anchor.
        anchor: Vec<String>,
    },
}

/// One grounding request: a referring expression and how many candidates to
/// keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundArgs {
    /// The referring expression, e.g. "the red shield left of the fire".
    pub expression: String,
    /// How many candidates to keep, never fewer than [`MIN_TOP_K`].
    pub top_k: usize,
    /// The viewer whose seat frames the directional terms; the world frame
    /// when there is none.
    pub viewer: Option<u32>,
}

impl GroundArgs {
    /// A request for `expression` with the default candidate count.
    #[must_use]
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            top_k: DEFAULT_TOP_K,
            viewer: None,
        }
    }

    /// Set the candidate count; the count never drops below [`MIN_TOP_K`].
    #[must_use]
    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.top_k = top_k.max(MIN_TOP_K);
        self
    }

    /// Set the viewer that frames the directional terms.
    #[must_use]
    pub fn with_viewer(mut self, viewer: u32) -> Self {
        self.viewer = Some(viewer);
        self
    }
}

/// One ranked candidate node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundHit {
    /// The actor id of the candidate.
    pub id: u32,
    /// The actor name, as the scene spells it.
    pub name: String,
    /// The integer score; the bigger the better.
    pub score: i32,
    /// True when the candidate holds every spatial and containment term.
    pub matched: bool,
    /// Why it ranks here, in short words.
    pub reason: String,
}

/// The whole answer: the parsed terms and the ranked candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grounding {
    /// The expression as it was asked.
    pub expression: String,
    /// The parsed terms, in expression order.
    pub terms: Vec<Term>,
    /// The ranked candidates, best first.
    pub hits: Vec<GroundHit>,
    /// FNV-1a over [`grounding_bytes`].
    pub digest: u64,
    /// Always 0: the grounding is advisory and applies nothing.
    pub auto_applied: u32,
}

/// The words a phrase drops: determiners and connectives.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "of", "to", "from", "is", "some", "any", "that", "this", "there", "and",
];

/// The heads that name no kind of thing: "the one behind the door".
const GENERIC_WORDS: &[&str] = &["one", "thing", "item", "object", "shape", "piece"];

/// What a cue phrase asks for.
#[derive(Debug, Clone, Copy)]
enum Cue {
    /// A directed relation of the relation graph.
    Directed(Rel),
    /// A contact or support edge of the support graph.
    Edge(Relation),
    /// Containment with an anchor.
    Contain,
    /// Nearest or farthest to an anchor.
    Order(bool),
}

/// The cue phrases, longest first so "in front of" wins over "in".
const CUES: &[(&[&str], Cue)] = &[
    (&["in", "front", "of"], Cue::Directed(Rel::Front)),
    (&["on", "top", "of"], Cue::Edge(Relation::OnTopOf)),
    (&["left", "of"], Cue::Directed(Rel::Left)),
    (&["right", "of"], Cue::Directed(Rel::Right)),
    (&["next", "to"], Cue::Edge(Relation::NextTo)),
    (&["farthest", "from"], Cue::Order(false)),
    (&["furthest", "from"], Cue::Order(false)),
    (&["nearest", "to"], Cue::Order(true)),
    (&["closest", "to"], Cue::Order(true)),
    // The bare order words: "the torch nearest the door" needs no "to".
    // They sit after the two-word forms so "nearest to" still wins.
    (&["farthest"], Cue::Order(false)),
    (&["furthest"], Cue::Order(false)),
    (&["nearest"], Cue::Order(true)),
    (&["closest"], Cue::Order(true)),
    (&["inside", "of"], Cue::Contain),
    (&["aligned", "with"], Cue::Directed(Rel::Aligned)),
    (&["far", "from"], Cue::Directed(Rel::Far)),
    (&["west", "of"], Cue::Directed(Rel::Left)),
    (&["east", "of"], Cue::Directed(Rel::Right)),
    (&["north", "of"], Cue::Directed(Rel::Front)),
    (&["south", "of"], Cue::Directed(Rel::Behind)),
    (&["in", "front"], Cue::Directed(Rel::Front)),
    (&["behind"], Cue::Directed(Rel::Behind)),
    (&["below"], Cue::Directed(Rel::Below)),
    (&["under"], Cue::Directed(Rel::Below)),
    (&["above"], Cue::Directed(Rel::Above)),
    (&["over"], Cue::Directed(Rel::Above)),
    (&["near"], Cue::Directed(Rel::Near)),
    (&["against"], Cue::Directed(Rel::Against)),
    (&["around"], Cue::Directed(Rel::Around)),
    (&["facing"], Cue::Edge(Relation::Facing)),
    (&["inside"], Cue::Contain),
    (&["in"], Cue::Contain),
    (&["on"], Cue::Edge(Relation::OnTopOf)),
    (&["with"], Cue::Edge(Relation::Supports)),
];

/// One word of a phrase and the class tags it names.
struct WordSpec {
    /// The word, lowercased.
    word: String,
    /// The class tags the lexicon lifts from the word.
    tags: Vec<&'static str>,
}

/// The class phrase of a phrase: the head noun and its modifiers.
struct ClassSpec {
    /// The head noun.
    head: WordSpec,
    /// The modifiers, in phrase order.
    modifiers: Vec<WordSpec>,
    /// True when the head names no kind of thing.
    generic: bool,
}

/// One term of a phrase with its anchor resolved.
enum Planned {
    /// The class phrase.
    Class(ClassSpec),
    /// A spatial term: the relation and the nodes that hold it.
    Spatial {
        /// The relation the candidate must hold of the anchor.
        rel: SpatialRel,
        /// The anchor node, when the anchor words name one.
        anchor: Option<u32>,
        /// The nodes that hold the relation of the anchor.
        holds: BTreeSet<u32>,
    },
    /// A containment term: the nodes the anchor contains.
    Contain {
        /// The anchor node, when the anchor words name one.
        anchor: Option<u32>,
        /// The nodes the anchor holds, by box or by group.
        holds: BTreeSet<u32>,
    },
    /// An order term: the anchor to measure the distance from.
    Order {
        /// True for nearest, false for farthest.
        nearest: bool,
        /// The anchor node, when the anchor words name one.
        anchor: Option<u32>,
    },
}

/// One candidate node while it is scored.
struct Scored {
    /// The index of the node in the scene.
    index: usize,
    /// The score so far.
    score: i32,
    /// True while every spatial and containment term holds.
    matched: bool,
    /// Why it ranks here.
    reason: String,
}

/// The searchable text of one node.
struct NodeText {
    /// The words of the node label.
    own: Vec<String>,
    /// The class tags the lexicon lifts from the label.
    tags: Vec<&'static str>,
    /// The words of the group names the node sits in.
    rooms: Vec<String>,
}

/// One node of the scene, with its searchable text.
struct Node {
    /// The actor id.
    id: u32,
    /// The actor name.
    name: String,
    /// The searchable text.
    text: NodeText,
    /// The group ids above the node, nearest first.
    ancestors: Vec<u32>,
}

/// The scene the grounding reads: id-sorted nodes with their text.
struct Scene {
    /// The nodes, id-sorted.
    nodes: Vec<Node>,
}

impl Scene {
    /// The scene of an actor snapshot, id-sorted.
    fn build(actors: &[Actor]) -> Scene {
        let mut sorted: Vec<&Actor> = actors.iter().collect();
        sorted.sort_by_key(|a| a.id);
        let by_id: BTreeMap<u32, &Actor> = sorted.iter().map(|a| (a.id, *a)).collect();
        let mut nodes = Vec::new();
        for actor in &sorted {
            let ancestors = ancestors_of(actor.id, &by_id);
            let mut rooms = Vec::new();
            for id in &ancestors {
                if let Some(parent) = by_id.get(id) {
                    rooms.extend(label_words(&parent.name));
                }
            }
            nodes.push(Node {
                id: actor.id,
                name: actor.name.clone(),
                text: NodeText {
                    own: label_words(&actor.name),
                    tags: tag_lift::lift_label(&actor.name),
                    rooms,
                },
                ancestors,
            });
        }
        Scene { nodes }
    }

    /// The name of a node, empty when the id names none.
    fn name_of(&self, id: u32) -> &str {
        self.nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.name.as_str())
            .unwrap_or("")
    }
}

/// Parse a referring expression into terms.
///
/// The class phrase is the words in front of the first cue; every cue opens a
/// term whose anchor phrase runs to the next cue.
#[must_use]
pub fn parse_expression(expression: &str) -> Vec<Term> {
    let words: Vec<String> = expression
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let mut terms: Vec<Term> = Vec::new();
    let mut class: Vec<String> = Vec::new();
    let mut at = 0;
    while at < words.len() {
        match cue_at(&words, at) {
            Some((len, cue)) => {
                let mut anchor: Vec<String> = Vec::new();
                let mut next = at + len;
                while next < words.len() && cue_at(&words, next).is_none() {
                    anchor.push(words[next].clone());
                    next += 1;
                }
                let anchor = drop_stop_words(anchor);
                match cue {
                    Cue::Directed(rel) => {
                        terms.push(Term::Spatial {
                            rel: SpatialRel::Directed(rel),
                            anchor,
                        });
                    }
                    Cue::Edge(rel) => {
                        terms.push(Term::Spatial {
                            rel: SpatialRel::Edge(rel),
                            anchor,
                        });
                    }
                    Cue::Contain => terms.push(Term::Contain { anchor }),
                    Cue::Order(nearest) => terms.push(Term::Order { nearest, anchor }),
                }
                at = next;
            }
            None => {
                class.push(words[at].clone());
                at += 1;
            }
        }
    }
    let class = drop_stop_words(class);
    if !class.is_empty() {
        let head = class[class.len() - 1].clone();
        let modifiers = class[..class.len() - 1].to_vec();
        terms.insert(0, Term::Class { head, modifiers });
    }
    terms
}

/// Rank the nodes of the scene for one referring expression.
///
/// The answer is advisory: the scene is read, never written, and
/// [`Grounding::auto_applied`] stays 0.
#[must_use]
pub fn ground(actors: &[Actor], args: &GroundArgs) -> Grounding {
    let terms = parse_expression(&args.expression);
    let scene = Scene::build(actors);
    let frame = match args.viewer {
        Some(viewer) => Frame::Viewer(viewer),
        None => Frame::World,
    };
    let plan = plan_terms(actors, &scene, &terms, frame);
    let hits = rank(actors, &scene, &plan, args.top_k.max(MIN_TOP_K));
    let mut answer = Grounding {
        expression: args.expression.clone(),
        terms,
        hits,
        digest: 0,
        auto_applied: 0,
    };
    answer.digest = hash_bytes(&grounding_bytes(&answer));
    answer
}

/// The canonical bytes of an answer: the digest covers these.
///
/// Lengths and counts are little-endian; no float reaches these bytes.
#[must_use]
pub fn grounding_bytes(answer: &Grounding) -> Vec<u8> {
    let mut out = Vec::new();
    write_str(&mut out, &answer.expression);
    out.extend_from_slice(&as_u32(answer.terms.len()).to_le_bytes());
    for term in &answer.terms {
        term_bytes(term, &mut out);
    }
    out.extend_from_slice(&as_u32(answer.hits.len()).to_le_bytes());
    for hit in &answer.hits {
        out.extend_from_slice(&hit.id.to_le_bytes());
        out.extend_from_slice(&hit.score.to_le_bytes());
        out.push(u8::from(hit.matched));
        write_str(&mut out, &hit.name);
    }
    out.extend_from_slice(&answer.auto_applied.to_le_bytes());
    out
}

/// The planned terms: every anchor resolved and every relation read.
fn plan_terms(actors: &[Actor], scene: &Scene, terms: &[Term], frame: Frame) -> Vec<Planned> {
    let graph = support_graph(actors, terms);
    let mut out = Vec::new();
    for term in terms {
        match term {
            Term::Class { head, modifiers } => {
                out.push(Planned::Class(class_spec(head, modifiers)));
            }
            Term::Spatial { rel, anchor } => {
                let anchor_id = resolve_anchor(scene, anchor);
                let holds = match anchor_id {
                    Some(id) => holders(actors, graph.as_ref(), *rel, id, frame),
                    None => BTreeSet::new(),
                };
                out.push(Planned::Spatial {
                    rel: *rel,
                    anchor: anchor_id,
                    holds,
                });
            }
            Term::Contain { anchor } => {
                let anchor_id = resolve_anchor(scene, anchor);
                let holds = match anchor_id {
                    Some(id) => contained(graph.as_ref(), scene, id),
                    None => BTreeSet::new(),
                };
                out.push(Planned::Contain {
                    anchor: anchor_id,
                    holds,
                });
            }
            Term::Order { nearest, anchor } => {
                let anchor_id = resolve_anchor(scene, anchor);
                out.push(Planned::Order {
                    nearest: *nearest,
                    anchor: anchor_id,
                });
            }
        }
    }
    out
}

/// The support graph, built only when a term reads an edge of it.
fn support_graph(actors: &[Actor], terms: &[Term]) -> Option<Graph> {
    let wanted = terms.iter().any(|term| match term {
        Term::Spatial { rel, .. } => matches!(rel, SpatialRel::Edge(_)),
        Term::Contain { .. } => true,
        _ => false,
    });
    if wanted {
        Some(build(actors))
    } else {
        None
    }
}

/// The nodes that hold `rel` of the anchor.
fn holders(
    actors: &[Actor],
    graph: Option<&Graph>,
    rel: SpatialRel,
    anchor: u32,
    frame: Frame,
) -> BTreeSet<u32> {
    match rel {
        SpatialRel::Directed(kind) => subjects_of(actors, kind, anchor, frame)
            .into_iter()
            .collect(),
        SpatialRel::Edge(kind) => {
            let mut out = BTreeSet::new();
            if let Some(graph) = graph {
                for &(a, edge, b) in &graph.edges {
                    if edge != kind {
                        continue;
                    }
                    if b == anchor {
                        out.insert(a);
                    }
                    if kind == Relation::NextTo && a == anchor {
                        out.insert(b);
                    }
                }
            }
            out
        }
    }
}

/// The nodes the anchor holds: by box and by group.
fn contained(graph: Option<&Graph>, scene: &Scene, anchor: u32) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    if let Some(graph) = graph {
        for &(a, edge, b) in &graph.edges {
            if edge == Relation::Inside && b == anchor {
                out.insert(a);
            }
        }
    }
    for node in &scene.nodes {
        if node.ancestors.contains(&anchor) {
            out.insert(node.id);
        }
    }
    out
}

/// The class phrase of a term.
fn class_spec(head: &str, modifiers: &[String]) -> ClassSpec {
    let head = word_spec(head);
    let generic = GENERIC_WORDS.contains(&head.word.as_str());
    ClassSpec {
        head,
        modifiers: modifiers.iter().map(|word| word_spec(word)).collect(),
        generic,
    }
}

/// The node an anchor phrase names: the best text match, ties by id.
fn resolve_anchor(scene: &Scene, phrase: &[String]) -> Option<u32> {
    let head = phrase.last()?;
    let head = word_spec(head);
    let modifiers: Vec<WordSpec> = phrase[..phrase.len() - 1]
        .iter()
        .map(|w| word_spec(w))
        .collect();
    let mut best: Option<(i32, u32)> = None;
    for node in &scene.nodes {
        let points = word_points(&head, &node.text);
        if points == 0 {
            continue;
        }
        let mut total = points;
        for modifier in &modifiers {
            total += word_points(modifier, &node.text);
        }
        let better = match best {
            None => true,
            Some((top, _)) => total > top,
        };
        if better {
            best = Some((total, node.id));
        }
    }
    best.map(|(_, id)| id)
}

/// The candidates of a plan, ranked.
fn rank(actors: &[Actor], scene: &Scene, plan: &[Planned], top_k: usize) -> Vec<GroundHit> {
    let class = plan.iter().find_map(|term| match term {
        Planned::Class(spec) => Some(spec),
        _ => None,
    });
    // The most specific reading of the head word wins: when a node spells
    // the head, only those nodes stay and the looser readings drop away.
    let mut best = 0;
    if let Some(spec) = class {
        if !spec.generic {
            for node in &scene.nodes {
                let points = word_points(&spec.head, &node.text);
                if points > best {
                    best = points;
                }
            }
        }
    }
    if class.is_some() && best == 0 {
        return Vec::new();
    }
    let mut scored: Vec<Scored> = Vec::new();
    for (index, node) in scene.nodes.iter().enumerate() {
        if let Some(spec) = class {
            if !spec.generic && word_points(&spec.head, &node.text) != best {
                continue;
            }
        }
        let (mut score, mut parts) = text_score(class, node);
        let mut matched = true;
        for term in plan {
            match term {
                Planned::Class(_) => {}
                Planned::Spatial { rel, anchor, holds } => {
                    if let Some(id) = anchor {
                        if holds.contains(&node.id) {
                            score += SPATIAL_WEIGHT;
                            parts.push(format!("{}:{}", rel.name(), scene.name_of(*id)));
                        } else {
                            matched = false;
                        }
                    }
                }
                Planned::Contain { anchor, holds } => {
                    if let Some(id) = anchor {
                        if holds.contains(&node.id) {
                            score += SPATIAL_WEIGHT;
                            parts.push(format!("in:{}", scene.name_of(*id)));
                        } else {
                            matched = false;
                        }
                    }
                }
                Planned::Order { .. } => {}
            }
        }
        scored.push(Scored {
            index,
            score,
            matched,
            reason: parts.join(" "),
        });
    }
    apply_order(actors, scene, plan, &mut scored);
    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| scene.nodes[a.index].id.cmp(&scene.nodes[b.index].id))
    });
    scored.truncate(top_k);
    scored
        .into_iter()
        .map(|row| GroundHit {
            id: scene.nodes[row.index].id,
            name: scene.nodes[row.index].name.clone(),
            score: row.score,
            matched: row.matched,
            reason: row.reason,
        })
        .collect()
}

/// True when the node is a candidate of the class phrase.
///
/// The text points of a candidate against the class phrase.
fn text_score(class: Option<&ClassSpec>, node: &Node) -> (i32, Vec<String>) {
    let mut score = 0;
    let mut parts = Vec::new();
    if let Some(spec) = class {
        let head = word_points(&spec.head, &node.text);
        score += head;
        parts.push(format!("text {}:{}", spec.head.word, head));
        for modifier in &spec.modifiers {
            let points = word_points(modifier, &node.text);
            score += points;
            if points > 0 {
                parts.push(format!("{}:{}", modifier.word, points));
            }
        }
    }
    (score, parts)
}

/// The order terms rank the candidates by their distance to the anchor.
fn apply_order(actors: &[Actor], scene: &Scene, plan: &[Planned], scored: &mut [Scored]) {
    for term in plan {
        let Planned::Order {
            nearest,
            anchor: Some(anchor),
        } = term
        else {
            continue;
        };
        let mut rows: Vec<(i64, u32, usize)> = Vec::new();
        for (at, row) in scored.iter().enumerate() {
            let id = scene.nodes[row.index].id;
            if id == *anchor {
                continue;
            }
            let distance = super::relation::center_dist2_cm2(actors, id, *anchor);
            rows.push((distance.unwrap_or(i64::MAX), id, at));
        }
        if *nearest {
            rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        } else {
            rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        }
        for (rank, row) in rows.iter().enumerate() {
            let step = i32::try_from(rank).unwrap_or(i32::MAX);
            scored[row.2].score += (ORDER_BASE - ORDER_STEP * step).max(0);
        }
    }
}

/// One word of a phrase, with the class tags it names.
fn word_spec(word: &str) -> WordSpec {
    WordSpec {
        word: word.to_owned(),
        tags: tag_lift::lift_label(word),
    }
}

/// The points one word earns against one node.
fn word_points(spec: &WordSpec, text: &NodeText) -> i32 {
    let mut best = 0;
    for token in &text.own {
        best = best.max(token_points(&spec.word, token));
    }
    for tag in &text.tags {
        if *tag == spec.word {
            best = best.max(MATCH_TAG);
        }
        if spec.tags.contains(tag) {
            best = best.max(MATCH_TAG_SHARED);
        }
    }
    for token in &text.rooms {
        if *token == spec.word {
            best = best.max(MATCH_GROUP);
        }
    }
    best
}

/// The points one word earns against one label token.
fn token_points(word: &str, token: &str) -> i32 {
    if word == token {
        return MATCH_EXACT;
    }
    if singular(word) == singular(token) {
        return MATCH_PLURAL;
    }
    if word.len() >= 4 && token.len() >= 4 && (word.starts_with(token) || token.starts_with(word)) {
        return MATCH_PREFIX;
    }
    if one_edit(word, token) {
        return MATCH_TYPO;
    }
    0
}

/// The word without a trailing plural `s`.
fn singular(word: &str) -> &str {
    if word.len() > 3 && word.ends_with('s') {
        &word[..word.len() - 1]
    } else {
        word
    }
}

/// True when two words differ by at most one letter.
fn one_edit(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        if a.len().abs_diff(b.len()) != 1 {
            return false;
        }
        let (long, short) = if a.len() > b.len() { (a, b) } else { (b, a) };
        let mut i = 0;
        let mut j = 0;
        let mut skipped = false;
        while i < long.len() && j < short.len() {
            if long[i] == short[j] {
                i += 1;
                j += 1;
                continue;
            }
            if skipped {
                return false;
            }
            skipped = true;
            i += 1;
        }
        return true;
    }
    let mut diff = 0;
    for i in 0..a.len() {
        if a[i] != b[i] {
            diff += 1;
            if diff > 1 {
                return false;
            }
        }
    }
    true
}

/// The words of a label: the name stem and the raw name, split on
/// non-alphanumerics, without the counters.
fn label_words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for source in [fold::stem(name), name] {
        for word in source.split(|c: char| !c.is_ascii_alphanumeric()) {
            if word.is_empty() || word.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let lower = word.to_ascii_lowercase();
            if !out.contains(&lower) {
                out.push(lower);
            }
        }
    }
    out
}

/// The group ids above a node, nearest first.
fn ancestors_of(id: u32, by_id: &BTreeMap<u32, &Actor>) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let mut current = by_id.get(&id).and_then(|actor| actor.parent);
    while let Some(parent) = current {
        if out.contains(&parent) {
            break;
        }
        out.push(parent);
        current = by_id.get(&parent).and_then(|actor| actor.parent);
    }
    out
}

/// The cue phrase at a word, if any.
fn cue_at(words: &[String], at: usize) -> Option<(usize, Cue)> {
    for (phrase, cue) in CUES {
        if at + phrase.len() > words.len() {
            continue;
        }
        if words[at..at + phrase.len()]
            .iter()
            .zip(phrase.iter())
            .all(|(word, part)| word == part)
        {
            return Some((phrase.len(), *cue));
        }
    }
    None
}

/// The words of a phrase without the stop words.
fn drop_stop_words(words: Vec<String>) -> Vec<String> {
    words
        .into_iter()
        .filter(|word| !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

/// The bytes of one term.
fn term_bytes(term: &Term, out: &mut Vec<u8>) {
    match term {
        Term::Class { head, modifiers } => {
            out.push(0);
            write_str(out, head);
            out.extend_from_slice(&as_u32(modifiers.len()).to_le_bytes());
            for modifier in modifiers {
                write_str(out, modifier);
            }
        }
        Term::Spatial { rel, anchor } => {
            out.push(1);
            out.push(match rel {
                SpatialRel::Directed(_) => 0,
                SpatialRel::Edge(_) => 1,
            });
            write_str(out, rel.name());
            out.extend_from_slice(&as_u32(anchor.len()).to_le_bytes());
            for word in anchor {
                write_str(out, word);
            }
        }
        Term::Contain { anchor } => {
            out.push(2);
            out.extend_from_slice(&as_u32(anchor.len()).to_le_bytes());
            for word in anchor {
                write_str(out, word);
            }
        }
        Term::Order { nearest, anchor } => {
            out.push(3);
            out.push(u8::from(*nearest));
            out.extend_from_slice(&as_u32(anchor.len()).to_le_bytes());
            for word in anchor {
                write_str(out, word);
            }
        }
    }
}

/// One length-prefixed string.
fn write_str(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&as_u32(value.len()).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

/// Lossless usize -> u32, saturating only past the u32 range (never here).
fn as_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relation::{center_dist2_cm2, holds};

    /// The yard the corpus talks about: props that share class words, so the
    /// spatial term has to pick the referent.
    ///
    /// Meters in, the crate turns them into integer cm. North is -Z, east is
    /// +X, the world frame puts front north and right east.
    fn yard() -> Vec<Actor> {
        let mut out: Vec<Actor> = Vec::new();
        let mut add = |id: u32,
                       name: &str,
                       x: f32,
                       y: f32,
                       z: f32,
                       sx: f32,
                       sy: f32,
                       sz: f32,
                       yaw: f32,
                       parent: Option<u32>| {
            out.push(Actor {
                id,
                name: name.to_owned(),
                x,
                y,
                z,
                sx,
                sy,
                sz,
                parent,
                physical: false,
                vx: 0.0,
                vz: 0.0,
                yaw,
            });
        };
        add(1, "brazier_fire", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0, None);
        add(2, "red_shield", -3.0, 0.5, 0.0, 0.6, 1.0, 0.3, 0.0, None);
        add(3, "blue_shield", 3.0, 0.5, 0.0, 0.6, 1.0, 0.3, 0.0, None);
        add(4, "torch_north", 0.0, 0.5, -4.0, 0.4, 1.0, 0.4, 0.0, None);
        add(5, "torch_south", 0.0, 0.5, 4.0, 0.4, 1.0, 0.4, 0.0, None);
        add(6, "wooden_door", 0.0, 1.0, 8.0, 1.2, 2.0, 0.2, 0.0, None);
        add(7, "lantern_oil", 1.5, 0.5, 8.5, 0.4, 1.0, 0.4, 0.0, None);
        add(8, "iron_banner", 0.0, 2.4, 8.0, 1.0, 0.3, 0.1, 0.0, None);
        add(9, "stone_column", -8.0, 0.5, -6.0, 0.8, 1.0, 0.8, 0.0, None);
        add(10, "oak_barrel", -7.0, 0.5, -6.0, 1.2, 1.0, 0.8, 0.0, None);
        add(
            11,
            "bronze_statue",
            10.0,
            0.5,
            0.0,
            0.6,
            1.0,
            0.6,
            0.0,
            None,
        );
        add(12, "iron_keg", 11.2, 0.5, 0.0, 0.6, 1.0, 0.6, 0.0, None);
        add(
            13,
            "pillar_sandstone",
            0.0,
            0.5,
            -8.0,
            1.0,
            1.0,
            1.0,
            0.0,
            None,
        );
        add(
            14,
            "pillar_granite",
            0.2,
            0.5,
            -8.0,
            1.0,
            1.0,
            1.0,
            0.0,
            None,
        );
        add(15, "oak_table", 0.0, 0.5, -12.0, 2.0, 1.0, 1.0, 0.0, None);
        add(16, "oak_crate", 0.0, 1.3, -12.0, 0.8, 0.6, 0.8, 0.0, None);
        add(
            17,
            "iron_chest",
            -2.0,
            0.5,
            8.0,
            1.2,
            1.0,
            0.8,
            0.0,
            Some(22),
        );
        add(
            18,
            "clay_amphora",
            -2.0,
            0.3,
            8.0,
            0.3,
            0.4,
            0.3,
            0.0,
            Some(17),
        );
        add(19, "stone_altar", 6.0, 0.5, 2.0, 1.0, 1.0, 1.0, 0.0, None);
        add(
            20,
            "guard_captain",
            8.0,
            0.9,
            2.0,
            0.6,
            1.8,
            0.4,
            std::f32::consts::PI,
            None,
        );
        add(21, "iron_spear", 8.0, 2.0, 2.0, 0.1, 0.4, 0.1, 0.0, None);
        add(22, "armory", -8.0, 1.0, 8.0, 2.0, 2.0, 0.6, 0.0, None);
        add(
            23,
            "pillar_cracked",
            12.5,
            0.5,
            0.0,
            1.0,
            1.0,
            1.0,
            0.0,
            None,
        );
        add(24, "candle_stand", 0.0, 1.5, 8.0, 0.4, 0.4, 0.4, 0.0, None);
        out
    }

    /// The seeded corpus: a phrase and the node it names.
    const CORPUS: [(&str, u32); 60] = [
        ("the shield left of the brazier", 2),
        ("the shield right of the brazier", 3),
        ("the shield west of the brazier", 2),
        ("the shield east of the brazier", 3),
        ("the torch in front of the brazier", 4),
        ("the torch behind the brazier", 5),
        ("the torch north of the brazier", 4),
        ("the torch south of the brazier", 5),
        ("the light in front of the brazier", 4),
        ("the light next to the door", 7),
        ("the altar left of the guard", 19),
        ("the guard right of the altar", 20),
        ("the chest left of the door", 17),
        ("the door right of the chest", 6),
        ("the statue right of the brazier", 11),
        ("the statue left of the keg", 11),
        ("the table north of the brazier", 15),
        ("the brazier south of the table", 1),
        ("the lantern right of the door", 7),
        ("the door left of the lantern", 6),
        ("the crate on the table", 16),
        ("the table with the crate", 15),
        ("the amphora in the chest", 18),
        ("the chest in the armory", 17),
        ("the amphora in the armory", 18),
        ("the column against the barrel", 9),
        ("the barrel against the column", 10),
        ("the column next to the barrel", 9),
        ("the barrel next to the column", 10),
        ("the statue next to the keg", 11),
        ("the keg next to the statue", 12),
        ("the lantern next to the door", 7),
        ("the door next to the lantern", 6),
        ("the chest next to the door", 17),
        ("the door next to the chest", 6),
        ("the statue around the keg", 11),
        ("the keg around the statue", 12),
        ("the banner above the door", 8),
        ("the candle below the banner", 24),
        ("the guard facing the altar", 20),
        ("the altar facing the guard", 19),
        ("the pillar aligned with the granite", 13),
        ("the pillar aligned with the sandstone", 14),
        ("the red shield near the brazier", 2),
        ("the blue shield near the brazier", 3),
        ("the brazier near the red shield", 1),
        ("the red shield far from the table", 2),
        ("the guard far from the table", 20),
        ("the statue far from the table", 11),
        ("the banner far from the table", 8),
        ("the armory far from the table", 22),
        ("the spear far from the table", 21),
        ("the candle far from the table", 24),
        ("the lantern far from the table", 7),
        ("the table far from the door", 15),
        ("the torch nearest the door", 5),
        ("the torch farthest from the door", 4),
        ("the light nearest the table", 4),
        ("the light farthest from the table", 7),
        ("the container next to the door", 17),
    ];

    /// The CPU oracle: a strict resolver with no scoring and no ranking.
    ///
    /// It shares the phrase lexicon and the relation substrate with the
    /// module and re-derives the answer with plain filters: the most
    /// specific reading of the head word, the modifiers that must hold, the
    /// relation that must be true, and the group tree for containment.
    fn oracle(actors: &[Actor], description: &str) -> Option<u32> {
        let terms = parse_expression(description);
        let graph = build(actors);
        let mut class: Option<(String, Vec<String>)> = None;
        let mut spatial: Vec<(SpatialRel, u32)> = Vec::new();
        let mut contain: Vec<u32> = Vec::new();
        let mut order: Vec<(bool, u32)> = Vec::new();
        for term in &terms {
            match term {
                Term::Class { head, modifiers } => {
                    class = Some((head.clone(), modifiers.clone()));
                }
                Term::Spatial { rel, anchor } => {
                    spatial.push((*rel, oracle_anchor(actors, anchor)?));
                }
                Term::Contain { anchor } => contain.push(oracle_anchor(actors, anchor)?),
                Term::Order { nearest, anchor } => {
                    order.push((*nearest, oracle_anchor(actors, anchor)?));
                }
            }
        }
        let mut ids = oracle_class(actors, class.as_ref());
        ids.retain(|id| oracle_spatial(actors, &graph, *id, &spatial, &contain));
        for (nearest, anchor) in order {
            let mut rows: Vec<(i64, u32)> = ids
                .iter()
                .filter(|id| **id != anchor)
                .map(|id| {
                    (
                        center_dist2_cm2(actors, *id, anchor).unwrap_or(i64::MAX),
                        *id,
                    )
                })
                .collect();
            if nearest {
                rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            } else {
                rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            }
            ids = rows.into_iter().take(1).map(|(_, id)| id).collect();
        }
        if ids.len() == 1 {
            Some(ids[0])
        } else {
            None
        }
    }

    /// The nodes the class phrase names: the label reading first, the class
    /// tags when no label spells the head.
    fn oracle_class(actors: &[Actor], class: Option<&(String, Vec<String>)>) -> Vec<u32> {
        let Some((head, modifiers)) = class else {
            return actors.iter().map(|a| a.id).collect();
        };
        if GENERIC_WORDS.contains(&head.as_str()) {
            return actors.iter().map(|a| a.id).collect();
        }
        let spelled: Vec<u32> = actors
            .iter()
            .filter(|a| own_says(a, head))
            .map(|a| a.id)
            .collect();
        let mut ids = if spelled.is_empty() {
            actors
                .iter()
                .filter(|a| tag_says(a, head))
                .map(|a| a.id)
                .collect()
        } else {
            spelled
        };
        if !modifiers.is_empty() {
            ids.retain(|id| {
                let Some(actor) = actors.iter().find(|a| a.id == *id) else {
                    return false;
                };
                modifiers
                    .iter()
                    .all(|word| own_says(actor, word) || tag_says(actor, word))
            });
        }
        ids
    }

    /// The nodes that hold every spatial and containment term.
    fn oracle_spatial(
        actors: &[Actor],
        graph: &Graph,
        id: u32,
        spatial: &[(SpatialRel, u32)],
        contain: &[u32],
    ) -> bool {
        for (rel, anchor) in spatial {
            let held = match rel {
                SpatialRel::Directed(kind) => holds(actors, id, *kind, *anchor, Frame::World),
                SpatialRel::Edge(kind) => graph.edges.iter().any(|&(a, edge, b)| {
                    edge == *kind
                        && ((b == *anchor && a == id)
                            || (*kind == Relation::NextTo && a == *anchor && b == id))
                }),
            };
            if !held {
                return false;
            }
        }
        for anchor in contain {
            let inside = graph
                .edges
                .iter()
                .any(|&(a, edge, b)| edge == Relation::Inside && b == *anchor && a == id);
            if !inside && !group_holds(actors, id, *anchor) {
                return false;
            }
        }
        true
    }

    /// The node an anchor phrase names: the one label that spells the head
    /// and every modifier of the phrase.
    ///
    /// "the red shield" names the red one: two actors spell "shield", but
    /// only one of them spells "red" too.
    fn oracle_anchor(actors: &[Actor], phrase: &[String]) -> Option<u32> {
        let head = phrase.last()?;
        let modifiers = &phrase[..phrase.len() - 1];
        let ids: Vec<u32> = actors
            .iter()
            .filter(|a| own_says(a, head))
            .filter(|a| {
                modifiers
                    .iter()
                    .all(|word| own_says(a, word) || tag_says(a, word))
            })
            .map(|a| a.id)
            .collect();
        if ids.len() == 1 {
            Some(ids[0])
        } else {
            None
        }
    }

    /// True when the node's own label spells the word.
    fn own_says(actor: &Actor, word: &str) -> bool {
        label_words(&actor.name)
            .iter()
            .any(|token| token == word || singular(token) == singular(word))
    }

    /// True when the class tags of the node name the word, or share one.
    fn tag_says(actor: &Actor, word: &str) -> bool {
        let tags = tag_lift::lift_label(&actor.name);
        for tag in &tags {
            if *tag == word {
                return true;
            }
        }
        tag_lift::lift_label(word)
            .iter()
            .any(|tag| tags.contains(tag))
    }

    /// True when the node sits in the group the anchor names.
    fn group_holds(actors: &[Actor], id: u32, group: u32) -> bool {
        let mut current = actors.iter().find(|a| a.id == id).and_then(|a| a.parent);
        while let Some(parent) = current {
            if parent == group {
                return true;
            }
            current = actors
                .iter()
                .find(|a| a.id == parent)
                .and_then(|a| a.parent);
        }
        false
    }

    #[test]
    fn grounding_ranks_the_referent_of_every_seeded_description() {
        let actors = yard();
        let mut top1 = 0;
        let mut top3 = 0;
        let mut misses: Vec<String> = Vec::new();
        let mut disagreements: Vec<String> = Vec::new();
        for &(description, gold) in &CORPUS {
            let answer = ground(&actors, &GroundArgs::new(description).with_top_k(3));
            let ids: Vec<u32> = answer.hits.iter().map(|hit| hit.id).collect();
            if ids.first() == Some(&gold) {
                top1 += 1;
            }
            if ids.contains(&gold) {
                top3 += 1;
            } else {
                misses.push(format!("{description} -> {ids:?} (gold {gold})"));
            }
            let resolved = oracle(&actors, description);
            if resolved != Some(gold) {
                disagreements.push(format!("{description}: oracle {resolved:?} gold {gold}"));
            }
        }
        assert!(
            disagreements.is_empty(),
            "the oracle disagrees with the corpus: {disagreements:?}"
        );
        let recall1 = top1 * 100 / CORPUS.len();
        let recall3 = top3 * 100 / CORPUS.len();
        println!(
            "SP33 receipt: recall@3={recall3}% recall@1={recall1}% corpus={}",
            CORPUS.len()
        );
        assert!(misses.is_empty(), "the corpus missed: {misses:?}");
        assert!(recall3 >= 90, "recall@3 {recall3}% is under 90%");
    }

    #[test]
    fn grounding_bytes_are_stable_over_the_same_actors() {
        let actors = yard();
        for &(description, _) in &CORPUS {
            let args = GroundArgs::new(description).with_top_k(3);
            let first = ground(&actors, &args);
            let second = ground(&actors, &args);
            assert_eq!(
                grounding_bytes(&first),
                grounding_bytes(&second),
                "{description}"
            );
            assert_eq!(first.digest, second.digest, "{description}");
            let mut shuffled = actors.clone();
            shuffled.reverse();
            let other = ground(&shuffled, &args);
            assert_eq!(
                grounding_bytes(&first),
                grounding_bytes(&other),
                "{description}"
            );
        }
    }

    #[test]
    fn grounding_is_advisory_and_writes_nothing() {
        let actors = yard();
        let before = actors.clone();
        let answer = ground(
            &actors,
            &GroundArgs::new("the red shield left of the brazier"),
        );
        assert_eq!(answer.auto_applied, 0);
        assert_eq!(actors, before);
    }

    #[test]
    fn grounding_tolerates_a_plural_and_one_wrong_letter() {
        let actors = yard();
        let plural = ground(&actors, &GroundArgs::new("the shields left of the brazier"));
        assert_eq!(plural.hits.first().map(|hit| hit.id), Some(2));
        let typo = ground(&actors, &GroundArgs::new("the sheld left of the brazier"));
        assert_eq!(typo.hits.first().map(|hit| hit.id), Some(2));
    }

    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "the grounding receipt measures wall time in-test only; no clock reaches the module, the tick or any hash"
    )]
    #[test]
    fn grounding_answers_under_a_millisecond() {
        use std::time::Instant;
        let actors = yard();
        let mut micros: Vec<u128> = Vec::new();
        for &(description, _) in &CORPUS {
            let args = GroundArgs::new(description).with_top_k(3);
            let started = Instant::now();
            let answer = ground(&actors, &args);
            micros.push(started.elapsed().as_micros());
            assert!(!answer.hits.is_empty(), "{description} found no candidate");
        }
        micros.sort_unstable();
        let p95 = micros[(micros.len() * 95 / 100).min(micros.len() - 1)];
        let worst = micros[micros.len() - 1];
        println!(
            "SP33 receipt: p95_us={p95} worst_us={worst} queries={}",
            micros.len()
        );
        assert!(p95 <= 1_000, "grounding p95 {p95} us is over 1 ms");
    }
}
