//! fight_report: one schema-pinned JSON report per fight tape (plan eyes E4).
//!
//! In: a seed-fixed tape's numbers, its captures (the E1 manifest entries) and
//! its spatial brief. Out: one JSON line plus its digest line, so the same
//! inputs always render the same bytes and a rerun compares bytes, never prose.
//! The verdict is [`BoutWinner`], combat's shipped vocabulary; this crate adds
//! none.
//!
//! Units: ticks and counts are integers, capture and brief sizes are bytes, and
//! a digest is the engine-core FNV over the bytes it names. Errors:
//! [`ReportError`], never a panic. Budgets: 64 captures, a 4096-byte brief, a
//! 96-byte capture name. Determinism: capture order is canonicalised away, so
//! the same set in any order is one report.

use std::fmt::Write as _;

use engine_combat::feel::BoutWinner;
use engine_core::hash_bytes;

/// Schema name every report carries.
pub const REPORT_SCHEMA: &str = "e2040-fight-report";
/// Schema version this build writes.
pub const REPORT_VERSION: u32 = 1;
/// Largest capture set one report holds.
pub const MAX_CAPTURES: usize = 64;
/// Largest brief one report names, in bytes.
pub const MAX_BRIEF_BYTES: usize = 4096;
/// Largest capture name, in bytes.
pub const MAX_CAPTURE_NAME_BYTES: usize = 96;

/// What one seed-fixed tape measured: the numbers, never prose. The caller
/// fills it from its own tape (the staged fight, the arena spine track or the
/// feel harness); this crate reads none of those private tallies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapeSummary {
    /// Seed the tape ran under.
    pub seed: u64,
    /// Steps the tape holds.
    pub ticks: u32,
    /// Rounds fought.
    pub rounds: u32,
    /// Strikes that landed.
    pub hits: u32,
    /// Knockdown events.
    pub knockdowns: u32,
    /// Digest of the tape's own steps, so a report names the bytes it measured.
    pub digest: u64,
}

/// One capture in a report: the E1 manifest entry (name, sha256, size).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CaptureRef {
    /// File name the capture was written under.
    pub name: String,
    /// SHA-256 of the PNG bytes.
    pub sha256: [u8; 32],
    /// Size of the PNG in bytes.
    pub bytes: u64,
}

/// The spatial brief the fight was read against: digest and size, never the
/// text itself (the brief stays with its owner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BriefRef {
    /// Digest of the brief text.
    pub digest: u64,
    /// Size of the brief in bytes.
    pub bytes: u64,
}

/// One schema-pinned report over a fight tape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FightReport {
    tape: TapeSummary,
    verdict: BoutWinner,
    brief: BriefRef,
    captures: Vec<CaptureRef>,
}

/// Why a report was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// More captures than one report holds.
    TooManyCaptures {
        /// How many were offered.
        count: usize,
        /// The bound.
        max: usize,
    },
    /// A capture name longer than the bound.
    CaptureNameTooLong {
        /// Name length in bytes.
        bytes: usize,
        /// The bound in bytes.
        max: usize,
    },
    /// A capture name with no bytes.
    EmptyCaptureName,
    /// One name, two different captures.
    DuplicateCapture {
        /// The name that appeared twice with different bytes.
        name: String,
    },
    /// A brief larger than the bound.
    BriefTooLarge {
        /// Brief size in bytes.
        bytes: usize,
        /// The bound in bytes.
        max: usize,
    },
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyCaptures { count, max } => {
                write!(f, "a report holds at most {max} captures; got {count}")
            }
            Self::CaptureNameTooLong { bytes, max } => {
                write!(f, "a capture name is at most {max} bytes; got {bytes}")
            }
            Self::EmptyCaptureName => write!(f, "a capture name cannot be empty"),
            Self::DuplicateCapture { name } => {
                write!(f, "two different captures are both named {name:?}")
            }
            Self::BriefTooLarge { bytes, max } => {
                write!(f, "a brief is at most {max} bytes; got {bytes}")
            }
        }
    }
}

impl std::error::Error for ReportError {}

impl FightReport {
    /// A report over one tape, canonicalised: captures sort by name and digest,
    /// a repeated name with the same bytes collapses, and a repeated name with
    /// different bytes is refused.
    ///
    /// Errors: [`ReportError`] for a capture set over the bound, an empty or
    /// oversized name, a drift under one name, or an oversized brief.
    pub fn of(
        tape: TapeSummary,
        verdict: BoutWinner,
        brief: BriefRef,
        mut captures: Vec<CaptureRef>,
    ) -> Result<Self, ReportError> {
        if captures.len() > MAX_CAPTURES {
            return Err(ReportError::TooManyCaptures {
                count: captures.len(),
                max: MAX_CAPTURES,
            });
        }
        let brief_bytes = usize::try_from(brief.bytes).unwrap_or(usize::MAX);
        if brief_bytes > MAX_BRIEF_BYTES {
            return Err(ReportError::BriefTooLarge {
                bytes: brief_bytes,
                max: MAX_BRIEF_BYTES,
            });
        }
        for capture in &captures {
            match capture.name.len() {
                0 => return Err(ReportError::EmptyCaptureName),
                len if len > MAX_CAPTURE_NAME_BYTES => {
                    return Err(ReportError::CaptureNameTooLong {
                        bytes: len,
                        max: MAX_CAPTURE_NAME_BYTES,
                    });
                }
                _ => {}
            }
        }
        captures.sort();
        let mut held: Vec<CaptureRef> = Vec::with_capacity(captures.len());
        for capture in captures {
            match held.last() {
                Some(last) if last.name == capture.name => {
                    if last.sha256 != capture.sha256 || last.bytes != capture.bytes {
                        return Err(ReportError::DuplicateCapture { name: capture.name });
                    }
                }
                _ => held.push(capture),
            }
        }
        Ok(Self {
            tape,
            verdict,
            brief,
            captures: held,
        })
    }

    /// The tape the report measures.
    #[must_use]
    pub fn tape(&self) -> TapeSummary {
        self.tape
    }

    /// The fight's verdict.
    #[must_use]
    pub fn verdict(&self) -> BoutWinner {
        self.verdict
    }

    /// The brief the fight was read against.
    #[must_use]
    pub fn brief(&self) -> BriefRef {
        self.brief
    }

    /// The captures, sorted by name.
    #[must_use]
    pub fn captures(&self) -> &[CaptureRef] {
        &self.captures
    }

    /// The report's JSON body: one line, key order pinned, schema and version
    /// first, captures last. This is line 1 of [`Self::render`].
    #[must_use]
    pub fn body(&self) -> String {
        let mut out = String::with_capacity(256);
        let _ = write!(
            out,
            "{{\"schema\":\"{REPORT_SCHEMA}\",\"version\":{REPORT_VERSION},\"seed\":{},\
             \"ticks\":{},\"rounds\":{},\"hits\":{},\"knockdowns\":{},\"tape_digest\":\"{:016x}\",\
             \"verdict\":\"{}\",\"brief_digest\":\"{:016x}\",\"brief_bytes\":{},\"captures\":[",
            self.tape.seed,
            self.tape.ticks,
            self.tape.rounds,
            self.tape.hits,
            self.tape.knockdowns,
            self.tape.digest,
            verdict_name(self.verdict),
            self.brief.digest,
            self.brief.bytes,
        );
        for (index, capture) in self.captures.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":\"");
            escape_json(&capture.name, &mut out);
            out.push_str("\",\"sha256\":\"");
            for byte in &capture.sha256 {
                let _ = write!(out, "{byte:02x}");
            }
            let _ = write!(out, "\",\"bytes\":{}}}", capture.bytes);
        }
        out.push_str("]}");
        out
    }

    /// The digest over the body's bytes: the value the digest line carries.
    #[must_use]
    pub fn digest(&self) -> u64 {
        hash_bytes(self.body().as_bytes())
    }

    /// The report as it is written: line 1 the body, line 2 `digest <016x>`
    /// over line 1's bytes. The same report twice gives the same bytes.
    #[must_use]
    pub fn render(&self) -> String {
        let body = self.body();
        format!("{body}\ndigest {:016x}\n", hash_bytes(body.as_bytes()))
    }

    /// Reads the digest line back: true exactly when line 1 still hashes to the
    /// value line 2 carries. A tampered body, a foreign digest line or extra
    /// lines are false, never a panic.
    #[must_use]
    pub fn verify(rendered: &str) -> bool {
        let mut lines = rendered.lines();
        let (Some(body), Some(digest_line)) = (lines.next(), lines.next()) else {
            return false;
        };
        if lines.next().is_some() {
            return false;
        }
        let Some(hex) = digest_line.strip_prefix("digest ") else {
            return false;
        };
        match u64::from_str_radix(hex, 16) {
            Ok(claimed) => hash_bytes(body.as_bytes()) == claimed,
            Err(_) => false,
        }
    }
}

/// The verdict as the report spells it.
fn verdict_name(verdict: BoutWinner) -> &'static str {
    match verdict {
        BoutWinner::A => "a",
        BoutWinner::B => "b",
        BoutWinner::Draw => "draw",
    }
}

/// Writes `text` as a JSON string body: quote and backslash escaped, control
/// characters as `\u00xx`, so any capture name stays one valid JSON line.
fn escape_json(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if ch.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(ch));
            }
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(seed: u64) -> TapeSummary {
        let mut rng = seed;
        rng = engine_core::splitmix64(rng);
        let rounds = u32::try_from(rng % 3 + 1).unwrap_or(1);
        rng = engine_core::splitmix64(rng);
        let hits = u32::try_from(rng % 40).unwrap_or(0);
        rng = engine_core::splitmix64(rng);
        TapeSummary {
            seed,
            ticks: 600,
            rounds,
            hits,
            knockdowns: hits / 8,
            digest: rng,
        }
    }

    fn capture(index: u8) -> CaptureRef {
        CaptureRef {
            name: format!("frame-{index:03}.png"),
            sha256: [index; 32],
            bytes: 4096 + u64::from(index),
        }
    }

    fn brief() -> BriefRef {
        BriefRef {
            digest: 0x5eed_1234_5678_9abc,
            bytes: 512,
        }
    }

    fn report(seed: u64, captures: Vec<CaptureRef>) -> FightReport {
        FightReport::of(tape(seed), BoutWinner::A, brief(), captures).expect("a valid report")
    }

    #[test]
    fn three_tapes_render_identical_bytes_twice() {
        let mut rendered = Vec::new();
        for seed in [7u64, 11, 13] {
            let report = report(
                seed,
                vec![
                    capture(1),
                    capture(2),
                    capture(u8::try_from(seed).unwrap_or(3)),
                ],
            );
            let a = report.render();
            let b = report.render();
            assert_eq!(a, b, "the same report twice is the same bytes");
            assert!(FightReport::verify(&a), "and its digest line reads back");
            assert!(a.starts_with(&format!(
                "{{\"schema\":\"{REPORT_SCHEMA}\",\"version\":{REPORT_VERSION},\"seed\":{seed},"
            )));
            assert!(a.contains("\"verdict\":\"a\""), "combat's own verdict word");
            rendered.push(a);
        }
        assert_ne!(rendered[0], rendered[1], "different tapes differ");
        assert_ne!(rendered[1], rendered[2]);
        assert_eq!(rendered[0].lines().count(), 2, "body plus digest line");
    }

    #[test]
    fn the_digest_line_reads_back() {
        let text = report(7, vec![capture(1)]).render();
        assert!(FightReport::verify(&text));
        // A tampered body no longer hashes to the line's value.
        let tampered = text.replacen("\"hits\":", "\"hits\":9", 1);
        assert!(!FightReport::verify(&tampered), "a tampered body fails");
        // Nor does a rewritten digest line.
        let lines: Vec<&str> = text.lines().collect();
        let wrong = format!("{}\ndigest 0000000000000000\n", lines[0]);
        assert!(!FightReport::verify(&wrong));
        // A foreign, truncated or extended render fails, never panics.
        assert!(!FightReport::verify(""));
        assert!(!FightReport::verify("{}\n"));
        assert!(!FightReport::verify(&format!("{text}extra\n")));
        assert_eq!(text.lines().next().unwrap(), lines[0]);
        let mut hasher = engine_core::Fnv64::new();
        hasher.write_str(lines[0]);
        assert_eq!(
            format!("digest {:016x}", hasher.finish()),
            lines[1],
            "the line is the body's digest"
        );
    }

    #[test]
    fn capture_order_is_canonicalised_away() {
        let forward = report(7, vec![capture(1), capture(2), capture(3)]);
        let backward = report(7, vec![capture(3), capture(2), capture(1)]);
        assert_eq!(forward.render(), backward.render());
        assert_eq!(
            forward
                .captures()
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
            vec![
                "frame-001.png".to_owned(),
                "frame-002.png".to_owned(),
                "frame-003.png".to_owned()
            ]
        );
        // One name twice with the same bytes is one capture.
        let doubled = report(7, vec![capture(1), capture(1), capture(2)]);
        assert_eq!(doubled.captures().len(), 2);
        assert_eq!(forward.captures()[..2], doubled.captures()[..2]);
        // One name twice with different bytes is refused.
        let mut other = capture(2);
        other.sha256 = [9; 32];
        let err = FightReport::of(tape(7), BoutWinner::Draw, brief(), vec![capture(2), other])
            .expect_err("a drift under one name is refused");
        assert_eq!(
            err,
            ReportError::DuplicateCapture {
                name: "frame-002.png".to_owned()
            }
        );
        assert!(err.to_string().contains("frame-002.png"));
    }

    #[test]
    fn the_report_refuses_unbounded_input() {
        let many: Vec<CaptureRef> = (0..=u8::try_from(MAX_CAPTURES).unwrap_or(64))
            .map(capture)
            .collect();
        assert_eq!(
            FightReport::of(tape(7), BoutWinner::Draw, brief(), many),
            Err(ReportError::TooManyCaptures {
                count: MAX_CAPTURES + 1,
                max: MAX_CAPTURES,
            })
        );
        let mut empty = capture(1);
        empty.name = String::new();
        assert_eq!(
            FightReport::of(tape(7), BoutWinner::Draw, brief(), vec![empty]),
            Err(ReportError::EmptyCaptureName)
        );
        let mut long = capture(1);
        long.name = "n".repeat(MAX_CAPTURE_NAME_BYTES + 1);
        assert_eq!(
            FightReport::of(tape(7), BoutWinner::Draw, brief(), vec![long]),
            Err(ReportError::CaptureNameTooLong {
                bytes: MAX_CAPTURE_NAME_BYTES + 1,
                max: MAX_CAPTURE_NAME_BYTES,
            })
        );
        let fat_brief = BriefRef {
            digest: 1,
            bytes: MAX_BRIEF_BYTES as u64 + 1,
        };
        assert_eq!(
            FightReport::of(tape(7), BoutWinner::Draw, fat_brief, Vec::new()),
            Err(ReportError::BriefTooLarge {
                bytes: MAX_BRIEF_BYTES + 1,
                max: MAX_BRIEF_BYTES,
            })
        );
        // The bound itself is legal, and a report with no captures is legal.
        let full = BriefRef {
            digest: 1,
            bytes: MAX_BRIEF_BYTES as u64,
        };
        let bare = FightReport::of(tape(7), BoutWinner::Draw, full, Vec::new()).expect("legal");
        assert!(bare.captures().is_empty());
        assert!(FightReport::verify(&bare.render()));
        assert!(bare.render().contains("\"captures\":[]"));
    }

    #[test]
    fn the_json_is_canonical_and_escapes_names() {
        let mut odd = capture(1);
        odd.name = "a \"quoted\"\\name\n\twith\u{7}controls".to_owned();
        let text = report(7, vec![odd]).render();
        let body = text.lines().next().unwrap();
        assert!(body.contains("a \\\"quoted\\\"\\\\name\\n\\twith\\u0007controls"));
        assert!(body.ends_with("]}"), "captures close the body");
        assert_eq!(body.matches("\"name\":").count(), 1);
        // Key order is pinned: schema, version, then the tape's numbers.
        let order = [
            "\"schema\"",
            "\"version\"",
            "\"seed\"",
            "\"ticks\"",
            "\"rounds\"",
            "\"hits\"",
            "\"knockdowns\"",
            "\"tape_digest\"",
            "\"verdict\"",
            "\"brief_digest\"",
            "\"brief_bytes\"",
            "\"captures\"",
        ];
        let mut at = 0usize;
        for key in order {
            let found = body[at..].find(key).map(|offset| at + offset);
            assert!(found.is_some(), "body carries {key}");
            at = found.unwrap() + key.len();
        }
        assert_eq!(verdict_name(BoutWinner::A), "a");
        assert_eq!(verdict_name(BoutWinner::B), "b");
        assert_eq!(verdict_name(BoutWinner::Draw), "draw");
        assert!(FightReport::verify(&text), "odd names stay one line");
    }
}
