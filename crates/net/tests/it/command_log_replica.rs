// Attribution: idea from apple/foundationdb (Apache-2.0, verified) --
// deterministic simulation plus a ring-integrity checker. Fresh write here:
// no donor bytes. Compact tick-command log with a byte-identical replica
// check under seeded fault load.
//!
//! Command-log replica test, FoundationDB-style: a compact deterministic
//! tick-command log is replicated to two replicas through a seeded lossy
//! channel with retransmit repair. Both replicas must end byte-identical
//! with zero mismatches.

use engine_core::{Fnv64, Rng};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TickCommand {
    tick: u32,
    fighter: u8,
    mx: i8,
    mz: i8,
    buttons: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CommandLog {
    entries: Vec<TickCommand>,
}

const ENTRY_BYTES: usize = 9;
const PACKET_BYTES: usize = ENTRY_BYTES * 4;

impl CommandLog {
    fn push(&mut self, cmd: TickCommand) {
        self.entries.push(cmd);
    }
    fn encode_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.entries.len() * ENTRY_BYTES);
        for e in &self.entries {
            out.extend_from_slice(&e.tick.to_le_bytes());
            out.push(e.fighter);
            out.push(e.mx as u8);
            out.push(e.mz as u8);
            out.extend_from_slice(&e.buttons.to_le_bytes());
        }
        out
    }
    fn decode_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() % ENTRY_BYTES != 0 {
            return None;
        }
        let mut entries = Vec::with_capacity(bytes.len() / ENTRY_BYTES);
        for chunk in bytes.chunks_exact(ENTRY_BYTES) {
            let tick = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let buttons = u16::from_le_bytes([chunk[7], chunk[8]]);
            entries.push(TickCommand {
                tick,
                fighter: chunk[4],
                mx: chunk[5] as i8,
                mz: chunk[6] as i8,
                buttons,
            });
        }
        Some(Self { entries })
    }
    fn digest(&self) -> u64 {
        let mut fold = Fnv64::new();
        for b in self.encode_bytes() {
            fold.write_u8(b);
        }
        fold.finish()
    }
}

fn mismatch_count(a: &[u8], b: &[u8]) -> usize {
    let mut n = 0;
    for i in 0..a.len().max(b.len()) {
        if a.get(i) != b.get(i) {
            n += 1;
        }
    }
    n
}

struct ReplicaOutcome {
    bytes: Vec<u8>,
    drops: u64,
    rounds: u32,
}

fn replicate_through_faults(bytes: &[u8], seed: u64) -> ReplicaOutcome {
    if bytes.is_empty() {
        return ReplicaOutcome {
            bytes: Vec::new(),
            drops: 0,
            rounds: 1,
        };
    }
    let packets: Vec<&[u8]> = bytes.chunks(PACKET_BYTES).collect();
    let mut have: Vec<Option<Vec<u8>>> = vec![None; packets.len()];
    let mut remaining = packets.len();
    let mut rng = Rng::new(seed);
    let mut drops = 0_u64;
    let mut rounds = 0_u32;
    while remaining > 0 {
        rounds += 1;
        for (i, pkt) in packets.iter().enumerate() {
            if have[i].is_some() {
                continue;
            }
            if rng.next_range(0, 100) < 25 {
                drops += 1;
                continue;
            }
            have[i] = Some(pkt.to_vec());
            remaining -= 1;
        }
    }
    let mut out = Vec::with_capacity(bytes.len());
    for slot in &have {
        out.extend_from_slice(slot.as_ref().expect("all packets repaired"));
    }
    ReplicaOutcome {
        bytes: out,
        drops,
        rounds,
    }
}

fn build_log(seed: u64, ticks: u32) -> CommandLog {
    let mut rng = Rng::new(seed).fork_named("command-log");
    let mut log = CommandLog::default();
    for t in 0..ticks {
        for fighter in 0..2_u8 {
            let word = rng.next_u64();
            log.push(TickCommand {
                tick: t,
                fighter,
                mx: ((word & 7) as i8) - 3,
                mz: (((word >> 4) & 7) as i8) - 3,
                buttons: (word >> 16) as u16 & 0x3f,
            });
        }
    }
    log
}

#[test]
fn encode_roundtrip_preserves_entries() {
    let log = build_log(2040, 64);
    let bytes = log.encode_bytes();
    assert_eq!(bytes.len(), 64 * 2 * ENTRY_BYTES);
    let back = CommandLog::decode_bytes(&bytes).expect("valid encoding decodes");
    assert_eq!(back, log);
    assert_eq!(back.digest(), log.digest());
}

#[test]
fn replicas_byte_identical_under_seeded_fault_load() {
    for seed in [2040_u64, 7, 99, 1234] {
        let log = build_log(seed, 256);
        let primary = log.encode_bytes();
        let a = replicate_through_faults(&primary, seed.wrapping_mul(1_000_003).wrapping_add(11));
        let b = replicate_through_faults(&primary, seed.wrapping_mul(6_700_417).wrapping_add(77));
        assert_eq!(
            a.bytes, primary,
            "replica A must equal primary (seed {seed})"
        );
        assert_eq!(
            b.bytes, primary,
            "replica B must equal primary (seed {seed})"
        );
        assert_eq!(a.bytes, b.bytes, "2 replicas byte-identical (seed {seed})");
        assert_eq!(
            mismatch_count(&a.bytes, &b.bytes),
            0,
            "0 mismatches (seed {seed})"
        );
        assert_eq!(mismatch_count(&primary, &b.bytes), 0);
        let ra = CommandLog::decode_bytes(&a.bytes).expect("replica A decodes");
        let rb = CommandLog::decode_bytes(&b.bytes).expect("replica B decodes");
        assert_eq!(ra, log);
        assert_eq!(rb, log);
        assert_eq!(ra.digest(), log.digest());
        assert_eq!(rb.digest(), log.digest());
    }
}

#[test]
fn fault_load_drops_and_heals() {
    let log = build_log(2040, 256);
    let primary = log.encode_bytes();
    let out = replicate_through_faults(&primary, 2040);
    assert!(out.drops > 0, "fault load must drop at least one packet");
    assert!(out.rounds > 1, "repair must need retransmit rounds");
    assert_eq!(out.bytes, primary);
}

#[test]
fn tamper_is_detected() {
    let log = build_log(2040, 64);
    let primary = log.encode_bytes();
    let mut tampered = primary.clone();
    tampered[5] ^= 0x01;
    assert_eq!(mismatch_count(&primary, &primary), 0);
    assert!(mismatch_count(&primary, &tampered) > 0);
    let good = CommandLog::decode_bytes(&primary).expect("good decodes");
    let bad = CommandLog::decode_bytes(&tampered).expect("tampered still decodes");
    assert_ne!(good.digest(), bad.digest());
    assert_ne!(good, bad);
}
