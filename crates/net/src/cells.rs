//! cells: tick-budgeted pre-cooked map-cell streaming.
//! Owner: net. Units: cell ids, ticks at 60 Hz, bytes. Determinism: integer
//! math plus stable id order only, hashes via `engine-core` FNV; no float,
//! no wall clock, no `HashMap` in hashed state. Errors: [`CellError`].
//! Caller: `HeadlessServer::tick` drives [`CellQueue`]; `session::Client`
//! decodes via [`decode_packet`] plus `ingest_cell_bytes`.
//! PORT-IDEA(veloren/veloren@2dbddc2: budget-plus-priority queue shape only;
//! presence-drives-priority mapped onto `interest.rs` rings; combat-dirty
//! never culled; dirty-set-flushes-off-tick future, receipt names version).

use crate::interest::{dist2_mm, InterestCfg};
use engine_core::Fnv64;

/// Grid width of the 4x4 fixture.
pub const GRID_W: u8 = 4;
/// Grid height of the 4x4 fixture.
pub const GRID_H: u8 = 4;
/// Cells in the 4x4 fixture.
pub const CELL_COUNT: usize = 16;
/// Pre-cooked payload bytes per cell.
pub const CELL_BYTES: usize = 48;
/// Wire bytes per cell packet (magic 2 plus version 1 plus cell 1 plus
/// tick 4 plus cell-version 4).
pub const PACKET_BYTES: usize = 12;
/// Default per-tick cell budget in bytes (one datagram).
pub const TICK_BUDGET_BYTES: usize = 1200;
/// Catch-up bar in ticks for the full 4x4 set.
pub const MAX_CATCHUP_TICKS: u32 = 60;
/// Far-ring LOD cadence: far cells eligible every 3rd tick.
pub const FAR_EVERY_TICKS: u32 = 3;
/// Cell spacing in mm (10 m cells over a 40 m district tile).
pub const CELL_STEP_MM: i32 = 10_000;
/// Wire magic `b"C1"`.
pub const CELL_MAGIC: u16 = 0x3143;
/// Wire version.
pub const CELL_VERSION_WIRE: u8 = 1;

/// Cell streaming failures, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellError {
    /// Cell id outside `0..CELL_COUNT`, culprit carried.
    BadCell(u8),
    /// Packet bytes fail magic, version or length.
    BadWire,
    /// Combat-dirty set alone exceeds the tick budget.
    OverBudget { need: usize, budget: usize },
}

impl std::fmt::Display for CellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadCell(id) => write!(f, "bad cell {id}"),
            Self::BadWire => write!(f, "bad cell wire bytes"),
            Self::OverBudget { need, budget } => {
                write!(f, "cell budget blown: need {need} over {budget}")
            }
        }
    }
}

impl std::error::Error for CellError {}

/// Validate one cell id.
pub fn validate_cell_id(cell: u8) -> Result<(), CellError> {
    if usize::from(cell) < CELL_COUNT {
        Ok(())
    } else {
        Err(CellError::BadCell(cell))
    }
}

/// Centre of `cell` in mm (viewer stands at the corner cell 0).
pub const fn cell_xy(cell: u8) -> (i32, i32) {
    let x = (cell % GRID_W) as i32 * CELL_STEP_MM;
    let z = (cell / GRID_W) as i32 * CELL_STEP_MM;
    (x, z)
}

/// Pre-cooked fixture bytes for (`cell`, `version`): SplitMix-style integer
/// mix only, identical on every machine. Generation stays out of the tick.
#[must_use]
pub fn cook_cell(cell: u8, version: u32) -> [u8; CELL_BYTES] {
    let mut out = [0u8; CELL_BYTES];
    let mut s = u64::from(cell) * 0x9E37_79B1 + u64::from(version) * 0x85EB_CA6B + 0x1234_5678;
    for chunk in out.chunks_mut(8) {
        s = engine_core::splitmix64(s);
        chunk.copy_from_slice(&s.to_le_bytes()[..chunk.len()]);
    }
    out
}

/// Hash of one pre-cooked cell (id plus version plus payload).
#[must_use]
pub fn cell_hash(cell: u8, version: u32) -> u64 {
    let mut h = Fnv64::new();
    h.write_u8(cell);
    h.write_u32(version);
    h.write(&cook_cell(cell, version));
    h.finish()
}

/// One cell receipt: tick, cell, bytes, hash plus the cell version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellReceipt {
    /// Server tick the cell rode.
    pub tick: u32,
    /// Cell id.
    pub cell: u8,
    /// Wire bytes for this cell.
    pub bytes: usize,
    /// [`cell_hash`] at send time.
    pub hash: u64,
    /// Cell version (dirty-set seam for a future row).
    pub version: u32,
}

/// One cell packet on the wire: fixed 12 bytes, payload is pre-cooked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellPacket {
    /// Server tick this packet rode.
    pub tick: u32,
    /// Cell id.
    pub cell: u8,
    /// Cell version.
    pub version: u32,
}

/// Encode one cell packet: magic 2 plus version 1 plus tick 4 plus cell 1
/// plus cell-version 4. No padding, no trailing bytes.
#[must_use]
pub fn encode_packet(p: &CellPacket) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..2].copy_from_slice(&CELL_MAGIC.to_le_bytes());
    out[2] = CELL_VERSION_WIRE;
    out[3] = p.cell;
    out[4..8].copy_from_slice(&p.tick.to_le_bytes());
    out[8..12].copy_from_slice(&p.version.to_le_bytes());
    out
}

/// Decode and fully validate one cell packet.
pub fn decode_packet(bytes: &[u8]) -> Result<CellPacket, CellError> {
    if bytes.len() != 12 {
        return Err(CellError::BadWire);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != CELL_MAGIC {
        return Err(CellError::BadWire);
    }
    if bytes[2] != CELL_VERSION_WIRE {
        return Err(CellError::BadWire);
    }
    let cell = bytes[3];
    validate_cell_id(cell)?;
    Ok(CellPacket {
        tick: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        cell,
        version: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
    })
}

/// Server send queue: subscribed mask plus combat-dirty mask, drained in
/// priority order (combat first, then near, then due far) within the byte
/// budget. Stable ascending id order inside every class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellQueue {
    /// Bits of subscribed cells.
    pub subscribed: u32,
    /// Bits of combat-dirty cells (never culled).
    pub combat: u32,
    /// Bits already sent.
    pub sent: u32,
}

impl CellQueue {
    /// Empty queue (nothing subscribed).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            subscribed: 0,
            combat: 0,
            sent: 0,
        }
    }

    /// Subscribe the full 4x4 fixture.
    pub fn subscribe_all(&mut self) {
        self.subscribed = (1u32 << CELL_COUNT) - 1;
    }

    /// Mark one cell combat-dirty (never culled for budget).
    pub fn mark_combat(&mut self, cell: u8) -> Result<(), CellError> {
        validate_cell_id(cell)?;
        self.combat |= 1u32 << cell;
        Ok(())
    }

    /// Drain due cells for `tick` within `budget` bytes, appending packets
    /// plus receipts. Combat-dirty overflow refuses instead of desyncing.
    pub fn drain(
        &mut self,
        tick: u32,
        budget: usize,
        packets: &mut Vec<CellPacket>,
        receipts: &mut Vec<CellReceipt>,
    ) -> Result<usize, CellError> {
        let cfg = InterestCfg::default_siege();
        let near_mm = i64::from(cfg.near_m) * 1000;
        let far_mm = i64::from(cfg.far_m) * 1000;
        let (near2, far2) = (near_mm * near_mm, far_mm * far_mm);
        let mut used = 0usize;
        // Class passes: 0 combat, 1 near, 2 due far.
        for pass in 0..3 {
            for cell in 0..GRID_W * GRID_H {
                if self.subscribed & (1u32 << cell) == 0 || self.sent & (1u32 << cell) != 0 {
                    continue;
                }
                let dirty = self.combat & (1u32 << cell) != 0;
                let (x, z) = cell_xy(cell);
                let d2 = dist2_mm(x, z, 0, 0);
                let cls = if dirty {
                    0
                } else if d2 <= near2 {
                    1
                } else if d2 <= far2 && tick.is_multiple_of(FAR_EVERY_TICKS) {
                    2
                } else {
                    continue;
                };
                if cls != pass {
                    continue;
                }
                if used + PACKET_BYTES > budget {
                    if pass == 0 {
                        return Err(CellError::OverBudget {
                            need: used + PACKET_BYTES,
                            budget,
                        });
                    }
                    return Ok(used);
                }
                let version = u32::from(dirty);
                let p = CellPacket {
                    tick,
                    cell,
                    version,
                };
                packets.push(p);
                receipts.push(CellReceipt {
                    tick,
                    cell,
                    bytes: PACKET_BYTES,
                    hash: cell_hash(cell, version),
                    version,
                });
                used += PACKET_BYTES;
                self.sent |= 1u32 << cell;
            }
        }
        Ok(used)
    }
}

impl Default for CellQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Client mirror: bitmask of held cells plus per-cell versions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellMirror {
    /// Bits of held cells.
    pub have: u32,
    /// Version held per cell id, index by cell.
    pub versions: [u32; CELL_COUNT],
}

impl CellMirror {
    /// Empty mirror.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            have: 0,
            versions: [0; CELL_COUNT],
        }
    }

    /// Apply one decoded packet; duplicates are idempotent.
    pub fn apply(&mut self, p: &CellPacket) -> Result<CellReceipt, CellError> {
        validate_cell_id(p.cell)?;
        self.have |= 1u32 << p.cell;
        self.versions[usize::from(p.cell)] = p.version;
        Ok(CellReceipt {
            tick: p.tick,
            cell: p.cell,
            bytes: PACKET_BYTES,
            hash: cell_hash(p.cell, p.version),
            version: p.version,
        })
    }

    /// Cells held.
    #[must_use]
    pub const fn count(self) -> u32 {
        self.have.count_ones()
    }

    /// True when the full 4x4 set is held.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.have == (1u32 << CELL_COUNT) - 1
    }

    /// Canonical digest over held cells in ascending id order.
    #[must_use]
    pub fn digest(self) -> u64 {
        let mut h = Fnv64::new();
        for cell in 0..GRID_W * GRID_H {
            if self.have & (1u32 << cell) != 0 {
                h.write_u8(cell);
                h.write_u64(cell_hash(cell, self.versions[usize::from(cell)]));
            }
        }
        h.finish()
    }
}

impl Default for CellMirror {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedicated::HeadlessServer;
    use crate::session::Client;
    use engine_arena::ArenaConfig;
    use engine_input::FighterId;

    fn fixture_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 64,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        }
    }

    fn twin_digest() -> u64 {
        let mut twin = CellMirror::new();
        for cell in 0..CELL_COUNT {
            let id = u8::try_from(cell).expect("4x4 fits in u8");
            let p = CellPacket {
                tick: 0,
                cell: id,
                version: 0,
            };
            twin.apply(&p).expect("twin cell valid");
        }
        twin.digest()
    }

    struct StreamRun {
        digest: u64,
        catchup: u32,
        per_tick: Vec<usize>,
        receipts: Vec<CellReceipt>,
    }

    fn run_once() -> StreamRun {
        let cfg = fixture_config();
        let mut server = HeadlessServer::new(cfg).expect("server");
        server.cell_queue_mut().subscribe_all();
        let mut client = Client::new(cfg, FighterId::new(0).expect("slot")).expect("client");
        let mut per_tick = Vec::new();
        let mut receipts = Vec::new();
        let mut catchup = MAX_CATCHUP_TICKS + 1;
        // Fixed short run: far LOD needs a tick divisible by 3, plus
        // steady-state ticks that must send nothing further.
        for _ in 0..8u32 {
            let (checkpoint, _delta) = server.tick().expect("server tick");
            let bytes = server.cell_bytes_last_tick();
            assert!(
                bytes <= TICK_BUDGET_BYTES,
                "per-tick cell bytes {bytes} over budget"
            );
            per_tick.push(bytes);
            for packet in server.cell_packets_last_tick() {
                let wire = encode_packet(packet);
                let hash = client.ingest_cell_bytes(&wire).expect("ingest");
                assert_eq!(hash, cell_hash(packet.cell, packet.version));
            }
            receipts.extend_from_slice(server.cell_receipts_last_tick());
            // Arena truth still converges while cells stream alongside.
            let snapshot = server.snapshot().expect("snapshot");
            let tip = checkpoint;
            client
                .on_server_state(tip, &snapshot)
                .expect("witness tracks truth");
            client.resimulate();
            if catchup > MAX_CATCHUP_TICKS
                && client.cell_count() == u32::try_from(CELL_COUNT).expect("16 fits")
            {
                catchup = u32::try_from(per_tick.len()).expect("short run fits");
            }
        }
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "cell streaming must not fork arena truth"
        );
        StreamRun {
            digest: client.cell_digest(),
            catchup,
            per_tick,
            receipts,
        }
    }

    #[test]
    fn cells_tick_streams_with_receipts() {
        // Expected first: 16/16 cells, twin digest, <=1200 B per tick,
        // catch-up within 60 ticks, arena checkpoints equal.
        let expected = twin_digest();
        let first = run_once();
        assert!(first.catchup <= MAX_CATCHUP_TICKS, "catch-up over bar");
        assert_eq!(
            u32::try_from(first.receipts.len()).expect("16 fits"),
            u32::try_from(CELL_COUNT).expect("16 fits"),
            "must stream the full 4x4 set once"
        );
        for r in &first.receipts {
            assert_eq!(r.bytes, PACKET_BYTES, "receipt must name wire bytes");
            assert_eq!(r.hash, cell_hash(r.cell, r.version));
            assert_eq!(r.version, 0, "fixture has no dirty version yet");
        }
        assert_eq!(first.digest, expected, "streamed digest must equal twin");
        // Determinism x2: a second identical run converges to the same digest.
        let second = run_once();
        assert_eq!(second.digest, expected, "twin must repeat");
        assert_eq!(
            first.digest, second.digest,
            "streaming must be deterministic"
        );
        assert_eq!(first.per_tick, second.per_tick, "budget use must repeat");
    }

    #[test]
    fn cells_reject_bad_wire_and_combat_overbudget() {
        // Bad ids refuse before any state changes.
        assert_eq!(
            validate_cell_id(16),
            Err(CellError::BadCell(16)),
            "cell 16 is outside the 4x4 set"
        );
        assert_eq!(
            decode_packet(&[0u8; 5]),
            Err(CellError::BadWire),
            "short packets refuse"
        );
        let mut bad_magic = encode_packet(&CellPacket {
            tick: 0,
            cell: 0,
            version: 0,
        });
        bad_magic[0] ^= 0xFF;
        assert_eq!(
            decode_packet(&bad_magic),
            Err(CellError::BadWire),
            "wrong magic refuses"
        );
        // Combat-dirty overflow refuses instead of desyncing.
        let mut q = CellQueue::new();
        q.subscribe_all();
        for cell in 0..CELL_COUNT {
            q.mark_combat(u8::try_from(cell).expect("fits"))
                .expect("mark");
        }
        let mut packets = Vec::new();
        let mut receipts = Vec::new();
        assert!(
            q.drain(0, PACKET_BYTES - 1, &mut packets, &mut receipts)
                .is_err(),
            "combat set over a tiny budget must refuse"
        );
    }
}
