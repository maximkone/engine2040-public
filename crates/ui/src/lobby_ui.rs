//! lobby_ui: the LAN lobby screen as data: browse hosts, join, ready up, start.
//!
//! Consumer: the windowed loop draws [`LobbyScreen::hosts`] and
//! [`LobbyScreen::slots`], feeds one [`LobbyInput`] per press and runs each
//! returned [`LobbyCommand`] against the net lobby. The game layer owns the
//! sockets: this module never sees an address, a port or a typed string. A
//! host is picked by list position and carried as a [`HostId`] the game layer
//! maps back to the sender it learned from the beacon datagram, so two
//! machines form a fight with no address typed. Data owner: `engine-ui`.
//! Units: wall time is whole milliseconds handed in by the caller (never read
//! here), slots are in lobby order. Determinism: every transition is a pure
//! function of the screen state and one input or event; no clock, no IO.
//! Errors: [`LobbyUiError`]; a refusal changes nothing. Budget: constant time
//! per input, the host list is capped by [`LobbyLimits::max_hosts`] and the
//! slot list by the canonical `engine-input` fighter cap.
//!
//! Pacing and caps come in through [`LobbyLimits`] from the net lobby's own
//! constants, so this screen holds no mirror of them.

use super::menus::{FightRequest, MenuError};
use engine_input::MAX_PARTICIPANTS;

/// Caps and pacing the game layer takes from the net lobby.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LobbyLimits {
    /// Whole milliseconds between host-list refreshes, nonzero.
    pub refresh_period_ms: u64,
    /// Most hosts the list holds, nonzero.
    pub max_hosts: usize,
}

/// One host in the list, named by position in the sightings the game layer
/// keeps. Carries no address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HostId(pub u16);

/// One host row to draw.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HostRow {
    /// Handle the join command carries back.
    pub id: HostId,
    /// Slots in that host's match.
    pub fighters: u8,
}

/// One lobby slot to draw.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SlotRow {
    /// Beyond the match size; blocks start if it is shown.
    Closed,
    /// Waiting for a human or an AI fill.
    Open,
    /// AI-filled fighter.
    Ai,
    /// Human holder.
    Human {
        /// This screen's player.
        you: bool,
        /// Ready flag.
        ready: bool,
    },
}

/// Rows drawn per frame; the screen virtualizes the slot list through this window.
pub const LOBBY_VIEW_ROWS: usize = 8;
/// Draw units per slot row (status dot plus label plus ready flag).
pub const SLOT_ROW_UNITS: usize = 3;

/// Scale numbers for a slot list: slots, bytes, draw units. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LobbyScale {
    /// Slots counted.
    pub slots: usize,
    /// Bytes held for those slots.
    pub bytes: usize,
    /// Draw units those slots cost.
    pub units: usize,
}

impl LobbyScale {
    /// Scale for `slots` rows. Pure, no allocation.
    #[must_use]
    pub fn for_slots(slots: usize) -> Self {
        Self {
            slots,
            bytes: slots.saturating_mul(core::mem::size_of::<SlotRow>()),
            units: slots.saturating_mul(SLOT_ROW_UNITS),
        }
    }
}

/// Party board point in the ludus hub (`map.json` party_board interact
/// board x -13 z 1.2) in whole millimetres, plus the party size the queue
/// admits. Private: the screen never mirrors net constants, it only gates
/// the queue press on the same board and size.
const PARTY_SIZE: usize = 3;
const PARTY_BOARD_X_MM: i32 = -13_000;
const PARTY_BOARD_Z_MM: i32 = 1_200;

fn party_board_ok(x_mm: i32, z_mm: i32) -> bool {
    x_mm == PARTY_BOARD_X_MM && z_mm == PARTY_BOARD_Z_MM
}

/// Where the screen is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LobbyPhase {
    /// Looking at the host list.
    Browsing,
    /// Join sent to this host, waiting for the grant.
    Joining(HostId),
    /// Opening a game of our own, waiting for the lobby to form.
    Hosting,
    /// In a lobby: ready up, fill, start.
    InLobby,
    /// The fight request is out; the loop takes it with
    /// [`LobbyScreen::take_fight`].
    Starting,
}

/// One press on the lobby screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LobbyInput {
    /// Move the host highlight up.
    Up,
    /// Move the host highlight down.
    Down,
    /// Join the highlighted host.
    Confirm,
    /// Open a game of our own (the lobby host).
    Host,
    /// Leave the lobby, cancel a join or leave the screen.
    Back,
    /// Flip this player's ready flag.
    ToggleReady,
    /// Host only: fill open slots with AI.
    FillAi,
    /// Host only: start once every slot is filled and every human is ready.
    Start,
    /// Host only: queue the board party of 3 into one match.
    QueueParty,
}

/// What the game layer runs against the net lobby. No variant carries an
/// address or a string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LobbyCommand {
    /// Poll the beacon browser (paced by [`LobbyLimits::refresh_period_ms`]).
    Refresh,
    /// Join the named host.
    Join(HostId),
    /// Form a lobby and beacon it; the game layer answers with
    /// [`LobbyScreen::joined`] once the lobby exists.
    Host,
    /// Release the slot or the pending join.
    Leave,
    /// Post this player's ready flag.
    SetReady(bool),
    /// Host: AI-fill the open slots.
    FillAi,
    /// Host: try to start the match.
    Start,
    /// Host: queue the board party of 3 into one match.
    QueueParty,
    /// Leave the lobby screen for the menu.
    Exit,
}

/// What refused a lobby screen call. Refusals change nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LobbyUiError {
    /// Zero refresh period or zero host cap.
    BadLimits,
    /// More hosts than the cap, count carried.
    TooManyHosts(usize),
    /// More slots than the canonical fighter cap, count carried.
    TooManySlots(usize),
    /// Join with no host in the list.
    NoHost,
    /// Fill or start by a guest.
    NotHost,
    /// Start before every slot is filled and every human is ready.
    NotReady,
    /// The slot list has no row for this player.
    NoSlotForYou,
    /// The press or event does not fit the phase; phase carried.
    WrongPhase(LobbyPhase),
    /// The fight request failed the menu's own check.
    BadFight(MenuError),
}

impl core::fmt::Display for LobbyUiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadLimits => write!(f, "lobby limits need a nonzero period and host cap"),
            Self::TooManyHosts(n) => write!(f, "too many hosts ({n})"),
            Self::TooManySlots(n) => write!(f, "too many slots ({n})"),
            Self::NoHost => write!(f, "no host to join"),
            Self::NotHost => write!(f, "only the host can do that"),
            Self::NotReady => write!(f, "not every slot is filled and ready"),
            Self::NoSlotForYou => write!(f, "no slot for this player"),
            Self::WrongPhase(phase) => write!(f, "not allowed while {phase:?}"),
            Self::BadFight(err) => write!(f, "bad fight request: {err}"),
        }
    }
}

impl std::error::Error for LobbyUiError {}

/// The lobby screen state: host list, highlight, slots and the fight request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LobbyScreen {
    limits: LobbyLimits,
    phase: LobbyPhase,
    hosts: Vec<HostRow>,
    selected: usize,
    slots: Vec<SlotRow>,
    host: bool,
    last_refresh_ms: Option<u64>,
    fight: Option<FightRequest>,
}

impl LobbyScreen {
    /// Fresh screen in the host list; the first [`LobbyScreen::poll`] asks
    /// for a refresh at once.
    ///
    /// # Errors
    /// [`LobbyUiError::BadLimits`] on a zero period or zero host cap.
    pub fn new(limits: LobbyLimits) -> Result<Self, LobbyUiError> {
        if limits.refresh_period_ms == 0 || limits.max_hosts == 0 {
            return Err(LobbyUiError::BadLimits);
        }
        Ok(Self {
            limits,
            phase: LobbyPhase::Browsing,
            hosts: Vec::new(),
            selected: 0,
            slots: Vec::new(),
            host: false,
            last_refresh_ms: None,
            fight: None,
        })
    }

    /// Current phase.
    #[must_use]
    pub fn phase(&self) -> LobbyPhase {
        self.phase
    }

    /// Host rows in list order.
    #[must_use]
    pub fn hosts(&self) -> &[HostRow] {
        &self.hosts
    }

    /// Highlighted host row, if any.
    #[must_use]
    pub fn selected(&self) -> Option<HostRow> {
        self.hosts.get(self.selected).copied()
    }

    /// Slot rows in lobby order (empty outside a lobby).
    #[must_use]
    pub fn slots(&self) -> &[SlotRow] {
        &self.slots
    }

    /// Virtualized window over the slot list: at most [`LOBBY_VIEW_ROWS`]
    /// rows from `offset`. Pure, no allocation; past-end draws nothing.
    #[must_use]
    pub fn visible_slots(&self, offset: usize) -> &[SlotRow] {
        let len = self.slots.len();
        if offset >= len {
            return &[];
        }
        let take = len.saturating_sub(offset).min(LOBBY_VIEW_ROWS);
        let end = offset.saturating_add(take);
        match self.slots.get(offset..end) {
            Some(window) => window,
            None => &[],
        }
    }

    /// True when this player hosts the lobby they are in.
    #[must_use]
    pub fn is_host(&self) -> bool {
        self.host
    }

    /// One-line prompt for the current phase, ASCII.
    #[must_use]
    pub fn status(&self) -> &'static str {
        match self.phase {
            LobbyPhase::Browsing if self.hosts.is_empty() => "Looking for games on this network",
            LobbyPhase::Browsing => "Pick a game and press confirm",
            LobbyPhase::Joining(_) => "Joining",
            LobbyPhase::Hosting => "Opening a game",
            LobbyPhase::InLobby if self.can_start() => "Everyone is ready",
            LobbyPhase::InLobby => "Ready up",
            LobbyPhase::Starting => "Starting",
        }
    }

    /// True when every slot is an AI or a ready human and at least one human
    /// sits in the lobby: what the net lobby needs to start.
    #[must_use]
    pub fn can_start(&self) -> bool {
        debug_assert!(self.visible_slots(0).len() <= LOBBY_VIEW_ROWS);
        let mut humans = 0usize;
        for slot in &self.slots {
            match slot {
                SlotRow::Closed | SlotRow::Open | SlotRow::Human { ready: false, .. } => {
                    return false;
                }
                SlotRow::Human { ready: true, .. } => humans += 1,
                SlotRow::Ai => {}
            }
        }
        humans > 0
    }

    /// True when the board party of 3 is queued: exactly [`PARTY_SIZE`]
    /// ready humans, no open or closed rows, AI fills the rest. The board
    /// point gates the queue: same mm as the net party form-up.
    fn party_ready(&self) -> bool {
        debug_assert!(party_board_ok(PARTY_BOARD_X_MM, PARTY_BOARD_Z_MM));
        let mut humans = 0usize;
        for slot in &self.slots {
            match slot {
                SlotRow::Closed | SlotRow::Open | SlotRow::Human { ready: false, .. } => {
                    return false;
                }
                SlotRow::Human { ready: true, .. } => humans += 1,
                SlotRow::Ai => {}
            }
        }
        humans == PARTY_SIZE
    }

    /// Ask for a host-list refresh at most once per period while browsing.
    /// The first call answers at once; the clock is the caller's.
    pub fn poll(&mut self, now_ms: u64) -> Option<LobbyCommand> {
        if self.phase != LobbyPhase::Browsing {
            return None;
        }
        let due = self
            .last_refresh_ms
            .is_none_or(|last| now_ms.wrapping_sub(last) >= self.limits.refresh_period_ms);
        if !due {
            return None;
        }
        self.last_refresh_ms = Some(now_ms);
        Some(LobbyCommand::Refresh)
    }

    /// Replace the host list after a refresh. The highlight stays on the same
    /// host when it is still listed, else it clamps into the new list.
    ///
    /// # Errors
    /// [`LobbyUiError`] over the cap or outside the host list phase.
    pub fn set_hosts(&mut self, hosts: &[HostRow]) -> Result<(), LobbyUiError> {
        if self.phase != LobbyPhase::Browsing {
            return Err(LobbyUiError::WrongPhase(self.phase));
        }
        if hosts.len() > self.limits.max_hosts {
            return Err(LobbyUiError::TooManyHosts(hosts.len()));
        }
        let kept = self
            .selected()
            .and_then(|row| hosts.iter().position(|other| other.id == row.id));
        self.selected = kept.unwrap_or_else(|| self.selected.min(hosts.len().saturating_sub(1)));
        self.hosts = hosts.to_vec();
        Ok(())
    }

    /// One press. Returns the command the game layer runs, if any.
    ///
    /// # Errors
    /// [`LobbyUiError`] when the press does not fit the phase, no host is
    /// listed, a guest fills or starts, or the lobby is not ready.
    pub fn input(&mut self, input: LobbyInput) -> Result<Option<LobbyCommand>, LobbyUiError> {
        match self.phase {
            LobbyPhase::Browsing => self.input_browsing(input),
            LobbyPhase::Joining(_) | LobbyPhase::Hosting => match input {
                LobbyInput::Back => {
                    self.phase = LobbyPhase::Browsing;
                    Ok(Some(LobbyCommand::Leave))
                }
                _ => Err(LobbyUiError::WrongPhase(self.phase)),
            },
            LobbyPhase::InLobby => self.input_in_lobby(input),
            LobbyPhase::Starting => Err(LobbyUiError::WrongPhase(self.phase)),
        }
    }

    fn input_browsing(&mut self, input: LobbyInput) -> Result<Option<LobbyCommand>, LobbyUiError> {
        match input {
            LobbyInput::Up => {
                self.selected = self.selected.saturating_sub(1);
                Ok(None)
            }
            LobbyInput::Down => {
                self.selected = (self.selected + 1).min(self.hosts.len().saturating_sub(1));
                Ok(None)
            }
            LobbyInput::Confirm => {
                let row = self.selected().ok_or(LobbyUiError::NoHost)?;
                self.phase = LobbyPhase::Joining(row.id);
                Ok(Some(LobbyCommand::Join(row.id)))
            }
            LobbyInput::Host => {
                self.phase = LobbyPhase::Hosting;
                Ok(Some(LobbyCommand::Host))
            }
            LobbyInput::Back => Ok(Some(LobbyCommand::Exit)),
            LobbyInput::ToggleReady
            | LobbyInput::FillAi
            | LobbyInput::Start
            | LobbyInput::QueueParty => Err(LobbyUiError::WrongPhase(self.phase)),
        }
    }

    fn input_in_lobby(&mut self, input: LobbyInput) -> Result<Option<LobbyCommand>, LobbyUiError> {
        match input {
            LobbyInput::Up | LobbyInput::Down | LobbyInput::Confirm | LobbyInput::Host => Ok(None),
            LobbyInput::Back => {
                self.leave_lobby();
                Ok(Some(LobbyCommand::Leave))
            }
            LobbyInput::ToggleReady => {
                let ready = self
                    .slots
                    .iter()
                    .find_map(|slot| match slot {
                        SlotRow::Human { you: true, ready } => Some(*ready),
                        _ => None,
                    })
                    .ok_or(LobbyUiError::NoSlotForYou)?;
                Ok(Some(LobbyCommand::SetReady(!ready)))
            }
            LobbyInput::FillAi if self.host => Ok(Some(LobbyCommand::FillAi)),
            LobbyInput::Start if self.host && self.can_start() => Ok(Some(LobbyCommand::Start)),
            LobbyInput::Start if self.host => Err(LobbyUiError::NotReady),
            LobbyInput::QueueParty if self.host && self.party_ready() => {
                Ok(Some(LobbyCommand::QueueParty))
            }
            LobbyInput::QueueParty if self.host => Err(LobbyUiError::NotReady),
            LobbyInput::FillAi | LobbyInput::Start | LobbyInput::QueueParty => {
                Err(LobbyUiError::NotHost)
            }
        }
    }

    fn leave_lobby(&mut self) {
        self.phase = LobbyPhase::Browsing;
        self.slots.clear();
        self.host = false;
    }

    /// The join was granted: show the lobby. `host` is true when this player
    /// hosts it.
    ///
    /// # Errors
    /// [`LobbyUiError`] outside a pending join or over the slot cap.
    pub fn joined(&mut self, slots: &[SlotRow], host: bool) -> Result<(), LobbyUiError> {
        if !matches!(self.phase, LobbyPhase::Joining(_) | LobbyPhase::Hosting) {
            return Err(LobbyUiError::WrongPhase(self.phase));
        }
        Self::check_slots(slots)?;
        self.slots = slots.to_vec();
        self.host = host;
        self.phase = LobbyPhase::InLobby;
        Ok(())
    }

    /// The join was refused (full, wrong build): back to the host list.
    ///
    /// # Errors
    /// [`LobbyUiError::WrongPhase`] outside a pending join.
    pub fn join_refused(&mut self) -> Result<(), LobbyUiError> {
        if !matches!(self.phase, LobbyPhase::Joining(_) | LobbyPhase::Hosting) {
            return Err(LobbyUiError::WrongPhase(self.phase));
        }
        self.phase = LobbyPhase::Browsing;
        Ok(())
    }

    /// New slot rows from the lobby (a join, a ready flag, an AI fill).
    ///
    /// # Errors
    /// [`LobbyUiError`] outside a lobby or over the slot cap.
    pub fn set_slots(&mut self, slots: &[SlotRow]) -> Result<(), LobbyUiError> {
        if self.phase != LobbyPhase::InLobby {
            return Err(LobbyUiError::WrongPhase(self.phase));
        }
        Self::check_slots(slots)?;
        self.slots = slots.to_vec();
        Ok(())
    }

    fn check_slots(slots: &[SlotRow]) -> Result<(), LobbyUiError> {
        if slots.len() > usize::from(MAX_PARTICIPANTS) {
            return Err(LobbyUiError::TooManySlots(slots.len()));
        }
        let scale = LobbyScale::for_slots(slots.len());
        debug_assert_eq!(scale.slots, slots.len());
        Ok(())
    }

    /// The match start came back from the lobby (host's own start, or the
    /// start message a guest received): hold the fight request for the loop.
    ///
    /// # Errors
    /// [`LobbyUiError`] outside a lobby or on a bad fighter count.
    pub fn started(&mut self, request: FightRequest) -> Result<(), LobbyUiError> {
        if self.phase != LobbyPhase::InLobby {
            return Err(LobbyUiError::WrongPhase(self.phase));
        }
        request.validate().map_err(LobbyUiError::BadFight)?;
        self.fight = Some(request);
        self.phase = LobbyPhase::Starting;
        Ok(())
    }

    /// Hand the fight request to the loop, once.
    pub fn take_fight(&mut self) -> Option<FightRequest> {
        self.fight.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: LobbyLimits = LobbyLimits {
        refresh_period_ms: 250,
        max_hosts: 4,
    };

    fn host(id: u16, fighters: u8) -> HostRow {
        HostRow {
            id: HostId(id),
            fighters,
        }
    }

    fn you(ready: bool) -> SlotRow {
        SlotRow::Human { you: true, ready }
    }

    fn screen() -> LobbyScreen {
        LobbyScreen::new(LIMITS).expect("limits are valid")
    }

    /// Browsing with two hosts, joined to the second as host or guest.
    fn in_lobby(host_flag: bool, slots: &[SlotRow]) -> LobbyScreen {
        let mut screen = screen();
        screen.set_hosts(&[host(10, 2), host(11, 2)]).unwrap();
        screen.input(LobbyInput::Confirm).unwrap();
        screen.joined(slots, host_flag).unwrap();
        screen
    }

    #[test]
    fn limits_must_be_nonzero() {
        let zero_period = LobbyLimits {
            refresh_period_ms: 0,
            ..LIMITS
        };
        let zero_hosts = LobbyLimits {
            max_hosts: 0,
            ..LIMITS
        };
        assert_eq!(LobbyScreen::new(zero_period), Err(LobbyUiError::BadLimits));
        assert_eq!(LobbyScreen::new(zero_hosts), Err(LobbyUiError::BadLimits));
    }

    #[test]
    fn a_pick_by_position_joins_that_host_with_no_address_anywhere() {
        let mut screen = screen();
        assert_eq!(screen.status(), "Looking for games on this network");
        screen
            .set_hosts(&[host(10, 4), host(11, 2), host(12, 8)])
            .unwrap();
        assert_eq!(screen.status(), "Pick a game and press confirm");
        assert_eq!(screen.input(LobbyInput::Down), Ok(None));
        assert_eq!(screen.input(LobbyInput::Down), Ok(None));
        assert_eq!(
            screen.input(LobbyInput::Down),
            Ok(None),
            "clamps at the end"
        );
        assert_eq!(screen.input(LobbyInput::Up), Ok(None));
        assert_eq!(
            screen.input(LobbyInput::Confirm),
            Ok(Some(LobbyCommand::Join(HostId(11))))
        );
        assert_eq!(screen.phase(), LobbyPhase::Joining(HostId(11)));
    }

    #[test]
    fn joining_with_an_empty_list_is_refused_and_changes_nothing() {
        let mut screen = screen();
        let before = screen.clone();
        assert_eq!(screen.input(LobbyInput::Confirm), Err(LobbyUiError::NoHost));
        assert_eq!(screen, before);
    }

    #[test]
    fn refresh_is_asked_once_per_period_and_only_while_browsing() {
        let mut screen = screen();
        assert_eq!(
            screen.poll(1000),
            Some(LobbyCommand::Refresh),
            "first poll at once"
        );
        assert_eq!(screen.poll(1100), None);
        assert_eq!(screen.poll(1249), None);
        assert_eq!(screen.poll(1250), Some(LobbyCommand::Refresh));
        // A 16 ms frame loop for ten seconds: one refresh per 250 ms.
        let mut refreshes = 0u32;
        let mut now = 2000u64;
        while now < 12_000 {
            refreshes += u32::from(screen.poll(now).is_some());
            now += 16;
        }
        assert!(
            (39..=41).contains(&refreshes),
            "{refreshes} refreshes in 10 s"
        );
        // Clock wrap does not stall or storm the refresh.
        let mut wrapped = LobbyScreen::new(LIMITS).unwrap();
        assert!(wrapped.poll(u64::MAX - 10).is_some());
        assert_eq!(wrapped.poll(u64::MAX), None);
        assert_eq!(wrapped.poll(300), Some(LobbyCommand::Refresh));
        // No refresh while joining or in a lobby.
        wrapped.set_hosts(&[host(1, 2)]).unwrap();
        wrapped.input(LobbyInput::Confirm).unwrap();
        assert_eq!(wrapped.poll(10_000), None);
    }

    #[test]
    fn a_refresh_keeps_the_highlight_on_the_same_host_or_clamps() {
        let mut screen = screen();
        screen
            .set_hosts(&[host(1, 2), host(2, 2), host(3, 2)])
            .unwrap();
        screen.input(LobbyInput::Down).unwrap();
        assert_eq!(screen.selected(), Some(host(2, 2)));
        // The list reorders: the highlight follows host 2.
        screen
            .set_hosts(&[host(3, 2), host(1, 2), host(2, 2)])
            .unwrap();
        assert_eq!(screen.selected(), Some(host(2, 2)));
        // Host 2 vanishes and the list shrinks: clamp into it.
        screen.set_hosts(&[host(3, 2)]).unwrap();
        assert_eq!(screen.selected(), Some(host(3, 2)));
        screen.set_hosts(&[]).unwrap();
        assert_eq!(screen.selected(), None);
        assert_eq!(
            screen.set_hosts(&[host(1, 2); 5]),
            Err(LobbyUiError::TooManyHosts(5))
        );
        assert!(screen.hosts().is_empty(), "a refused list changes nothing");
    }

    #[test]
    fn a_refused_join_goes_back_to_the_list_and_back_cancels_with_a_leave() {
        let mut screen = screen();
        screen.set_hosts(&[host(1, 2)]).unwrap();
        screen.input(LobbyInput::Confirm).unwrap();
        assert_eq!(
            screen.input(LobbyInput::Start),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Joining(HostId(1))))
        );
        screen.join_refused().unwrap();
        assert_eq!(screen.phase(), LobbyPhase::Browsing);
        assert_eq!(
            screen.join_refused(),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Browsing))
        );
        screen.input(LobbyInput::Confirm).unwrap();
        assert_eq!(
            screen.input(LobbyInput::Back),
            Ok(Some(LobbyCommand::Leave))
        );
        assert_eq!(screen.phase(), LobbyPhase::Browsing);
        assert_eq!(screen.input(LobbyInput::Back), Ok(Some(LobbyCommand::Exit)));
    }

    #[test]
    fn hosting_opens_a_lobby_as_host_and_back_cancels_it() {
        let mut screen = screen();
        assert_eq!(screen.input(LobbyInput::Host), Ok(Some(LobbyCommand::Host)));
        assert_eq!(screen.phase(), LobbyPhase::Hosting);
        assert_eq!(screen.status(), "Opening a game");
        assert_eq!(screen.poll(5_000), None, "no refresh while opening");
        assert_eq!(
            screen.input(LobbyInput::Start),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Hosting))
        );
        screen.joined(&[you(false), SlotRow::Open], true).unwrap();
        assert!(screen.is_host());
        assert_eq!(screen.phase(), LobbyPhase::InLobby);
        assert_eq!(
            screen.input(LobbyInput::Host),
            Ok(None),
            "already in a lobby"
        );
        screen.input(LobbyInput::Back).unwrap();
        // Back while the lobby is still forming cancels it too.
        screen.input(LobbyInput::Host).unwrap();
        assert_eq!(
            screen.input(LobbyInput::Back),
            Ok(Some(LobbyCommand::Leave))
        );
        assert_eq!(screen.phase(), LobbyPhase::Browsing);
        screen.input(LobbyInput::Host).unwrap();
        screen.join_refused().unwrap();
        assert_eq!(screen.phase(), LobbyPhase::Browsing);
    }

    #[test]
    fn ready_flips_from_the_row_the_lobby_reported() {
        let mut screen = in_lobby(
            false,
            &[
                SlotRow::Human {
                    you: false,
                    ready: true,
                },
                you(false),
            ],
        );
        assert_eq!(screen.status(), "Ready up");
        assert_eq!(
            screen.input(LobbyInput::ToggleReady),
            Ok(Some(LobbyCommand::SetReady(true)))
        );
        screen
            .set_slots(&[
                SlotRow::Human {
                    you: false,
                    ready: true,
                },
                you(true),
            ])
            .unwrap();
        assert_eq!(
            screen.input(LobbyInput::ToggleReady),
            Ok(Some(LobbyCommand::SetReady(false)))
        );
        let mut nobody = in_lobby(false, &[SlotRow::Open, SlotRow::Open]);
        assert_eq!(
            nobody.input(LobbyInput::ToggleReady),
            Err(LobbyUiError::NoSlotForYou)
        );
    }

    #[test]
    fn a_guest_cannot_fill_or_start_and_the_host_waits_for_ready() {
        let all_ready = [you(true), SlotRow::Ai];
        let mut guest = in_lobby(false, &all_ready);
        assert_eq!(guest.input(LobbyInput::FillAi), Err(LobbyUiError::NotHost));
        assert_eq!(guest.input(LobbyInput::Start), Err(LobbyUiError::NotHost));
        let mut host_screen = in_lobby(true, &[you(false), SlotRow::Open]);
        assert!(!host_screen.can_start());
        assert_eq!(
            host_screen.input(LobbyInput::Start),
            Err(LobbyUiError::NotReady)
        );
        assert_eq!(
            host_screen.input(LobbyInput::FillAi),
            Ok(Some(LobbyCommand::FillAi))
        );
        host_screen.set_slots(&[you(false), SlotRow::Ai]).unwrap();
        assert_eq!(
            host_screen.input(LobbyInput::Start),
            Err(LobbyUiError::NotReady)
        );
        host_screen.set_slots(&[you(true), SlotRow::Ai]).unwrap();
        assert_eq!(host_screen.status(), "Everyone is ready");
        assert_eq!(
            host_screen.input(LobbyInput::Start),
            Ok(Some(LobbyCommand::Start))
        );
        // An all-AI lobby never starts: someone human must be in it.
        host_screen.set_slots(&[SlotRow::Ai, SlotRow::Ai]).unwrap();
        assert!(!host_screen.can_start());
        // A closed or open slot blocks too.
        host_screen
            .set_slots(&[you(true), SlotRow::Closed])
            .unwrap();
        assert!(!host_screen.can_start());
    }

    #[test]
    fn start_hands_the_fight_over_once_and_a_bad_request_is_refused() {
        let mut screen = in_lobby(true, &[you(true), SlotRow::Ai]);
        let bad = FightRequest {
            seed: 1,
            fighters: 0,
        };
        assert_eq!(
            screen.started(bad),
            Err(LobbyUiError::BadFight(MenuError::BadFighters(0)))
        );
        assert_eq!(screen.phase(), LobbyPhase::InLobby);
        let request = FightRequest {
            seed: 77,
            fighters: 2,
        };
        screen.started(request).unwrap();
        assert_eq!(screen.phase(), LobbyPhase::Starting);
        assert_eq!(screen.status(), "Starting");
        assert_eq!(
            screen.input(LobbyInput::Back),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Starting))
        );
        assert_eq!(screen.take_fight(), Some(request));
        assert_eq!(screen.take_fight(), None);
    }

    #[test]
    fn back_in_a_lobby_leaves_and_returns_to_the_list() {
        let mut screen = in_lobby(true, &[you(false), SlotRow::Open]);
        assert_eq!(
            screen.input(LobbyInput::Back),
            Ok(Some(LobbyCommand::Leave))
        );
        assert_eq!(screen.phase(), LobbyPhase::Browsing);
        assert!(screen.slots().is_empty());
        assert!(!screen.is_host());
        assert_eq!(
            screen.hosts().len(),
            2,
            "the list is still there to pick again"
        );
    }

    #[test]
    fn events_in_the_wrong_phase_are_refused_and_change_nothing() {
        let mut screen = screen();
        let before = screen.clone();
        assert_eq!(
            screen.joined(&[you(false)], false),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Browsing))
        );
        assert_eq!(
            screen.set_slots(&[you(false)]),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Browsing))
        );
        assert_eq!(
            screen.started(FightRequest::default_fight()),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Browsing))
        );
        assert_eq!(
            screen.input(LobbyInput::ToggleReady),
            Err(LobbyUiError::WrongPhase(LobbyPhase::Browsing))
        );
        assert_eq!(screen, before);
        let mut in_it = in_lobby(true, &[you(false)]);
        assert_eq!(
            in_it.set_hosts(&[]),
            Err(LobbyUiError::WrongPhase(LobbyPhase::InLobby))
        );
        let too_many = vec![SlotRow::Open; usize::from(MAX_PARTICIPANTS) + 1];
        assert_eq!(
            in_it.set_slots(&too_many),
            Err(LobbyUiError::TooManySlots(too_many.len()))
        );
    }

    #[test]
    fn party_of_three_queues_from_the_board() {
        assert!(party_board_ok(PARTY_BOARD_X_MM, PARTY_BOARD_Z_MM));
        assert!(!party_board_ok(0, 0));
        let other = SlotRow::Human {
            you: false,
            ready: true,
        };
        let mut screen = in_lobby(true, &[you(true), other, other, SlotRow::Ai]);
        assert!(screen.party_ready());
        assert_eq!(
            screen.input(LobbyInput::QueueParty),
            Ok(Some(LobbyCommand::QueueParty))
        );
    }

    #[test]
    fn party_queue_refuses_guest_pair_and_open_slot() {
        let other = SlotRow::Human {
            you: false,
            ready: true,
        };
        let mut guest = in_lobby(false, &[you(true), other, other, SlotRow::Ai]);
        assert_eq!(
            guest.input(LobbyInput::QueueParty),
            Err(LobbyUiError::NotHost)
        );
        let mut pair = in_lobby(true, &[you(true), other, SlotRow::Ai]);
        assert!(!pair.party_ready());
        assert_eq!(
            pair.input(LobbyInput::QueueParty),
            Err(LobbyUiError::NotReady)
        );
        let mut open = in_lobby(true, &[you(true), other, other, SlotRow::Open]);
        assert!(!open.party_ready());
        assert_eq!(
            open.input(LobbyInput::QueueParty),
            Err(LobbyUiError::NotReady)
        );
    }

    #[test]
    fn lobby_scale_sixteen_virtualized_with_cost_and_projection() {
        let slots = vec![SlotRow::Ai; usize::from(MAX_PARTICIPANTS)];
        let screen = in_lobby(true, &slots);
        let mut gathered = Vec::new();
        gathered.extend_from_slice(screen.visible_slots(0));
        gathered.extend_from_slice(screen.visible_slots(LOBBY_VIEW_ROWS));
        assert_eq!(gathered, slots);
        assert_eq!(screen.visible_slots(LOBBY_VIEW_ROWS).len(), LOBBY_VIEW_ROWS);
        let one = LobbyScale::for_slots(1);
        assert_eq!(one.units, SLOT_ROW_UNITS);
        assert_eq!(one.bytes, core::mem::size_of::<SlotRow>());
        let sixteen = LobbyScale::for_slots(slots.len());
        assert_eq!(sixteen.units, 48);
        let projected = LobbyScale::for_slots(64);
        assert_eq!(projected.units, 192);
        assert_eq!(projected.bytes, one.bytes.saturating_mul(64));
        assert_eq!(projected.units, sixteen.units.saturating_mul(4));
    }

    #[test]
    fn lobby_scale_window_past_end_draws_nothing() {
        let lobby = in_lobby(true, &[you(true), SlotRow::Ai]);
        assert!(lobby.visible_slots(2).is_empty());
        assert!(lobby.visible_slots(usize::MAX).is_empty());
        assert!(screen().visible_slots(0).is_empty());
    }
}
