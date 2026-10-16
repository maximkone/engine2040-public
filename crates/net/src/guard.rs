//! guard: server-authority cheat checks over match inputs, movement and hits.
//!
//! Owner: net crate; the admission path feeds `session::Server` ticks, hit
//! claims run only through `lagcomp` adjudication, and every refusal carries
//! the tape position needed to rebuild the `replay` report. Units: fixed
//! ticks with integer floor-plane centimeters. Determinism: integer math and
//! stable order only, no wall clock; the same claims twice give identical
//! verdicts and digests. Errors: [`GuardError`] for setup, [`Refusal`] for
//! refused claims (logged, never applied). Budget: O(1) per call over
//! bounded pre-reserved logs; no heap growth once warm.

use engine_arena::{ArenaConfig, Checkpoint};
use engine_core::Fnv64;
use engine_input::{ParticipantError, ParticipantInput, MAX_PARTICIPANTS};

use crate::lagcomp::{HitClaim, HitReceipt, LagError, LagTape, LagWindow};
use crate::replay::{ReplayError, ReplayTape};
use crate::server::MAX_STEP_CM;

/// Ticks of authority time in one tumbling rate window.
pub const RATE_WINDOW_TICKS: u32 = 8;
/// Most inputs one fighter may place in a single rate window (one per tick).
pub const RATE_MAX_INPUTS: u8 = 8;
/// Input ticks past authority still admitted (late-packet slack).
pub const FUTURE_SLACK_TICKS: u32 = 2;
/// Inputs older than authority minus this refuse as stale.
pub const STALE_WINDOW_TICKS: u32 = 32;
/// Widest accepted movement gap in ticks; wider gaps must resync first.
pub const MAX_MOVE_GAP_TICKS: u32 = 8;
/// Teleport flag distance in centimeters (the float guard bound, in cm).
pub const TELEPORT_CM: i32 = 1200;
/// Cap on stored refusals; overflow still returns, counted in `dropped`.
pub const MAX_REFUSALS: usize = 256;
/// Cap on stored accepted inputs (the replay tape source).
pub const MAX_ACCEPTED: usize = 4096;
/// Per-fighter slot storage. The bound is a literal because array lengths
/// need a const expression; `open` and the pin test below tie it to the
/// input owner instead of re-deriving the cap.
const SLOTS: usize = 16;

/// Setup failures for [`ServerGuard::open`]: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuardError {
    /// Participant count outside `2..=MAX_PARTICIPANTS`, culprit carried.
    BadParticipants(u8),
}

impl core::fmt::Display for GuardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BadParticipants(n) => write!(f, "fighter count out of range: {n}"),
        }
    }
}

impl std::error::Error for GuardError {}

/// Why one claim was refused. Every variant logs; none applies state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefusalKind {
    /// Input failed the participant contract ranges, cause carried.
    BadRange(ParticipantError),
    /// Slot at or above the match fighter count, culprit carried.
    BadSlot(u8),
    /// Input tick past authority plus slack.
    FutureTick,
    /// Input tick older than the stale window or behind the last admission.
    StaleTick,
    /// Same `(tick, fighter)` admitted twice.
    DuplicateTick,
    /// More than [`RATE_MAX_INPUTS`] admissions inside one rate window.
    RateBurst,
    /// Accepted-input store already holds [`MAX_ACCEPTED`] entries.
    AcceptedFull,
    /// Step past the per-tick server bound but below teleport range.
    Speed,
    /// Step past [`TELEPORT_CM`].
    Teleport,
    /// Movement gap wider than [`MAX_MOVE_GAP_TICKS`].
    MoveGap,
    /// Hit verdict refused: outside the window or ahead of authority.
    HitOutsideWindow,
    /// Needed tick missing from authority history, tick carried.
    HitUnknownTick(u32),
    /// Hit receipt breaks stable tape order.
    HitOutOfOrder,
    /// Hit claim or window malformed, cause carried.
    HitBadClaim(LagError),
    /// Hit tape already holds its bound.
    TapeFull,
}

impl RefusalKind {
    /// Stable discriminant for digests.
    const fn id(self) -> u8 {
        match self {
            Self::BadRange(_) => 1,
            Self::BadSlot(_) => 2,
            Self::FutureTick => 3,
            Self::StaleTick => 4,
            Self::DuplicateTick => 5,
            Self::RateBurst => 6,
            Self::AcceptedFull => 7,
            Self::Speed => 8,
            Self::Teleport => 9,
            Self::MoveGap => 10,
            Self::HitOutsideWindow => 11,
            Self::HitUnknownTick(_) => 12,
            Self::HitOutOfOrder => 13,
            Self::HitBadClaim(_) => 14,
            Self::TapeFull => 15,
        }
    }

    /// Payload folded into digests (fixed-width, platform-independent).
    fn detail(self) -> u64 {
        match self {
            Self::BadRange(cause) => cause.range_detail(),
            Self::BadSlot(slot) => u64::from(slot),
            Self::HitUnknownTick(tick) => u64::from(tick),
            Self::HitBadClaim(cause) => cause.detail(),
            Self::FutureTick
            | Self::StaleTick
            | Self::DuplicateTick
            | Self::RateBurst
            | Self::AcceptedFull
            | Self::Speed
            | Self::Teleport
            | Self::MoveGap
            | Self::HitOutsideWindow
            | Self::HitOutOfOrder
            | Self::TapeFull => 0,
        }
    }
}

impl core::fmt::Display for RefusalKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BadRange(cause) => write!(f, "input out of range: {cause:?}"),
            Self::BadSlot(slot) => write!(f, "fighter slot out of range: {slot}"),
            Self::FutureTick => write!(f, "input tick ahead of authority"),
            Self::StaleTick => write!(f, "input tick stale"),
            Self::DuplicateTick => write!(f, "duplicate input for tick and fighter"),
            Self::RateBurst => write!(f, "input rate over window budget"),
            Self::AcceptedFull => write!(f, "accepted-input store full"),
            Self::Speed => write!(f, "movement past the per-tick step bound"),
            Self::Teleport => write!(f, "movement past teleport range"),
            Self::MoveGap => write!(f, "movement gap needs a resync"),
            Self::HitOutsideWindow => write!(f, "hit outside the compensation window"),
            Self::HitUnknownTick(tick) => write!(f, "hit needs unknown tick {tick}"),
            Self::HitOutOfOrder => write!(f, "hit breaks stable tape order"),
            Self::HitBadClaim(cause) => write!(f, "hit claim malformed: {cause}"),
            Self::TapeFull => write!(f, "hit tape full"),
        }
    }
}

/// One refused claim: where authority stood, what was claimed, and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refusal {
    /// Authority tick when the claim arrived.
    pub server_tick: u32,
    /// Claimed tick (input, movement, or hit view tick).
    pub tick: u32,
    /// Claiming fighter slot.
    pub fighter: u8,
    /// Why the claim was refused.
    pub kind: RefusalKind,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "tick {} fighter {} refused at authority {}: {}",
            self.tick, self.fighter, self.server_tick, self.kind
        )
    }
}

/// Server-authority cheat screen: input rate plus range checks, integer
/// movement sanity, hits only through lag compensation, and every refusal
/// logged beside the accepted tape that backs the replay report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerGuard {
    fighters: u8,
    authority: u32,
    window_base: u32,
    counts: [u8; SLOTS],
    last_admit: [u32; SLOTS],
    has_admit: [bool; SLOTS],
    last_pos: [(i32, i32); SLOTS],
    last_move: [u32; SLOTS],
    has_pos: [bool; SLOTS],
    hits: LagTape,
    refusals: Vec<Refusal>,
    accepted: Vec<ParticipantInput>,
    dropped: u64,
}

impl ServerGuard {
    /// Open the screen for `fighters` at `authority`.
    ///
    /// ```rust
    /// use engine_net::guard::ServerGuard;
    /// let guard = ServerGuard::open(2, 100).expect("two fighters");
    /// assert_eq!(guard.authority(), 100);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`GuardError::BadParticipants`] when `fighters` sits outside
    /// `2..=MAX_PARTICIPANTS`.
    pub fn open(fighters: u8, authority: u32) -> Result<Self, GuardError> {
        debug_assert_eq!(usize::from(MAX_PARTICIPANTS), SLOTS);
        if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
            return Err(GuardError::BadParticipants(fighters));
        }
        Ok(Self {
            fighters,
            authority,
            window_base: authority,
            counts: [0; SLOTS],
            last_admit: [0; SLOTS],
            has_admit: [false; SLOTS],
            last_pos: [(0, 0); SLOTS],
            last_move: [0; SLOTS],
            has_pos: [false; SLOTS],
            hits: match LagTape::new(fighters) {
                Ok(tape) => tape,
                Err(_) => return Err(GuardError::BadParticipants(fighters)),
            },
            refusals: Vec::with_capacity(MAX_REFUSALS),
            accepted: Vec::with_capacity(MAX_ACCEPTED),
            dropped: 0,
        })
    }

    /// Participant count this screen guards.
    pub fn fighters(&self) -> u8 {
        self.fighters
    }

    /// Current authority tick admissions are judged against.
    pub fn authority(&self) -> u32 {
        self.authority
    }

    /// Move authority forward (production: every authored tick). Crossing a
    /// rate-window boundary opens a fresh tumbling window.
    pub fn set_authority(&mut self, authority: u32) {
        self.authority = authority;
        if authority < self.window_base
            || authority.saturating_sub(self.window_base) >= RATE_WINDOW_TICKS
        {
            self.window_base = authority;
            self.counts = [0; SLOTS];
        }
    }

    /// Refusals in arrival order (bounded at [`MAX_REFUSALS`]).
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }

    /// Accepted inputs in admission order (the replay tape source).
    pub fn accepted_inputs(&self) -> &[ParticipantInput] {
        &self.accepted
    }

    /// Fair hit receipts in stable tape order.
    pub fn hit_receipts(&self) -> &[HitReceipt] {
        self.hits.receipts()
    }

    /// Refusals that arrived after the log filled (still returned, unstored).
    pub fn dropped_count(&self) -> u64 {
        self.dropped
    }

    /// Reserved room in the accepted-input store (steady-state proof hook).
    pub fn accepted_capacity(&self) -> usize {
        self.accepted.capacity()
    }

    /// Reserved room in the refusal log (steady-state proof hook).
    pub fn refusal_capacity(&self) -> usize {
        self.refusals.capacity()
    }

    /// Log one refusal; saturates with a counter once full so the bound holds.
    fn deny(&mut self, server_tick: u32, tick: u32, fighter: u8, kind: RefusalKind) -> Refusal {
        let refusal = Refusal {
            server_tick,
            tick,
            fighter,
            kind,
        };
        if self.refusals.len() < MAX_REFUSALS {
            self.refusals.push(refusal);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
        refusal
    }

    /// Admit one participant input: contract ranges, slot, timing, order, then
    /// the per-window rate budget. Accepted inputs join the replay source;
    /// anything else returns its logged [`Refusal`] and stores nothing.
    pub fn admit(&mut self, input: ParticipantInput) -> Result<ParticipantInput, Refusal> {
        let here = self.authority;
        if let Err(cause) = input.validate() {
            return Err(self.deny(
                here,
                input.tick,
                input.fighter.0,
                RefusalKind::BadRange(cause),
            ));
        }
        let slot = input.fighter.0;
        if slot >= self.fighters {
            return Err(self.deny(here, input.tick, slot, RefusalKind::BadSlot(slot)));
        }
        if input.tick > here.saturating_add(FUTURE_SLACK_TICKS) {
            return Err(self.deny(here, input.tick, slot, RefusalKind::FutureTick));
        }
        if input.tick.saturating_add(STALE_WINDOW_TICKS) < here {
            return Err(self.deny(here, input.tick, slot, RefusalKind::StaleTick));
        }
        let at = usize::from(slot);
        if self.has_admit[at] {
            if input.tick == self.last_admit[at] {
                return Err(self.deny(here, input.tick, slot, RefusalKind::DuplicateTick));
            }
            if input.tick < self.last_admit[at] {
                return Err(self.deny(here, input.tick, slot, RefusalKind::StaleTick));
            }
        }
        if self.accepted.len() >= MAX_ACCEPTED {
            return Err(self.deny(here, input.tick, slot, RefusalKind::AcceptedFull));
        }
        self.counts[at] = self.counts[at].saturating_add(1);
        if self.counts[at] > RATE_MAX_INPUTS {
            return Err(self.deny(here, input.tick, slot, RefusalKind::RateBurst));
        }
        self.accepted.push(input);
        self.last_admit[at] = input.tick;
        self.has_admit[at] = true;
        Ok(input)
    }

    /// Check one integer floor-plane position in centimeters against the last
    /// accepted one: monotonic ticks, the per-tick server step scaled by the
    /// gap, and the teleport flag. Refusals keep the old anchor so a cheat
    /// cannot launder a new origin.
    pub fn watch(&mut self, fighter: u8, pos_cm: (i32, i32), tick: u32) -> Result<(), Refusal> {
        let here = self.authority;
        if fighter >= self.fighters {
            return Err(self.deny(here, tick, fighter, RefusalKind::BadSlot(fighter)));
        }
        let at = usize::from(fighter);
        if !self.has_pos[at] {
            self.has_pos[at] = true;
            self.last_pos[at] = pos_cm;
            self.last_move[at] = tick;
            return Ok(());
        }
        if tick <= self.last_move[at] {
            return Err(self.deny(here, tick, fighter, RefusalKind::StaleTick));
        }
        let gap = tick.saturating_sub(self.last_move[at]);
        if gap > MAX_MOVE_GAP_TICKS {
            return Err(self.deny(here, tick, fighter, RefusalKind::MoveGap));
        }
        let dx = i64::from(pos_cm.0) - i64::from(self.last_pos[at].0);
        let dy = i64::from(pos_cm.1) - i64::from(self.last_pos[at].1);
        let dist2 = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
        let step = i64::from(MAX_STEP_CM).saturating_mul(i64::from(gap));
        let teleport2 = i64::from(TELEPORT_CM).saturating_mul(i64::from(TELEPORT_CM));
        if dist2 > teleport2 {
            return Err(self.deny(here, tick, fighter, RefusalKind::Teleport));
        }
        if dist2 > step.saturating_mul(step) {
            return Err(self.deny(here, tick, fighter, RefusalKind::Speed));
        }
        self.last_pos[at] = pos_cm;
        self.last_move[at] = tick;
        Ok(())
    }

    /// Judge one hit claim through lag compensation only: the claim must be
    /// well-formed, its compensated tick must exist in `history`, and the
    /// verdict must be fair. Anything else returns its logged [`Refusal`];
    /// fair receipts join the ordered hit tape.
    pub fn judge(
        &mut self,
        claim: HitClaim,
        window: LagWindow,
        history: &[Checkpoint],
    ) -> Result<HitReceipt, Refusal> {
        let here = self.authority;
        let receipt = match crate::lagcomp::adjudicate_at(claim, window, self.fighters, history) {
            Ok(receipt) => receipt,
            Err(LagError::UnknownTick(need)) => {
                return Err(self.deny(
                    here,
                    claim.server_tick,
                    claim.attacker,
                    RefusalKind::HitUnknownTick(need),
                ));
            }
            Err(cause) => {
                return Err(self.deny(
                    here,
                    claim.server_tick,
                    claim.attacker,
                    RefusalKind::HitBadClaim(cause),
                ));
            }
        };
        if !receipt.accepted {
            return Err(self.deny(
                here,
                claim.server_tick,
                claim.attacker,
                RefusalKind::HitOutsideWindow,
            ));
        }
        match self.hits.push_receipt(receipt) {
            Ok(()) => Ok(receipt),
            Err(LagError::UnorderedTape) => Err(self.deny(
                here,
                claim.server_tick,
                claim.attacker,
                RefusalKind::HitOutOfOrder,
            )),
            Err(_) => Err(self.deny(
                here,
                claim.server_tick,
                claim.attacker,
                RefusalKind::TapeFull,
            )),
        }
    }

    /// Rebuild the replay report source from accepted inputs: the same tape
    /// the theater and divergence checks run, so a refusal log always pairs
    /// with a tape that replays exactly.
    pub fn record_accepted(&self, config: ArenaConfig) -> Result<ReplayTape, ReplayError> {
        crate::replay::record_tape(config, &self.accepted)
    }

    /// Deterministic digest over accepted inputs (integer fields only).
    pub fn accepted_digest(&self) -> u64 {
        let mut fold = Fnv64::new();
        for input in &self.accepted {
            fold.write_u32(input.tick);
            fold.write_u8(input.fighter.0);
            fold.write(&input.mx.to_le_bytes());
            fold.write(&input.mz.to_le_bytes());
            fold.write(&input.facing.to_le_bytes());
            fold.write_u8(input.buttons.0);
        }
        fold.finish()
    }

    /// Deterministic digest over logged refusals in arrival order.
    pub fn refusal_digest(&self) -> u64 {
        let mut fold = Fnv64::new();
        for refusal in &self.refusals {
            fold.write_u32(refusal.server_tick);
            fold.write_u32(refusal.tick);
            fold.write_u8(refusal.fighter);
            fold.write_u8(refusal.kind.id());
            fold.write_u64(refusal.kind.detail());
        }
        fold.write_u64(self.dropped);
        fold.finish()
    }

    /// Deterministic digest over fair hit receipts (the lag tape owner).
    pub fn hit_digest(&self) -> u64 {
        self.hits.hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::ArenaConfig;
    use engine_input::fighter::Buttons;
    use engine_input::{ParticipantId, ParticipantInput};

    fn wish(tick: u32, slot: u8, mx: i8) -> ParticipantInput {
        ParticipantInput {
            tick,
            fighter: ParticipantId::new(slot).expect("slot under test cap"),
            mx,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    fn claim(server_tick: u32, view_tick: u32) -> HitClaim {
        HitClaim {
            attacker: 0,
            target: 1,
            server_tick,
            view_tick,
        }
    }

    fn history(last: u32) -> Vec<Checkpoint> {
        use engine_core::hash::GOLDEN_GAMMA;
        (0..=last)
            .map(|tick| Checkpoint {
                tick,
                hash: u64::from(tick).wrapping_mul(GOLDEN_GAMMA),
            })
            .collect()
    }

    #[test]
    fn slot_storage_pins_to_the_input_owner() {
        assert_eq!(usize::from(MAX_PARTICIPANTS), SLOTS);
        assert_eq!(ServerGuard::open(1, 0), Err(GuardError::BadParticipants(1)));
        assert_eq!(
            ServerGuard::open(MAX_PARTICIPANTS + 1, 0),
            Err(GuardError::BadParticipants(MAX_PARTICIPANTS + 1))
        );
        assert_eq!(
            GuardError::BadParticipants(9).to_string(),
            "fighter count out of range: 9"
        );
    }

    #[test]
    fn range_and_slot_cheats_refuse_and_store_nothing() {
        let mut guard = ServerGuard::open(2, 100).expect("screen");
        let mut wild = wish(100, 0, 0);
        wild.mx = 101;
        let refused = guard.admit(wild).expect_err("wish past range refuses");
        assert!(matches!(refused.kind, RefusalKind::BadRange(_)));
        let mut noisy = wish(100, 0, 0);
        noisy.buttons = Buttons(0xFF);
        assert!(matches!(
            guard.admit(noisy).expect_err("unknown buttons refuse").kind,
            RefusalKind::BadRange(_)
        ));
        let foreign = ParticipantInput {
            tick: 100,
            fighter: ParticipantId::new(5).expect("slot under cap"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(
            guard
                .admit(foreign)
                .expect_err("slot past match refuses")
                .kind,
            RefusalKind::BadSlot(5)
        );
        assert!(guard.accepted_inputs().is_empty());
        assert_eq!(guard.refusals().len(), 3);
        assert!(!guard.refusals().is_empty());
    }

    #[test]
    fn rate_future_stale_and_duplicate_cheats_refuse() {
        let mut guard = ServerGuard::open(2, 100).expect("screen");
        assert_eq!(
            guard
                .admit(wish(103, 0, 0))
                .expect_err("past slack refuses")
                .kind,
            RefusalKind::FutureTick
        );
        assert_eq!(
            guard
                .admit(wish(60, 0, 0))
                .expect_err("old tick refuses")
                .kind,
            RefusalKind::StaleTick
        );
        guard.admit(wish(99, 1, 0)).expect("first admission");
        assert_eq!(
            guard
                .admit(wish(99, 1, 0))
                .expect_err("resend refuses")
                .kind,
            RefusalKind::DuplicateTick
        );
        assert_eq!(
            guard
                .admit(wish(98, 1, 0))
                .expect_err("rewind refuses")
                .kind,
            RefusalKind::StaleTick
        );
        // Eight distinct ticks fill the window; the ninth floods it.
        let mut flood = ServerGuard::open(2, 100).expect("screen");
        for tick in 94..=101 {
            flood.admit(wish(tick, 0, 0)).expect("window fills");
        }
        assert_eq!(
            flood
                .admit(wish(102, 0, 0))
                .expect_err("flood refuses")
                .kind,
            RefusalKind::RateBurst
        );
        // The other slot keeps its own budget, and a fresh window resets it.
        flood.admit(wish(100, 1, 0)).expect("per-slot budget");
        flood.set_authority(108);
        flood.admit(wish(106, 0, 0)).expect("fresh window admits");
        assert!(flood.refusal_digest() != 0);
    }

    #[test]
    fn speed_and_teleport_cheats_refuse_without_moving_the_anchor() {
        let mut guard = ServerGuard::open(2, 0).expect("screen");
        assert!(guard.watch(9, (0, 0), 1).is_err());
        guard.watch(0, (0, 0), 10).expect("anchor");
        guard.watch(0, (10, 0), 11).expect("step inside bound");
        assert_eq!(
            guard
                .watch(0, (100, 0), 12)
                .expect_err("speed refuses")
                .kind,
            RefusalKind::Speed
        );
        // The refused lunge never became the anchor: a legal step from the
        // old anchor still passes.
        guard.watch(0, (26, 0), 12).expect("anchor never moved");
        assert_eq!(
            guard
                .watch(0, (5000, 0), 13)
                .expect_err("teleport refuses")
                .kind,
            RefusalKind::Teleport
        );
        guard
            .watch(0, (26, 0), 13)
            .expect("replay of the tick is fine");
        assert_eq!(
            guard
                .watch(0, (26, 0), 13)
                .expect_err("same tick refuses")
                .kind,
            RefusalKind::StaleTick
        );
        assert_eq!(
            guard.watch(0, (40, 0), 30).expect_err("gap refuses").kind,
            RefusalKind::MoveGap
        );
        // Exact bound passes; one centimeter past it does not.
        guard.watch(1, (0, 0), 40).expect("second anchor");
        guard.watch(1, (16, 0), 41).expect("exact bound passes");
        assert_eq!(
            guard
                .watch(1, (33, 0), 42)
                .expect_err("one past refuses")
                .kind,
            RefusalKind::Speed
        );
    }

    #[test]
    fn hits_run_through_lag_compensation_only() {
        use crate::lagcomp::LagWindow;
        let past = history(120);
        let mut guard = ServerGuard::open(2, 120).expect("screen");
        let fair = guard
            .judge(claim(40, 38), LagWindow::FULL, &past)
            .expect("inside-window hit lands");
        assert!(fair.accepted);
        assert_eq!(fair.compensated_tick, 38);
        assert_eq!(guard.hit_receipts().len(), 1);
        assert_eq!(
            guard
                .judge(claim(41, 30), LagWindow::FULL, &past)
                .expect_err("stale view refuses")
                .kind,
            RefusalKind::HitOutsideWindow
        );
        assert_eq!(
            guard
                .judge(claim(40, 41), LagWindow::FULL, &past)
                .expect_err("future view refuses")
                .kind,
            RefusalKind::HitOutsideWindow
        );
        assert_eq!(
            guard
                .judge(claim(200, 198), LagWindow::FULL, &past)
                .expect_err("unknown tick refuses")
                .kind,
            RefusalKind::HitUnknownTick(198)
        );
        let wild = HitClaim {
            attacker: 7,
            target: 1,
            server_tick: 40,
            view_tick: 40,
        };
        assert!(matches!(
            guard
                .judge(wild, LagWindow::FULL, &past)
                .expect_err("bad slot refuses")
                .kind,
            RefusalKind::HitBadClaim(_)
        ));
        guard
            .judge(claim(41, 40), LagWindow::FULL, &past)
            .expect("ordered claim lands");
        assert_eq!(
            guard
                .judge(claim(40, 39), LagWindow::FULL, &past)
                .expect_err("rewound claim refuses")
                .kind,
            RefusalKind::HitOutOfOrder
        );
        assert!(guard.hits.verify_replay());
        assert_ne!(guard.hit_digest(), LagTape::new(2).expect("tape").hash());
    }

    #[test]
    fn accepted_inputs_replay_and_match_authority() {
        use crate::session::Server;
        let config = ArenaConfig {
            seed: 2040,
            ticks: 8,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        };
        let mut guard = ServerGuard::open(2, 0).expect("screen");
        for tick in 0..8 {
            guard.set_authority(tick);
            for slot in 0..2 {
                let mx = if (tick + u32::from(slot)) % 2 == 0 {
                    100
                } else {
                    -100
                };
                guard.admit(wish(tick, slot, mx)).expect("scripted input");
            }
        }
        assert_eq!(guard.accepted_inputs().len(), 16);
        let tape = guard.record_accepted(config).expect("replay records");
        tape.check().expect("tape consistent");
        let mut server = Server::new(config).expect("authority");
        for tick in 0..8 {
            let slice: Vec<ParticipantInput> = guard
                .accepted_inputs()
                .iter()
                .copied()
                .filter(|input| input.tick == tick)
                .collect();
            let (point, _) = server.tick(&slice).expect("authority authors");
            assert_eq!(point, tape.checkpoints[tick as usize]);
        }
        let again = guard.record_accepted(config).expect("replay repeats");
        assert_eq!(tape.tape_hash(), again.tape_hash());
    }

    #[test]
    fn same_script_twice_gives_identical_digests() {
        use crate::lagcomp::LagWindow;
        let script = |guard: &mut ServerGuard, past: &[Checkpoint]| {
            guard.set_authority(100);
            guard.admit(wish(100, 0, 50)).expect("admit");
            guard.admit(wish(100, 1, -50)).expect("admit");
            guard.admit(wish(103, 0, 0)).expect_err("future");
            guard.watch(0, (0, 0), 10).expect("anchor");
            guard.watch(0, (5000, 0), 11).expect_err("teleport");
            guard
                .judge(claim(40, 38), LagWindow::FULL, past)
                .expect("fair");
            guard
                .judge(claim(41, 30), LagWindow::FULL, past)
                .expect_err("stale view");
        };
        let past = history(120);
        let mut first = ServerGuard::open(2, 0).expect("screen");
        let mut second = ServerGuard::open(2, 0).expect("screen");
        script(&mut first, &past);
        script(&mut second, &past);
        assert_eq!(first.accepted_digest(), second.accepted_digest());
        assert_eq!(first.refusal_digest(), second.refusal_digest());
        assert_eq!(first.hit_digest(), second.hit_digest());
        second.admit(wish(101, 0, 0)).expect("extra");
        assert_ne!(first.accepted_digest(), second.accepted_digest());
    }

    #[test]
    fn steady_state_grows_no_heap() {
        let mut guard = ServerGuard::open(2, 100).expect("screen");
        for tick in 96..100 {
            guard.admit(wish(tick, 0, 0)).expect("warm inputs");
        }
        guard.admit(wish(103, 0, 0)).expect_err("warm refusal");
        guard.watch(0, (0, 0), 10).expect("warm anchor");
        let (accepted_cap, refusal_cap) = (guard.accepted_capacity(), guard.refusal_capacity());
        for tick in 100..102 {
            guard.admit(wish(tick, 1, 0)).expect("steady inputs");
        }
        guard.admit(wish(104, 1, 0)).expect_err("steady refusal");
        guard.watch(0, (8, 0), 11).expect("steady move");
        assert_eq!(guard.accepted_capacity(), accepted_cap);
        assert_eq!(guard.refusal_capacity(), refusal_cap);
        assert_eq!(guard.accepted_inputs().len(), 6);
        assert_eq!(guard.refusals().len(), 2);
    }

    #[test]
    fn error_strings_name_the_fault() {
        let refusal = Refusal {
            server_tick: 100,
            tick: 103,
            fighter: 0,
            kind: RefusalKind::FutureTick,
        };
        assert_eq!(
            refusal.to_string(),
            "tick 103 fighter 0 refused at authority 100: input tick ahead of authority"
        );
        assert!(!RefusalKind::RateBurst.to_string().is_empty());
        assert!(!RefusalKind::Speed.to_string().is_empty());
        assert!(!RefusalKind::Teleport.to_string().is_empty());
        assert!(!RefusalKind::MoveGap.to_string().is_empty());
        assert!(!RefusalKind::StaleTick.to_string().is_empty());
        assert!(!RefusalKind::DuplicateTick.to_string().is_empty());
        assert!(!RefusalKind::AcceptedFull.to_string().is_empty());
        assert!(!RefusalKind::HitOutsideWindow.to_string().is_empty());
        assert!(!RefusalKind::HitOutOfOrder.to_string().is_empty());
        assert!(!RefusalKind::TapeFull.to_string().is_empty());
        assert!(!RefusalKind::HitUnknownTick(7).to_string().is_empty());
        assert!(!RefusalKind::BadSlot(7).to_string().is_empty());
    }
}
