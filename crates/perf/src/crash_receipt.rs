//! Crash receipts: one file per crashed run, printed once by the next start.
//!
//! Consumer: the windowed session driver (`crates/game/src/windowed.rs`), which
//! installs a [`CrashWatch`] panic hook and prints any pending receipt at
//! start. The mechanism lives here so it is headless and testable: a panic
//! hook is only a caller of [`CrashWatch::install`].
//!
//! A receipt is one simple text file under `target/reports/crash/` (override
//! with `E2040_CRASH_DIR`) holding four rows: the fixed tick, the state hash,
//! the last call the run made, and the backtrace text. File names are stable:
//! `crash-<tick>-000-<hash:016x>.txt`, then `001`, `002`, ... for a second
//! crash at the same tick and hash, so names sort in crash order. Writing is a
//! single `create_new` write, so one crash leaves exactly one file and never
//! overwrites an earlier receipt.
//!
//! The reader consumes: [`take_pending`] renames the file to `*.txt.seen`
//! before returning it, so a second start is silent and the bytes stay on disk
//! for a later look. Printing is one stderr write at start, never a per-tick
//! cost.
//!
//! Determinism: the hook reads best-effort snapshot values with relaxed loads
//! and never enters the fixed tick or a simulation hash; the recorded backtrace
//! and file IO are presentation only.

use std::fmt;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, TryLockError};

use engine_core::text::clip_bytes;

/// Environment variable that overrides the crash-receipt folder.
pub const CRASH_DIR_ENV: &str = "E2040_CRASH_DIR";
/// Default crash-receipt folder, relative to the working directory.
pub const DEFAULT_CRASH_DIR: &str = "target/reports/crash";
/// Stable file-name prefix of a pending receipt.
pub const RECEIPT_PREFIX: &str = "crash-";
/// Stable file-name suffix of a pending receipt.
pub const RECEIPT_SUFFIX: &str = ".txt";
/// Suffix a consumed receipt gains; such a file is never printed again.
pub const SEEN_SUFFIX: &str = ".seen";
/// Longest `last_call` text a receipt carries, in characters.
pub const MAX_CALL_CHARS: usize = 160;
/// Longest backtrace text a receipt carries, in bytes.
pub const MAX_BACKTRACE_BYTES: usize = 64 * 1024;
/// Call label used when no call was recorded before the panic.
pub const UNKNOWN_CALL: &str = "unknown: no call recorded";
/// Most names tried before a crash folder reads as full.
pub const MAX_RECEIPT_TRIES: u32 = 1000;

const HEADER: &str = "# engine2040 crash receipt v1";
const BACKTRACE_KEY: &str = "backtrace:";

/// One crash receipt: the four facts a crashed run leaves behind.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CrashReceipt {
    /// Fixed tick the run had completed when it crashed (`0` before tick 0).
    pub tick: u32,
    /// State hash after that tick (`0` before the first accepted tick).
    pub hash: u64,
    /// Last call the run recorded, one line, at most [`MAX_CALL_CHARS`].
    pub last_call: String,
    /// Panic text plus the backtrace, at most [`MAX_BACKTRACE_BYTES`].
    pub backtrace: String,
}

impl CrashReceipt {
    /// Build a receipt. `last_call` newlines fold to spaces and both long
    /// fields are clipped at their bounds, so one file stays one write.
    #[must_use]
    pub fn new(tick: u32, hash: u64, last_call: &str, backtrace: &str) -> Self {
        let folded: String = last_call
            .chars()
            .map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch })
            .take(MAX_CALL_CHARS)
            .collect();
        let call = folded.trim().to_owned();
        Self {
            tick,
            hash,
            last_call: if call.is_empty() {
                UNKNOWN_CALL.to_owned()
            } else {
                call
            },
            backtrace: clip_bytes(backtrace, MAX_BACKTRACE_BYTES).to_owned(),
        }
    }

    /// Stable file name for this receipt (`crash-<tick>-000-<hash>.txt`); a
    /// second crash at the same tick and hash gets the next three-digit
    /// counter, so names sort in crash order.
    #[must_use]
    pub fn file_name(&self) -> String {
        format!(
            "{RECEIPT_PREFIX}{}-000-{:016x}{RECEIPT_SUFFIX}",
            self.tick, self.hash
        )
    }

    /// The receipt as the plain-text file a person reads: a version comment,
    /// `tick`, `hash` (16 lowercase hex digits, the hash-law format),
    /// `last_call`, then `backtrace:` and the captured text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(96 + self.backtrace.len());
        out.push_str(HEADER);
        out.push('\n');
        out.push_str("tick=");
        out.push_str(&self.tick.to_string());
        out.push('\n');
        out.push_str("hash=");
        out.push_str(&format!("{:016x}", self.hash));
        out.push('\n');
        out.push_str("last_call=");
        out.push_str(&self.last_call);
        out.push('\n');
        out.push_str(BACKTRACE_KEY);
        out.push('\n');
        out.push_str(&self.backtrace);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    /// Parse the text [`CrashReceipt::to_text`] writes. The four rows are
    /// required; comment and blank lines are skipped.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptError::MissingField`] for a missing row or backtrace
    /// block and [`ReceiptError::BadValue`] for a malformed tick, hash or row.
    pub fn parse(text: &str) -> Result<Self, ReceiptError> {
        let (head, tail) = text
            .split_once(BACKTRACE_KEY)
            .ok_or(ReceiptError::MissingField("backtrace"))?;
        let mut tick = None;
        let mut hash = None;
        let mut last_call = None;
        for line in head.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or(ReceiptError::BadValue("row"))?;
            match key {
                "tick" => {
                    let parsed = value.trim().parse::<u32>();
                    tick = Some(parsed.map_err(|_| ReceiptError::BadValue("tick"))?);
                }
                "hash" => {
                    let parsed = u64::from_str_radix(value.trim(), 16);
                    hash = Some(parsed.map_err(|_| ReceiptError::BadValue("hash"))?);
                }
                "last_call" => last_call = Some(value.trim().to_owned()),
                _ => return Err(ReceiptError::BadValue("row")),
            }
        }
        let tick = tick.ok_or(ReceiptError::MissingField("tick"))?;
        let hash = hash.ok_or(ReceiptError::MissingField("hash"))?;
        let last_call = last_call.ok_or(ReceiptError::MissingField("last_call"))?;
        let tail = tail.strip_prefix('\n').unwrap_or(tail);
        Ok(Self {
            tick,
            hash,
            last_call,
            backtrace: tail.trim_end_matches('\n').to_owned(),
        })
    }

    /// Write this receipt as one new file under `dir`, creating the folder.
    ///
    /// The file is created with `create_new`, so an existing receipt is never
    /// overwritten: the same tick and hash twice give two numbered files.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptError::Io`] for a folder or file failure and
    /// [`ReceiptError::TooManyReceipts`] after [`MAX_RECEIPT_TRIES`] names.
    pub fn write_to(&self, dir: &Path) -> Result<PathBuf, ReceiptError> {
        std::fs::create_dir_all(dir).map_err(|err| ReceiptError::Io(err.kind()))?;
        let text = self.to_text();
        for attempt in 0..MAX_RECEIPT_TRIES {
            let name = if attempt == 0 {
                self.file_name()
            } else {
                format!(
                    "{RECEIPT_PREFIX}{}-{attempt:03}-{:016x}{RECEIPT_SUFFIX}",
                    self.tick, self.hash
                )
            };
            let path = dir.join(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(text.as_bytes())
                        .map_err(|err| ReceiptError::Io(err.kind()))?;
                    return Ok(path);
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(ReceiptError::Io(err.kind())),
            }
        }
        Err(ReceiptError::TooManyReceipts)
    }
}

/// Crash-receipt failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReceiptError {
    /// Filesystem failure, kind carried; paths stay with the caller.
    Io(std::io::ErrorKind),
    /// A required receipt row is absent; the row name is carried.
    MissingField(&'static str),
    /// A receipt row is malformed; the row name is carried.
    BadValue(&'static str),
    /// Every candidate name in the crash folder already exists.
    TooManyReceipts,
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "crash receipt io failed: {kind:?}"),
            Self::MissingField(row) => write!(f, "crash receipt misses {row}"),
            Self::BadValue(row) => write!(f, "crash receipt row is bad: {row}"),
            Self::TooManyReceipts => write!(f, "crash receipt folder is full"),
        }
    }
}

impl std::error::Error for ReceiptError {}

/// The crash-receipt folder: `E2040_CRASH_DIR` when set, else
/// [`DEFAULT_CRASH_DIR`] under the working directory.
#[must_use]
pub fn default_dir() -> PathBuf {
    match std::env::var_os(CRASH_DIR_ENV) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(DEFAULT_CRASH_DIR),
    }
}

/// Pending receipt files under `dir`, sorted by name. Missing folder reads as
/// empty; a consumed `*.txt.seen` file is not listed.
///
/// # Errors
///
/// Returns [`ReceiptError::Io`] when the folder cannot be listed.
pub fn receipt_files(dir: &Path) -> Result<Vec<PathBuf>, ReceiptError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(ReceiptError::Io(err.kind())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| ReceiptError::Io(err.kind()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(RECEIPT_PREFIX) && name.ends_with(RECEIPT_SUFFIX) {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

/// Take the oldest pending receipt and mark it seen, so the next start is
/// silent. The bytes stay on disk as `<name>.seen`.
///
/// # Errors
///
/// Returns [`ReceiptError::Io`] on a read or rename failure and the parse
/// errors of [`CrashReceipt::parse`] for a malformed file (already marked seen,
/// so a bad file is reported once, not on every start).
pub fn take_pending(dir: &Path) -> Result<Option<CrashReceipt>, ReceiptError> {
    Ok(consume_first(dir)?.map(|(_, receipt)| receipt))
}

/// Print the oldest pending receipt to stderr once and mark it seen; returns
/// the path it printed. A second start finds nothing.
///
/// # Errors
///
/// The same failures as [`take_pending`]; a folder that does not exist is not
/// an error and reads as `None`.
pub fn print_pending(dir: &Path) -> Result<Option<PathBuf>, ReceiptError> {
    let Some((path, receipt)) = consume_first(dir)? else {
        return Ok(None);
    };
    let _ = writeln!(
        std::io::stderr(),
        "engine2040 crash receipt: {}\n{}",
        path.display(),
        receipt.to_text()
    );
    Ok(Some(path))
}

/// Read and mark seen the oldest pending receipt. A file another start already
/// consumed reads as `None`, so two starts never print the same bytes.
fn consume_first(dir: &Path) -> Result<Option<(PathBuf, CrashReceipt)>, ReceiptError> {
    let Some(path) = receipt_files(dir)?.into_iter().next() else {
        return Ok(None);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(ReceiptError::Io(err.kind())),
    };
    let seen = seen_path(&path);
    match std::fs::rename(&path, &seen) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(ReceiptError::Io(err.kind())),
    }
    let receipt = CrashReceipt::parse(&text)?;
    Ok(Some((path, receipt)))
}

/// `<name>.seen`: the path a consumed receipt keeps its bytes on.
fn seen_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new(""))
        .to_os_string();
    name.push(SEEN_SUFFIX);
    path.with_file_name(name)
}

/// Live snapshot the panic hook reads: update it from the run loop, then
/// install it once.
///
/// Every read on the panic path is best effort: relaxed loads and a `try_lock`
/// mean a crash never waits on a lock another thread holds.
pub struct CrashWatch {
    dir: PathBuf,
    tick: AtomicU32,
    hash: AtomicU64,
    last_call: Mutex<String>,
}

/// The process's one installed watch; a second `install` is a no-op.
static WATCH: OnceLock<Arc<CrashWatch>> = OnceLock::new();

impl CrashWatch {
    /// A watch that writes to `dir` with nothing recorded yet.
    #[must_use]
    pub fn new(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            dir,
            tick: AtomicU32::new(0),
            hash: AtomicU64::new(0),
            last_call: Mutex::new(UNKNOWN_CALL.to_owned()),
        })
    }

    /// Record the fixed tick and hash the last accepted tick reached. Two
    /// relaxed stores: the crash path reads a snapshot, never a barrier.
    pub fn note_tick(&self, tick: u32, hash: u64) {
        self.tick.store(tick, Ordering::Relaxed);
        self.hash.store(hash, Ordering::Relaxed);
    }

    /// Record the last call the run made (a door step or a loop step), shown
    /// by the next start's receipt. The text is bounded here, so a per-tick
    /// label does not grow the slot.
    pub fn note_call(&self, call: &str) {
        let mut slot = self
            .last_call
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.clear();
        for ch in call.chars().take(MAX_CALL_CHARS) {
            slot.push(if ch.is_control() { ' ' } else { ch });
        }
    }

    /// Install this watch as the process panic hook, chained after any hook
    /// already installed (so the normal backtrace still prints and the crash
    /// report keeps its own hook). Returns true for the one install that
    /// happened; later calls are no-ops.
    ///
    /// # Panics
    ///
    /// Like [`std::panic::set_hook`], panics when called from a panicking
    /// thread; callers install at start, before a tick runs.
    pub fn install(self: &Arc<Self>) -> bool {
        if WATCH.set(Arc::clone(self)).is_err() {
            return false;
        }
        let previous = std::panic::take_hook();
        let watch = Arc::clone(self);
        std::panic::set_hook(Box::new(move |info| {
            watch.write_crash_receipt(info);
            previous(info);
        }));
        true
    }

    /// One receipt from the snapshot in hand; a failed write never panics
    /// inside the hook, and the folder is created when missing.
    fn write_crash_receipt(&self, info: &PanicHookInfo<'_>) {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let call = self.last_call_or_unknown();
        let receipt = CrashReceipt::new(
            self.tick.load(Ordering::Relaxed),
            self.hash.load(Ordering::Relaxed),
            &call,
            &format!("{info}\n{backtrace}"),
        );
        let _ = receipt.write_to(&self.dir);
    }

    /// The last call, never blocking the crash hook: a held lock reads as
    /// [`UNKNOWN_CALL`].
    fn last_call_or_unknown(&self) -> String {
        match self.last_call.try_lock() {
            Ok(call) => call.clone(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().clone(),
            Err(TryLockError::WouldBlock) => UNKNOWN_CALL.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: u64 = 0xd57b5c8ae3096614;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("engine2040-perf-crash")
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> CrashReceipt {
        CrashReceipt::new(
            600,
            GOLDEN,
            "windowed: app.tick",
            "panicked at test\n 0: frame",
        )
    }

    #[test]
    fn crash_receipt_text_roundtrips_all_four_fields() {
        let receipt = sample();
        let text = receipt.to_text();
        assert!(text.contains("tick=600"), "{text}");
        assert!(text.contains("hash=d57b5c8ae3096614"), "{text}");
        assert!(text.contains("last_call=windowed: app.tick"), "{text}");
        assert!(text.contains("backtrace:\n"), "{text}");
        let parsed = CrashReceipt::parse(&text).expect("the receipt parses");
        assert_eq!(parsed, receipt);
    }

    #[test]
    fn crash_receipt_new_folds_long_calls_and_keeps_a_label() {
        let receipt = CrashReceipt::new(3, 9, "a\nb\r\nc", "backtrace");
        assert_eq!(receipt.last_call, "a b  c");
        let empty = CrashReceipt::new(0, 0, "  ", "backtrace");
        assert_eq!(empty.last_call, UNKNOWN_CALL);
    }

    #[test]
    fn crash_receipt_parse_names_a_missing_row() {
        let error = CrashReceipt::parse("# header\nhash=00\nbacktrace:\n")
            .expect_err("a receipt without a tick is refused");
        assert_eq!(error, ReceiptError::MissingField("tick"));
        let error = CrashReceipt::parse("tick=x\nhash=00\nlast_call=c\nbacktrace:\n")
            .expect_err("a bad tick is refused");
        assert_eq!(error, ReceiptError::BadValue("tick"));
        let error = CrashReceipt::parse("tick=1\nhash=zz\nlast_call=c\nbacktrace:\n")
            .expect_err("a bad hash is refused");
        assert_eq!(error, ReceiptError::BadValue("hash"));
    }

    #[test]
    fn crash_receipt_writer_keeps_one_file_per_crash() {
        let dir = temp_dir("files");
        let receipt = sample();
        let first = receipt.write_to(&dir).expect("the first receipt lands");
        let second = receipt.write_to(&dir).expect("the second receipt lands");
        assert_ne!(first, second, "a crash never overwrites a receipt");
        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some(receipt.file_name().as_str()),
            "the first file name is the stable one"
        );
        let files = receipt_files(&dir).expect("the folder lists");
        assert_eq!(files, vec![first, second], "both receipts are pending");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_receipt_reader_prints_once_and_a_second_read_is_empty() {
        let dir = temp_dir("once");
        let receipt = sample();
        let path = receipt.write_to(&dir).expect("the receipt lands");
        let printed = print_pending(&dir).expect("the first start prints");
        assert_eq!(printed.as_deref(), Some(path.as_path()));
        let again = print_pending(&dir).expect("the second start lists");
        assert!(again.is_none(), "a seen receipt is never printed twice");
        assert!(
            receipt_files(&dir).expect("the folder lists").is_empty(),
            "nothing is pending after the print"
        );
        let seen = dir.join(format!("{}{SEEN_SUFFIX}", receipt.file_name()));
        assert!(seen.is_file(), "the seen bytes stay for a later look");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forced_panic_writes_one_receipt_with_tick_and_hash() {
        let dir = temp_dir("panic");
        let watch = CrashWatch::new(dir.clone());
        watch.note_tick(600, GOLDEN);
        watch.note_call("windowed: app.tick");
        assert!(watch.install(), "the first install owns the process hook");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("forced crash receipt");
        }));
        assert!(caught.is_err(), "the forced panic unwinds");
        let files = receipt_files(&dir).expect("the crash folder is made");
        assert_eq!(files.len(), 1, "one panic, one receipt: {files:?}");
        let text = std::fs::read_to_string(&files[0]).expect("the receipt is readable");
        let receipt = CrashReceipt::parse(&text).expect("the receipt parses");
        assert_eq!(receipt.tick, 600);
        assert_eq!(receipt.hash, GOLDEN);
        assert_eq!(receipt.last_call, "windowed: app.tick");
        assert!(!receipt.backtrace.is_empty(), "the backtrace is recorded");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
