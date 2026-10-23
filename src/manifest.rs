//! The project manifest `game.toml`: the flat file that says what a game project is.
//!
//! It is the one input of `engine2040 run` and `engine2040 build`. The format is a strict
//! subset of TOML: `key = value` lines, `#` comments, quoted strings and one-line lists of
//! quoted strings, no tables. Every refusal names the key and the line so the file can be
//! fixed without guessing. Pure over the text: reading files is `project::load_project`.

use engine_render::manifest::QualityTier;
use std::fmt;
use std::path::{Component, Path};

/// Longest project name, in bytes.
const MAX_NAME_BYTES: usize = 64;
/// Most entries in one list.
const MAX_LIST: usize = 16;
/// Longest piece of the file echoed back in an error, in characters.
const ECHO_CHARS: usize = 40;

/// Every key `game.toml` knows; any other key is refused.
const KEYS: [&str; 8] = [
    "name",
    "targets",
    "quality",
    "physics_layers",
    "gore",
    "content",
    "seed_file",
    "game",
];

/// Keys that parse (so a bad value is still named with its line) but reach no build or run:
/// `build` and `run` refuse a file that sets one, rather than show a setting that does nothing.
const UNWIRED: [(&str, &str); 4] = [
    (
        "quality",
        "the build and the run use the engine's own tier (medium)",
    ),
    ("gore", "the run uses the engine's own gore setting (full)"),
    (
        "physics_layers",
        "the run always uses the authority and physical layers",
    ),
    (
        "content",
        "the run reads the engine's own content/ tree, not project content roots",
    ),
];

/// What the engine does instead of the key's value, when the key is not wired.
pub fn unwired_note(key: &str) -> Option<&'static str> {
    UNWIRED
        .iter()
        .find(|(known, _)| *known == key)
        .map(|(_, note)| *note)
}

/// A closed set of words a key may take; the error for a bad word lists them all.
pub trait Choice: Copy + PartialEq + 'static {
    /// Every value, in the order the docs list them.
    const WORDS: &'static [Self];
    /// The word written in `game.toml`.
    fn word(self) -> &'static str;
}

/// The word's value, if it is one of `T::WORDS`.
pub fn lookup<T: Choice>(word: &str) -> Option<T> {
    T::WORDS.iter().copied().find(|value| value.word() == word)
}

/// The words of `values` in `game.toml` spelling, one `separator` apart.
pub fn joined<T: Choice>(values: &[T], separator: &str) -> String {
    let words: Vec<&str> = values.iter().map(|value| value.word()).collect();
    words.join(separator)
}

/// `T::WORDS` as one comma-separated list for error messages.
pub fn valid_words<T: Choice>() -> String {
    joined(T::WORDS, ", ")
}

/// A platform the game builds for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The native game exe.
    Windows,
    /// The browser build (WebGPU plus WebAssembly).
    Web,
}

impl Choice for Target {
    const WORDS: &'static [Self] = &[Self::Windows, Self::Web];

    fn word(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Web => "web",
        }
    }
}

impl Choice for QualityTier {
    const WORDS: &'static [Self] = Self::ALL;

    fn word(self) -> &'static str {
        self.name()
    }
}

/// How much blood and severing the presentation shows; never feeds the hashed simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gore {
    /// No blood: impact flashes only.
    Off,
    /// Blood without severed parts.
    Reduced,
    /// Blood and severed parts.
    Full,
}

impl Choice for Gore {
    const WORDS: &'static [Self] = &[Self::Off, Self::Reduced, Self::Full];

    fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Reduced => "reduced",
            Self::Full => "full",
        }
    }
}

/// One of the two physics layers a game can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Hashed integers; decides every game outcome.
    Authority,
    /// Floats on each client; ragdolls, limbs and debris that only have to look right.
    Physical,
}

impl Choice for Layer {
    const WORDS: &'static [Self] = &[Self::Authority, Self::Physical];

    fn word(self) -> &'static str {
        match self {
            Self::Authority => "authority",
            Self::Physical => "physical",
        }
    }
}

/// The game a project builds and runs: one bin of one workspace package.
///
/// `game.toml` writes it as `game = "<package>:<bin>"`. The shape is checked here; that the
/// package is a workspace member and that the bin is one of its bins is checked by
/// `project::Project::load_in`, the one place that reads the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    /// Workspace package name, e.g. `engine-game`.
    pub package: String,
    /// A bin that package declares, e.g. `arena`.
    pub bin: String,
}

impl Default for Game {
    /// The engine's own arena: what every project built before `game` existed.
    fn default() -> Self {
        Self {
            package: "engine-game".to_owned(),
            bin: "arena".to_owned(),
        }
    }
}

/// A parsed `game.toml`. Every field passed its check: names are plain, paths are relative
/// and stay inside the project, lists have no repeats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Project name (`check_name`).
    pub name: String,
    /// Platforms the project builds for; never empty.
    pub targets: Vec<Target>,
    /// Quality tier.
    pub quality: QualityTier,
    /// Physics layers in use; always contains [`Layer::Authority`].
    pub physics_layers: Vec<Layer>,
    /// Gore level.
    pub gore: Gore,
    /// Content roots, relative to the manifest, forward slashes.
    pub content: Vec<String>,
    /// File holding the run seed, relative to the manifest; the default seed when absent.
    pub seed_file: Option<String>,
    /// The game this project builds and runs; the engine's arena when the file names none.
    pub game: Game,
    /// Every key the file set, with its line, in file order; the rest took their defaults.
    keys: Vec<(&'static str, usize)>,
}

/// A refused manifest: which key, on which line, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct ManifestError {
    /// 1-based line of the offending row; 0 when a required key is missing.
    pub line: usize,
    /// The key (or the start of an unreadable row).
    pub key: String,
    /// What is wrong and what is allowed.
    pub reason: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "key `{}`: {}", self.key, self.reason)
        } else {
            write!(f, "line {}, key `{}`: {}", self.line, self.key, self.reason)
        }
    }
}

impl std::error::Error for ManifestError {}

fn fail(line: usize, key: &str, reason: impl Into<String>) -> ManifestError {
    ManifestError {
        line,
        key: key.to_owned(),
        reason: reason.into(),
    }
}

/// The start of `text` for an error message.
fn echo(text: &str) -> String {
    let mut shown: String = text.chars().take(ECHO_CHARS).collect();
    if text.chars().nth(ECHO_CHARS).is_some() {
        shown.push_str("...");
    }
    shown
}

enum Value {
    Text(String),
    List(Vec<String>),
}

struct Row {
    line: usize,
    key: &'static str,
    value: Value,
}

impl Row {
    fn err(&self, reason: impl Into<String>) -> ManifestError {
        fail(self.line, self.key, reason)
    }

    fn text(&self) -> Result<&str, ManifestError> {
        match &self.value {
            Value::Text(text) => Ok(text),
            Value::List(_) => Err(self.err("expected one \"quoted string\", not a list")),
        }
    }

    fn list(&self) -> Result<&[String], ManifestError> {
        match &self.value {
            Value::List(items) if items.len() <= MAX_LIST => Ok(items),
            Value::List(_) => Err(self.err(format!("at most {MAX_LIST} entries"))),
            Value::Text(_) => Err(self.err("expected a list like [\"one\", \"two\"]")),
        }
    }

    /// One word out of `T::WORDS`.
    fn pick<T: Choice>(&self) -> Result<T, ManifestError> {
        let word = self.text()?;
        self.pick_word(word)
    }

    fn pick_word<T: Choice>(&self, word: &str) -> Result<T, ManifestError> {
        lookup::<T>(word).ok_or_else(|| {
            self.err(format!(
                "unknown value `{}`; valid values: {}",
                echo(word),
                valid_words::<T>()
            ))
        })
    }

    /// A non-empty list of distinct words out of `T::WORDS`.
    fn pick_many<T: Choice>(&self) -> Result<Vec<T>, ManifestError> {
        let items = self.list()?;
        if items.is_empty() {
            return Err(self.err(format!("empty list; valid values: {}", valid_words::<T>())));
        }
        let mut picked: Vec<T> = Vec::with_capacity(items.len());
        for word in items {
            let value = self.pick_word::<T>(word)?;
            if picked.contains(&value) {
                return Err(self.err(format!("`{word}` is listed twice")));
            }
            picked.push(value);
        }
        Ok(picked)
    }

    /// A relative path that stays inside the project.
    fn path(&self, path: &str) -> Result<String, ManifestError> {
        check_relative(path).map_err(|why| self.err(format!("`{}`: {why}", echo(path))))?;
        Ok(path.to_owned())
    }
}

/// Cursor over the value text of one row.
struct Scanner<'a> {
    rest: &'a str,
}

impl<'a> Scanner<'a> {
    fn skip_space(&mut self) {
        self.rest = self.rest.trim_start();
    }

    /// A basic string: `"..."`, with `\"` and `\\` as the only escapes.
    fn string(&mut self) -> Result<String, String> {
        let text: &'a str = self.rest;
        let mut chars = text.char_indices();
        chars.next(); // the opening quote
        let mut out = String::new();
        while let Some((at, ch)) = chars.next() {
            match ch {
                '"' => {
                    self.rest = &text[at + 1..];
                    return Ok(out);
                }
                '\\' => match chars.next() {
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    _ => {
                        return Err("only the escapes \\\" and \\\\ exist in strings \
                                    (write paths with forward slashes)"
                            .to_owned());
                    }
                },
                _ => out.push(ch),
            }
        }
        Err("string is missing its closing quote".to_owned())
    }

    /// `["one", "two"]` on one line; a trailing comma is fine.
    fn list(&mut self) -> Result<Vec<String>, String> {
        self.rest = &self.rest[1..]; // the opening bracket
        let mut items = Vec::new();
        loop {
            self.skip_space();
            if let Some(after) = self.rest.strip_prefix(']') {
                self.rest = after;
                return Ok(items);
            }
            if !self.rest.starts_with('"') {
                return Err(if self.rest.is_empty() {
                    "list is missing its closing `]` (a list stays on one line)".to_owned()
                } else {
                    format!(
                        "list items are \"quoted strings\", got `{}`",
                        echo(self.rest)
                    )
                });
            }
            items.push(self.string()?);
            self.skip_space();
            match self.rest.chars().next() {
                Some(',') => self.rest = &self.rest[1..],
                Some(']') => {}
                Some(_) => {
                    return Err(format!(
                        "expected `,` or `]` after a list item, got `{}`",
                        echo(self.rest)
                    ));
                }
                None => {
                    return Err(
                        "list is missing its closing `]` (a list stays on one line)".to_owned()
                    );
                }
            }
        }
    }
}

/// The value after `=`: a string or a one-line list, then at most a `# comment`.
fn parse_value(text: &str) -> Result<Value, String> {
    let mut scan = Scanner { rest: text };
    let value = if text.starts_with('"') {
        Value::Text(scan.string()?)
    } else if text.starts_with('[') {
        Value::List(scan.list()?)
    } else {
        return Err(format!(
            "expected a \"quoted string\" or a [\"list\", \"of\", \"strings\"], got `{}`",
            echo(text)
        ));
    };
    scan.skip_space();
    if scan.rest.is_empty() || scan.rest.starts_with('#') {
        Ok(value)
    } else {
        Err(format!("unexpected `{}` after the value", echo(scan.rest)))
    }
}

/// Every `key = value` row, each key known and given once.
fn read_rows(text: &str) -> Result<Vec<Row>, ManifestError> {
    let mut rows: Vec<Row> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if body.starts_with('[') {
            let reason = "tables are not supported: game.toml is flat `key = value` lines";
            return Err(fail(line, &echo(body), reason));
        }
        let Some((key_text, value_text)) = body.split_once('=') else {
            return Err(fail(line, &echo(body), "expected `key = value`"));
        };
        let key_text = key_text.trim();
        let Some(&key) = KEYS.iter().find(|known| **known == key_text) else {
            let reason = format!("unknown key; valid keys: {}", KEYS.join(", "));
            return Err(fail(line, &echo(key_text), reason));
        };
        if let Some(first) = rows.iter().find(|row| row.key == key) {
            let reason = format!("given twice (first on line {})", first.line);
            return Err(fail(line, key, reason));
        }
        let value = parse_value(value_text.trim()).map_err(|reason| fail(line, key, reason))?;
        rows.push(Row { line, key, value });
    }
    Ok(rows)
}

fn take(rows: &mut Vec<Row>, key: &str) -> Option<Row> {
    let at = rows.iter().position(|row| row.key == key)?;
    Some(rows.swap_remove(at))
}

fn require(rows: &mut Vec<Row>, key: &str) -> Result<Row, ManifestError> {
    take(rows, key).ok_or_else(|| fail(0, key, "required but missing"))
}

/// A project name: 1 to 64 of a-z, 0-9, `-` and `_`, starting with a letter or digit.
pub fn check_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err("a name is 1 to 64 characters");
    }
    let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !name.starts_with(plain) || !name.chars().all(|c| plain(c) || c == '-' || c == '_') {
        return Err("a name uses a-z, 0-9, '-' and '_' and starts with a letter or digit");
    }
    Ok(())
}

/// The `game` value: `"<package>:<bin>"`, both halves a [`check_name`] name. Whether the
/// package and bin exist is the workspace check in `project::Project::load_in`.
fn parse_game(row: &Row) -> Result<Game, ManifestError> {
    let word = row.text()?;
    if word.matches(':').count() != 1 {
        return Err(row.err(format!(
            "expected one \"workspace-package:bin\", got `{}`; for example \"engine-game:arena\"",
            echo(word)
        )));
    }
    let (package, bin) = word.split_once(':').ok_or_else(|| {
        row.err(format!(
            "expected one \"workspace-package:bin\", got `{}`; for example \"engine-game:arena\"",
            echo(word)
        ))
    })?;
    check_name(package).map_err(|why| row.err(format!("package `{}`: {why}", echo(package))))?;
    check_name(bin).map_err(|why| row.err(format!("bin `{}`: {why}", echo(bin))))?;
    Ok(Game {
        package: package.to_owned(),
        bin: bin.to_owned(),
    })
}

/// A path inside the project: forward slashes, no `..`, no drive, no leading `./` or `/`.
fn check_relative(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("the path is empty");
    }
    if path.contains(['\\', ':']) || path.chars().any(char::is_control) {
        return Err("write paths relative to the manifest with forward slashes, no drive letter");
    }
    if !Path::new(path)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("the path must stay inside the project (no `..`, `.` or leading `/`)");
    }
    Ok(())
}

impl Manifest {
    /// Parse the text of a `game.toml`.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut rows = read_rows(text)?;
        let keys: Vec<(&'static str, usize)> = rows.iter().map(|row| (row.key, row.line)).collect();

        let row = require(&mut rows, "name")?;
        let name = row.text()?.to_owned();
        check_name(&name).map_err(|why| row.err(format!("`{}`: {why}", echo(&name))))?;

        let targets = require(&mut rows, "targets")?.pick_many::<Target>()?;

        let quality = match take(&mut rows, "quality") {
            Some(row) => row.pick::<QualityTier>()?,
            None => QualityTier::Medium,
        };

        let physics_layers = match take(&mut rows, "physics_layers") {
            Some(row) => {
                let layers = row.pick_many::<Layer>()?;
                if !layers.contains(&Layer::Authority) {
                    return Err(row.err(
                        "must include \"authority\": the hashed layer decides every game outcome",
                    ));
                }
                layers
            }
            None => vec![Layer::Authority, Layer::Physical],
        };

        let gore = match take(&mut rows, "gore") {
            Some(row) => row.pick::<Gore>()?,
            None => Gore::Full,
        };

        let mut content: Vec<String> = Vec::new();
        if let Some(row) = take(&mut rows, "content") {
            for path in row.list()? {
                if content.contains(path) {
                    return Err(row.err(format!("`{}` is listed twice", echo(path))));
                }
                content.push(row.path(path)?);
            }
        }

        let seed_file = match take(&mut rows, "seed_file") {
            Some(row) => Some(row.path(row.text()?)?),
            None => None,
        };

        let game = match take(&mut rows, "game") {
            Some(row) => parse_game(&row)?,
            None => Game::default(),
        };

        Ok(Self {
            name,
            targets,
            quality,
            physics_layers,
            gore,
            content,
            seed_file,
            game,
            keys,
        })
    }

    /// Whether the file set `key`; the others took their defaults.
    pub fn sets(&self, key: &str) -> bool {
        self.keys.iter().any(|(set, _)| *set == key)
    }

    /// Every key the file sets that no build or run reads yet, with its line, in file order.
    pub fn unwired(&self) -> Vec<(&'static str, usize)> {
        self.keys
            .iter()
            .copied()
            .filter(|(key, _)| unwired_note(key).is_some())
            .collect()
    }

    /// One line naming what the project is, for the run receipt.
    pub fn summary(&self) -> String {
        format!(
            "{} quality={} gore={} layers={} targets={}",
            self.name,
            self.quality.word(),
            self.gore.word(),
            joined(&self.physics_layers, ","),
            joined(&self.targets, ",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "\
# a project
name = \"duel-1\"   # trailing comment
targets = [\"windows\", \"web\",]
quality = \"high\"
physics_layers = [\"authority\"]
gore = \"off\"
content = [\"content\", \"art/sets\"]
seed_file = \"content/seed.txt\"
";

    fn refused(text: &str) -> ManifestError {
        Manifest::parse(text).expect_err("manifest must be refused")
    }

    #[test]
    fn full_manifest_parses_every_key() {
        let manifest = Manifest::parse(FULL).unwrap();
        assert_eq!(manifest.name, "duel-1");
        assert_eq!(manifest.targets, [Target::Windows, Target::Web]);
        assert_eq!(manifest.quality, QualityTier::High);
        assert_eq!(manifest.physics_layers, [Layer::Authority]);
        assert_eq!(manifest.gore, Gore::Off);
        assert_eq!(manifest.content, ["content", "art/sets"]);
        assert_eq!(manifest.seed_file.as_deref(), Some("content/seed.txt"));
        assert_eq!(
            manifest.summary(),
            "duel-1 quality=high gore=off layers=authority targets=windows,web"
        );
    }

    #[test]
    fn minimal_manifest_takes_defaults_and_survives_bom_and_crlf() {
        let text = "\u{feff}name = \"a\"\r\ntargets = [\"web\"]\r\n";
        let manifest = Manifest::parse(text).unwrap();
        assert_eq!(manifest.quality, QualityTier::Medium);
        assert_eq!(manifest.gore, Gore::Full);
        assert_eq!(manifest.physics_layers, [Layer::Authority, Layer::Physical]);
        assert!(manifest.content.is_empty());
        assert_eq!(manifest.seed_file, None);
    }

    #[test]
    fn game_key_names_a_package_and_bin_and_defaults_to_the_arena() {
        let manifest = Manifest::parse(FULL).unwrap();
        assert_eq!(manifest.game, Game::default());
        assert!(!manifest.sets("game"));
        assert!(manifest.sets("content"));

        let text = "name = \"a\"\ntargets = [\"web\"]\ngame = \"engine-endpoint:e2040-mcp\"\n";
        let manifest = Manifest::parse(text).unwrap();
        assert_eq!(
            manifest.game,
            Game {
                package: "engine-endpoint".to_owned(),
                bin: "e2040-mcp".to_owned(),
            }
        );
        assert!(manifest.sets("game"));
    }

    #[test]
    fn bad_game_shape_names_the_line_and_the_form() {
        let cases: [(&str, &str); 7] = [
            (
                "game = \"arena\"\n",
                "expected one \"workspace-package:bin\"",
            ),
            (
                "game = \"a:b:c\"\n",
                "expected one \"workspace-package:bin\"",
            ),
            ("game = \"engine-game:\"\n", "bin ``: a name is 1 to 64"),
            ("game = \":arena\"\n", "package ``: a name is 1 to 64"),
            ("game = \"Engine-Game:arena\"\n", "package `Engine-Game`"),
            ("game = \"engine-game:arena \"\n", "bin `arena `"),
            (
                "game = [\"engine-game:arena\"]\n",
                "expected one \"quoted string\"",
            ),
        ];
        for (row, reason) in cases {
            let text = format!("name = \"a\"\ntargets = [\"web\"]\n{row}");
            let error = refused(&text);
            assert_eq!(
                (error.line, error.key.as_str()),
                (3, "game"),
                "{row}: {error}"
            );
            assert!(error.reason.contains(reason), "{row}: {error}");
        }
    }

    #[test]
    fn unwired_keys_keep_file_order_with_their_lines() {
        let text = "name = \"a\"\ntargets = [\"web\"]\ncontent = [\"content\"]\ngore = \"off\"\n";
        let manifest = Manifest::parse(text).unwrap();
        assert_eq!(manifest.unwired(), [("content", 3), ("gore", 4)]);
        assert!(Manifest::parse("name = \"a\"\ntargets = [\"web\"]\n")
            .unwrap()
            .unwired()
            .is_empty());
        for key in ["quality", "gore", "physics_layers", "content"] {
            assert!(unwired_note(key).is_some(), "{key}");
        }
        assert!(unwired_note("name").is_none());
        assert!(unwired_note("game").is_none());
    }

    #[test]
    fn bad_manifest_names_key_and_line() {
        let cases: [(&str, usize, &str, &str); 12] = [
            (
                "name = \"a\"\ntargets = [\"web\"]\nqualty = \"low\"\n",
                3,
                "qualty",
                "valid keys: name, targets, quality",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\nquality = \"ultra2\"\n",
                3,
                "quality",
                "valid values: minimal, low, medium, high, ultra",
            ),
            (
                "name = \"a\"\n\ntargets = [\"windows\", \"linux\"]\n",
                3,
                "targets",
                "valid values: windows, web",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\nname = \"b\"\n",
                3,
                "name",
                "first on line 1",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\n[extra]\n",
                3,
                "[extra]",
                "tables are not supported",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"\n",
                2,
                "targets",
                "missing its closing `]`",
            ),
            (
                "name = \"Bad Name\"\ntargets = [\"web\"]\n",
                1,
                "name",
                "a-z, 0-9",
            ),
            (
                "name = \"a\"\ntargets = \"web\"\n",
                2,
                "targets",
                "expected a list",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\nphysics_layers = [\"physical\"]\n",
                3,
                "physics_layers",
                "must include \"authority\"",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\ncontent = [\"ok\", \"../x\"]\n",
                3,
                "content",
                "stay inside the project",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\nseed_file = \"C:/seed.txt\"\n",
                3,
                "seed_file",
                "no drive letter",
            ),
            (
                "name = \"a\"\ntargets = [\"web\"]\ngore = full\n",
                3,
                "gore",
                "expected a \"quoted string\"",
            ),
        ];
        for (text, line, key, reason) in cases {
            let error = refused(text);
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert_eq!(error.key, key, "{text:?}: {error}");
            assert!(error.reason.contains(reason), "{text:?}: {error}");
            let shown = error.to_string();
            assert!(shown.contains(&format!("line {line}")), "{shown}");
            assert!(shown.contains(&format!("key `{key}`")), "{shown}");
        }
    }

    #[test]
    fn missing_required_key_has_no_line() {
        let error = refused("targets = [\"web\"]\n");
        assert_eq!((error.line, error.key.as_str()), (0, "name"));
        assert_eq!(error.to_string(), "key `name`: required but missing");
        assert_eq!(refused("name = \"a\"\n").key, "targets");
    }

    #[test]
    fn strings_take_two_escapes_and_lists_stay_short() {
        let error = refused("name = \"a\\n\"\ntargets = [\"web\"]\n");
        assert!(error.reason.contains("only the escapes"), "{error}");
        let many: Vec<String> = (0..=MAX_LIST).map(|i| format!("\"p{i}\"")).collect();
        let text = format!(
            "name = \"a\"\ntargets = [\"web\"]\ncontent = [{}]\n",
            many.join(", ")
        );
        assert!(refused(&text).reason.contains("at most 16"));
    }

    #[test]
    fn name_rule() {
        for good in ["a", "arena-duel", "x_1", "9lives"] {
            assert_eq!(check_name(good), Ok(()), "{good}");
        }
        for bad in ["", "-a", "_a", "A", "a b", "a.b", &"a".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
