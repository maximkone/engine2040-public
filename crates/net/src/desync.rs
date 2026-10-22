//! desync: a report that names the guilty side of a divergent tick.
//!
//! Owner: `engine-net`; consumer: [`crate::rollback::P2pSession::check_desync`],
//! which folds one of these into [`crate::rollback::RollbackError::Desync`]
//! when a peer hash refuses, so any caller or the door can read the verdict
//! instead of "hashes differ". Report-only: nothing here changes a tick, a
//! tape, a wire byte or the golden hash.
//!
//! The method and the struct shape follow the desync report of
//! `renew-engine/renew` (`crates/net/src/desync.rs`, Apache-2.0, read
//! 2026-10-22): the report carries the tick, the local peer id, per-peer
//! state digests, per-peer input digests and a last-agreed watermark, and
//! one bit — [`DesyncReport::inputs_agree`] — answers the first question a
//! developer asks: *whose fault is it*. Reimplemented std-only in our own
//! words, ideas only, no vendor bytes, no new dependency.
//!
//! Why the bit matters: when every input of the divergent tick was settled
//! (true, confirmed bytes on both seats, no repeat-last prediction still
//! standing) and the state digests still differ, the network is exonerated
//! and the bug lives in the simulation, the toolchain, the content or the
//! rules. When inputs do not agree — or the tick was never witnessed — the
//! report refuses to name the sim: an unreported tick is not agreement.
//!
//! No wire change: every field is built from data the local session already
//! holds. The remote input digest is folded here from the confirmed bytes
//! this side actually played; it is this side's witness of the remote
//! input, not the remote's own fold. Stated limits: the report cannot see
//! a byte corrupted in flight (that is the transport's packet-check job)
//! and it cannot prove what the remote holds until the remote reports its
//! own side. What it rules out is the prediction-and-missing-input
//! explanation, the fault a rollback tape can actually be lying about.
//!
//! Units: frames (`u32`), digests (`u64`), peer ids (`u8`, the tape seat).
//! Determinism: integer-only folds through the shared [`Fnv64`] owner.
//! Errors: none — a report is data; construction cannot fail and nothing
//! here panics.

use engine_core::Fnv64;

use crate::rollback::PLAYERS;

/// Peer id of this side on the two-seat tape.
pub const PEER_LOCAL: u8 = 0;
/// Peer id of the other seat on the two-seat tape.
pub const PEER_REMOTE: u8 = 1;

/// Everything the local session knew when a peer hash refused at `tick`.
///
/// The state and input arrays are indexed by peer id. The local slot stays
/// `None`: this side reports for its peers and carries its own digests in
/// the `local_state` / `local_inputs` scalars (the donor shape). A remote
/// slot is `None` when that side never reported the tick, which is never
/// read as agreement ([`Self::witnessed`], [`Self::inputs_agree`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesyncReport {
    /// The frame at which the two sides' state digests disagreed.
    pub tick: u32,
    /// Peer id this report speaks for (the local seat, [`PEER_LOCAL`]).
    pub local_peer: u8,
    /// This side's tape hash at `tick`.
    pub local_state: u64,
    /// This side's input digest at `tick` (the fold of the input row it
    /// played, [`Self::digest_inputs`]); `None` when the row dropped off
    /// the tape and cannot be re-folded.
    pub local_inputs: Option<u64>,
    /// Per-peer state digests at `tick`, indexed by peer id; `None` where
    /// that peer never reported the tick.
    pub peer_states: [Option<u64>; PLAYERS],
    /// Per-peer input digests at `tick`, indexed by peer id; `None` where
    /// that peer never reported the tick or its input was still a standing
    /// prediction when the divergence was caught. See the module doc: this
    /// side folds the remote slot from the confirmed bytes it holds.
    pub peer_inputs: [Option<u64>; PLAYERS],
    /// The last frame below `tick` on which a peer probe agreed; `None`
    /// when nothing below it was ever agreed. The divergence sits inside
    /// `(last_agreed_tick, tick]`, which is what makes the hunt cheap.
    pub last_agreed_tick: Option<u32>,
}

impl DesyncReport {
    /// Fold one input row into its digest: the `u32` words in seat order
    /// through the shared [`Fnv64`] owner. The same row folds to the same
    /// digest on every machine, so two peers can compare inputs without
    /// shipping the row itself.
    pub(crate) fn digest_inputs(row: &[u32; PLAYERS]) -> u64 {
        let mut fold = Fnv64::new();
        for input in row {
            fold.write_u32(*input);
        }
        fold.finish()
    }

    /// Build the report for one refused probe from data the session holds.
    /// `remote_inputs` is `Some` only when the remote input for the tick is
    /// settled (its true bytes arrived and were confirmed); a standing
    /// repeat-last prediction folds no digest and keeps the wire un-
    /// exonerated.
    pub(crate) fn from_probe(
        tick: u32,
        local_state: u64,
        peer_state: u64,
        local_inputs: Option<u64>,
        remote_inputs: Option<u64>,
        last_agreed_tick: Option<u32>,
    ) -> Self {
        Self {
            tick,
            local_peer: PEER_LOCAL,
            local_state,
            local_inputs,
            peer_states: [None, Some(peer_state)],
            peer_inputs: [None, remote_inputs],
            last_agreed_tick,
        }
    }

    /// True when at least one remote peer reported a digest for this tick.
    ///
    /// A report about a tick nobody witnessed compares nothing, and must
    /// never read as agreement.
    #[must_use]
    pub fn witnessed(&self) -> bool {
        let local = usize::from(self.local_peer);
        (0..PLAYERS).any(|peer| {
            peer != local && (self.peer_states[peer].is_some() || self.peer_inputs[peer].is_some())
        })
    }

    /// The one bit: true only when the tick was witnessed, this side folds
    /// its own input row, and every remote peer reported an input digest
    /// equal to it.
    ///
    /// `true` — both sides ran the same settled inputs and still reached
    /// different state, so the network is exonerated by evidence and the
    /// bug is in the simulation, the toolchain, the content or the rules.
    /// `false` — inputs differ or a side never reported an input digest,
    /// and the state digest is a symptom, not the disease; the wire (or a
    /// prediction still standing) stays a suspect. An unreported slot is
    /// never counted as agreement.
    #[must_use]
    pub fn inputs_agree(&self) -> bool {
        let Some(mine) = self.local_inputs else {
            return false;
        };
        if !self.witnessed() {
            return false;
        }
        let local = usize::from(self.local_peer);
        self.peer_inputs
            .iter()
            .enumerate()
            .filter(|(peer, _)| *peer != local)
            .all(|(_, reported)| *reported == Some(mine))
    }

    /// Which side the digests name for this tick.
    #[must_use]
    pub fn verdict(&self) -> DesyncVerdict {
        if !self.witnessed() {
            return DesyncVerdict::Unwitnessed;
        }
        if self.inputs_agree() {
            return DesyncVerdict::SimulationSide;
        }
        let local = usize::from(self.local_peer);
        let differ = match self.local_inputs {
            Some(mine) => self.peer_inputs.iter().enumerate().any(|(peer, reported)| {
                peer != local && reported.is_some() && *reported != Some(mine)
            }),
            None => false,
        };
        if differ {
            DesyncVerdict::InputsDiffer
        } else {
            DesyncVerdict::InputsUnsettled
        }
    }

    /// The peer id this report speaks against (the other tape seat).
    fn other_peer(&self) -> usize {
        usize::from(if self.local_peer == PEER_LOCAL {
            PEER_REMOTE
        } else {
            PEER_LOCAL
        })
    }
}

/// Which side the digests of a [`DesyncReport`] name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesyncVerdict {
    /// Every reported input digest matched: both sides ran settled, equal
    /// inputs and still reached different state. The network is exonerated
    /// (within the scope of the module doc); the bug is in the simulation,
    /// the toolchain, the content or the rules.
    SimulationSide,
    /// A peer reported an input digest different from this side's: the two
    /// tapes did not feed the same inputs to the same frame.
    InputsDiffer,
    /// The tick was witnessed but a side's input digest is missing (a
    /// prediction still standing, or a row off the tape): nothing exonerates
    /// the wire yet.
    InputsUnsettled,
    /// Nobody reported this tick at all: nothing to compare, never
    /// agreement.
    Unwitnessed,
}

impl std::fmt::Display for DesyncReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let remote = self.other_peer();
        match self.verdict() {
            DesyncVerdict::SimulationSide => write!(
                f,
                "inputs agreed, state differs at tick {}, peer {remote}",
                self.tick
            )?,
            DesyncVerdict::InputsDiffer => write!(
                f,
                "inputs differ at tick {}, peer {remote}: the tapes fed different inputs",
                self.tick
            )?,
            DesyncVerdict::InputsUnsettled => write!(
                f,
                "inputs unsettled at tick {}, peer {remote}: a seat never reported its input digest",
                self.tick
            )?,
            DesyncVerdict::Unwitnessed => {
                return write!(
                    f,
                    "no peer reported tick {}: nothing to compare, never agreement",
                    self.tick
                )
            }
        };
        match self.last_agreed_tick {
            Some(tick) => write!(f, ", last agreed tick {tick}"),
            None => write!(f, ", no earlier tick agreed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollback::{P2pSession, RollbackError, SimTick};

    /// Integer-only probe sim: every input byte reaches the hash.
    #[derive(Clone, Default)]
    struct ProbeSim {
        acc: u64,
    }

    impl SimTick for ProbeSim {
        fn advance(&mut self, inputs: [u32; PLAYERS]) {
            for input in inputs {
                self.acc = self
                    .acc
                    .wrapping_mul(0x0000_0100_0000_0001)
                    .wrapping_add(u64::from(input));
            }
        }

        fn state_hash(&self) -> u64 {
            self.acc
        }
    }

    /// `frames` settled frames: every seat's true input arrives on time.
    fn settled_session(frames: u32) -> P2pSession<ProbeSim> {
        let mut session = P2pSession::new(ProbeSim::default());
        for f in 0..frames {
            session.advance_frame(f.wrapping_add(1), Some(f.wrapping_add(2)));
        }
        session
    }

    #[test]
    fn settled_inputs_name_the_sim_not_the_wire() {
        let session = settled_session(6);
        let good = session.hash_at(3).expect("frame 3 authored");
        let bad = good.wrapping_add(1);
        let err = session
            .check_desync(3, bad)
            .expect_err("a forged peer hash must refuse");
        let RollbackError::Desync {
            frame,
            local,
            peer,
            report,
        } = err
        else {
            panic!("a forged probe must refuse with Desync, got {err}");
        };
        assert_eq!(
            (frame, local, peer),
            (3, good, bad),
            "the probe context stays exact"
        );
        assert!(report.witnessed(), "the peer reported state for the tick");
        assert!(
            report.inputs_agree(),
            "settled inputs must exonerate the wire"
        );
        assert_eq!(
            report.verdict(),
            DesyncVerdict::SimulationSide,
            "identical inputs plus different state names the sim"
        );
        assert_eq!(
            report.peer_states[usize::from(PEER_REMOTE)],
            Some(bad),
            "the peer state digest rides in its slot"
        );
        assert_eq!(
            report.peer_inputs[usize::from(PEER_REMOTE)],
            report.local_inputs,
            "both seats fed the same input row"
        );
        let line = report.to_string();
        assert!(
            line.contains("inputs agreed, state differs at tick 3, peer 1"),
            "report line names the guilty side: {line}"
        );
    }

    #[test]
    fn standing_prediction_cannot_exonerate_the_wire() {
        let mut session = P2pSession::new(ProbeSim::default());
        for f in 0u32..6 {
            // The remote never arrives: every frame plays a prediction.
            session.advance_frame(f.wrapping_add(1), None);
        }
        let local = session.hash_at(3).expect("frame 3 authored");
        let err = session
            .check_desync(3, local.wrapping_add(7))
            .expect_err("a forged peer hash must refuse");
        let RollbackError::Desync { report, .. } = err else {
            panic!("a forged probe must refuse with Desync, got {err}");
        };
        assert!(report.witnessed(), "the peer reported state");
        assert!(
            !report.inputs_agree(),
            "a seat with a standing prediction is never agreement"
        );
        assert_eq!(
            report.verdict(),
            DesyncVerdict::InputsUnsettled,
            "the wire stays a suspect"
        );
        assert_eq!(
            report.peer_inputs[usize::from(PEER_REMOTE)],
            None,
            "the remote never reported its input digest"
        );
    }

    #[test]
    fn different_inputs_name_the_wire() {
        let report = DesyncReport {
            tick: 9,
            local_peer: PEER_LOCAL,
            local_state: 0x1111,
            local_inputs: Some(DesyncReport::digest_inputs(&[1, 2])),
            peer_states: [None, Some(0x2222)],
            peer_inputs: [None, Some(DesyncReport::digest_inputs(&[2, 1]))],
            last_agreed_tick: Some(8),
        };
        assert!(report.witnessed(), "the peer reported state");
        assert!(
            !report.inputs_agree(),
            "different input digests are not agreement"
        );
        assert_eq!(
            report.verdict(),
            DesyncVerdict::InputsDiffer,
            "differing inputs name the wire, not the sim"
        );
        let line = report.to_string();
        assert!(
            line.contains("inputs differ at tick 9, peer 1"),
            "report line names the wire: {line}"
        );
    }

    #[test]
    fn an_unreported_tick_is_never_agreement() {
        let report = DesyncReport {
            tick: 42,
            local_peer: PEER_LOCAL,
            local_state: 7,
            local_inputs: Some(9),
            peer_states: [None, None],
            peer_inputs: [None, None],
            last_agreed_tick: None,
        };
        assert!(!report.witnessed(), "nobody reported this tick");
        assert!(
            !report.inputs_agree(),
            "an unreported tick never reads as agreement"
        );
        assert_eq!(
            report.verdict(),
            DesyncVerdict::Unwitnessed,
            "no side may be named from nothing"
        );
        let line = report.to_string();
        assert!(
            line.contains("no peer reported tick 42"),
            "report line says nothing was witnessed: {line}"
        );
    }

    #[test]
    fn last_agreed_is_the_watermark() {
        let session = settled_session(8);
        for f in 0..6 {
            let hash = session.hash_at(f).expect("authored frame hashed");
            session
                .check_desync(f, hash)
                .expect("true peer hash passes");
        }
        let bad = session
            .hash_at(6)
            .expect("frame 6 authored")
            .wrapping_add(1);
        let RollbackError::Desync { report, .. } = session
            .check_desync(6, bad)
            .expect_err("a forged peer hash must refuse")
        else {
            panic!("a forged probe must refuse with Desync");
        };
        assert_eq!(
            report.last_agreed_tick,
            Some(5),
            "the watermark is the last green probe below the divergence"
        );

        // A green probe ABOVE the divergence names no watermark below it:
        // the clamp keeps the report from over-claiming.
        let high = session.hash_at(7).expect("frame 7 authored");
        session
            .check_desync(7, high)
            .expect("true peer hash passes");
        let RollbackError::Desync { report, .. } = session
            .check_desync(2, bad.wrapping_add(1))
            .expect_err("a forged peer hash must refuse")
        else {
            panic!("a forged probe must refuse with Desync");
        };
        assert_eq!(
            report.last_agreed_tick, None,
            "a later agreement is not a watermark below the divergence"
        );
    }

    #[test]
    fn input_digest_is_order_sensitive_and_stable() {
        let a = DesyncReport::digest_inputs(&[1, 2]);
        let again = DesyncReport::digest_inputs(&[1, 2]);
        let swapped = DesyncReport::digest_inputs(&[2, 1]);
        assert_eq!(a, again, "the same row must fold twice equal");
        assert_ne!(a, swapped, "swapped seats must not alias");
    }
}
