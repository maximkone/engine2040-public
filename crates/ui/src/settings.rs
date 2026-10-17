//! settings: volume, graphics and bindings as staged batches with a replay tape.
//!
//! Consumer: the windowed loop's settings screen (the menu's
//! [`Screen::Settings`](super::menus::Screen::Settings) arms it; the values
//! live here, not in the menu). Data owner: [`SettingsPanel`] owns the live
//! [`Settings`], one draft, the staged batch, the undo stack and the
//! append-only [`SettingsTape`]. Units: volumes are permille (`0..=1000`),
//! render scale is percent (`50..=100`), the deadzone is wish units, ticks are
//! zero-based fixed steps. Determinism: no IO, no wall clock; the same staged
//! batches stamped at the same ticks give the same settings on every tick and
//! the same [`SettingsTape::digest`]. Errors: [`SettingsError`]; a refusal
//! changes nothing.
//!
//! Staging discipline (the editor's live tuning panel, for the player): an edit
//! is validated against the draft and joins the staged batch; nothing is live
//! until [`SettingsPanel::apply`] stamps the batch with a boundary tick that is
//! not in the past. [`SettingsPanel::advance`] moves the live view to a tick
//! and [`SettingsTape::settings_at`] folds the tape, so a replay reads the same
//! value on the same tick. Every edit is undoable: a staged edit pops off the
//! batch, an applied batch is undone by appending its inverse to the tape (the
//! tape is never rewritten, history before the undo tick stays replayable).
//!
//! Persistence: [`encode`] writes the settings as canonical `key=value` text
//! with a checksum line, [`decode`] refuses anything else, and [`startup`]
//! picks the values a run starts with. There is no file IO here: the caller
//! reads and writes the bytes. A file that does not read gives the defaults
//! plus a [`SettingsFileError`] notice, never a half-applied file.

use engine_core::Fnv64;
use engine_input::{Bindings, BoundAction, PhysicalButton};

/// Most edits one staged batch holds.
pub const MAX_STAGED: usize = 32;
/// Most records the tape holds (applies plus undos).
pub const MAX_TAPE_RECORDS: usize = 256;
/// Loudest volume: permille.
pub const VOLUME_MAX: u16 = 1000;
/// Smallest render scale in percent.
pub const RENDER_SCALE_MIN: u8 = 50;
/// Largest render scale in percent.
pub const RENDER_SCALE_MAX: u8 = 100;
/// Largest deadzone a player can pick, in wish units.
pub const DEADZONE_MAX: u8 = 50;

/// One volume bus.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VolumeBus {
    /// Everything, applied on top of the other two.
    Master,
    /// Music bed.
    Music,
    /// Hits, footsteps, crowd.
    Effects,
}

impl VolumeBus {
    /// Every bus in fixed order.
    pub const ALL: [Self; 3] = [Self::Master, Self::Music, Self::Effects];

    const fn index(self) -> usize {
        match self {
            Self::Master => 0,
            Self::Music => 1,
            Self::Effects => 2,
        }
    }
}

/// Graphics quality preset the renderer maps to its own knobs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Quality {
    /// Fewest passes.
    Low,
    /// Balanced.
    Medium,
    /// Every pass the build has.
    High,
}

impl Quality {
    const fn code(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }

    /// The word the settings file stores for this preset.
    const fn word(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Preset from its stored word; `None` for any other text.
    fn from_word(word: &str) -> Option<Self> {
        match word {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// The player's settings: what the loop reads each tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Settings {
    volumes: [u16; 3],
    /// Graphics quality preset.
    pub quality: Quality,
    /// Render scale in percent, `RENDER_SCALE_MIN..=RENDER_SCALE_MAX`.
    pub render_scale_pct: u8,
    /// Wait for vertical sync when presenting.
    pub vsync: bool,
    /// Button table plus deadzone, handed to the input layer unchanged.
    pub bindings: Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volumes: [800, 600, 800],
            quality: Quality::High,
            render_scale_pct: RENDER_SCALE_MAX,
            vsync: true,
            bindings: Bindings::new(),
        }
    }
}

impl Settings {
    /// Volume of one bus in permille.
    #[must_use]
    pub fn volume(&self, bus: VolumeBus) -> u16 {
        self.volumes[bus.index()]
    }

    /// Why `change` would be refused on top of these settings, if it would.
    /// Pure; nothing changes.
    ///
    /// # Errors
    /// [`SettingsError`] naming the bound or the action holding the button.
    pub fn check(&self, change: SettingChange) -> Result<(), SettingsError> {
        match change {
            SettingChange::Volume { permille, .. } if permille > VOLUME_MAX => {
                Err(SettingsError::VolumeOutOfRange { permille })
            }
            SettingChange::RenderScale(pct)
                if !(RENDER_SCALE_MIN..=RENDER_SCALE_MAX).contains(&pct) =>
            {
                Err(SettingsError::RenderScaleOutOfRange { pct })
            }
            SettingChange::Deadzone(deadzone) if deadzone > DEADZONE_MAX => {
                Err(SettingsError::DeadzoneOutOfRange { deadzone })
            }
            SettingChange::Rebind { action, button } => self.check_rebind(action, button),
            _ => Ok(()),
        }
    }

    /// One button drives one action: a rebind onto a button another action
    /// holds is refused (rebind that action first).
    fn check_rebind(
        &self,
        action: BoundAction,
        button: PhysicalButton,
    ) -> Result<(), SettingsError> {
        for other in BoundAction::ALL {
            if other != action && self.bindings.button(other) == button {
                return Err(SettingsError::ButtonTaken {
                    action: other,
                    wanted: action,
                });
            }
        }
        Ok(())
    }

    /// The change that puts back what `change` is about to overwrite. Read it
    /// before [`Settings::set`].
    #[must_use]
    pub fn inverse(&self, change: SettingChange) -> SettingChange {
        match change {
            SettingChange::Volume { bus, .. } => SettingChange::Volume {
                bus,
                permille: self.volume(bus),
            },
            SettingChange::Quality(_) => SettingChange::Quality(self.quality),
            SettingChange::RenderScale(_) => SettingChange::RenderScale(self.render_scale_pct),
            SettingChange::Vsync(_) => SettingChange::Vsync(self.vsync),
            SettingChange::Rebind { action, .. } => SettingChange::Rebind {
                action,
                button: self.bindings.button(action),
            },
            SettingChange::Deadzone(_) => SettingChange::Deadzone(self.bindings.deadzone()),
        }
    }

    /// Write one already-checked change. The tape folds with this, so it never
    /// refuses: a replay must land where the live run landed.
    pub fn set(&mut self, change: SettingChange) {
        match change {
            SettingChange::Volume { bus, permille } => self.volumes[bus.index()] = permille,
            SettingChange::Quality(quality) => self.quality = quality,
            SettingChange::RenderScale(pct) => self.render_scale_pct = pct,
            SettingChange::Vsync(on) => self.vsync = on,
            SettingChange::Rebind { action, button } => self.bindings.rebind(action, button),
            SettingChange::Deadzone(deadzone) => self.bindings.set_deadzone(deadzone),
        }
    }
}

/// One edit on the settings screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SettingChange {
    /// Set one bus volume, permille.
    Volume {
        /// Bus to set.
        bus: VolumeBus,
        /// New volume, `0..=VOLUME_MAX`.
        permille: u16,
    },
    /// Pick a graphics preset.
    Quality(Quality),
    /// Set the render scale in percent.
    RenderScale(u8),
    /// Turn vertical sync on or off.
    Vsync(bool),
    /// Bind an action to a button no other action holds.
    Rebind {
        /// Action to rebind.
        action: BoundAction,
        /// Button to bind it to.
        button: PhysicalButton,
    },
    /// Set the stick deadzone in wish units.
    Deadzone(u8),
}

impl SettingChange {
    fn fold_into(self, hasher: &mut Fnv64) {
        match self {
            Self::Volume { bus, permille } => {
                hasher.write(&[0, u8::try_from(bus.index()).unwrap_or(u8::MAX)]);
                hasher.write_u16(permille);
            }
            Self::Quality(quality) => hasher.write(&[1, quality.code()]),
            Self::RenderScale(pct) => hasher.write(&[2, pct]),
            Self::Vsync(on) => hasher.write(&[3, u8::from(on)]),
            Self::Rebind { action, button } => hasher.write(&[
                4,
                u8::try_from(action.index()).unwrap_or(u8::MAX),
                button.kind(),
                button.code(),
            ]),
            Self::Deadzone(deadzone) => hasher.write(&[5, deadzone]),
        }
    }
}

/// What refused a settings call. Refusals change nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SettingsError {
    /// Volume above [`VOLUME_MAX`], culprit carried.
    VolumeOutOfRange {
        /// Volume asked for, permille.
        permille: u16,
    },
    /// Render scale outside `RENDER_SCALE_MIN..=RENDER_SCALE_MAX`.
    RenderScaleOutOfRange {
        /// Percent asked for.
        pct: u8,
    },
    /// Deadzone above [`DEADZONE_MAX`].
    DeadzoneOutOfRange {
        /// Wish units asked for.
        deadzone: u8,
    },
    /// The button already drives another action; both actions are carried.
    ButtonTaken {
        /// Action holding the button.
        action: BoundAction,
        /// Action that asked for it.
        wanted: BoundAction,
    },
    /// Every action already took its button in this drill.
    DrillDone,
    /// The staged batch already holds [`MAX_STAGED`] edits.
    StagedFull,
    /// Nothing staged to apply or pop.
    NothingStaged,
    /// Nothing applied to undo.
    NothingToUndo,
    /// Staged edits are pending: apply or pop them before undoing an apply.
    StagedPending,
    /// The tape holds [`MAX_TAPE_RECORDS`] records.
    TapeFull,
    /// The boundary tick is before the live tick or the last record.
    BoundaryInPast {
        /// Tick asked for.
        at_tick: u32,
        /// Earliest tick the call may stamp.
        earliest: u32,
    },
    /// The live view only moves forward.
    TickWentBack {
        /// Tick the view is at.
        cursor: u32,
        /// Tick asked for.
        tick: u32,
    },
}

impl core::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::VolumeOutOfRange { permille } => {
                write!(f, "volume {permille} is above {VOLUME_MAX} permille")
            }
            Self::RenderScaleOutOfRange { pct } => write!(
                f,
                "render scale {pct}% is outside {RENDER_SCALE_MIN}..={RENDER_SCALE_MAX}%"
            ),
            Self::DeadzoneOutOfRange { deadzone } => {
                write!(f, "deadzone {deadzone} is above {DEADZONE_MAX}")
            }
            Self::ButtonTaken { action, wanted } => write!(
                f,
                "{wanted:?} cannot take a button that drives {action:?}; rebind {action:?} first"
            ),
            Self::DrillDone => write!(f, "every action already has its button"),
            Self::StagedFull => write!(f, "staged batch is full ({MAX_STAGED} edits)"),
            Self::NothingStaged => write!(f, "nothing staged"),
            Self::NothingToUndo => write!(f, "nothing applied to undo"),
            Self::StagedPending => write!(f, "staged edits are pending; apply or pop them first"),
            Self::TapeFull => write!(f, "settings tape is full ({MAX_TAPE_RECORDS} records)"),
            Self::BoundaryInPast { at_tick, earliest } => {
                write!(f, "tick {at_tick} is in the past (earliest {earliest})")
            }
            Self::TickWentBack { cursor, tick } => {
                write!(f, "live tick is {cursor}, cannot go back to {tick}")
            }
        }
    }
}

impl std::error::Error for SettingsError {}

/// One tape row: a batch that takes effect on `at_tick`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Record {
    at_tick: u32,
    changes: Vec<SettingChange>,
}

/// Append-only journal of applied batches. Replaying it from the defaults
/// gives the same settings on the same tick on every machine.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct SettingsTape {
    records: Vec<Record>,
}

impl SettingsTape {
    /// Empty tape: every tick reads the defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records on the tape.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when nothing was ever applied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Tick of the newest record, `0` on an empty tape.
    #[must_use]
    pub fn last_tick(&self) -> u32 {
        self.records.last().map_or(0, |record| record.at_tick)
    }

    /// Append one batch. Ticks never go back, so the fold order is the tape
    /// order.
    ///
    /// # Errors
    /// [`SettingsError::BoundaryInPast`] before the newest record,
    /// [`SettingsError::TapeFull`] at the cap.
    pub fn record(
        &mut self,
        at_tick: u32,
        changes: Vec<SettingChange>,
    ) -> Result<(), SettingsError> {
        if at_tick < self.last_tick() {
            return Err(SettingsError::BoundaryInPast {
                at_tick,
                earliest: self.last_tick(),
            });
        }
        if self.records.len() >= MAX_TAPE_RECORDS {
            return Err(SettingsError::TapeFull);
        }
        self.records.push(Record { at_tick, changes });
        Ok(())
    }

    /// Settings in force on `tick`: the defaults with every record at or
    /// before `tick` folded in tape order. Pure.
    #[must_use]
    pub fn settings_at(&self, tick: u32) -> Settings {
        let mut settings = Settings::default();
        for record in self
            .records
            .iter()
            .take_while(|record| record.at_tick <= tick)
        {
            for &change in &record.changes {
                settings.set(change);
            }
        }
        settings
    }

    /// Digest of the whole tape for replay checks. A view receipt outside any
    /// simulation hash scope. Same records give the same digest. Pure.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u64(u64::try_from(self.records.len()).unwrap_or(u64::MAX));
        for record in &self.records {
            hasher.write_u32(record.at_tick);
            hasher.write_u64(u64::try_from(record.changes.len()).unwrap_or(u64::MAX));
            for &change in &record.changes {
                change.fold_into(&mut hasher);
            }
        }
        hasher.finish()
    }
}

/// What one apply or undo stamped on the tape.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SettingsReceipt {
    /// Tick the batch takes effect on.
    pub at_tick: u32,
    /// Edits in the batch.
    pub changes: usize,
    /// Tape digest after the record.
    pub tape_digest: u64,
}

/// One staged edit and the edit that undoes it against the draft before it.
#[derive(Clone, Copy, Debug)]
struct Staged {
    change: SettingChange,
    inverse: SettingChange,
}

/// The settings screen's state: draft, staged batch, undo stack and tape.
#[derive(Clone, Debug)]
pub struct SettingsPanel {
    live: Settings,
    draft: Settings,
    staged: Vec<Staged>,
    /// Inverse batches of applied records, newest last; each is already in
    /// undo order (last edit first).
    applied: Vec<Vec<SettingChange>>,
    tape: SettingsTape,
    cursor: u32,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsPanel {
    /// Fresh panel at tick 0 on the defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            live: Settings::default(),
            draft: Settings::default(),
            staged: Vec::new(),
            applied: Vec::new(),
            tape: SettingsTape::new(),
            cursor: 0,
        }
    }

    /// Panel that starts from saved settings. The tape opens with one record
    /// at tick 0 that writes every value, so later ticks replay from the saved
    /// values, not from the defaults. Undo starts empty.
    #[must_use]
    pub fn from_saved(saved: Settings) -> Self {
        let mut panel = Self::new();
        // An empty tape holds the first record at tick 0 without a cap check.
        panel.tape.records.push(Record {
            at_tick: 0,
            changes: snapshot(&saved),
        });
        panel.live = saved;
        panel.draft = saved;
        panel
    }

    /// Settings in force at the live tick.
    #[must_use]
    pub fn live(&self) -> &Settings {
        &self.live
    }

    /// What the screen shows: live plus every staged edit.
    #[must_use]
    pub fn draft(&self) -> &Settings {
        &self.draft
    }

    /// Tick the live view is at.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Edits waiting in the staged batch.
    #[must_use]
    pub fn staged_len(&self) -> usize {
        self.staged.len()
    }

    /// Batches that an undo can still take back.
    #[must_use]
    pub fn undoable(&self) -> usize {
        self.applied.len()
    }

    /// The replay tape.
    #[must_use]
    pub fn tape(&self) -> &SettingsTape {
        &self.tape
    }

    /// Move the live view to `tick`, reading the tape. Boundary rule: an
    /// applied batch becomes live exactly when the view reaches its tick.
    ///
    /// # Errors
    /// [`SettingsError::TickWentBack`] when `tick` is before the live tick.
    pub fn advance(&mut self, tick: u32) -> Result<(), SettingsError> {
        if tick < self.cursor {
            return Err(SettingsError::TickWentBack {
                cursor: self.cursor,
                tick,
            });
        }
        self.cursor = tick;
        self.live = self.tape.settings_at(tick);
        Ok(())
    }

    /// Stage one edit on the draft.
    ///
    /// # Errors
    /// [`SettingsError`] when the value is out of range, the button is taken
    /// or the batch is full; nothing changes.
    pub fn stage(&mut self, change: SettingChange) -> Result<(), SettingsError> {
        if self.staged.len() >= MAX_STAGED {
            return Err(SettingsError::StagedFull);
        }
        self.draft.check(change)?;
        let inverse = self.draft.inverse(change);
        self.draft.set(change);
        self.staged.push(Staged { change, inverse });
        Ok(())
    }

    /// Take the newest staged edit back off the draft.
    ///
    /// # Errors
    /// [`SettingsError::NothingStaged`] on an empty batch.
    pub fn undo_staged(&mut self) -> Result<SettingChange, SettingsError> {
        let Some(last) = self.staged.pop() else {
            return Err(SettingsError::NothingStaged);
        };
        self.draft.set(last.inverse);
        Ok(last.change)
    }

    /// Stamp the staged batch on the tape to take effect on `at_tick` (the
    /// next boundary): never before the live tick or the newest record.
    ///
    /// # Errors
    /// [`SettingsError`] on an empty batch, a past tick or a full tape;
    /// the batch stays staged.
    pub fn apply(&mut self, at_tick: u32) -> Result<SettingsReceipt, SettingsError> {
        if self.staged.is_empty() {
            return Err(SettingsError::NothingStaged);
        }
        self.check_boundary(at_tick)?;
        let changes: Vec<SettingChange> = self.staged.iter().map(|staged| staged.change).collect();
        let inverse: Vec<SettingChange> = self
            .staged
            .iter()
            .rev()
            .map(|staged| staged.inverse)
            .collect();
        let count = changes.len();
        self.tape.record(at_tick, changes)?;
        self.staged.clear();
        self.applied.push(inverse);
        Ok(self.receipt(at_tick, count))
    }

    /// Undo the newest applied batch by stamping its inverse on `at_tick`.
    /// Earlier ticks keep reading the batch (replays do not change).
    ///
    /// # Errors
    /// [`SettingsError`] when nothing was applied, staged edits are pending,
    /// the tick is in the past or the tape is full.
    pub fn undo_applied(&mut self, at_tick: u32) -> Result<SettingsReceipt, SettingsError> {
        if !self.staged.is_empty() {
            return Err(SettingsError::StagedPending);
        }
        let Some(inverse) = self.applied.last() else {
            return Err(SettingsError::NothingToUndo);
        };
        self.check_boundary(at_tick)?;
        let count = inverse.len();
        self.tape.record(at_tick, inverse.clone())?;
        for &change in inverse {
            self.draft.set(change);
        }
        self.applied.pop();
        Ok(self.receipt(at_tick, count))
    }

    fn check_boundary(&self, at_tick: u32) -> Result<(), SettingsError> {
        let earliest = self.cursor.max(self.tape.last_tick());
        if at_tick < earliest {
            return Err(SettingsError::BoundaryInPast { at_tick, earliest });
        }
        Ok(())
    }

    fn receipt(&self, at_tick: u32, changes: usize) -> SettingsReceipt {
        SettingsReceipt {
            at_tick,
            changes,
            tape_digest: self.tape.digest(),
        }
    }
}

/// Pad-only rebind drill: the screen asks for one action at a time in
/// [`BoundAction::ALL`] order and each press binds it, so a full rebind needs
/// no keyboard. Every press is an ordinary staged edit on the panel, so the
/// whole drill lands at one boundary tick and replays from the tape. A press
/// onto a button another action holds is refused naming both actions and the
/// drill keeps asking for the same action.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct RebindDrill {
    next: usize,
}

impl RebindDrill {
    /// Drill at the first action.
    #[must_use]
    pub const fn new() -> Self {
        Self { next: 0 }
    }

    /// Action the next press binds; `None` once every action has its button.
    #[must_use]
    pub const fn awaiting(&self) -> Option<BoundAction> {
        BoundAction::from_index(self.next)
    }

    /// Stage the awaited action onto `button` and move to the next action.
    ///
    /// # Errors
    /// [`SettingsError::ButtonTaken`] naming both actions, a full staged
    /// batch or [`SettingsError::DrillDone`]; the drill does not advance.
    pub fn press(
        &mut self,
        panel: &mut SettingsPanel,
        button: PhysicalButton,
    ) -> Result<BoundAction, SettingsError> {
        let action = self.awaiting().ok_or(SettingsError::DrillDone)?;
        panel.stage(SettingChange::Rebind { action, button })?;
        self.next += 1;
        Ok(action)
    }
}

/// Version of the settings file layout. A newer file is refused with a notice.
/// A future layout adds its migration at the one version boundary in
/// [`read_version`] and [`parse_body`], never inside a field.
pub const SETTINGS_VERSION: u32 = 1;

/// Keys a file carries besides the per-action bindings, in write order.
const FIXED_KEYS: [&str; 8] = [
    "version",
    "volume.master",
    "volume.music",
    "volume.effects",
    "quality",
    "render_scale",
    "vsync",
    "deadzone",
];

/// Why a settings file was not used. The loop shows it as a notice and runs
/// on the defaults.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SettingsFileError {
    /// The bytes are not UTF-8 text.
    NotText,
    /// The version line names a layout this build does not read.
    FutureVersion {
        /// Version the file names.
        found: u32,
    },
    /// The first line is not a readable `version=N` with `N >= 1`.
    BadVersion,
    /// The check line is missing, unreadable or does not match the body.
    Checksum,
    /// A line is not `key=value`, or a key appears twice.
    Malformed {
        /// One-based line number.
        line: usize,
    },
    /// A key this build does not know.
    UnknownField {
        /// One-based line number.
        line: usize,
        /// The unknown key.
        key: String,
    },
    /// A key the layout needs is absent.
    MissingField {
        /// The missing key.
        key: String,
    },
    /// A value does not parse, or sits outside its bound.
    BadValue {
        /// One-based line number.
        line: usize,
    },
    /// Two actions share one button: the action named is the later one.
    ButtonShared {
        /// The later action, whose button an earlier action already holds.
        action: BoundAction,
    },
}

impl core::fmt::Display for SettingsFileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotText => write!(f, "settings file is not text"),
            Self::FutureVersion { found } => write!(
                f,
                "settings file is version {found}; this build reads version {SETTINGS_VERSION}"
            ),
            Self::BadVersion => write!(f, "settings file has no readable version line"),
            Self::Checksum => write!(
                f,
                "settings file check line does not match (edited or torn)"
            ),
            Self::Malformed { line } => {
                write!(
                    f,
                    "settings file line {line} is not key=value or repeats a key"
                )
            }
            Self::UnknownField { line, key } => {
                write!(f, "settings file line {line} has unknown field {key}")
            }
            Self::MissingField { key } => write!(f, "settings file has no {key} field"),
            Self::BadValue { line } => {
                write!(f, "settings file line {line} has a value out of range")
            }
            Self::ButtonShared { action } => write!(
                f,
                "settings file binds {action:?} to a button another action holds"
            ),
        }
    }
}

impl std::error::Error for SettingsFileError {}

/// What a run starts with: the settings to use, and the notice for a file
/// that was not used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    /// Settings the run starts on: the file's, or the defaults.
    pub settings: Settings,
    /// Why the file was not used. `None` for a clean file and for a first run
    /// with no file.
    pub notice: Option<SettingsFileError>,
}

/// Pick the startup settings. `None` is a first run with no file: the
/// defaults and no notice. An unreadable file gives the defaults and its
/// [`SettingsFileError`]. Pure.
#[must_use]
pub fn startup(file: Option<&[u8]>) -> Startup {
    match file.map(decode) {
        None => Startup {
            settings: Settings::default(),
            notice: None,
        },
        Some(Ok(settings)) => Startup {
            settings,
            notice: None,
        },
        Some(Err(error)) => Startup {
            settings: Settings::default(),
            notice: Some(error),
        },
    }
}

/// Canonical file bytes: a `version=` line, one `key=value` line per field in
/// fixed order, then `check=` with the FNV-1a64 of every byte before it. The
/// same settings always give the same bytes. Pure.
#[must_use]
pub fn encode(settings: &Settings) -> Vec<u8> {
    let mut body = String::new();
    push_line(&mut body, "version", SETTINGS_VERSION);
    push_line(
        &mut body,
        "volume.master",
        settings.volume(VolumeBus::Master),
    );
    push_line(&mut body, "volume.music", settings.volume(VolumeBus::Music));
    push_line(
        &mut body,
        "volume.effects",
        settings.volume(VolumeBus::Effects),
    );
    push_line(&mut body, "quality", settings.quality.word());
    push_line(&mut body, "render_scale", settings.render_scale_pct);
    push_line(
        &mut body,
        "vsync",
        if settings.vsync { "on" } else { "off" },
    );
    push_line(&mut body, "deadzone", settings.bindings.deadzone());
    for action in BoundAction::ALL {
        let button = settings.bindings.button(action);
        push_line(
            &mut body,
            &format!("bind.{}", action.index()),
            format!("{}:{}", button.kind(), button.code()),
        );
    }
    let check = fnv_bytes(body.as_bytes());
    push_line(&mut body, "check", format!("{check:016x}"));
    body.into_bytes()
}

/// Read canonical bytes back. Any deviation is refused with a
/// [`SettingsFileError`]; nothing is guessed. Pure.
///
/// # Errors
/// [`SettingsFileError`] for a file that names another version, is not text,
/// fails its check line, or has a missing, unknown, repeated or out-of-range
/// field.
pub fn decode(bytes: &[u8]) -> Result<Settings, SettingsFileError> {
    read_version(bytes)?;
    let text = core::str::from_utf8(bytes).map_err(|_| SettingsFileError::NotText)?;
    parse_body(verified_body(text)?)
}

/// Append one `key=value` line.
fn push_line(out: &mut String, key: &str, value: impl core::fmt::Display) {
    out.push_str(key);
    out.push('=');
    out.push_str(&value.to_string());
    out.push('\n');
}

/// FNV-1a64 of file bytes: the check line that catches edited or torn files.
fn fnv_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(bytes);
    hasher.finish()
}

/// The first line must name a layout this build reads. Checked before the
/// check line, so a newer file reports its version, not a checksum.
fn read_version(bytes: &[u8]) -> Result<(), SettingsFileError> {
    let first_line = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let digits = first_line
        .strip_prefix(b"version=")
        .ok_or(SettingsFileError::BadVersion)?;
    let found = core::str::from_utf8(digits)
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
        .ok_or(SettingsFileError::BadVersion)?;
    match found.cmp(&SETTINGS_VERSION) {
        core::cmp::Ordering::Greater => Err(SettingsFileError::FutureVersion { found }),
        core::cmp::Ordering::Equal => Ok(()),
        // No file ever wrote version 0, and no older layout exists to migrate from.
        core::cmp::Ordering::Less => Err(SettingsFileError::BadVersion),
    }
}

/// Split the check line off `text` and verify it against the body before it.
/// Returns the verified body. Pure.
fn verified_body(text: &str) -> Result<&str, SettingsFileError> {
    let unterminated = text.strip_suffix('\n').ok_or(SettingsFileError::Checksum)?;
    let start = unterminated.rfind('\n').map_or(0, |index| index + 1);
    let body = text.get(..start).ok_or(SettingsFileError::Checksum)?;
    let check_line = text.get(start..).ok_or(SettingsFileError::Checksum)?;
    let hex = check_line
        .strip_prefix("check=")
        .and_then(|rest| rest.strip_suffix('\n'))
        .ok_or(SettingsFileError::Checksum)?;
    let stored = u64::from_str_radix(hex, 16).map_err(|_| SettingsFileError::Checksum)?;
    if stored == fnv_bytes(body.as_bytes()) {
        Ok(body)
    } else {
        Err(SettingsFileError::Checksum)
    }
}

/// Fold the body lines into settings. Every key appears once, every key is
/// known, and every value passes the same checks as an edit. Pure.
fn parse_body(body: &str) -> Result<Settings, SettingsFileError> {
    let mut settings = Settings::default();
    let mut seen: Vec<&str> = Vec::new();
    for (index, raw) in body.lines().enumerate() {
        let line = index + 1;
        let (key, value) = raw
            .split_once('=')
            .ok_or(SettingsFileError::Malformed { line })?;
        if seen.contains(&key) {
            return Err(SettingsFileError::Malformed { line });
        }
        seen.push(key);
        match parse_field(key, value) {
            Ok(Field::Version) => {}
            Ok(Field::Change(change)) => {
                // Rebinds are checked against each other below, not the defaults.
                let rebind = matches!(change, SettingChange::Rebind { .. });
                if !rebind && settings.check(change).is_err() {
                    return Err(SettingsFileError::BadValue { line });
                }
                settings.set(change);
            }
            Err(FieldFault::Unknown) => {
                return Err(SettingsFileError::UnknownField {
                    line,
                    key: key.to_owned(),
                });
            }
            Err(FieldFault::BadValue) => return Err(SettingsFileError::BadValue { line }),
        }
    }
    for key in known_keys() {
        if !seen.contains(&key.as_str()) {
            return Err(SettingsFileError::MissingField { key });
        }
    }
    check_buttons(&settings.bindings)?;
    Ok(settings)
}

/// What one parsed line means.
enum Field {
    /// The version line, already checked by [`read_version`].
    Version,
    /// A line that sets one value.
    Change(SettingChange),
}

/// Why one line did not parse.
enum FieldFault {
    /// The key is not in this layout.
    Unknown,
    /// The key is known but its value is not.
    BadValue,
}

/// Parse one `key=value` pair into its change. Pure.
fn parse_field(key: &str, value: &str) -> Result<Field, FieldFault> {
    let change = match key {
        "version" => return Ok(Field::Version),
        "volume.master" => SettingChange::Volume {
            bus: VolumeBus::Master,
            permille: number(value)?,
        },
        "volume.music" => SettingChange::Volume {
            bus: VolumeBus::Music,
            permille: number(value)?,
        },
        "volume.effects" => SettingChange::Volume {
            bus: VolumeBus::Effects,
            permille: number(value)?,
        },
        "quality" => SettingChange::Quality(Quality::from_word(value).ok_or(FieldFault::BadValue)?),
        "render_scale" => SettingChange::RenderScale(number(value)?),
        "vsync" => SettingChange::Vsync(match value {
            "on" => true,
            "off" => false,
            _ => return Err(FieldFault::BadValue),
        }),
        "deadzone" => SettingChange::Deadzone(number(value)?),
        _ => {
            let index = key.strip_prefix("bind.").ok_or(FieldFault::Unknown)?;
            let action = index
                .parse::<usize>()
                .ok()
                .and_then(BoundAction::from_index)
                .ok_or(FieldFault::Unknown)?;
            let (kind, code) = value.split_once(':').ok_or(FieldFault::BadValue)?;
            let button = PhysicalButton::from_parts(number(kind)?, number(code)?)
                .ok_or(FieldFault::BadValue)?;
            SettingChange::Rebind { action, button }
        }
    };
    Ok(Field::Change(change))
}

/// Parse one number; text that does not parse is a bad value. Pure.
fn number<T: core::str::FromStr>(text: &str) -> Result<T, FieldFault> {
    text.parse::<T>().ok().ok_or(FieldFault::BadValue)
}

/// Every key a complete file carries.
fn known_keys() -> Vec<String> {
    let mut keys: Vec<String> = FIXED_KEYS.iter().map(|key| (*key).to_owned()).collect();
    keys.extend(
        BoundAction::ALL
            .iter()
            .map(|action| format!("bind.{}", action.index())),
    );
    keys
}

/// One button drives one action: refuse a file where an earlier action
/// already holds the later action's button.
fn check_buttons(bindings: &Bindings) -> Result<(), SettingsFileError> {
    for (index, action) in BoundAction::ALL.iter().enumerate() {
        for later in &BoundAction::ALL[index + 1..] {
            if bindings.button(*action) == bindings.button(*later) {
                return Err(SettingsFileError::ButtonShared { action: *later });
            }
        }
    }
    Ok(())
}

/// Every value of `settings` as one batch of edits, so one tape record
/// replays them from the defaults. Pure.
fn snapshot(settings: &Settings) -> Vec<SettingChange> {
    let mut changes: Vec<SettingChange> = VolumeBus::ALL
        .iter()
        .map(|&bus| SettingChange::Volume {
            bus,
            permille: settings.volume(bus),
        })
        .collect();
    changes.push(SettingChange::Quality(settings.quality));
    changes.push(SettingChange::RenderScale(settings.render_scale_pct));
    changes.push(SettingChange::Vsync(settings.vsync));
    changes.push(SettingChange::Deadzone(settings.bindings.deadzone()));
    changes.extend(
        BoundAction::ALL
            .iter()
            .map(|&action| SettingChange::Rebind {
                action,
                button: settings.bindings.button(action),
            }),
    );
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::fighter::ATTACK;
    use engine_input::{Key, ACTION_COUNT};

    const FREE_KEY: PhysicalButton = PhysicalButton::Key(200);

    fn volume(bus: VolumeBus, permille: u16) -> SettingChange {
        SettingChange::Volume { bus, permille }
    }

    /// One edit per kind, each one different from the default.
    fn every_kind() -> [SettingChange; 6] {
        [
            volume(VolumeBus::Music, 100),
            SettingChange::Quality(Quality::Low),
            SettingChange::RenderScale(75),
            SettingChange::Vsync(false),
            SettingChange::Rebind {
                action: BoundAction::Attack,
                button: FREE_KEY,
            },
            SettingChange::Deadzone(20),
        ]
    }

    #[test]
    fn out_of_range_edits_are_refused_and_change_nothing() {
        let mut panel = SettingsPanel::new();
        assert_eq!(
            panel.stage(volume(VolumeBus::Master, VOLUME_MAX + 1)),
            Err(SettingsError::VolumeOutOfRange { permille: 1001 })
        );
        assert_eq!(
            panel.stage(SettingChange::RenderScale(49)),
            Err(SettingsError::RenderScaleOutOfRange { pct: 49 })
        );
        assert_eq!(
            panel.stage(SettingChange::RenderScale(101)),
            Err(SettingsError::RenderScaleOutOfRange { pct: 101 })
        );
        assert_eq!(
            panel.stage(SettingChange::Deadzone(DEADZONE_MAX + 1)),
            Err(SettingsError::DeadzoneOutOfRange { deadzone: 51 })
        );
        assert_eq!(panel.staged_len(), 0);
        assert_eq!(panel.draft(), &Settings::default());
        panel.stage(volume(VolumeBus::Master, VOLUME_MAX)).unwrap();
        panel
            .stage(SettingChange::RenderScale(RENDER_SCALE_MIN))
            .unwrap();
        assert_eq!(panel.staged_len(), 2);
    }

    #[test]
    fn an_apply_goes_live_exactly_on_its_boundary_tick() {
        let mut panel = SettingsPanel::new();
        panel.stage(volume(VolumeBus::Music, 100)).unwrap();
        assert_eq!(
            panel.live().volume(VolumeBus::Music),
            600,
            "staged is not live"
        );
        assert_eq!(panel.draft().volume(VolumeBus::Music), 100);
        let receipt = panel.apply(10).unwrap();
        assert_eq!((receipt.at_tick, receipt.changes), (10, 1));
        panel.advance(9).unwrap();
        assert_eq!(panel.live().volume(VolumeBus::Music), 600);
        panel.advance(10).unwrap();
        assert_eq!(panel.live().volume(VolumeBus::Music), 100);
        assert_eq!(panel.tape().settings_at(9).volume(VolumeBus::Music), 600);
        assert_eq!(panel.tape().settings_at(10).volume(VolumeBus::Music), 100);
    }

    #[test]
    fn a_boundary_in_the_past_is_refused_and_the_batch_stays_staged() {
        let mut panel = SettingsPanel::new();
        panel.advance(20).unwrap();
        panel.stage(SettingChange::Vsync(false)).unwrap();
        assert_eq!(
            panel.apply(5),
            Err(SettingsError::BoundaryInPast {
                at_tick: 5,
                earliest: 20
            })
        );
        assert_eq!(panel.staged_len(), 1);
        assert!(panel.tape().is_empty());
        assert_eq!(
            panel.advance(19),
            Err(SettingsError::TickWentBack {
                cursor: 20,
                tick: 19
            })
        );
        panel.apply(20).unwrap();
        // A later record cannot be stamped before an earlier one.
        panel.stage(SettingChange::Vsync(true)).unwrap();
        panel.apply(30).unwrap();
        panel.stage(SettingChange::Quality(Quality::Low)).unwrap();
        assert_eq!(
            panel.apply(25),
            Err(SettingsError::BoundaryInPast {
                at_tick: 25,
                earliest: 30
            })
        );
    }

    /// The same session twice: same tick-by-tick settings, same digest.
    fn session() -> SettingsPanel {
        let mut panel = SettingsPanel::new();
        for (at_tick, batch) in [
            (
                4,
                vec![volume(VolumeBus::Master, 300), SettingChange::Vsync(false)],
            ),
            (
                12,
                vec![SettingChange::Rebind {
                    action: BoundAction::Dodge,
                    button: FREE_KEY,
                }],
            ),
            (12, vec![SettingChange::Quality(Quality::Medium)]),
        ] {
            for change in batch {
                panel.stage(change).unwrap();
            }
            panel.apply(at_tick).unwrap();
        }
        panel.advance(8).unwrap();
        panel
    }

    #[test]
    fn replay_reads_the_same_settings_on_the_same_tick_twice() {
        let first = session();
        let second = session();
        assert_eq!(first.tape().digest(), second.tape().digest());
        for tick in 0..40 {
            assert_eq!(
                first.tape().settings_at(tick),
                second.tape().settings_at(tick)
            );
        }
        assert_eq!(first.tape().settings_at(3), Settings::default());
        assert_eq!(first.tape().settings_at(4).volume(VolumeBus::Master), 300);
        assert!(!first.tape().settings_at(4).vsync);
        assert_eq!(first.tape().settings_at(12).quality, Quality::Medium);
        let mut other = SettingsPanel::new();
        other.stage(SettingChange::Vsync(false)).unwrap();
        other.apply(4).unwrap();
        assert_ne!(other.tape().digest(), first.tape().digest());
    }

    #[test]
    fn a_staged_edit_is_undone_off_the_draft() {
        let mut panel = SettingsPanel::new();
        assert_eq!(panel.undo_staged(), Err(SettingsError::NothingStaged));
        for change in every_kind() {
            panel.stage(change).unwrap();
        }
        assert_ne!(panel.draft(), panel.live());
        for change in every_kind().into_iter().rev() {
            assert_eq!(panel.undo_staged(), Ok(change));
        }
        assert_eq!(panel.draft(), &Settings::default());
        assert_eq!(panel.staged_len(), 0);
    }

    #[test]
    fn every_kind_of_applied_edit_is_undone_and_history_stays_replayable() {
        for change in every_kind() {
            let mut panel = SettingsPanel::new();
            panel.stage(change).unwrap();
            panel.apply(5).unwrap();
            let after_apply = panel.tape().digest();
            let undone = panel.undo_applied(8).unwrap();
            assert_ne!(undone.tape_digest, after_apply, "{change:?}");
            panel.advance(7).unwrap();
            assert_ne!(
                panel.live(),
                &Settings::default(),
                "{change:?} stays until its undo tick"
            );
            panel.advance(8).unwrap();
            assert_eq!(panel.live(), &Settings::default(), "{change:?} undone");
            assert_eq!(panel.draft(), &Settings::default());
            assert_eq!(panel.undoable(), 0);
        }
    }

    #[test]
    fn a_batch_of_edits_undoes_in_one_step_in_reverse_order() {
        let mut panel = SettingsPanel::new();
        // Two edits on one field: only the reverse order restores the default.
        panel.stage(volume(VolumeBus::Master, 100)).unwrap();
        panel.stage(volume(VolumeBus::Master, 900)).unwrap();
        for change in every_kind() {
            panel.stage(change).unwrap();
        }
        panel.apply(3).unwrap();
        assert_eq!(panel.undoable(), 1);
        panel.undo_applied(6).unwrap();
        panel.advance(6).unwrap();
        assert_eq!(panel.live(), &Settings::default());
        assert_eq!(panel.undo_applied(6), Err(SettingsError::NothingToUndo));
    }

    #[test]
    fn two_applies_undo_one_at_a_time_newest_first() {
        let mut panel = SettingsPanel::new();
        panel.stage(SettingChange::Quality(Quality::Low)).unwrap();
        panel.apply(2).unwrap();
        panel
            .stage(SettingChange::Quality(Quality::Medium))
            .unwrap();
        panel.apply(4).unwrap();
        panel.undo_applied(6).unwrap();
        panel.advance(6).unwrap();
        assert_eq!(panel.live().quality, Quality::Low);
        panel.undo_applied(7).unwrap();
        panel.advance(7).unwrap();
        assert_eq!(panel.live().quality, Quality::High);
        assert_eq!(panel.tape().len(), 4);
    }

    /// Seven distinct pad buttons, one per action in table order.
    fn pad_presses() -> [PhysicalButton; ACTION_COUNT] {
        core::array::from_fn(|index| PhysicalButton::Pad(u8::try_from(index).unwrap()))
    }

    /// Run the whole drill from a fresh panel and apply it on tick `at`.
    fn pad_drill_panel(at: u32) -> SettingsPanel {
        let mut panel = SettingsPanel::new();
        let mut drill = RebindDrill::new();
        for (index, button) in pad_presses().into_iter().enumerate() {
            assert_eq!(drill.awaiting(), BoundAction::from_index(index));
            let bound = drill.press(&mut panel, button).unwrap();
            assert_eq!(Some(bound), BoundAction::from_index(index));
        }
        assert_eq!(drill.awaiting(), None);
        panel.apply(at).unwrap();
        panel
    }

    #[test]
    fn a_full_rebind_needs_only_the_pad_and_goes_live_on_its_boundary() {
        let mut panel = pad_drill_panel(20);
        panel.advance(19).unwrap();
        assert_eq!(
            panel.live().bindings.button(BoundAction::Attack),
            PhysicalButton::Key(Key::F.0),
            "staged and stamped is not live before the boundary"
        );
        panel.advance(20).unwrap();
        for (action, button) in BoundAction::ALL.into_iter().zip(pad_presses()) {
            assert_eq!(panel.live().bindings.button(action), button);
        }
        // The input layer reads the pad press as the action; the old key is dead.
        let pad = panel.live().bindings.button_bits(&[PhysicalButton::Pad(0)]);
        assert_ne!(pad.0 & ATTACK, 0);
        let key = panel
            .live()
            .bindings
            .button_bits(&[PhysicalButton::Key(Key::F.0)]);
        assert_eq!(key.0 & ATTACK, 0);
    }

    #[test]
    fn the_pad_drill_replays_exactly_twice() {
        let first = pad_drill_panel(20);
        let second = pad_drill_panel(20);
        assert_eq!(first.tape().digest(), second.tape().digest());
        assert_eq!(first.tape(), second.tape());
        assert_eq!(
            first.tape().settings_at(20),
            second.tape().settings_at(20),
            "the tape folds to the same bindings on the same tick"
        );
        assert_eq!(first.tape().settings_at(19), Settings::default());
        // An undo stamps the inverse on the tape; earlier ticks still replay.
        let mut undone = first.clone();
        undone.undo_applied(30).unwrap();
        assert_eq!(undone.tape().settings_at(25), first.tape().settings_at(20));
        assert_eq!(undone.tape().settings_at(30), Settings::default());
    }

    #[test]
    fn a_pad_button_another_action_holds_is_refused_naming_both() {
        let mut panel = pad_drill_panel(5);
        panel.advance(5).unwrap();
        let mut drill = RebindDrill::new();
        // Attack asks for Block's pad button.
        let err = drill.press(&mut panel, PhysicalButton::Pad(1)).unwrap_err();
        assert_eq!(
            err,
            SettingsError::ButtonTaken {
                action: BoundAction::Block,
                wanted: BoundAction::Attack,
            }
        );
        let text = err.to_string();
        assert!(text.contains("Block") && text.contains("Attack"), "{text}");
        // Refused: nothing staged, the drill still asks for Attack.
        assert_eq!(panel.staged_len(), 0);
        assert_eq!(drill.awaiting(), Some(BoundAction::Attack));
        assert_eq!(
            drill.press(&mut panel, PhysicalButton::Pad(9)),
            Ok(BoundAction::Attack)
        );
        assert_eq!(drill.awaiting(), Some(BoundAction::Block));
    }

    #[test]
    fn a_finished_drill_refuses_more_presses() {
        let mut panel = SettingsPanel::new();
        let mut drill = RebindDrill::new();
        for button in pad_presses() {
            drill.press(&mut panel, button).unwrap();
        }
        assert_eq!(
            drill.press(&mut panel, PhysicalButton::Pad(40)),
            Err(SettingsError::DrillDone)
        );
        assert_eq!(panel.staged_len(), ACTION_COUNT);
    }

    #[test]
    fn undo_waits_for_staged_edits() {
        let mut panel = SettingsPanel::new();
        panel.stage(SettingChange::Vsync(false)).unwrap();
        panel.apply(1).unwrap();
        panel.stage(SettingChange::Vsync(true)).unwrap();
        assert_eq!(panel.undo_applied(2), Err(SettingsError::StagedPending));
        panel.undo_staged().unwrap();
        panel.undo_applied(2).unwrap();
    }

    #[test]
    fn a_rebind_onto_a_taken_button_is_refused_and_a_free_one_reaches_the_input_layer() {
        let mut panel = SettingsPanel::new();
        assert_eq!(
            panel.stage(SettingChange::Rebind {
                action: BoundAction::MoveBack,
                button: PhysicalButton::Key(Key::W.0),
            }),
            Err(SettingsError::ButtonTaken {
                action: BoundAction::MoveForward,
                wanted: BoundAction::MoveBack,
            })
        );
        // Rebinding an action onto its own button is a no-op, not a clash.
        panel
            .stage(SettingChange::Rebind {
                action: BoundAction::Attack,
                button: PhysicalButton::Key(Key::F.0),
            })
            .unwrap();
        panel
            .stage(SettingChange::Rebind {
                action: BoundAction::Attack,
                button: FREE_KEY,
            })
            .unwrap();
        // The freed default button is free for another action in the same batch.
        panel
            .stage(SettingChange::Rebind {
                action: BoundAction::Block,
                button: PhysicalButton::Key(Key::F.0),
            })
            .unwrap();
        panel.apply(1).unwrap();
        panel.advance(1).unwrap();
        let bindings = panel.live().bindings;
        assert_eq!(bindings.button_bits(&[FREE_KEY]).0 & ATTACK, ATTACK);
        assert_eq!(
            bindings.button_bits(&[PhysicalButton::Key(Key::F.0)]).0 & ATTACK,
            0
        );
        assert_eq!(
            bindings.button(BoundAction::Block),
            PhysicalButton::Key(Key::F.0)
        );
    }

    #[test]
    fn the_staged_batch_and_the_tape_are_capped() {
        let mut panel = SettingsPanel::new();
        for index in 0..MAX_STAGED {
            panel
                .stage(volume(VolumeBus::Effects, u16::try_from(index).unwrap()))
                .unwrap();
        }
        assert_eq!(
            panel.stage(SettingChange::Vsync(false)),
            Err(SettingsError::StagedFull)
        );
        assert_eq!(panel.staged_len(), MAX_STAGED);
        let mut tape = SettingsTape::new();
        for tick in 0..MAX_TAPE_RECORDS {
            tape.record(
                u32::try_from(tick).unwrap(),
                vec![SettingChange::Vsync(false)],
            )
            .unwrap();
        }
        assert_eq!(tape.record(999, vec![]), Err(SettingsError::TapeFull));
    }

    #[test]
    fn deadzone_edits_reach_the_bindings_the_sim_reads() {
        let mut panel = SettingsPanel::new();
        panel.stage(SettingChange::Deadzone(20)).unwrap();
        panel.apply(0).unwrap();
        panel.advance(0).unwrap();
        assert_eq!(panel.live().bindings.deadzone(), 20);
        assert_eq!(
            Settings::default().bindings.deadzone(),
            engine_input::DEFAULT_DEADZONE
        );
    }

    /// Every field off its default, so each one reaches the file.
    fn sample() -> Settings {
        let mut settings = Settings::default();
        settings.set(volume(VolumeBus::Music, 150));
        settings.set(SettingChange::Quality(Quality::Low));
        settings.set(SettingChange::RenderScale(75));
        settings.set(SettingChange::Vsync(false));
        settings.set(SettingChange::Deadzone(20));
        settings.set(SettingChange::Rebind {
            action: BoundAction::Dodge,
            button: FREE_KEY,
        });
        settings
    }

    /// The file text before its check line.
    fn body_of(file: &[u8]) -> String {
        let text = core::str::from_utf8(file).expect("encoded file is text");
        text[..text.rfind("check=").expect("encoded file has a check line")].to_owned()
    }

    /// Bytes with a fresh check line, so an edited body reaches the field parser.
    fn signed(body: &str) -> Vec<u8> {
        format!("{body}check={:016x}\n", fnv_bytes(body.as_bytes())).into_bytes()
    }

    #[test]
    fn a_round_trip_gives_identical_bytes_twice() {
        let settings = sample();
        let first = encode(&settings);
        let back = decode(&first).expect("canonical bytes decode");
        assert_eq!(back, settings);
        let second = encode(&back);
        assert_eq!(first, second);
        assert_eq!(encode(&decode(&second).expect("second decode")), first);
        let defaults = encode(&Settings::default());
        assert_eq!(decode(&defaults), Ok(Settings::default()));
    }

    #[test]
    fn first_run_without_a_file_is_the_defaults_with_no_notice() {
        assert_eq!(
            startup(None),
            Startup {
                settings: Settings::default(),
                notice: None,
            }
        );
    }

    #[test]
    fn a_clean_file_starts_the_saved_settings_with_no_notice() {
        let file = encode(&sample());
        assert_eq!(
            startup(Some(file.as_slice())),
            Startup {
                settings: sample(),
                notice: None,
            }
        );
    }

    #[test]
    fn a_corrupt_file_falls_back_to_defaults_with_a_notice() {
        let file = encode(&sample());
        let body = body_of(&file);
        let text = String::from_utf8(file.clone()).expect("encoded file is text");
        let mut clash = Settings::default();
        let attack = clash.bindings.button(BoundAction::Attack);
        clash.set(SettingChange::Rebind {
            action: BoundAction::Block,
            button: attack,
        });
        let mut not_text = b"version=1\n".to_vec();
        not_text.extend_from_slice(&[0xFF, b'\n']);
        // The sample body is 15 lines, so an appended line is line 16.
        let cases: Vec<(&str, Vec<u8>, SettingsFileError)> = vec![
            (
                "binary garbage",
                vec![0xFF, 0xFE, 0x00],
                SettingsFileError::BadVersion,
            ),
            ("not text", not_text, SettingsFileError::NotText),
            (
                "a flipped digit",
                text.replace("volume.music=150", "volume.music=151")
                    .into_bytes(),
                SettingsFileError::Checksum,
            ),
            (
                "truncated",
                file[..file.len() - 5].to_vec(),
                SettingsFileError::Checksum,
            ),
            (
                "unknown field",
                signed(&format!("{body}volume.extra=5\n")),
                SettingsFileError::UnknownField {
                    line: 16,
                    key: "volume.extra".to_owned(),
                },
            ),
            (
                "malformed line",
                signed(&format!("{body}nonsense\n")),
                SettingsFileError::Malformed { line: 16 },
            ),
            (
                "repeated field",
                signed(&format!("{body}vsync=on\n")),
                SettingsFileError::Malformed { line: 16 },
            ),
            (
                "volume above the bound",
                signed(&body.replace("volume.master=800", "volume.master=1001")),
                SettingsFileError::BadValue { line: 2 },
            ),
            (
                "missing field",
                signed(&body.replace("deadzone=20\n", "")),
                SettingsFileError::MissingField {
                    key: "deadzone".to_owned(),
                },
            ),
            (
                "two actions on one button",
                encode(&clash),
                SettingsFileError::ButtonShared {
                    action: BoundAction::Block,
                },
            ),
        ];
        for (name, bytes, expected) in cases {
            assert_eq!(decode(&bytes), Err(expected.clone()), "{name}");
            assert_eq!(
                startup(Some(bytes.as_slice())),
                Startup {
                    settings: Settings::default(),
                    notice: Some(expected),
                },
                "{name}"
            );
        }
    }

    #[test]
    fn a_future_version_falls_back_to_defaults_and_names_its_version() {
        let text = String::from_utf8(encode(&sample())).expect("encoded file is text");
        let newer = text.replacen("version=1", "version=2", 1).into_bytes();
        let loaded = startup(Some(newer.as_slice()));
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(
            loaded.notice,
            Some(SettingsFileError::FutureVersion { found: 2 })
        );
        let zero = text.replacen("version=1", "version=0", 1).into_bytes();
        assert_eq!(decode(&zero), Err(SettingsFileError::BadVersion));
    }

    #[test]
    fn a_changed_setting_still_replays_exactly_after_a_restart() {
        let mut first = SettingsPanel::new();
        first
            .stage(SettingChange::Quality(Quality::Low))
            .expect("quality stages");
        first
            .stage(volume(VolumeBus::Music, 150))
            .expect("volume stages");
        first
            .stage(SettingChange::Rebind {
                action: BoundAction::Dodge,
                button: FREE_KEY,
            })
            .expect("rebind stages");
        first.apply(10).expect("batch applies at tick 10");
        first.advance(12).expect("view moves to 12");
        let file = encode(first.live());

        let saved = decode(&file).expect("saved file decodes");
        let mut second = SettingsPanel::from_saved(saved);
        second.advance(12).expect("restart view moves to 12");
        assert_eq!(second.live(), first.live());
        assert_eq!(
            encode(second.live()),
            file,
            "the file reads back identically"
        );
        for tick in 10..40 {
            assert_eq!(
                second.tape().settings_at(tick),
                first.tape().settings_at(tick),
                "tick {tick} replays the same after a restart"
            );
        }

        // A new edit after the restart still lands on its own boundary tick.
        second
            .stage(SettingChange::Vsync(false))
            .expect("vsync stages");
        second.apply(20).expect("batch applies at tick 20");
        second.advance(20).expect("view moves to 20");
        assert!(!second.live().vsync);
        assert_eq!(second.tape().settings_at(19), saved);
    }
}
