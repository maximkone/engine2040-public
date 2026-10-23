//! Menu screens as data: a project's `.ui.toml` names the same rows the Rust builders in
//! [`menus`] name, in the same order, so an order edit is a file edit.
//!
//! Shape (one table per screen, rows are the locale keys the Rust builders already carry):
//!
//! ```toml
//! [[screen]]
//! name = "main"
//! rows = ["start-fight", "settings", "quit"]
//! ```
//!
//! Consumer: whoever builds the screens at boot; [`MenuLayout::items`] returns the same
//! [`MenuItem`] values [`crate::menus::main_items_locale`] returns, labels resolved per locale.
//! Failure mode: [`LayoutError`] names the line and the legal values; parsing never panics.

use crate::menus::locale;
use crate::menus::{localized_label, MenuItem, MenuItemId};

/// One screen a layout file may name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ScreenName {
    /// Top-level menu: start, settings, quit.
    Main,
    /// In-fight menu: resume, settings, quit to main.
    Pause,
}

impl ScreenName {
    /// Every screen a file may name, in the order the layout file lists them.
    pub const ALL: [Self; 2] = [Self::Main, Self::Pause];

    /// The name a layout file writes for this screen.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Pause => "pause",
        }
    }

    /// The screen `name` writes, when a file may name it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|screen| screen.as_str() == name)
    }
}

/// One screen's rows in draw order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LayoutScreen {
    /// Which screen these rows belong to.
    pub name: ScreenName,
    /// The rows, top to bottom.
    pub rows: Vec<MenuItemId>,
}

/// Every screen one layout file declares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MenuLayout {
    screens: Vec<LayoutScreen>,
}

/// Why a layout file was refused: one line, one cause, and what the file may say instead.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LayoutError {
    /// The file declares no screen at all.
    Empty,
    /// `name = "..."` names a screen this engine does not have.
    UnknownScreen { name: String, line: usize },
    /// A `rows` entry names no menu row.
    UnknownRow { key: String, line: usize },
    /// The same screen is listed twice, so one table would silently win.
    DuplicateScreen { name: &'static str, line: usize },
    /// A `[[screen]]` table has no `name` line.
    MissingName { line: usize },
    /// A screen lists no rows, so it could never draw.
    MissingRows { name: &'static str, line: usize },
    /// A line is not one of the three forms a layout file knows.
    NotALayoutLine { line: usize, text: String },
}

impl core::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "the layout file declares no screen; add [[screen]] with a name and rows"
            ),
            Self::UnknownScreen { name, line } => write!(
                f,
                "line {line}: screen `{name}` is not one of {}",
                screen_names()
            ),
            Self::UnknownRow { key, line } => write!(
                f,
                "line {line}: row `{key}` is not a menu row; the rows are {}",
                row_keys()
            ),
            Self::DuplicateScreen { name, line } => {
                write!(f, "line {line}: screen `{name}` is listed twice")
            }
            Self::MissingName { line } => {
                write!(
                    f,
                    "line {line}: a [[screen]] needs name = \"main\" or \"pause\""
                )
            }
            Self::MissingRows { name, line } => write!(
                f,
                "line {line}: screen `{name}` lists no rows; rows = [\"start-fight\", ...]"
            ),
            Self::NotALayoutLine { line, text } => write!(
                f,
                "line {line}: `{text}` is not a layout line; the forms are [[screen]], \
                 name = \"main\" and rows = [\"start-fight\"]"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl MenuLayout {
    /// Parse the `.ui.toml` subset a layout file uses: one `[[screen]]` table per screen, a
    /// `name` and a `rows` list of locale keys. Whole-line comments start with `#`.
    pub fn parse(text: &str) -> Result<Self, LayoutError> {
        let mut screens: Vec<LayoutScreen> = Vec::new();
        let mut open = Open::default();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let text = strip_comment(raw).trim();
            if text.is_empty() {
                continue;
            }
            if text == "[[screen]]" {
                open.flush(&mut screens)?;
                open = Open {
                    start: Some(line),
                    ..Open::default()
                };
                continue;
            }
            if open.start.is_none() {
                return Err(LayoutError::NotALayoutLine {
                    line,
                    text: text.to_owned(),
                });
            }
            if let Some(value) = text.strip_prefix("name") {
                let value = value.trim_start().strip_prefix('=').map(str::trim);
                let Some(value) = value else {
                    return Err(LayoutError::NotALayoutLine {
                        line,
                        text: text.to_owned(),
                    });
                };
                let name = unquote(value);
                let Some(screen) = ScreenName::from_name(name) else {
                    return Err(LayoutError::UnknownScreen {
                        name: name.to_owned(),
                        line,
                    });
                };
                open.name = Some((screen, line));
                continue;
            }
            if let Some(value) = text.strip_prefix("rows") {
                let value = value.trim_start().strip_prefix('=').map(str::trim);
                let Some(value) = value else {
                    return Err(LayoutError::NotALayoutLine {
                        line,
                        text: text.to_owned(),
                    });
                };
                open.rows = Some((parse_rows(value, line)?, line));
                continue;
            }
            return Err(LayoutError::NotALayoutLine {
                line,
                text: text.to_owned(),
            });
        }
        open.flush(&mut screens)?;
        if screens.is_empty() {
            return Err(LayoutError::Empty);
        }
        Ok(Self { screens })
    }

    /// The screen `name` declares, when the file lists it.
    #[must_use]
    pub fn screen(&self, name: ScreenName) -> Option<&LayoutScreen> {
        self.screens.iter().find(|screen| screen.name == name)
    }

    /// The drawable rows of one screen, labels resolved from `locale`. Empty when the file
    /// does not list that screen.
    #[must_use]
    pub fn items(&self, name: ScreenName, locale: locale::LocaleId) -> Vec<MenuItem> {
        self.screen(name).map_or_else(Vec::new, |screen| {
            screen
                .rows
                .iter()
                .map(|id| MenuItem {
                    id: *id,
                    label: localized_label(*id, locale),
                    enabled: true,
                })
                .collect()
        })
    }
}

/// One `[[screen]]` table while it is being read.
#[derive(Default)]
struct Open {
    start: Option<usize>,
    name: Option<(ScreenName, usize)>,
    rows: Option<(Vec<MenuItemId>, usize)>,
}

impl Open {
    /// End the open table: a named screen with at least one row goes into `screens`.
    fn flush(&mut self, screens: &mut Vec<LayoutScreen>) -> Result<(), LayoutError> {
        let Some(start) = self.start.take() else {
            return Ok(());
        };
        let Some((name, name_line)) = self.name.take() else {
            return Err(LayoutError::MissingName { line: start });
        };
        let Some((rows, _)) = self.rows.take() else {
            return Err(LayoutError::MissingRows {
                name: name.as_str(),
                line: name_line,
            });
        };
        if rows.is_empty() {
            return Err(LayoutError::MissingRows {
                name: name.as_str(),
                line: name_line,
            });
        }
        if screens.iter().any(|screen| screen.name == name) {
            return Err(LayoutError::DuplicateScreen {
                name: name.as_str(),
                line: name_line,
            });
        }
        screens.push(LayoutScreen { name, rows });
        Ok(())
    }
}

/// Cut a whole-line comment; a `#` inside quotes is part of the value.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (index, byte) in line.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'#' if !quoted => return &line[..index],
            _ => {}
        }
    }
    line
}

/// The value between one pair of double quotes, or the text as it is.
fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(value)
}

/// `["start-fight", "settings"]` as menu rows, refusing a key no row carries.
fn parse_rows(value: &str, line: usize) -> Result<Vec<MenuItemId>, LayoutError> {
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    else {
        return Err(LayoutError::NotALayoutLine {
            line,
            text: value.to_owned(),
        });
    };
    let mut rows = Vec::new();
    for part in inner.split(',') {
        let key = unquote(part.trim());
        if key.is_empty() {
            continue;
        }
        let Some(id) = MenuItemId::from_key(key) else {
            return Err(LayoutError::UnknownRow {
                key: key.to_owned(),
                line,
            });
        };
        rows.push(id);
    }
    Ok(rows)
}

/// `main, pause`, for an error that names the legal screens.
fn screen_names() -> String {
    ScreenName::ALL
        .iter()
        .map(|screen| screen.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The locale keys a layout row may name, in the order the menus use them.
fn row_keys() -> String {
    let keys = [
        locale::KEY_START_FIGHT,
        locale::KEY_SETTINGS,
        locale::KEY_QUIT,
        locale::KEY_RESUME,
        locale::KEY_QUIT_TO_MAIN,
        locale::KEY_BACK,
        locale::KEY_VOLUME_MASTER,
        locale::KEY_VOLUME_MUSIC,
        locale::KEY_VOLUME_EFFECTS,
        locale::KEY_QUALITY,
        locale::KEY_RENDER_SCALE,
        locale::KEY_VSYNC,
    ];
    keys.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::{main_items_locale, pause_items_locale};

    /// The shipped layout file: the same rows the Rust builders draw, as data.
    const SHIPPED: &str = include_str!("../../../content/ui/menus.ui.toml");

    #[test]
    fn the_shipped_file_is_the_rust_main_and_pause_menus() {
        let layout = MenuLayout::parse(SHIPPED).unwrap();
        for locale in [locale::LocaleId::En, locale::LocaleId::De] {
            assert_eq!(
                layout.items(ScreenName::Main, locale),
                main_items_locale(locale).to_vec(),
                "the layout names the main menu the Rust builder builds"
            );
            assert_eq!(
                layout.items(ScreenName::Pause, locale),
                pause_items_locale(locale).to_vec(),
                "the layout names the pause menu the Rust builder builds"
            );
        }
    }

    #[test]
    fn a_screen_the_file_omits_draws_nothing() {
        let layout = MenuLayout::parse(SHIPPED).unwrap();
        assert!(layout.screen(ScreenName::Pause).is_some());
        assert_eq!(
            layout.items(ScreenName::Pause, locale::LocaleId::En).len(),
            pause_items_locale(locale::LocaleId::En).len()
        );
    }

    #[test]
    fn an_unknown_screen_or_row_names_the_legal_ones() {
        let screen =
            MenuLayout::parse("[[screen]]\nname = \"options\"\nrows = [\"quit\"]\n").unwrap_err();
        let shown = screen.to_string();
        assert!(shown.contains("line 2"), "{shown}");
        assert!(shown.contains("`options`"), "{shown}");
        assert!(shown.contains("main, pause"), "{shown}");

        let row =
            MenuLayout::parse("[[screen]]\nname = \"main\"\nrows = [\"quit\", \"explode\"]\n")
                .unwrap_err();
        let shown = row.to_string();
        assert!(shown.contains("line 3"), "{shown}");
        assert!(shown.contains("`explode`"), "{shown}");
        assert!(shown.contains("start-fight, settings, quit"), "{shown}");
    }

    #[test]
    fn an_empty_file_a_double_screen_and_a_missing_row_are_named() {
        assert_eq!(
            MenuLayout::parse("# nothing yet\n").unwrap_err(),
            LayoutError::Empty
        );
        let twice = MenuLayout::parse(
            "[[screen]]\nname = \"main\"\nrows = [\"quit\"]\n\
             [[screen]]\nname = \"main\"\nrows = [\"quit\"]\n",
        )
        .unwrap_err();
        assert!(twice
            .to_string()
            .contains("line 5: screen `main` is listed twice"));
        let none = MenuLayout::parse("[[screen]]\nname = \"pause\"\n").unwrap_err();
        assert!(none.to_string().contains("`pause` lists no rows"), "{none}");
        assert_eq!(
            MenuLayout::parse("[[screen]]\nrows = [\"quit\"]\n").unwrap_err(),
            LayoutError::MissingName { line: 1 }
        );
    }

    #[test]
    fn a_line_that_is_not_layout_names_the_three_forms() {
        let error =
            MenuLayout::parse("[[screen]]\nname = \"main\"\ncolor = \"red\"\n").unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("line 3"), "{shown}");
        assert!(shown.contains("`color = \"red\"`"), "{shown}");
        assert!(shown.contains("[[screen]]"), "{shown}");
    }

    #[test]
    fn a_comment_inside_a_row_list_is_not_cut_away() {
        let text = "[[screen]] # the top level\nname = \"main\"\nrows = [\"quit\"] # last\n";
        let layout = MenuLayout::parse(text).unwrap();
        assert_eq!(
            layout.items(ScreenName::Main, locale::LocaleId::En),
            [MenuItem {
                id: MenuItemId::QuitApp,
                label: "Quit",
                enabled: true
            }]
        );
    }
}
