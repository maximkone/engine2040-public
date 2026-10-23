//! Command palette: one fuzzy index over dock panels, door verbs and assets.
//!
//! Owner: the editor session; the palette panel draws the hits. The index is
//! built once per listing (the nine dock panels, the caller's verb list and the
//! caller's asset list) and searched per keystroke.
//! Matching is case-insensitive over ASCII and graded: an exact label beats a
//! prefix, a prefix beats a word start, a word start beats a subsequence. The
//! score subtracts the label length, so at one tier the shorter row wins, and
//! the tie-break is `(score, kind, label, id)` with a unique id last, so two
//! runs over the same index always return the same order.
//! An empty query returns the caller's most-recently-used order first, then
//! the rest of the index in listing order.
//! Errors: [`PaletteError`]; a refused build holds no index and a refused
//! search returns no hit.

use crate::browser::{AssetKind, MAX_NAME_CHARS, MAX_QUERY_CHARS};
use crate::shell::Panel;

/// Hits one search returns when the caller asks for no limit of its own.
pub const DEFAULT_LIMIT: usize = 20;
/// Longest verb name the index accepts, in chars.
pub const MAX_VERB_CHARS: usize = 64;
/// Queries this short rank only exact, prefix and word-start hits: a
/// one-or-two-letter subsequence would match almost every row.
pub const SHORT_QUERY_CHARS: usize = 2;
/// Frecency a single use adds, in permille of the text score.
pub const FRECENCY_STEP_PERMILLE: u32 = 60;
/// Frecency cap, in permille (+60%): a well-used row lifts inside its tier.
pub const FRECENCY_CAP_PERMILLE: u32 = 600;
/// Most recent ids one overlay keeps.
pub const RECENT_CAP: usize = 16;
/// Most rows the overlay draws at once.
pub const OVERLAY_ROWS: usize = 10;
/// Widest score span one match tier owns, so frecency never crosses a tier.
const TIER_BAND: u32 = 999;

/// Where a palette row came from; the ranking also breaks score ties by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandKind {
    /// A dock panel.
    Panel = 0,
    /// A door verb.
    Verb = 1,
    /// A listed asset.
    Asset = 2,
}

/// How a query met a label, strongest first. Recent rows carry no match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    /// No letters matched: the caller's most-recently-used order.
    Recent = 0,
    /// Query letters appear in order with gaps.
    Subsequence = 1,
    /// Query sits right after a separator (`.`, `_`, `-`, `/`, `:`, space).
    WordStart = 2,
    /// Label starts with the query.
    Prefix = 3,
    /// Whole label equals the query.
    Exact = 4,
}

impl MatchTier {
    /// Base score of the tier before the label-length penalty.
    fn base(self) -> u32 {
        match self {
            Self::Recent => 0,
            Self::Subsequence => 1_000,
            Self::WordStart => 2_000,
            Self::Prefix => 3_000,
            Self::Exact => 4_000,
        }
    }

    /// Short tag the overlay draws beside a row.
    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Subsequence => "fuzzy",
            Self::WordStart => "word",
            Self::Prefix => "prefix",
            Self::Exact => "exact",
        }
    }
}

/// One palette row: stable id, display label and source kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// Stable id: `panel.<title>`, `verb.<name>` or `asset.<kind>.<name>`.
    pub id: String,
    /// Text the query matches and the row shows.
    pub label: String,
    /// Source the row came from.
    pub kind: CommandKind,
    /// How often the caller picked this row; a bump is one use. Frecency
    /// scales the score inside its tier ([`FRECENCY_STEP_PERMILLE`] a use,
    /// capped at [`FRECENCY_CAP_PERMILLE`]) and never crosses tiers.
    pub frecency: u16,
}

impl Command {
    /// The row for one dock panel.
    pub fn panel(panel: Panel) -> Self {
        let title = panel.title();
        Self {
            id: format!("panel.{}", title.to_ascii_lowercase()),
            label: title.to_string(),
            kind: CommandKind::Panel,
            frecency: 0,
        }
    }

    /// The row for one door verb. An empty or over-long name refuses.
    pub fn verb(name: &str) -> Result<Self, PaletteError> {
        let name = name.trim();
        check_len("verb", name.chars().count(), MAX_VERB_CHARS)?;
        Ok(Self {
            id: format!("verb.{name}"),
            label: name.to_string(),
            kind: CommandKind::Verb,
            frecency: 0,
        })
    }

    /// The row for one listed asset. An empty or over-long name refuses.
    pub fn asset(kind: AssetKind, name: &str) -> Result<Self, PaletteError> {
        let name = name.trim();
        check_len("asset name", name.chars().count(), MAX_NAME_CHARS)?;
        Ok(Self {
            id: format!("asset.{}.{name}", kind.label()),
            label: name.to_string(),
            kind: CommandKind::Asset,
            frecency: 0,
        })
    }

    /// The panel this row focuses, when it is a panel row.
    pub fn panel_target(&self) -> Option<Panel> {
        let id = self.id.strip_prefix("panel.")?;
        Panel::all()
            .into_iter()
            .find(|panel| panel.title().to_ascii_lowercase() == id)
    }

    /// The verb name this row runs, when it is a verb row.
    pub fn verb_name(&self) -> Option<&str> {
        self.id.strip_prefix("verb.")
    }

    /// The asset name this row selects, when it is an asset row.
    pub fn asset_name(&self) -> Option<&str> {
        self.id
            .strip_prefix("asset.")
            .and_then(|rest| rest.split_once('.'))
            .map(|(_, name)| name)
    }
}

/// One ranked hit: the row, its score and the matched label ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandHit {
    /// The matched row.
    pub command: Command,
    /// Rank score; higher first. Zero for a recent row.
    pub score: u32,
    /// How the query met the label.
    pub tier: MatchTier,
    /// Matched char ranges in `label`, half-open, ascending, merged when
    /// adjacent. Empty for a recent row.
    pub ranges: Vec<(usize, usize)>,
}

/// The palette index: panels, then verbs, then assets, in listing order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandIndex {
    commands: Vec<Command>,
}

impl CommandIndex {
    /// Build the index from the nine dock panels, a caller verb list and a
    /// caller asset list.
    ///
    /// Panels come in dock order ([`Panel::all`]); verbs and assets keep the
    /// caller's order. A repeated id refuses the whole build.
    pub fn build(verbs: &[&str], assets: &[(AssetKind, &str)]) -> Result<Self, PaletteError> {
        let mut commands: Vec<Command> = Panel::all().into_iter().map(Command::panel).collect();
        for verb in verbs {
            commands.push(Command::verb(verb)?);
        }
        for (kind, name) in assets {
            commands.push(Command::asset(*kind, name)?);
        }
        let mut seen = std::collections::BTreeSet::new();
        for command in &commands {
            if !seen.insert(command.id.as_str()) {
                return Err(PaletteError::DuplicateId {
                    id: command.id.clone(),
                });
            }
        }
        Ok(Self { commands })
    }

    /// Row count.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// True when the index holds no row.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Every row in listing order.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Count one use of `id`: its frecency rises by one, saturating. False
    /// when the index holds no such row.
    pub fn bump(&mut self, id: &str) -> bool {
        let Some(command) = self.commands.iter_mut().find(|row| row.id == id) else {
            return false;
        };
        command.frecency = command.frecency.saturating_add(1);
        true
    }

    /// Rank the index against `query`, returning at most `limit` hits.
    ///
    /// `recent` is the caller's most-recently-used id order; it only shapes an
    /// empty query, yet it is always accepted so a panel can pass it on every
    /// keystroke. An empty (or blank) query returns the recent rows that still
    /// exist first, in that order, then the rest of the index.
    pub fn search(
        &self,
        query: &str,
        recent: &[String],
        limit: usize,
    ) -> Result<Vec<CommandHit>, PaletteError> {
        let trimmed = query.trim();
        let query_len = trimmed.chars().count();
        if query_len > MAX_QUERY_CHARS {
            return Err(PaletteError::QueryTooLong {
                len: query_len,
                max: MAX_QUERY_CHARS,
            });
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        if trimmed.is_empty() {
            return Ok(self.recent_hits(recent, limit));
        }
        let needle: Vec<char> = trimmed.chars().map(|c| c.to_ascii_lowercase()).collect();
        let mut hits: Vec<CommandHit> = self
            .commands
            .iter()
            .filter_map(|command| {
                let (tier, ranges) = match_label(&command.label, &needle)?;
                if needle.len() <= SHORT_QUERY_CHARS && tier == MatchTier::Subsequence {
                    return None;
                }
                Some(CommandHit {
                    command: command.clone(),
                    score: scaled_score(tier, command.label.chars().count(), command.frecency),
                    tier,
                    ranges,
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(a.command.kind.cmp(&b.command.kind))
                .then(a.command.label.cmp(&b.command.label))
                .then(a.command.id.cmp(&b.command.id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// Empty-query path: recent ids first, then the rest of the index.
    fn recent_hits(&self, recent: &[String], limit: usize) -> Vec<CommandHit> {
        let mut taken = std::collections::BTreeSet::new();
        let mut hits = Vec::with_capacity(limit.min(self.commands.len()));
        for id in recent {
            if hits.len() >= limit {
                return hits;
            }
            if taken.contains(id.as_str()) {
                continue;
            }
            if let Some(command) = self.commands.iter().find(|row| &row.id == id) {
                taken.insert(command.id.as_str());
                hits.push(CommandHit {
                    command: command.clone(),
                    score: 0,
                    tier: MatchTier::Recent,
                    ranges: Vec::new(),
                });
            }
        }
        for command in &self.commands {
            if hits.len() >= limit {
                break;
            }
            if taken.insert(command.id.as_str()) {
                hits.push(CommandHit {
                    command: command.clone(),
                    score: 0,
                    tier: MatchTier::Recent,
                    ranges: Vec::new(),
                });
            }
        }
        hits
    }
}

/// One overlay row: what to draw, its tier tag, its highlight ranges and
/// whether the cursor sits on it. The dock paints labels and ranges and never
/// re-ranks, so the draw list is testable without a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    /// Row label to draw.
    pub label: String,
    /// Row id, drawn dim beside the label.
    pub detail: String,
    /// Match tier, drawn as [`MatchTier::label`].
    pub tier: MatchTier,
    /// Ranges to highlight in `label`.
    pub ranges: Vec<(usize, usize)>,
    /// True when the cursor is on this row.
    pub selected: bool,
}

/// Keys the palette overlay handles; the dock maps egui keys onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKey {
    /// Ctrl+K or Ctrl+P: toggle the overlay.
    Toggle,
    /// Up arrow: move the cursor one row up.
    MoveUp,
    /// Down arrow: move the cursor one row down.
    MoveDown,
    /// Enter: dispatch the row under the cursor.
    Enter,
    /// Escape: close the overlay.
    Escape,
}

/// What a dispatched row asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    /// Focus that dock panel (the panel row).
    FocusPanel(Panel),
    /// Run that door verb through the command service.
    RunVerb(String),
    /// Select that asset in the browser.
    SelectAsset(String),
}

/// Palette overlay state: open flag, query, cursor and the caller's recent
/// ids. The dock owns one; every key goes through [`PaletteState::key`], so
/// the whole overlay is testable without a window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaletteState {
    open: bool,
    query: String,
    cursor: usize,
    recent: Vec<String>,
}

impl PaletteState {
    /// Closed overlay with an empty query and an empty recent list.
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the overlay is shown.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Current query text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Row the cursor sits on, `0` when the query is empty.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The caller's recent ids, newest first.
    pub fn recent(&self) -> &[String] {
        &self.recent
    }

    /// The ranked rows for the current query.
    pub fn hits(&self, index: &CommandIndex) -> Result<Vec<CommandHit>, PaletteError> {
        index.search(&self.query, &self.recent, DEFAULT_LIMIT)
    }

    /// Rows the overlay draws for the current query, cursor included.
    pub fn draw_rows(&self, index: &CommandIndex) -> Result<Vec<PaletteRow>, PaletteError> {
        let hits = index.search(&self.query, &self.recent, OVERLAY_ROWS)?;
        Ok(hits
            .iter()
            .enumerate()
            .map(|(row, hit)| PaletteRow {
                label: hit.command.label.clone(),
                detail: hit.command.id.clone(),
                tier: hit.tier,
                ranges: hit.ranges.clone(),
                selected: row == self.cursor,
            })
            .collect())
    }

    /// Replace the query (one keystroke); the cursor returns to the top row.
    pub fn set_query(&mut self, query: &str) {
        self.query.clear();
        self.query.push_str(query);
        self.cursor = 0;
    }

    /// Run one key. `Ok(Some(action))` when the shell must act on it (Enter),
    /// `Ok(None)` when the key only changed overlay state, and `Err` when the
    /// query refused ([`PaletteError`]).
    ///
    /// Enter bumps the chosen row's frecency and records its id as newest in
    /// the recent list, then closes the overlay.
    pub fn key(
        &mut self,
        key: PaletteKey,
        index: &mut CommandIndex,
    ) -> Result<Option<PaletteAction>, PaletteError> {
        match key {
            PaletteKey::Toggle => {
                self.open = !self.open;
                if self.open {
                    self.cursor = 0;
                } else {
                    self.query.clear();
                }
            }
            PaletteKey::Escape => {
                self.open = false;
                self.query.clear();
                self.cursor = 0;
            }
            PaletteKey::MoveUp => self.cursor = self.cursor.saturating_sub(1),
            PaletteKey::MoveDown => {
                let len = self.hits(index)?.len();
                if self.cursor + 1 < len {
                    self.cursor += 1;
                }
            }
            PaletteKey::Enter => {
                let hits = self.hits(index)?;
                let Some(hit) = hits.get(self.cursor) else {
                    return Ok(None);
                };
                let Some(action) = dispatch(&hit.command) else {
                    return Ok(None);
                };
                index.bump(&hit.command.id);
                self.recent.retain(|id| id != &hit.command.id);
                self.recent.insert(0, hit.command.id.clone());
                self.recent.truncate(RECENT_CAP);
                self.open = false;
                self.query.clear();
                self.cursor = 0;
                return Ok(Some(action));
            }
        }
        Ok(None)
    }
}

/// Map one row onto the shell action its kind names, `None` for a row whose id
/// does not carry its kind (a caller-built row).
fn dispatch(command: &Command) -> Option<PaletteAction> {
    match command.kind {
        CommandKind::Panel => command.panel_target().map(PaletteAction::FocusPanel),
        CommandKind::Verb => command
            .verb_name()
            .map(|name| PaletteAction::RunVerb(name.to_string())),
        CommandKind::Asset => command
            .asset_name()
            .map(|name| PaletteAction::SelectAsset(name.to_string())),
    }
}

/// Refuse a text field that is empty or past its char bound.
fn check_len(field: &'static str, len: usize, max: usize) -> Result<(), PaletteError> {
    if len == 0 {
        return Err(PaletteError::EmptyLabel { field });
    }
    if len > max {
        return Err(PaletteError::TooLong { field, len, max });
    }
    Ok(())
}

/// Score one matched row: the tier's band, the text gap inside it, then the
/// frecency multiplier capped by [`FRECENCY_CAP_PERMILLE`].
///
/// The band keeps `[base - 999, base]` and the next tier starts at
/// `base + 1` once its own text gap is subtracted, so a fully used lower-tier
/// row can never pass a higher-tier row.
fn scaled_score(tier: MatchTier, label_len: usize, frecency: u16) -> u32 {
    if tier == MatchTier::Recent {
        return 0;
    }
    let penalty = u32::try_from(label_len).unwrap_or(u32::MAX).min(TIER_BAND);
    let inner = TIER_BAND - penalty;
    let boost = u32::from(frecency)
        .saturating_mul(FRECENCY_STEP_PERMILLE)
        .min(FRECENCY_CAP_PERMILLE);
    let scaled = inner.saturating_mul(1000 + boost) / 1000;
    tier.base() - TIER_BAND + scaled.min(TIER_BAND)
}

/// True for the characters that start a new word in a label.
fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '.' | '_' | '-' | '/' | ':')
}

/// Fuzzy match one label: tier plus the matched char ranges.
fn match_label(label: &str, query: &[char]) -> Option<(MatchTier, Vec<(usize, usize)>)> {
    let chars: Vec<char> = label.chars().map(|c| c.to_ascii_lowercase()).collect();
    if query.is_empty() || query.len() > chars.len() {
        return None;
    }
    let head = chars[..query.len()].iter().eq(query.iter());
    if head {
        let tier = if chars.len() == query.len() {
            MatchTier::Exact
        } else {
            MatchTier::Prefix
        };
        return Some((tier, vec![(0, query.len())]));
    }
    if let Some(start) = (1..=chars.len() - query.len()).find(|start| {
        is_separator(chars[start - 1])
            && chars[*start..*start + query.len()].iter().eq(query.iter())
    }) {
        return Some((MatchTier::WordStart, vec![(start, start + query.len())]));
    }
    subsequence_ranges(&chars, query).map(|ranges| (MatchTier::Subsequence, ranges))
}

/// Greedy subsequence walk; adjacent matched chars merge into one range.
fn subsequence_ranges(chars: &[char], query: &[char]) -> Option<Vec<(usize, usize)>> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut next = 0usize;
    for wanted in query {
        let found = chars[next..]
            .iter()
            .position(|c| c == wanted)
            .map(|offset| next + offset)?;
        match ranges.last_mut() {
            Some(last) if last.1 == found => last.1 = found + 1,
            _ => ranges.push((found, found + 1)),
        }
        next = found + 1;
    }
    Some(ranges)
}

/// What refused a build or a search. Nothing was indexed or returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteError {
    /// A row carried no text in `field`.
    EmptyLabel {
        /// The field that was empty, e.g. `verb`.
        field: &'static str,
    },
    /// Two rows share one id.
    DuplicateId {
        /// The repeated id.
        id: String,
    },
    /// One field went over its char bound.
    TooLong {
        /// The field name.
        field: &'static str,
        /// Chars given.
        len: usize,
        /// Chars allowed.
        max: usize,
    },
    /// The query went over the char bound.
    QueryTooLong {
        /// Chars given.
        len: usize,
        /// Chars allowed.
        max: usize,
    },
}

impl std::fmt::Display for PaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyLabel { field } => {
                write!(f, "{field} holds no text; pass 1 to {MAX_VERB_CHARS} chars")
            }
            Self::DuplicateId { id } => write!(
                f,
                "palette id `{id}` repeats; rename one row or drop the duplicate"
            ),
            Self::TooLong { field, len, max } => write!(
                f,
                "{field} holds {len} chars over the {max} bound; shorten it to {max} chars"
            ),
            Self::QueryTooLong { len, max } => write!(
                f,
                "query holds {len} chars over the {max} bound; shorten it to {max} chars"
            ),
        }
    }
}

impl std::error::Error for PaletteError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> CommandIndex {
        CommandIndex::build(
            &[
                "scene.place",
                "scene.place.batch",
                "scene.pick",
                "view.capture",
            ],
            &[
                (AssetKind::Mesh, "crate"),
                (AssetKind::Mesh, "wall_a"),
                (AssetKind::Mesh, "wall_b"),
                (AssetKind::Material, "stone"),
            ],
        )
        .unwrap()
    }

    fn labels(hits: &[CommandHit]) -> Vec<String> {
        hits.iter().map(|hit| hit.command.label.clone()).collect()
    }

    #[test]
    fn prefix_ranks_the_inspector_panel_first() {
        let hits = index().search("insp", &[], 10).unwrap();
        assert_eq!(hits[0].command.label, "Inspector");
        assert_eq!(hits[0].tier, MatchTier::Prefix);
        assert_eq!(hits[0].ranges, vec![(0, 4)]);
        assert_eq!(hits[0].command.kind, CommandKind::Panel);
        // Case does not matter and the ranges follow the label spelling.
        let upper = index().search("INSP", &[], 10).unwrap();
        assert_eq!(labels(&upper), labels(&hits));
    }

    #[test]
    fn exact_verb_beats_a_longer_match() {
        let hits = index().search("scene.place", &[], 10).unwrap();
        assert_eq!(hits[0].command.label, "scene.place");
        assert_eq!(hits[0].tier, MatchTier::Exact);
        assert_eq!(hits[0].command.id, "verb.scene.place");
        let longer = hits
            .iter()
            .position(|hit| hit.command.label == "scene.place.batch")
            .expect("the longer verb is still a hit");
        assert!(longer > 0, "{:?}", labels(&hits));
        assert_eq!(hits[longer].tier, MatchTier::Prefix);
    }

    #[test]
    fn subsequence_hits_carry_merged_ranges() {
        let hits = index().search("vcap", &[], 10).unwrap();
        let hit = hits
            .iter()
            .find(|hit| hit.command.label == "view.capture")
            .expect("a subsequence hit");
        assert_eq!(hit.tier, MatchTier::Subsequence);
        assert_eq!(hit.ranges, vec![(0, 1), (5, 8)]);
    }

    #[test]
    fn tie_break_is_stable_across_two_runs() {
        let forward = index().search("wall", &[], 10).unwrap();
        // Same rows in the opposite build order must rank the same way.
        let reversed = CommandIndex::build(
            &[],
            &[(AssetKind::Mesh, "wall_b"), (AssetKind::Mesh, "wall_a")],
        )
        .unwrap()
        .search("wall", &[], 10)
        .unwrap();
        assert_eq!(labels(&forward[..2]), vec!["wall_a", "wall_b"]);
        assert_eq!(labels(&forward[..2]), labels(&reversed[..2]));
        assert_eq!(forward[0].score, forward[1].score, "a real score tie");
        assert_eq!(index().search("wall", &[], 10).unwrap(), forward);
    }

    #[test]
    fn frecency_lifts_inside_a_tier_but_never_crosses_one() {
        let mut index = CommandIndex::build(
            &[],
            &[
                (AssetKind::Mesh, "wall"),
                (AssetKind::Mesh, "wall_a"),
                (AssetKind::Mesh, "wall_b"),
            ],
        )
        .unwrap();
        for _ in 0..20 {
            assert!(index.bump("asset.mesh.wall_b"));
        }
        let hits = index.search("wall", &[], 10).unwrap();
        assert_eq!(hits[0].command.label, "wall", "the exact row still leads");
        assert_eq!(hits[0].tier, MatchTier::Exact);
        assert_eq!(hits[1].command.label, "wall_b", "usage breaks the text tie");
        assert_eq!(hits[2].command.label, "wall_a");
        assert!(!index.bump("verb.gone"), "an unknown id is not counted");

        // A lower tier at the frecency cap never passes the exact row.
        for _ in 0..100 {
            index.bump("asset.mesh.wall_a");
        }
        let capped = index.search("wall", &[], 10).unwrap();
        assert_eq!(capped[0].command.label, "wall");
        assert_eq!(capped[0].tier, MatchTier::Exact);
        assert!(
            capped[0].score > 3000,
            "exact band sits above every prefix score: {}",
            capped[0].score
        );
        // Two runs over the bumped index are equal.
        assert_eq!(index.search("wall", &[], 10).unwrap(), capped);
    }

    #[test]
    fn short_queries_skip_subsequence_noise() {
        let index = CommandIndex::build(&[], &[(AssetKind::Mesh, "brazier")]).unwrap();
        assert!(index.search("z", &[], 10).unwrap().is_empty(), "one letter");
        assert!(
            index.search("ra", &[], 10).unwrap().is_empty(),
            "two letters"
        );
        let long = CommandIndex::build(&["view.capture"], &[]).unwrap();
        let hits = long.search("vcap", &[], 10).unwrap();
        assert_eq!(hits[0].tier, MatchTier::Subsequence);
    }

    #[test]
    fn overlay_state_machine_dispatches_by_kind() {
        let mut index = index();
        let mut state = PaletteState::new();
        assert!(!state.is_open());
        assert_eq!(state.key(PaletteKey::Toggle, &mut index).unwrap(), None);
        assert!(state.is_open());
        state.set_query("insp");
        assert_eq!(
            state.key(PaletteKey::Enter, &mut index).unwrap(),
            Some(PaletteAction::FocusPanel(Panel::Inspector))
        );
        assert!(!state.is_open(), "Enter closes the overlay");
        assert_eq!(state.recent()[0], "panel.inspector");

        state.key(PaletteKey::Toggle, &mut index).unwrap();
        state.set_query("scene.place");
        assert_eq!(
            state.key(PaletteKey::Enter, &mut index).unwrap(),
            Some(PaletteAction::RunVerb("scene.place".to_string()))
        );
        let verb = index
            .commands()
            .iter()
            .find(|row| row.id == "verb.scene.place")
            .unwrap();
        assert_eq!(verb.frecency, 1, "a dispatch counts one use");

        state.key(PaletteKey::Toggle, &mut index).unwrap();
        state.set_query("crate");
        assert_eq!(
            state.key(PaletteKey::Enter, &mut index).unwrap(),
            Some(PaletteAction::SelectAsset("crate".to_string()))
        );
        assert_eq!(
            state.recent()[..3],
            [
                "asset.mesh.crate".to_string(),
                "verb.scene.place".to_string(),
                "panel.inspector".to_string()
            ],
            "newest first"
        );

        // Cursor clamps at both ends of the current row list.
        state.key(PaletteKey::Toggle, &mut index).unwrap();
        state.set_query("");
        let rows = state.hits(&index).unwrap().len();
        state.key(PaletteKey::MoveUp, &mut index).unwrap();
        assert_eq!(state.cursor(), 0);
        state.key(PaletteKey::MoveDown, &mut index).unwrap();
        assert_eq!(state.cursor(), 1);
        for _ in 0..(rows + 10) {
            state.key(PaletteKey::MoveDown, &mut index).unwrap();
        }
        assert_eq!(state.cursor(), rows - 1);
        assert_eq!(state.key(PaletteKey::Escape, &mut index).unwrap(), None);
        assert!(!state.is_open());
        assert_eq!(state.query(), "");
        // An empty row list makes Enter a no-op, never a panic.
        let mut empty = CommandIndex::default();
        let mut bare = PaletteState::new();
        bare.set_query("zzz");
        assert_eq!(bare.key(PaletteKey::Enter, &mut empty).unwrap(), None);
    }

    #[test]
    fn overlay_draw_rows_marks_the_cursor_and_carries_highlights() {
        let mut index = index();
        let mut state = PaletteState::new();
        state.key(PaletteKey::Toggle, &mut index).unwrap();
        state.set_query("insp");
        let rows = state.draw_rows(&index).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Inspector");
        assert_eq!(rows[0].tier, MatchTier::Prefix);
        assert_eq!(rows[0].tier.label(), "prefix");
        assert_eq!(rows[0].ranges, vec![(0, 4)]);
        assert!(rows[0].selected);
        assert_eq!(rows[0].detail, "panel.inspector");

        // Empty query: at most OVERLAY_ROWS rows, exactly one selected.
        state.set_query("");
        let rows = state.draw_rows(&index).unwrap();
        assert!(rows.len() <= OVERLAY_ROWS);
        assert_eq!(rows.iter().filter(|row| row.selected).count(), 1);
        assert_eq!(rows[0].tier, MatchTier::Recent);
        // No hit: no row, so nothing is selected.
        state.set_query("zzzz");
        assert!(state.draw_rows(&index).unwrap().is_empty());
    }

    #[test]
    fn empty_query_returns_the_caller_recent_order() {
        let recent = vec![
            "verb.scene.pick".to_string(),
            "panel.inspector".to_string(),
            "gone.row".to_string(),
        ];
        let hits = index().search("   ", &recent, 3).unwrap();
        assert_eq!(
            hits.iter()
                .map(|hit| hit.command.id.as_str())
                .collect::<Vec<_>>(),
            vec!["verb.scene.pick", "panel.inspector", "panel.hierarchy"]
        );
        assert!(hits.iter().all(|hit| hit.tier == MatchTier::Recent));
        assert!(hits.iter().all(|hit| hit.ranges.is_empty()));
        // Dedupe: a repeated recent id is returned once.
        let twice = vec!["panel.inspector".to_string(), "panel.inspector".to_string()];
        assert_eq!(
            index().search("", &twice, 10).unwrap()[0].command.label,
            "Inspector"
        );
        assert_eq!(
            index().search("", &twice, 10).unwrap()[1].command.id,
            "panel.hierarchy"
        );
    }

    #[test]
    fn limit_bounds_the_result_and_bad_rows_refuse() {
        let index = index();
        assert!(index.search("", &[], 0).unwrap().is_empty());
        assert_eq!(index.search("wall", &[], 2).unwrap().len(), 2);
        assert_eq!(index.search("zzzz", &[], 10).unwrap().len(), 0);
        assert_eq!(
            CommandIndex::build(&["scene.place", "scene.place"], &[]),
            Err(PaletteError::DuplicateId {
                id: "verb.scene.place".to_string()
            })
        );
        assert_eq!(
            CommandIndex::build(&["  "], &[]),
            Err(PaletteError::EmptyLabel { field: "verb" })
        );
        let long = "x".repeat(MAX_QUERY_CHARS + 1);
        assert_eq!(
            index.search(&long, &[], 10),
            Err(PaletteError::QueryTooLong {
                len: MAX_QUERY_CHARS + 1,
                max: MAX_QUERY_CHARS
            })
        );
        // The index is panels + verbs + assets, in that order.
        assert_eq!(index.len(), Panel::all().len() + 4 + 4);
        assert_eq!(index.commands()[0].kind, CommandKind::Panel);
        assert_eq!(index.commands()[index.len() - 1].kind, CommandKind::Asset);
        // One name in two asset kinds stays two rows: the id carries the kind.
        let two = CommandIndex::build(
            &[],
            &[(AssetKind::Mesh, "wall"), (AssetKind::Material, "wall")],
        )
        .unwrap();
        assert_eq!(two.len(), Panel::all().len() + 2);
    }
}
