//! prediction: input prediction plus checksum desync detection.
//!
//! Owner: `engine-net`; consumers: the session's remote-input path and the
//! desync report. Units: ticks (`u32`), compact inputs (`u32`), state hashes
//! (`u64`). Determinism: integer-only, fixed rings, no wall clock, no float,
//! no map iteration in hashed state. Errors: none; a tick whose guess or
//! checksum has left its ring answers `None` (nothing to compare) instead of
//! a false alarm.
//!
//! The rollback-library shape as an idea only, reimplemented std-only with
//! zero donor bytes (MIT OR Apache-2.0): [`InputPredictor`](crate::prediction::InputPredictor) guesses a remote
//! input that has not arrived, [`PredictionTracker`](crate::prediction::PredictionTracker) files each guess by tick
//! and reports the guesses a true input later contradicts plus the streak
//! that says a peer stopped sending, and [`DesyncDetector`](crate::prediction::DesyncDetector) compares one
//! checksum per tick at a check interval and names the first divergent tick.
//! The peer-to-peer session model is not taken: no request loop, no state
//! ring and no resimulation lives here; the session layer owns those.

/// Ring depth of filed guesses: one tick per slot, so a guess is checked
/// against its true input up to this many ticks later.
pub const PREDICTION_RING: usize = 64;
/// Ring depth of the local checksum tape a peer probe compares against.
pub const DESYNC_RING: usize = 128;
/// Default desync check interval in ticks: compare every tick, so a
/// divergence is named on the tick it happened.
pub const CHECK_EVERY_TICK: u32 = 1;

/// Ring slot of a tick (one slot per tick, wrapping).
fn slot(tick: u32) -> usize {
    usize::try_from(tick).unwrap_or(usize::MAX) % PREDICTION_RING
}

/// Guesses a remote input that has not arrived yet.
///
/// One implementation per input shape: a held button wants the last confirmed
/// input, a one-off press wants the neutral default. `last_confirmed` is the
/// newest true input for that seat (`0` before any arrived).
pub trait InputPredictor {
    /// Guess the input for a tick whose true input is still missing.
    fn predict(&mut self, last_confirmed: u32) -> u32;
}

/// Repeat-last predictor: holds the newest confirmed input (held buttons).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PredictRepeatLast;

impl InputPredictor for PredictRepeatLast {
    fn predict(&mut self, last_confirmed: u32) -> u32 {
        last_confirmed
    }
}

/// Neutral predictor: answers zero, the default of a one-off press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PredictNeutral;

impl InputPredictor for PredictNeutral {
    fn predict(&mut self, _last_confirmed: u32) -> u32 {
        0
    }
}

/// Per-seat prediction bookkeeping: filed guesses, mispredictions, streaks.
///
/// A guess is filed by tick when a remote input is missing and cleared when
/// the true input arrives, so a guess is counted once however often the tick
/// is revisited. `streak` is the run of consecutive guessed ticks per seat: a
/// long streak with no error is a peer that stopped sending, not a healthy
/// link. Fixed size, no allocation per tick; seats outside `N` answer `0`
/// (predict) or `false` (confirm) without filing anything.
#[derive(Clone, Debug)]
pub struct PredictionTracker<const N: usize, P: InputPredictor> {
    predictor: P,
    last: [u32; N],
    filed: [[Option<(u32, u32)>; PREDICTION_RING]; N],
    streak: [u32; N],
    predicted: u64,
    mispredicted: u64,
    max_streak: u32,
}

impl<const N: usize, P: InputPredictor> PredictionTracker<N, P> {
    /// Empty tracker for an `N`-seat session.
    #[must_use]
    pub fn new(predictor: P) -> Self {
        Self {
            predictor,
            last: [0; N],
            filed: [[None; PREDICTION_RING]; N],
            streak: [0; N],
            predicted: 0,
            mispredicted: 0,
            max_streak: 0,
        }
    }

    /// Guess `tick`'s input for `seat` and file it for the later check.
    /// Returns zero for a seat outside the session without filing a guess.
    pub fn predict(&mut self, tick: u32, seat: usize) -> u32 {
        if seat >= N {
            return 0;
        }
        let guess = self.predictor.predict(self.last[seat]);
        self.filed[seat][slot(tick)] = Some((tick, guess));
        let streak = self.streak[seat].saturating_add(1);
        self.streak[seat] = streak;
        self.max_streak = self.max_streak.max(streak);
        self.predicted += 1;
        guess
    }

    /// File the true input for `tick`: clears the guess, resets the seat's
    /// streak, and answers `true` when the guess was wrong (one
    /// misprediction). A tick whose guess left the ring answers `false`.
    pub fn confirm(&mut self, tick: u32, seat: usize, input: u32) -> bool {
        if seat >= N {
            return false;
        }
        self.last[seat] = input;
        self.streak[seat] = 0;
        let cell = &mut self.filed[seat][slot(tick)];
        let wrong = match *cell {
            Some((filed, guess)) => filed == tick && guess != input,
            None => false,
        };
        *cell = None;
        if wrong {
            self.mispredicted += 1;
        }
        wrong
    }

    /// Newest confirmed input for `seat` (`0` before any arrived).
    #[must_use]
    pub fn last(&self, seat: usize) -> u32 {
        self.last.get(seat).copied().unwrap_or(0)
    }

    /// Consecutive guessed ticks for `seat`, reset by a confirmation.
    #[must_use]
    pub fn streak(&self, seat: usize) -> u32 {
        self.streak.get(seat).copied().unwrap_or(0)
    }

    /// Longest streak any seat reached.
    #[must_use]
    pub fn max_streak(&self) -> u32 {
        self.max_streak
    }

    /// Guesses filed so far.
    #[must_use]
    pub fn predicted_count(&self) -> u64 {
        self.predicted
    }

    /// Guesses a true input contradicted.
    #[must_use]
    pub fn mispredicted_count(&self) -> u64 {
        self.mispredicted
    }

    /// Mispredictions per 1000 played frames, rounded down (integer only).
    #[must_use]
    pub fn mispredictions_per_1000(&self, frames: u32) -> u32 {
        if frames == 0 {
            return 0;
        }
        u32::try_from(self.mispredicted.saturating_mul(1000) / u64::from(frames))
            .unwrap_or(u32::MAX)
    }
}

/// One checksum disagreement: the first divergent tick a probe named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesyncEvent {
    /// First tick whose checksums disagree.
    pub tick: u32,
    /// Local (authoritative) checksum at `tick`.
    pub local: u64,
    /// Peer-claimed checksum at `tick`.
    pub remote: u64,
    /// Tick at which the peer's checksum was compared.
    pub at: u32,
}

impl DesyncEvent {
    /// Ticks between the divergence and its detection (`0` when the peer
    /// checksum was compared on the tick it belongs to).
    #[must_use]
    pub fn lag(&self) -> u32 {
        self.at.saturating_sub(self.tick)
    }
}

/// Checksum desync probe: one local checksum per tick, one peer probe each.
///
/// The authority records its state hash for every played tick; a peer's
/// checksum for the same tick is compared against it. The first disagreement
/// is kept in [`Self::first`] even after later ticks keep disagreeing, so the
/// report names the tick that diverged rather than the newest mismatch. A
/// check interval above one compares only ticks on that cadence (a cheaper
/// report that names a later tick); every tick is the default.
#[derive(Clone, Debug)]
pub struct DesyncDetector {
    interval: u32,
    local: [Option<(u32, u64)>; DESYNC_RING],
    first: Option<DesyncEvent>,
    checked: u64,
}

impl DesyncDetector {
    /// Probe comparing every `interval`-th tick (`0` clamps to one).
    #[must_use]
    pub fn new(interval: u32) -> Self {
        Self {
            interval: interval.max(CHECK_EVERY_TICK),
            local: [None; DESYNC_RING],
            first: None,
            checked: 0,
        }
    }

    /// Probe comparing every tick: the divergence tick is named exactly.
    #[must_use]
    pub fn every_tick() -> Self {
        Self::new(CHECK_EVERY_TICK)
    }

    /// Check interval in ticks.
    #[must_use]
    pub fn interval(&self) -> u32 {
        self.interval
    }

    /// Record the authoritative checksum of `tick` (ring slot per tick).
    pub fn record(&mut self, tick: u32, checksum: u64) {
        self.local[slot(tick)] = Some((tick, checksum));
    }

    /// Authoritative checksum of `tick`, or `None` once it left the ring.
    #[must_use]
    pub fn checksum_at(&self, tick: u32) -> Option<u64> {
        match self.local[slot(tick)] {
            Some((recorded, checksum)) if recorded == tick => Some(checksum),
            _ => None,
        }
    }

    /// Compare a peer checksum for `tick` that arrived at `at`.
    ///
    /// Answers the mismatch (`Some`) and keeps the earliest one; a matching
    /// tick, a tick off the check interval and a tick whose local checksum
    /// left the ring all answer `None`.
    pub fn observe(&mut self, tick: u32, remote: u64, at: u32) -> Option<DesyncEvent> {
        if !tick.is_multiple_of(self.interval) {
            return None;
        }
        let local = self.checksum_at(tick)?;
        self.checked += 1;
        if local == remote {
            return None;
        }
        let event = DesyncEvent {
            tick,
            local,
            remote,
            at,
        };
        if self.first.is_none() {
            self.first = Some(event);
        }
        Some(event)
    }

    /// Earliest disagreement the probe named, if any.
    #[must_use]
    pub fn first(&self) -> Option<DesyncEvent> {
        self.first
    }

    /// Peer checksums actually compared against a local checksum.
    #[must_use]
    pub fn checked(&self) -> u64 {
        self.checked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{Frame as LinkFrame, LinkProfile, LoopbackWire, Peer, Wire};
    use std::collections::BTreeMap;

    /// Seats in the local proof: one server truth plus three clients.
    const SEATS: usize = 3;
    /// Ticks a scripted input is held, human-shaped.
    const HOLD: u32 = 4;
    /// Neutral ticks at the start of a match: nobody presses yet.
    const QUIET: u32 = 4;

    /// History-sensitive integer sim: any wrong input changes every later hash.
    #[derive(Clone, Default)]
    struct TriSim {
        acc: u64,
    }

    impl TriSim {
        fn step(&mut self, inputs: [u32; SEATS]) {
            let mut mix = 0u64;
            for (seat, input) in inputs.iter().enumerate() {
                let shift = u32::try_from(seat).unwrap_or(0) * 8;
                mix ^= u64::from(*input) << shift;
            }
            self.acc = engine_core::mix64(self.acc ^ mix);
        }

        fn hash(&self) -> u64 {
            self.acc
        }
    }

    fn script(seat: usize, tick: u32) -> u32 {
        if tick < QUIET {
            return 0;
        }
        let lane = 0x51u64 ^ u64::try_from(seat).unwrap_or(0);
        u32::try_from(engine_core::mix64(u64::from(tick / HOLD) ^ lane) % 4).unwrap_or(0)
    }

    /// One wire frame of the local proof: an authored input or a checksum.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Probe {
        Input { tick: u32, seat: usize, input: u32 },
        Checksum { tick: u32, hash: u64 },
    }

    fn input_frame(tick: u32, seat: usize, input: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(13);
        bytes.push(0);
        bytes.extend_from_slice(&tick.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(seat).unwrap_or(u32::MAX).to_le_bytes());
        bytes.extend_from_slice(&input.to_le_bytes());
        bytes
    }

    fn checksum_frame(tick: u32, hash: u64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(13);
        bytes.push(1);
        bytes.extend_from_slice(&tick.to_le_bytes());
        bytes.extend_from_slice(&hash.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Probe> {
        if bytes.len() < 13 {
            return None;
        }
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let tick = word(1);
        match bytes[0] {
            0 => Some(Probe::Input {
                tick,
                seat: usize::try_from(word(5)).unwrap_or(usize::MAX),
                input: word(9),
            }),
            1 => {
                let mut hash = [0u8; 8];
                hash.copy_from_slice(&bytes[5..13]);
                Some(Probe::Checksum {
                    tick,
                    hash: u64::from_le_bytes(hash),
                })
            }
            _ => None,
        }
    }

    fn peer(seat: usize) -> Peer {
        Peer::Client(u8::try_from(seat).unwrap_or(u8::MAX))
    }

    /// One predicting client of the local proof.
    struct Client {
        seat: usize,
        sim: TriSim,
        played: u32,
        own: BTreeMap<u32, u32>,
        truth: BTreeMap<(u32, usize), u32>,
        tracker: PredictionTracker<SEATS, PredictRepeatLast>,
    }

    impl Client {
        fn new(seat: usize) -> Self {
            Self {
                seat,
                sim: TriSim::default(),
                played: 0,
                own: BTreeMap::new(),
                truth: BTreeMap::new(),
                tracker: PredictionTracker::new(PredictRepeatLast),
            }
        }

        /// Author `tick`: file it locally and hand the datagram to the wire.
        fn author(&mut self, tick: u32, wire: &mut LoopbackWire) {
            let sample = script(self.seat, tick);
            self.own.insert(tick, sample);
            wire.datagram(
                peer(self.seat),
                Peer::Server,
                &input_frame(tick, self.seat, sample),
            );
        }

        /// Take in the inputs the server relayed: a tick already played is
        /// checked against its guess, a future tick is held for the play.
        fn absorb(&mut self, wire: &mut LoopbackWire) {
            for (_, frame) in wire.recv(peer(self.seat)) {
                let LinkFrame::Datagram(bytes) = frame else {
                    continue;
                };
                let Some(Probe::Input { tick, seat, input }) = decode(&bytes) else {
                    continue;
                };
                if seat == self.seat {
                    continue;
                }
                if tick < self.played {
                    self.tracker.confirm(tick, seat, input);
                } else {
                    self.truth.insert((tick, seat), input);
                }
            }
        }

        /// Play `tick`: own input from its authored slot, a held remote input
        /// wins, a missing one is predicted. Returns the post-tick hash.
        fn play(&mut self, tick: u32) -> u64 {
            let mut inputs = [0u32; SEATS];
            inputs[self.seat] = self.own.get(&tick).copied().unwrap_or(0);
            for (seat, cell) in inputs.iter_mut().enumerate() {
                if seat == self.seat {
                    continue;
                }
                *cell = match self.truth.get(&(tick, seat)).copied() {
                    Some(input) => {
                        self.tracker.confirm(tick, seat, input);
                        input
                    }
                    None => self.tracker.predict(tick, seat),
                };
            }
            self.played = tick.saturating_add(1);
            self.sim.step(inputs);
            self.sim.hash()
        }
    }

    /// What one three-client local run measured.
    #[derive(Clone, Copy, Debug)]
    struct Receipt {
        ticks: u32,
        predicted: u64,
        mispredicted: u64,
        per_1000: u32,
        desyncs: u64,
        first_desync: Option<DesyncEvent>,
        tips: [u64; SEATS],
        truth: u64,
    }

    /// The same authored inputs applied on time: the authority's tape.
    fn truth_tape(ticks: u32) -> (Vec<u64>, u64) {
        let mut sim = TriSim::default();
        let mut tape = Vec::with_capacity(usize::try_from(ticks).unwrap_or(0));
        for tick in 0..ticks {
            let mut inputs = [0u32; SEATS];
            for (seat, cell) in inputs.iter_mut().enumerate() {
                *cell = script(seat, tick);
            }
            sim.step(inputs);
            tape.push(sim.hash());
        }
        (tape, sim.hash())
    }

    /// Three predicting clients over a loopback wire, `delay` frames of local
    /// input delay, one authoritative tape. `perturb` adds one to that seat's
    /// state at that tick (a seeded determinism bug).
    fn run_three(delay: u32, ticks: u32, perturb: Option<(usize, u32)>) -> Receipt {
        let lan = LinkProfile {
            latency: 1,
            jitter: 0,
            loss_pct: 0,
            reorder_pct: 0,
        };
        let mut wire = LoopbackWire::new(u8::try_from(SEATS).unwrap_or(u8::MAX), lan, lan, 0x225E);
        let mut clients: [Client; SEATS] = std::array::from_fn(Client::new);
        let (tape, truth) = truth_tape(ticks);
        let mut detector = DesyncDetector::every_tick();
        let mut desyncs = 0u64;
        let mut first_desync = None;
        // Countdown: author the first `delay` ticks before the first play.
        for w in 0..delay {
            for client in clients.iter_mut() {
                client.author(w, &mut wire);
            }
            wire.tick();
        }
        for w in 0..ticks {
            for client in clients.iter_mut() {
                client.author(w.saturating_add(delay), &mut wire);
            }
            for client in clients.iter_mut() {
                client.absorb(&mut wire);
            }
            detector.record(w, tape[usize::try_from(w).unwrap_or(0)]);
            for (_, frame) in wire.recv(Peer::Server) {
                let LinkFrame::Datagram(bytes) = frame else {
                    continue;
                };
                match decode(&bytes) {
                    Some(Probe::Input { .. }) => {
                        for seat in 0..SEATS {
                            wire.datagram(Peer::Server, peer(seat), &bytes);
                        }
                    }
                    Some(Probe::Checksum { tick, hash }) => {
                        if let Some(event) = detector.observe(tick, hash, w) {
                            desyncs += 1;
                            if first_desync.is_none() {
                                first_desync = Some(event);
                            }
                        }
                    }
                    None => {}
                }
            }
            for (seat, client) in clients.iter_mut().enumerate() {
                if perturb == Some((seat, w)) {
                    client.sim.acc = client.sim.acc.wrapping_add(1);
                }
                let hash = client.play(w);
                wire.datagram(peer(seat), Peer::Server, &checksum_frame(w, hash));
            }
            wire.tick();
        }
        let mut predicted = 0u64;
        let mut mispredicted = 0u64;
        let mut tips = [0u64; SEATS];
        for (seat, client) in clients.iter().enumerate() {
            predicted += client.tracker.predicted_count();
            mispredicted += client.tracker.mispredicted_count();
            tips[seat] = client.sim.hash();
        }
        Receipt {
            ticks,
            predicted,
            mispredicted,
            per_1000: u32::try_from(mispredicted.saturating_mul(1000) / u64::from(ticks.max(1)))
                .unwrap_or(u32::MAX),
            desyncs,
            first_desync,
            tips,
            truth,
        }
    }

    #[test]
    fn three_clients_two_frame_delay_mispredict_under_two_per_thousand() {
        let ticks = 1000;
        let on = run_three(2, ticks, None);
        println!("three clients, delay 2: {on:?}");
        assert_eq!(on.ticks, ticks, "the receipt names its frame count");
        assert!(
            on.mispredicted < 2 && on.per_1000 < 2,
            "a two-frame delay covers the one-tick relay: {on:?}"
        );
        assert!(on.predicted > 0, "the predictor must have run: {on:?}");
        for tip in on.tips {
            assert_eq!(tip, on.truth, "each client tip is the authority: {on:?}");
        }
        assert!(on.first_desync.is_none(), "no desync on a covered link");
        let again = run_three(2, ticks, None);
        assert_eq!(
            (again.predicted, again.mispredicted, again.tips, again.truth),
            (on.predicted, on.mispredicted, on.tips, on.truth),
            "same seed, same receipt"
        );
        // Non-vacuous contrast: without the delay the same seats guess late.
        let bare = run_three(0, ticks, None);
        println!("three clients, delay 0: {bare:?}");
        assert!(
            bare.mispredicted > 100 && bare.predicted > 100,
            "delay 0 must guess every remote tick: {bare:?}"
        );
        assert!(bare.per_1000 > 100, "{bare:?}");
    }

    #[test]
    fn desync_is_detected_within_one_tick() {
        let ticks = 200;
        let diverged = 120;
        let receipt = run_three(2, ticks, Some((1, diverged)));
        println!("three clients, one perturbed seat: {receipt:?}");
        let event = receipt.first_desync.expect("the perturbed seat desyncs");
        assert_eq!(
            event.tick, diverged,
            "the probe names the divergent tick: {event:?}"
        );
        assert!(
            event.lag() <= 1,
            "detected within one tick of the divergence: {event:?}"
        );
        assert_ne!(event.local, event.remote);
        assert!(receipt.desyncs > 0);
    }

    #[test]
    fn predictors_repeat_last_and_answer_neutral() {
        let mut repeat = PredictRepeatLast;
        assert_eq!(repeat.predict(7), 7);
        assert_eq!(repeat.predict(0), 0);
        let mut neutral = PredictNeutral;
        assert_eq!(neutral.predict(7), 0);
    }

    #[test]
    fn tracker_counts_one_misprediction_and_resets_the_streak() {
        let mut tracker = PredictionTracker::<SEATS, PredictRepeatLast>::new(PredictRepeatLast);
        tracker.confirm(0, 1, 5);
        assert_eq!(tracker.predict(1, 1), 5, "repeat the last confirmed");
        assert_eq!(tracker.predict(2, 1), 5);
        assert_eq!(tracker.streak(1), 2);
        assert!(tracker.confirm(1, 1, 9), "the true input contradicted it");
        assert!(!tracker.confirm(1, 1, 9), "a guess is counted once");
        assert!(!tracker.confirm(2, 1, 5), "the second guess was right");
        assert_eq!(tracker.mispredicted_count(), 1);
        assert_eq!(tracker.predicted_count(), 2);
        assert_eq!(tracker.mispredictions_per_1000(1000), 1);
        assert_eq!(tracker.streak(1), 0);
        assert_eq!(tracker.max_streak(), 2);
        assert_eq!(tracker.last(1), 5);
        assert!(!tracker.confirm(0, SEATS, 1), "no such seat");
        assert_eq!(tracker.predict(0, SEATS), 0, "no such seat");
    }

    #[test]
    fn tracker_forgets_a_guess_that_left_its_ring() {
        let mut tracker = PredictionTracker::<1, PredictNeutral>::new(PredictNeutral);
        let depth = u32::try_from(PREDICTION_RING).unwrap_or(0);
        for tick in 0..=depth {
            tracker.predict(tick, 0);
        }
        // Tick `depth` took tick 0's slot: a stale confirm is no mismatch.
        assert!(!tracker.confirm(0, 0, 3), "the slot now holds another tick");
        assert_eq!(tracker.mispredicted_count(), 0);
        let again = depth.saturating_mul(2);
        tracker.predict(again, 0);
        assert!(tracker.confirm(again, 0, 3), "the filed guess was zero");
        assert_eq!(tracker.mispredicted_count(), 1);
    }

    #[test]
    fn detector_names_the_first_divergent_tick_and_keeps_it() {
        let mut detector = DesyncDetector::every_tick();
        for tick in 0..16u32 {
            detector.record(tick, u64::from(tick) * 3);
        }
        for tick in 0..16u32 {
            assert_eq!(detector.observe(tick, u64::from(tick) * 3, tick + 1), None);
        }
        assert_eq!(detector.checked(), 16);
        assert!(detector.first().is_none());
        let event = detector
            .observe(7, 0xDEAD, 8)
            .expect("checksums disagree at tick 7");
        assert_eq!(
            (event.tick, event.local, event.remote, event.at),
            (7, 21, 0xDEAD, 8)
        );
        assert_eq!(event.lag(), 1);
        let later = detector.observe(9, 0xBEEF, 10).expect("still diverged");
        assert_eq!(later.tick, 9);
        assert_eq!(
            detector.first(),
            Some(event),
            "the first divergence is kept"
        );
        assert_eq!(detector.checked(), 18);
        assert_eq!(detector.observe(200, 1, 201), None, "unknown tick");
    }

    #[test]
    fn detector_checks_only_ticks_on_its_interval() {
        let mut detector = DesyncDetector::new(4);
        assert_eq!(detector.interval(), 4);
        assert_eq!(DesyncDetector::new(0).interval(), CHECK_EVERY_TICK);
        for tick in 0..16u32 {
            detector.record(tick, 1);
        }
        assert_eq!(detector.observe(5, 2, 6), None, "5 is off the cadence");
        let event = detector.observe(8, 2, 9).expect("8 is on the cadence");
        assert_eq!((event.tick, event.lag()), (8, 1));
        assert_eq!(detector.checked(), 1);
    }
}
