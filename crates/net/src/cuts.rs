//! cuts: the severed-joint event on the wire and in the input log (BX9).
//!
//! Owner: net lane. Units: exactly [`CUT_BYTES`] wire bytes per cut, ticks at
//! 60 Hz, energy and protection in Q16. Determinism: a tick's rows sort by
//! `(swing, victim, joint)` and the log keeps one row per `(tick, victim,
//! joint)`, so any order inside a tick, and any replay, gives the same bytes,
//! the same row order and the same digest. Ticks only move forward. Errors:
//! [`NetError`], never panics.
//!
//! Consumer: [`crate::session::Server`] records each authored tick's cuts from
//! the live stepper (the arena already hashes them, so the wire row shows a
//! cut, it never decides one); [`crate::cuts::encode_cuts`] frames a window
//! for one viewer; [`crate::dedicated::HeadlessServer`] serves that packet per
//! connection; [`crate::cut_soak`] soaks 64 clients at 8 cuts/s.
//!
//! The row is the combat [`Cut`] field for field, in its documented order, so
//! a client rebuilds the arena value ([`WireCut::to_cut`]) and drives the same
//! limb spawn the authority drove.

use engine_combat::{Cut, JointId};
use engine_core::Fnv64;

use crate::session::{NetError, KIND_CUTS, MAX_PACKET_BYTES, WIRE_MAGIC, WIRE_VERSION};

/// One cut, exactly this many wire bytes.
pub const CUT_BYTES: usize = 16;

/// Most rows one cut packet frames: `12 + 32 * 16 = 524` bytes, under the
/// datagram bound, and about four minutes of cuts at the BX9 rate.
pub const MAX_CUTS_PER_PACKET: usize = 32;

/// Digest prefix: the cut log's own lane, never mixed with another fold.
const LOG_PREFIX: &[u8] = b"e2netcuts1";

/// One severed-joint event as it rides the wire.
///
/// Field order and widths are the combat [`Cut`] layout: `swing` (sequence
/// number, first), `attacker`, `victim`, `joint` wire value, reserved `flags`,
/// then the two Q16 values. `flags` is always zero in V1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WireCut {
    /// Caller sequence number, copied from the swing.
    pub swing: u32,
    /// Attacking slot.
    pub attacker: u8,
    /// Victim slot; differs from `attacker`.
    pub victim: u8,
    /// Severed joint wire value ([`JointId::as_u8`]).
    pub joint: u8,
    /// Reserved flags, always zero in V1.
    pub flags: u8,
    /// Winning effective energy in Q16, saturated.
    pub energy_q16: u32,
    /// Joint resistance in Q16, saturated.
    pub protection_q16: u32,
}

impl WireCut {
    /// Copy an arena cut onto the wire.
    #[must_use]
    pub const fn from_cut(cut: Cut) -> Self {
        Self {
            swing: cut.swing,
            attacker: cut.attacker,
            victim: cut.victim,
            joint: cut.joint,
            flags: cut.flags,
            energy_q16: cut.energy_q16,
            protection_q16: cut.protection_q16,
        }
    }

    /// Rebuild the arena cut, or `None` for a joint the arena cannot sever.
    #[must_use]
    pub fn to_cut(self) -> Option<Cut> {
        self.severed_joint()?;
        Some(Cut {
            swing: self.swing,
            attacker: self.attacker,
            victim: self.victim,
            joint: self.joint,
            flags: self.flags,
            energy_q16: self.energy_q16,
            protection_q16: self.protection_q16,
        })
    }

    /// The joint this row severs, or `None` when the wire value names no
    /// severable joint.
    #[must_use]
    pub fn severed_joint(self) -> Option<JointId> {
        JointId::from_u8(self.joint).filter(|joint| joint.is_severable())
    }

    /// Range-check one row: a severable joint, two distinct slots and V1's
    /// zero flags. Energy and protection stay any `u32`: the arena saturates
    /// them, and a saturated authority row must still cross the wire.
    pub fn validate(self) -> Result<(), NetError> {
        if self.severed_joint().is_none() || self.attacker == self.victim || self.flags != 0 {
            return Err(NetError::BadPayload);
        }
        Ok(())
    }

    /// The 16 wire bytes, little-endian integers.
    #[must_use]
    pub fn encode(self) -> [u8; CUT_BYTES] {
        let mut out = [0u8; CUT_BYTES];
        out[0..4].copy_from_slice(&self.swing.to_le_bytes());
        out[4] = self.attacker;
        out[5] = self.victim;
        out[6] = self.joint;
        out[7] = self.flags;
        out[8..12].copy_from_slice(&self.energy_q16.to_le_bytes());
        out[12..16].copy_from_slice(&self.protection_q16.to_le_bytes());
        out
    }

    /// Decode and range-check one row.
    pub fn decode(bytes: &[u8]) -> Result<Self, NetError> {
        if bytes.len() != CUT_BYTES {
            return Err(NetError::Truncated);
        }
        let mut swing = [0u8; 4];
        swing.copy_from_slice(&bytes[0..4]);
        let mut energy = [0u8; 4];
        energy.copy_from_slice(&bytes[8..12]);
        let mut protection = [0u8; 4];
        protection.copy_from_slice(&bytes[12..16]);
        let cut = Self {
            swing: u32::from_le_bytes(swing),
            attacker: bytes[4],
            victim: bytes[5],
            joint: bytes[6],
            flags: bytes[7],
            energy_q16: u32::from_le_bytes(energy),
            protection_q16: u32::from_le_bytes(protection),
        };
        cut.validate()?;
        Ok(cut)
    }
}

/// The arena's tick cuts as wire rows, in the arena's own order.
#[must_use]
pub fn wire_cuts(cuts: &engine_arena::TickCuts) -> Vec<WireCut> {
    cuts.cuts.iter().copied().map(WireCut::from_cut).collect()
}

/// One cut at one tick: the input log's event row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CutRow {
    /// Tick the cut was resolved on.
    pub tick: u32,
    /// The cut itself.
    pub cut: WireCut,
}

/// The cut lane of the authoritative input log: tick-indexed rows, sorted.
///
/// Ticks only move forward. A row for a tick at or below the tip is a replay:
/// it is kept out of the older part of the log and deduplicated inside the
/// tip tick, so re-delivery of the same window changes nothing.
#[derive(Clone, Default, Debug)]
pub struct CutLog {
    rows: Vec<CutRow>,
}

impl CutLog {
    /// Empty log.
    #[must_use]
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Rows in canonical order.
    #[must_use]
    pub fn rows(&self) -> &[CutRow] {
        &self.rows
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no cut was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Newest tick in the log.
    #[must_use]
    pub fn tip(&self) -> Option<u32> {
        self.rows.last().map(|row| row.tick)
    }

    /// Wire bytes the rows need, without packet headers.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.rows.len().saturating_mul(CUT_BYTES)
    }

    /// Rows with `first_tick <= tick <= last_tick`.
    #[must_use]
    pub fn window(&self, first_tick: u32, last_tick: u32) -> &[CutRow] {
        let start = self.rows.partition_point(|row| row.tick < first_tick);
        let end = self.rows.partition_point(|row| row.tick <= last_tick);
        &self.rows[start..end.max(start)]
    }

    /// Record `tick`'s cuts. Returns the rows added; a replayed tick adds
    /// nothing.
    pub fn record(&mut self, tick: u32, cuts: &[WireCut]) -> usize {
        let mut added = 0;
        for cut in cuts {
            if self.push(CutRow { tick, cut: *cut }) {
                added += 1;
            }
        }
        self.sort_tick(tick);
        added
    }

    /// Record rows that already carry their tick, oldest first.
    pub fn record_rows(&mut self, rows: &[CutRow]) -> usize {
        let mut added = 0;
        let mut sorted = TickSet::default();
        for row in rows {
            if self.push(*row) {
                added += 1;
                sorted.mark(row.tick);
            }
        }
        if added > 0 {
            self.sort_ticks(&sorted);
        }
        added
    }

    /// Digest of the rows in order: the lane's receipt, never a gameplay hash.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write(LOG_PREFIX);
        fold.write_len(self.rows.len());
        for row in &self.rows {
            fold.write_u32(row.tick);
            fold.write(&row.cut.encode());
        }
        fold.finish()
    }

    /// True when this log holds exactly `other`'s rows in the same order.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.rows == other.rows
    }

    /// Append one row unless the log already holds it.
    fn push(&mut self, row: CutRow) -> bool {
        if let Some(tip) = self.tip() {
            if row.tick < tip {
                return false;
            }
            if row.tick == tip
                && self
                    .rows
                    .iter()
                    .rev()
                    .take_while(|held| held.tick == tip)
                    .any(|held| {
                        held.cut.victim == row.cut.victim && held.cut.joint == row.cut.joint
                    })
            {
                return false;
            }
        }
        self.rows.push(row);
        true
    }

    /// Sort one tick's rows in place: `(swing, victim, joint)`.
    fn sort_tick(&mut self, tick: u32) {
        let first = self
            .rows
            .iter()
            .rposition(|row| row.tick != tick)
            .map_or(0, |at| at.saturating_add(1));
        self.rows[first..].sort_by_key(|row| (row.cut.swing, row.cut.victim, row.cut.joint));
    }

    /// Sort every tick this batch touched, deepest tick first so the shuffle
    /// of a later tick never crosses an earlier one.
    fn sort_ticks(&mut self, ticks: &TickSet) {
        for tick in ticks.descending() {
            self.sort_tick(tick);
        }
    }
}

/// The ticks one batch touched, as a fixed-size ascending set: a packet holds
/// at most [`MAX_CUTS_PER_PACKET`] rows, so at most that many ticks.
#[derive(Default)]
struct TickSet {
    ticks: Vec<u32>,
}

impl TickSet {
    fn mark(&mut self, tick: u32) {
        if let Err(at) = self.ticks.binary_search(&tick) {
            self.ticks.insert(at, tick);
        }
    }

    fn descending(&self) -> impl Iterator<Item = u32> + '_ {
        self.ticks.iter().rev().copied()
    }
}

/// Keep the rows a viewer selected: a cut is routed by its victim, the body
/// that loses the limb. `selected` is ascending and unique, as every
/// [`crate::interest::ConnectionInterest::select`] output is.
#[must_use]
pub fn route(selected: &[u32], rows: &[CutRow]) -> Vec<CutRow> {
    rows.iter()
        .copied()
        .filter(|row| selected.binary_search(&u32::from(row.cut.victim)).is_ok())
        .collect()
}

/// Cut packet header: magic 2 + version 1 + kind 1 + first tick 4 + last tick
/// 4 + count 1.
pub const CUTS_HEADER_LEN: usize = 13;

/// Cut packet row: the row's tick 4 plus the 16-byte cut event.
pub const CUTS_ROW_BYTES: usize = 4 + CUT_BYTES;

/// Encode a cut packet: magic, version, [`KIND_CUTS`], first and last tick,
/// count, then `count` rows of `tick + 16 cut bytes`. Rows must be ascending,
/// non-empty and at most [`MAX_CUTS_PER_PACKET`]; the tick frame is the
/// input-log coordinate, the event itself stays 16 bytes.
pub fn encode_cuts(rows: &[CutRow]) -> Result<Vec<u8>, NetError> {
    let first = rows.first().ok_or(NetError::BadPayload)?.tick;
    let last = rows.last().ok_or(NetError::BadPayload)?.tick;
    if rows.len() > MAX_CUTS_PER_PACKET || first > last {
        return Err(NetError::BadPayload);
    }
    let mut previous: Option<(u32, u32, u8, u8)> = None;
    for row in rows {
        row.cut.validate()?;
        if row.tick < first || row.tick > last {
            return Err(NetError::BadPayload);
        }
        let key = (row.tick, row.cut.swing, row.cut.victim, row.cut.joint);
        if previous.is_some_and(|held| held >= key) {
            return Err(NetError::BadPayload);
        }
        previous = Some(key);
    }
    let mut out = Vec::with_capacity(CUTS_HEADER_LEN + rows.len() * CUTS_ROW_BYTES);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_CUTS);
    out.extend_from_slice(&first.to_le_bytes());
    out.extend_from_slice(&last.to_le_bytes());
    out.push(u8::try_from(rows.len()).map_err(|_| NetError::BadPayload)?);
    for row in rows {
        out.extend_from_slice(&row.tick.to_le_bytes());
        out.extend_from_slice(&row.cut.encode());
    }
    Ok(out)
}

/// Decode and validate a cut packet: `(last tick, rows)` in canonical order.
/// A row names a severable joint and two distinct slots; a frame that holds
/// rows outside `first..=last` or out of order refuses loudly.
pub fn decode_cuts(bytes: &[u8]) -> Result<(u32, Vec<CutRow>), NetError> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    if bytes.len() < CUTS_HEADER_LEN {
        return Err(NetError::Truncated);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != WIRE_MAGIC {
        return Err(NetError::BadMagic);
    }
    if bytes[2] != WIRE_VERSION {
        return Err(NetError::VersionMismatch { got: bytes[2] });
    }
    if bytes[3] != KIND_CUTS {
        return Err(NetError::BadKind(bytes[3]));
    }
    let first = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let last = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let count = usize::from(bytes[12]);
    if first > last || count == 0 || count > MAX_CUTS_PER_PACKET {
        return Err(NetError::BadPayload);
    }
    let body = &bytes[CUTS_HEADER_LEN..];
    if body.len() != count * CUTS_ROW_BYTES {
        return Err(NetError::Truncated);
    }
    let mut rows = Vec::with_capacity(count);
    let mut previous: Option<(u32, u32, u8, u8)> = None;
    for chunk in body.chunks_exact(CUTS_ROW_BYTES) {
        let tick = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let cut = WireCut::decode(&chunk[4..])?;
        let row = CutRow { tick, cut };
        let key = (row.tick, row.cut.swing, row.cut.victim, row.cut.joint);
        if row.tick < first || row.tick > last || previous.is_some_and(|held| held >= key) {
            return Err(NetError::BadPayload);
        }
        previous = Some(key);
        rows.push(row);
    }
    Ok((last, rows))
}

/// Frame one viewer's window: the rows of `first_tick..=last_tick` whose
/// victim the viewer's selection holds, oldest first, capped at
/// [`MAX_CUTS_PER_PACKET`]. `None` when nothing is visible, so an idle lane
/// costs no packet. The caller re-sends while the viewer's ack has not passed
/// the rows, which is what makes the lane loss-proof.
pub fn frame_window(
    log: &CutLog,
    first_tick: u32,
    last_tick: u32,
    selected: &[u32],
) -> Result<Option<Vec<u8>>, NetError> {
    if first_tick > last_tick {
        return Ok(None);
    }
    let rows = route(selected, log.window(first_tick, last_tick));
    let take = rows.len().min(MAX_CUTS_PER_PACKET);
    if take == 0 {
        return Ok(None);
    }
    encode_cuts(&rows[..take]).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::limbs::spawn_limbs;
    use engine_physics::limb::LimbPool;

    fn cut(swing: u32, victim: u8, joint: u8) -> WireCut {
        WireCut {
            swing,
            attacker: victim.wrapping_add(1) % 8,
            victim,
            joint,
            flags: 0,
            energy_q16: 600_000,
            protection_q16: 589_824,
        }
    }

    fn limb_joint() -> u8 {
        JointId::all()
            .iter()
            .copied()
            .find(|joint| joint.is_severable())
            .expect("the anatomy has a severable joint")
            .as_u8()
    }

    #[test]
    fn wire_cut_is_16_bytes_and_round_trips() {
        let cut = WireCut {
            swing: 0xDEAD_BEEF,
            attacker: 3,
            victim: 5,
            joint: limb_joint(),
            flags: 0,
            energy_q16: 123_456,
            protection_q16: 654_321,
        };
        let bytes = cut.encode();
        assert_eq!(bytes.len(), CUT_BYTES);
        assert_eq!(WireCut::decode(&bytes), Ok(cut));
        assert_eq!(cut.to_cut().map(WireCut::from_cut), Some(cut));
    }

    #[test]
    fn wire_cut_refuses_bad_rows() {
        let good = cut(1, 2, limb_joint());
        assert_eq!(good.validate(), Ok(()));
        assert_eq!(
            WireCut { flags: 1, ..good }.validate(),
            Err(NetError::BadPayload)
        );
        assert_eq!(
            WireCut {
                victim: good.attacker,
                ..good
            }
            .validate(),
            Err(NetError::BadPayload)
        );
        assert_eq!(
            WireCut {
                joint: u8::MAX,
                ..good
            }
            .validate(),
            Err(NetError::BadPayload)
        );
        assert_eq!(WireCut::decode(&[0u8; 15]), Err(NetError::Truncated));
    }

    #[test]
    fn log_sorts_one_tick_and_keeps_replays_out() {
        let joint = limb_joint();
        let mut log = CutLog::new();
        assert_eq!(log.record(0, &[cut(9, 2, joint), cut(3, 1, joint)]), 2);
        assert_eq!(
            log.rows()
                .iter()
                .map(|row| (row.cut.swing, row.cut.victim))
                .collect::<Vec<_>>(),
            vec![(3, 1), (9, 2)]
        );
        assert_eq!(
            log.record(0, &[cut(3, 1, joint)]),
            0,
            "a replay adds nothing"
        );
        assert_eq!(log.record(2, &[cut(4, 3, joint)]), 1);
        assert_eq!(log.record(1, &[cut(5, 4, joint)]), 0, "no tick regression");
        assert_eq!(log.tip(), Some(2));
        assert_eq!(log.bytes(), 3 * CUT_BYTES);
        assert_eq!(log.window(0, 0).len(), 2);
        assert_eq!(log.window(1, 2).len(), 1);
        assert_eq!(log.window(9, 9).len(), 0);
    }

    #[test]
    fn log_digest_is_order_independent_x2() {
        let joint = limb_joint();
        let mut first = CutLog::new();
        first.record(0, &[cut(2, 4, joint)]);
        first.record(1, &[cut(3, 5, joint), cut(1, 6, joint)]);
        let mut second = CutLog::new();
        second.record(0, &[cut(2, 4, joint)]);
        second.record(1, &[cut(3, 5, joint)]);
        second.record(1, &[cut(1, 6, joint)]);
        assert!(first.matches(&second));
        assert_eq!(first.digest(), second.digest());
        assert_ne!(first.digest(), CutLog::new().digest());
    }

    #[test]
    fn route_keeps_the_selected_victims() {
        let joint = limb_joint();
        let mut log = CutLog::new();
        log.record(0, &[cut(1, 0, joint), cut(2, 1, joint)]);
        log.record(1, &[cut(3, 2, joint)]);
        let rows = log.rows();
        let kept = route(&[1, 2], rows);
        assert_eq!(
            kept.iter().map(|row| row.cut.victim).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(route(&[7], rows).is_empty());
    }

    /// BX9 settle bar: the client rebuilds the limb from the 16-byte row and
    /// the authority's own pool ten ticks later sits within 5 cm of it.
    #[test]
    fn client_limb_from_the_wire_settles_within_5_cm_in_10_ticks() {
        let joint = limb_joint();
        let rows = [CutRow {
            tick: 4,
            cut: cut(11, 2, joint),
        }];
        let cut = WireCut::decode(&rows[0].cut.encode())
            .expect("valid row")
            .to_cut()
            .expect("severable joint");
        let mut authority = LimbPool::new(4).expect("pool");
        let mut client = LimbPool::new(4).expect("pool");
        spawn_limbs(&mut authority, &[cut], 0).expect("authority spawn");
        spawn_limbs(&mut client, &[cut], 0).expect("client spawn");
        for _ in 0..10 {
            authority.step();
            client.step();
        }
        let (a, c) = (authority.limbs()[0].pos_um, client.limbs()[0].pos_um);
        let dx = i64::from(a[0]) - i64::from(c[0]);
        let dy = i64::from(a[1]) - i64::from(c[1]);
        let dz = i64::from(a[2]) - i64::from(c[2]);
        let d2_um = dx * dx + dy * dy + dz * dz;
        // 5 cm = 50_000 um, so the bar is 2_500_000_000 um^2.
        assert!(
            d2_um <= 2_500_000_000,
            "limb off by {d2_um} um^2 after 10 ticks"
        );
    }

    #[test]
    fn cut_packet_frames_and_refuses_junk() {
        use crate::session::KIND_STATE;

        let joint = limb_joint();
        let mut log = CutLog::new();
        log.record(7, &[cut(1, 2, joint)]);
        log.record(9, &[cut(2, 3, joint)]);
        let bytes = encode_cuts(log.rows()).expect("frames");
        assert_eq!(bytes.len(), CUTS_HEADER_LEN + 2 * CUTS_ROW_BYTES);
        assert_eq!(bytes[3], KIND_CUTS);
        let (last, rows) = decode_cuts(&bytes).expect("decodes");
        assert_eq!(last, 9);
        assert_eq!(rows, log.rows());

        let mut wrong_kind = bytes.clone();
        wrong_kind[3] = KIND_STATE;
        assert_eq!(decode_cuts(&wrong_kind), Err(NetError::BadKind(KIND_STATE)));
        let short = &bytes[..bytes.len() - 1];
        assert_eq!(decode_cuts(short), Err(NetError::Truncated));
        assert_eq!(encode_cuts(&[]), Err(NetError::BadPayload));
        let mut unordered = log.rows().to_vec();
        unordered.swap(0, 1);
        assert_eq!(encode_cuts(&unordered), Err(NetError::BadPayload));
    }

    #[test]
    fn frame_window_routes_and_caps() {
        let joint = limb_joint();
        let mut log = CutLog::new();
        for tick in 0..40u32 {
            log.record(tick, &[cut(tick, 0, joint)]);
        }
        let bytes = frame_window(&log, 0, 39, &[0, 1])
            .expect("frames")
            .expect("routed rows");
        let (_, rows) = decode_cuts(&bytes).expect("decode");
        assert_eq!(rows.len(), MAX_CUTS_PER_PACKET, "the cap holds");
        assert!(rows.iter().all(|row| row.cut.victim == 0));
        assert_eq!(
            frame_window(&log, 0, 39, &[9]).expect("frames"),
            None,
            "an empty selection costs no packet"
        );
        assert_eq!(frame_window(&log, 5, 4, &[0]).expect("frames"), None);
    }

    /// Live v2 fixture: the 5 m arena on seed 167, where slot 0's scripted
    /// circle-strafe meets the brain and severs five joints in six seconds
    /// (probed once over seeds 0..200; deterministic from then on).
    fn live_cut_spec() -> engine_arena::MatchSpec {
        let config = engine_arena::ArenaConfig::try_new(167, 360, 2, 5.0).expect("config");
        engine_arena::MatchSpec::kit_duel(config).expect("kit world")
    }

    /// Slot 0's scripted input for the live cut fixture: the arena's own live
    /// script shape, a circle-strafe that swings every 30 ticks.
    fn live_cut_input(tick: u32, slot: u8) -> engine_input::FighterInput {
        let (mx, mz) = match (tick / 15) % 4 {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        engine_input::FighterInput {
            tick,
            fighter: engine_input::FighterId::new(slot).expect("slot"),
            mx,
            mz,
            facing: 0,
            buttons: if tick.is_multiple_of(30) {
                engine_input::Buttons(engine_input::fighter::ATTACK)
            } else {
                engine_input::Buttons::NONE
            },
        }
    }

    /// The production seam: a live v2 match severs joints, the server records
    /// them in the input log's cut lane, one viewer's packet frames them, and
    /// the client ends with exactly the authority's rows.
    #[test]
    fn live_server_cut_lane_reaches_a_client() {
        use crate::session::{Client, Server};

        let spec = live_cut_spec();
        let mut server = Server::from_spec(spec.clone()).expect("server");
        let mut client = Client::from_spec(spec, engine_input::FighterId::new(0).expect("slot"))
            .expect("client");
        for tick in 0..360 {
            server.tick(&[live_cut_input(tick, 0)]).expect("tick");
        }
        let authority = server.cut_log();
        assert!(!authority.is_empty(), "the live v2 match severs joints");
        let mut framed = 0;
        for tick in 0..360 {
            let rows = server.cut_rows(tick);
            if rows.is_empty() {
                continue;
            }
            let bytes = encode_cuts(rows).expect("frame");
            assert_eq!(bytes.len(), CUTS_HEADER_LEN + rows.len() * CUTS_ROW_BYTES);
            assert_eq!(client.apply_cuts(&bytes).expect("apply"), tick);
            framed += rows.len();
        }
        assert_eq!(framed, authority.len());
        assert!(client.cut_log().matches(authority));
        let (_, rows) =
            decode_cuts(&encode_cuts(authority.rows()).expect("frame all")).expect("decode all");
        assert_eq!(rows, authority.rows());
    }

    /// The dedicated server serves the lane per connection: a selection that
    /// holds the victim gets the row, one without it gets no packet.
    #[test]
    fn headless_cut_packet_follows_interest() {
        use crate::dedicated::HeadlessServer;

        let mut server = HeadlessServer::from_spec(live_cut_spec()).expect("server");
        let token = 7u64;
        let slot = server.connect(token).expect("connect");
        assert_eq!(slot, 0, "the first connection takes slot 0");
        for tick in 0..360 {
            let input = live_cut_input(server.tick_now(), slot);
            let _ = tick;
            server.submit(token, input).expect("submit");
            server.tick().expect("tick");
        }
        assert!(!server.cut_log().is_empty(), "the live match severs joints");
        let row = server.cut_log().rows()[0];
        let victim = u32::from(row.cut.victim);
        let bytes = server
            .cut_packet(0, 359, &[victim])
            .expect("framing")
            .expect("a selected victim gets its rows");
        let (_, rows) = decode_cuts(&bytes).expect("decode");
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| u32::from(row.cut.victim) == victim));
        let other = (victim + 1) % 64;
        assert!(
            server
                .cut_packet(0, 359, &[other])
                .expect("framing")
                .is_none(),
            "a viewer without the victim is not sent the cut"
        );
    }
}
