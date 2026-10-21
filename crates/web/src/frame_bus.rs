//! frame_bus: the seqlock that carries one frame from the sim thread to the
//! draw thread.
//!
//! One writer, many readers, nobody waits. The writer bumps a counter to
//! odd, writes the payload, bumps it to even. A reader that sees the counter
//! change throws its torn copy away and tries again, at most
//! [`MAX_READ_ATTEMPTS`] times, and then says so instead of spinning.
//!
//! Owner: this module owns the bus, the frame and the reader's counters; the
//! page owns the transport that carries the bytes.
//! Units: bytes, little-endian, no floats and no clock.
//! Determinism: the same published bytes give the same frame on every
//! platform; the bytes are copied, never interpreted.
//! Errors: [`FrameError`] for bytes that do not fit one frame, [`ReadError`]
//! for a read that never found a stable frame.
//! Budget: one copy of [`FRAME_CAPACITY`] bytes per read, no allocation.
//! Consumer: the draw thread, and the page's postMessage fallback.
//!
//! The payload lives in atomic words, not behind an `UnsafeCell`, so the
//! module needs no `unsafe`: the writer stores the words with relaxed stores
//! and publishes them with a release store of the counter; the reader loads
//! the words relaxed and fences around the counter it reads twice.
//!
//! The same reader runs over a plain byte buffer ([`PlainFrame`]), so the
//! postMessage fallback and the shared path share one code path.

use std::sync::atomic::{fence, AtomicU32, Ordering};
use std::sync::Arc;

/// Bytes one frame holds.
pub const FRAME_CAPACITY: usize = 2048;

/// How many copies a reader may make before it gives up.
pub const MAX_READ_ATTEMPTS: u32 = 8;

/// `u32` words per frame: the length word, then the payload.
const WORDS: usize = 1 + FRAME_CAPACITY / 4;

/// Keeps the counter off the payload words' cache line.
#[repr(align(64))]
struct Padded<T>(T);

/// One frame on the bus: a fixed-capacity byte buffer.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    bytes: [u8; FRAME_CAPACITY],
    len: usize,
}

impl Frame {
    /// A frame holding `bytes`.
    ///
    /// # Errors
    ///
    /// [`FrameError::TooLarge`] when the bytes do not fit one frame.
    pub fn new(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() > FRAME_CAPACITY {
            return Err(FrameError::TooLarge {
                bytes: bytes.len(),
                capacity: FRAME_CAPACITY,
            });
        }
        let mut frame = Self::empty();
        frame.bytes[..bytes.len()].copy_from_slice(bytes);
        frame.len = bytes.len();
        Ok(frame)
    }

    /// An empty frame.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            bytes: [0; FRAME_CAPACITY],
            len: 0,
        }
    }

    /// How many bytes the frame holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the frame holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes the frame holds.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Take the bytes of `other`.
    fn copy_from(&mut self, other: &Frame) {
        self.bytes = other.bytes;
        self.len = other.len;
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame {{ len: {}, bytes: [", self.len)?;
        for (i, byte) in self.bytes[..self.len.min(16)].iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{byte:02x}")?;
        }
        write!(f, "] }}")
    }
}

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The bytes do not fit in one frame.
    TooLarge {
        /// How many bytes were given.
        bytes: usize,
        /// How many bytes one frame holds.
        capacity: usize,
    },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes, capacity } => {
                write!(
                    f,
                    "frame_bus: {bytes} bytes do not fit a frame of {capacity}"
                )
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Why a read gave up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The writer never left a stable frame within the attempt budget.
    Unstable {
        /// How many copies were made and thrown away.
        attempts: u32,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unstable { attempts } => {
                write!(f, "frame_bus: no stable frame in {attempts} attempts")
            }
        }
    }
}

impl std::error::Error for ReadError {}

/// Where a reader finds the latest frame.
pub trait FrameSource {
    /// The counter now: even means the frame is stable, odd means a write is
    /// in flight.
    fn sequence(&self) -> u32;

    /// Copy the current frame into `out`. The copy may be torn; the reader
    /// reads the counter around it and throws a torn copy away.
    fn copy_frame(&self, out: &mut Frame);
}

/// The shared state of a bus: the counter and the payload words.
///
/// The payload is a fixed array of atomic words, so the bus needs no
/// `unsafe`: the writer fills the words with relaxed stores while the counter
/// is odd, and the release store of the even counter publishes them.
pub struct Shared {
    /// Even when the frame is stable, odd while a write is in flight.
    seq: Padded<AtomicU32>,
    /// Word 0 is the frame length in bytes, the rest are the payload.
    words: [AtomicU32; WORDS],
}

impl Shared {
    /// An empty bus state.
    fn new() -> Self {
        Self {
            seq: Padded(AtomicU32::new(0)),
            words: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// Copy the payload into `out`. The copy may be torn: the caller reads
    /// the counter around it.
    fn load_frame(&self, out: &mut Frame) {
        let mut words = [0u32; WORDS];
        for (cell, word) in self.words.iter().zip(words.iter_mut()) {
            *word = cell.load(Ordering::Relaxed);
        }
        *out = Self::frame_of(&words);
    }

    /// The frame the words hold: word 0 is the length, the rest are the
    /// payload, four bytes to a word, little-endian.
    fn frame_of(words: &[u32; WORDS]) -> Frame {
        let len = usize::try_from(words[0])
            .unwrap_or(FRAME_CAPACITY)
            .min(FRAME_CAPACITY);
        let mut frame = Frame::empty();
        let mut i = 0;
        while i < len {
            let bytes = words[1 + i / 4].to_le_bytes();
            let take = (len - i).min(4);
            frame.bytes[i..i + take].copy_from_slice(&bytes[..take]);
            i += take;
        }
        frame.bytes[len..].fill(0);
        frame.len = len;
        frame
    }
}

impl FrameSource for Arc<Shared> {
    fn sequence(&self) -> u32 {
        self.seq.0.load(Ordering::Acquire)
    }

    fn copy_frame(&self, out: &mut Frame) {
        self.load_frame(out);
    }
}

/// The postMessage fallback: a frame that arrived as plain bytes.
///
/// The reader runs over it exactly as it runs over the shared path, so the
/// two agree byte for byte. A transferred buffer is a private copy, so no
/// write can be in flight and the counter never changes.
#[derive(Clone, Debug)]
pub struct PlainFrame {
    bytes: Box<[u8]>,
}

impl PlainFrame {
    /// The bytes the sim posted.
    ///
    /// # Errors
    ///
    /// [`FrameError::TooLarge`] when the bytes do not fit one frame.
    pub fn new(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() > FRAME_CAPACITY {
            return Err(FrameError::TooLarge {
                bytes: bytes.len(),
                capacity: FRAME_CAPACITY,
            });
        }
        Ok(Self {
            bytes: Box::from(bytes),
        })
    }
}

impl FrameSource for PlainFrame {
    fn sequence(&self) -> u32 {
        0
    }

    fn copy_frame(&self, out: &mut Frame) {
        let len = self.bytes.len().min(FRAME_CAPACITY);
        out.bytes[..len].copy_from_slice(&self.bytes[..len]);
        out.bytes[len..].fill(0);
        out.len = len;
    }
}

/// What a reader has done so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// Frames handed to the caller.
    pub frames: u64,
    /// Copies thrown away because the counter changed while they were made.
    pub torn: u64,
    /// Attempts skipped because a write was in flight.
    pub busy: u64,
    /// Attempts made, the torn and the busy ones included.
    pub attempts: u64,
}

/// Reads the latest frame from a source. Never blocks.
pub struct FrameReader<S> {
    source: S,
    scratch: Frame,
    stats: ReadStats,
}

impl<S: FrameSource> FrameReader<S> {
    /// A reader over `source`.
    pub fn new(source: S) -> Self {
        Self {
            source,
            scratch: Frame::empty(),
            stats: ReadStats::default(),
        }
    }

    /// Copy the latest stable frame into `out`.
    ///
    /// The reader never waits for the writer: it reads the counter, copies
    /// the frame, reads the counter again and throws the copy away when the
    /// counter changed. It tries at most [`MAX_READ_ATTEMPTS`] times.
    ///
    /// On failure `out` is left alone, so it still holds the last frame the
    /// caller drew.
    ///
    /// # Errors
    ///
    /// [`ReadError::Unstable`] when the writer never left a stable frame
    /// within the attempt budget.
    pub fn read_into(&mut self, out: &mut Frame) -> Result<(), ReadError> {
        for _ in 0..MAX_READ_ATTEMPTS {
            self.stats.attempts += 1;
            let first = self.source.sequence();
            if first % 2 == 1 {
                self.stats.busy += 1;
                continue;
            }
            // The copy must land between the two counter reads.
            fence(Ordering::Acquire);
            self.source.copy_frame(&mut self.scratch);
            fence(Ordering::Acquire);
            let second = self.source.sequence();
            if first != second {
                self.stats.torn += 1;
                continue;
            }
            self.stats.frames += 1;
            out.copy_from(&self.scratch);
            return Ok(());
        }
        Err(ReadError::Unstable {
            attempts: MAX_READ_ATTEMPTS,
        })
    }

    /// The latest stable frame.
    ///
    /// # Errors
    ///
    /// As [`Self::read_into`].
    pub fn read(&mut self) -> Result<Frame, ReadError> {
        let mut frame = Frame::empty();
        self.read_into(&mut frame)?;
        Ok(frame)
    }

    /// What the reader has done so far.
    #[must_use]
    pub fn stats(&self) -> ReadStats {
        self.stats
    }
}

/// A reader over the shared path.
pub type SharedReader = FrameReader<Arc<Shared>>;

/// The bus: the state the sim writes and the draw thread reads.
#[derive(Clone)]
pub struct FrameBus {
    shared: Arc<Shared>,
}

impl FrameBus {
    /// A new bus with an empty frame.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared::new()),
        }
    }

    /// The writer end. One per bus: two writers would interleave their
    /// counter bumps and a reader could accept a half-written frame.
    pub fn writer(&self) -> FrameWriter {
        FrameWriter {
            shared: Arc::clone(&self.shared),
        }
    }

    /// A reader end. As many as the draw threads need; every reader keeps
    /// its own counters.
    pub fn reader(&self) -> SharedReader {
        FrameReader::new(Arc::clone(&self.shared))
    }
}

impl Default for FrameBus {
    fn default() -> Self {
        Self::new()
    }
}

/// The writer end: the sim thread's side of the bus.
pub struct FrameWriter {
    shared: Arc<Shared>,
}

impl FrameWriter {
    /// Publish a frame: bump the counter to odd, write the payload, bump it
    /// to even. Never blocks and never allocates.
    pub fn publish(&mut self, frame: &Frame) {
        let odd = self.begin();
        self.write_payload(frame);
        self.commit(odd);
    }

    /// Publish the bytes as a frame.
    ///
    /// # Errors
    ///
    /// [`FrameError::TooLarge`] when the bytes do not fit one frame.
    pub fn publish_bytes(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        let frame = Frame::new(bytes)?;
        self.publish(&frame);
        Ok(())
    }

    /// Bump the counter to odd: a write is in flight and no reader may
    /// accept the payload.
    fn begin(&mut self) -> u32 {
        let odd = self.shared.seq.0.load(Ordering::Relaxed).wrapping_add(1);
        self.shared.seq.0.store(odd, Ordering::Release);
        odd
    }

    /// Write the payload words. The counter is odd, so no reader accepts
    /// this copy; the release store in `commit` publishes it.
    fn write_payload(&mut self, frame: &Frame) {
        let len = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        self.shared.words[0].store(len, Ordering::Relaxed);
        let mut i = 0;
        while i < frame.len() {
            self.shared.words[1 + i / 4].store(pack_word(&frame.bytes, i), Ordering::Relaxed);
            i += 4;
        }
    }

    /// Bump the counter to even: the frame is stable again.
    fn commit(&mut self, odd: u32) {
        self.shared
            .seq
            .0
            .store(odd.wrapping_add(1), Ordering::Release);
    }
}

/// The word that holds the four bytes at `start`, little-endian.
fn pack_word(bytes: &[u8; FRAME_CAPACITY], start: usize) -> u32 {
    u32::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::AtomicBool;

    /// A source that lets a writer land in the middle of a copy: the copy the
    /// reader gets is half the old frame and half the new one.
    struct StraddleSource {
        shared: Arc<Shared>,
        writer: RefCell<FrameWriter>,
        next: Frame,
        split: usize,
        fired: Cell<bool>,
    }

    impl FrameSource for StraddleSource {
        fn sequence(&self) -> u32 {
            self.shared.seq.0.load(Ordering::Acquire)
        }

        fn copy_frame(&self, out: &mut Frame) {
            if !self.fired.replace(true) {
                let mut words = [0u32; WORDS];
                for (i, word) in words.iter_mut().enumerate().take(self.split) {
                    *word = self.shared.words[i].load(Ordering::Relaxed);
                }
                // The writer lands here: the second half of the copy is new.
                self.writer.borrow_mut().publish(&self.next);
                for (i, word) in words.iter_mut().enumerate().skip(self.split) {
                    *word = self.shared.words[i].load(Ordering::Relaxed);
                }
                *out = Shared::frame_of(&words);
                return;
            }
            self.shared.load_frame(out);
        }
    }

    fn bus() -> FrameBus {
        FrameBus::new()
    }

    #[test]
    fn a_reader_never_sees_a_half_written_frame() {
        let bus = bus();
        let mut writer = bus.writer();
        let mut reader = bus.reader();
        let old = [0x11u8; 64];
        let new = [0x22u8; 64];
        writer.publish_bytes(&old).expect("fits");
        let mut frame = Frame::empty();
        reader.read_into(&mut frame).expect("a stable frame");
        assert_eq!(frame.as_bytes(), old.as_slice());

        // The writer gets half way through the next frame: the counter is odd
        // and only the first payload word is new.
        let odd = writer.begin();
        writer.shared.words[0].store(64, Ordering::Relaxed);
        writer.shared.words[1].store(0x2222_2222, Ordering::Relaxed);

        let err = reader
            .read_into(&mut frame)
            .expect_err("the frame is in flight");
        assert_eq!(
            err,
            ReadError::Unstable {
                attempts: MAX_READ_ATTEMPTS
            }
        );
        assert_eq!(reader.stats().busy, u64::from(MAX_READ_ATTEMPTS));
        assert_eq!(reader.stats().torn, 0, "no torn copy was made");
        assert_eq!(
            frame.as_bytes(),
            old.as_slice(),
            "the last good frame stands"
        );

        // The writer finishes: the reader sees the whole new frame.
        writer.write_payload(&Frame::new(&new).expect("fits"));
        writer.commit(odd);
        reader.read_into(&mut frame).expect("a stable frame");
        assert_eq!(frame.as_bytes(), new.as_slice());
    }

    #[test]
    fn a_reader_throws_a_torn_copy_away_and_reads_again() {
        let bus = bus();
        let mut writer = bus.writer();
        let old = [0x11u8; 64];
        let new = [0x22u8; 64];
        writer.publish_bytes(&old).expect("fits");

        let source = StraddleSource {
            shared: Arc::clone(&bus.shared),
            writer: RefCell::new(writer),
            next: Frame::new(&new).expect("fits"),
            split: 9,
            fired: Cell::new(false),
        };
        let mut reader = FrameReader::new(source);
        let frame = reader.read().expect("a stable frame");
        assert_eq!(
            frame.as_bytes(),
            new.as_slice(),
            "the torn copy was thrown away"
        );
        assert_eq!(reader.stats().torn, 1, "one copy was torn");
        assert_eq!(reader.stats().frames, 1);
    }

    #[test]
    fn a_reader_never_sees_a_torn_frame_under_a_real_race() {
        let bus = bus();
        let mut writer = bus.writer();
        let mut reader = bus.reader();
        let old = [0xAAu8; 64];
        let new = [0x55u8; 64];
        writer.publish_bytes(&old).expect("fits");
        let mut frame = Frame::empty();
        reader.read_into(&mut frame).expect("a stable frame");

        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for i in 0..2000 {
                    let bytes = if i % 2 == 0 { &new } else { &old };
                    writer.publish_bytes(bytes).expect("fits");
                    std::thread::yield_now();
                }
                done.store(true, Ordering::Release);
            });
            scope.spawn(|| {
                let mut spins = 0;
                while !done.load(Ordering::Acquire) {
                    spins += 1;
                    if spins > 1_000_000 {
                        break;
                    }
                    if reader.read_into(&mut frame).is_ok() {
                        let bytes = frame.as_bytes();
                        assert!(
                            bytes == old.as_slice() || bytes == new.as_slice(),
                            "the reader saw a torn frame"
                        );
                    }
                }
            });
        });

        reader.read_into(&mut frame).expect("a stable frame");
        let bytes = frame.as_bytes();
        assert!(
            bytes == old.as_slice() || bytes == new.as_slice(),
            "the last frame is whole"
        );
    }

    #[test]
    fn the_plain_buffer_path_and_the_shared_path_agree_byte_for_byte() {
        let bus = bus();
        let mut writer = bus.writer();
        let mut shared = bus.reader();
        let mut bytes = [0u8; 256];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::try_from(i).unwrap_or(0);
        }
        writer.publish_bytes(&bytes).expect("fits");

        let from_shared = shared.read().expect("a stable frame");
        let plain = PlainFrame::new(&bytes).expect("fits");
        let from_plain = FrameReader::new(plain).read().expect("a stable frame");
        assert_eq!(from_shared.as_bytes(), from_plain.as_bytes());
        assert_eq!(from_shared.as_bytes(), bytes.as_slice());
        assert_eq!(from_shared, from_plain, "the whole frame is the same");

        // A short frame after a long one leaves no stale bytes on either path.
        writer.publish_bytes(&[1, 2, 3]).expect("fits");
        let short_shared = shared.read().expect("a stable frame");
        let short_plain = FrameReader::new(PlainFrame::new(&[1, 2, 3]).expect("fits"))
            .read()
            .expect("a stable frame");
        assert_eq!(short_shared, short_plain);
        assert_eq!(short_shared.as_bytes(), &[1, 2, 3][..]);
    }

    #[test]
    fn a_read_never_waits_for_a_write() {
        let bus = bus();
        let mut writer = bus.writer();
        let mut reader = bus.reader();
        writer.publish_bytes(&[1u8; 8]).expect("fits");
        let mut frame = Frame::empty();
        reader.read_into(&mut frame).expect("a stable frame");

        // The writer is half way through the next frame.
        let odd = writer.begin();
        writer.write_payload(&Frame::new(&[2u8; 8]).expect("fits"));
        assert_eq!(
            bus.shared.seq.0.load(Ordering::Acquire) % 2,
            1,
            "the writer is in flight"
        );

        // The read comes back while the writer is still writing.
        let err = reader
            .read_into(&mut frame)
            .expect_err("the writer is in flight");
        assert_eq!(
            err,
            ReadError::Unstable {
                attempts: MAX_READ_ATTEMPTS
            }
        );
        assert_eq!(
            bus.shared.seq.0.load(Ordering::Acquire) % 2,
            1,
            "the read did not wait for the write"
        );
        assert_eq!(
            frame.as_bytes(),
            &[1u8; 8][..],
            "the last good frame stands"
        );

        writer.commit(odd);
        reader.read_into(&mut frame).expect("a stable frame");
        assert_eq!(frame.as_bytes(), &[2u8; 8][..]);
    }

    #[test]
    fn a_read_gives_up_after_a_bounded_number_of_attempts() {
        let bus = bus();
        let mut reader = bus.reader();
        // A write that never finishes: the counter stays odd.
        bus.shared.seq.0.store(1, Ordering::Release);
        let mut frame = Frame::empty();
        let err = reader
            .read_into(&mut frame)
            .expect_err("there is no stable frame");
        assert_eq!(
            err,
            ReadError::Unstable {
                attempts: MAX_READ_ATTEMPTS
            }
        );
        assert_eq!(reader.stats().attempts, u64::from(MAX_READ_ATTEMPTS));
        assert_eq!(reader.stats().busy, u64::from(MAX_READ_ATTEMPTS));
        assert_eq!(reader.stats().frames, 0);
        assert!(frame.is_empty());
    }

    #[test]
    fn the_shared_path_stores_the_payload_as_little_endian_words() {
        let bus = bus();
        let mut writer = bus.writer();
        writer
            .publish_bytes(&[0x01, 0x02, 0x03, 0x04, 0x05])
            .expect("fits");
        // Word 0 is the length in bytes, word 1 holds the first four bytes.
        assert_eq!(bus.shared.words[0].load(Ordering::Relaxed), 5);
        assert_eq!(bus.shared.words[1].load(Ordering::Relaxed), 0x0403_0201);
        assert_eq!(bus.shared.words[2].load(Ordering::Relaxed), 0x0000_0005);
    }

    #[test]
    fn bytes_that_do_not_fit_in_one_frame_are_refused() {
        let too_big = [0u8; FRAME_CAPACITY + 1];
        let err = Frame::new(&too_big).expect_err("too big");
        assert_eq!(
            err,
            FrameError::TooLarge {
                bytes: FRAME_CAPACITY + 1,
                capacity: FRAME_CAPACITY,
            }
        );
        let bus = bus();
        let mut writer = bus.writer();
        assert_eq!(
            writer.publish_bytes(&too_big),
            Err(FrameError::TooLarge {
                bytes: FRAME_CAPACITY + 1,
                capacity: FRAME_CAPACITY,
            })
        );
        assert!(PlainFrame::new(&too_big).is_err());
        // A frame of exactly the capacity fits.
        let exact = [1u8; FRAME_CAPACITY];
        assert!(Frame::new(&exact).is_ok());
        writer.publish_bytes(&exact).expect("fits");
    }

    #[test]
    fn many_readers_see_the_same_frame() {
        let bus = bus();
        let mut writer = bus.writer();
        let mut first = bus.reader();
        let mut second = bus.reader();
        writer.publish_bytes(&[7u8; 32]).expect("fits");
        assert_eq!(first.read().expect("stable").as_bytes(), &[7u8; 32][..]);
        assert_eq!(second.read().expect("stable").as_bytes(), &[7u8; 32][..]);
        // A reader that was made before the publish sees the new frame.
        writer.publish_bytes(&[9u8; 32]).expect("fits");
        assert_eq!(first.read().expect("stable").as_bytes(), &[9u8; 32][..]);
        assert_eq!(second.read().expect("stable").as_bytes(), &[9u8; 32][..]);
    }

    #[test]
    fn a_frame_keeps_its_bytes_exactly() {
        let bus = bus();
        let mut writer = bus.writer();
        let mut reader = bus.reader();
        let mut all = [0u8; 256];
        for (i, byte) in all.iter_mut().enumerate() {
            *byte = u8::try_from(i).unwrap_or(0);
        }
        writer.publish_bytes(&all).expect("fits");
        let frame = reader.read().expect("a stable frame");
        assert_eq!(frame.as_bytes(), all.as_slice());
        assert_eq!(frame.len(), 256);
        // A shorter frame leaves no bytes of the long one behind.
        writer.publish_bytes(&[1, 2, 3]).expect("fits");
        let short = reader.read().expect("a stable frame");
        assert_eq!(short.as_bytes(), &[1, 2, 3][..]);
        assert_eq!(short.len(), 3);
    }
}
