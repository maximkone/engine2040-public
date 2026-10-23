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
}

impl Command {
    /// The row for one dock panel.
    pub fn panel(panel: Panel) -> Self {
        let title = panel.title();
        Self {
            id: format!("panel.{}", title.to_ascii_lowercase()),
            label: title.to_string(),
            kind: CommandKind::Panel,
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
        })
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
                let penalty = u32::try_from(command.label.chars().count())
                    .unwrap_or(u32::MAX)
                    .min(999);
                Some(CommandHit {
                    command: command.clone(),
                    score: tier.base().saturating_sub(penalty),
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
        assert_eq!(index.search("e", &[], 2).unwrap().len(), 2);
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
