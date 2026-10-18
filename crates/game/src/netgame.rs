//! netgame: LAN multiplayer session in the arena app.
//!
//! Consumer: packaging and LAN playtests. Data owner: each peer owns its
//! local [`App`](super::App) view (HUD/audio/progression/presentation) plus
//! its net role — the host owns the authoritative
//! [`Server`](engine_net::session::Server), the guest owns a predicting
//! [`Client`](engine_net::session::Client). Authority stays in `engine-arena`
//! via `engine-net`; this crate only stages inputs, shuttles versioned
//! datagrams over [`std::net`] UDP and reconciles the local view.
//! Units: fixed ticks at the arena rate; the snapshot path quantizes per
//! the net snapshot rules (mm, decidegrees). Determinism scope: same config plus same inputs on
//! both peers gives identical checkpoints; sockets, latency and loss never
//! enter hashed state. Errors: [`GameError`](super::GameError); invalid wire
//! bytes or session codes reject loudly and change nothing, never panic.
//!
//! Roles and slots: 2 human slots max — the host plays slot 0, the guest
//! plays slot 1, AI fills every other slot per the net brain fill. Prediction,
//! rollback/resimulation and remote interpolation are reused from `engine-net`
//! ([`Client`](engine_net::session::Client),
//! [`Server`](engine_net::session::Server),
//! [`Interpolator`](engine_net::session::Interpolator), the versioned wire
//! codec and redundancy windows), never reimplemented here.
//!
//! Transport lives in the headless leg (no feature gate): [`NetGame::host`]
//! binds a loopback-ephemeral socket, [`NetGame::join`] dials the host
//! address carried out-of-band with the invitation code. Tests shuttle
//! through real localhost sockets plus an injected latency/loss matrix; no
//! external processes.
//!
//! Desync diagnosis is checkpoint hashes plus the first divergent tick
//! ([`DesyncReport`], [`first_divergence`]). Versioning: the session code and
//! every datagram carry [`WIRE_VERSION`](engine_net::session::WIRE_VERSION);
//! anything else is refused before anything is applied.
//!
//! Window play (`net_seat`, the `arena --host/--join` path) adds three things on
//! the same wire: a hello that carries the guest's [`MatchSpec`] identity so a
//! host refuses another match loudly ([`GameError::SpecMismatch`]), a lockstep
//! guest ([`NetGame::join_lockstep`]) that sends its inputs ahead and replays
//! only authority (no prediction, so no from-zero rebuild on a rules v2 match
//! that has no snapshot), and a coverage ack ([`KIND_NEED`]) the host answers
//! with re-sent state windows. Datagrams go to the peer directly or through the
//! self-hosted relay (`net_relay`), and an optional seeded [`FaultLink`] drops,
//! delays and reorders what this peer sends.
//!
//! [`MatchSpec`]: engine_arena::MatchSpec

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use engine_anim::net_pose::NetPose;
use engine_arena::Checkpoint;
use engine_core::fixed::Fixed;
use engine_input::{FighterId, FighterInput};
use engine_net::link::{FaultLink, LinkProfile, LinkStats};
use engine_net::session::{
    decode_input, decode_snapshot, decode_state, encode_input, encode_state, Client, FullSnap,
    InterpFrame, InterpResult, NetError, RemoteView, Server, SnapshotDelta, StatePacket,
    KIND_INPUT, KIND_SNAPSHOT, KIND_STATE, MAX_INPUTS_PER_PACKET, REDUNDANCY, WIRE_VERSION,
};

use super::{App, GameConfig, GameError, TickOutcome};

#[path = "netgame_route.rs"]
mod netgame_route;
#[path = "netgame_wire.rs"]
mod netgame_wire;

use self::netgame_route::Route;
pub use self::netgame_wire::{
    bind_for, decode_greeting, decode_session_code, decode_session_mode, encode_duel_session_code,
    encode_greeting, encode_need, encode_session_code, Invite, NetRules, KIND_HELLO, KIND_NEED,
    KIND_PONG, KIND_WELCOME, MAX_CODE_BYTES, SESSION_CODE_DUEL, SESSION_CODE_PREFIX,
    SESSION_CODE_SPEAR,
};
use self::netgame_wire::{
    body_words, header, open_socket, peek_kind, tape_window, word_at, SessionSocket,
};

/// Authoritative inputs repeated in every host state packet (loss cover; the
/// net loopback proof uses the same order of cover).
pub const STATE_COVER: usize = 8;
/// Remote-view interpolation delay in ticks (`≈100 ms` at `60 Hz`, the net
/// default expressed in the tick units the frozen interpolator inside
/// [`RemoteView`] samples are pushed with here).
pub const REMOTE_VIEW_DELAY_TICKS: u64 = 6;
/// Trailing delay of the smooth remote cursor in ticks: one snapshot interval
/// (`SNAPSHOT_EVERY_TICKS`) plus jitter room, inside the sampler's clamp.
pub const REMOTE_CURSOR_DELAY_TICKS: u32 = 8;
/// Inputs staged for ticks past this horizon are refused (flood guard).
pub const MAX_AHEAD_TICKS: u32 = 64;

/// Desync diagnosis: checkpoint hashes plus the first divergent tick. Pure
/// data; produced by [`first_divergence`] and carried by [`NetApplied`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DesyncReport {
    /// First tick where the two histories disagree.
    pub first_tick: u32,
    /// Local checkpoint hash at [`DesyncReport::first_tick`].
    pub local_hash: u64,
    /// Remote checkpoint hash at [`DesyncReport::first_tick`].
    pub remote_hash: u64,
}

/// First index where both histories are present and their hashes disagree, or
/// `None` when the common prefix agrees (a longer history is just ahead, not
/// diverged). Pure.
#[must_use]
pub fn first_divergence(local: &[Checkpoint], remote: &[Checkpoint]) -> Option<DesyncReport> {
    let common = local.len().min(remote.len());
    for index in 0..common {
        if local[index].hash != remote[index].hash {
            return Some(DesyncReport {
                first_tick: local[index].tick,
                local_hash: local[index].hash,
                remote_hash: remote[index].hash,
            });
        }
    }
    None
}

/// Remote-fighter pose checkpoints over the real wire path: each authority
/// pick is encoded to wire bytes (what the LAN carries) and decoded back
/// before checkpointing, so the guest compares exactly what the remote
/// posed after rollback resim. Same authority stream twice gives the same
/// checkpoints. Pure, total: a [`NetPose`] decode failure skips that pose
/// instead of panicking, and lane indices saturate instead of wrapping.
#[must_use]
pub fn remote_pose_checkpoints(authority: &[NetPose], start_tick: u32) -> Vec<(u32, u64)> {
    let mut out = Vec::with_capacity(authority.len());
    for (k, pose) in authority.iter().enumerate() {
        let wire = pose.encode();
        let Ok(back) = NetPose::decode(&wire) else {
            continue;
        };
        let tick = start_tick.wrapping_add(u32::try_from(k).unwrap_or(u32::MAX));
        out.push((tick, back.checkpoint(tick)));
    }
    out
}

/// Lobby slot rows for the windowed screen: one row per fighter in lobby order.
/// Pure; same lobby plus same `me` token gives same rows. No sockets, no
/// addresses; the game layer maps senders to tokens elsewhere.
/// Caller: [`windowed_lobby_entry`] below wires the live screen.
#[must_use]
pub fn lobby_slot_rows(
    lobby: &engine_net::lobby::Lobby,
    me: u64,
) -> Vec<engine_ui::lobby_ui::SlotRow> {
    use engine_net::lobby::Slot as NetSlot;
    use engine_ui::lobby_ui::SlotRow;
    lobby.slots()[..usize::from(lobby.fighters())]
        .iter()
        .map(|slot| match *slot {
            NetSlot::Closed => SlotRow::Closed,
            NetSlot::Open => SlotRow::Open,
            NetSlot::Ai => SlotRow::Ai,
            NetSlot::Human { token, ready } => SlotRow::Human {
                you: token == me,
                ready,
            },
        })
        .collect()
}

/// Windowed loop lobby step: one press plus one command run.
/// Feeds `input` to `screen`, runs `Host`/`FillAi`/`SetReady`/`Start` against `lobby`,
/// refreshes the screen slots via [`lobby_slot_rows`], and returns the command
/// the loop saw. Sockets stay in the caller and no address is typed: hosts are
/// [`engine_ui::lobby_ui::HostId`] positions the caller maps to senders.
/// Pure glue; presentation only, ticks and checkpoints never see it.
/// Errors: [`engine_ui::lobby_ui::LobbyUiError`] when the press or the slot
/// refresh refuses; a refused lobby post leaves the screen as-is.
/// Caller: `windowed::windowed_lobby_frame` drives it from the live redraw.
pub fn windowed_lobby_entry(
    screen: &mut engine_ui::lobby_ui::LobbyScreen,
    lobby: &mut engine_net::lobby::Lobby,
    input: engine_ui::lobby_ui::LobbyInput,
    me: u64,
) -> Result<Option<engine_ui::lobby_ui::LobbyCommand>, engine_ui::lobby_ui::LobbyUiError> {
    use engine_ui::lobby_ui::LobbyCommand;
    let command = screen.input(input)?;
    match command {
        Some(LobbyCommand::Host) => {
            if lobby.join(me, lobby.build()).is_ok() {
                let rows = lobby_slot_rows(lobby, me);
                screen.joined(&rows, true)?;
            }
        }
        Some(LobbyCommand::FillAi) => {
            lobby.ai_fill();
            let rows = lobby_slot_rows(lobby, me);
            screen.set_slots(&rows)?;
        }
        Some(LobbyCommand::SetReady(on)) => {
            if lobby.set_ready(me, on).is_ok() {
                let rows = lobby_slot_rows(lobby, me);
                screen.set_slots(&rows)?;
            }
        }
        Some(LobbyCommand::Start) => {
            if let Ok(params) = lobby.try_start() {
                screen.started(engine_ui::menus::FightRequest {
                    seed: params.seed,
                    fighters: params.fighters,
                })?;
            }
        }
        _ => {}
    }
    Ok(command)
}

/// What [`NetGame::on_net_event`] applied for one datagram.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NetApplied {
    /// Host staged a guest input window: newly staged vs late (already
    /// authored ticks lose per net server authority; duplicates from the
    /// redundancy cover restage silently onto the same entry).
    InputsStaged {
        /// Inputs newly staged for future ticks.
        staged: usize,
        /// Inputs for already-authored ticks, dropped.
        late: usize,
    },
    /// Guest applied a server state: rollback named its mispredicts and the
    /// view was rebuilt from server truth; `desync` is `Some` only when the
    /// rebuilt view still disagrees with the resimulated truth past the
    /// server tick (same-machine runs never hit this; it is the loud
    /// nondeterminism tripwire).
    StateApplied {
        /// Ticks where the prediction diverged from server truth, ascending.
        mispredicts: Vec<u32>,
        /// Persistent divergence after resimulation, if any.
        desync: Option<DesyncReport>,
    },
    /// Guest buffered a resync snapshot into the interpolation buffer.
    SnapshotBuffered {
        /// Snapshot tick now available to [`NetGame::remote_view`].
        tick: u32,
    },
    /// Host read a hello. `accepted` pairs the guest (same match identity);
    /// a refused hello was answered with the host's own welcome and pairs nothing.
    Hello {
        /// The identities matched and the guest is paired.
        accepted: bool,
        /// The guest's match identity.
        guest_identity: u64,
        /// The guest's match code, for the refusal line.
        guest_code: String,
    },
    /// Guest read the host's welcome. `accepted` is false when the host plays
    /// another match; the seat turns that into [`GameError::SpecMismatch`].
    Welcome {
        /// The identities matched.
        accepted: bool,
        /// The host's match identity.
        host_identity: u64,
        /// The host's match code, for the refusal line.
        host_code: String,
    },
    /// Lockstep guest merged one authoritative state window.
    Truth {
        /// The window's checkpoint tick.
        tick: u32,
        /// Every authoritative tick below this one is now known.
        covered: u32,
    },
    /// Host read a lockstep guest's coverage ack (a pong is already queued).
    Need {
        /// The guest holds every authoritative tick below this one.
        need: u32,
    },
    /// Lockstep guest read the echo of a need's stamp.
    Pong {
        /// The stamp the guest sent.
        stamp: u32,
    },
    /// The relay confirmed this peer's registration.
    RelayRegistered,
}

/// The lockstep guest's view of authority: which state windows arrived and
/// which of its own inputs the host has not confirmed yet.
#[derive(Default)]
struct Lockstep {
    /// Every authoritative tick below this is known (inputs merged into `truth`).
    need: u32,
    /// Window ticks received past a hole, waiting for the hole to fill.
    pending: BTreeSet<u32>,
    /// Newest state tick seen.
    latest: Option<u32>,
    /// Next tick this guest sends an input for.
    send_cursor: u32,
    /// Sent inputs at or past `need`, in tick order (re-sent until covered).
    unconfirmed: Vec<FighterInput>,
}

/// Net role: the host authors ticks, the guest predicts them.
#[expect(
    clippy::large_enum_variant,
    reason = "one Role per session, built once and never moved on a hot path"
)]
enum Role {
    /// Authoritative match plus not-yet-consumed guest inputs.
    Host {
        /// Net authority, reused (never reimplemented).
        server: Server,
        /// Guest inputs keyed by `(tick, slot)` for ticks not yet authored.
        pending: HashMap<(u32, u8), FighterInput>,
    },
    /// Predicting match plus server truth learned so far.
    Guest {
        /// Net prediction/rollback, reused (never reimplemented).
        client: Client,
    },
    /// Window guest: sends inputs ahead, replays authority only.
    Lockstep(Lockstep),
}

/// LAN multiplayer session: one [`App`](super::App) view plus one net role.
/// Hosts play slot 0 and author; guests play slot 1 and predict (or, in
/// lockstep, replay authority). AI fills every remaining slot on both peers
/// per the net brain fill.
pub struct NetGame {
    app: App,
    config: GameConfig,
    /// The rules this session plays (the window's duel, with or without the
    /// spear, or the frozen v1 open-sand match).
    rules: NetRules,
    /// [`MatchSpec::identity`](engine_arena::MatchSpec::identity) of the match:
    /// what a hello carries and a welcome must equal.
    identity: u64,
    local_slot: u8,
    remote_slot: u8,
    role: Role,
    sock: SessionSocket,
    /// Where datagrams go: the peer itself, or the relay on a relay route.
    peer: Option<SocketAddr>,
    route: Route,
    /// Seeded loss, delay and reorder on what this peer sends; `None` sends as is.
    fault: Option<FaultLink>,
    /// Datagrams from strangers, or junk that named no paired peer: counted, never applied.
    stray: u64,
    outbound: Vec<Vec<u8>>,
    /// Local inputs by tick (`index == tick`): `Some` for predicted ticks,
    /// `None` where server truth AI-filled this slot after a lost input.
    local_history: Vec<Option<FighterInput>>,
    /// Host: authoritative tape (both human slots) backing state windows.
    tape: Vec<FighterInput>,
    /// Guest: server-truth inputs merged from state windows.
    truth: BTreeMap<(u32, u8), FighterInput>,
    /// Guest: authoritative checkpoints by tick.
    truth_cps: HashMap<u32, Checkpoint>,
    /// Guest remote-presentation view (net type, reused): the frozen tick-clock
    /// interpolator plus the Hermite/quaternion/guard path on one snapshot stream.
    view: RemoteView,
    /// Last state bytes the host emitted (harness final retransmit).
    last_state_bytes: Option<Vec<u8>>,
    /// Mispredict ticks named by the last applied server state.
    last_mispredicts: Vec<u32>,
    /// Persistent divergence found by the last reconciliation, if any.
    last_report: Option<DesyncReport>,
}

impl NetGame {
    /// Host a match: validates the config, opens the authority at slot 0 and
    /// binds a loopback-ephemeral socket. Returns the invitation code (binds
    /// config plus wire version) and the session; the guest address is
    /// learned from its first input packet (or pinned with
    /// [`NetGame::set_peer`]).
    pub fn host(config: GameConfig) -> Result<(String, Self), GameError> {
        Self::host_loopback(config, NetRules::V1)
    }

    /// [`NetGame::host`] for the match the window plays: rules v2 on the kit
    /// walls. The authority server builds from the host view's own spec
    /// ([`App::spec`]) and the invitation code carries the mode, so the guest
    /// predicts the same match.
    pub fn host_duel(config: GameConfig) -> Result<(String, Self), GameError> {
        Self::host_loopback(config, NetRules::Duel)
    }

    /// The test-facing host: a loopback-ephemeral socket, no relay.
    fn host_loopback(config: GameConfig, rules: NetRules) -> Result<(String, Self), GameError> {
        let at = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        Self::host_on(config, rules, at, None).map(|(invite, game)| (invite.code(), game))
    }

    /// Host a match for window play: the authority at slot 0 on a socket bound
    /// at `bind` (`0.0.0.0:<port>` for a LAN, loopback for one PC). With
    /// `relay = Some((relay_addr, my_id))` every datagram goes through the
    /// self-hosted relay under `my_id` and the invitation carries that id, so
    /// the guest reaches the host without a reachable port. Returns the
    /// invitation (the code the guest types) and the session.
    pub fn host_on(
        config: GameConfig,
        rules: NetRules,
        bind: SocketAddr,
        relay: Option<(SocketAddr, u64)>,
    ) -> Result<(Invite, Self), GameError> {
        let app = Self::session_app(config, 0, rules)?;
        let server = Server::from_spec(app.spec()).map_err(GameError::Net)?;
        let (peer, route) = Self::route_for(relay, None)?;
        let invite = Invite {
            config,
            rules,
            relay_host: relay.map(|(_, id)| id),
        };
        let role = Role::Host {
            server,
            pending: HashMap::new(),
        };
        let game = Self::assemble(
            app,
            config,
            rules,
            (0, 1),
            role,
            open_socket(bind)?,
            peer,
            route,
        );
        Ok((invite, game))
    }

    /// Join a match: parses the invitation code (version mismatch refused,
    /// config bound by construction), opens the predictor at slot 1 and dials
    /// `host_addr`.
    pub fn join(host_addr: SocketAddr, code: &str) -> Result<Self, GameError> {
        let invite = Invite::decode(code)?;
        let app = Self::session_app(invite.config, 1, invite.rules)?;
        let local = FighterId::new(1).ok_or(GameError::BadConfig("fighters"))?;
        let client = Client::from_spec(app.spec(), local).map_err(GameError::Net)?;
        let sock = open_socket(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
        let role = Role::Guest { client };
        Ok(Self::assemble(
            app,
            invite.config,
            invite.rules,
            (1, 0),
            role,
            sock,
            Some(host_addr),
            Route::Direct,
        ))
    }

    /// Join a match for window play as the lockstep guest (slot 1): inputs go
    /// ahead to the host ([`NetGame::send_ahead`]) and the view replays only
    /// authority ([`NetGame::replay_next`]). `target` is the host, or the
    /// relay when the code carries a relay id; `my_relay_id` names this peer
    /// there (ignored on a direct route). Queues the hello.
    pub fn join_lockstep(
        target: SocketAddr,
        code: &str,
        my_relay_id: u64,
    ) -> Result<Self, GameError> {
        let invite = Invite::decode(code)?;
        let app = Self::session_app(invite.config, 1, invite.rules)?;
        let relay = invite.relay_host.map(|_| (target, my_relay_id));
        let (_, route) = Self::route_for(relay, invite.relay_host)?;
        let sock = open_socket(bind_for(target, 0))?;
        let role = Role::Lockstep(Lockstep::default());
        let mut game = Self::assemble(
            app,
            invite.config,
            invite.rules,
            (1, 0),
            role,
            sock,
            Some(target),
            route,
        );
        game.queue_hello();
        Ok(game)
    }

    /// Peer address plus route for an optional relay `(addr, my_id)`; a zero
    /// id or a relay id equal to the host's refuses.
    fn route_for(
        relay: Option<(SocketAddr, u64)>,
        peer_id: Option<u64>,
    ) -> Result<(Option<SocketAddr>, Route), GameError> {
        match relay {
            None => Ok((None, Route::Direct)),
            Some((_, 0)) => Err(GameError::BadConfig("relay_id")),
            Some((_, me)) if peer_id == Some(me) => Err(GameError::BadConfig("relay_id")),
            Some((addr, me)) => Ok((
                Some(addr),
                Route::Relay {
                    me,
                    peer_id,
                    registered: false,
                },
            )),
        }
    }

    /// Every constructor's field fill: one view, one role, one socket.
    #[expect(
        clippy::too_many_arguments,
        reason = "one private assembly point for four constructors; each argument is a distinct part"
    )]
    fn assemble(
        app: App,
        config: GameConfig,
        rules: NetRules,
        (local_slot, remote_slot): (u8, u8),
        role: Role,
        sock: SessionSocket,
        peer: Option<SocketAddr>,
        route: Route,
    ) -> Self {
        let identity = app.spec().identity();
        Self {
            app,
            config,
            rules,
            identity,
            local_slot,
            remote_slot,
            role,
            sock,
            peer,
            route,
            fault: None,
            stray: 0,
            outbound: Vec::new(),
            local_history: Vec::new(),
            tape: Vec::new(),
            truth: BTreeMap::new(),
            truth_cps: HashMap::new(),
            view: RemoteView::new(REMOTE_VIEW_DELAY_TICKS, REMOTE_CURSOR_DELAY_TICKS),
            last_state_bytes: None,
            last_mispredicts: Vec::new(),
            last_report: None,
        }
    }

    /// A fresh view for `slot`: the window's duel (rules v2, kit walls, with
    /// or without the spear) or the frozen v1 session.
    fn session_app(config: GameConfig, slot: u8, rules: NetRules) -> Result<App, GameError> {
        let world = crate::arena_kit::kit_match_world_or_empty;
        match rules {
            NetRules::V1 => App::new_for_slot(config, slot),
            NetRules::Duel => App::new_for_slot_world(config, slot, world())?.into_duel(),
            NetRules::DuelSpear => App::new_for_slot_world(config, slot, world())?
                .into_duel()?
                .into_spear(),
        }
    }

    /// True when the session plays the window's duel match.
    #[must_use]
    pub fn is_duel(&self) -> bool {
        self.rules != NetRules::V1
    }

    /// The rules this session plays.
    #[must_use]
    pub fn rules(&self) -> NetRules {
        self.rules
    }

    /// The match identity a hello carries ([`MatchSpec::identity`](engine_arena::MatchSpec::identity)).
    #[must_use]
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// Lend the session view to a window that owns an `App` slot of its own:
    /// the two swap. The window draws `slot` between net calls and swaps back
    /// before the next one, so the session always ticks its own view.
    pub fn swap_view(&mut self, slot: &mut App) {
        std::mem::swap(&mut self.app, slot);
    }

    /// Seed a fault link on everything this peer sends: loss, delay and
    /// reorder per `profile`, in send-call time (one call per frame).
    pub fn set_fault(&mut self, profile: LinkProfile, seed: u64) {
        self.fault = Some(FaultLink::new(profile, seed));
    }

    /// What the fault link measured so far; `None` without one.
    #[must_use]
    pub fn fault_stats(&self) -> Option<LinkStats> {
        self.fault.as_ref().map(|link| *link.stats())
    }

    /// Datagrams from strangers or junk that paired nothing, counted and dropped.
    #[must_use]
    pub fn stray(&self) -> u64 {
        self.stray
    }

    /// Queue the hello (lockstep guest): this match's identity and code.
    pub fn queue_hello(&mut self) {
        let code = self.match_code();
        self.outbound
            .push(encode_greeting(KIND_HELLO, self.identity, &code));
    }

    /// The match part of this session's invitation code.
    #[must_use]
    pub fn match_code(&self) -> String {
        Invite {
            config: self.config,
            rules: self.rules,
            relay_host: None,
        }
        .match_code()
    }

    /// True on the authoring peer.
    #[must_use]
    pub fn is_host(&self) -> bool {
        matches!(self.role, Role::Host { .. })
    }

    /// Local human slot (0 host, 1 guest).
    #[must_use]
    pub fn local_slot(&self) -> u8 {
        self.local_slot
    }

    /// Remote human slot (1 host, 0 guest).
    #[must_use]
    pub fn remote_slot(&self) -> u8 {
        self.remote_slot
    }

    /// Local view: HUD, audio data, hp, progression, presentation.
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }

    /// Host tape for tests: authoritative inputs in `(tick, fighter)` order.
    #[cfg(test)]
    fn tape_for_test(&self) -> &[FighterInput] {
        &self.tape
    }

    /// Session configuration (bound by the invitation code on guests).
    #[must_use]
    pub fn config(&self) -> GameConfig {
        self.config
    }

    /// Fixed-tick cursor of the local view.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.app.cursor()
    }

    /// Authority history: server checkpoints on the host, converged client
    /// checkpoints on the guest.
    #[must_use]
    pub fn authority(&self) -> &[Checkpoint] {
        match &self.role {
            Role::Host { server, .. } => server.checkpoints(),
            Role::Guest { client } => client.checkpoints(),
            Role::Lockstep(_) => self.app.checkpoints(),
        }
    }

    /// Last applied server tick (guest only; the host authors, it never
    /// applies). The lockstep guest reports the newest state tick it saw.
    #[must_use]
    pub fn server_tick(&self) -> Option<u32> {
        match &self.role {
            Role::Host { .. } => None,
            Role::Guest { client } => client.server_tick(),
            Role::Lockstep(lockstep) => lockstep.latest,
        }
    }

    /// Host: the remote human's input for the next tick to author has arrived.
    #[must_use]
    pub fn remote_ready(&self) -> bool {
        match &self.role {
            Role::Host { server, pending } => {
                pending.contains_key(&(server.tick_now(), self.remote_slot))
            }
            _ => false,
        }
    }

    /// Lockstep guest: every authoritative tick below this is known.
    #[must_use]
    pub fn covered(&self) -> u32 {
        match &self.role {
            Role::Lockstep(lockstep) => lockstep.need,
            _ => 0,
        }
    }

    /// Lockstep guest: the next tick [`NetGame::send_ahead`] stamps.
    #[must_use]
    pub fn send_cursor(&self) -> u32 {
        match &self.role {
            Role::Lockstep(lockstep) => lockstep.send_cursor,
            _ => 0,
        }
    }

    /// Lockstep guest: send the local input for the next send tick, never
    /// below the newest state tick (a tick the host already authored is
    /// past). The packet repeats every sent input the host has not confirmed
    /// yet, oldest first, up to the packet bound. Returns the stamped tick;
    /// refuses a foreign slot, a bad input or a finished session, changing nothing.
    pub fn send_ahead(&mut self, input: FighterInput) -> Result<u32, GameError> {
        let ticks = self.config.ticks;
        let slot = self.local_slot;
        let Role::Lockstep(lockstep) = &mut self.role else {
            return Err(GameError::Net(NetError::BadKind(KIND_INPUT)));
        };
        let tick = lockstep
            .send_cursor
            .max(lockstep.latest.map_or(0, |latest| latest + 1));
        let mut stamped = input;
        stamped.tick = tick;
        if stamped.fighter.get() != slot || stamped.validate().is_err() {
            return Err(GameError::BadRemote("slot"));
        }
        if tick >= ticks {
            return Err(GameError::BadRemote("over"));
        }
        lockstep.send_cursor = tick + 1;
        lockstep.unconfirmed.push(stamped);
        let window = &lockstep.unconfirmed[..lockstep.unconfirmed.len().min(MAX_INPUTS_PER_PACKET)];
        let newest = window.last().map_or(tick, |last| last.tick);
        let bytes = encode_input(newest, window).map_err(GameError::Net)?;
        self.outbound.push(bytes);
        Ok(tick)
    }

    /// Lockstep guest: replay the next authoritative tick into the view when
    /// it is covered; `Ok(None)` while the view waits for authority (or the
    /// match ended). The checkpoint is checked against the host's for that
    /// tick when it arrived: a mismatch is kept as [`NetGame::last_report`],
    /// the loud determinism tripwire. Inputs the view no longer needs are
    /// dropped. Authority the view refuses (a remote input it cannot stage)
    /// fails loudly.
    pub fn replay_next(&mut self) -> Result<Option<TickOutcome>, GameError> {
        let tick = self.app.cursor();
        let need = match &self.role {
            Role::Lockstep(lockstep) => lockstep.need,
            _ => return Ok(None),
        };
        if tick >= need || tick >= self.config.ticks {
            return Ok(None);
        }
        let Some((_, outcome)) = self.run_truth(tick) else {
            return Err(GameError::Net(NetError::BadState));
        };
        if let (Some(mine), Some(theirs)) = (outcome.checkpoint, self.truth_cps.get(&tick)) {
            if mine.hash != theirs.hash && self.last_report.is_none() {
                self.last_report = Some(DesyncReport {
                    first_tick: tick,
                    local_hash: mine.hash,
                    remote_hash: theirs.hash,
                });
            }
        }
        self.truth = self.truth.split_off(&(tick + 1, 0));
        self.truth_cps.remove(&tick);
        Ok(Some(outcome))
    }

    /// Host: queue re-sent state windows from the guest's `need` on: one per
    /// [`STATE_COVER`] ticks up to the authored tip (the tip's own window
    /// last), at most `max`. Returns how many were queued; none when the
    /// guest already holds every authored tick.
    pub fn queue_repair(&mut self, need: u32, max: usize) -> usize {
        let Role::Host { server, .. } = &self.role else {
            return 0;
        };
        let Some(last) = server.tick_now().checked_sub(1) else {
            return 0;
        };
        if need > last {
            return 0;
        }
        let cover = u32::try_from(STATE_COVER).unwrap_or(u32::MAX);
        let mut end = need.saturating_add(cover - 1).min(last);
        let mut packets = Vec::new();
        while packets.len() < max {
            let checkpoint = usize::try_from(end)
                .ok()
                .and_then(|at| server.checkpoints().get(at));
            let Some(checkpoint) = checkpoint else {
                break;
            };
            if let Ok(bytes) = encode_state(*checkpoint, tape_window(&self.tape, end, STATE_COVER))
            {
                packets.push(bytes);
            }
            if end >= last {
                break;
            }
            end = end.saturating_add(cover).min(last);
        }
        let queued = packets.len();
        self.outbound.extend(packets);
        queued
    }

    /// Host: queue the last state window again (the match end, where no newer
    /// window will cover a lost one).
    pub fn resend_last_state(&mut self) {
        if let Some(bytes) = self.last_state_bytes.clone() {
            self.outbound.push(bytes);
        }
    }

    /// Lockstep guest: queue a coverage ack with the round-trip `stamp`.
    pub fn queue_need(&mut self, stamp: u32) {
        let need = self.covered();
        self.outbound.push(encode_need(need, stamp));
    }

    /// Mispredict ticks named by the last applied server state (guest only).
    #[must_use]
    pub fn last_mispredicts(&self) -> &[u32] {
        &self.last_mispredicts
    }

    /// Persistent divergence from the last reconciliation, if any.
    #[must_use]
    pub fn last_report(&self) -> Option<DesyncReport> {
        self.last_report
    }

    /// Buffered interpolation samples (guest snapshot path).
    #[must_use]
    pub fn interp_len(&self) -> usize {
        self.view.len()
    }

    /// Interpolated remote view at `at_tick` (tick units, the delay is
    /// [`REMOTE_VIEW_DELAY_TICKS`]); `None` until snapshots arrive.
    #[must_use]
    pub fn remote_view(&self, at_tick: u64) -> Option<InterpResult> {
        self.view.legacy_at(at_tick)
    }

    /// Smooth remote frame for the render clock: the cursor trails the newest
    /// snapshot by [`REMOTE_CURSOR_DELAY_TICKS`] and `alpha` (`0..=1`, integer
    /// math from the render clock) places the frame inside the tick. Hermite
    /// positions, quaternion yaw, the guard's interpolate/extrapolate class;
    /// `None` until snapshots arrive or past the guard's horizon. Presentation
    /// only: never hashed.
    pub fn remote_frame(&mut self, alpha: Fixed) -> Option<InterpFrame> {
        self.view.present(alpha)
    }

    /// Last state bytes the host emitted (harness final retransmit).
    #[must_use]
    pub fn last_state_bytes(&self) -> Option<&[u8]> {
        self.last_state_bytes.as_deref()
    }

    /// Socket address of this peer.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    /// Remote peer address, if known.
    #[must_use]
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// Pin the remote peer address (the host learns it from the first input
    /// packet; tests pin it for the direct socket path).
    pub fn set_peer(&mut self, peer: SocketAddr) {
        self.peer = Some(peer);
    }

    /// Encoded datagrams awaiting send.
    #[must_use]
    pub fn outbound(&self) -> &[Vec<u8>] {
        &self.outbound
    }

    /// Take every queued datagram (the harness drains these through its
    /// latency/loss matrix; [`NetGame::flush`] sends them direct).
    pub fn drain_outbound(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outbound)
    }

    /// Advance one fixed tick on a local input and queue the outbound
    /// datagram (input window on guests, state window on hosts). The input
    /// must name the local slot and validate, and the session must have ticks
    /// left, or a rejected outcome returns and nothing advances — authority
    /// and view stay aligned because neither moves. Callers tick in order;
    /// missing remote inputs are AI-filled per the net rules, late ones lose.
    pub fn tick_local(&mut self, input: FighterInput) -> TickOutcome {
        if self.is_host() {
            self.host_tick(input)
        } else {
            self.guest_tick(input)
        }
    }

    /// Host tick: author with the staged remote input when it arrived (AI
    /// fill otherwise), mirror into the view, broadcast the state window.
    fn host_tick(&mut self, input: FighterInput) -> TickOutcome {
        let cursor = self.app.cursor();
        let mut stamped = input;
        stamped.tick = cursor;
        if stamped.fighter.get() != self.local_slot
            || stamped.validate().is_err()
            || cursor >= self.config.ticks
        {
            return self.app.tick(input);
        }
        let Role::Host { server, pending } = &mut self.role else {
            return self.app.tick(spoof_invalid(stamped, self.remote_slot));
        };
        if server.tick_now() != cursor {
            return self.app.tick(spoof_invalid(stamped, self.remote_slot));
        }
        let remote = pending.remove(&(cursor, self.remote_slot));
        let mut inputs = vec![stamped];
        if let Some(staged) = remote {
            inputs.push(staged);
        }
        let checkpoint = match server.tick(&inputs) {
            Ok((checkpoint, _)) => checkpoint,
            Err(_) => return self.app.tick(spoof_invalid(stamped, self.remote_slot)),
        };
        // Mirror into the view over the same tape: offer the remote input
        // first so `(tick, fighter)` order holds, then tick the local input.
        // Both were validated (remote at stage time, local above), so these
        // cannot fail without diverging authority from view.
        if let Some(staged) = remote {
            if self.app.offer_remote(staged).is_err() {
                return self.app.tick(spoof_invalid(stamped, self.remote_slot));
            }
        }
        // Keep `(tick, fighter)` order without re-sorting the ever-growing
        // tape: each staged input lands at its `partition_point`, the same
        // pattern as `App::stage_input` in `lib.rs`. Order holds for the
        // two-inputs-same-tick LAN case either way inputs arrive.
        for staged in &inputs {
            let key = (staged.tick, staged.fighter.0);
            let index = self
                .tape
                .partition_point(|known| (known.tick, known.fighter.0) <= key);
            self.tape.insert(index, *staged);
        }
        pending.retain(|(tick, _), _| *tick > cursor);
        let outcome = self.app.tick(stamped);
        debug_assert!(outcome.input_accepted);
        debug_assert_eq!(outcome.checkpoint, Some(checkpoint));
        match encode_state(
            checkpoint,
            tape_window(&self.tape, checkpoint.tick, STATE_COVER),
        ) {
            Ok(bytes) => {
                self.last_state_bytes = Some(bytes.clone());
                self.outbound.push(bytes);
            }
            Err(_) => {
                debug_assert!(false, "host state window is built valid");
            }
        }
        outcome
    }

    /// Guest tick: predict the local input (AI fills the unknown remote),
    /// mirror into the view, queue the input window. Ticks at or below the
    /// last server tick replay server truth instead of predicting.
    fn guest_tick(&mut self, input: FighterInput) -> TickOutcome {
        let cursor = self.app.cursor();
        let mut stamped = input;
        stamped.tick = cursor;
        if stamped.fighter.get() != self.local_slot
            || stamped.validate().is_err()
            || cursor >= self.config.ticks
        {
            return self.app.tick(input);
        }
        // Server-ahead catch-up first: replay authority instead of predicting
        // (needs no predictor alignment, only truth).
        if self.server_tick().is_some_and(|done| cursor <= done) {
            return self.replay_one(cursor);
        }
        let tick_now = match &self.role {
            Role::Guest { client } => client.tick_now(),
            Role::Host { .. } | Role::Lockstep(_) => {
                return self.app.tick(spoof_invalid(stamped, self.remote_slot));
            }
        };
        if tick_now != cursor {
            return self.app.tick(spoof_invalid(stamped, self.remote_slot));
        }
        {
            let Role::Guest { client } = &mut self.role else {
                return self.app.tick(spoof_invalid(stamped, self.remote_slot));
            };
            if client.predict(stamped).is_err() {
                return self.app.tick(spoof_invalid(stamped, self.remote_slot));
            }
        }
        // Stage whatever remote truth names for a fresh tick (empty in
        // lockstep; present only when the server runs ahead).
        if let Some(known) = self.truth.get(&(cursor, self.remote_slot)).copied() {
            if self.app.offer_remote(known).is_err() {
                return self.app.tick(spoof_invalid(stamped, self.remote_slot));
            }
        }
        self.local_history.push(Some(stamped));
        let outcome = self.app.tick(stamped);
        debug_assert!(outcome.input_accepted);
        let from = self.local_history.len().saturating_sub(REDUNDANCY);
        let window: Vec<FighterInput> = self.local_history[from..]
            .iter()
            .filter_map(|slot| *slot)
            .collect();
        match encode_input(cursor, &window) {
            Ok(bytes) => self.outbound.push(bytes),
            Err(_) => {
                debug_assert!(false, "guest input window is built valid");
            }
        }
        outcome
    }

    /// Replay one authoritative tick into the view (server-ahead catch-up):
    /// stage the known remote input, then tick the known local input or — when
    /// the server AI-filled this slot after a lost input — advance with every
    /// slot AI-filled, exactly like the server did. The effective input (or
    /// its absence) is recorded in the local history so later rebuilds replay
    /// the same tape the predictor would have.
    fn replay_one(&mut self, tick: u32) -> TickOutcome {
        match self.run_truth(tick) {
            Some((stored, outcome)) => {
                self.local_history.push(stored);
                outcome
            }
            None => self.app.tick(spoof_invalid_for(self.remote_slot, tick)),
        }
    }

    /// Run authoritative `tick` in the view: stage the known remote input,
    /// then tick the known local input, or advance with every slot AI-filled
    /// where the server AI-filled this one. Returns the local input it used
    /// (or its absence) with the outcome; `None` when the tick is past the
    /// session or the remote input was refused, with nothing advanced.
    fn run_truth(&mut self, tick: u32) -> Option<(Option<FighterInput>, TickOutcome)> {
        if tick >= self.config.ticks {
            return None;
        }
        if let Some(known) = self.truth.get(&(tick, self.remote_slot)).copied() {
            self.app.offer_remote(known).ok()?;
        }
        let stored = self.truth.get(&(tick, self.local_slot)).copied();
        let outcome = match stored {
            Some(known) => self.app.tick(known),
            None => self.app.tick_empty(),
        };
        Some((stored, outcome))
    }

    /// Apply one datagram by kind and role. Anything unexpected for the role
    /// (states at the host, inputs at the guest, snapshots at the host,
    /// fighter-state deltas anywhere in this session) fails loudly and
    /// changes nothing. Hellos pair through [`NetGame::poll`], which knows
    /// the sender; here they refuse like any other wrong kind.
    pub fn on_net_event(&mut self, bytes: &[u8]) -> Result<NetApplied, GameError> {
        let kind = peek_kind(bytes)?;
        let lockstep = matches!(self.role, Role::Lockstep(_));
        match kind {
            KIND_INPUT if self.is_host() => {
                let packet = decode_input(bytes).map_err(GameError::Net)?;
                self.stage_inputs(&packet.inputs)
            }
            KIND_STATE if lockstep => {
                let packet = decode_state(bytes).map_err(GameError::Net)?;
                self.apply_window(&packet)
            }
            KIND_STATE if !self.is_host() => {
                let packet = decode_state(bytes).map_err(GameError::Net)?;
                self.apply_state(&packet)
            }
            KIND_SNAPSHOT if !self.is_host() => {
                let snapshot = decode_snapshot(bytes).map_err(GameError::Net)?;
                self.apply_snapshot(&snapshot)
            }
            KIND_WELCOME if !self.is_host() => {
                let (host_identity, host_code) = decode_greeting(bytes)?;
                Ok(NetApplied::Welcome {
                    accepted: host_identity == self.identity,
                    host_identity,
                    host_code,
                })
            }
            KIND_NEED if self.is_host() => {
                let body = body_words::<8>(bytes)?;
                let (need, stamp) = (word_at(&body, 0), word_at(&body, 4));
                let mut pong = header(KIND_PONG, 4);
                pong.extend_from_slice(&stamp.to_le_bytes());
                self.outbound.push(pong);
                Ok(NetApplied::Need { need })
            }
            KIND_PONG if lockstep => {
                let body = body_words::<4>(bytes)?;
                Ok(NetApplied::Pong {
                    stamp: word_at(&body, 0),
                })
            }
            _ => Err(GameError::Net(NetError::BadKind(kind))),
        }
    }

    /// Merge one authoritative state window (lockstep guest): the window must
    /// hold only ticks of `tick - (STATE_COVER - 1) ..= tick`, so absent
    /// entries in that span are AI-filled ticks, and coverage advances past
    /// every window that now touches the known prefix. Ticks the view already
    /// ran are not kept. A window past the session or of the wrong shape
    /// fails loudly and changes nothing.
    fn apply_window(&mut self, packet: &StatePacket) -> Result<NetApplied, GameError> {
        let checkpoint = packet.checkpoint;
        let back = u32::try_from(STATE_COVER - 1).unwrap_or(u32::MAX);
        let floor = checkpoint.tick.saturating_sub(back);
        let shaped = packet
            .inputs
            .iter()
            .all(|input| (floor..=checkpoint.tick).contains(&input.tick));
        if packet.delta.is_some() || checkpoint.tick >= self.config.ticks || !shaped {
            return Err(GameError::Net(NetError::BadState));
        }
        let ran = self.app.cursor();
        for input in packet.inputs.iter().filter(|input| input.tick >= ran) {
            self.truth.insert((input.tick, input.fighter.get()), *input);
        }
        if checkpoint.tick >= ran {
            self.truth_cps.insert(checkpoint.tick, checkpoint);
        }
        let Role::Lockstep(lockstep) = &mut self.role else {
            return Err(GameError::Net(NetError::BadKind(KIND_STATE)));
        };
        lockstep.latest = lockstep.latest.max(Some(checkpoint.tick));
        lockstep.pending.insert(checkpoint.tick);
        while let Some(first) = lockstep.pending.first().copied() {
            if first.saturating_sub(back) > lockstep.need {
                break;
            }
            lockstep.need = lockstep.need.max(first + 1);
            lockstep.pending.remove(&first);
        }
        let need = lockstep.need;
        lockstep.unconfirmed.retain(|input| input.tick >= need);
        Ok(NetApplied::Truth {
            tick: checkpoint.tick,
            covered: need,
        })
    }

    /// Stage one validated guest input window (host only). Every entry is
    /// validated before anything is staged: a foreign slot, an out-of-range
    /// field or a far-future tick fails loudly and stages nothing.
    fn stage_inputs(&mut self, inputs: &[FighterInput]) -> Result<NetApplied, GameError> {
        let now = match &self.role {
            Role::Host { server, .. } => server.tick_now(),
            Role::Guest { .. } | Role::Lockstep(_) => {
                return Err(GameError::Net(NetError::BadKind(KIND_INPUT)));
            }
        };
        let mut fresh: Vec<FighterInput> = Vec::with_capacity(inputs.len());
        let mut late = 0usize;
        for input in inputs {
            if input.fighter.get() != self.remote_slot || input.validate().is_err() {
                return Err(GameError::Net(NetError::BadInput));
            }
            if input.tick < now {
                late += 1;
                continue;
            }
            if input.tick > now + MAX_AHEAD_TICKS {
                return Err(GameError::Net(NetError::BadInput));
            }
            fresh.push(*input);
        }
        let mut staged = 0usize;
        let Role::Host { pending, .. } = &mut self.role else {
            return Err(GameError::Net(NetError::BadKind(KIND_INPUT)));
        };
        for input in fresh {
            // Repeats from the redundancy cover restage silently onto the same
            // entry; only newly learned ticks count as staged.
            if pending
                .insert((input.tick, input.fighter.get()), input)
                .is_none()
            {
                staged += 1;
            }
        }
        Ok(NetApplied::InputsStaged { staged, late })
    }

    /// Apply one authoritative server state (guest only): merge the window
    /// into server truth, name mispredicts through the net client,
    /// resimulate to convergence and rebuild the view from truth.
    fn apply_state(&mut self, packet: &StatePacket) -> Result<NetApplied, GameError> {
        if packet.delta.is_some() {
            return Err(GameError::Net(NetError::BadState));
        }
        let checkpoint = packet.checkpoint;
        for input in &packet.inputs {
            self.truth.insert((input.tick, input.fighter.get()), *input);
        }
        self.truth_cps.insert(checkpoint.tick, checkpoint);
        // `truth` is a `BTreeMap`, so the range walk is already
        // `(tick, fighter)`-ordered: no filter-plus-sort plus dead dedup
        // (keys are unique, the `(tick, fighter)` dedup could never fire).
        let tape: Vec<FighterInput> = self
            .truth
            .range(..=(checkpoint.tick, u8::MAX))
            .map(|(_, input)| *input)
            .collect();
        let delta = SnapshotDelta {
            version: WIRE_VERSION,
            tick: checkpoint.tick,
            hash: checkpoint.hash,
            tape,
        };
        let Role::Guest { client } = &mut self.role else {
            return Err(GameError::Net(NetError::BadKind(KIND_STATE)));
        };
        let mispredicts = client
            .on_server_state(checkpoint, &delta)
            .map_err(GameError::Net)?;
        let converged = client.resimulate();
        // Rebuild the view only when it fell behind the converged history:
        // the lockstep path leaves it already equal, so no fresh `App` and no
        // from-zero view replay runs per state. Byte-true: the old rebuild
        // produced exactly this history, so the desync answer is the same
        // either way, and the tripwire still runs whenever a rebuild is owed.
        let desync = if self.app.checkpoints() == converged.as_slice() {
            None
        } else {
            self.rebuild_view()?;
            first_divergence(self.app.checkpoints(), &converged)
        };
        self.last_mispredicts.clone_from(&mispredicts);
        self.last_report = desync;
        Ok(NetApplied::StateApplied {
            mispredicts,
            desync,
        })
    }

    /// Rebuild the view from server truth plus still-unacked local
    /// predictions: ticks at or below the server tick replay authority (AI
    /// fill where the server AI-filled after a lost input), later ticks
    /// replay local predictions. Deterministic: the same truth plus the same
    /// predictions rebuild the same view twice.
    fn rebuild_view(&mut self) -> Result<(), GameError> {
        let done = self.server_tick().map_or(0, |tick| tick + 1);
        let predicted = self.local_history.len() as u32;
        let mut fresh = Self::session_app(self.config, self.local_slot, self.rules)?;
        for tick in 0..predicted {
            if let Some(known) = self.truth.get(&(tick, self.remote_slot)).copied() {
                fresh
                    .offer_remote(known)
                    .map_err(|_| GameError::Net(NetError::BadState))?;
            }
            let outcome = if tick < done {
                match self.truth.get(&(tick, self.local_slot)).copied() {
                    Some(known) => fresh.tick(known),
                    None => fresh.tick_empty(),
                }
            } else {
                match self.local_history[tick as usize] {
                    Some(predicted) => fresh.tick(predicted),
                    None => return Err(GameError::Net(NetError::SimRejected)),
                }
            };
            if !outcome.input_accepted {
                return Err(GameError::Net(NetError::SimRejected));
            }
        }
        self.app = fresh;
        Ok(())
    }

    /// Buffer one resync snapshot into the interpolation buffer (guest only).
    fn apply_snapshot(&mut self, snapshot: &FullSnap) -> Result<NetApplied, GameError> {
        if self.is_host() {
            return Err(GameError::Net(NetError::BadKind(KIND_SNAPSHOT)));
        }
        // Sample times are tick units with the delay in [`REMOTE_VIEW_DELAY_TICKS`].
        if !self.view.push(snapshot) {
            return Err(GameError::Net(NetError::BadState));
        }
        Ok(NetApplied::SnapshotBuffered {
            tick: snapshot.tick,
        })
    }
}

/// Build an input the view is guaranteed to reject (wrong slot): returns a
/// rejected outcome without advancing anything. `stamped` carries the cursor
/// tick already.
fn spoof_invalid(stamped: FighterInput, wrong_slot: u8) -> FighterInput {
    spoof_invalid_for(wrong_slot, stamped.tick)
}

/// Wrong-slot input for `tick`: [`App::tick`] rejects it without advancing.
fn spoof_invalid_for(wrong_slot: u8, tick: u32) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId(wrong_slot),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: engine_input::fighter::Buttons::NONE,
    }
}

#[cfg(test)]
mod tests {
    use super::netgame_wire::HEADER_BYTES;
    use super::*;
    use engine_input::fighter::{Buttons, ATTACK};
    use engine_net::session::{encode_snapshot, FighterSnap, MAX_PACKET_BYTES};
    use std::collections::VecDeque;
    use std::net::UdpSocket;

    const TICKS: u32 = 90;
    const LATENCY: u32 = 3;
    const FIGHTERS: u8 = 3;

    fn config() -> GameConfig {
        GameConfig {
            seed: 7,
            ticks: TICKS,
            fighters: FIGHTERS,
            half_extent_m: 20.0,
        }
    }

    fn small_config() -> GameConfig {
        GameConfig {
            seed: 7,
            ticks: 16,
            fighters: 2,
            half_extent_m: 12.0,
        }
    }

    fn host_input(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: if tick % 20 < 10 { 100 } else { -100 },
            mz: 0,
            facing: 0,
            buttons: if tick.is_multiple_of(30) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        }
    }

    fn guest_input(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(1).expect("slot 1"),
            mx: if tick % 16 < 8 { -100 } else { 100 },
            mz: if tick % 8 < 4 { 100 } else { 0 },
            facing: 0,
            buttons: if tick.is_multiple_of(20) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        }
    }

    /// Deterministic xorshift64 for seeded loss injection (test-only).
    fn xorshift(mut x: u64) -> u64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }

    /// Loopback fault link between harness sockets: seeded 10% loss applies
    /// before transmit over real localhost UDP, arrivals wait `latency` ticks
    /// before delivery. Single-threaded, deterministic, no external processes.
    struct FaultLink {
        send_sock: UdpSocket,
        recv_sock: UdpSocket,
        latency: u32,
        now: u32,
        rng: u64,
        queue: VecDeque<(u32, Vec<u8>)>,
        sent: u64,
        dropped: u64,
    }

    impl FaultLink {
        fn new(latency: u32, seed: u64) -> Self {
            let send_sock = UdpSocket::bind("127.0.0.1:0").expect("harness socket");
            let recv_sock = UdpSocket::bind("127.0.0.1:0").expect("harness socket");
            recv_sock.set_nonblocking(true).expect("nonblocking");
            Self {
                send_sock,
                recv_sock,
                latency,
                now: 0,
                rng: seed | 1,
                queue: VecDeque::new(),
                sent: 0,
                dropped: 0,
            }
        }

        fn addr(&self) -> SocketAddr {
            self.recv_sock.local_addr().expect("harness addr")
        }

        /// Maybe transit one datagram over real localhost UDP (loss first).
        fn send(&mut self, bytes: &[u8]) {
            self.sent += 1;
            self.rng = xorshift(self.rng);
            if self.rng.is_multiple_of(10) {
                self.dropped += 1;
                return;
            }
            self.send_sock
                .send_to(bytes, self.addr())
                .expect("loopback send");
        }

        /// Drain arrived datagrams into the delay queue.
        fn pump(&mut self) {
            let mut buf = [0u8; MAX_PACKET_BYTES];
            while let Ok((n, _)) = self.recv_sock.recv_from(&mut buf) {
                self.queue
                    .push_back((self.now + self.latency, buf[..n].to_vec()));
            }
        }

        /// Pop every datagram whose delivery tick arrived, in send order.
        fn due(&mut self) -> Vec<Vec<u8>> {
            let mut out = Vec::new();
            while self.queue.front().is_some_and(|(at, _)| *at <= self.now) {
                if let Some((_, bytes)) = self.queue.pop_front() {
                    out.push(bytes);
                }
            }
            out
        }

        fn tick(&mut self) {
            self.now += 1;
        }
    }

    #[test]
    fn remote_pose_checkpoints_follow_the_wire() {
        use engine_anim::net_pose::{checkpoints_for_stream, resimulate};
        let auth: Vec<NetPose> = (0..24u16)
            .map(|k| {
                NetPose::from_pick(k % 3, k, f32::from(k % 8) / 8.0).expect("fixture phase finite")
            })
            .collect();
        // Clean wire matches the direct checkpoints, twice identically.
        assert_eq!(
            remote_pose_checkpoints(&auth, 100),
            checkpoints_for_stream(&auth, 100)
        );
        // A delayed remote diverges pre-resim, then resim repairs it.
        let mut late = vec![auth[0]; 9];
        late.extend_from_slice(&auth[..auth.len() - 9]);
        assert_ne!(
            remote_pose_checkpoints(&late, 100),
            remote_pose_checkpoints(&auth, 100),
            "delay must move remote checkpoints"
        );
        let split = late
            .iter()
            .zip(auth.iter())
            .position(|(a, b)| a != b)
            .expect("delay diverges");
        assert_eq!(
            remote_pose_checkpoints(&resimulate(&late, &auth, split), 100),
            remote_pose_checkpoints(&auth, 100)
        );
        // One flipped pick moves the checkpoints, never silently.
        let mut flipped = auth.clone();
        flipped[5] = NetPose::from_pick(9, 9, 0.5).expect("fixture phase finite");
        assert_ne!(
            remote_pose_checkpoints(&flipped, 100),
            remote_pose_checkpoints(&auth, 100)
        );
    }

    #[test]
    fn session_code_roundtrip_and_refusal() {
        let code = encode_session_code(&config());
        let decoded = decode_session_code(&code).unwrap();
        assert_eq!(decoded, config());
        // Corrupt codes fail loudly.
        for bad in [
            "junk",
            "E2040NET-v1-zz",
            "E2040NET-v1-0000000000000007-90-3-xxxxxxxx",
            "E2040NET-v1-0000000000000007-0-3-41800000",
            "E2040NET-v1-0000000000000007-90-1-41800000",
        ] {
            assert!(decode_session_code(bad).is_err(), "code accepted: {bad}");
        }
        // Future wire version refused with the culprit carried.
        let future = code.replacen(&format!("v{WIRE_VERSION}"), "v99", 1);
        assert_eq!(
            decode_session_code(&future),
            Err(GameError::Net(NetError::VersionMismatch { got: 99 }))
        );
        // Older version refused the same way.
        let older = code.replacen(&format!("v{WIRE_VERSION}"), "v0", 1);
        assert_eq!(
            decode_session_code(&older),
            Err(GameError::Net(NetError::VersionMismatch { got: 0 }))
        );
    }

    #[test]
    fn spec_duel_code_carries_the_mode_and_plain_codes_stay_v1() {
        let plain = encode_session_code(&config());
        let duel = encode_duel_session_code(&config());
        assert_eq!(duel, format!("{plain}-duel"));
        assert_eq!(decode_session_mode(&plain), Ok((config(), false)));
        assert_eq!(decode_session_mode(&duel), Ok((config(), true)));
        assert_eq!(decode_session_code(&duel), Ok(config()));
        for bad in [format!("{duel}-duel"), format!("{plain}-solo")] {
            assert!(decode_session_mode(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn invite_codes_carry_the_spear_and_the_relay_id_and_refuse_the_rest() {
        let spear = Invite {
            config: config(),
            rules: NetRules::DuelSpear,
            relay_host: Some(0x00ab_cdef_0123_4567),
        };
        let code = spear.code();
        assert!(code.ends_with("-duel-spear-r00abcdef01234567"), "{code}");
        assert_eq!(Invite::decode(&code), Ok(spear));
        assert_eq!(
            spear.match_code(),
            format!("{}-spear", encode_duel_session_code(&config()))
        );
        // The spear rides only on the duel, the relay id is 16 hex digits and nonzero.
        let plain = encode_session_code(&config());
        for bad in [
            format!("{plain}-spear"),
            format!("{plain}-duel-spear-spear"),
            format!("{plain}-duel-r123"),
            format!("{plain}-duel-r0000000000000000"),
            format!("{plain}-duel-r00abcdef01234567-x"),
        ] {
            assert!(Invite::decode(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn greetings_and_needs_round_trip_and_refuse_junk() {
        let hello = encode_greeting(KIND_HELLO, 0x1234, "E2040NET-v1-x");
        assert_eq!(peek_kind(&hello), Ok(KIND_HELLO));
        assert_eq!(
            decode_greeting(&hello),
            Ok((0x1234, "E2040NET-v1-x".to_string()))
        );
        assert!(decode_greeting(&hello[..10]).is_err(), "short body");
        let mut spaced = hello.clone();
        spaced.push(b' ');
        assert!(
            decode_greeting(&spaced).is_err(),
            "a space is not a code byte"
        );
        let long = encode_greeting(KIND_HELLO, 1, &"E".repeat(MAX_CODE_BYTES + 9));
        assert_eq!(
            long.len(),
            HEADER_BYTES + 8 + MAX_CODE_BYTES,
            "codes are cut"
        );
        let need = encode_need(77, 9);
        let body = body_words::<8>(&need).expect("need body");
        assert_eq!((word_at(&body, 0), word_at(&body, 4)), (77, 9));
        assert!(body_words::<8>(&need[..11]).is_err());
    }

    #[test]
    fn tape_window_slices_the_last_ticks_by_binary_search() {
        let input = |tick: u32, slot: u8| FighterInput {
            tick,
            fighter: FighterId(slot),
            ..host_input(tick)
        };
        let tape: Vec<FighterInput> = (0..40u32)
            .flat_map(|tick| [input(tick, 0), input(tick, 1)])
            .filter(|known| known.tick != 35 || known.fighter.0 == 0)
            .collect();
        let window = tape_window(&tape, 37, STATE_COVER);
        assert_eq!(window.first().map(|known| known.tick), Some(30));
        assert_eq!(window.last().map(|known| known.tick), Some(37));
        assert_eq!(window.len(), 15, "8 ticks of two slots, one AI-filled");
        assert_eq!(
            tape_window(&tape, 3, STATE_COVER).len(),
            8,
            "the start floors at 0"
        );
    }

    /// Host and lockstep guest over real loopback sockets, the guest's inputs
    /// sent `lead` ticks ahead, the host authoring only once each is in.
    fn lockstep_pair(rules: NetRules) -> (NetGame, NetGame) {
        let host_bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let (invite, host) = NetGame::host_on(config(), rules, host_bind, None).expect("host");
        let target = host.local_addr().expect("host addr");
        let guest = NetGame::join_lockstep(target, &invite.code(), 7).expect("guest");
        (host, guest)
    }

    /// Pump both until `done`, bounded; returns the frames it took.
    fn pump_until(
        host: &mut NetGame,
        guest: &mut NetGame,
        mut done: impl FnMut(&mut NetGame, &mut NetGame) -> bool,
    ) -> u32 {
        for frame in 0..5_000 {
            if done(host, guest) {
                return frame;
            }
            guest.flush().expect("guest flush");
            for _ in 0..3 {
                host.poll().expect("host poll");
            }
            host.flush().expect("host flush");
            for _ in 0..3 {
                guest.poll().expect("guest poll");
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        panic!("the pair never got there");
    }

    #[test]
    fn a_lockstep_guest_replays_the_hosts_exact_checkpoints() {
        let (mut host, mut guest) = lockstep_pair(NetRules::V1);
        pump_until(&mut host, &mut guest, |host, _| host.paired());
        assert_eq!(host.peer(), Some(guest.local_addr().expect("guest addr")));
        for _ in 0..200_000 {
            if guest.cursor() >= TICKS {
                break;
            }
            if guest.send_cursor() < TICKS && guest.send_cursor() < host.cursor() + 4 {
                let tick = guest.send_cursor();
                assert_eq!(guest.send_ahead(guest_input(tick)), Ok(tick));
            }
            guest.flush().expect("guest flush");
            host.poll().expect("host poll");
            if host.remote_ready() {
                let tick = host.cursor();
                assert!(host.tick_local(host_input(tick)).input_accepted);
            }
            host.flush().expect("host flush");
            guest.poll().expect("guest poll");
            while let Some(outcome) = guest.replay_next().expect("authority replays") {
                assert!(outcome.input_accepted);
            }
        }
        assert_eq!(guest.cursor(), TICKS, "the guest ran every tick");
        assert_eq!(guest.app().checkpoints(), host.authority());
        assert_eq!(guest.authority(), host.authority());
        assert_eq!(guest.last_report(), None);
        // The guest's own inputs are what the host authored for slot 1.
        let ones = host
            .tape_for_test()
            .iter()
            .filter(|known| known.fighter.0 == 1)
            .count();
        assert_eq!(ones, TICKS as usize, "no tick of the guest was AI-filled");
    }

    #[test]
    fn a_hole_in_the_windows_waits_for_a_repair_and_the_view_never_guesses() {
        let (mut host, mut guest) = lockstep_pair(NetRules::V1);
        pump_until(&mut host, &mut guest, |host, _| host.paired());
        host.drain_outbound();
        // The host authors 30 ticks with the guest AI-filled, and every window is lost.
        for tick in 0..30 {
            assert!(host.tick_local(host_input(tick)).input_accepted);
        }
        host.drain_outbound();
        assert_eq!(
            guest.replay_next(),
            Ok(None),
            "nothing known, nothing replayed"
        );
        // Only the newest window arrives: a hole before it.
        host.resend_last_state();
        host.flush().expect("flush");
        pump_until(&mut host, &mut guest, |_, guest| {
            guest.server_tick() == Some(29)
        });
        assert_eq!(guest.covered(), 0, "ticks 0..22 are unknown");
        assert_eq!(guest.replay_next(), Ok(None));
        // The coverage ack names the hole; the repair fills it.
        guest.queue_need(5);
        let mut repaired = 0;
        pump_until(&mut host, &mut guest, |host, guest| {
            if repaired == 0 {
                repaired = host.queue_repair(guest.covered(), 8);
            }
            guest.covered() == 30
        });
        assert_eq!(repaired, 4, "windows ending at 7, 15, 23 and the tip 29");
        while guest.replay_next().expect("replays").is_some() {}
        assert_eq!(guest.app().checkpoints(), host.authority());
        assert_eq!(guest.last_report(), None);
    }

    #[test]
    fn a_hello_for_another_match_is_refused_on_both_sides() {
        // The host plays the duel (rules v2); one guest's code asks for the
        // duel with the spear, another for the frozen v1 match.
        for rules in [NetRules::DuelSpear, NetRules::V1] {
            refused_both_sides(rules);
        }
    }

    fn refused_both_sides(rules: NetRules) {
        let host_bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let (invite, mut host) =
            NetGame::host_on(config(), NetRules::Duel, host_bind, None).expect("host");
        let wrong = Invite { rules, ..invite };
        let target = host.local_addr().expect("host addr");
        let mut guest = NetGame::join_lockstep(target, &wrong.code(), 7).expect("guest");
        assert_ne!(guest.identity(), host.identity());
        guest.flush().expect("hello out");
        let mut heard = Vec::new();
        for _ in 0..2_000 {
            heard.extend(host.poll().expect("host polls"));
            if !heard.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        assert!(
            matches!(heard.as_slice(), [NetApplied::Hello { accepted: false, guest_identity, .. }] if *guest_identity == guest.identity()),
            "{heard:?}"
        );
        assert!(!host.paired(), "a refused guest pairs nothing");
        let mut answer = Vec::new();
        for _ in 0..2_000 {
            answer.extend(guest.poll().expect("guest polls"));
            if !answer.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        assert!(
            matches!(answer.as_slice(), [NetApplied::Welcome { accepted: false, host_identity, .. }] if *host_identity == host.identity()),
            "{answer:?}"
        );
    }

    #[test]
    fn host_tape_keeps_tick_fighter_order_for_same_tick_inputs() {
        // The LAN tick stages two inputs for the same tick (local slot 0
        // plus staged remote slot 1): the tape must stay
        // `(tick, fighter)`-ordered so state windows encode identically.
        let (_code, mut host) = NetGame::host(small_config()).unwrap();
        for tick in 0..4 {
            host.stage_inputs(&[guest_input(tick)]).unwrap();
            assert!(host.tick_local(host_input(tick)).input_accepted);
        }
        let mut last: Option<(u32, u8)> = None;
        for input in host.tape_for_test().iter() {
            let key = (input.tick, input.fighter.0);
            if let Some(prev) = last {
                assert!(prev <= key, "tape out of order: {prev:?} then {key:?}");
            }
            last = Some(key);
        }
        // Same-tick pair lands fighter-ordered (slot 0 before slot 1).
        let tick0: Vec<u8> = host
            .tape_for_test()
            .iter()
            .filter(|input| input.tick == 0)
            .map(|input| input.fighter.0)
            .collect();
        assert_eq!(tick0, vec![0, 1], "same-tick pair must be fighter-ordered");
    }

    #[test]
    fn two_peer_loopback_converges_under_latency_and_loss() {
        let hosted = NetGame::host(config()).unwrap();
        loopback_converges(hosted, false);
    }

    /// DSN1: the same loopback over the window's match. The host's server and
    /// the guest's client build from the host view's spec, so the converged
    /// history is rules v2 on the kit walls, not the old v1 open sand.
    #[test]
    fn spec_two_peer_loopback_converges_on_the_duel_match() {
        let hosted = NetGame::host_duel(config()).unwrap();
        loopback_converges(hosted, true);
    }

    fn loopback_converges((code, mut host): (String, NetGame), duel: bool) {
        assert_eq!(host.is_duel(), duel);
        let host_addr = host.local_addr().unwrap();
        let mut guest = NetGame::join(host_addr, &code).unwrap();
        assert_eq!(guest.is_duel(), duel, "the code carries the mode");
        assert!(host.is_host() && !guest.is_host());
        assert_eq!((host.local_slot(), host.remote_slot()), (0, 1));
        assert_eq!((guest.local_slot(), guest.remote_slot()), (1, 0));

        let mut up = FaultLink::new(LATENCY, 0xC10C);
        let mut down = FaultLink::new(LATENCY, 0xD0ED);
        // Cached down bytes for the lossless final catch-up.
        let mut down_cache: Vec<Vec<u8>> = Vec::new();
        let mut states_applied = 0u32;
        let mut mispredict_seen = 0u64;

        let steps = TICKS + 2 * LATENCY + 12;
        for step in 0..steps {
            if step < TICKS {
                let outcome = guest.tick_local(guest_input(step));
                assert!(outcome.input_accepted, "guest tick {step} rejected");
                for bytes in guest.drain_outbound() {
                    up.send(&bytes);
                }
            }
            // Transit over real localhost UDP, then into the delay queues.
            up.pump();
            down.pump();
            for bytes in up.due() {
                let applied = host.on_net_event(&bytes).expect("host stages");
                assert!(matches!(applied, NetApplied::InputsStaged { .. }));
            }
            // The host authors every tick whose inputs cleared the floor.
            while host.cursor() < TICKS && host.cursor() + LATENCY <= step {
                let cursor = host.cursor();
                let outcome = host.tick_local(host_input(cursor));
                assert!(outcome.input_accepted, "host tick {cursor} rejected");
                for bytes in host.drain_outbound() {
                    down_cache.push(bytes.clone());
                    down.send(&bytes);
                }
            }
            for bytes in down.due() {
                match guest.on_net_event(&bytes).expect("guest applies") {
                    NetApplied::StateApplied {
                        mispredicts,
                        desync,
                    } => {
                        states_applied += 1;
                        mispredict_seen += mispredicts.len() as u64;
                        assert_eq!(desync, None, "persistent desync after resimulation");
                    }
                    other => panic!("unexpected application: {other:?}"),
                }
            }
            up.tick();
            down.tick();
        }
        // Lossless catch-up: replay cached states until the guest holds the
        // final authority tick (duplicates apply to nothing).
        for bytes in down_cache {
            if guest.server_tick() == Some(TICKS - 1) {
                break;
            }
            let _ = guest.on_net_event(&bytes);
        }
        assert_eq!(host.authority().len(), TICKS as usize);
        assert_eq!(guest.authority().len(), TICKS as usize);
        assert_eq!(guest.server_tick(), Some(TICKS - 1));
        assert!(up.sent > 0 && down.sent > 0 && states_applied > 0);
        assert!(
            up.dropped + down.dropped > 0,
            "the loss matrix must have dropped at least one datagram"
        );
        assert!(
            mispredict_seen > 0,
            "loss must have forced at least one rollback resimulation"
        );
        assert_eq!(
            guest.authority(),
            host.authority(),
            "guest prediction must converge to server truth"
        );
        assert_eq!(
            guest.app().checkpoints(),
            host.authority(),
            "guest view must converge to server truth"
        );
        assert_eq!(
            host.app().checkpoints(),
            host.authority(),
            "host view must equal its authority tape-for-tape"
        );
        assert_eq!(guest.app().hp(), host.app().hp());
        assert_eq!(guest.last_report(), None);
        let v1 = App::new(config()).unwrap().spec();
        assert_eq!(
            host.app().spec() == v1,
            !duel,
            "the session plays the mode its code names"
        );
    }

    #[test]
    fn mispredict_names_the_first_divergent_tick() {
        let (code, mut host) = NetGame::host(small_config()).unwrap();
        let mut guest = NetGame::join(host.local_addr().unwrap(), &code).unwrap();
        // The guest predicts an attack; the host never receives it and
        // authors the tick AI-filled instead.
        let predicted = guest.tick_local(FighterInput {
            tick: 999,
            fighter: FighterId::new(1).expect("slot 1"),
            mx: 100,
            mz: 100,
            facing: 0,
            buttons: Buttons(ATTACK),
        });
        assert!(predicted.input_accepted);
        let truth = host.tick_local(host_input(0));
        assert!(truth.input_accepted);
        assert_ne!(
            predicted.checkpoint, truth.checkpoint,
            "a lost attack-plus-wish must diverge from AI fill"
        );
        let bytes = host.drain_outbound();
        assert_eq!(bytes.len(), 1);
        match guest.on_net_event(&bytes[0]).expect("state applies") {
            NetApplied::StateApplied {
                mispredicts,
                desync,
            } => {
                assert_eq!(mispredicts, vec![0], "divergence tick must be named");
                assert_eq!(desync, None, "rollback must converge, not desync");
            }
            other => panic!("unexpected application: {other:?}"),
        }
        assert_eq!(guest.last_mispredicts(), &[0]);
        assert_eq!(guest.authority(), host.authority());
        assert_eq!(guest.app().checkpoints(), host.authority());
    }

    #[test]
    fn first_divergence_is_pure_and_exact() {
        let (_code, mut host) = NetGame::host(small_config()).unwrap();
        for tick in 0..4 {
            assert!(host.tick_local(host_input(tick)).input_accepted);
        }
        let truth = host.authority().to_vec();
        assert_eq!(first_divergence(&truth, &truth), None);
        assert_eq!(first_divergence(&truth, &truth[..2]), None);
        assert_eq!(first_divergence(&truth[..2], &truth), None);
        assert_eq!(first_divergence(&[], &[]), None);
        let mut tampered = truth.clone();
        tampered[2].hash ^= 0x9E37;
        let report = first_divergence(&truth, &tampered).expect("must name tick 2");
        assert_eq!(report.first_tick, 2);
        assert_eq!(report.local_hash, truth[2].hash);
        assert_eq!(report.remote_hash, tampered[2].hash);
        // First means first: an earlier tamper wins.
        tampered[0].hash ^= 1;
        assert_eq!(
            first_divergence(&truth, &tampered)
                .expect("must name tick 0")
                .first_tick,
            0
        );
    }

    #[test]
    fn version_mismatch_refused_on_code_and_wire() {
        let (code, mut host) = NetGame::host(small_config()).unwrap();
        // Invitation from the future: join refuses before binding any role.
        let future = code.replacen(&format!("v{WIRE_VERSION}"), "v7", 1);
        match NetGame::join(host.local_addr().unwrap(), &future) {
            Err(err) => assert_eq!(err, GameError::Net(NetError::VersionMismatch { got: 7 })),
            Ok(_) => panic!("join must refuse a future wire version"),
        }
        // Tampered wire version: refused before anything is applied.
        let mut guest = NetGame::join(host.local_addr().unwrap(), &code).unwrap();
        assert!(guest.tick_local(guest_input(0)).input_accepted);
        let mut bytes = guest.drain_outbound().pop().expect("input packet");
        bytes[2] = WIRE_VERSION + 1;
        let before = host.cursor();
        assert_eq!(
            host.on_net_event(&bytes),
            Err(GameError::Net(NetError::VersionMismatch {
                got: WIRE_VERSION + 1,
            }))
        );
        assert_eq!(host.cursor(), before, "refused bytes must stage nothing");
        // Bad magic is refused the same way.
        bytes[0] ^= 0xFF;
        bytes[2] = WIRE_VERSION;
        assert_eq!(
            host.on_net_event(&bytes),
            Err(GameError::Net(NetError::BadMagic))
        );
    }

    #[test]
    fn late_and_foreign_inputs_diagnosed() {
        let (_code, mut host) = NetGame::host(GameConfig {
            fighters: 4,
            ..small_config()
        })
        .unwrap();
        // Wrong role: states at the host fail loudly.
        assert!(host.tick_local(host_input(0)).input_accepted);
        let state_bytes = host.drain_outbound().pop().expect("state packet");
        assert_eq!(
            host.on_net_event(&state_bytes),
            Err(GameError::Net(NetError::BadKind(KIND_STATE)))
        );
        // Foreign human slot (2 is a fighter, but the pinned remote is 1).
        let foreign = FighterInput {
            tick: 1,
            fighter: FighterId::new(2).expect("slot 2"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        let bytes =
            engine_net::session::encode_input(1, std::slice::from_ref(&foreign)).expect("encode");
        assert_eq!(
            host.on_net_event(&bytes),
            Err(GameError::Net(NetError::BadInput))
        );
        // Far-future tick: flood guard, refused.
        let far = FighterInput {
            tick: host.cursor() + MAX_AHEAD_TICKS + 1,
            fighter: FighterId::new(1).expect("slot 1"),
            ..guest_input(0)
        };
        let bytes = engine_net::session::encode_input(far.tick, std::slice::from_ref(&far))
            .expect("encode");
        assert_eq!(
            host.on_net_event(&bytes),
            Err(GameError::Net(NetError::BadInput))
        );
        // Late input for the already-authored tick 0: counted, dropped.
        let late = FighterInput {
            tick: 0,
            ..guest_input(0)
        };
        let bytes =
            engine_net::session::encode_input(0, std::slice::from_ref(&late)).expect("encode");
        assert_eq!(
            host.on_net_event(&bytes).expect("late counts"),
            NetApplied::InputsStaged { staged: 0, late: 1 }
        );
        assert_eq!(host.cursor(), 1, "late inputs must not move authority");
    }

    /// Reliable receive: loopback UDP has no arrival guarantee by the time
    /// a single non-blocking `poll()` runs, so a lone `poll()` flakes to
    /// `[]` under full-workspace load. Drain until `want` items arrive or
    /// the deadline hits; arrivals are asserted, never timing.
    fn poll_until(host: &mut NetGame, want: usize) -> Vec<NetApplied> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut out = Vec::new();
        while out.len() < want {
            let mut got = host.poll().expect("polls");
            out.append(&mut got);
            if out.len() >= want || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        out
    }

    #[test]
    fn real_sockets_deliver_both_directions() {
        let (code, mut host) = NetGame::host(small_config()).unwrap();
        let mut guest = NetGame::join(host.local_addr().unwrap(), &code).unwrap();
        host.set_peer(guest.local_addr().unwrap());
        // Guest input over the real socket; the host learns the guest.
        assert!(guest.tick_local(guest_input(0)).input_accepted);
        assert_eq!(guest.flush().expect("flush inputs"), 1);
        let staged = poll_until(&mut host, 1);
        assert_eq!(staged.len(), 1);
        assert_eq!(host.peer(), Some(guest.local_addr().unwrap()));
        // Host state over the real socket; the guest names the tick where its
        // prediction (which could not know the host input) diverged, then
        // converges exactly through rollback.
        assert!(host.tick_local(host_input(0)).input_accepted);
        assert_eq!(host.flush().expect("flush states"), 1);
        match poll_until(&mut guest, 1).as_slice() {
            [NetApplied::StateApplied {
                mispredicts,
                desync,
            }] => {
                assert_eq!(*mispredicts, vec![0]);
                assert_eq!(*desync, None);
            }
            other => panic!("unexpected poll result: {other:?}"),
        }
        assert_eq!(guest.authority(), host.authority());
        assert_eq!(guest.app().checkpoints(), host.authority());
    }

    #[test]
    fn snapshot_path_feeds_the_interpolated_remote_view() {
        let (code, _) = NetGame::host(small_config()).unwrap();
        let host_addr: SocketAddr = "127.0.0.1:9".parse().expect("addr");
        let mut guest = NetGame::join(host_addr, &code).unwrap();
        assert_eq!(guest.interp_len(), 0);
        assert_eq!(guest.remote_view(10), None);
        let snap = |tick: u32, x_m: f32| {
            FullSnap::new(
                tick,
                vec![
                    FighterSnap::quantize(0, [x_m, 0.0, 2.0], 0.5, 1000, 100).expect("slot"),
                    FighterSnap::quantize(1, [5.0, 0.0, 0.0], -1.0, 880, 60).expect("slot"),
                ],
            )
            .expect("snapshot")
        };
        let first = encode_snapshot(&snap(10, 1.0)).expect("encode");
        assert_eq!(
            guest.on_net_event(&first).expect("buffer"),
            NetApplied::SnapshotBuffered { tick: 10 }
        );
        assert_eq!(guest.interp_len(), 1);
        // One sample: the sample itself, flagged as clamped (no extrapolation).
        let solo = guest.remote_view(10).expect("solo");
        assert!(solo.clamped);
        assert_eq!(solo.state.fighters[0].x_mm, 1000);
        let second = encode_snapshot(&snap(16, 2.0)).expect("encode");
        assert_eq!(
            guest.on_net_event(&second).expect("buffer"),
            NetApplied::SnapshotBuffered { tick: 16 }
        );
        // Midpoint blends with the reused net integer math, not a reimplementation.
        let mid = guest.remote_view(13).expect("midpoint");
        assert!(!mid.clamped);
        assert_eq!(mid.state.fighters[0].x_mm, 1500);
    }

    #[test]
    fn server_ahead_replays_truth_without_predicting() {
        let (code, mut host) = NetGame::host(small_config()).unwrap();
        let mut guest = NetGame::join(host.local_addr().unwrap(), &code).unwrap();
        // The host authors tick 0 before the guest predicts anything; the
        // guest input never arrived, so the server AI-filled slot 1.
        assert!(host.tick_local(host_input(0)).input_accepted);
        let bytes = host.drain_outbound().pop().expect("state packet");
        guest.on_net_event(&bytes).expect("state applies");
        assert_eq!(guest.server_tick(), Some(0));
        // The next local tick replays authority: the AI-filled slot advances
        // empty, exactly like the server did.
        let outcome = guest.tick_local(guest_input(0));
        assert!(outcome.input_accepted);
        assert_eq!(outcome.checkpoint, host.authority().first().copied());
        assert_eq!(guest.app().checkpoints(), host.authority());
    }

    #[test]
    fn single_player_path_unchanged_by_lan_hooks() {
        // The default constructor still seats slot 0 with single-player behavior.
        let legacy = App::new(small_config()).unwrap();
        assert_eq!(legacy.human_slot(), super::super::HUMAN_SLOT);
        let slotted = App::new_for_slot(small_config(), super::super::HUMAN_SLOT).unwrap();
        assert_eq!(slotted.human_slot(), super::super::HUMAN_SLOT);
        // Unknown slot refused at construction.
        match App::new_for_slot(small_config(), 2) {
            Err(err) => assert_eq!(err, GameError::BadConfig("human_slot")),
            Ok(_) => panic!("slot 2 of 2 must be refused"),
        }
        // Same scripted inputs give identical sessions on both constructors.
        fn run(
            mut app: App,
        ) -> (
            Vec<Checkpoint>,
            [u16; engine_input::MAX_PARTICIPANTS as usize],
        ) {
            for tick in 0..16 {
                let outcome = app.tick(FighterInput {
                    tick,
                    fighter: FighterId::new(0).expect("slot 0"),
                    mx: 100,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                });
                assert!(outcome.input_accepted);
            }
            (app.checkpoints().to_vec(), *app.hp())
        }
        let (legacy_cps, legacy_hp) = run(App::new(small_config()).unwrap());
        let (slotted_cps, slotted_hp) =
            run(App::new_for_slot(small_config(), super::super::HUMAN_SLOT).unwrap());
        assert_eq!(legacy_cps, slotted_cps);
        assert_eq!(legacy_hp, slotted_hp);
        // The local slot can never be staged as remote.
        let mut app = App::new(small_config()).unwrap();
        let selfie = FighterInput {
            tick: 0,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(app.offer_remote(selfie), Err(GameError::BadRemote("slot")));
        assert_eq!(app.cursor(), 0, "refused offers must not advance");
    }

    #[test]
    fn remote_offer_rules_reject_loudly() {
        let mut app = App::new_for_slot(small_config(), 0).unwrap();
        let good = FighterInput {
            tick: 999,
            fighter: FighterId::new(1).expect("slot 1"),
            mx: 50,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        // Caller ticks are transport order: staged at the cursor.
        app.offer_remote(good).expect("stage");
        // Same slot staged twice for one tick: duplicate, rejected.
        assert_eq!(
            app.offer_remote(good),
            Err(GameError::BadRemote("duplicate"))
        );
        let outcome = app.tick(FighterInput {
            tick: 999,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        });
        assert!(outcome.input_accepted);
        assert_eq!(outcome.tick, 0);
        assert_eq!(app.cursor(), 1);
        // Out-of-range and unknown slots fail without staging.
        let bad_range = FighterInput {
            tick: 1,
            fighter: FighterId::new(1).expect("slot 1"),
            mx: 101,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(
            app.offer_remote(bad_range),
            Err(GameError::BadRemote("range"))
        );
        // Session end: offers stop, ticks reject, nothing moves.
        let mut short = App::new_for_slot(
            GameConfig {
                ticks: 1,
                ..small_config()
            },
            0,
        )
        .unwrap();
        short.offer_remote(good).expect("stage");
        assert!(short.tick(host_input(0)).input_accepted);
        assert_eq!(short.offer_remote(good), Err(GameError::BadRemote("over")));
        assert!(!short.tick(host_input(1)).input_accepted);
        assert_eq!(short.cursor(), 1);
        // Empty ticks advance AI-filled and report the tick as run.
        let mut empty = App::new(small_config()).unwrap();
        let outcome = empty.tick_empty();
        assert!(outcome.input_accepted);
        assert_eq!(outcome.tick, 0);
        assert!(outcome.checkpoint.is_some());
        assert_eq!(empty.cursor(), 1);
    }
}
