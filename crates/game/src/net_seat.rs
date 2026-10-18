//! net_seat: one seat of a two-human arena match over UDP, driven once per
//! frame by the window (`arena --host` / `arena --join`) or by
//! [`play_headless`] (the loopback proof).
//!
//! Consumer: `bin/arena.rs` and `bin/arena_frame` (window play, the headless
//! proof). Data owner: the seat owns its [`NetGame`] plus the pacing and
//! liveness state around it; the session's view is the `App` the window draws
//! (lent with [`NetSeat::swap_view`]). Units: fixed ticks at 60 Hz, wall
//! milliseconds from the caller's clock (`now_ms`, any monotonic origin).
//!
//! The match is lockstep with the host as authority: the host plays slot 0
//! with no input delay and authors a tick only once the guest's input for it
//! arrived (or the guest went silent past [`GONE_AFTER_MS`], then AI plays the
//! guest's slot). The guest plays slot 1: it sends each input `lead` ticks
//! ahead of the newest authority it saw (`lead` follows the measured round
//! trip, [`lead_for_rtt`]) and its view replays only authority, so it never
//! rolls back and never draws a guess. Determinism: the same two input
//! streams give the same checkpoints on both peers whatever the network did
//! (loss, delay, reorder, a relay hop): a late input stalls the host instead
//! of changing the match. Sockets and clocks never reach a hash.
//!
//! Errors: [`GameError`]; a host that plays another match refuses the guest
//! with [`GameError::SpecMismatch`] on both sides, a host that never answers
//! times out loudly after [`JOIN_GIVE_UP_MS`].

#[cfg(not(target_arch = "wasm32"))]
use std::net::UdpSocket;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use engine_input::fighter::{Buttons, ATTACK, BLOCK, DODGE, THROW_SPEAR};
use engine_input::{FighterId, FighterInput};
use engine_net::link::{LinkProfile, LinkStats};

use super::netgame::{DesyncReport, Invite, NetApplied, NetGame, NetRules, STATE_COVER};
use super::{App, GameConfig, GameError, TickOutcome};

/// UDP port a host listens on by default.
pub const DEFAULT_PORT: u16 = 7043;
/// A guest repeats its hello this often until the host answers.
pub const HELLO_EVERY_MS: u64 = 250;
/// A guest that hears no welcome for this long gives up loudly.
pub const JOIN_GIVE_UP_MS: u64 = 20_000;
/// Silence after which the peer counts as gone: the host plays on with AI in
/// the guest's slot and takes a new hello; the guest's view waits.
pub const GONE_AFTER_MS: u64 = 5_000;
/// A guest acks its coverage (and pings) every this many frames; every frame
/// while it sees a hole or once its view finished.
pub const NEED_EVERY_FRAMES: u32 = 6;
/// The host answers a hole at most once per this many frames.
pub const REPAIR_EVERY_FRAMES: u32 = 3;
/// State windows one repair answer re-sends at most.
pub const REPAIR_PACKETS: usize = 8;
/// After the last tick the host keeps serving (re-sending the last window,
/// repairing) until the guest holds every tick or this much time passed.
pub const LINGER_MS: u64 = 4_000;
/// Input lead ticks on top of the round trip.
pub const LEAD_MARGIN_TICKS: u32 = 2;
/// Smallest input lead in ticks.
pub const MIN_LEAD_TICKS: u32 = 2;
/// Largest input lead in ticks (half a second); the host refuses inputs far past its tip.
pub const MAX_LEAD_TICKS: u32 = 30;
/// Round-trip samples the lead reads (the slowest of them wins).
const RTT_SAMPLES: usize = 8;
/// Inputs a guest sends at most in one frame (catching up after a stall).
const MAX_SENDS_PER_FRAME: u32 = 8;
/// Authority ticks a guest's view may hold back before it replays one extra per frame.
const VIEW_SLACK_TICKS: u32 = 2;
/// Authority ticks behind past which a guest's view jumps to the slack at once.
const VIEW_SNAP_TICKS: u32 = 30;
/// Relay registration is retried every this many frames until confirmed.
const REGISTER_EVERY_FRAMES: u32 = 30;
/// Fixed tick length of the headless loop.
const TICK: Duration = Duration::from_micros(16_667);

/// Input lead in ticks for a round trip of `rtt_ms`: the round trip in ticks
/// (rounded up) plus [`LEAD_MARGIN_TICKS`], clamped to the lead bounds. Pure.
#[must_use]
pub fn lead_for_rtt(rtt_ms: u64) -> u32 {
    let ticks = rtt_ms.saturating_mul(60).div_ceil(1000);
    let ticks = u32::try_from(ticks).unwrap_or(u32::MAX);
    ticks
        .saturating_add(LEAD_MARGIN_TICKS)
        .clamp(MIN_LEAD_TICKS, MAX_LEAD_TICKS)
}

/// Something the caller should say about the session (the window prints
/// these as `arena net:` lines).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SeatNote {
    /// Host: a guest paired.
    Paired {
        /// The guest's match code.
        guest_code: String,
    },
    /// Host: a hello for another match was refused (the guest was told).
    Refused {
        /// The refused guest's match code.
        guest_code: String,
    },
    /// Guest: the host welcomed this seat.
    Welcomed {
        /// First measured round trip, milliseconds.
        rtt_ms: u64,
        /// Input lead it set, ticks.
        lead: u32,
    },
    /// The relay confirmed this seat.
    RelayRegistered,
    /// The peer went silent past [`GONE_AFTER_MS`].
    PeerGone,
    /// The peer is heard again.
    PeerBack,
    /// The guest's view left the host's checkpoints (nondeterminism).
    Desync(DesyncReport),
}

/// Counters of one seat, for the receipt line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SeatStats {
    /// Host: frames the authoring waited on the guest's input.
    pub stalls: u64,
    /// Host: ticks authored with the guest gone (AI in its slot).
    pub ai_ticks: u64,
    /// Host: state windows re-sent for holes.
    pub repairs: u64,
    /// Guest: coverage acks sent.
    pub needs: u64,
    /// Guest: frames its view had no new authority to show.
    pub view_waits: u64,
    /// Last measured round trip, milliseconds.
    pub rtt_ms: u64,
    /// Slowest measured round trip, milliseconds.
    pub rtt_max_ms: u64,
    /// Largest input lead used, ticks.
    pub lead_max: u32,
    /// Hellos refused for another match.
    pub refused: u64,
}

/// One seat: the session plus its pacing, liveness and counters.
pub struct NetSeat {
    game: NetGame,
    invite: Invite,
    frames: u32,
    started_ms: Option<u64>,
    last_heard_ms: Option<u64>,
    /// Host: a guest paired at least once (authoring may start).
    ever_paired: bool,
    gone: bool,
    /// Guest: the host's welcome arrived.
    welcomed: bool,
    last_hello_ms: Option<u64>,
    rtts: Vec<u64>,
    lead: u32,
    /// Host: the guest's newest coverage ack.
    guest_need: u32,
    last_repair_frame: u32,
    /// Host: when the last tick was authored (the linger clock).
    ended_ms: Option<u64>,
    desync_noted: bool,
    stats: SeatStats,
}

impl NetSeat {
    /// Host a match on `bind` (`0.0.0.0:<port>` for a LAN, loopback for one
    /// PC); with `relay`, every datagram goes through that relay under a
    /// fresh relay id the invitation carries.
    pub fn host(
        config: GameConfig,
        rules: NetRules,
        bind: SocketAddr,
        relay: Option<SocketAddr>,
    ) -> Result<Self, GameError> {
        let relay = relay.map(|addr| (addr, fresh_relay_id(0)));
        let (invite, game) = NetGame::host_on(config, rules, bind, relay)?;
        Ok(Self::around(game, invite))
    }

    /// Join the match `code` names at `target` (the host, or the relay when
    /// the code carries a relay id).
    pub fn join(target: SocketAddr, code: &str) -> Result<Self, GameError> {
        let invite = Invite::decode(code)?;
        let me = fresh_relay_id(invite.relay_host.unwrap_or(0));
        let game = NetGame::join_lockstep(target, code, me)?;
        Ok(Self::around(game, invite))
    }

    fn around(game: NetGame, invite: Invite) -> Self {
        Self {
            game,
            invite,
            frames: 0,
            started_ms: None,
            last_heard_ms: None,
            ever_paired: false,
            gone: false,
            welcomed: false,
            last_hello_ms: None,
            rtts: Vec::with_capacity(RTT_SAMPLES),
            lead: lead_for_rtt(0),
            guest_need: 0,
            last_repair_frame: 0,
            ended_ms: None,
            desync_noted: false,
            stats: SeatStats::default(),
        }
    }

    /// The invitation: the code a guest types (host) or the one it joined.
    #[must_use]
    pub fn invite(&self) -> Invite {
        self.invite
    }

    /// True on the authoring seat.
    #[must_use]
    pub fn is_host(&self) -> bool {
        self.game.is_host()
    }

    /// This seat's fighter slot (0 host, 1 guest).
    #[must_use]
    pub fn local_slot(&self) -> u8 {
        self.game.local_slot()
    }

    /// The session (for receipts and tests).
    #[must_use]
    pub fn game(&self) -> &NetGame {
        &self.game
    }

    /// Socket address this seat listens on.
    pub fn local_addr(&self) -> Result<SocketAddr, GameError> {
        self.game
            .local_addr()
            .map_err(|err| GameError::Io(err.kind()))
    }

    /// Lend the session's view to a window `App` slot (see [`NetGame::swap_view`]).
    pub fn swap_view(&mut self, slot: &mut App) {
        self.game.swap_view(slot);
    }

    /// Seed a fault link on what this seat sends.
    pub fn set_fault(&mut self, profile: LinkProfile, seed: u64) {
        self.game.set_fault(profile, seed);
    }

    /// Counters so far.
    #[must_use]
    pub fn stats(&self) -> SeatStats {
        SeatStats {
            lead_max: self.stats.lead_max.max(self.lead),
            ..self.stats
        }
    }

    /// The guest's current input lead in ticks.
    #[must_use]
    pub fn lead(&self) -> u32 {
        self.lead
    }

    /// True while the peer counts as gone.
    #[must_use]
    pub fn peer_gone(&self) -> bool {
        self.gone
    }

    /// Host: a guest paired at least once (the window leaves "waiting for a
    /// guest"). Guest: the host welcomed this seat.
    #[must_use]
    pub fn connected(&self) -> bool {
        if self.is_host() {
            self.ever_paired
        } else {
            self.welcomed
        }
    }

    /// One frame's receive side: register with the relay, read every
    /// datagram, then the timers (hello, coverage ack, repair, liveness,
    /// linger). Returns what to say. Call [`NetSeat::flush`] at the end of
    /// the frame, after the ticks queued their packets.
    pub fn pump(&mut self, now_ms: u64) -> Result<Vec<SeatNote>, GameError> {
        self.frames = self.frames.wrapping_add(1);
        let started = *self.started_ms.get_or_insert(now_ms);
        if self.game.relay_pending() && self.frames % REGISTER_EVERY_FRAMES == 1 {
            self.game.relay_register()?;
        }
        let mut notes = Vec::new();
        for applied in self.game.poll()? {
            self.on_applied(applied, now_ms, &mut notes)?;
        }
        if self.connected() && !self.gone {
            let heard = self.last_heard_ms.unwrap_or(now_ms);
            if now_ms.saturating_sub(heard) > GONE_AFTER_MS {
                self.gone = true;
                self.game.unpair();
                notes.push(SeatNote::PeerGone);
            }
        }
        if self.is_host() {
            self.host_timers(now_ms);
        } else {
            self.guest_timers(now_ms, started)?;
            if let Some(report) = self.game.last_report().filter(|_| !self.desync_noted) {
                self.desync_noted = true;
                notes.push(SeatNote::Desync(report));
            }
        }
        Ok(notes)
    }

    fn on_applied(
        &mut self,
        applied: NetApplied,
        now_ms: u64,
        notes: &mut Vec<SeatNote>,
    ) -> Result<(), GameError> {
        match applied {
            NetApplied::Hello {
                accepted: false,
                guest_code,
                ..
            } => {
                self.stats.refused += 1;
                notes.push(SeatNote::Refused { guest_code });
                return Ok(());
            }
            NetApplied::Hello { guest_code, .. } if !self.ever_paired => {
                self.ever_paired = true;
                notes.push(SeatNote::Paired { guest_code });
            }
            NetApplied::Welcome {
                accepted: false,
                host_identity,
                ..
            } => {
                return Err(GameError::SpecMismatch {
                    host: host_identity,
                    guest: self.game.identity(),
                });
            }
            NetApplied::Welcome { .. } if !self.welcomed => {
                self.welcomed = true;
                let rtt = now_ms.saturating_sub(self.last_hello_ms.unwrap_or(now_ms));
                self.sample_rtt(rtt);
                notes.push(SeatNote::Welcomed {
                    rtt_ms: rtt,
                    lead: self.lead,
                });
            }
            NetApplied::Need { need } => {
                self.guest_need = self.guest_need.max(need);
                self.repair(need);
            }
            NetApplied::Pong { stamp } => {
                let rtt = u64::from(low_ms(now_ms).wrapping_sub(stamp));
                self.sample_rtt(rtt);
            }
            NetApplied::RelayRegistered => {
                notes.push(SeatNote::RelayRegistered);
                return Ok(());
            }
            _ => {}
        }
        self.last_heard_ms = Some(now_ms);
        if self.gone {
            self.gone = false;
            notes.push(SeatNote::PeerBack);
        }
        Ok(())
    }

    /// Host: answer a hole older than the newest window, rate-limited.
    fn repair(&mut self, need: u32) {
        let cover = u32::try_from(STATE_COVER).unwrap_or(u32::MAX);
        let tip = self.game.cursor();
        let hole = need.saturating_add(cover) < tip || (self.ended_ms.is_some() && need < tip);
        if hole && self.frames.wrapping_sub(self.last_repair_frame) >= REPAIR_EVERY_FRAMES {
            self.last_repair_frame = self.frames;
            let sent = self.game.queue_repair(need, REPAIR_PACKETS);
            self.stats.repairs += u64::try_from(sent).unwrap_or(u64::MAX);
        }
    }

    fn host_timers(&mut self, now_ms: u64) {
        if self.game.cursor() >= self.game.config().ticks {
            let ended = *self.ended_ms.get_or_insert(now_ms);
            if self.frames.is_multiple_of(4) && now_ms.saturating_sub(ended) < LINGER_MS {
                self.game.resend_last_state();
            }
        }
    }

    fn guest_timers(&mut self, now_ms: u64, started: u64) -> Result<(), GameError> {
        if !self.welcomed {
            if now_ms.saturating_sub(started) > JOIN_GIVE_UP_MS {
                return Err(GameError::Io(std::io::ErrorKind::TimedOut));
            }
            let due = self
                .last_hello_ms
                .is_none_or(|sent| now_ms.saturating_sub(sent) >= HELLO_EVERY_MS);
            if due {
                if self.last_hello_ms.is_some() {
                    self.game.queue_hello();
                }
                self.last_hello_ms = Some(now_ms);
            }
            return Ok(());
        }
        let cover = u32::try_from(STATE_COVER).unwrap_or(u32::MAX);
        let covered = self.game.covered();
        let hole = self
            .game
            .server_tick()
            .is_some_and(|latest| latest >= covered.saturating_add(cover));
        let done = self.game.cursor() >= self.game.config().ticks;
        if !done && covered <= self.game.cursor() {
            self.stats.view_waits += 1;
        }
        if hole || done || self.frames.is_multiple_of(NEED_EVERY_FRAMES) {
            self.game.queue_need(low_ms(now_ms));
            self.stats.needs += 1;
        }
        Ok(())
    }

    fn sample_rtt(&mut self, rtt_ms: u64) {
        if self.rtts.len() == RTT_SAMPLES {
            self.rtts.remove(0);
        }
        self.rtts.push(rtt_ms);
        self.stats.rtt_ms = rtt_ms;
        self.stats.rtt_max_ms = self.stats.rtt_max_ms.max(rtt_ms);
        let slowest = self.rtts.iter().copied().max().unwrap_or(rtt_ms);
        self.lead = lead_for_rtt(slowest);
        self.stats.lead_max = self.stats.lead_max.max(self.lead);
    }

    /// Send everything this frame queued (fault link and relay applied).
    /// While no guest is paired the queue is dropped, so a host waiting
    /// alone never piles up windows.
    pub fn flush(&mut self) -> Result<(), GameError> {
        self.game.flush().map_err(|err| GameError::Io(err.kind()))?;
        if !self.game.paired() {
            self.game.drain_outbound();
        }
        Ok(())
    }

    /// Host: the next tick may be authored now (a guest paired, its input for
    /// the tick arrived or it is gone, and the match is not over).
    #[must_use]
    pub fn can_author(&self) -> bool {
        self.is_host()
            && self.ever_paired
            && self.game.cursor() < self.game.config().ticks
            && (self.gone || self.game.remote_ready())
    }

    /// Host: count one frame whose authoring waited on the guest.
    pub fn note_stall(&mut self) {
        self.stats.stalls += 1;
    }

    /// Host: author the next tick with the local `input` (stamped to the
    /// tick); the guest's staged input rides along, or AI plays its slot when
    /// it is gone. A refused tick fails loudly.
    pub fn author(&mut self, input: FighterInput) -> Result<TickOutcome, GameError> {
        let outcome = self.game.tick_local(input);
        if !outcome.input_accepted {
            return Err(GameError::BadRemote("slot"));
        }
        if self.gone {
            self.stats.ai_ticks += 1;
        }
        Ok(outcome)
    }

    /// Guest: inputs owed this frame, so the newest sent tick stays `lead`
    /// ticks past the newest authority seen; none before the welcome.
    #[must_use]
    pub fn sends_owed(&self) -> u32 {
        if self.is_host() || !self.welcomed {
            return 0;
        }
        let next = self.next_send_tick();
        let floor = self.game.server_tick().map_or(0, |latest| latest + 1);
        let target = floor
            .saturating_add(self.lead)
            .min(self.game.config().ticks);
        target.saturating_sub(next).min(MAX_SENDS_PER_FRAME)
    }

    /// Guest: the tick the next [`NetSeat::send`] stamps.
    #[must_use]
    pub fn next_send_tick(&self) -> u32 {
        let floor = self.game.server_tick().map_or(0, |latest| latest + 1);
        self.game.send_cursor().max(floor)
    }

    /// Guest: send the local input for [`NetSeat::next_send_tick`].
    pub fn send(&mut self, input: FighterInput) -> Result<u32, GameError> {
        self.game.send_ahead(input)
    }

    /// Guest: authority ticks the view replays this frame for `wall_steps`
    /// fixed steps of wall time: the steps, one more while it holds more than
    /// a little slack, everything but the slack after a long wait; never past
    /// what is known.
    #[must_use]
    pub fn views_owed(&self, wall_steps: u32) -> u32 {
        if self.is_host() {
            return 0;
        }
        let known = self.game.covered().saturating_sub(self.game.cursor());
        let wanted = if known > VIEW_SNAP_TICKS {
            known - VIEW_SLACK_TICKS
        } else if known > VIEW_SLACK_TICKS {
            wall_steps + 1
        } else {
            wall_steps
        };
        wanted.min(known)
    }

    /// Guest: replay the next authoritative tick into the view; `None` when
    /// authority has not arrived.
    pub fn replay(&mut self) -> Result<Option<TickOutcome>, GameError> {
        self.game.replay_next()
    }

    /// Ticks the session's view ran.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.game.cursor()
    }

    /// The seat is done: the guest's view ran every tick; the host authored
    /// every tick and the guest holds them all, went silent, or the linger ran out.
    #[must_use]
    pub fn finished(&self, now_ms: u64) -> bool {
        let ticks = self.game.config().ticks;
        if self.game.cursor() < ticks {
            return false;
        }
        if !self.is_host() {
            return true;
        }
        let lingered = self
            .ended_ms
            .is_some_and(|ended| now_ms.saturating_sub(ended) >= LINGER_MS);
        self.guest_need >= ticks || self.gone || lingered
    }

    /// The fault link's counters, when one is set.
    #[must_use]
    pub fn fault_stats(&self) -> Option<LinkStats> {
        self.game.fault_stats()
    }

    /// One receipt line: role, rules, ticks, final hash, counters, round
    /// trip, lead and, with a fault link, what it did to the sent datagrams.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        let stats = self.stats();
        let hash = self
            .game
            .authority()
            .last()
            .map_or_else(|| "-".to_string(), |point| format!("{:016x}", point.hash));
        let fault = self.fault_stats().map_or_else(String::new, |link| {
            format!(
                " fault_sent={} fault_dropped={} fault_reordered={} fault_delay_p50={} fault_delay_max={}",
                link.sent,
                link.dropped,
                link.reordered,
                link.delay_pct(50),
                link.delay_max()
            )
        });
        let desync = self.game.last_report().map_or_else(
            || "none".to_string(),
            |report| report.first_tick.to_string(),
        );
        format!(
            "role={} rules={} ticks={} hash={hash} desync={desync} stalls={} ai_ticks={} repairs={} needs={} view_waits={} rtt_ms={} rtt_max_ms={} lead_max={} refused={} stray={}{fault}",
            if self.is_host() { "host" } else { "guest" },
            self.game.rules().label(),
            self.game.cursor(),
            stats.stalls,
            stats.ai_ticks,
            stats.repairs,
            stats.needs,
            stats.view_waits,
            stats.rtt_ms,
            stats.rtt_max_ms,
            stats.lead_max,
            stats.refused,
            self.game.stray(),
        )
    }
}

/// The low 32 bits of a millisecond clock: the round-trip stamp.
fn low_ms(now_ms: u64) -> u32 {
    u32::try_from(now_ms & u64::from(u32::MAX)).unwrap_or(0)
}

/// A nonzero relay id unlike `avoid`, mixed from the wall clock and the
/// process id (two processes on one PC never share one). Never hashed.
#[cfg(not(target_arch = "wasm32"))]
fn fresh_relay_id(avoid: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let seed = u64::try_from(nanos & u128::from(u64::MAX)).unwrap_or(0)
        ^ u64::from(std::process::id()).rotate_left(32);
    let mut id = engine_core::mix64(seed);
    while id == 0 || id == avoid {
        id = engine_core::mix64(id.wrapping_add(1));
    }
    id
}

/// A nonzero relay id unlike `avoid`. A browser tab has no wall clock at
/// this seam, so the wasm32 build mixes a fixed seed. Never hashed.
#[cfg(target_arch = "wasm32")]
fn fresh_relay_id(avoid: u64) -> u64 {
    let mut id = engine_core::mix64(1);
    while id == 0 || id == avoid {
        id = engine_core::mix64(id.wrapping_add(1));
    }
    id
}

/// This PC's address on its LAN, as other machines see it: the source
/// address the OS would route an outside datagram from (no datagram is
/// sent). `None` when there is no route.
#[must_use]
#[cfg(not(target_arch = "wasm32"))]
pub fn lan_ip() -> Option<IpAddr> {
    let probe = UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect("192.0.2.1:9").ok()?;
    let ip = probe.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// A browser tab has no raw sockets: no LAN address to report.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn lan_ip() -> Option<IpAddr> {
    None
}

/// The scripted input of an automated run: a pure function of the tick and
/// the slot, so two runs (and two peers) send the same streams whatever the
/// clocks did. The fighters walk toward each other across the pit for four
/// seconds, then circle, swing, block, dodge and throw (the throw is masked
/// on a match without the spear).
#[must_use]
pub fn script_input(tick: u32, slot: u8) -> FighterInput {
    let toward: i8 = if slot.is_multiple_of(2) { 100 } else { -100 };
    let (mx, mz) = if tick < 240 {
        (toward, 0)
    } else if (tick / 45).is_multiple_of(2) {
        (toward / 4, 70)
    } else {
        (toward / 4, -70)
    };
    let phase = tick.wrapping_add(u32::from(slot) * 11);
    let buttons = match phase % 150 {
        75 => THROW_SPEAR,
        0 | 20 | 40 | 100 | 120 => ATTACK,
        60..=66 => BLOCK,
        90 => DODGE,
        _ => 0,
    };
    let facing = if slot.is_multiple_of(2) { 9000 } else { -9000 };
    FighterInput {
        tick,
        fighter: FighterId(slot),
        mx,
        mz,
        facing,
        buttons: Buttons(buttons),
    }
}

impl NetSeat {
    /// One whole frame of one fixed step with no window: pump, then the host
    /// authors one tick (or counts a stall) while the guest sends what it
    /// owes and replays what it may, the local slot's input from `input`
    /// (tick, slot); then flush. Returns the notes.
    pub fn step_frame(
        &mut self,
        now_ms: u64,
        mut input: impl FnMut(u32, u8) -> FighterInput,
    ) -> Result<Vec<SeatNote>, GameError> {
        let notes = self.pump(now_ms)?;
        let slot = self.local_slot();
        if self.is_host() {
            if self.can_author() {
                let tick = self.cursor();
                self.author(input(tick, slot))?;
            } else if self.connected() && self.cursor() < self.game.config().ticks {
                self.note_stall();
            }
        } else {
            for _ in 0..self.sends_owed() {
                let tick = self.next_send_tick();
                self.send(input(tick, slot))?;
            }
            for _ in 0..self.views_owed(1) {
                self.replay()?;
            }
        }
        self.flush()?;
        Ok(notes)
    }
}

/// Play one seat to the end on the scripted input, one [`NetSeat::step_frame`]
/// per fixed tick of wall time. `say` hears every note as it happens. Leaves
/// after the seat finished (a few more frames, so the last acks go out), or
/// fails loudly at `deadline_ms`.
#[cfg(not(target_arch = "wasm32"))]
pub fn play_headless(
    seat: &mut NetSeat,
    deadline_ms: u64,
    mut say: impl FnMut(&SeatNote),
) -> Result<(), GameError> {
    let start = Instant::now();
    let mut next = start;
    let mut after = 0u32;
    loop {
        let now_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        for note in seat.step_frame(now_ms, script_input)? {
            say(&note);
        }
        if seat.finished(now_ms) {
            after += 1;
            if after > 10 {
                return Ok(());
            }
        }
        if now_ms > deadline_ms {
            return Err(GameError::Io(std::io::ErrorKind::TimedOut));
        }
        next += TICK;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

/// A browser tab has no blocking loop over wall time: the wasm32 build
/// refuses instead of parking the tab.
#[cfg(target_arch = "wasm32")]
pub fn play_headless(
    _seat: &mut NetSeat,
    _deadline_ms: u64,
    _say: impl FnMut(&SeatNote),
) -> Result<(), GameError> {
    Err(GameError::Io(std::io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_net::relay::{RelayConfig, RelayHub};
    use std::net::{Ipv4Addr, SocketAddr};

    const TICKS: u32 = 240;

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: TICKS,
            fighters: 3,
            half_extent_m: 20.0,
        }
    }

    fn loopback() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
    }

    /// An in-process copy of `net_relay`: the hub on its own loopback socket.
    struct Relay {
        sock: UdpSocket,
        hub: RelayHub,
    }

    impl Relay {
        fn new() -> Self {
            let sock = UdpSocket::bind(loopback()).expect("relay socket");
            sock.set_nonblocking(true).expect("nonblocking");
            Self {
                sock,
                hub: RelayHub::new(RelayConfig::default()),
            }
        }

        fn addr(&self) -> SocketAddr {
            self.sock.local_addr().expect("relay addr")
        }

        fn pump(&mut self) {
            let mut buf = [0u8; 2048];
            loop {
                match self.sock.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        for (to, bytes) in self.hub.handle(src, &buf[..n]) {
                            self.sock.send_to(&bytes, to).expect("relay send");
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => {}
                    Err(_) => break,
                }
            }
        }
    }

    /// Both seats to the end over real loopback sockets on a synthetic 60 Hz
    /// clock (a short sleep per frame lets the datagrams land). Returns the
    /// host's and the guest's final checkpoint hash.
    fn play_pair(
        host: &mut NetSeat,
        guest: &mut NetSeat,
        mut relay: Option<&mut Relay>,
    ) -> (u64, u64) {
        for frame in 0..30_000u64 {
            let now = frame * 16;
            if let Some(relay) = relay.as_deref_mut() {
                relay.pump();
            }
            host.step_frame(now, script_input).expect("host frame");
            if let Some(relay) = relay.as_deref_mut() {
                relay.pump();
            }
            guest.step_frame(now, script_input).expect("guest frame");
            if host.finished(now) && guest.finished(now) {
                let last = |seat: &NetSeat| seat.game().authority().last().map(|point| point.hash);
                return (
                    last(host).expect("host ran"),
                    last(guest).expect("guest ran"),
                );
            }
            std::thread::sleep(Duration::from_micros(150));
        }
        panic!(
            "the pair never finished: host {} guest {}",
            host.receipt_line(),
            guest.receipt_line()
        );
    }

    fn pair(rules: NetRules, fault: Option<LinkProfile>) -> (NetSeat, NetSeat) {
        let mut host = NetSeat::host(config(), rules, loopback(), None).expect("host");
        let target = host.local_addr().expect("host addr");
        let mut guest = NetSeat::join(target, &host.invite().code()).expect("guest");
        if let Some(profile) = fault {
            host.set_fault(profile, 1);
            guest.set_fault(profile, 2);
        }
        (host, guest)
    }

    #[test]
    fn two_seats_end_on_one_hash_whatever_the_link_did() {
        let (mut host, mut guest) = pair(NetRules::Duel, None);
        let (clean, mirrored) = play_pair(&mut host, &mut guest, None);
        assert_eq!(clean, mirrored, "{}", guest.receipt_line());
        assert_eq!(guest.game().last_report(), None);
        assert_eq!(
            host.stats().ai_ticks,
            0,
            "the guest played every tick itself"
        );
        let rough = LinkProfile {
            latency: 2,
            jitter: 3,
            loss_pct: 10,
            reorder_pct: 10,
        };
        let (mut host, mut guest) = pair(NetRules::Duel, Some(rough));
        let (faulted, faulted_guest) = play_pair(&mut host, &mut guest, None);
        assert_eq!(faulted, faulted_guest, "{}", guest.receipt_line());
        assert_eq!(
            faulted, clean,
            "a late input stalls the host, it never changes the match"
        );
        let dropped = |seat: &NetSeat| seat.fault_stats().map_or(0, |link| link.dropped);
        assert!(
            dropped(&host) > 0 && dropped(&guest) > 0,
            "the link dropped both ways"
        );
        assert!(host.stats().stalls > 0, "the host waited on late inputs");
        assert_eq!(host.stats().ai_ticks, 0);
    }

    #[test]
    fn the_spear_rides_the_code_and_another_match_is_refused() {
        let (mut host, mut guest) = pair(NetRules::DuelSpear, None);
        assert_eq!(guest.game().rules(), NetRules::DuelSpear);
        let (spear, spear_guest) = play_pair(&mut host, &mut guest, None);
        assert_eq!(spear, spear_guest);
        let (mut host, mut guest) = pair(NetRules::Duel, None);
        let (plain, _) = play_pair(&mut host, &mut guest, None);
        assert_ne!(spear, plain, "the scripted throws change a spear match");
        // A guest whose code asks for the spear against a host without it.
        let mut host = NetSeat::host(config(), NetRules::Duel, loopback(), None).expect("host");
        let wrong = Invite {
            rules: NetRules::DuelSpear,
            ..host.invite()
        };
        let target = host.local_addr().expect("host addr");
        let mut guest = NetSeat::join(target, &wrong.code()).expect("guest");
        let mut host_notes = Vec::new();
        for frame in 0..2_000u64 {
            host_notes.extend(
                host.step_frame(frame * 16, script_input)
                    .expect("host frame"),
            );
            match guest.step_frame(frame * 16, script_input) {
                Ok(_) => {}
                Err(err) => {
                    assert!(matches!(err, GameError::SpecMismatch { .. }), "{err:?}");
                    assert!(host_notes
                        .iter()
                        .any(|note| matches!(note, SeatNote::Refused { .. })));
                    assert!(!host.connected(), "a refused guest never pairs");
                    return;
                }
            }
            std::thread::sleep(Duration::from_micros(150));
        }
        panic!("the guest was never refused");
    }

    #[test]
    fn the_relay_carries_the_same_match() {
        let (mut host, mut guest) = pair(NetRules::Duel, None);
        let (direct, _) = play_pair(&mut host, &mut guest, None);
        let mut relay = Relay::new();
        let mut host = NetSeat::host(config(), NetRules::Duel, loopback(), Some(relay.addr()))
            .expect("relay host");
        let code = host.invite().code();
        assert!(
            host.invite().relay_host.is_some(),
            "the code names the host's relay id: {code}"
        );
        let mut guest = NetSeat::join(relay.addr(), &code).expect("relay guest");
        let (hosted, joined) = play_pair(&mut host, &mut guest, Some(&mut relay));
        assert_eq!(
            (hosted, joined),
            (direct, direct),
            "{}",
            guest.receipt_line()
        );
        assert!(
            relay.hub.server().stats().forwarded > u64::from(TICKS),
            "every tick crossed the relay"
        );
    }

    #[test]
    fn the_lead_follows_the_round_trip_and_the_script_is_valid() {
        assert_eq!(lead_for_rtt(0), MIN_LEAD_TICKS);
        assert_eq!(lead_for_rtt(16), 1 + LEAD_MARGIN_TICKS);
        assert_eq!(lead_for_rtt(100), 6 + LEAD_MARGIN_TICKS);
        assert_eq!(lead_for_rtt(60_000), MAX_LEAD_TICKS);
        for tick in 0..600 {
            for slot in 0..2 {
                let input = script_input(tick, slot);
                assert!(input.validate().is_ok(), "{input:?}");
                assert_eq!((input.tick, input.fighter.0), (tick, slot));
            }
        }
    }
}
