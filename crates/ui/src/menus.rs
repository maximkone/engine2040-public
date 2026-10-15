//! menus: main/pause/settings-armed flows over mode/match hooks.
//!
//! Consumer: the windowed loop drives [`Menu::step`] from UI input
//! and runs the returned [`MenuCommand`] through [`dispatch`] against the
//! mode/match functions in [`ModeHooks`]; the settings values live outside
//! this module. Data owner: `engine-ui`. Units: none, this is a state
//! machine over menu rows; fight seeds are opaque `u64`. Determinism: every
//! transition is a pure function of the current screen, selection and one
//! [`MenuInput`]; item tables are fixed order, no floats, no clock, no IO.
//! Failure mode: out-of-range picks and unbound hooks return [`MenuError`].
//! Budget: O(1) per input, no allocation in [`Menu::step`] or [`dispatch`].

use engine_input::MAX_PARTICIPANTS;

#[path = "locale.rs"]
/// Locale tables for menu labels (static en/de, integer plurals, test pseudo).
pub mod locale;

/// Most fighters a menu-started fight may request: the canonical match-slot
/// cap from `engine-input` (one cap everywhere; legal range `1..=MAX_PARTICIPANTS`).
pub const MAX_MENU_FIGHTERS: u8 = MAX_PARTICIPANTS;

// Menu cap tracks the canonical cap exactly; no private menu bound.
const _: () = assert!(MAX_MENU_FIGHTERS == MAX_PARTICIPANTS);
/// Seed used when the menu starts a fight without a staged value.
pub const DEFAULT_FIGHT_SEED: u64 = 0x1C3A_9E77_51F2_0B41;

/// Menu failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MenuError {
    /// `Choose` index outside the current item table; index and length carried.
    BadIndex { index: usize, len: usize },
    /// A mode/match hook fired with no function bound; hook name carried.
    MissingHook(&'static str),
    /// A fight request names an out-of-range fighter count; culprit carried.
    BadFighters(u8),
}

impl core::fmt::Display for MenuError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadIndex { index, len } => {
                write!(f, "menu index {index} outside {len} items")
            }
            Self::MissingHook(hook) => write!(f, "menu hook '{hook}' not bound"),
            Self::BadFighters(n) => {
                write!(f, "fighter count {n} outside 1..={MAX_MENU_FIGHTERS}")
            }
        }
    }
}

impl std::error::Error for MenuError {}

/// Which row of a menu table fired. Local shape; mirrors nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MenuItemId {
    /// Main: open a fight through the mode table.
    StartFight,
    /// Main/pause: arm the settings screen.
    OpenSettings,
    /// Main: leave the application.
    QuitApp,
    /// Pause: return to the running fight.
    Resume,
    /// Pause: drop the fight and return to main.
    QuitToMain,
    /// Settings: disarm and return to the arming screen.
    Back,
}

/// One selectable row: id plus the label the renderer draws. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MenuItem {
    /// Which action this row fires on confirm.
    pub id: MenuItemId,
    /// Label drawn by the renderer as text quads.
    pub label: &'static str,
    /// Rows are always enabled; unenabled input is refused, never drawn dead.
    pub enabled: bool,
}

impl MenuItemId {
    /// Locale key for this row id. Pure.
    #[must_use]
    pub const fn locale_key(self) -> &'static str {
        match self {
            Self::StartFight => locale::KEY_START_FIGHT,
            Self::OpenSettings => locale::KEY_SETTINGS,
            Self::QuitApp => locale::KEY_QUIT,
            Self::Resume => locale::KEY_RESUME,
            Self::QuitToMain => locale::KEY_QUIT_TO_MAIN,
            Self::Back => locale::KEY_BACK,
        }
    }
}

/// Localized label for a row id via `locale::lookup`. Pure, no allocation.
#[must_use]
pub fn localized_label(id: MenuItemId, locale: locale::LocaleId) -> &'static str {
    locale::lookup(id.locale_key(), locale)
}

/// Main-menu rows with localized labels, same ids and order. Pure, no allocation.
#[must_use]
pub fn main_items_locale(locale: locale::LocaleId) -> [MenuItem; 3] {
    [
        MenuItem {
            id: MenuItemId::StartFight,
            label: localized_label(MenuItemId::StartFight, locale),
            enabled: true,
        },
        MenuItem {
            id: MenuItemId::OpenSettings,
            label: localized_label(MenuItemId::OpenSettings, locale),
            enabled: true,
        },
        MenuItem {
            id: MenuItemId::QuitApp,
            label: localized_label(MenuItemId::QuitApp, locale),
            enabled: true,
        },
    ]
}

/// Pause-menu rows with localized labels, same ids and order. Pure, no allocation.
#[must_use]
pub fn pause_items_locale(locale: locale::LocaleId) -> [MenuItem; 3] {
    [
        MenuItem {
            id: MenuItemId::Resume,
            label: localized_label(MenuItemId::Resume, locale),
            enabled: true,
        },
        MenuItem {
            id: MenuItemId::OpenSettings,
            label: localized_label(MenuItemId::OpenSettings, locale),
            enabled: true,
        },
        MenuItem {
            id: MenuItemId::QuitToMain,
            label: localized_label(MenuItemId::QuitToMain, locale),
            enabled: true,
        },
    ]
}

/// Where the settings screen returns when disarmed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ReturnTo {
    /// Armed from the main menu.
    Main,
    /// Armed from the pause menu.
    Pause,
}

/// Visible menu screen. Settings always remembers where it armed from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Screen {
    /// Top-level menu: start, settings, quit.
    Main,
    /// In-fight menu: resume, settings, quit to main.
    Pause,
    /// Settings armed from main or pause; its values live outside this module.
    Settings { from: ReturnTo },
}

/// Plain input event feeding the state machine. No crate types cross here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MenuInput {
    /// Move the highlight one row up (wraps).
    Up,
    /// Move the highlight one row down (wraps).
    Down,
    /// Fire the highlighted row.
    Confirm,
    /// Back out: close settings, or resume from pause.
    Back,
    /// Pause key: resume from pause, close settings, else no-op.
    PauseToggle,
    /// Jump to row `0..len` and fire it; out-of-range fails.
    Choose(usize),
}

/// Request handed to the mode table when a fight starts. Plain data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FightRequest {
    /// Opaque match seed picked by the menu.
    pub seed: u64,
    /// Fighters in the requested match, `1..=MAX_MENU_FIGHTERS`
    /// (the canonical `engine-input` cap).
    pub fighters: u8,
}

impl FightRequest {
    /// Default request: default seed, two fighters. Pure.
    #[must_use]
    pub const fn default_fight() -> Self {
        Self {
            seed: DEFAULT_FIGHT_SEED,
            fighters: 2,
        }
    }

    /// Reject fighter counts outside `1..=MAX_MENU_FIGHTERS` (the canonical
    /// cap); the refusal names the culprit plus the legal range. Pure.
    pub const fn validate(&self) -> Result<(), MenuError> {
        let clamped = DragValue::clamped(self.fighters, 1, MAX_MENU_FIGHTERS);
        if clamped != self.fighters {
            return Err(MenuError::BadFighters(self.fighters));
        }
        Ok(())
    }
}

/// Receipt returned by the `start_fight` hook. Plain data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FightStarted {
    /// Seed the match actually started with.
    pub seed: u64,
    /// Fighters in the started match.
    pub fighters: u8,
}

/// Command a transition asks the owner to run. Pure output, no side effects.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MenuCommand {
    /// Selection moved or input ignored; nothing to run.
    None,
    /// Start a match with this request.
    StartFight(FightRequest),
    /// Return to the running fight.
    Resume,
    /// Settings screen armed.
    OpenSettings,
    /// Settings screen disarmed.
    CloseSettings,
    /// Drop the fight and show the main menu.
    QuitToMain,
    /// Leave the application.
    QuitApp,
}

/// Observable effect of running a command through [`dispatch`]. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MenuEffect {
    /// Nothing ran.
    Nothing,
    /// Settings screen armed.
    SettingsOpened,
    /// Settings screen disarmed.
    SettingsClosed,
    /// The mode table started this match.
    FightStarted(FightStarted),
    /// The match resumed.
    Resumed,
    /// Returned to the main menu.
    QuitToMain,
    /// Application quit requested.
    QuitApp,
}

/// Mode/match function hooks: fn-pointer table the menu flows call into.
/// Unbound hooks fail loudly via [`MenuError::MissingHook`], never silently.
#[derive(Clone, Copy, Debug)]
pub struct ModeHooks {
    /// Start a match from a menu request.
    pub start_fight: Option<fn(FightRequest) -> FightStarted>,
    /// Resume the running match (pause hidden by the caller).
    pub resume: Option<fn()>,
    /// Drop the match and return to the main menu.
    pub quit_to_main: Option<fn()>,
    /// Leave the application.
    pub quit_app: Option<fn()>,
}

impl ModeHooks {
    /// Empty table: every hook unbound until the owner fills it. Pure.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            start_fight: None,
            resume: None,
            quit_to_main: None,
            quit_app: None,
        }
    }
}

/// Run a command against the hook table. Pure dispatch, O(1), no allocation.
pub fn dispatch(cmd: MenuCommand, hooks: &ModeHooks) -> Result<MenuEffect, MenuError> {
    match cmd {
        MenuCommand::None => Ok(MenuEffect::Nothing),
        MenuCommand::OpenSettings => Ok(MenuEffect::SettingsOpened),
        MenuCommand::CloseSettings => Ok(MenuEffect::SettingsClosed),
        MenuCommand::StartFight(request) => {
            request.validate()?;
            match hooks.start_fight {
                Some(start) => Ok(MenuEffect::FightStarted(start(request))),
                None => Err(MenuError::MissingHook("start_fight")),
            }
        }
        MenuCommand::Resume => match hooks.resume {
            Some(resume) => {
                resume();
                Ok(MenuEffect::Resumed)
            }
            None => Err(MenuError::MissingHook("resume")),
        },
        MenuCommand::QuitToMain => match hooks.quit_to_main {
            Some(quit) => {
                quit();
                Ok(MenuEffect::QuitToMain)
            }
            None => Err(MenuError::MissingHook("quit_to_main")),
        },
        MenuCommand::QuitApp => match hooks.quit_app {
            Some(quit) => {
                quit();
                Ok(MenuEffect::QuitApp)
            }
            None => Err(MenuError::MissingHook("quit_app")),
        },
    }
}

/// Main-menu rows in fixed order.
const MAIN_ITEMS: [MenuItem; 3] = [
    MenuItem {
        id: MenuItemId::StartFight,
        label: "Start Fight",
        enabled: true,
    },
    MenuItem {
        id: MenuItemId::OpenSettings,
        label: "Settings",
        enabled: true,
    },
    MenuItem {
        id: MenuItemId::QuitApp,
        label: "Quit",
        enabled: true,
    },
];

/// Pause-menu rows in fixed order.
const PAUSE_ITEMS: [MenuItem; 3] = [
    MenuItem {
        id: MenuItemId::Resume,
        label: "Resume",
        enabled: true,
    },
    MenuItem {
        id: MenuItemId::OpenSettings,
        label: "Settings",
        enabled: true,
    },
    MenuItem {
        id: MenuItemId::QuitToMain,
        label: "Quit to Main",
        enabled: true,
    },
];

/// Settings rows in fixed order: disarm only; the values live outside this module.
const SETTINGS_ITEMS: [MenuItem; 1] = [MenuItem {
    id: MenuItemId::Back,
    label: "Back",
    enabled: true,
}];

/// O-023 menu look (`docs/archive/2026-10-08/assets-O-023/layout.json` `menu` list, 640x360
/// reference; the renderer scales by 2/3/4 and draws 9-slice pieces from
/// `docs/archive/2026-10-08/assets-O-023/ui-atlas.json` with point sampling). The state machine
/// above keeps its rows; these numbers only dress them.
pub mod o023 {
    /// Reference width the `menu` list is laid out in.
    pub const REF_W: u32 = 640;
    /// Reference height the `menu` list is laid out in.
    pub const REF_H: u32 = 360;
    /// Center panel (`menu-panel`, `panel` piece) width.
    pub const PANEL_W: u32 = 210;
    /// Center panel (`menu-panel`, `panel` piece) height.
    pub const PANEL_H: u32 = 226;
    /// Button width (`btn-*`, 160x24 in the `menu` list).
    pub const BUTTON_W: u32 = 160;
    /// Button height (`btn-*`, 160x24 in the `menu` list).
    pub const BUTTON_H: u32 = 24;
    /// Title (`ARENA`) text scale; subtitle and body rows use scale 1.
    pub const TITLE_SCALE: u32 = 4;
    /// Button atlas states in the `menu` list (`button` + `button-primary`
    /// groups cover all four; `btn-start` shows `hover`).
    pub const BUTTON_STATES: [&str; 4] = ["normal", "hover", "pressed", "disabled"];
    /// Body text color (`ink` in `docs/archive/2026-10-08/assets-O-023/tokens.json`).
    pub const COLOR_INK: &str = "ink";
    /// Dim text color (`muted` in `docs/archive/2026-10-08/assets-O-023/tokens.json`).
    pub const COLOR_MUTED: &str = "muted";
    /// Title text color (`accent` in `docs/archive/2026-10-08/assets-O-023/tokens.json`).
    pub const COLOR_ACCENT: &str = "accent";
    /// Text color on the accent button (`onAccent` in tokens).
    pub const COLOR_ON_ACCENT: &str = "onAccent";

    /// One O-023 menu button row: label, atlas state and text color, in
    /// `layout.json` order (`btn-continue` is `disabled` until a fight runs).
    pub const BUTTONS: [(&str, &str, &str); 4] = [
        ("CONTINUE", "disabled", COLOR_MUTED),
        ("START", "hover", COLOR_ON_ACCENT),
        ("OPTIONS", "normal", COLOR_INK),
        ("QUIT", "normal", COLOR_INK),
    ];
}

/// UI-kit-v3 scroll list; same windowing as lobby `visible_slots`. Pure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UiScroll;
impl UiScroll {
    /// Window over `items` from `offset`, at most `view` rows. Pure, no allocation.
    #[rustfmt::skip]
    pub fn window<T>(items: &[T], offset: usize, view: usize) -> &[T] {
        let rest = items.len().saturating_sub(offset);
        if rest == 0 || view == 0 { return &[]; }
        items.get(offset..offset.saturating_add(rest.min(view))).unwrap_or(&[])
    }
}
/// UI-kit-v3 dropdown over fixed options. Pure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Dropdown;
impl Dropdown {
    /// Pick `index` over `len`; out-of-range fails. Pure.
    #[rustfmt::skip]
    pub fn pick(index: usize, len: usize) -> Result<usize, MenuError> {
        (index < len).then_some(index).ok_or(MenuError::BadIndex { index, len })
    }
}
/// UI-kit-v3 drag value over `[min, max]`. Pure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DragValue;
impl DragValue {
    /// Clamp `value` into `[min, max]`; `min > max` clamps to `min`. Pure.
    #[rustfmt::skip]
    pub const fn clamped(value: u8, min: u8, max: u8) -> u8 {
        if min > max || value < min { min } else if value > max { max } else { value }
    }
}

/// Menu state machine: screen plus highlight. Pure transitions via [`Menu::step`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Menu {
    screen: Screen,
    selected: usize,
}

impl Menu {
    /// Fresh menu on the main screen, highlight on the first row. Pure.
    #[must_use]
    pub const fn main() -> Self {
        Self {
            screen: Screen::Main,
            selected: 0,
        }
    }

    /// Fresh menu on the pause screen, highlight on the first row. Pure.
    #[must_use]
    pub const fn pause() -> Self {
        Self {
            screen: Screen::Pause,
            selected: 0,
        }
    }

    /// Current screen. Pure.
    #[must_use]
    pub const fn screen(&self) -> Screen {
        self.screen
    }

    /// Highlighted row index, always inside [`Menu::items`]. Pure.
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Item table for a screen, in fixed order. Pure.
    #[must_use]
    pub const fn items_for(screen: Screen) -> &'static [MenuItem] {
        match screen {
            Screen::Main => &MAIN_ITEMS,
            Screen::Pause => &PAUSE_ITEMS,
            Screen::Settings { .. } => &SETTINGS_ITEMS,
        }
    }

    /// Item table for the current screen, in fixed order. Pure.
    #[must_use]
    pub fn items(&self) -> &'static [MenuItem] {
        Self::items_for(self.screen)
    }

    /// Highlighted row, if the table is non-empty. Pure.
    #[must_use]
    pub fn current(&self) -> Option<&'static MenuItem> {
        UiScroll::window(self.items(), 0, self.items().len()).get(self.selected)
    }

    /// Move the highlight to `index`; out-of-range fails and keeps state. Pure.
    pub fn select(&mut self, index: usize) -> Result<(), MenuError> {
        let len = self.items().len();
        let picked = Dropdown::pick(index, len)?;
        self.selected = picked;
        Ok(())
    }

    /// Feed one input event: move the highlight or fire a row. The returned
    /// command runs through [`dispatch`]; failed picks keep the old state.
    /// Pure, O(1), no allocation.
    pub fn step(&mut self, input: MenuInput) -> Result<MenuCommand, MenuError> {
        match input {
            MenuInput::Up => {
                let len = self.items().len();
                if len > 0 {
                    self.selected = (self.selected + len - 1) % len;
                }
                Ok(MenuCommand::None)
            }
            MenuInput::Down => {
                let len = self.items().len();
                if len > 0 {
                    self.selected = (self.selected + 1) % len;
                }
                Ok(MenuCommand::None)
            }
            MenuInput::Choose(index) => {
                self.select(index)?;
                self.confirm()
            }
            MenuInput::Confirm => self.confirm(),
            MenuInput::Back | MenuInput::PauseToggle => Ok(self.back()),
        }
    }

    /// Fire the highlighted row. Pure.
    fn confirm(&mut self) -> Result<MenuCommand, MenuError> {
        let len = self.items().len();
        let item = self.items().get(self.selected).ok_or(MenuError::BadIndex {
            index: self.selected,
            len,
        })?;
        if !item.enabled {
            // Tables keep every row enabled; this arm refuses stale state
            // instead of firing a row the renderer shows as dead.
            return Err(MenuError::BadIndex {
                index: self.selected,
                len,
            });
        }
        Ok(match (self.screen, item.id) {
            (Screen::Main, MenuItemId::StartFight) => {
                MenuCommand::StartFight(FightRequest::default_fight())
            }
            (Screen::Main, MenuItemId::QuitApp) => MenuCommand::QuitApp,
            (Screen::Pause, MenuItemId::Resume) => MenuCommand::Resume,
            (Screen::Pause, MenuItemId::QuitToMain) => {
                self.screen = Screen::Main;
                self.selected = 0;
                MenuCommand::QuitToMain
            }
            (_, MenuItemId::OpenSettings) => {
                let from = match self.screen {
                    Screen::Pause => ReturnTo::Pause,
                    Screen::Main | Screen::Settings { .. } => ReturnTo::Main,
                };
                self.screen = Screen::Settings { from };
                self.selected = 0;
                MenuCommand::OpenSettings
            }
            (Screen::Settings { from }, MenuItemId::Back) => self.close_settings(from),
            // Rows that never sit on this screen stay put and emit nothing.
            _ => MenuCommand::None,
        })
    }

    /// Back key and pause key alike: close settings, resume from pause, ignore
    /// on main. Pure.
    fn back(&mut self) -> MenuCommand {
        match self.screen {
            Screen::Main => MenuCommand::None,
            Screen::Pause => MenuCommand::Resume,
            Screen::Settings { from } => self.close_settings(from),
        }
    }

    /// Leave settings for the screen that armed them, highlight on its first
    /// row. Pure.
    fn close_settings(&mut self, from: ReturnTo) -> MenuCommand {
        self.screen = match from {
            ReturnTo::Main => Screen::Main,
            ReturnTo::Pause => Screen::Pause,
        };
        self.selected = 0;
        MenuCommand::CloseSettings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn bound_hooks() -> ModeHooks {
        ModeHooks {
            start_fight: Some(record_start),
            resume: Some(mark_resumed),
            quit_to_main: Some(mark_quit_to_main),
            quit_app: Some(mark_quit_app),
        }
    }

    static START_SEED: AtomicBool = AtomicBool::new(false);
    static RESUMED: AtomicBool = AtomicBool::new(false);
    static QUIT_MAIN: AtomicBool = AtomicBool::new(false);
    static QUIT_APP: AtomicBool = AtomicBool::new(false);

    fn record_start(request: FightRequest) -> FightStarted {
        START_SEED.store(true, Ordering::SeqCst);
        FightStarted {
            seed: request.seed,
            fighters: request.fighters,
        }
    }

    fn mark_resumed() {
        RESUMED.store(true, Ordering::SeqCst);
    }

    fn mark_quit_to_main() {
        QUIT_MAIN.store(true, Ordering::SeqCst);
    }

    fn mark_quit_app() {
        QUIT_APP.store(true, Ordering::SeqCst);
    }

    #[test]
    fn main_boots_on_start_fight() {
        let menu = Menu::main();
        assert_eq!(menu.screen(), Screen::Main);
        assert_eq!(menu.selected(), 0);
        assert_eq!(menu.current().unwrap().id, MenuItemId::StartFight);
    }

    #[test]
    fn pause_boots_on_resume() {
        let menu = Menu::pause();
        assert_eq!(menu.screen(), Screen::Pause);
        assert_eq!(menu.current().unwrap().id, MenuItemId::Resume);
    }

    #[test]
    fn move_wraps_both_ends() {
        let mut menu = Menu::main();
        menu.step(MenuInput::Up).unwrap();
        assert_eq!(menu.current().unwrap().id, MenuItemId::QuitApp);
        menu.step(MenuInput::Down).unwrap();
        assert_eq!(menu.current().unwrap().id, MenuItemId::StartFight);
        menu.step(MenuInput::Down).unwrap();
        menu.step(MenuInput::Down).unwrap();
        assert_eq!(menu.current().unwrap().id, MenuItemId::QuitApp);
    }

    #[test]
    fn main_confirm_start_emits_default_request() {
        let mut menu = Menu::main();
        let cmd = menu.step(MenuInput::Confirm).unwrap();
        match cmd {
            MenuCommand::StartFight(request) => {
                assert_eq!(request.seed, DEFAULT_FIGHT_SEED);
                assert_eq!(request.fighters, 2);
                request.validate().unwrap();
            }
            other => panic!("wrong command: {other:?}"),
        }
        assert_eq!(menu.screen(), Screen::Main);
    }

    #[test]
    fn main_settings_arms_from_main() {
        let mut menu = Menu::main();
        menu.step(MenuInput::Choose(1)).unwrap();
        assert_eq!(
            menu.screen(),
            Screen::Settings {
                from: ReturnTo::Main
            }
        );
        assert_eq!(menu.selected(), 0);
    }

    #[test]
    fn main_confirm_quit_app() {
        let mut menu = Menu::main();
        let cmd = menu.step(MenuInput::Choose(2)).unwrap();
        assert_eq!(cmd, MenuCommand::QuitApp);
        assert_eq!(menu.screen(), Screen::Main);
    }

    #[test]
    fn main_back_and_pause_toggle_are_noops() {
        let mut menu = Menu::main();
        assert_eq!(menu.step(MenuInput::Back).unwrap(), MenuCommand::None);
        assert_eq!(
            menu.step(MenuInput::PauseToggle).unwrap(),
            MenuCommand::None
        );
        assert_eq!(menu.screen(), Screen::Main);
        assert_eq!(menu.selected(), 0);
    }

    #[test]
    fn pause_confirm_resume() {
        let mut menu = Menu::pause();
        assert_eq!(menu.step(MenuInput::Confirm).unwrap(), MenuCommand::Resume);
        assert_eq!(menu.screen(), Screen::Pause);
    }

    #[test]
    fn pause_back_resumes() {
        let mut menu = Menu::pause();
        assert_eq!(menu.step(MenuInput::Back).unwrap(), MenuCommand::Resume);
    }

    #[test]
    fn pause_toggle_resumes() {
        let mut menu = Menu::pause();
        assert_eq!(
            menu.step(MenuInput::PauseToggle).unwrap(),
            MenuCommand::Resume
        );
    }

    #[test]
    fn pause_settings_arms_from_pause() {
        let mut menu = Menu::pause();
        assert_eq!(
            menu.step(MenuInput::Choose(1)).unwrap(),
            MenuCommand::OpenSettings
        );
        assert_eq!(
            menu.screen(),
            Screen::Settings {
                from: ReturnTo::Pause
            }
        );
    }

    #[test]
    fn pause_quit_returns_to_main() {
        let mut menu = Menu::pause();
        assert_eq!(
            menu.step(MenuInput::Choose(2)).unwrap(),
            MenuCommand::QuitToMain
        );
        assert_eq!(menu.screen(), Screen::Main);
        assert_eq!(menu.selected(), 0);
        assert_eq!(menu.current().unwrap().id, MenuItemId::StartFight);
    }

    #[test]
    fn settings_confirm_back_returns_to_main() {
        let mut menu = Menu::main();
        menu.step(MenuInput::Choose(1)).unwrap();
        assert_eq!(
            menu.step(MenuInput::Confirm).unwrap(),
            MenuCommand::CloseSettings
        );
        assert_eq!(menu.screen(), Screen::Main);
    }

    #[test]
    fn settings_back_returns_to_pause() {
        let mut menu = Menu::pause();
        menu.step(MenuInput::Choose(1)).unwrap();
        assert_eq!(
            menu.step(MenuInput::Back).unwrap(),
            MenuCommand::CloseSettings
        );
        assert_eq!(menu.screen(), Screen::Pause);
        assert_eq!(menu.current().unwrap().id, MenuItemId::Resume);
    }

    #[test]
    fn settings_pause_toggle_closes_to_origin() {
        let mut menu = Menu::pause();
        menu.step(MenuInput::Choose(1)).unwrap();
        assert_eq!(
            menu.step(MenuInput::PauseToggle).unwrap(),
            MenuCommand::CloseSettings
        );
        assert_eq!(menu.screen(), Screen::Pause);
    }

    #[test]
    fn settings_moves_are_noops_on_single_row() {
        let mut menu = Menu::main();
        menu.step(MenuInput::Choose(1)).unwrap();
        assert_eq!(menu.step(MenuInput::Up).unwrap(), MenuCommand::None);
        assert_eq!(menu.step(MenuInput::Down).unwrap(), MenuCommand::None);
        assert_eq!(menu.selected(), 0);
    }

    #[test]
    fn choose_out_of_range_fails_and_keeps_state() {
        let mut menu = Menu::main();
        let err = menu.step(MenuInput::Choose(9)).unwrap_err();
        assert_eq!(err, MenuError::BadIndex { index: 9, len: 3 });
        assert_eq!(menu.screen(), Screen::Main);
        assert_eq!(menu.selected(), 0);
    }

    #[test]
    fn select_out_of_range_fails() {
        let mut menu = Menu::pause();
        assert!(menu.select(3).is_err());
        assert_eq!(menu.selected(), 0);
        menu.select(2).unwrap();
        assert_eq!(menu.current().unwrap().id, MenuItemId::QuitToMain);
    }

    #[test]
    fn dispatch_start_fight_calls_hook() {
        START_SEED.store(false, Ordering::SeqCst);
        let hooks = bound_hooks();
        let cmd = MenuCommand::StartFight(FightRequest::default_fight());
        match dispatch(cmd, &hooks).unwrap() {
            MenuEffect::FightStarted(started) => {
                assert_eq!(started.seed, DEFAULT_FIGHT_SEED);
                assert_eq!(started.fighters, 2);
            }
            other => panic!("wrong effect: {other:?}"),
        }
        assert!(START_SEED.load(Ordering::SeqCst));
    }

    #[test]
    fn dispatch_resume_calls_hook() {
        RESUMED.store(false, Ordering::SeqCst);
        assert_eq!(
            dispatch(MenuCommand::Resume, &bound_hooks()).unwrap(),
            MenuEffect::Resumed
        );
        assert!(RESUMED.load(Ordering::SeqCst));
    }

    #[test]
    fn dispatch_quits_call_hooks() {
        QUIT_MAIN.store(false, Ordering::SeqCst);
        QUIT_APP.store(false, Ordering::SeqCst);
        let hooks = bound_hooks();
        assert_eq!(
            dispatch(MenuCommand::QuitToMain, &hooks).unwrap(),
            MenuEffect::QuitToMain
        );
        assert_eq!(
            dispatch(MenuCommand::QuitApp, &hooks).unwrap(),
            MenuEffect::QuitApp
        );
        assert!(QUIT_MAIN.load(Ordering::SeqCst));
        assert!(QUIT_APP.load(Ordering::SeqCst));
    }

    #[test]
    fn dispatch_settings_needs_no_hooks() {
        let empty = ModeHooks::empty();
        assert_eq!(
            dispatch(MenuCommand::OpenSettings, &empty).unwrap(),
            MenuEffect::SettingsOpened
        );
        assert_eq!(
            dispatch(MenuCommand::CloseSettings, &empty).unwrap(),
            MenuEffect::SettingsClosed
        );
        assert_eq!(
            dispatch(MenuCommand::None, &empty).unwrap(),
            MenuEffect::Nothing
        );
    }

    #[test]
    fn dispatch_unbound_hooks_fail_loudly() {
        let empty = ModeHooks::empty();
        assert_eq!(
            dispatch(
                MenuCommand::StartFight(FightRequest::default_fight()),
                &empty
            )
            .unwrap_err(),
            MenuError::MissingHook("start_fight")
        );
        assert_eq!(
            dispatch(MenuCommand::Resume, &empty).unwrap_err(),
            MenuError::MissingHook("resume")
        );
        assert_eq!(
            dispatch(MenuCommand::QuitToMain, &empty).unwrap_err(),
            MenuError::MissingHook("quit_to_main")
        );
        assert_eq!(
            dispatch(MenuCommand::QuitApp, &empty).unwrap_err(),
            MenuError::MissingHook("quit_app")
        );
    }

    #[test]
    fn menu_cap_follows_canonical_cap() {
        // One cap everywhere: the menu bound tracks `engine-input` exactly.
        assert_eq!(MAX_MENU_FIGHTERS, MAX_PARTICIPANTS);
        FightRequest {
            seed: 1,
            fighters: MAX_PARTICIPANTS,
        }
        .validate()
        .expect("canonical cap must validate");
        assert_eq!(
            FightRequest {
                seed: 1,
                fighters: MAX_PARTICIPANTS + 1,
            }
            .validate(),
            Err(MenuError::BadFighters(MAX_PARTICIPANTS + 1))
        );
    }

    #[test]
    fn dispatch_bad_fighter_count_refused() {
        let hooks = bound_hooks();
        // Matrix 0..=17: exactly the legal range validates, everything else
        // refuses through both `validate` and `dispatch`.
        for n in 0u8..=17 {
            let request = FightRequest {
                seed: 7,
                fighters: n,
            };
            let cmd = MenuCommand::StartFight(request);
            if n == 0 || n > MAX_MENU_FIGHTERS {
                assert_eq!(request.validate(), Err(MenuError::BadFighters(n)));
                assert_eq!(
                    dispatch(cmd, &hooks).unwrap_err(),
                    MenuError::BadFighters(n)
                );
                // The refusal names the culprit plus the legal range.
                let text = MenuError::BadFighters(n).to_string();
                assert!(text.contains(&n.to_string()), "names culprit: {text}");
                assert!(
                    text.contains(&format!("1..={MAX_MENU_FIGHTERS}")),
                    "names legal range: {text}"
                );
            } else {
                request.validate().unwrap();
                assert!(
                    dispatch(cmd, &hooks).is_ok(),
                    "count {n} inside the cap must dispatch"
                );
            }
        }
        // No behavior change at 8 or below: the old menu range still passes.
        for n in 1u8..=8 {
            FightRequest {
                seed: 7,
                fighters: n,
            }
            .validate()
            .expect("counts 1..=8 must keep validating");
        }
    }

    #[test]
    fn click_through_main_settings_then_fight() {
        let hooks = bound_hooks();
        let mut menu = Menu::main();
        // Open settings, disarm, then start the fight: the no-console path.
        assert_eq!(
            menu.step(MenuInput::Choose(1)).unwrap(),
            MenuCommand::OpenSettings
        );
        assert!(matches!(
            dispatch(MenuCommand::OpenSettings, &hooks).unwrap(),
            MenuEffect::SettingsOpened
        ));
        assert_eq!(
            menu.step(MenuInput::Back).unwrap(),
            MenuCommand::CloseSettings
        );
        let cmd = menu.step(MenuInput::Choose(0)).unwrap();
        assert!(matches!(cmd, MenuCommand::StartFight(_)));
        assert!(matches!(
            dispatch(cmd, &hooks).unwrap(),
            MenuEffect::FightStarted(_)
        ));
    }

    #[test]
    fn pause_flow_quit_dispatch_returns_to_main() {
        let hooks = bound_hooks();
        let mut menu = Menu::pause();
        let cmd = menu.step(MenuInput::Choose(2)).unwrap();
        assert_eq!(dispatch(cmd, &hooks).unwrap(), MenuEffect::QuitToMain);
        assert_eq!(menu.screen(), Screen::Main);
    }

    #[test]
    fn error_display_names_each_case() {
        assert_eq!(
            MenuError::BadIndex { index: 5, len: 3 }.to_string(),
            "menu index 5 outside 3 items"
        );
        assert_eq!(
            MenuError::MissingHook("resume").to_string(),
            "menu hook 'resume' not bound"
        );
        assert_eq!(
            MenuError::BadFighters(0).to_string(),
            format!("fighter count 0 outside 1..={MAX_MENU_FIGHTERS}")
        );
        assert_eq!(
            MenuError::BadFighters(MAX_MENU_FIGHTERS + 1).to_string(),
            format!(
                "fighter count {} outside 1..={MAX_MENU_FIGHTERS}",
                MAX_MENU_FIGHTERS + 1
            )
        );
    }

    #[test]
    fn scroll_windows_like_lobby_slots() {
        let items = Menu::main().items();
        assert_eq!(UiScroll::window(items, 0, 8).len(), items.len());
        assert!(UiScroll::window(items, 99, 8).is_empty());
    }

    #[test]
    #[rustfmt::skip]
    fn dropdown_pick_names_bad_index() {
        assert_eq!(Dropdown::pick(1, 3).unwrap(), 1);
        assert_eq!(Dropdown::pick(9, 3).unwrap_err(), MenuError::BadIndex { index: 9, len: 3 });
    }

    #[test]
    fn drag_clamps_fighter_range() {
        assert_eq!(DragValue::clamped(0, 1, 8), 1);
        assert_eq!(DragValue::clamped(99, 1, 8), 8);
    }
}
