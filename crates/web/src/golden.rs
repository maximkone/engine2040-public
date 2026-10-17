//! The engine, not only the GPU: the golden tape and the break corpus, run inside the page.
//!
//! On load the shell records the 600-tick, seed-2040 tape and runs the BX28 cut corpus through
//! `engine-proof`'s matrix collector, in the wasm module, on the browser's own machine. Three
//! numbers come out, and they must be the numbers every other machine gets (x86-64 Windows and
//! Linux, aarch64 Linux, wasm32 under Node): the golden hash, the state hash of the last tick,
//! and the corpus digest. The page prints one line saying so, or says MISMATCH loudly with both
//! sets of values.
//!
//! Pure over a clock closure, so it runs and tests natively; the shell passes
//! `performance.now()`. `Instant` and `SystemTime` trap on `wasm32-unknown-unknown`, so they
//! are not used here.

use engine_proof::{matrix, record, Config, GOLDEN_600};

/// State hash of the last tick of the default tape. Equal on every machine; the native test
/// below recomputes it, so a change in the simulation fails there first.
pub const EXPECTED_STATE: u64 = 0x2aca9338670aa0cb;

/// Digest of the full cut corpus (20 blocks). Same rule as [`EXPECTED_STATE`].
pub const EXPECTED_CORPUS: u64 = 0xb37fc6d66df2d417;

/// The three numbers one machine produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observed {
    /// Last legacy checkpoint: the frozen golden.
    pub golden: u64,
    /// State hash of the last tick.
    pub state: u64,
    /// Corpus digest.
    pub corpus: u64,
}

/// What every machine must produce.
pub const EXPECTED: Observed = Observed {
    golden: GOLDEN_600,
    state: EXPECTED_STATE,
    corpus: EXPECTED_CORPUS,
};

/// How a check ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// All three numbers equal the expected ones.
    Ok,
    /// The run finished with different numbers: the simulation is not the same engine here.
    Mismatch,
    /// The run could not finish.
    Error,
}

impl Status {
    /// The word the page puts in `data-golden`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Mismatch => "mismatch",
            Status::Error => "error",
        }
    }
}

/// The result: a status and the text lines for the page and the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub status: Status,
    /// The one `golden ...` line.
    pub line: String,
    /// The wasm cost per tick, when the tape ran.
    pub tick_line: Option<String>,
}

impl Outcome {
    /// Both lines, one per row, as the page shows them.
    #[must_use]
    pub fn text(&self) -> String {
        match &self.tick_line {
            Some(tick) => format!("{}\n{tick}", self.line),
            None => self.line.clone(),
        }
    }
}

/// The verdict line for numbers `seen` after `ms` milliseconds.
#[must_use]
pub fn judge(seen: Observed, ms: f64) -> (Status, String) {
    if seen == EXPECTED {
        let line = format!(
            "golden {:016x} state {:016x} corpus {:016x} OK {:.0} ms",
            seen.golden, seen.state, seen.corpus, ms
        );
        return (Status::Ok, line);
    }
    let line = format!(
        "golden MISMATCH got {:016x} state {:016x} corpus {:016x}, expected {:016x} state {:016x} corpus {:016x} ({:.0} ms)",
        seen.golden,
        seen.state,
        seen.corpus,
        EXPECTED.golden,
        EXPECTED.state,
        EXPECTED.corpus,
        ms
    );
    (Status::Mismatch, line)
}

/// The cost of the tape in this module: `ticks` fixed steps in `ms` milliseconds.
#[must_use]
pub fn tick_line(ticks: u32, ms: f64) -> String {
    let per_tick = if ticks == 0 {
        0.0
    } else {
        ms / f64::from(ticks)
    };
    format!("tape {ticks} ticks in {ms:.1} ms, {per_tick:.4} ms per tick")
}

fn failure(what: &str, error: &dyn std::fmt::Display) -> Outcome {
    Outcome {
        status: Status::Error,
        line: format!("golden ERROR {what}: {error}"),
        tick_line: None,
    }
}

/// Record the tape (timed alone, for the per-tick cost), then run the whole matrix row
/// (tape, replay, corpus) and compare it with [`EXPECTED`]. `now_ms` is the clock.
pub fn check(now_ms: &mut dyn FnMut() -> f64) -> Outcome {
    let start = now_ms();
    let tape = match record(Config::default(), &[]) {
        Ok(tape) => tape,
        Err(error) => return failure("tape", &error),
    };
    let tape_ms = now_ms() - start;

    let row_start = now_ms();
    let entry = match matrix::collect_local_entry() {
        Ok(entry) => entry,
        Err(error) => return failure("matrix row", &error),
    };
    let row_ms = now_ms() - row_start;

    let seen = Observed {
        golden: entry.golden_last,
        state: entry.checkpoints.last().map_or(0, |c| c.state),
        corpus: entry.cut_digest,
    };
    let (status, line) = judge(seen, row_ms);
    Outcome {
        status,
        line,
        tick_line: Some(tick_line(tape.config.ticks, tape_ms)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock that moves 5 ms per reading.
    fn fake_clock() -> impl FnMut() -> f64 {
        let mut t = 0.0;
        move || {
            t += 5.0;
            t
        }
    }

    #[test]
    fn this_machine_computes_the_pinned_values() {
        // The same run the page does. It fails first when the simulation changes: update the
        // constants (and the golden matrix) together, never one without the other.
        let out = check(&mut fake_clock());
        assert_eq!(out.status, Status::Ok, "{}", out.line);
        assert_eq!(
            out.line,
            "golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK 5 ms"
        );
        assert_eq!(
            out.tick_line.as_deref(),
            Some("tape 600 ticks in 5.0 ms, 0.0083 ms per tick")
        );
    }

    #[test]
    fn the_golden_constant_is_the_frozen_one() {
        assert_eq!(EXPECTED.golden, 0xd57b5c8ae3096614);
        assert_eq!(EXPECTED.state, 0x2aca9338670aa0cb);
        assert_eq!(EXPECTED.corpus, 0xb37fc6d66df2d417);
    }

    #[test]
    fn a_wrong_number_is_a_loud_mismatch_naming_both_values() {
        for broken in [
            Observed {
                golden: EXPECTED.golden ^ 1,
                ..EXPECTED
            },
            Observed {
                state: EXPECTED.state ^ 1,
                ..EXPECTED
            },
            Observed {
                corpus: EXPECTED.corpus ^ 1,
                ..EXPECTED
            },
        ] {
            let (status, line) = judge(broken, 12.4);
            assert_eq!(status, Status::Mismatch);
            assert!(line.starts_with("golden MISMATCH got "), "{line}");
            assert!(!line.contains(" OK "), "{line}");
            // Both the observed and the expected values are on the line.
            for want in [
                broken.golden,
                broken.state,
                broken.corpus,
                EXPECTED.golden,
                EXPECTED.state,
                EXPECTED.corpus,
            ] {
                assert!(
                    line.contains(&format!("{want:016x}")),
                    "{line} lacks {want:016x}"
                );
            }
        }
    }

    #[test]
    fn the_ok_line_has_the_shape_the_headless_check_reads() {
        let (status, line) = judge(EXPECTED, 412.6);
        assert_eq!(status, Status::Ok);
        assert_eq!(
            line,
            "golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK 413 ms"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn tick_line_divides_by_ticks_and_survives_zero() {
        assert_eq!(
            tick_line(600, 60.0),
            "tape 600 ticks in 60.0 ms, 0.1000 ms per tick"
        );
        assert_eq!(
            tick_line(0, 5.0),
            "tape 0 ticks in 5.0 ms, 0.0000 ms per tick"
        );
    }

    #[test]
    fn text_puts_the_tick_line_under_the_verdict() {
        let out = Outcome {
            status: Status::Ok,
            line: "golden x".into(),
            tick_line: Some("tape y".into()),
        };
        assert_eq!(out.text(), "golden x\ntape y");
        let err = failure("tape", &"boom");
        assert_eq!(err.status, Status::Error);
        assert_eq!(err.text(), "golden ERROR tape: boom");
        assert_eq!(Status::Ok.word(), "ok");
        assert_eq!(Status::Mismatch.word(), "mismatch");
        assert_eq!(Status::Error.word(), "error");
    }
}
