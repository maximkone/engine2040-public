//! Journal tests: layout pins, torn tails, corruption and writer failures.

use super::*;
use crate::{record, replay, FIXTURE_VERSION, GOLDEN_600};

fn input(tick: u32, body: u32, vx: f32, vy: f32) -> Input {
    Input { tick, body, vx, vy }
}

fn config() -> Config {
    Config {
        seed: 77,
        ticks: 10,
    }
}

fn inputs() -> Vec<Input> {
    vec![
        input(0, 3, 1.5, -2.0),
        input(0, 9, 0.25, 4.0),
        input(3, 0, -8.0, 4.0),
        input(7, 12, 6.0, 0.5),
        input(7, 99, 3.5, -3.5),
    ]
}

fn recorded() -> (Tape, Vec<u8>) {
    let mut bytes = Vec::new();
    let tape = record_journal(config(), &inputs(), &mut bytes).unwrap();
    (tape, bytes)
}

/// Record end offsets from the documented layout, not from the encoder:
/// header 8+2+2+8+4+8, then per tick 4+4+8 head, 16 per input, 16 checkpoint,
/// 8 sum. The first entry is the header end.
fn ends(tape: &Tape) -> Vec<usize> {
    let mut ends = vec![8 + 2 + 2 + 8 + 4 + 8];
    for tick in 0..tape.config.ticks {
        let count = tape.inputs.iter().filter(|i| i.tick == tick).count();
        let last = *ends.last().unwrap();
        ends.push(last + 4 + 4 + 8 + 16 * count + 16 + 8);
    }
    ends
}

fn header(version: u16, fixture: u16, seed: u64, ticks: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"E2040JN\0");
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&fixture.to_le_bytes());
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&ticks.to_le_bytes());
    let sum = hash_bytes(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}

/// Writer that stops accepting bytes at `cap` total, like a full disk.
struct Capped {
    data: Vec<u8>,
    cap: usize,
    flushes: usize,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = self.cap.saturating_sub(self.data.len());
        if room == 0 {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "disk full"));
        }
        let take = room.min(buf.len());
        self.data.extend_from_slice(&buf[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn journal_roundtrip_twice_is_byte_identical() {
    let (tape, first) = recorded();
    let (_, second) = recorded();
    assert_eq!(first, second);
    assert_eq!(first.len(), *ends(&tape).last().unwrap());
    assert_eq!(&first[..8], b"E2040JN\0");
    let read = read_journal(&first).unwrap();
    assert!(!read.torn_tail);
    assert_eq!(read.ticks_present(), 10);
    assert_eq!(read.missing_ticks(), 0);
}

#[test]
fn journal_into_tape_equals_direct_tape_and_encodes_byte_identical() {
    let (_, bytes) = recorded();
    let direct = record(config(), &inputs()).unwrap();
    let rebuilt = read_journal(&bytes).unwrap().into_tape().unwrap();
    assert_eq!(rebuilt, direct);
    assert_eq!(rebuilt.encode().unwrap(), direct.encode().unwrap());
    replay(&rebuilt).unwrap();
    let again = read_journal(&bytes).unwrap().into_tape().unwrap();
    assert_eq!(again.encode().unwrap(), rebuilt.encode().unwrap());
}

#[test]
fn journal_hand_written_ticks_match_the_recorder_bytes() {
    let (tape, bytes) = recorded();
    let mut writer = JournalWriter::begin(Vec::new(), tape.config).unwrap();
    for (tick, checkpoint) in (0..tape.config.ticks).zip(&tape.checkpoints) {
        let own: Vec<Input> = tape
            .inputs
            .iter()
            .filter(|i| i.tick == tick)
            .copied()
            .collect();
        assert_eq!(writer.next_tick(), tick);
        writer.append_tick(&own, *checkpoint).unwrap();
    }
    assert!(writer.is_complete());
    assert_eq!(writer.into_inner(), bytes);
}

#[test]
fn journal_golden_run_ends_on_the_golden_checkpoint() {
    let mut bytes = Vec::new();
    let tape = record_journal(Config::default(), &[], &mut bytes).unwrap();
    assert_eq!(tape.checkpoints.last().unwrap().legacy, GOLDEN_600);
    let rebuilt = read_journal(&bytes).unwrap().into_tape().unwrap();
    assert_eq!(rebuilt.encode().unwrap(), tape.encode().unwrap());
}

#[test]
fn journal_every_truncation_reads_a_valid_prefix_and_flags_the_torn_tail() {
    let (tape, bytes) = recorded();
    let ends = ends(&tape);
    for cut in 0..=bytes.len() {
        let result = read_journal(&bytes[..cut]);
        if cut < ends[0] {
            assert!(
                matches!(result, Err(JournalError::Header(_))),
                "cut {cut}: {result:?}"
            );
            continue;
        }
        let read = result.unwrap_or_else(|error| panic!("cut {cut}: {error}"));
        let whole = ends.iter().filter(|end| **end <= cut).count() - 1;
        assert_eq!(read.checkpoints, tape.checkpoints[..whole], "cut {cut}");
        let want: Vec<Input> = tape
            .inputs
            .iter()
            .filter(|i| usize::try_from(i.tick).unwrap() < whole)
            .copied()
            .collect();
        assert_eq!(read.inputs, want, "cut {cut}");
        assert_eq!(read.torn_tail, !ends.contains(&cut), "cut {cut}");
        let missing = tape.config.ticks - u32::try_from(whole).unwrap();
        assert_eq!(read.missing_ticks(), missing, "cut {cut}");
        match read.into_tape() {
            Ok(rebuilt) => assert_eq!((missing, rebuilt), (0, tape.clone())),
            Err(error @ JournalError::Incomplete { .. }) => {
                assert_ne!(missing, 0, "cut {cut}");
                assert!(
                    error.to_string().contains(&format!("{missing} of 10")),
                    "{error}"
                );
            }
            Err(other) => panic!("cut {cut}: {other}"),
        }
    }
}

#[test]
fn journal_one_flipped_byte_anywhere_is_a_hard_error_naming_the_tick() {
    let (tape, bytes) = recorded();
    let ends = ends(&tape);
    for offset in 0..bytes.len() {
        for mask in [0x01, 0x80, 0xff] {
            let mut bad = bytes.clone();
            bad[offset] ^= mask;
            let error = read_journal(&bad).expect_err(&format!("flip {offset}^{mask:#x}"));
            if offset < ends[0] {
                assert!(
                    matches!(error, JournalError::Header(_) | JournalError::Proof(_)),
                    "header flip {offset}: {error}"
                );
                continue;
            }
            let owner = ends
                .windows(2)
                .position(|w| (w[0]..w[1]).contains(&offset))
                .unwrap();
            let want = u32::try_from(owner).unwrap();
            assert!(
                matches!(error, JournalError::Corrupt { tick, .. } if tick == want),
                "flip {offset}^{mask:#x} in tick {want}: {error}"
            );
            assert!(
                error.to_string().contains(&format!("tick {want}")),
                "{error}"
            );
        }
    }
}

#[test]
fn journal_gap_reorder_and_trailing_bytes_are_corruption() {
    let (tape, bytes) = recorded();
    let ends = ends(&tape);
    let mut gap = bytes[..ends[4]].to_vec();
    gap.extend_from_slice(&bytes[ends[5]..]);
    assert!(matches!(
        read_journal(&gap),
        Err(JournalError::Corrupt { tick: 4, .. })
    ));
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(matches!(
        read_journal(&extra),
        Err(JournalError::Corrupt { tick: 10, .. })
    ));
    let mut doubled = bytes.clone();
    doubled.extend_from_slice(&bytes[ends[9]..]);
    assert!(matches!(
        read_journal(&doubled),
        Err(JournalError::Corrupt { tick: 10, .. })
    ));
}

#[test]
fn journal_header_is_bounded_and_versioned() {
    let ok = header(JOURNAL_VERSION, FIXTURE_VERSION, 5, 3);
    let read = read_journal(&ok).unwrap();
    assert_eq!(
        (read.config, read.missing_ticks()),
        (Config { seed: 5, ticks: 3 }, 3)
    );
    assert!(!read.torn_tail);
    assert!(
        read.checkpoints.capacity() <= 1,
        "claimed ticks must not allocate"
    );
    let huge = read_journal(&header(
        JOURNAL_VERSION,
        FIXTURE_VERSION,
        5,
        crate::MAX_TICKS,
    ))
    .unwrap();
    assert!(huge.checkpoints.capacity() <= 1);
    for bad in [
        header(JOURNAL_VERSION + 1, FIXTURE_VERSION, 5, 3),
        header(JOURNAL_VERSION, FIXTURE_VERSION + 1, 5, 3),
        header(JOURNAL_VERSION, FIXTURE_VERSION, 5, 0),
        header(JOURNAL_VERSION, FIXTURE_VERSION, 5, crate::MAX_TICKS + 1),
    ] {
        assert!(read_journal(&bad).is_err());
    }
    let mut magic = ok.clone();
    magic[0] = b'X';
    assert!(matches!(read_journal(&magic), Err(JournalError::Header(_))));
    let oversized = vec![0_u8; max_journal_bytes() + 1];
    assert!(read_journal(&oversized).is_err());
}

#[test]
fn journal_record_head_with_a_valid_sum_but_hostile_length_is_refused() {
    let bytes = header(JOURNAL_VERSION, FIXTURE_VERSION, 5, 3);
    for len in [0_u32, 15, 17, u32::MAX, 16 * 102] {
        let mut head = Vec::new();
        head.extend_from_slice(&len.to_le_bytes());
        head.extend_from_slice(&0_u32.to_le_bytes());
        let sum = hash_bytes(&head);
        head.extend_from_slice(&sum.to_le_bytes());
        let mut attempt = bytes.clone();
        attempt.extend_from_slice(&head);
        attempt.extend_from_slice(&[0; 64]);
        assert!(
            matches!(
                read_journal(&attempt),
                Err(JournalError::Corrupt { tick: 0, .. })
            ),
            "len {len}"
        );
    }
}

#[test]
fn journal_writer_flushes_each_tick_and_stays_closed_after_a_disk_failure() {
    let (tape, bytes) = recorded();
    let ends = ends(&tape);
    let mut full = Capped {
        data: Vec::new(),
        cap: usize::MAX,
        flushes: 0,
    };
    let again = record_journal(config(), &inputs(), &mut full).unwrap();
    assert_eq!(
        (again, full.flushes, full.data.clone()),
        (tape.clone(), 11, bytes.clone())
    );
    // Disk fills in the middle of tick 4: the file holds ticks 0..4 whole plus
    // a partial record, and the writer refuses to append after the failure.
    let cap = ends[4] + 11;
    let mut sink = Capped {
        data: Vec::new(),
        cap,
        flushes: 0,
    };
    let mut writer = JournalWriter::begin(&mut sink, tape.config).unwrap();
    for tick in 0..4_u32 {
        let own: Vec<Input> = tape
            .inputs
            .iter()
            .filter(|i| i.tick == tick)
            .copied()
            .collect();
        writer
            .append_tick(&own, tape.checkpoints[usize::try_from(tick).unwrap()])
            .unwrap();
    }
    let failed = writer.append_tick(&[], tape.checkpoints[4]).unwrap_err();
    assert_eq!(failed, JournalError::Io(io::ErrorKind::StorageFull));
    assert_eq!(writer.next_tick(), 4);
    let closed = writer.append_tick(&[], tape.checkpoints[4]).unwrap_err();
    assert!(matches!(closed, JournalError::Closed(_)), "{closed}");
    drop(writer);
    let read = read_journal(&sink.data).unwrap();
    assert_eq!((read.ticks_present(), read.torn_tail), (4, true));
    assert_eq!(read.checkpoints, tape.checkpoints[..4]);
}

#[test]
fn journal_writer_refuses_wrong_tick_overflow_and_oversized_budgets() {
    let zero = Checkpoint {
        legacy: 1,
        state: 2,
    };
    let mut writer = JournalWriter::begin(Vec::new(), Config { seed: 1, ticks: 2 }).unwrap();
    let wrong_tick = writer.append_tick(&[input(1, 0, 0.0, 0.0)], zero);
    assert!(matches!(wrong_tick, Err(JournalError::Proof(_))));
    let nan = writer.append_tick(&[input(0, 0, f32::NAN, 0.0)], zero);
    assert!(matches!(nan, Err(JournalError::Proof(_))));
    assert_eq!(writer.next_tick(), 0, "refused ticks write nothing");
    assert_eq!(writer.into_inner().len(), 32);
    let mut writer = JournalWriter::begin(Vec::new(), Config { seed: 1, ticks: 2 }).unwrap();
    writer.append_tick(&[], zero).unwrap();
    writer.append_tick(&[], zero).unwrap();
    assert!(writer.is_complete());
    assert!(matches!(
        writer.append_tick(&[], zero),
        Err(JournalError::Closed(_))
    ));
    // 41 full ticks of 100 inputs pass the per-tick check but break the
    // whole-journal budget of crate::MAX_INPUTS.
    let mut writer = JournalWriter::begin(Vec::new(), Config { seed: 1, ticks: 50 }).unwrap();
    for tick in 0..40_u32 {
        let full: Vec<Input> = (0..100).map(|body| input(tick, body, 1.0, 1.0)).collect();
        writer.append_tick(&full, zero).unwrap();
    }
    let full: Vec<Input> = (0..100).map(|body| input(40, body, 1.0, 1.0)).collect();
    assert!(matches!(
        writer.append_tick(&full, zero),
        Err(JournalError::Proof(_))
    ));
    assert_eq!(writer.next_tick(), 40);
}

#[test]
fn journal_default_fixture_with_an_input_roundtrips() {
    let mut bytes = Vec::new();
    let tape = record_journal(Config::default(), &[input(5, 1, 2.0, 2.0)], &mut bytes).unwrap();
    assert_eq!(read_journal(&bytes).unwrap().into_tape().unwrap(), tape);
}
