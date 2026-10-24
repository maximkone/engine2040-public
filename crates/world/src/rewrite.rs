//! rewrite: a seeded grid rewrite-rule generator an AI authors as text.
//! Source: web: mxgmn/MarkovJunior (nodes `one` / `all` / `prl` / `markov` /
//! `sequence`, symmetry-expanded `IN=OUT` rules), read 2026-10-24, the idea
//! only, zero bytes vendored; the text format below is ours (MarkovJunior is XML).
//! Owner: engine-world, bake-time only, never the tick or the sim hash.
//! Units: cells are `u8` symbol indices into the alphabet, `x` fastest then `y`,
//! row 0 is the top row, `y` grows downward; a pattern lists its rows top first.
//! Determinism: the same `(rules text, seed)` always gives the same grid and
//! [`Grid::hash`]; every draw is one `splitmix64` step off the seed, integers
//! only, and cells are scanned in grid order, never through a map.
//! Errors: [`RulesError`] refuses a bad rules text with the 1-based line number;
//! [`RunError`] refuses a run that passes [`MAX_STEPS`] or [`MAX_WORK`], never a
//! partial grid (the author fixes the rules or picks fewer steps).
//! Budget: parse allocates the program once; a run allocates the grid plus at
//! most one grid-sized scratch buffer per node kind it uses, and no step
//! allocates. `width, height <= MAX_SIDE`, [`MAX_STEPS`] steps and [`MAX_WORK`]
//! cell comparisons per run, so no rule set can hang a bake.
//! Consumer: arena and street layout bakes (`content/maps/rewrite/*.rules`), the
//! UW12/UW20 insula floor plans, and a `world.rewrite` door beside `world.wfc`.
//!
//! # Rules text
//!
//! One statement per line (a `{` ends a statement and a `}` stands alone, each
//! as its own space-separated word, so `one { rule A = B }` fits on one line);
//! `#` starts a comment anywhere on a line; blank lines are ignored. Symbols are single ASCII graphic characters other than
//! `# / * = { }`. A statement is one of:
//!
//! ```text
//! size 31 31               # width height, 1..=128 each; first statement
//! alphabet BWDR            # the symbols; the first one fills the grid at start
//! put 1 1 R                # set one cell:      put X Y SYMBOL
//! rect 3 3 18 12 W         # fill a rectangle:  rect X Y W H SYMBOL
//! one steps=40 {           # a node; steps=N caps how many steps it takes
//!   rule RBB = GGR         # IN = OUT, rows separated by `/`
//!   rule W/B = W/W p=50 sym=none
//! }
//! ```
//!
//! Nodes (`steps=N` is optional on `one`, `all` and `prl`; none means "until
//! nothing matches"):
//! - `one { rule ... }` takes one step at a time: pick one match uniformly from
//!   every match of every rule and apply it.
//! - `all { rule ... }` takes one round per step: visit the cells in a seeded
//!   random order and apply each match whose cells no earlier match of the
//!   round touched, so the applied matches never overlap.
//! - `prl { rule ... }` takes one round per step: find every match in the grid
//!   as it was at the start of the round and apply them all at once (rule
//!   order, then grid order; where two overlap the later write wins).
//! - `sequence { node ... }` runs its children in order, each until it can take
//!   no more steps. The whole file is a `sequence`.
//! - `markov { node ... }` takes one step of its first child that can: after
//!   every step it starts again from the first child, until none can step.
//! - `put` and `rect` are one-step nodes that write cells (not allowed inside
//!   `one`, `all` or `prl`, which hold only rules).
//!
//! A rule is `rule IN = OUT [p=N] [sym=S]`. `IN` and `OUT` are rows of symbols
//! joined by `/` and must have the same shape (at most 16 by 16). In `IN`, `*`
//! matches any symbol; in `OUT`, `*` leaves the cell alone. A match must change
//! at least one cell, so a rule never matches where it has already been applied
//! and a rule that can never change a cell is refused. `p=N` (1..=100, default
//! 100) is the percent chance a drawn match is applied; a failed roll still
//! spends the step. `sym=` picks the orientations tried: `none`, `flip` (the
//! pattern and its mirror), `rot` (four rotations) or `all` (the 8 symmetries
//! of the square, the default, as in MarkovJunior); duplicates are dropped.
//!
//! Example, a maze (a head `R` digs two cells at a time and backs out of dead
//! ends, so the passages `W` form one tree inside a wall `B`):
//!
//! ```text
//! size 15 15
//! alphabet BWGR
//! put 1 1 R
//! markov {
//!   one { rule RBB = GGR }
//!   one { rule RGG = WWR }
//! }
//! one { rule R = W }
//! ```

use std::fmt;

use engine_core::{hash::GOLDEN_GAMMA, mix64, Fnv64};

/// Rules text format version.
pub const REWRITE_VERSION: u8 = 1;
/// Largest accepted grid side in cells, checked before any allocation.
pub const MAX_SIDE: u16 = 128;
/// Largest alphabet: the wildcard byte stays free and a symbol fits one `u8`.
pub const MAX_SYMBOLS: usize = 62;
/// Largest pattern side in cells.
pub const MAX_PATTERN_SIDE: usize = 16;
/// Largest rule count in one `one` / `all` / `prl` node.
pub const MAX_RULES_PER_NODE: usize = 64;
/// Largest rule count in a whole program.
pub const MAX_RULES: usize = 256;
/// Largest node count in a whole program.
pub const MAX_NODES: usize = 256;
/// Deepest `{ }` nesting.
pub const MAX_DEPTH: usize = 16;
/// Steps one run may take, all nodes together, before it is refused.
pub const MAX_STEPS: u32 = 200_000;
/// Cell comparisons one run may spend before it is refused (about a second).
pub const MAX_WORK: u64 = 400_000_000;
/// Largest `steps=N`.
pub const MAX_NODE_STEPS: u32 = 1_000_000;

/// A pattern cell that matches any symbol (`in`) or leaves the cell (`out`).
const WILD: u8 = u8::MAX;

/// Why a rules text was refused. `line` is 1-based; 0 means the whole file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesError {
    /// 1-based line of the statement at fault (the opening line for an unclosed block).
    pub line: usize,
    /// What is wrong.
    pub kind: RulesErrorKind,
}

/// What was wrong with a rules statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesErrorKind {
    /// The first word is not a statement.
    UnknownStatement(String),
    /// A statement lacks an argument; names the expected form.
    Missing(&'static str),
    /// Words left over after a complete statement.
    Trailing(String),
    /// A number that is not an unsigned integer.
    BadNumber(String),
    /// A number outside its range; names the field and the range.
    OutOfRange(&'static str),
    /// A statement that may appear only once.
    Duplicate(&'static str),
    /// A statement that needs `size` or `alphabet` first.
    NeedHeader(&'static str),
    /// A bad alphabet; names why.
    BadAlphabet(&'static str),
    /// A symbol that is not in the alphabet.
    UnknownSymbol(char),
    /// A bad pattern; names why.
    BadPattern(&'static str),
    /// A rule that can never change a cell.
    NoChange,
    /// An attribute that is not valid here.
    BadAttr(String),
    /// A statement in the wrong place; names the rule.
    Misplaced(&'static str),
    /// A node with no rules or no children.
    EmptyNode,
    /// A `{` that is never closed.
    Unclosed,
    /// A `}` with no `{`.
    StrayClose,
    /// A cap in this module's budget; names it.
    TooMany(&'static str),
    /// The file holds no node at all.
    NoNodes,
}

impl fmt::Display for RulesErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStatement(w) => write!(f, "unknown statement `{w}`"),
            Self::Missing(form) => write!(f, "missing argument, expected `{form}`"),
            Self::Trailing(w) => write!(f, "unexpected `{w}` after the statement"),
            Self::BadNumber(w) => write!(f, "`{w}` is not an unsigned number"),
            Self::OutOfRange(what) => write!(f, "{what} is out of range"),
            Self::Duplicate(what) => write!(f, "`{what}` appears twice"),
            Self::NeedHeader(what) => write!(f, "`{what}` must come before this statement"),
            Self::BadAlphabet(why) => write!(f, "bad alphabet: {why}"),
            Self::UnknownSymbol(c) => write!(f, "symbol `{c}` is not in the alphabet"),
            Self::BadPattern(why) => write!(f, "bad pattern: {why}"),
            Self::NoChange => write!(f, "the rule can never change a cell"),
            Self::BadAttr(w) => write!(f, "bad attribute `{w}`"),
            Self::Misplaced(why) => write!(f, "misplaced statement: {why}"),
            Self::EmptyNode => write!(f, "the node holds nothing"),
            Self::Unclosed => write!(f, "this `{{` is never closed"),
            Self::StrayClose => write!(f, "`}}` closes nothing"),
            Self::TooMany(what) => write!(f, "too many {what}"),
            Self::NoNodes => write!(f, "the rules have no node (put, rect, one, all, prl, ...)"),
        }
    }
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "rules: {}", self.kind)
        } else {
            write!(f, "rules line {}: {}", self.line, self.kind)
        }
    }
}

impl std::error::Error for RulesError {}

/// Why a run was refused. No grid is returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunError {
    /// The run took more than [`MAX_STEPS`] steps (a rule set that never settles).
    StepCap,
    /// The run spent more than [`MAX_WORK`] cell comparisons.
    WorkCap,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StepCap => write!(f, "run passed {MAX_STEPS} steps without settling"),
            Self::WorkCap => write!(f, "run passed {MAX_WORK} cell comparisons"),
        }
    }
}

impl std::error::Error for RunError {}

/// One orientation of a rule: `inp` and `out` are `w * h` cells, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Variant {
    w: usize,
    h: usize,
    inp: Box<[u8]>,
    out: Box<[u8]>,
}

impl Variant {
    /// Does the pattern match with its top-left cell at `(x, y)` (the caller
    /// checked the bounds) and would applying it change a cell? Also the cost
    /// in cell comparisons.
    fn matches(&self, grid: &[u8], width: usize, x: usize, y: usize) -> (bool, u64) {
        let mut cost = 1u64;
        let mut changes = false;
        for dy in 0..self.h {
            let row = (y + dy) * width + x;
            for dx in 0..self.w {
                cost += 1;
                let at = dy * self.w + dx;
                let cell = grid[row + dx];
                let want = self.inp[at];
                if want != WILD && want != cell {
                    return (false, cost);
                }
                let put = self.out[at];
                if put != WILD && put != cell {
                    changes = true;
                }
            }
        }
        (changes, cost)
    }

    /// Write the output cells with the top-left cell at `(x, y)`.
    fn write(&self, grid: &mut [u8], width: usize, x: usize, y: usize) {
        for dy in 0..self.h {
            for dx in 0..self.w {
                let put = self.out[dy * self.w + dx];
                if put != WILD {
                    grid[(y + dy) * width + x + dx] = put;
                }
            }
        }
    }
}

/// A rule: its orientations (already symmetry-expanded) and the percent chance.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    p: u8,
    variants: Vec<Variant>,
}

/// How a rules node takes a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    One,
    All,
    Prl,
}

/// A rectangle write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fill {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    sym: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NodeKind {
    Rules {
        mode: Mode,
        steps: u32,
        rules: Vec<Rule>,
    },
    Fill(Fill),
    Sequence(Vec<Node>),
    Markov(Vec<Node>),
}

/// A node plus its slot in the run's per-node counters.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    id: u32,
    kind: NodeKind,
}

/// A parsed rules text: grid size, alphabet and the node tree. Immutable; a
/// [`Program::run`] with one seed always gives the same [`Grid`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    width: u16,
    height: u16,
    symbols: Vec<u8>,
    root: Node,
    node_count: usize,
    uses_all: bool,
    uses_prl: bool,
}

/// The grid a run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    width: u16,
    height: u16,
    symbols: Vec<u8>,
    cells: Vec<u8>,
    steps: u32,
}

impl Grid {
    /// Cells across.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Cells down.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Symbol indices, row-major, row 0 on top.
    #[must_use]
    pub fn cells(&self) -> &[u8] {
        &self.cells
    }

    /// Steps the run took.
    #[must_use]
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// The symbol character at `(x, y)`, `None` outside the grid.
    #[must_use]
    pub fn symbol_at(&self, x: u16, y: u16) -> Option<char> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = usize::from(y) * usize::from(self.width) + usize::from(x);
        self.symbols
            .get(usize::from(self.cells[at]))
            .map(|&b| char::from(b))
    }

    /// How many cells hold `symbol`.
    #[must_use]
    pub fn count(&self, symbol: char) -> usize {
        let Some(index) = self.symbols.iter().position(|&b| char::from(b) == symbol) else {
            return 0;
        };
        self.cells
            .iter()
            .filter(|&&c| usize::from(c) == index)
            .count()
    }

    /// One line per row, symbol characters, `\n` after each row.
    #[must_use]
    pub fn render(&self) -> String {
        let width = usize::from(self.width);
        let mut text = String::with_capacity((width + 1) * usize::from(self.height));
        for row in self.cells.chunks(width) {
            for &cell in row {
                text.push(char::from(self.symbols[usize::from(cell)]));
            }
            text.push('\n');
        }
        text
    }

    /// Content hash: size, then the cells ([`engine_core::Fnv64`]).
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u16(self.width);
        h.write_u16(self.height);
        h.write(&self.cells);
        h.finish()
    }
}

// ---------------------------------------------------------------- parsing

/// An open `{` while parsing.
struct Frame {
    line: usize,
    kind: FrameKind,
    rules: Vec<Rule>,
    children: Vec<Node>,
}

#[derive(Clone, Copy)]
enum FrameKind {
    Rules(Mode, u32),
    Sequence,
    Markov,
}

/// Parser state across lines.
struct Parser {
    size: Option<(u16, u16)>,
    symbols: Option<Vec<u8>>,
    stack: Vec<Frame>,
    top: Vec<Node>,
    rules: usize,
    nodes: usize,
    uses_all: bool,
    uses_prl: bool,
}

fn err<T>(line: usize, kind: RulesErrorKind) -> Result<T, RulesError> {
    Err(RulesError { line, kind })
}

fn number(line: usize, word: &str) -> Result<u32, RulesError> {
    match word.parse::<u32>() {
        Ok(n) => Ok(n),
        Err(_) => err(line, RulesErrorKind::BadNumber(word.to_owned())),
    }
}

fn reserved(c: char) -> bool {
    !c.is_ascii_graphic() || matches!(c, '#' | '/' | '*' | '=' | '{' | '}')
}

impl Parser {
    fn new() -> Self {
        Self {
            size: None,
            symbols: None,
            stack: Vec::new(),
            top: Vec::new(),
            rules: 0,
            nodes: 0,
            uses_all: false,
            uses_prl: false,
        }
    }

    fn statement(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        let head = words[0];
        let in_rules =
            matches!(self.stack.last(), Some(f) if matches!(f.kind, FrameKind::Rules(..)));
        match head {
            "}" => self.close(line, words),
            "rule" => {
                if !in_rules {
                    return err(
                        line,
                        RulesErrorKind::Misplaced("`rule` belongs inside one, all or prl"),
                    );
                }
                self.rule(line, words)
            }
            _ if in_rules => err(
                line,
                RulesErrorKind::Misplaced("one, all and prl hold only `rule` lines"),
            ),
            "size" | "alphabet" if !self.stack.is_empty() => err(
                line,
                RulesErrorKind::Misplaced("size and alphabet go at the top, outside every block"),
            ),
            "size" => self.size(line, words),
            "alphabet" => self.alphabet(line, words),
            "put" => self.put(line, words),
            "rect" => self.rect(line, words),
            "one" | "all" | "prl" | "sequence" | "markov" => self.open(line, words),
            other => err(line, RulesErrorKind::UnknownStatement(other.to_owned())),
        }
    }

    fn size(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        if words.len() < 3 {
            return err(line, RulesErrorKind::Missing("size WIDTH HEIGHT"));
        }
        if words.len() > 3 {
            return err(line, RulesErrorKind::Trailing(words[3].to_owned()));
        }
        if self.size.is_some() {
            return err(line, RulesErrorKind::Duplicate("size"));
        }
        let w = number(line, words[1])?;
        let h = number(line, words[2])?;
        let side = 1..=u32::from(MAX_SIDE);
        if !side.contains(&w) || !side.contains(&h) {
            return err(line, RulesErrorKind::OutOfRange("size (1..=128 per side)"));
        }
        let (Ok(w), Ok(h)) = (u16::try_from(w), u16::try_from(h)) else {
            return err(line, RulesErrorKind::OutOfRange("size (1..=128 per side)"));
        };
        self.size = Some((w, h));
        Ok(())
    }

    fn alphabet(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        if words.len() < 2 {
            return err(line, RulesErrorKind::Missing("alphabet SYMBOLS"));
        }
        if self.symbols.is_some() {
            return err(line, RulesErrorKind::Duplicate("alphabet"));
        }
        let mut symbols: Vec<u8> = Vec::new();
        for c in words[1..].iter().flat_map(|w| w.chars()) {
            if reserved(c) {
                return err(
                    line,
                    RulesErrorKind::BadAlphabet("symbols are ASCII graphic, not # / * = { }"),
                );
            }
            let Ok(byte) = u8::try_from(c) else {
                return err(line, RulesErrorKind::BadAlphabet("symbols are ASCII"));
            };
            if symbols.contains(&byte) {
                return err(line, RulesErrorKind::BadAlphabet("a symbol appears twice"));
            }
            symbols.push(byte);
        }
        if symbols.len() > MAX_SYMBOLS {
            return err(line, RulesErrorKind::BadAlphabet("more than 62 symbols"));
        }
        self.symbols = Some(symbols);
        Ok(())
    }

    /// The size and alphabet, or the error naming the missing header.
    fn header(&self, line: usize) -> Result<((usize, usize), &[u8]), RulesError> {
        let Some((w, h)) = self.size else {
            return err(line, RulesErrorKind::NeedHeader("size"));
        };
        let Some(symbols) = self.symbols.as_deref() else {
            return err(line, RulesErrorKind::NeedHeader("alphabet"));
        };
        Ok(((usize::from(w), usize::from(h)), symbols))
    }

    fn symbol(&self, line: usize, word: &str) -> Result<u8, RulesError> {
        let (_, symbols) = self.header(line)?;
        let mut chars = word.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return err(
                line,
                RulesErrorKind::BadPattern("a symbol is one character"),
            );
        };
        match u8::try_from(c)
            .ok()
            .and_then(|b| symbols.iter().position(|&s| s == b))
        {
            Some(i) => u8::try_from(i).or_else(|_| err(line, RulesErrorKind::UnknownSymbol(c))),
            None => err(line, RulesErrorKind::UnknownSymbol(c)),
        }
    }

    fn put(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        if words.len() < 4 {
            return err(line, RulesErrorKind::Missing("put X Y SYMBOL"));
        }
        if words.len() > 4 {
            return err(line, RulesErrorKind::Trailing(words[4].to_owned()));
        }
        self.fill(line, &words[1..3], &["1", "1"], words[3])
    }

    fn rect(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        if words.len() < 6 {
            return err(line, RulesErrorKind::Missing("rect X Y W H SYMBOL"));
        }
        if words.len() > 6 {
            return err(line, RulesErrorKind::Trailing(words[6].to_owned()));
        }
        self.fill(line, &words[1..3], &words[3..5], words[5])
    }

    fn fill(
        &mut self,
        line: usize,
        at: &[&str],
        extent: &[&str],
        sym: &str,
    ) -> Result<(), RulesError> {
        let ((gw, gh), _) = self.header(line)?;
        let sym = self.symbol(line, sym)?;
        let x = number(line, at[0])? as usize;
        let y = number(line, at[1])? as usize;
        let w = number(line, extent[0])? as usize;
        let h = number(line, extent[1])? as usize;
        if w == 0 || h == 0 || x.saturating_add(w) > gw || y.saturating_add(h) > gh {
            return err(
                line,
                RulesErrorKind::OutOfRange("rectangle (must lie inside the grid)"),
            );
        }
        self.emit(line, NodeKind::Fill(Fill { x, y, w, h, sym }))
    }

    fn open(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        self.header(line)?;
        if words.last() != Some(&"{") {
            return err(line, RulesErrorKind::Missing("NODE [steps=N] {"));
        }
        if self.stack.len() >= MAX_DEPTH {
            return err(line, RulesErrorKind::TooMany("nested blocks"));
        }
        let mut steps = 0u32;
        for word in &words[1..words.len() - 1] {
            let Some(("steps", n)) = word.split_once('=') else {
                return err(line, RulesErrorKind::BadAttr((*word).to_owned()));
            };
            if matches!(words[0], "sequence" | "markov") {
                return err(line, RulesErrorKind::BadAttr((*word).to_owned()));
            }
            steps = number(line, n)?;
            if steps == 0 || steps > MAX_NODE_STEPS {
                return err(line, RulesErrorKind::OutOfRange("steps (1..=1000000)"));
            }
        }
        let kind = match words[0] {
            "one" => FrameKind::Rules(Mode::One, steps),
            "all" => {
                self.uses_all = true;
                FrameKind::Rules(Mode::All, steps)
            }
            "prl" => {
                self.uses_prl = true;
                FrameKind::Rules(Mode::Prl, steps)
            }
            "sequence" => FrameKind::Sequence,
            _ => FrameKind::Markov,
        };
        self.stack.push(Frame {
            line,
            kind,
            rules: Vec::new(),
            children: Vec::new(),
        });
        Ok(())
    }

    fn close(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        if words.len() > 1 {
            return err(line, RulesErrorKind::Trailing(words[1].to_owned()));
        }
        let Some(frame) = self.stack.pop() else {
            return err(line, RulesErrorKind::StrayClose);
        };
        let kind = match frame.kind {
            FrameKind::Rules(mode, steps) if !frame.rules.is_empty() => NodeKind::Rules {
                mode,
                steps,
                rules: frame.rules,
            },
            FrameKind::Sequence if !frame.children.is_empty() => NodeKind::Sequence(frame.children),
            FrameKind::Markov if !frame.children.is_empty() => NodeKind::Markov(frame.children),
            _ => return err(frame.line, RulesErrorKind::EmptyNode),
        };
        self.emit(frame.line, kind)
    }

    fn emit(&mut self, line: usize, kind: NodeKind) -> Result<(), RulesError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return err(line, RulesErrorKind::TooMany("nodes"));
        }
        let node = Node { id: 0, kind };
        match self.stack.last_mut() {
            Some(frame) => frame.children.push(node),
            None => self.top.push(node),
        }
        Ok(())
    }

    fn rule(&mut self, line: usize, words: &[&str]) -> Result<(), RulesError> {
        if words.len() < 4 || words[2] != "=" {
            return err(line, RulesErrorKind::Missing("rule IN = OUT [p=N] [sym=S]"));
        }
        let (_, symbols) = self.header(line)?;
        let (iw, ih, inp) = pattern(line, words[1], symbols)?;
        let (ow, oh, out) = pattern(line, words[3], symbols)?;
        if (iw, ih) != (ow, oh) {
            return err(
                line,
                RulesErrorKind::BadPattern("IN and OUT differ in shape"),
            );
        }
        if !inp.iter().zip(&out).any(|(&a, &b)| b != WILD && a != b) {
            return err(line, RulesErrorKind::NoChange);
        }
        let (p, sym) = rule_attrs(line, &words[4..])?;
        let variants = expand(iw, ih, &inp, &out, sym);
        let frame_rules = match self.stack.last() {
            Some(f) => f.rules.len(),
            None => 0,
        };
        self.rules += 1;
        if frame_rules >= MAX_RULES_PER_NODE {
            return err(line, RulesErrorKind::TooMany("rules in one node (64)"));
        }
        if self.rules > MAX_RULES {
            return err(line, RulesErrorKind::TooMany("rules (256)"));
        }
        if let Some(frame) = self.stack.last_mut() {
            frame.rules.push(Rule { p, variants });
        }
        Ok(())
    }
}

/// `(p, sym)` from the attribute words of a rule.
fn rule_attrs(line: usize, words: &[&str]) -> Result<(u8, Sym), RulesError> {
    let mut p = 100u8;
    let mut sym = Sym::All;
    for word in words {
        match word.split_once('=') {
            Some(("p", n)) => {
                let n = number(line, n)?;
                p = match u8::try_from(n) {
                    Ok(v @ 1..=100) => v,
                    _ => return err(line, RulesErrorKind::OutOfRange("p (1..=100)")),
                };
            }
            Some(("sym", "none")) => sym = Sym::None,
            Some(("sym", "flip")) => sym = Sym::Flip,
            Some(("sym", "rot")) => sym = Sym::Rot,
            Some(("sym", "all")) => sym = Sym::All,
            _ => return err(line, RulesErrorKind::BadAttr((*word).to_owned())),
        }
    }
    Ok((p, sym))
}

/// Parse `ABC/DEF` into `(w, h, cells)`; `*` becomes the wildcard.
fn pattern(line: usize, word: &str, symbols: &[u8]) -> Result<(usize, usize, Vec<u8>), RulesError> {
    let mut cells = Vec::new();
    let mut width = 0usize;
    let mut height = 0usize;
    for row in word.split('/') {
        if row.is_empty() {
            return err(line, RulesErrorKind::BadPattern("an empty row"));
        }
        let mut n = 0usize;
        for c in row.chars() {
            n += 1;
            if c == '*' {
                cells.push(WILD);
                continue;
            }
            let found = u8::try_from(c)
                .ok()
                .and_then(|b| symbols.iter().position(|&s| s == b));
            let Some(index) = found.and_then(|i| u8::try_from(i).ok()) else {
                return err(line, RulesErrorKind::UnknownSymbol(c));
            };
            cells.push(index);
        }
        if height == 0 {
            width = n;
        } else if n != width {
            return err(line, RulesErrorKind::BadPattern("rows differ in length"));
        }
        height += 1;
    }
    if width > MAX_PATTERN_SIDE || height > MAX_PATTERN_SIDE {
        return err(line, RulesErrorKind::BadPattern("larger than 16 by 16"));
    }
    Ok((width, height, cells))
}

// ------------------------------------------------------------- symmetry

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sym {
    None,
    Flip,
    Rot,
    All,
}

/// Rotate a `w * h` grid a quarter turn; the result is `h * w`.
fn rot90(cells: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for ny in 0..w {
        for nx in 0..h {
            out[ny * h + nx] = cells[(h - 1 - nx) * w + ny];
        }
    }
    out
}

/// Mirror every row.
fn flip(cells: &[u8], w: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(cells.len());
    for row in cells.chunks(w) {
        out.extend(row.iter().rev());
    }
    out
}

/// The distinct orientations of one rule, in a fixed order.
fn expand(w: usize, h: usize, inp: &[u8], out: &[u8], sym: Sym) -> Vec<Variant> {
    let (mirrors, turns) = match sym {
        Sym::None => (1, 1),
        Sym::Flip => (2, 1),
        Sym::Rot => (1, 4),
        Sym::All => (2, 4),
    };
    let mut variants: Vec<Variant> = Vec::new();
    for mirror in 0..mirrors {
        let (mut a, mut b) = (inp.to_vec(), out.to_vec());
        if mirror == 1 {
            a = flip(&a, w);
            b = flip(&b, w);
        }
        let (mut vw, mut vh) = (w, h);
        for _ in 0..turns {
            let variant = Variant {
                w: vw,
                h: vh,
                inp: a.clone().into_boxed_slice(),
                out: b.clone().into_boxed_slice(),
            };
            if !variants.contains(&variant) {
                variants.push(variant);
            }
            a = rot90(&a, vw, vh);
            b = rot90(&b, vw, vh);
            std::mem::swap(&mut vw, &mut vh);
        }
    }
    variants
}

/// Number the nodes in preorder; `next` ends as the node count.
fn number_nodes(node: &mut Node, next: &mut u32) {
    node.id = *next;
    *next += 1;
    if let NodeKind::Sequence(children) | NodeKind::Markov(children) = &mut node.kind {
        for child in children {
            number_nodes(child, next);
        }
    }
}

/// Split one line into statements: a `{` ends a statement and a `}` stands
/// alone, so `one { rule A = B }` is three statements.
fn statements(code: &str) -> Vec<Vec<&str>> {
    let mut all = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for word in code.split_whitespace() {
        match word {
            "{" => {
                current.push(word);
                all.push(std::mem::take(&mut current));
            }
            "}" => {
                if !current.is_empty() {
                    all.push(std::mem::take(&mut current));
                }
                all.push(vec![word]);
            }
            _ => current.push(word),
        }
    }
    if !current.is_empty() {
        all.push(current);
    }
    all
}

impl Program {
    /// Parse a rules text.
    ///
    /// Errors: [`RulesError`] with the 1-based line of the first bad statement.
    pub fn parse(text: &str) -> Result<Self, RulesError> {
        let mut parser = Parser::new();
        for (index, raw) in text.lines().enumerate() {
            let code = raw.split('#').next().unwrap_or("");
            for words in statements(code) {
                parser.statement(index + 1, &words)?;
            }
        }
        if let Some(frame) = parser.stack.last() {
            return err(frame.line, RulesErrorKind::Unclosed);
        }
        let ((w, h), symbols) = match parser.header(0) {
            Ok((size, symbols)) => (size, symbols.to_vec()),
            Err(e) => return Err(e),
        };
        if parser.top.is_empty() {
            return err(0, RulesErrorKind::NoNodes);
        }
        let (Ok(width), Ok(height)) = (u16::try_from(w), u16::try_from(h)) else {
            return err(0, RulesErrorKind::OutOfRange("size"));
        };
        let mut root = Node {
            id: 0,
            kind: NodeKind::Sequence(parser.top),
        };
        let mut next = 0u32;
        number_nodes(&mut root, &mut next);
        Ok(Self {
            width,
            height,
            symbols,
            root,
            node_count: next as usize,
            uses_all: parser.uses_all,
            uses_prl: parser.uses_prl,
        })
    }

    /// Grid width in cells.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Grid height in cells.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The alphabet, fill symbol first.
    #[must_use]
    pub fn symbols(&self) -> String {
        self.symbols.iter().map(|&b| char::from(b)).collect()
    }

    /// Run the program with `seed`. Same program and seed give the same grid.
    ///
    /// Errors: [`RunError::StepCap`] or [`RunError::WorkCap`] for a rule set that
    /// does not settle; nothing partial is returned.
    pub fn run(&self, seed: u64) -> Result<Grid, RunError> {
        let width = usize::from(self.width);
        let cells = width * usize::from(self.height);
        let mut run = Run {
            width,
            height: usize::from(self.height),
            grid: vec![0u8; cells],
            next: if self.uses_prl {
                vec![0u8; cells]
            } else {
                Vec::new()
            },
            order: if self.uses_all {
                (0..cells).collect()
            } else {
                Vec::new()
            },
            touched: if self.uses_all {
                vec![false; cells]
            } else {
                Vec::new()
            },
            state: vec![0u32; self.node_count],
            rng: Rng(seed),
            steps: 0,
            work: 0,
        };
        while run.go(&self.root)? {}
        Ok(Grid {
            width: self.width,
            height: self.height,
            symbols: self.symbols.clone(),
            cells: run.grid,
            steps: run.steps,
        })
    }

    /// Orientations stored for all rules, for tests of the symmetry expansion.
    #[cfg(test)]
    fn variant_count(&self) -> usize {
        fn walk(node: &Node) -> usize {
            match &node.kind {
                NodeKind::Rules { rules, .. } => rules.iter().map(|r| r.variants.len()).sum(),
                NodeKind::Fill(_) => 0,
                NodeKind::Sequence(c) | NodeKind::Markov(c) => c.iter().map(walk).sum(),
            }
        }
        walk(&self.root)
    }
}

// ------------------------------------------------------------------ run

/// SplitMix64 stream off the seed.
struct Rng(u64);

impl Rng {
    fn draw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        mix64(self.0)
    }

    /// Uniform in `0..n` (`n > 0`) by the multiply-high method, no modulo bias loop.
    fn below(&mut self, n: u64) -> u64 {
        let wide = u128::from(self.draw()) * u128::from(n);
        u64::try_from(wide >> 64).unwrap_or(0)
    }

    /// Does a `percent` roll pass.
    fn roll(&mut self, percent: u8) -> bool {
        percent >= 100 || self.below(100) < u64::from(percent)
    }
}

/// One run's mutable state.
struct Run {
    width: usize,
    height: usize,
    grid: Vec<u8>,
    next: Vec<u8>,
    order: Vec<usize>,
    touched: Vec<bool>,
    /// Per node: a rules node's steps taken, a fill's done flag, a sequence's cursor.
    state: Vec<u32>,
    rng: Rng,
    steps: u32,
    work: u64,
}

/// Visit every match of every orientation of `rules` in rule, orientation,
/// row, column order; `visit` returns true to stop. Returns the comparisons spent.
fn scan(
    grid: &[u8],
    width: usize,
    height: usize,
    rules: &[Rule],
    mut visit: impl FnMut(usize, usize, usize, usize) -> bool,
) -> u64 {
    let mut work = 0u64;
    for (ri, rule) in rules.iter().enumerate() {
        for (vi, v) in rule.variants.iter().enumerate() {
            if v.w > width || v.h > height {
                continue;
            }
            for y in 0..=height - v.h {
                for x in 0..=width - v.w {
                    let (hit, cost) = v.matches(grid, width, x, y);
                    work += cost;
                    if hit && visit(ri, vi, x, y) {
                        return work;
                    }
                }
            }
        }
    }
    work
}

impl Run {
    fn spend(&mut self, work: u64) -> Result<(), RunError> {
        self.work += work;
        if self.work > MAX_WORK {
            return Err(RunError::WorkCap);
        }
        Ok(())
    }

    fn count_step(&mut self) -> Result<(), RunError> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(RunError::StepCap);
        }
        Ok(())
    }

    /// Zero the counters of a node and everything under it.
    fn reset(&mut self, node: &Node) {
        self.state[node.id as usize] = 0;
        if let NodeKind::Sequence(children) | NodeKind::Markov(children) = &node.kind {
            for child in children {
                self.reset(child);
            }
        }
    }

    /// Take one step of `node`; false when it has nothing left to do.
    fn go(&mut self, node: &Node) -> Result<bool, RunError> {
        let id = node.id as usize;
        match &node.kind {
            NodeKind::Rules { mode, steps, rules } => {
                if *steps > 0 && self.state[id] >= *steps {
                    return Ok(false);
                }
                let did = match mode {
                    Mode::One => self.step_one(rules)?,
                    Mode::All => self.step_all(rules)?,
                    Mode::Prl => self.step_prl(rules)?,
                };
                if did {
                    self.state[id] += 1;
                    self.count_step()?;
                }
                Ok(did)
            }
            NodeKind::Fill(f) => {
                if self.state[id] > 0 {
                    return Ok(false);
                }
                self.state[id] = 1;
                for y in f.y..f.y + f.h {
                    self.grid[y * self.width + f.x..y * self.width + f.x + f.w].fill(f.sym);
                }
                self.count_step()?;
                Ok(true)
            }
            NodeKind::Sequence(children) => {
                while let Some(child) = children.get(self.state[id] as usize) {
                    if self.go(child)? {
                        return Ok(true);
                    }
                    self.state[id] += 1;
                }
                self.reset(node);
                Ok(false)
            }
            NodeKind::Markov(children) => {
                for child in children {
                    if self.go(child)? {
                        return Ok(true);
                    }
                }
                self.reset(node);
                Ok(false)
            }
        }
    }

    /// `one`: count the matches, draw one uniformly, find it, roll `p`, apply.
    fn step_one(&mut self, rules: &[Rule]) -> Result<bool, RunError> {
        let mut total = 0u64;
        let work = scan(&self.grid, self.width, self.height, rules, |_, _, _, _| {
            total += 1;
            false
        });
        self.spend(work)?;
        if total == 0 {
            return Ok(false);
        }
        let pick = self.rng.below(total);
        let mut seen = 0u64;
        let mut found = (0, 0, 0, 0);
        let work = scan(
            &self.grid,
            self.width,
            self.height,
            rules,
            |ri, vi, x, y| {
                if seen == pick {
                    found = (ri, vi, x, y);
                    return true;
                }
                seen += 1;
                false
            },
        );
        self.spend(work)?;
        let (ri, vi, x, y) = found;
        if self.rng.roll(rules[ri].p) {
            rules[ri].variants[vi].write(&mut self.grid, self.width, x, y);
        }
        Ok(true)
    }

    /// `all`: shuffle the cells, apply each match whose footprint is untouched.
    fn step_all(&mut self, rules: &[Rule]) -> Result<bool, RunError> {
        for i in (1..self.order.len()).rev() {
            let j = usize::try_from(self.rng.below(i as u64 + 1)).unwrap_or(0);
            self.order.swap(i, j);
        }
        self.touched.fill(false);
        let (width, height) = (self.width, self.height);
        let mut candidates = false;
        let mut work = 0u64;
        for oi in 0..self.order.len() {
            let (x, y) = (self.order[oi] % width, self.order[oi] / width);
            'cell: for rule in rules {
                for v in &rule.variants {
                    if x + v.w > width || y + v.h > height {
                        continue;
                    }
                    let (hit, cost) = v.matches(&self.grid, width, x, y);
                    work += cost;
                    if !hit || footprint_touched(&self.touched, width, v, x, y) {
                        continue;
                    }
                    candidates = true;
                    if self.rng.roll(rule.p) {
                        v.write(&mut self.grid, width, x, y);
                        mark_touched(&mut self.touched, width, v, x, y);
                        break 'cell;
                    }
                }
            }
        }
        self.spend(work)?;
        Ok(candidates)
    }

    /// `prl`: every match of the round's starting grid writes into a copy.
    fn step_prl(&mut self, rules: &[Rule]) -> Result<bool, RunError> {
        self.next.copy_from_slice(&self.grid);
        let mut candidates = false;
        let (rng, next, width) = (&mut self.rng, &mut self.next, self.width);
        let work = scan(
            &self.grid,
            self.width,
            self.height,
            rules,
            |ri, vi, x, y| {
                candidates = true;
                if rng.roll(rules[ri].p) {
                    rules[ri].variants[vi].write(next, width, x, y);
                }
                false
            },
        );
        self.spend(work)?;
        if candidates {
            std::mem::swap(&mut self.grid, &mut self.next);
        }
        Ok(candidates)
    }
}

fn footprint_touched(touched: &[bool], width: usize, v: &Variant, x: usize, y: usize) -> bool {
    (0..v.h).any(|dy| {
        touched[(y + dy) * width + x..(y + dy) * width + x + v.w]
            .iter()
            .any(|&t| t)
    })
}

fn mark_touched(touched: &mut [bool], width: usize, v: &Variant, x: usize, y: usize) {
    for dy in 0..v.h {
        touched[(y + dy) * width + x..(y + dy) * width + x + v.w].fill(true);
    }
}

#[cfg(test)]
mod tests;
