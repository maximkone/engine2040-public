//! heal: disconnect/reconnect plus desync resync through the supervisor.
//!
//! Consumer: the dedicated-server operator. Data owner: [`NetHeal`] owns
//! the [`Supervisor`](engine_observe::supervisor::Supervisor), its
//! [`RingBuffer`], the per-slot quarantine table and the operator notices;
//! the [`HeadlessServer`](crate::dedicated::HeadlessServer) owns slots,
//! ticks and snapshots, and the [`Client`](crate::session::Client) owns
//! predictions. Units: ticks (`u64`, server ticks) and backoff (`u64`
//! milliseconds, a service sleep hint). Determinism scope: `BTreeMap` order,
//! saturating counters and tick-derived backoff from the shipped supervisor
//! catalog — no clock, no hash input. Errors: [`NetHealError`]; over-budget
//! faults quarantine the triggering slot loudly with a [`QuarantineNotice`].
//!
//! Routing: a dropped client reports `net.client.desync` through the
//! supervisor. While within budget the slot AI-fills (the server default)
//! and the [`ReconnectPlan`] carries the reconnect backoff; the same token
//! reclaims the same sticky slot on return and converges through the
//! authoritative snapshot. A hash mismatch reports the same code; the
//! deferred [`DesyncPlan`] applies at the next tick boundary and the client
//! resimulates from the authoritative snapshot
//! ([`Client::resync_from_authority`](crate::session::Client::resync_from_authority)).
//! Past the shared catalog budget (4 attempts) the slot quarantines: reclaim
//! and resync refuse with [`NetHealError::Quarantined`] until a verified
//! recovery is on file ([`NetHeal::verify_recovery`]) and
//! [`NetHeal::reset_slot`] releases the slot.

use std::collections::BTreeMap;

use engine_observe::heal::{HealError, HealKind};
use engine_observe::recovery::{HealObservation, HealValidation, ResetOutcome};
use engine_observe::supervisor::{AppliedHeal, HealTapeRecord, SupervisedDecision, Supervisor};
use engine_observe::{EventCode, RingBuffer};

use crate::dedicated::ms_to_ticks;

/// Fault code every net heal reports: the supervisor catalog heals it with
/// AI takeover, 4 attempts, 100–1600 ms backoff, one tick between retries.
const NET_FAULT: EventCode = EventCode::NET_CLIENT_DESYNC;

/// Heal event ring capacity: 128 rows of `heal.applied` / `heal.failed`.
/// The ring evicts oldest-first with a visible overflow count, never an error.
pub const HEAL_RING_CAP: usize = 128;

/// Maximum operator quarantine notices retained. Past this the oldest notice
/// is evicted and counted in [`NetHeal::notices_overflow`]: bounded, never
/// silent, never a heal refusal.
pub const MAX_NOTICES: usize = 64;

/// Heal failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetHealError {
    /// The slot spent its heal budget and is held by AI until the operator
    /// verifies recovery and resets it. Slot and attempt count carried.
    Quarantined {
        /// Slot held by AI.
        slot: u8,
        /// One-based attempt that spent the budget.
        attempts: u32,
    },
    /// Unknown, never-joined or already-dropped token. Token carried.
    StaleToken {
        /// Token the caller presented.
        token: u64,
    },
    /// No authoritative snapshot exists yet (the match has no ticks).
    NoSnapshot,
    /// Every unquarantined slot is connected; no slot to reclaim.
    NoSlot,
    /// The supervisor boundary queue or tape needs draining first.
    HealFull,
    /// The net fault has no supervisor policy row (unreachable with the
    /// shipped catalog; loud instead of silent).
    NotRegistered,
}

impl std::fmt::Display for NetHealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Quarantined { slot, attempts } => write!(
                f,
                "slot {slot} quarantined after {attempts} attempts; verify recovery and reset"
            ),
            Self::StaleToken { token } => write!(f, "stale client token {token}"),
            Self::NoSnapshot => write!(f, "no authoritative snapshot yet"),
            Self::NoSlot => write!(f, "no free slot"),
            Self::HealFull => write!(f, "heal queue or tape is full; drain first"),
            Self::NotRegistered => write!(f, "no heal registered for net.client.desync"),
        }
    }
}

impl std::error::Error for NetHealError {}

/// Map a supervisor refusal onto the typed net error. Called only on
/// [`Err`], so mapping consumes no budget — same as the supervisor.
///
/// The three proof refusals (`UnverifiedRecovery`, `ScopeMismatch`, `UnknownHeal`) all mean
/// the same thing to the net layer: the supervisor did not apply a row, so the fault is not
/// healed. They map to [`NetHealError::NotRegistered`], the net layer's "no policy row" word.
fn map_heal_error(error: HealError) -> NetHealError {
    match error {
        HealError::PendingFull | HealError::TapeFull => NetHealError::HealFull,
        HealError::NotRegistered(_)
        | HealError::UnknownCode(_)
        | HealError::UnverifiedRecovery { .. }
        | HealError::ScopeMismatch { .. }
        | HealError::UnknownHeal { .. } => NetHealError::NotRegistered,
    }
}

/// Operator notice for a quarantined slot: loud, bounded, replayable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuarantineNotice {
    /// Slot held by AI.
    pub slot: u8,
    /// Token that spent the budget.
    pub token: u64,
    /// Server tick of the fault that spent the budget.
    pub fault_tick: u64,
    /// One-based attempt that spent the budget.
    pub attempts: u32,
    /// Backoff advised with the quarantine, in milliseconds.
    pub backoff_ms: u64,
}

/// Supervised reconnect advice for one dropped client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReconnectPlan {
    /// Slot the token holds; AI fills it until reclaim.
    pub slot: u8,
    /// Dropped token.
    pub token: u64,
    /// One-based supervisor attempt.
    pub attempt: u32,
    /// Recovery the caller executes (AI takeover, or quarantine past budget).
    pub kind: HealKind,
    /// Backoff before the client retries, in milliseconds.
    pub backoff_ms: u64,
    /// Same backoff in whole ticks at 60 Hz (service sleep hint).
    pub wait_ticks: u64,
    /// Earliest server tick a reclaim may run.
    pub not_before_tick: u64,
    /// Boundary tick the AI takeover applies at (deferred sim heal).
    pub applies_at: u64,
    /// True when the budget is spent: the slot quarantined instead.
    pub quarantined: bool,
}

/// Supervised resync advice for one hash mismatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DesyncPlan {
    /// Slot the token holds.
    pub slot: u8,
    /// Token that reported the hash.
    pub token: u64,
    /// Authoritative tick the hashes disagree on.
    pub tick: u32,
    /// One-based supervisor attempt.
    pub attempt: u32,
    /// Recovery the caller executes (AI takeover, or quarantine past budget).
    pub kind: HealKind,
    /// Backoff before the next attempt, in milliseconds.
    pub backoff_ms: u64,
    /// Boundary tick the resync applies at (deferred sim heal): fetch the
    /// authoritative snapshot there and resimulate.
    pub applies_at: u64,
    /// True when the budget is spent: the slot quarantined instead.
    pub quarantined: bool,
}

/// Supervisor routing for net disconnects and desyncs: one shared
/// `net.client.desync` budget row, per-slot quarantine with notices.
#[derive(Debug, Clone)]
pub struct NetHeal {
    supervisor: Supervisor,
    ring: RingBuffer,
    quarantined: BTreeMap<u8, QuarantineNotice>,
    notices: Vec<QuarantineNotice>,
    notices_overflow: u64,
}

impl NetHeal {
    /// Healer over the shipped supervisor catalog with an empty ring.
    #[must_use]
    pub fn new() -> Self {
        Self {
            supervisor: Supervisor::new(),
            ring: RingBuffer::new(HEAL_RING_CAP),
            quarantined: BTreeMap::new(),
            notices: Vec::new(),
            notices_overflow: 0,
        }
    }

    /// Borrow the supervisor (budgets, tape, escalation).
    #[must_use]
    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    /// Borrow the heal event ring.
    #[must_use]
    pub fn ring(&self) -> &RingBuffer {
        &self.ring
    }

    /// Heal tape rows in report order.
    #[must_use]
    pub fn tape(&self) -> &[HealTapeRecord] {
        self.supervisor.tape()
    }

    /// Number of heal tape rows.
    #[must_use]
    pub fn tape_len(&self) -> usize {
        self.supervisor.tape_len()
    }

    /// Deferred heals waiting for a tick boundary.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.supervisor.pending_len()
    }

    /// Faults reported so far on the shared net row.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.supervisor.attempts(NET_FAULT)
    }

    /// True while `slot` is quarantined (held by AI, reclaim refused).
    #[must_use]
    pub fn is_quarantined_slot(&self, slot: u8) -> bool {
        self.quarantined.contains_key(&slot)
    }

    /// Quarantine notice for `slot`, if it quarantined.
    #[must_use]
    pub fn quarantine_of(&self, slot: u8) -> Option<QuarantineNotice> {
        self.quarantined.get(&slot).copied()
    }

    /// Operator quarantine notices in report order.
    #[must_use]
    pub fn notices(&self) -> &[QuarantineNotice] {
        &self.notices
    }

    /// Notices evicted past [`MAX_NOTICES`]. Monotonic; zero is healthy.
    #[must_use]
    pub fn notices_overflow(&self) -> u64 {
        self.notices_overflow
    }

    /// Route one disconnect: report the net fault and return the reconnect
    /// plan. The slot AI-fills from the next tick boundary; the client waits
    /// `backoff_ms` (about `wait_ticks`) and reclaims its sticky slot.
    /// Past the budget the plan quarantines and the slot stays AI-held with
    /// a notice. Refuses quarantined slots without consuming budget.
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::Quarantined`] when the slot already
    /// quarantined, [`NetHealError::HealFull`] when the supervisor queue or
    /// tape needs draining.
    pub fn on_disconnect(
        &mut self,
        slot: u8,
        token: u64,
        fault_tick: u64,
    ) -> Result<ReconnectPlan, NetHealError> {
        let decision = self.report(slot, token, fault_tick)?;
        let not_before = self.supervisor.escalation_of(NET_FAULT).map_or_else(
            || fault_tick.saturating_add(1),
            |state| state.not_before_tick,
        );
        Ok(ReconnectPlan {
            slot,
            token,
            attempt: decision.attempt,
            kind: decision.kind,
            backoff_ms: decision.backoff_ms,
            wait_ticks: ms_to_ticks(decision.backoff_ms),
            not_before_tick: not_before,
            applies_at: decision.applies_at,
            quarantined: decision.is_quarantine(),
        })
    }

    /// Route one hash mismatch: report the net fault and return the resync
    /// plan. The resync applies at `applies_at` (next tick boundary): fetch
    /// the authoritative snapshot there and resimulate. Past the budget the
    /// plan quarantines and the slot stays AI-held with a notice. Refuses
    /// quarantined slots without consuming budget.
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::Quarantined`] when the slot already
    /// quarantined, [`NetHealError::HealFull`] when the supervisor queue or
    /// tape needs draining.
    pub fn on_desync(
        &mut self,
        slot: u8,
        token: u64,
        tick: u32,
        fault_tick: u64,
    ) -> Result<DesyncPlan, NetHealError> {
        let decision = self.report(slot, token, fault_tick)?;
        Ok(DesyncPlan {
            slot,
            token,
            tick,
            attempt: decision.attempt,
            kind: decision.kind,
            backoff_ms: decision.backoff_ms,
            applies_at: decision.applies_at,
            quarantined: decision.is_quarantine(),
        })
    }

    /// Apply every deferred heal due at or before `boundary_tick`, oldest
    /// first. Returns the applied heals for the caller to execute (AI
    /// takeover and snapshot resync land here); empty when nothing is due.
    pub fn apply_tick_boundary(&mut self, boundary_tick: u64) -> Vec<AppliedHeal> {
        self.supervisor.apply_tick_boundary(boundary_tick)
    }

    /// Prove a resync: `last_good` is the authoritative state digest, `healed`
    /// the digest the client holds after applying it. Equal digests are a
    /// verified recovery and clear the shared budget; different digests are a
    /// failed attempt and spend it. `attempt` is the one-based attempt the
    /// plan carried.
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::NotRegistered`] when no tape row holds `attempt`.
    pub fn verify_recovery(
        &mut self,
        attempt: u32,
        last_good: u64,
        healed: u64,
    ) -> Result<HealValidation, NetHealError> {
        self.supervisor
            .verify_heal(
                NET_FAULT,
                attempt,
                &HealObservation::State {
                    before_digest: last_good,
                    after_digest: healed,
                },
                &mut self.ring,
            )
            .map_err(map_heal_error)
    }

    /// Clear the shared budget and the slot quarantine after a verified
    /// recovery (see [`Self::verify_recovery`]). The supervisor refuses the
    /// budget reset without a green proof on the tape: a refused reset leaves
    /// the slot quarantined. History stays: tape and notices are evidence,
    /// not budget. Returns true when the slot was quarantined and is released.
    pub fn reset_slot(&mut self, slot: u8) -> bool {
        if self.supervisor.reset(NET_FAULT) != ResetOutcome::Cleared {
            return false;
        }
        self.quarantined.remove(&slot).is_some()
    }

    /// Operator escape hatch: clear the shared budget and the slot quarantine
    /// with no proof on file. Loud: the supervisor logs the reset without
    /// proof in its events. Returns true when the slot was quarantined.
    pub fn reset_slot_unverified(&mut self, slot: u8) -> bool {
        self.supervisor.reset_unverified(NET_FAULT);
        self.quarantined.remove(&slot).is_some()
    }

    /// Shared report path: refuse quarantined slots, else report the net
    /// fault and quarantine loudly past the budget. Refusals and supervisor
    /// errors consume no attempt.
    fn report(
        &mut self,
        slot: u8,
        token: u64,
        fault_tick: u64,
    ) -> Result<SupervisedDecision, NetHealError> {
        if let Some(notice) = self.quarantined.get(&slot) {
            return Err(NetHealError::Quarantined {
                slot,
                attempts: notice.attempts,
            });
        }
        let decision = self
            .supervisor
            .report_fault(NET_FAULT, fault_tick, &mut self.ring)
            .map_err(map_heal_error)?;
        if decision.is_quarantine() {
            let notice = QuarantineNotice {
                slot,
                token,
                fault_tick,
                attempts: decision.attempt,
                backoff_ms: decision.backoff_ms,
            };
            self.quarantined.insert(slot, notice);
            self.push_notice(notice);
        }
        Ok(decision)
    }

    /// Retain one quarantine notice, bounded with a visible overflow count.
    fn push_notice(&mut self, notice: QuarantineNotice) {
        if self.notices.len() >= MAX_NOTICES {
            self.notices.remove(0);
            self.notices_overflow = self.notices_overflow.saturating_add(1);
        }
        self.notices.push(notice);
    }
}

impl Default for NetHeal {
    /// Healer over the shipped supervisor catalog with an empty ring.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedicated::HeadlessServer;
    use crate::session::{
        decode_input, decode_state, encode_input, encode_state, redundancy_window, send_packet,
        Client, SnapshotDelta, REDUNDANCY, WIRE_VERSION,
    };
    use engine_arena::{ArenaConfig, Checkpoint};
    use engine_input::fighter::Buttons;
    use engine_input::{FighterId, FighterInput};
    use std::net::{SocketAddr, UdpSocket};

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 64,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        }
    }

    fn wish(tick: u32, slot: u8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(slot).expect("slot"),
            mx: if tick % 20 < 10 { 100 } else { -100 },
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    /// Drain every waiting datagram on `sock` in send order.
    fn drain(sock: &UdpSocket) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = [0u8; crate::session::MAX_PACKET_BYTES];
        while let Ok((n, _)) = sock.recv_from(&mut buf) {
            out.push(buf[..n].to_vec());
        }
        out
    }

    /// One live loopback tick: client predicts, both directions exchange
    /// datagrams over real localhost UDP, the server authors, the client
    /// tracks truth and resimulates.
    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn live_tick(
        server_sock: &UdpSocket,
        client_sock: &UdpSocket,
        server_addr: SocketAddr,
        client_addr: SocketAddr,
        server: &mut HeadlessServer,
        client: &mut Client,
        seen: &mut Vec<FighterInput>,
        states_applied: &mut u32,
        step: u32,
        token: u64,
        predicted: &mut Vec<FighterInput>,
    ) {
        let input = wish(step, 0);
        client.predict(input).expect("predict");
        predicted.push(input);
        let window = redundancy_window(predicted, step, REDUNDANCY);
        let bytes = encode_input(step, &window).expect("encode input");
        send_packet(client_sock, server_addr, &bytes).expect("send");
        for bytes in drain(server_sock) {
            if let Ok(packet) = decode_input(&bytes) {
                for input in packet.inputs {
                    let _ = server.submit(token, input);
                }
            }
        }
        let (checkpoint, delta) = server.tick().expect("server tick");
        let window = redundancy_window(&delta.tape, checkpoint.tick, 8);
        let bytes = encode_state(checkpoint, &window).expect("encode state");
        send_packet(server_sock, client_addr, &bytes).expect("send");
        for bytes in drain(client_sock) {
            if let Ok(packet) = decode_state(&bytes) {
                if client
                    .server_tick()
                    .is_some_and(|done| packet.checkpoint.tick <= done)
                {
                    continue;
                }
                seen.extend_from_slice(&packet.inputs);
                seen.sort_by_key(|input| (input.tick, input.fighter.0));
                seen.dedup_by_key(|input| (input.tick, input.fighter.0));
                let delta = SnapshotDelta {
                    version: WIRE_VERSION,
                    tick: packet.checkpoint.tick,
                    hash: packet.checkpoint.hash,
                    tape: seen.clone(),
                };
                if client.on_server_state(packet.checkpoint, &delta).is_ok() {
                    *states_applied += 1;
                    client.resimulate();
                }
            }
        }
    }

    #[test]
    fn heal_units_errors_and_determinism() {
        // Units: 100 ms of backoff is 6 ticks at 60 Hz; 3 ticks are 50 ms.
        assert_eq!(ms_to_ticks(100), 6);
        assert_eq!(crate::dedicated::ticks_to_ms(3), 50);
        // Errors are typed with distinct loud messages.
        let messages = [
            NetHealError::Quarantined {
                slot: 1,
                attempts: 5,
            }
            .to_string(),
            NetHealError::StaleToken { token: 7 }.to_string(),
            NetHealError::NoSnapshot.to_string(),
            NetHealError::NoSlot.to_string(),
            NetHealError::HealFull.to_string(),
            NetHealError::NotRegistered.to_string(),
        ];
        for (index, message) in messages.iter().enumerate() {
            assert!(!message.is_empty(), "error {index} must name itself");
            for other in messages.iter().skip(index + 1) {
                assert_ne!(message, other, "errors must stay distinct");
            }
        }
        let _: &dyn std::error::Error = &NetHealError::HealFull;
        // Same fault sequence twice gives the same tape.
        fn sequence() -> Vec<HealTapeRecord> {
            let mut heal = NetHeal::new();
            for tick in 0..3_u64 {
                heal.on_disconnect(0, 11, tick).expect("within budget");
                let _ = heal.apply_tick_boundary(tick + 1);
            }
            heal.tape().to_vec()
        }
        assert_eq!(sequence(), sequence());
        // A fresh healer holds no budget, no quarantine, no notices.
        let heal = NetHeal::new();
        assert_eq!(heal.attempts(), 0);
        assert!(!heal.is_quarantined_slot(0));
        assert_eq!(heal.quarantine_of(0), None);
        assert!(heal.notices().is_empty());
        assert_eq!(heal.notices_overflow(), 0);
        assert_eq!(heal.tape_len(), 0);
        assert_eq!(heal.pending_len(), 0);
        assert_eq!(heal.ring().capacity(), HEAL_RING_CAP);
    }

    #[test]
    fn supervised_loopback_drop_reconnect_converges() {
        const DROP_AT: u32 = 20;
        const BACK_AT: u32 = 26;
        const END: u32 = 40;
        const TOKEN: u64 = 0xEA11;
        let cfg = config();
        let server_sock = crate::session::bind_loopback().expect("server socket");
        let client_sock = crate::session::bind_loopback().expect("client socket");
        server_sock.set_nonblocking(true).expect("nonblocking");
        client_sock.set_nonblocking(true).expect("nonblocking");
        let server_addr = server_sock.local_addr().expect("server addr");
        let client_addr = client_sock.local_addr().expect("client addr");

        let mut server = HeadlessServer::new(cfg).expect("server");
        let mut client = Client::new(cfg, FighterId::new(0).expect("slot 0")).expect("client");
        let home = server.reclaim_slot(TOKEN).expect("supervised join");
        assert_eq!(home, 0);
        let mut predicted: Vec<FighterInput> = Vec::new();
        let mut seen: Vec<FighterInput> = Vec::new();
        let mut states_applied = 0u32;

        // Phase 1: live play over real loopback UDP datagrams.
        for step in 0..DROP_AT {
            live_tick(
                &server_sock,
                &client_sock,
                server_addr,
                client_addr,
                &mut server,
                &mut client,
                &mut seen,
                &mut states_applied,
                step,
                TOKEN,
                &mut predicted,
            );
        }
        assert!(states_applied > 0, "client must track server truth");
        assert_eq!(client.checkpoints(), server.checkpoints());

        // Phase 2: the drop routes to the supervisor; AI holds the slot.
        let plan = server.disconnect_via_supervisor(TOKEN).expect("heal plan");
        assert_eq!(plan.slot, 0);
        assert_eq!(plan.token, TOKEN);
        assert_eq!(plan.attempt, 1);
        assert_eq!(plan.kind, HealKind::AiTakeover);
        assert_eq!(plan.backoff_ms, 100);
        assert_eq!(plan.wait_ticks, ms_to_ticks(100));
        assert_eq!(plan.not_before_tick, u64::from(DROP_AT) + 1);
        assert_eq!(plan.applies_at, u64::from(DROP_AT) + 1);
        assert!(!plan.quarantined);
        assert_eq!(server.connected_count(), 0);
        let mut peak_ai = 0usize;
        for _ in DROP_AT..BACK_AT {
            let (_, _) = server.tick().expect("ai tick");
            peak_ai = peak_ai.max(server.ai_filled_last_tick());
        }
        assert!(peak_ai >= 1, "dropped slots must AI-fill (peak {peak_ai})");
        // The advised wait (6 ticks) elapsed; the deferred takeover applied.
        let applied = server.apply_heal_boundary(u64::from(BACK_AT));
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].fault_tick, u64::from(DROP_AT));
        assert_eq!(applied[0].applied_tick, u64::from(BACK_AT));
        assert_eq!(applied[0].kind, HealKind::AiTakeover);

        // Phase 3: the same token reclaims its slot and resyncs to authority.
        let reclaimed = server.reclaim_slot(TOKEN).expect("reclaim");
        assert_eq!(reclaimed, home, "reconnect must reclaim its slot");
        let snapshot = server.resync_via_supervisor(TOKEN).expect("resync");
        assert_eq!(snapshot.tick, BACK_AT - 1);
        let mispredicts = client
            .resync_from_authority(&snapshot)
            .expect("resync applies");
        assert!(
            mispredicts.is_empty(),
            "shared prefix must agree after resync"
        );
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "rejoined client must converge to server truth"
        );
        // Verified recovery clears the budget; history stays as evidence.
        let healed = client.checkpoints().last().expect("tick").hash;
        let last_good = server.checkpoints().last().expect("tick").hash;
        assert_eq!(
            server.heal_verify_recovery(plan.attempt, last_good, healed),
            Ok(HealValidation::Recovered)
        );
        let tape_before = server.heal().tape_len();
        server.heal_reset_slot(home);
        assert_eq!(server.heal().attempts(), 0);
        assert_eq!(server.heal().tape_len(), tape_before);
        assert!(server.heal_notices().is_empty());

        // Phase 4: live play resumes on the reclaimed slot to the final tick.
        predicted.retain(|input| input.tick < BACK_AT);
        seen = snapshot.tape.clone();
        for step in BACK_AT..END {
            live_tick(
                &server_sock,
                &client_sock,
                server_addr,
                client_addr,
                &mut server,
                &mut client,
                &mut seen,
                &mut states_applied,
                step,
                TOKEN,
                &mut predicted,
            );
        }
        client.resimulate();
        assert_eq!(server.checkpoints().len(), END as usize);
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "supervised drop plus rejoin must converge end to end"
        );
        println!(
            "HEAL loopback drop_at={DROP_AT} back_at={BACK_AT} end={END} \
             backoff_ms=100 wait_ticks={} ai_peak={peak_ai} states_applied={states_applied} \
             hash={:016x}",
            ms_to_ticks(100),
            server.checkpoints().last().expect("checkpoint").hash,
        );
    }

    #[test]
    fn supervised_desync_resync_converges() {
        const TOKEN: u64 = 0xDE57C;
        const DIVERGED_TICK: u32 = 3;
        const END: u32 = 6;
        let cfg = config();
        let mut server = HeadlessServer::new(cfg).expect("server");
        let mut client = Client::new(cfg, FighterId::new(0).expect("slot 0")).expect("client");
        assert_eq!(server.reclaim_slot(TOKEN), Ok(0));

        // Six ticks where the client predicts one wish but the server
        // authors the mirrored wish at tick 3: guaranteed divergence.
        for tick in 0..END {
            let input = wish(tick, 0);
            client.predict(input).expect("predict");
            let mut authored = input;
            if tick == DIVERGED_TICK {
                authored.mx = -authored.mx;
            }
            server.submit(TOKEN, authored).expect("submit");
            server.tick().expect("tick");
        }
        let truth = server.checkpoints()[DIVERGED_TICK as usize];
        let predicted_hash = client.checkpoints()[DIVERGED_TICK as usize].hash;
        assert_ne!(
            predicted_hash, truth.hash,
            "mirrored wish must diverge from prediction"
        );
        assert_eq!(
            server.verify(TOKEN, DIVERGED_TICK, predicted_hash),
            Err(crate::dedicated::ServerError::Desync {
                tick: DIVERGED_TICK,
                want: truth.hash,
                got: predicted_hash,
            })
        );

        // The mismatch routes to the supervisor: deferred to the boundary.
        let plan = server
            .note_desync_via_supervisor(TOKEN, DIVERGED_TICK, predicted_hash)
            .expect("heal plan")
            .expect("hashes disagree");
        assert_eq!(plan.slot, 0);
        assert_eq!(plan.tick, DIVERGED_TICK);
        assert_eq!(plan.attempt, 1);
        assert_eq!(plan.kind, HealKind::AiTakeover);
        assert_eq!(plan.backoff_ms, 100);
        assert_eq!(plan.applies_at, u64::from(DIVERGED_TICK) + 1);
        assert!(!plan.quarantined);
        // An agreeing hash routes nothing and consumes no budget.
        assert_eq!(
            server
                .note_desync_via_supervisor(TOKEN, DIVERGED_TICK, truth.hash)
                .expect("agreement is not a fault"),
            None
        );
        assert_eq!(server.heal().attempts(), 1);

        // At the boundary the client resyncs from the authoritative
        // snapshot and names the divergent tick.
        let applied = server.apply_heal_boundary(plan.applies_at);
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].kind, HealKind::AiTakeover);
        let snapshot = server.resync_via_supervisor(TOKEN).expect("resync");
        assert_eq!(snapshot.tick, END - 1);
        assert_eq!(
            snapshot.hash,
            server.checkpoints().last().expect("tick").hash
        );
        let mispredicts = client
            .resync_from_authority(&snapshot)
            .expect("resync applies");
        assert!(
            mispredicts.contains(&DIVERGED_TICK),
            "divergence tick must be named, got {mispredicts:?}"
        );
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "desynced client must converge to server truth"
        );
        let healed = client.checkpoints().last().expect("tick").hash;
        assert_eq!(
            server.heal_verify_recovery(plan.attempt, snapshot.hash, healed),
            Ok(HealValidation::Recovered)
        );
        server.heal_reset_slot(0);
        assert_eq!(server.heal().attempts(), 0);
        println!(
            "HEAL desync tick={DIVERGED_TICK} backoff_ms=100 applies_at={} \
             mispredicts={mispredicts:?} hash={:016x}",
            plan.applies_at, snapshot.hash,
        );
    }

    #[test]
    fn over_budget_quarantines_slot_with_notice() {
        const TOKEN_A: u64 = 0xA11CE;
        const TOKEN_B: u64 = 0xB0B;
        let cfg = config();
        let mut server = HeadlessServer::new(cfg).expect("server");
        assert_eq!(server.reclaim_slot(TOKEN_A), Ok(0));
        assert_eq!(server.reclaim_slot(TOKEN_B), Ok(1));

        // Five drops without a verified reset: the shared budget holds 4.
        let mut backoffs = Vec::new();
        let mut quarantined = None;
        for _ in 0..5 {
            let plan = server.disconnect_via_supervisor(TOKEN_A).expect("plan");
            backoffs.push(plan.backoff_ms);
            if plan.quarantined {
                quarantined = Some(plan);
                break;
            }
            assert_eq!(plan.kind, HealKind::AiTakeover);
            server.tick().expect("ai tick");
            let _ = server.apply_heal_boundary(u64::from(server.tick_now()));
            assert_eq!(server.reclaim_slot(TOKEN_A), Ok(0));
        }
        let plan = quarantined.expect("fifth fault must quarantine");
        assert_eq!(backoffs, vec![100, 200, 400, 800, 1600]);
        assert_eq!(plan.attempt, 5);
        assert_eq!(plan.kind, HealKind::Quarantine);
        // Loud notice names the slot, the token and the spent budget.
        assert_eq!(server.heal_notices().len(), 1);
        let notice = server.heal_notices()[0];
        assert_eq!(
            notice,
            QuarantineNotice {
                slot: 0,
                token: TOKEN_A,
                fault_tick: plan.applies_at,
                attempts: 5,
                backoff_ms: 1600,
            }
        );
        assert!(server.is_slot_quarantined(0));
        assert!(!server.is_slot_quarantined(1));
        // The slot stays AI-held: reclaim, resync and desync notes refuse
        // without consuming more budget.
        assert_eq!(
            server.reclaim_slot(TOKEN_A),
            Err(NetHealError::Quarantined {
                slot: 0,
                attempts: 5
            })
        );
        assert_eq!(
            server.resync_via_supervisor(TOKEN_A),
            Err(NetHealError::Quarantined {
                slot: 0,
                attempts: 5
            })
        );
        assert_eq!(
            server.note_desync_via_supervisor(TOKEN_A, 0, 1),
            Err(NetHealError::Quarantined {
                slot: 0,
                attempts: 5
            })
        );
        assert_eq!(server.heal().attempts(), 5);
        // The other slot keeps playing: quarantine is per-slot.
        server
            .submit(TOKEN_B, wish(server.tick_now(), 1))
            .expect("slot 1 submits");
        server.tick().expect("match continues");
        assert_eq!(server.reclaim_slot(TOKEN_B), Ok(1));
        // Nothing converged for slot 0, so no proof is on file: the gated
        // reset refuses and the slot stays held with its budget spent.
        assert!(!server.heal_reset_slot(0));
        assert!(server.is_slot_quarantined(0));
        // The operator escape hatch releases it, loudly; history stays as evidence.
        assert!(server.heal_reset_slot_unverified(0));
        assert!(!server.is_slot_quarantined(0));
        assert_eq!(server.heal().attempts(), 0);
        assert_eq!(server.heal_notices().len(), 1);
        assert_eq!(server.reclaim_slot(TOKEN_A), Ok(0));
        assert!(!server.heal_reset_slot(0));
    }

    #[test]
    fn supervised_errors_stay_typed() {
        let cfg = config();
        let mut server = HeadlessServer::new(cfg).expect("server");
        // Unknown and zero tokens never touch the supervisor budget.
        assert_eq!(
            server.disconnect_via_supervisor(99),
            Err(NetHealError::StaleToken { token: 99 })
        );
        assert_eq!(
            server.reclaim_slot(0),
            Err(NetHealError::StaleToken { token: 0 })
        );
        assert_eq!(
            server.resync_via_supervisor(99),
            Err(NetHealError::StaleToken { token: 99 })
        );
        assert_eq!(
            server.note_desync_via_supervisor(99, 0, 0),
            Err(NetHealError::StaleToken { token: 99 })
        );
        assert_eq!(server.heal().attempts(), 0);
        assert!(server.heal().tape().is_empty());
        // A fresh match has no snapshot to resync from.
        server.reclaim_slot(7).expect("join");
        assert_eq!(
            server.resync_via_supervisor(7),
            Err(NetHealError::NoSnapshot)
        );
        // A dropped token goes stale until it reclaims.
        server.disconnect_via_supervisor(7).expect("drop");
        assert_eq!(
            server.resync_via_supervisor(7),
            Err(NetHealError::StaleToken { token: 7 })
        );
        // Full slots refuse with the typed error, not a panic.
        let mut full = HeadlessServer::new(cfg).expect("server");
        full.reclaim_slot(1).expect("slot 0");
        full.reclaim_slot(2).expect("slot 1");
        assert_eq!(full.reclaim_slot(3), Err(NetHealError::NoSlot));
        // Checkpoint and snapshot agree on the fault-free path.
        let checkpoint = Checkpoint {
            tick: 9,
            hash: 0xBEEF,
        };
        let delta = SnapshotDelta {
            version: WIRE_VERSION,
            tick: 9,
            hash: 0xBEEF,
            tape: Vec::new(),
        };
        let mut witness = Client::new(cfg, FighterId::new(0).expect("slot 0")).expect("client");
        assert_eq!(
            witness.resync_from_authority(&SnapshotDelta {
                version: WIRE_VERSION + 1,
                ..delta.clone()
            }),
            Err(crate::session::NetError::VersionMismatch {
                got: WIRE_VERSION + 1,
            })
        );
        assert!(witness
            .resync_from_authority(&delta)
            .expect("apply")
            .is_empty());
        assert_eq!(witness.server_tick(), Some(checkpoint.tick));
    }
}
