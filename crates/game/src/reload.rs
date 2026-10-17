//! Hot-reload gate: staged generations applied at frame boundaries.
//!
//! Consumer: the window session polls its watcher on a presented-frame
//! cadence and settles through [`settle_paths`] (`bin/arena.rs`). Units:
//! frames/ticks as indices, generations as opaque `u64`; drain order is note
//! order, no clock. Errors: [`ReloadReason`]; the old generation stays live.

/// Staged paths past this shed the oldest first (see [`HotReload::dropped`]).
pub const MAX_PENDING: usize = 16;

/// Why a staged path did not apply; the live generation is untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadReason {
    /// The cook refused the path; the message names the cause.
    Cook(String),
}

/// One generation applied at a frame boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedReload {
    /// Path as passed to [`HotReload::note_changed`].
    pub path: String,
    /// Opaque generation from the cook (a byte hash in the window).
    pub generation: u64,
    /// Presented-frame index that applied it.
    pub frame: u32,
    /// Tick index stamped at apply; settle never advances it.
    pub tick: u32,
}

/// What one [`HotReload::settle`] call did, in drain order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettleReport {
    /// Applied generations, in note order.
    pub applied: Vec<AppliedReload>,
    /// Kept-last-good paths with their cook reasons, in note order.
    pub failed: Vec<(String, ReloadReason)>,
}

/// Staged-change gate: notes land any time, generations apply only when the
/// caller settles at a frame boundary. Pending never outgrows [`MAX_PENDING`].
#[derive(Clone, Debug, Default)]
pub struct HotReload {
    pending: Vec<String>,
    live: Vec<AppliedReload>,
    dropped: u64,
}

impl HotReload {
    /// Empty gate: nothing staged, nothing live.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage one changed path for the next settle. Empty paths refuse
    /// (counted as dropped); repeats dedup; past [`MAX_PENDING`] the
    /// oldest sheds first (counted).
    pub fn note_changed(&mut self, path: &str) {
        if path.is_empty() {
            self.dropped += 1;
        } else if !self.pending.iter().any(|p| p == path) {
            if self.pending.len() >= MAX_PENDING {
                self.pending.remove(0);
                self.dropped += 1;
            }
            self.pending.push(path.to_string());
        }
    }

    /// Drain the staged set in note order through `cook`: `Ok` applies at
    /// (`frame`, `tick`); `Err` keeps the old generation and records why.
    /// Failures never retry alone: a later change re-stages them.
    pub fn settle(
        &mut self,
        frame: u32,
        tick: u32,
        mut cook: impl FnMut(&str) -> Result<u64, ReloadReason>,
    ) -> SettleReport {
        let mut report = SettleReport::default();
        for path in core::mem::take(&mut self.pending) {
            match cook(&path) {
                Ok(generation) => {
                    let applied = AppliedReload {
                        path: path.clone(),
                        generation,
                        frame,
                        tick,
                    };
                    self.live.retain(|a| a.path != path);
                    self.live.push(applied.clone());
                    report.applied.push(applied);
                }
                Err(reason) => {
                    report.failed.push((path, reason));
                }
            }
        }
        report
    }

    /// Live generation for `path`, if one applied.
    #[must_use]
    pub fn generation(&self, path: &str) -> Option<u64> {
        self.live
            .iter()
            .find(|a| a.path == path)
            .map(|a| a.generation)
    }

    /// Shed plus refused-empty notes, lifetime of the gate.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl SettleReport {
    /// Applied paths that name the designer combat rows, in note order. Only
    /// these may stage the practice swap: a refused rows file never does.
    pub fn tuning_paths(&self) -> impl Iterator<Item = &str> {
        self.applied
            .iter()
            .map(|applied| applied.path.as_str())
            .filter(|path| crate::practice::is_design_combat_rows(path))
    }
}

/// Stage `paths` and settle them through [`cook_file`] at (`frame`, `tick`).
/// The window's whole poll step past the watcher: a refused file keeps its
/// last-good generation, an accepted one lands stamped with both indices.
pub fn settle_paths<'a>(
    gate: &mut HotReload,
    paths: impl IntoIterator<Item = &'a str>,
    frame: u32,
    tick: u32,
) -> SettleReport {
    for path in paths {
        gate.note_changed(path);
    }
    gate.settle(frame, tick, cook_file)
}

/// Largest file the cook reads; anything bigger refuses.
pub const MAX_COOK_BYTES: usize = 8 * 1024 * 1024;
/// Deepest JSON nesting the shape check walks; bounds its recursion.
const JSON_MAX_DEPTH: u32 = 64;

/// Read `path` and cook it: `Ok` is the byte hash, `Err` names the path and
/// why (unreadable, removed, or refused by [`cook_bytes`]).
pub fn cook_file(path: &str) -> Result<u64, ReloadReason> {
    let bytes = std::fs::read(path).map_err(|err| ReloadReason::Cook(format!("{path}: {err}")))?;
    cook_bytes(path, &bytes)
}

/// Check one file's bytes by kind and hash them. Every kind refuses an empty
/// or oversize file (a half-written save); `.json` must be well-formed JSON,
/// `.wgsl` must be UTF-8 with balanced delimiters, and the designer combat
/// rows must parse as combat tables. Other kinds hash as they are. Pure: the
/// same bytes give the same hash or the same refusal.
pub fn cook_bytes(path: &str, bytes: &[u8]) -> Result<u64, ReloadReason> {
    let refuse = |why: &dyn core::fmt::Display| ReloadReason::Cook(format!("{path}: {why}"));
    if bytes.is_empty() {
        return Err(refuse(&"empty file"));
    }
    if bytes.len() > MAX_COOK_BYTES {
        return Err(refuse(&"file over the cook size cap"));
    }
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str);
    let checked = match ext {
        Some("json") => text_of(bytes).and_then(check_json),
        Some("wgsl") => text_of(bytes).and_then(check_wgsl),
        Some("rows") if crate::practice::is_design_combat_rows(path) => {
            return text_of(bytes)
                .map_err(|why| refuse(&why))
                .and_then(crate::practice::cook_combat_rows);
        }
        _ => Ok(()),
    };
    checked.map_err(|why| refuse(&why))?;
    Ok(engine_core::hash_bytes(bytes))
}

/// Why a shape check refused: what was wrong and the byte it was found at.
#[derive(Debug)]
struct ShapeError {
    what: &'static str,
    at: usize,
}

impl core::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} at byte {}", self.what, self.at)
    }
}

fn text_of(bytes: &[u8]) -> Result<&str, ShapeError> {
    core::str::from_utf8(bytes).map_err(|err| ShapeError {
        what: "not utf-8",
        at: err.valid_up_to(),
    })
}

/// WGSL shape: every `{ ( [` closes in order outside `//` and nested `/* */`
/// comments. Catches a truncated or half-saved shader; it is not a compile.
fn check_wgsl(text: &str) -> Result<(), ShapeError> {
    let bytes = text.as_bytes();
    let mut open: Vec<u8> = Vec::new();
    let mut at = 0;
    let mut block = 0u32;
    while let Some(&byte) = bytes.get(at) {
        let next = bytes.get(at + 1).copied();
        if block > 0 {
            match (byte, next) {
                (b'*', Some(b'/')) => {
                    block -= 1;
                    at += 1;
                }
                (b'/', Some(b'*')) => {
                    block += 1;
                    at += 1;
                }
                _ => {}
            }
        } else {
            match (byte, next) {
                (b'/', Some(b'/')) => {
                    while bytes.get(at).is_some_and(|&b| b != b'\n') {
                        at += 1;
                    }
                    continue;
                }
                (b'/', Some(b'*')) => {
                    block = 1;
                    at += 1;
                }
                (b'{', _) => open.push(b'}'),
                (b'(', _) => open.push(b')'),
                (b'[', _) => open.push(b']'),
                (b'}' | b')' | b']', _) => {
                    if open.pop() != Some(byte) {
                        return Err(ShapeError {
                            what: "unbalanced closer",
                            at,
                        });
                    }
                }
                _ => {}
            }
        }
        at += 1;
    }
    let what = if block > 0 {
        "unterminated block comment"
    } else if !open.is_empty() {
        "missing closer before the end"
    } else {
        return Ok(());
    };
    Err(ShapeError { what, at })
}

/// Strict JSON well-formedness: exactly one value and nothing after it.
fn check_json(text: &str) -> Result<(), ShapeError> {
    let mut scan = JsonScan {
        bytes: text.as_bytes(),
        at: 0,
    };
    scan.space();
    scan.value(0)?;
    scan.space();
    if scan.at == scan.bytes.len() {
        Ok(())
    } else {
        Err(scan.fail("trailing bytes after the value"))
    }
}

struct JsonScan<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl JsonScan<'_> {
    fn fail(&self, what: &'static str) -> ShapeError {
        ShapeError { what, at: self.at }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, want: u8) -> bool {
        let hit = self.peek() == Some(want);
        if hit {
            self.at += 1;
        }
        hit
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn word(&mut self, word: &[u8]) -> Result<(), ShapeError> {
        if self
            .bytes
            .get(self.at..)
            .is_some_and(|rest| rest.starts_with(word))
        {
            self.at += word.len();
            Ok(())
        } else {
            Err(self.fail("bad literal"))
        }
    }

    fn value(&mut self, depth: u32) -> Result<(), ShapeError> {
        if depth > JSON_MAX_DEPTH {
            return Err(self.fail("nested too deep"));
        }
        match self.peek() {
            Some(b'{') => self.container(b'}', depth, true),
            Some(b'[') => self.container(b']', depth, false),
            Some(b'"') => self.string(),
            Some(b't') => self.word(b"true"),
            Some(b'f') => self.word(b"false"),
            Some(b'n') => self.word(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.fail("expected a value")),
        }
    }

    fn container(&mut self, close: u8, depth: u32, keyed: bool) -> Result<(), ShapeError> {
        self.at += 1;
        self.space();
        if self.eat(close) {
            return Ok(());
        }
        loop {
            self.space();
            if keyed {
                if self.peek() != Some(b'"') {
                    return Err(self.fail("expected a key"));
                }
                self.string()?;
                self.space();
                if !self.eat(b':') {
                    return Err(self.fail("expected `:`"));
                }
                self.space();
            }
            self.value(depth + 1)?;
            self.space();
            if self.eat(b',') {
                continue;
            }
            if self.eat(close) {
                return Ok(());
            }
            return Err(self.fail("expected `,` or the closing bracket"));
        }
    }

    fn string(&mut self) -> Result<(), ShapeError> {
        self.at += 1;
        loop {
            match self.peek() {
                None => return Err(self.fail("unterminated string")),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(());
                }
                Some(b'\\') => {
                    self.at += 1;
                    match self.peek() {
                        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                            self.at += 1;
                        }
                        Some(b'u') => {
                            self.at += 1;
                            for _ in 0..4 {
                                if !self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
                                    return Err(self.fail("bad \\u escape"));
                                }
                                self.at += 1;
                            }
                        }
                        _ => return Err(self.fail("bad escape")),
                    }
                }
                Some(byte) if byte < 0x20 => return Err(self.fail("control byte in string")),
                Some(_) => self.at += 1,
            }
        }
    }

    fn number(&mut self) -> Result<(), ShapeError> {
        self.eat(b'-');
        let int_start = self.at;
        let int_digits = self.digits();
        if int_digits == 0 || (int_digits > 1 && self.bytes.get(int_start) == Some(&b'0')) {
            return Err(self.fail("bad number"));
        }
        if self.eat(b'.') && self.digits() == 0 {
            return Err(self.fail("bad fraction"));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(self.fail("bad exponent"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_gen(gen: u64) -> impl FnMut(&str) -> Result<u64, ReloadReason> {
        move |_| Ok(gen)
    }

    #[test]
    fn reload_keeps_last_good() {
        let mut gate = HotReload::new();
        gate.note_changed("content/kit.json");
        let first = gate.settle(10, 100, ok_gen(7));
        assert_eq!(first.applied.len(), 1);
        assert_eq!(gate.generation("content/kit.json"), Some(7));
        // A bad cook leaves the old asset live and records its reason.
        gate.note_changed("content/kit.json");
        let bad = gate.settle(11, 101, |_| Err(ReloadReason::Cook("disk busy".into())));
        assert_eq!(gate.generation("content/kit.json"), Some(7));
        assert_eq!(bad.failed.len(), 1);
        assert_eq!(bad.failed[0].1, ReloadReason::Cook("disk busy".into()));
        // A good edit applies on the frame boundary with the tick stamped.
        gate.note_changed("content/kit.json");
        let good = gate.settle(12, 102, ok_gen(8));
        assert_eq!(gate.generation("content/kit.json"), Some(8));
        assert_eq!(good.applied[0].frame, 12);
        assert_eq!(good.applied[0].tick, 102);
        // Bounds: repeats dedup, overflow sheds oldest first, drain keeps note order.
        gate.note_changed("b");
        gate.note_changed("b");
        for n in 0..MAX_PENDING + 1 {
            gate.note_changed(&format!("s{n}"));
        }
        assert!(gate.dropped() > 0);
        let drain = gate.settle(13, 103, ok_gen(2));
        assert_eq!(drain.applied[0].path, "s1");
        assert_eq!(drain.applied.len(), MAX_PENDING);
    }

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    const GOOD_JSON: &str = r#"{"name":"kit","rows":[1,2.5,-3e2,true,null]}"#;
    const CONTENT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content");
    const SHADERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../render/src/shaders");

    static SCRATCH: AtomicU32 = AtomicU32::new(0);

    /// Fresh scratch dir per test (unit tests run in parallel threads).
    fn scratch(name: &str) -> PathBuf {
        let n = SCRATCH.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "engine-game-eg29-{name}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// Write `text` to `dir/file`; the path comes back as the window sees it.
    fn put(dir: &Path, file: &str, text: &str) -> String {
        let path = dir.join(file);
        std::fs::write(&path, text).expect("scratch write");
        path.to_string_lossy().into_owned()
    }

    fn shipped_rows() -> String {
        std::fs::read_to_string(format!("{CONTENT}/design/combat.rows")).expect("designer rows")
    }

    #[test]
    fn reload_keeps_last_good_after_a_bad_file_on_disk() {
        let dir = scratch("disk");
        let path = put(&dir, "kit.json", GOOD_JSON);
        let mut gate = HotReload::new();
        let first = settle_paths(&mut gate, [path.as_str()], 10, 100);
        assert_eq!(first.applied.len(), 1);
        let live = gate.generation(&path).expect("a good file applies");
        // A half-saved edit refuses: the old generation stays, the reason names the file.
        put(&dir, "kit.json", r#"{"name":"kit","rows":[1,2"#);
        let bad = settle_paths(&mut gate, [path.as_str()], 11, 101);
        assert!(bad.applied.is_empty());
        let why = format!("{:?}", bad.failed[0].1);
        assert!(why.contains("kit.json") && why.contains("byte"), "{why}");
        assert_eq!(gate.generation(&path), Some(live));
        // A vanished file keeps it too.
        std::fs::remove_file(&path).expect("remove");
        let gone = settle_paths(&mut gate, [path.as_str()], 12, 102);
        assert_eq!(gone.failed.len(), 1);
        assert_eq!(gate.generation(&path), Some(live));
        // The fixed save lands on the frame boundary with the tick as given.
        put(&dir, "kit.json", r#"{"name":"kit2","rows":[]}"#);
        let good = settle_paths(&mut gate, [path.as_str()], 13, 103);
        assert_eq!((good.applied[0].frame, good.applied[0].tick), (13, 103));
        assert_ne!(gate.generation(&path), Some(live));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_keeps_last_good_refuses_each_malformed_kind() {
        let rows = shipped_rows();
        let wrong_header = rows.replacen("table combat version 2", "table combat version 9", 1);
        let deep = format!("{}{}", "[".repeat(70), "]".repeat(70));
        let bad: [(&str, &[u8]); 16] = [
            ("a.json", b""),
            ("a.json", b"[1,]"),
            ("a.json", br#"{"a":1,}"#),
            ("a.json", br#"{"a" 1}"#),
            ("a.json", b"01"),
            ("a.json", b"{} x"),
            ("a.json", br#"["\q"]"#),
            ("a.json", &[b'"', 0xff, b'"']),
            ("a.json", deep.as_bytes()),
            ("a.wgsl", b"fn main() {"),
            ("a.wgsl", b"fn main() { ) }"),
            ("a.wgsl", b"/* open\nfn main() {}"),
            ("a.wgsl", &[0xff, 0xfe]),
            ("combat.rows", b""),
            ("combat.rows", wrong_header.as_bytes()),
            ("note.md", b""),
        ];
        for (path, bytes) in bad {
            assert!(cook_bytes(path, bytes).is_err(), "{path} {bytes:?} cooked");
        }
        let big = vec![b'x'; MAX_COOK_BYTES + 1];
        assert!(cook_bytes("note.md", &big).is_err());
        // Good twins cook, and the hash follows the bytes.
        let shader = "// } stray\n/* a /* nested */ { */\nfn main() { let a = array(1, 2); }";
        let wgsl = cook_bytes("a.wgsl", shader.as_bytes()).expect("balanced shader");
        assert_eq!(wgsl, engine_core::hash_bytes(shader.as_bytes()));
        assert_eq!(
            cook_bytes("a.json", b" [1, {\"k\": \"\\u00e9\\n\"}] "),
            Ok(engine_core::hash_bytes(b" [1, {\"k\": \"\\u00e9\\n\"}] "))
        );
        assert!(cook_bytes("combat.rows", rows.as_bytes()).is_ok());
        assert!(cook_bytes("note.md", b"# any text").is_ok());
    }

    #[test]
    fn reload_keeps_last_good_cooks_every_shipped_watched_file() {
        let mut cooked = 0;
        for dir in [CONTENT, SHADERS] {
            for entry in std::fs::read_dir(dir).expect("watched root").flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let path = path.to_string_lossy().into_owned();
                if let Err(why) = cook_file(&path) {
                    panic!("shipped file refused: {why:?}");
                }
                cooked += 1;
            }
        }
        assert!(cooked > 10, "watched roots held {cooked} files");
        assert!(cook_file(&format!("{CONTENT}/design/combat.rows")).is_ok());
    }

    #[test]
    fn reload_keeps_last_good_rows_stage_practice_only_when_accepted() {
        let dir = scratch("rows");
        let rows = shipped_rows();
        let path = put(&dir, "combat.rows", &rows);
        let mut gate = HotReload::new();
        let ok = settle_paths(&mut gate, [path.as_str()], 1, 5);
        assert_eq!(ok.tuning_paths().collect::<Vec<_>>(), [path.as_str()]);
        let live = gate.generation(&path);
        assert!(live.is_some());
        // A refused rows file keeps the live rows and stages nothing.
        let wrong = rows.replacen("table combat version 2", "table combat version 9", 1);
        put(&dir, "combat.rows", &wrong);
        let bad = settle_paths(&mut gate, [path.as_str()], 2, 6);
        assert_eq!(bad.tuning_paths().count(), 0);
        assert_eq!(bad.failed.len(), 1);
        assert_eq!(gate.generation(&path), live);
        // Other files never stage tuning.
        let other = put(&dir, "kit.json", GOOD_JSON);
        let kit = settle_paths(&mut gate, [other.as_str()], 3, 7);
        assert_eq!((kit.applied.len(), kit.tuning_paths().count()), (1, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "app")]
    #[test]
    fn reload_keeps_last_good_through_the_watcher() {
        use engine_assets::watcher::{WatchSettings, Watcher};
        let dir = scratch("watch");
        let path = put(&dir, "kit.json", GOOD_JSON);
        let mut watcher = Watcher::new();
        let first = WatchSettings {
            report_existing: true,
        };
        watcher.watch(&path, first);
        let mut gate = HotReload::new();
        // One window poll step: watcher events in, settle at (frame, tick).
        let mut step = |frame: u32, tick: u32| {
            let events = watcher.poll();
            let paths = events.iter().map(|event| event.path.as_str());
            let report = settle_paths(&mut gate, paths, frame, tick);
            (events.len(), report, gate.generation(&path))
        };
        // Each edit has its own length, so the signature moves on a coarse clock.
        let (seen, applied, live) = step(0, 0);
        assert_eq!((seen, applied.applied.len()), (1, 1));
        assert!(live.is_some());
        assert_eq!(step(30, 70).0, 0, "no edit, no event");
        put(&dir, "kit.json", r#"{"name":"kit""#);
        let (seen, bad, kept) = step(60, 140);
        assert_eq!((seen, bad.applied.len(), bad.failed.len()), (1, 0, 1));
        assert_eq!(kept, live, "the bad edit keeps the live generation");
        put(&dir, "kit.json", r#"{"name":"kit2","rows":[1]}"#);
        let (seen, good, next) = step(90, 210);
        assert_eq!(seen, 1);
        assert_eq!((good.applied[0].frame, good.applied[0].tick), (90, 210));
        assert_ne!(next, live);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
