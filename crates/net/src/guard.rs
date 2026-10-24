//! guard: input admission screen of the dedicated server.
//!
//! Owner: net crate. Caller: [`HeadlessServer::offer`](crate::dedicated::HeadlessServer::offer),
//! where every client input datagram enters the server. A datagram is
//! screened before any input reaches the match: the seat's packet rate on
//! this arrival tick, then the wire codec (malformed bytes, impossible
//! fields such as unknown button bits), then every input in it: its slot,
//! how far ahead of or behind authority it is, whether it repeats or
//! rewrites an input already admitted, and whether it arrived inside the
//! lag window ([`crate::lagcomp`]). Refusals never reach the match; they are
//! counted per slot by kind in a [`SlotReceipt`](crate::guard::SlotReceipt). Admitted inputs pass
//! [`sanitize`](crate::anticheat::sanitize) (a spear press in a match without the spear is masked) and
//! every flagged input is logged with its tick in a [`FlagLog`](crate::anticheat::FlagLog), so a replay
//! names it. Every attack press gets a lag receipt on the guard's
//! [`LagTape`](crate::lagcomp::LagTape).
//! Units: fixed ticks. Determinism: integer math, `BTreeMap` order and
//! arrival order, no clock; the same datagrams give the same receipts and
//! [`InputGuard::digest`](crate::guard::InputGuard::digest).
//! Errors: none; refusals are counts, never panics.
//! Budget: O(inputs per datagram x log n); memory is bounded by
//! [`MEMORY_TICKS`](crate::guard::MEMORY_TICKS) of admitted inputs, the flag log and the lag tape.

use std::collections::BTreeMap;
use std::fmt;

use engine_core::Fnv64;
use engine_input::fighter::{ATTACK, THROW_SPEAR};

use crate::anticheat::{sanitize, FlagLog, Flagged};
use crate::lagcomp::{adjudicate, HitClaim, LagTape, LagWindow};
use crate::session::{decode_input, BodyInput, InputPacket, NetError};

/// Inputs stamped further than this past the open tick are refused (flood
/// guard; matches the bout and duel hosts' inbox bound).
pub const MAX_AHEAD_TICKS: u32 = 64;
/// Admitted inputs are remembered this many ticks behind the open tick, so a
/// repeat is told from a rewrite; anything older is refused as stale.
pub const MEMORY_TICKS: u32 = 32;
/// Datagrams one slot may send on one arrival tick; past it they are
/// refused unread.
pub const RATE_MAX_PACKETS: u16 = 16;
/// Refusal kinds a [`SlotReceipt`] counts.
pub const REFUSAL_KINDS: usize = 8;

/// Why an input or a whole datagram was refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Refusal {
    /// The datagram does not decode: length, magic, version, kind or count.
    Malformed,
    /// The datagram decodes but a field is impossible (wish or facing out of
    /// range, unknown button bits, unsorted or repeated inputs).
    BadField,
    /// The input names a slot the sender does not hold.
    WrongSlot,
    /// The input is stamped more than [`MAX_AHEAD_TICKS`] past the open tick.
    Future,
    /// The input is older than the guard's memory: an old datagram replayed.
    Stale,
    /// The input arrived after its tick closed or outside the lag window.
    Late,
    /// The input differs from the one already admitted for its tick.
    Rewrite,
    /// The datagram came past [`RATE_MAX_PACKETS`] on one arrival tick.
    Rate,
}

impl Refusal {
    /// Every kind, in receipt order.
    pub const ALL: [Self; REFUSAL_KINDS] = [
        Self::Malformed,
        Self::BadField,
        Self::WrongSlot,
        Self::Future,
        Self::Stale,
        Self::Late,
        Self::Rewrite,
        Self::Rate,
    ];

    /// Position in [`SlotReceipt::refused`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Malformed => 0,
            Self::BadField => 1,
            Self::WrongSlot => 2,
            Self::Future => 3,
            Self::Stale => 4,
            Self::Late => 5,
            Self::Rewrite => 6,
            Self::Rate => 7,
        }
    }

    /// Short name for receipts and logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::BadField => "bad_field",
            Self::WrongSlot => "wrong_slot",
            Self::Future => "future",
            Self::Stale => "stale",
            Self::Late => "late",
            Self::Rewrite => "rewrite",
            Self::Rate => "rate",
        }
    }

    /// True for the kinds an honest client never produces; a late input is
    /// the network's doing, every other kind is a forged or abusive packet.
    #[must_use]
    pub const fn is_cheat(self) -> bool {
        !matches!(self, Self::Late)
    }
}

/// What the guard did with one slot's traffic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SlotReceipt {
    /// Match slot.
    pub slot: u8,
    /// Datagrams past the rate check.
    pub packets: u32,
    /// Inputs admitted for the match.
    pub accepted: u32,
    /// Identical copies of admitted inputs (the redundancy window), ignored.
    pub repeats: u32,
    /// Admitted inputs [`sanitize`] changed.
    pub sanitized: u32,
    /// Refusals by kind, indexed by [`Refusal::index`].
    pub refused: [u32; REFUSAL_KINDS],
    /// Largest one-way lag seen, in ticks: arrival tick minus the datagram's
    /// newest input tick.
    pub lag_max: u32,
    /// Round trip the transport reported, in ms (0 when none).
    pub rtt_ms: u64,
}

impl SlotReceipt {
    /// Refusals of `kind`.
    #[must_use]
    pub const fn count(&self, kind: Refusal) -> u32 {
        self.refused[kind.index()]
    }

    /// Refusals an honest client never produces ([`Refusal::is_cheat`]).
    #[must_use]
    pub fn cheats(&self) -> u32 {
        Refusal::ALL
            .iter()
            .filter(|kind| kind.is_cheat())
            .map(|kind| self.count(*kind))
            .sum()
    }

    /// This slot's lag window: the measured one-way lag or the reported
    /// round trip, whichever is wider; [`LagWindow::DEFAULT`] before either.
    #[must_use]
    pub fn window(&self) -> LagWindow {
        if self.packets == 0 && self.rtt_ms == 0 {
            return LagWindow::DEFAULT;
        }
        let measured = LagWindow::for_one_way_ticks(self.lag_max);
        let reported = LagWindow::for_rtt_ms(self.rtt_ms);
        if measured.ticks >= reported.ticks {
            measured
        } else {
            reported
        }
    }

    fn refuse(&mut self, kind: Refusal) {
        let count = &mut self.refused[kind.index()];
        *count = count.saturating_add(1);
    }

    fn fold(&self, fold: &mut Fnv64) {
        fold.write_u8(self.slot);
        for word in [
            self.packets,
            self.accepted,
            self.repeats,
            self.sanitized,
            self.lag_max,
        ] {
            fold.write_u32(word);
        }
        for count in self.refused {
            fold.write_u32(count);
        }
        fold.write_u64(self.rtt_ms);
    }
}

impl fmt::Display for SlotReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "slot {}: {} packets, {} accepted, {} repeats, {} sanitized, lag max {} ticks, window {} ticks, refused",
            self.slot,
            self.packets,
            self.accepted,
            self.repeats,
            self.sanitized,
            self.lag_max,
            self.window().ticks,
        )?;
        for kind in Refusal::ALL {
            write!(f, " {}={}", kind.name(), self.count(kind))?;
        }
        Ok(())
    }
}

/// True for a press the lag tape judges: an attack or a spear throw.
fn is_attack(input: &BodyInput) -> bool {
    input.buttons.0 & (ATTACK | THROW_SPEAR) != 0
}

/// The dedicated server's input screen: per-slot receipts, the admitted
/// inputs it remembers, the flag log and the lag tape.
#[derive(Clone, Debug)]
pub struct InputGuard {
    spear: bool,
    receipts: Vec<SlotReceipt>,
    rate: Vec<(u32, u16)>,
    admitted: BTreeMap<(u32, u8), BodyInput>,
    flagged: FlagLog,
    hits: LagTape,
}

impl InputGuard {
    /// A screen for `fighters` slots; `spear` says whether the match has the
    /// spear (the sanitize rule).
    #[must_use]
    pub fn new(fighters: u8, spear: bool) -> Self {
        Self {
            spear,
            receipts: (0..fighters)
                .map(|slot| SlotReceipt {
                    slot,
                    ..SlotReceipt::default()
                })
                .collect(),
            rate: vec![(0, 0); usize::from(fighters)],
            admitted: BTreeMap::new(),
            flagged: FlagLog::default(),
            hits: LagTape::new(),
        }
    }

    /// Per-slot receipts in slot order.
    #[must_use]
    pub fn receipts(&self) -> &[SlotReceipt] {
        &self.receipts
    }

    /// Admitted inputs [`sanitize`] changed, in admission order (bounded).
    #[must_use]
    pub fn flagged(&self) -> &[Flagged] {
        self.flagged.entries()
    }

    /// Lag receipts of every attack press, in arrival order.
    #[must_use]
    pub fn hits(&self) -> &LagTape {
        &self.hits
    }

    /// Record the round trip the transport measured for `slot`.
    pub fn set_rtt(&mut self, slot: u8, rtt_ms: u64) {
        if let Some(receipt) = self.receipts.get_mut(usize::from(slot)) {
            receipt.rtt_ms = rtt_ms;
        }
    }

    /// First half of screening one datagram from `slot` arriving on server
    /// clock tick `arrival` while `open_tick` is the next tick to author:
    /// the rate check, the wire codec, then the lag sample (skipped for a
    /// datagram older than the guard's memory, so a replayed old datagram
    /// cannot widen the window; a fresh one widens it at most to the cap).
    /// `None` when the datagram is refused whole.
    pub fn open(
        &mut self,
        slot: u8,
        open_tick: u32,
        arrival: u32,
        bytes: &[u8],
    ) -> Option<InputPacket> {
        let at = usize::from(slot);
        let (rate, receipt) = (self.rate.get_mut(at)?, self.receipts.get_mut(at)?);
        *rate = if rate.0 == arrival {
            (arrival, rate.1.saturating_add(1))
        } else {
            (arrival, 1)
        };
        if rate.1 > RATE_MAX_PACKETS {
            receipt.refuse(Refusal::Rate);
            return None;
        }
        receipt.packets = receipt.packets.saturating_add(1);
        let packet = match decode_input(bytes) {
            Ok(packet) => packet,
            Err(NetError::BadInput) => {
                receipt.refuse(Refusal::BadField);
                return None;
            }
            Err(_) => {
                receipt.refuse(Refusal::Malformed);
                return None;
            }
        };
        if packet.tick.saturating_add(MEMORY_TICKS) >= open_tick {
            receipt.lag_max = receipt.lag_max.max(arrival.saturating_sub(packet.tick));
        }
        Some(packet)
    }

    /// Second half: judge every input of an opened datagram from `slot`.
    /// `open_tick` is the next tick the server authors, `hold` the window
    /// it holds ticks open for. Admitted inputs, sanitized, go to `out`.
    pub fn admit(
        &mut self,
        slot: u8,
        open_tick: u32,
        arrival: u32,
        hold: LagWindow,
        packet: &InputPacket,
        out: &mut Vec<BodyInput>,
    ) {
        let Some(receipt) = self.receipts.get_mut(usize::from(slot)) else {
            return;
        };
        for &raw in &packet.inputs {
            if raw.fighter.0 != slot {
                receipt.refuse(Refusal::WrongSlot);
                continue;
            }
            if raw.tick > open_tick.saturating_add(MAX_AHEAD_TICKS) {
                receipt.refuse(Refusal::Future);
                continue;
            }
            if raw.tick.saturating_add(MEMORY_TICKS) < open_tick {
                receipt.refuse(Refusal::Stale);
                continue;
            }
            let (input, flags) = sanitize(raw, self.spear);
            match self.admitted.get(&(input.tick, slot)) {
                Some(kept) if *kept == input => {
                    receipt.repeats = receipt.repeats.saturating_add(1);
                    continue;
                }
                Some(_) => {
                    receipt.refuse(Refusal::Rewrite);
                    continue;
                }
                None => {}
            }
            let claim = HitClaim {
                attacker: slot,
                view_tick: input.tick,
                arrival_tick: arrival,
                open_tick,
            };
            let verdict = if is_attack(&input) {
                self.hits.record(claim, hold)
            } else {
                adjudicate(claim, hold)
            };
            if !verdict.is_ok_and(|receipt| receipt.accepted) {
                receipt.refuse(Refusal::Late);
                continue;
            }
            self.admitted.insert((input.tick, slot), input);
            receipt.accepted = receipt.accepted.saturating_add(1);
            if flags != 0 {
                receipt.sanitized = receipt.sanitized.saturating_add(1);
                self.flagged.note(Flagged {
                    tick: input.tick,
                    slot,
                    flags,
                });
            }
            out.push(input);
        }
    }

    /// Forget admitted inputs more than [`MEMORY_TICKS`] behind `open_tick`
    /// (called once per authored tick; no allocation).
    pub fn forget_before(&mut self, open_tick: u32) {
        let floor = open_tick.saturating_sub(MEMORY_TICKS);
        self.admitted.retain(|(tick, _), _| *tick >= floor);
    }

    /// Deterministic digest of every receipt, the flag log and the lag tape.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut fold = Fnv64::new();
        for receipt in &self.receipts {
            receipt.fold(&mut fold);
        }
        self.flagged.fold(&mut fold);
        fold.write_u64(self.hits.hash());
        fold.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anticheat::F_SPEAR;
    use crate::session::encode_input;
    use crate::session::BodyId;
    use engine_input::fighter::{Buttons, BLOCK, DODGE};

    fn press(tick: u32, slot: u8, buttons: u8) -> BodyInput {
        BodyInput {
            tick,
            fighter: BodyId::new(slot).expect("slot"),
            mx: 40,
            mz: -20,
            facing: 900,
            buttons: Buttons(buttons),
        }
    }

    fn packet(inputs: &[BodyInput]) -> Vec<u8> {
        let newest = inputs.iter().map(|input| input.tick).max().expect("inputs");
        encode_input(newest, inputs).expect("encode")
    }

    /// Screen one datagram the way the server does.
    fn offer(
        guard: &mut InputGuard,
        slot: u8,
        open: u32,
        arrival: u32,
        bytes: &[u8],
    ) -> Vec<BodyInput> {
        let mut out = Vec::new();
        if let Some(packet) = guard.open(slot, open, arrival, bytes) {
            let hold = guard.receipts()[usize::from(slot)].window();
            guard.admit(slot, open, arrival, hold, &packet, &mut out);
        }
        out
    }

    #[test]
    fn honest_redundancy_is_admitted_once_and_repeats_are_ignored() {
        let mut guard = InputGuard::new(2, false);
        let a = press(10, 0, 0);
        let b = press(11, 0, ATTACK);
        assert_eq!(offer(&mut guard, 0, 10, 11, &packet(&[a])), vec![a]);
        assert_eq!(offer(&mut guard, 0, 10, 12, &packet(&[a, b])), vec![b]);
        let receipt = guard.receipts()[0];
        assert_eq!((receipt.accepted, receipt.repeats), (2, 1));
        assert_eq!(receipt.cheats(), 0);
        assert_eq!(receipt.lag_max, 1);
        // The attack press got a fair lag receipt naming its window.
        let hits = guard.hits().receipts();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].accepted);
        assert_eq!(hits[0].window_ticks, 2);
    }

    #[test]
    fn every_cheat_kind_is_refused_counted_and_never_admitted() {
        let mut guard = InputGuard::new(2, false);
        let honest = press(20, 0, 0);
        assert_eq!(offer(&mut guard, 0, 20, 20, &packet(&[honest])).len(), 1);
        // Malformed: truncated bytes and a foreign magic.
        assert!(offer(&mut guard, 0, 20, 20, &[1, 2, 3]).is_empty());
        let mut foreign = packet(&[press(21, 0, 0)]);
        foreign[0] ^= 0xFF;
        assert!(offer(&mut guard, 0, 20, 20, &foreign).is_empty());
        // Impossible field: an unknown button bit (0x08) on the wire.
        let mut bad = packet(&[press(21, 0, 0)]);
        let last = bad.len() - 1;
        bad[last] = 0x08;
        assert!(offer(&mut guard, 0, 20, 20, &bad).is_empty());
        // Wrong slot, far future, an old replay, a rewrite of the admitted tick.
        let wrong = packet(&[press(21, 1, ATTACK)]);
        assert!(offer(&mut guard, 0, 20, 20, &wrong).is_empty());
        let future = packet(&[press(20 + MAX_AHEAD_TICKS + 1, 0, 0)]);
        assert!(offer(&mut guard, 0, 20, 20, &future).is_empty());
        let replay = packet(&[press(20, 0, ATTACK)]);
        assert!(offer(&mut guard, 0, 60, 60, &replay).is_empty());
        // The replayed old datagram did not widen the window.
        assert_eq!(guard.receipts()[0].lag_max, 0);
        assert!(offer(&mut guard, 0, 20, 20, &replay).is_empty());
        // A burst past the per-tick rate is refused unread.
        let fresh = packet(&[press(22, 0, 0)]);
        for _ in 0..RATE_MAX_PACKETS {
            let _ = offer(&mut guard, 0, 20, 21, &fresh);
        }
        assert!(offer(&mut guard, 0, 20, 21, &packet(&[press(23, 0, 0)])).is_empty());
        let receipt = guard.receipts()[0];
        for (kind, want) in [
            (Refusal::Malformed, 2),
            (Refusal::BadField, 1),
            (Refusal::WrongSlot, 1),
            (Refusal::Future, 1),
            (Refusal::Stale, 1),
            (Refusal::Rewrite, 1),
            (Refusal::Rate, 1),
            (Refusal::Late, 0),
        ] {
            assert_eq!(receipt.count(kind), want, "{}", kind.name());
        }
        assert_eq!(receipt.cheats(), 8);
        // Only the honest input and the one fresh tick reached the match.
        assert_eq!(receipt.accepted, 2);
        assert_eq!(
            guard.receipts()[1],
            SlotReceipt {
                slot: 1,
                ..SlotReceipt::default()
            }
        );
        assert!(receipt.to_string().contains("rewrite=1"));
    }

    #[test]
    fn a_press_after_its_tick_closed_is_late_and_its_hit_is_refused() {
        let mut guard = InputGuard::new(2, false);
        // Tick 30 is already authored (open tick 31) when the press arrives.
        assert!(offer(&mut guard, 1, 31, 32, &packet(&[press(30, 1, ATTACK)])).is_empty());
        let receipt = guard.receipts()[1];
        assert_eq!(receipt.count(Refusal::Late), 1);
        assert_eq!(receipt.cheats(), 0);
        let hits = guard.hits();
        assert_eq!(hits.counts(), (0, 1));
        assert!(hits.verify_replay());
    }

    #[test]
    fn sanitize_flags_name_their_ticks_the_same_twice() {
        let tape = |spear: bool| {
            let mut guard = InputGuard::new(2, spear);
            for tick in 0..12 {
                let buttons = match tick % 4 {
                    0 => THROW_SPEAR,
                    1 => ATTACK | THROW_SPEAR,
                    2 => BLOCK | DODGE,
                    _ => 0,
                };
                let got = offer(
                    &mut guard,
                    0,
                    tick,
                    tick,
                    &packet(&[press(tick, 0, buttons)]),
                );
                assert_eq!(got.len(), 1);
                assert_eq!(
                    got[0].buttons.0 & THROW_SPEAR != 0,
                    spear && buttons & THROW_SPEAR != 0
                );
            }
            guard
        };
        let (first, second) = (tape(false), tape(false));
        let ticks: Vec<u32> = first.flagged().iter().map(|flag| flag.tick).collect();
        assert_eq!(ticks, vec![0, 1, 4, 5, 8, 9]);
        assert!(first
            .flagged()
            .iter()
            .all(|flag| flag.flags == F_SPEAR && flag.slot == 0));
        assert_eq!(first.receipts()[0].sanitized, 6);
        assert_eq!(first.flagged(), second.flagged());
        assert_eq!(first.digest(), second.digest());
        // With the spear on nothing is masked.
        let spear = tape(true);
        assert!(spear.flagged().is_empty());
        assert_ne!(spear.digest(), first.digest());
        // A masked spear press is not an attack; an attack with it still is.
        assert_eq!(first.hits().receipts().len(), 3);
        assert_eq!(spear.hits().receipts().len(), 6);
    }

    #[test]
    fn memory_is_bounded_behind_the_open_tick() {
        let mut guard = InputGuard::new(2, false);
        for tick in 0..100 {
            let _ = offer(&mut guard, 0, tick, tick, &packet(&[press(tick, 0, 0)]));
            guard.forget_before(tick + 1);
        }
        assert!(guard.admitted.len() <= MEMORY_TICKS as usize + 1);
        assert_eq!(guard.receipts()[0].accepted, 100);
    }

    #[test]
    fn window_grows_with_measured_lag_and_reported_rtt() {
        let mut guard = InputGuard::new(2, false);
        assert_eq!(guard.receipts()[0].window(), LagWindow::DEFAULT);
        let _ = offer(&mut guard, 0, 10, 13, &packet(&[press(10, 0, 0)]));
        assert_eq!(guard.receipts()[0].window().ticks, 4);
        guard.set_rtt(0, 300);
        assert_eq!(guard.receipts()[0].window().ticks, 10);
        let _ = offer(&mut guard, 0, 11, 1_000, &packet(&[press(11, 0, 0)]));
        assert_eq!(guard.receipts()[0].window().ticks, 16);
    }
}
