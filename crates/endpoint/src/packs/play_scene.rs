//! Seat requests (rows SP62, EG35): ask the live window one question and read
//! its one-line answer.
//!
//! The seat wire carries `input` lines out and `tick` and `view` lines in. The
//! scene tools add two request heads the window answers on its next presented
//! frame: `scene [pixels]` is answered by one `scene <json>` line and `steer
//! <op> ..` by one `steer <json>` line (`engine_game::scene_desk` owns the
//! grammar and the reply shapes). [`seat_ask`] sends one request on the
//! session's seat and returns the answer's JSON text; the tick and view lines
//! that arrive while it waits are folded into the seat's bookkeeping, so a
//! later `seat.step` still knows which tick the window is on.
//!
//! Units: ticks, seconds. Determinism: the door adds nothing to the window's
//! bytes. Errors: [`McpToolError`] `no_seat`, `seat_closed` and `seat_timeout`.

use std::io::{BufRead, Write};
use std::time::Duration;

use super::{lock, no_seat, parse_tick_line, SeatWire, WireError, SEAT};
use crate::mcp::McpToolError;

/// Longest wait for the window's answer to one request. A window draws a frame
/// in a few milliseconds; the bound covers a cold start that is still staging
/// its content, and names a minimized window (which draws nothing) when it runs
/// out.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Most raw lines one request reads before it gives up: a seat the door did
/// not read for a minute holds thousands of tick lines ahead of the answer.
const REQUEST_MAX_LINES: usize = 200_000;

impl SeatWire {
    /// Send `line` and read until a line starting with `prefix` arrives;
    /// return what follows the prefix and the newest tick line seen on the
    /// way. `view` lines are kept like `wait_tick` keeps them.
    pub(super) fn request(
        &mut self,
        line: &str,
        prefix: &str,
    ) -> Result<(String, Option<u32>), WireError> {
        self.stream
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| self.stream.flush())
            .map_err(|_| WireError::Closed)?;
        let was = self.stream.read_timeout().ok().flatten();
        let _ = self.stream.set_read_timeout(Some(REQUEST_TIMEOUT));
        let answer = self.read_answer(prefix);
        let _ = self.stream.set_read_timeout(was);
        answer
    }

    /// The reading half of [`Self::request`].
    fn read_answer(&mut self, prefix: &str) -> Result<(String, Option<u32>), WireError> {
        let mut seen = None;
        let mut text = String::new();
        for _ in 0..REQUEST_MAX_LINES {
            text.clear();
            match self.reader.read_line(&mut text) {
                Ok(0) => return Err(WireError::Closed),
                Ok(_) => {}
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(WireError::Timeout);
                }
                Err(_) => return Err(WireError::Closed),
            }
            if let Some(answer) = text.strip_prefix(prefix) {
                return Ok((answer.trim_end().to_owned(), seen));
            }
            if let Some(view) = text.strip_prefix("view ") {
                self.view = Some(view.trim_end().to_owned());
            } else if let Some((tick, _)) = text.strip_prefix("tick ").and_then(parse_tick_line) {
                seen = Some(tick);
            }
        }
        Err(WireError::Timeout)
    }
}

/// True while the session holds a live seat.
#[must_use]
pub fn seat_is_open() -> bool {
    lock(&SEAT).is_some()
}

/// Send one request line on the session's seat and return the JSON text of the
/// window's answer (`prefix` is `scene ` or `steer `).
///
/// # Errors
///
/// `no_seat` without a seat, `seat_closed` when the window hung up (the seat is
/// then dead and `seat.close` takes it), `seat_timeout` when the window drew no
/// frame in time.
pub fn seat_ask(line: &str, prefix: &str) -> Result<String, McpToolError> {
    let mut slot = lock(&SEAT);
    let Some(host) = slot.as_mut() else {
        return Err(no_seat());
    };
    match host.wire.request(line, prefix) {
        Ok((answer, seen)) => {
            if let Some(tick) = seen {
                host.next_tick = host.next_tick.max(tick.saturating_add(1));
            }
            Ok(answer)
        }
        Err(WireError::Timeout) => Err(McpToolError::new(
            "seat_timeout",
            format!(
                "no answer to `{}` within {} s; a minimized window draws no frames, so restore it \
                 or re-run seat.open",
                line.split_whitespace().next().unwrap_or("request"),
                REQUEST_TIMEOUT.as_secs()
            ),
        )),
        Err(_) => {
            host.kill();
            Err(McpToolError::new(
                "seat_closed",
                "the arena closed the seat before it answered; call seat.close, then re-run seat.open",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;

    /// A window that answers the first request with `answer` after a few tick
    /// and view lines, and returns the lines it read.
    fn fake_window(
        reply_prefix: &'static str,
        answer: String,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("request");
            writeln!(writer, "tick 7 00000000000000a7").expect("tick");
            writeln!(writer, "view arena view: frames=7").expect("view");
            writeln!(writer, "tick 8 00000000000000a8").expect("tick");
            writeln!(writer, "{reply_prefix}{answer}").expect("answer");
            vec![line.trim_end().to_owned()]
        });
        (port, handle)
    }

    #[test]
    fn a_request_reads_past_tick_and_view_lines_to_its_answer() {
        let (port, window) = fake_window("scene ", "{\"schema\":1}".to_owned());
        let mut wire = SeatWire::connect(port).expect("wire connects");
        let (answer, seen) = wire.request("scene", "scene ").expect("answered");
        assert_eq!(answer, "{\"schema\":1}");
        assert_eq!(seen, Some(8), "the newest tick line on the way");
        assert_eq!(wire.view.as_deref(), Some("arena view: frames=7"));
        assert_eq!(window.join().expect("window"), vec!["scene"]);
    }

    #[test]
    fn a_large_answer_arrives_whole() {
        let big = format!("{{\"pad\":\"{}\"}}", "x".repeat(150_000));
        let (port, window) = fake_window("steer ", big.clone());
        let mut wire = SeatWire::connect(port).expect("wire connects");
        let (answer, _) = wire.request("steer state", "steer ").expect("answered");
        assert_eq!(answer, big);
        assert_eq!(window.join().expect("window"), vec!["steer state"]);
    }

    #[test]
    fn a_window_that_hangs_up_is_a_closed_seat_not_an_answer() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let window = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            drop(stream);
        });
        let mut wire = SeatWire::connect(port).expect("wire connects");
        window.join().expect("window");
        assert!(matches!(
            wire.request("scene", "scene "),
            Err(WireError::Closed)
        ));
    }

    #[test]
    fn asking_without_a_seat_refuses_no_seat() {
        // The process-wide slot may hold another test's host; only assert the
        // refusal shape when it is empty.
        if !seat_is_open() {
            let err = seat_ask("scene", "scene ").expect_err("no seat");
            assert_eq!(err.code, "no_seat");
        }
    }
}
