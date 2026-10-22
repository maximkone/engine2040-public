//! An agent seat for the live window: one loopback client pushes validated
//! fighter input for the human slot and reads back each tick's state hash.
//!
//! The seat is a tool surface, not a sim feature. It never runs on the tick
//! thread deeper than one mutex-guarded pop or push: a reader thread parses
//! the client's lines into a small queue, the window asks for the input of
//! the tick it is about to run, and a publish writes one short line back.
//! Inputs pass [`FighterInput::validate`] exactly as a tape does, so an
//! agent run is replayable and the fixed-tick hash cannot see who authored
//! it. Presentation only: nothing here is hashed, and a window without
//! `--agent-seat` never binds a socket.
//!
//! Protocol, one line each way, UTF-8, `\n` terminated:
//! - client: `input <tick> <mx> <mz> <facing> <buttons>` (the fields of
//!   [`FighterInput`]; `buttons` is the raw legal bit set, `0..=55`, the
//!   bits of `engine_input::ALL_BUTTONS` (`JUMP` is `0x20`);
//! - client: `ping` (ignored) or `quit` (closes the seat);
//! - window: `tick <tick> <hash:016x>` after every fixed tick, and
//!   `view <arena view: ...>` every [`VIEW_EVERY_TICKS`] ticks.
//!
//! Units: tick is the zero-based fixed tick the input applies before, wish
//! and facing are the sim's own integer units. Determinism: the seat only
//! authors [`FighterInput`] values; the same inputs from a tape, a client or
//! the keyboard fold the same hash.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{TcpListener, TcpStream};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex, PoisonError};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use engine_input::{Buttons, FighterId, FighterInput};

/// Inputs the queue holds before the oldest drops.
pub const MAX_PENDING: usize = 32;
/// Ticks between `view` lines on the seat wire.
pub const VIEW_EVERY_TICKS: u32 = 60;
/// Longest a publish may block on a client that stopped reading, in ms.
#[cfg(not(target_arch = "wasm32"))]
const WRITE_TIMEOUT_MS: u64 = 20;

/// Why a seat line was refused. The line is named so the client can fix it.
#[derive(Debug)]
pub enum SeatError {
    /// A line that is not the protocol.
    BadLine(String),
    /// The socket, the thread or the listener failed.
    Io(std::io::Error),
}

impl std::fmt::Display for SeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadLine(why) => write!(f, "seat line: {why}"),
            Self::Io(err) => write!(f, "seat io: {err}"),
        }
    }
}

impl std::error::Error for SeatError {}

impl From<std::io::Error> for SeatError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// One validated command from the client, without the seat's fighter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatCommand {
    /// Fixed tick the input applies before.
    pub tick: u32,
    /// Forward wish, `-100..=100`.
    pub mx: i8,
    /// Right wish, `-100..=100`.
    pub mz: i8,
    /// Absolute facing in centidegrees, `-18000..=18000`.
    pub facing: i16,
    /// Raw button bit set, legal bits only.
    pub buttons: u8,
}

/// What the seat saw: one line for the run receipt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeatStats {
    /// Clients that attached during the run.
    pub clients: u32,
    /// Accepted `input` lines.
    pub inputs: u32,
    /// Inputs whose tick had already passed when the window asked.
    pub late: u32,
    /// Inputs for a tick that another input already held.
    pub superseded: u32,
    /// Ticks the window asked for with no input waiting.
    pub held: u32,
    /// Lines refused by the parser or by [`FighterInput::validate`].
    pub invalid: u32,
    /// Writes or queued inputs dropped: no client, full queue or dead socket.
    pub dropped: u32,
}

impl SeatStats {
    /// The `arena seat:` receipt line.
    #[must_use]
    pub fn line(self, port: u16) -> String {
        format!(
            "arena seat: port={port} clients={} inputs={} late={} superseded={} held={} invalid={} dropped={}",
            self.clients,
            self.inputs,
            self.late,
            self.superseded,
            self.held,
            self.invalid,
            self.dropped
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Shared {
    pending: VecDeque<SeatCommand>,
    stats: SeatStats,
    writer: Option<TcpStream>,
}

/// The listening seat. Drop closes the listener; a client already attached
/// keeps its reader until it closes or the process exits.
#[cfg(not(target_arch = "wasm32"))]
pub struct Seat {
    port: u16,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Seat {
    /// Bind `127.0.0.1:port` (0 asks the OS for one) and serve it in a
    /// reader thread. A bind failure is loud: a requested seat never falls
    /// back to the keyboard silently.
    ///
    /// # Errors
    ///
    /// [`SeatError::Io`] when the listener or the thread cannot be made.
    pub fn bind(port: u16) -> Result<Self, SeatError> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let bound = listener.local_addr()?.port();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let reader = Arc::clone(&shared);
        let halt = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("agent-seat".to_string())
            .spawn(move || serve(listener, reader, halt))?;
        Ok(Self {
            port: bound,
            shared,
            stop,
        })
    }

    /// The port the OS actually bound (useful after asking for 0).
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The one input for `tick`, if the client sent it: the newest line for
    /// that tick wins, past ticks count late, future ticks wait. The seat
    /// owns the slot, so the returned input carries `fighter`.
    #[must_use]
    pub fn input_for_tick(&self, tick: u32, fighter: FighterId) -> Option<FighterInput> {
        let mut shared = lock(&self.shared);
        let mut picked = None;
        while let Some(next) = shared.pending.front().copied() {
            if next.tick < tick {
                shared.pending.pop_front();
                shared.stats.late = shared.stats.late.saturating_add(1);
                continue;
            }
            if next.tick > tick {
                break;
            }
            if picked.is_some() {
                shared.stats.superseded = shared.stats.superseded.saturating_add(1);
            }
            picked = shared.pending.pop_front();
        }
        let Some(command) = picked else {
            shared.stats.held = shared.stats.held.saturating_add(1);
            return None;
        };
        let input = FighterInput {
            tick: command.tick,
            fighter,
            mx: command.mx,
            mz: command.mz,
            facing: command.facing,
            buttons: Buttons(command.buttons),
        };
        if input.validate().is_err() {
            shared.stats.invalid = shared.stats.invalid.saturating_add(1);
            return None;
        }
        Some(input)
    }

    /// Send one line to the attached client. A missing or dead client drops
    /// the line and counts it; the tick never waits for a reader.
    pub fn publish(&self, line: &str) {
        let mut shared = lock(&self.shared);
        let Some(writer) = shared.writer.as_mut() else {
            shared.stats.dropped = shared.stats.dropped.saturating_add(1);
            return;
        };
        if writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.flush())
            .is_err()
        {
            shared.writer = None;
            shared.stats.dropped = shared.stats.dropped.saturating_add(1);
        }
    }

    /// Counters for the run receipt.
    #[must_use]
    pub fn stats(&self) -> SeatStats {
        lock(&self.shared).stats
    }

    /// The `arena seat:` receipt line.
    #[must_use]
    pub fn receipt(&self) -> String {
        self.stats().line(self.port)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Seat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop; the line is irrelevant, the close is not.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// The seat on the wasm target. A tab cannot listen on a loopback port, so
/// [`Seat::bind`] refuses at once and no value of this type ever exists.
#[cfg(target_arch = "wasm32")]
pub struct Seat(std::convert::Infallible);

#[cfg(target_arch = "wasm32")]
impl Seat {
    /// Always refused on wasm.
    ///
    /// # Errors
    ///
    /// [`SeatError::Io`] (unsupported) on the wasm target.
    pub fn bind(_port: u16) -> Result<Self, SeatError> {
        Err(SeatError::Io(std::io::ErrorKind::Unsupported.into()))
    }

    /// Unreachable on wasm: there is no seat to ask (see [`Seat::bind`]).
    #[must_use]
    pub fn port(&self) -> u16 {
        match self.0 {}
    }

    /// Unreachable on wasm: there is no seat to ask (see [`Seat::bind`]).
    #[must_use]
    pub fn input_for_tick(&self, _tick: u32, _fighter: FighterId) -> Option<FighterInput> {
        match self.0 {}
    }

    /// Unreachable on wasm: there is no seat to write to (see [`Seat::bind`]).
    pub fn publish(&self, _line: &str) {
        match self.0 {}
    }

    /// Unreachable on wasm: there is no seat to count (see [`Seat::bind`]).
    #[must_use]
    pub fn stats(&self) -> SeatStats {
        match self.0 {}
    }

    /// Unreachable on wasm: there is no seat to report (see [`Seat::bind`]).
    #[must_use]
    pub fn receipt(&self) -> String {
        match self.0 {}
    }
}

/// The reader thread: one client at a time, lines into the queue. A second
/// client waits in the listener backlog until the first closes.
#[cfg(not(target_arch = "wasm32"))]
fn serve(listener: TcpListener, shared: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let Ok((stream, _)) = listener.accept() else {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let Ok(reader) = stream.try_clone() else {
            continue;
        };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(WRITE_TIMEOUT_MS)));
        {
            let mut shared = lock(&shared);
            shared.stats.clients = shared.stats.clients.saturating_add(1);
            shared.writer = Some(stream);
        }
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => handle_line(&shared, &line),
            }
        }
        lock(&shared).writer = None;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn handle_line(shared: &Mutex<Shared>, line: &str) {
    match parse_input(line) {
        Ok(Some(command)) => {
            let mut shared = lock(shared);
            shared.stats.inputs = shared.stats.inputs.saturating_add(1);
            if shared.pending.len() >= MAX_PENDING {
                shared.pending.pop_front();
                shared.stats.dropped = shared.stats.dropped.saturating_add(1);
            }
            shared.pending.push_back(command);
        }
        Ok(None) => {}
        Err(_) => {
            let mut shared = lock(shared);
            shared.stats.invalid = shared.stats.invalid.saturating_add(1);
        }
    }
}

/// Parse one client line: `Some(command)` for an input, `None` for `ping`
/// and blank lines, a refusal for anything else. Pure.
///
/// # Errors
///
/// [`SeatError::BadLine`] names the field that is missing, unparsable or
/// out of range.
pub fn parse_input(line: &str) -> Result<Option<SeatCommand>, SeatError> {
    let mut parts = line.split_whitespace();
    let Some(head) = parts.next() else {
        return Ok(None);
    };
    match head {
        "ping" | "quit" => return Ok(None),
        "input" => {}
        other => return Err(SeatError::BadLine(format!("expected input, got {other}"))),
    }
    let tick = number::<u32>(parts.next(), "tick")?;
    let mx = bounded(parts.next(), "mx", -100, 100)?;
    let mz = bounded(parts.next(), "mz", -100, 100)?;
    let facing = bounded(parts.next(), "facing", -18000, 18000)?;
    let buttons = bounded(parts.next(), "buttons", 0, 255)?;
    if parts.next().is_some() {
        return Err(SeatError::BadLine("input has extra fields".to_string()));
    }
    let mx = i8::try_from(mx).map_err(|_| SeatError::BadLine("mx range".to_string()))?;
    let mz = i8::try_from(mz).map_err(|_| SeatError::BadLine("mz range".to_string()))?;
    let facing =
        i16::try_from(facing).map_err(|_| SeatError::BadLine("facing range".to_string()))?;
    let buttons =
        u8::try_from(buttons).map_err(|_| SeatError::BadLine("buttons range".to_string()))?;
    if !Buttons(buttons).is_legal() {
        return Err(SeatError::BadLine(format!("buttons {buttons:#04x}")));
    }
    Ok(Some(SeatCommand {
        tick,
        mx,
        mz,
        facing,
        buttons,
    }))
}

fn number<T: std::str::FromStr>(part: Option<&str>, name: &str) -> Result<T, SeatError> {
    let value = part.ok_or_else(|| SeatError::BadLine(format!("{name} is missing")))?;
    value
        .parse()
        .map_err(|_| SeatError::BadLine(format!("{name} {value}")))
}

fn bounded(part: Option<&str>, name: &str, low: i32, high: i32) -> Result<i32, SeatError> {
    let value: i32 = number(part, name)?;
    if (low..=high).contains(&value) {
        Ok(value)
    } else {
        Err(SeatError::BadLine(format!(
            "{name} {value} outside {low}..={high}"
        )))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn a_valid_line_parses_and_bad_lines_are_named() {
        let Some(command) = parse_input("input 12 -100 100 -18000 23").expect("valid line") else {
            panic!("a valid line is an input");
        };
        assert_eq!(
            command,
            SeatCommand {
                tick: 12,
                mx: -100,
                mz: 100,
                facing: -18000,
                buttons: 23,
            }
        );
        assert!(matches!(parse_input("ping"), Ok(None)));
        assert!(matches!(parse_input("   "), Ok(None)));
        for (line, why) in [
            ("inp 1 0 0 0 0", "expected input"),
            ("input 1 0 0 0", "buttons is missing"),
            ("input x 0 0 0 0", "tick x"),
            ("input 1 101 0 0 0", "mx 101 outside"),
            ("input 1 0 0 0 0 extra", "extra fields"),
            ("input 1 0 0 0 128", "buttons 0x80"),
        ] {
            let err = parse_input(line).expect_err(why);
            assert!(err.to_string().contains(why), "{line}: {err}");
        }
    }

    #[test]
    fn a_client_input_reaches_the_tick_and_the_hash_reaches_the_client() {
        let seat = Seat::bind(0).expect("seat binds");
        let port = seat.port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).expect("client connects");
        client
            .write_all(b"input 5 25 -10 9000 1\n")
            .expect("input sends");
        let fighter = FighterId::new(0).expect("fleet slot 0");
        let input = wait_for(|| seat.input_for_tick(5, fighter)).expect("tick 5 input arrives");
        assert_eq!(
            (
                input.tick,
                input.mx,
                input.mz,
                input.facing,
                input.buttons.0
            ),
            (5, 25, -10, 9000, 1)
        );
        assert!(
            seat.input_for_tick(5, fighter).is_none(),
            "one input, one tick"
        );
        // A future tick waits; a past one is late, never replayed.
        client
            .write_all(b"input 8 0 0 0 0\n")
            .expect("future input sends");
        std::thread::sleep(Duration::from_millis(20));
        assert!(seat.input_for_tick(7, fighter).is_none(), "tick 8 waits");
        seat.publish("tick 7 0123456789abcdef\n");
        let mut line = String::new();
        BufReader::new(&client)
            .read_line(&mut line)
            .expect("publish reaches the client");
        assert_eq!(line, "tick 7 0123456789abcdef\n");
        let stats = seat.stats();
        assert_eq!(stats.inputs, 2);
        assert_eq!(stats.clients, 1);
        assert!(stats.held >= 2, "unfed ticks count held: {stats:?}");
    }

    #[test]
    fn a_past_input_counts_late_and_the_receipt_names_every_counter() {
        let seat = Seat::bind(0).expect("seat binds");
        let mut client = TcpStream::connect(("127.0.0.1", seat.port())).expect("connect");
        client
            .write_all(b"input 2 0 0 0 0\ninput 2 1 0 0 0\n")
            .expect("inputs send");
        let fighter = FighterId::new(0).expect("fleet slot 0");
        let input = wait_for(|| seat.input_for_tick(2, fighter)).expect("tick 2 input");
        assert_eq!(input.mx, 1, "the newest line for a tick wins");
        client
            .write_all(b"input 1 0 0 0 0\n")
            .expect("late input sends");
        std::thread::sleep(Duration::from_millis(20));
        assert!(seat.input_for_tick(3, fighter).is_none());
        let receipt = seat.receipt();
        assert!(receipt.starts_with("arena seat: port="), "{receipt}");
        for field in ["clients=1", "inputs=3", "late=1", "superseded=1", "held=2"] {
            assert!(receipt.contains(field), "{field} in {receipt}");
        }
    }

    fn wait_for(mut probe: impl FnMut() -> Option<FighterInput>) -> Option<FighterInput> {
        for _ in 0..200 {
            if let Some(input) = probe() {
                return Some(input);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }
}
