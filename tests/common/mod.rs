//! Shared snapshot checks for the root integration tests. std-only.
//!
//! Discipline (the insta idea, reimplemented with std): a reviewed snapshot is
//! compared text for text; a changed value writes `<name>.snap.new` and fails
//! with that pending path in the message; only `E2040_ACCEPT_SNAPSHOTS=1`
//! rewrites the reviewed file. A missing snapshot is written once and passes,
//! so a new check starts from a visible file in the same commit.
//!
//! Text is normalised first: CRLF becomes LF, trailing spaces go, blank lines
//! at the end collapse to one newline, and absolute paths, the home directory
//! and the temp directory become `<repo>`, `<home>` and `<temp>`.

use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Set to `1` to rewrite reviewed snapshots instead of failing.
pub const ACCEPT_ENV: &str = "E2040_ACCEPT_SNAPSHOTS";
/// Overrides the snapshot directory (default `tests/snapshots`).
pub const SNAPSHOT_DIR_ENV: &str = "E2040_SNAPSHOT_DIR";
/// Suffix of an unreviewed snapshot.
pub const PENDING_SUFFIX: &str = ".snap.new";
/// Suffix of a reviewed snapshot.
pub const SNAPSHOT_SUFFIX: &str = ".snap";

const HOME_PREFIXES: [&str; 5] = [
    "C:\\Users\\",
    "C:/Users/",
    "C:\\Documents and Settings\\",
    "/home/",
    "/Users/",
];

/// Whether a mismatch rewrites the reviewed file or leaves a pending one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Flag a mismatch: write `<name>.snap.new` and fail.
    Check,
    /// Reviewed accept: rewrite `<name>.snap` and drop the pending file.
    Accept,
}

impl Mode {
    /// `Some("1")` accepts; every other value, including absent, checks.
    pub fn from_value(value: Option<&str>) -> Self {
        if value == Some("1") {
            Self::Accept
        } else {
            Self::Check
        }
    }

    /// Reads [`ACCEPT_ENV`] from the process environment.
    pub fn from_env() -> Self {
        Self::from_value(env::var(ACCEPT_ENV).ok().as_deref())
    }
}

/// A mismatch whose reviewed snapshot is still the old text.
#[derive(Debug)]
pub struct PendingSnapshot {
    /// Snapshot name as the caller wrote it.
    pub name: String,
    /// Reviewed file that did not match.
    pub snapshot: PathBuf,
    /// Pending file written beside it.
    pub pending: PathBuf,
    /// First differing line, quoted.
    pub detail: String,
}

impl fmt::Display for PendingSnapshot {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "snapshot `{}` differs from {}\npending snapshot: {}\n{}\nreview it, then rerun with {}=1",
            self.name,
            self.snapshot.display(),
            self.pending.display(),
            self.detail,
            ACCEPT_ENV
        )
    }
}

/// Directory reviewed snapshots live in.
pub fn snapshot_root() -> PathBuf {
    env::var_os(SNAPSHOT_DIR_ENV).map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("snapshots")
        },
        PathBuf::from,
    )
}

/// Normalises text for comparison and storage.
pub fn normalize(text: &str) -> String {
    let text = redact(text);
    let mut out = String::with_capacity(text.len() + 1);
    for line in text.split('\n') {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Replaces the repo root, the temp directory and the user's home with stable
/// tokens, so a snapshot means the same on every machine and CI runner.
pub fn redact(text: &str) -> String {
    let mut out = replace_variants(text, env!("CARGO_MANIFEST_DIR"), "<repo>");
    if let Some(temp) = env::temp_dir().to_str() {
        out = replace_variants(&out, temp, "<temp>");
    }
    replace_user_home(&out)
}

fn replace_variants(text: &str, needle: &str, with: &str) -> String {
    let out = text.replace(needle, with);
    let slashed = needle.replace('\\', "/");
    if slashed == needle {
        out
    } else {
        out.replace(&slashed, with)
    }
}

fn replace_user_home(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let mut hit: Option<(usize, usize)> = None;
        for prefix in HOME_PREFIXES {
            if let Some(at) = rest.find(prefix) {
                if hit.is_none_or(|(best, _)| at < best) {
                    hit = Some((at, prefix.len()));
                }
            }
        }
        let Some((at, len)) = hit else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + len..];
        let end = after.find(['/', '\\']).unwrap_or(after.len());
        if end == 0 {
            out.push_str(&rest[at..at + len]);
            rest = after;
            continue;
        }
        out.push_str("<home>");
        rest = &after[end..];
    }
}

/// Compares `actual` with `<dir>/<name>.snap`, writes the pending or accepted
/// file, and returns the mismatch that keeps the check red.
pub fn check_snapshot_in(
    dir: &Path,
    name: &str,
    actual: &str,
    mode: Mode,
) -> Result<(), PendingSnapshot> {
    let actual = normalize(actual);
    let snapshot = dir.join(format!("{name}{SNAPSHOT_SUFFIX}"));
    let pending = dir.join(format!("{name}{PENDING_SUFFIX}"));
    let reviewed = match fs::read_to_string(&snapshot) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write(&snapshot, &actual);
            return Ok(());
        }
        Err(error) => panic!("read snapshot {}: {error}", snapshot.display()),
    };
    let reviewed = normalize(&reviewed);
    if reviewed == actual {
        let _ = fs::remove_file(&pending);
        return Ok(());
    }
    if mode == Mode::Accept {
        write(&snapshot, &actual);
        let _ = fs::remove_file(&pending);
        return Ok(());
    }
    write(&pending, &actual);
    Err(PendingSnapshot {
        name: name.to_owned(),
        snapshot,
        pending,
        detail: first_difference(&reviewed, &actual),
    })
}

/// Checks `name` against `tests/snapshots`, honouring [`ACCEPT_ENV`].
pub fn assert_snapshot(name: &str, actual: &str) {
    if let Err(pending) = check_snapshot_in(&snapshot_root(), name, actual, Mode::from_env()) {
        panic!("{pending}");
    }
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create snapshot dir {}: {error}", parent.display()));
    }
    fs::write(path, text)
        .unwrap_or_else(|error| panic!("write snapshot {}: {error}", path.display()));
}

fn first_difference(reviewed: &str, actual: &str) -> String {
    let expected: Vec<&str> = reviewed.lines().collect();
    let found: Vec<&str> = actual.lines().collect();
    let mut line = 0;
    for (index, (want, got)) in expected.iter().zip(&found).enumerate() {
        if want != got {
            line = index + 1;
            break;
        }
    }
    if line == 0 {
        line = expected.len().min(found.len()) + 1;
    }
    let at = |lines: &[&str]| clip(lines.get(line - 1).copied().unwrap_or("<eof>"));
    format!(
        "first difference at line {line}: reviewed `{}` actual `{}`",
        at(&expected),
        at(&found)
    )
}

fn clip(line: &str) -> String {
    let mut out: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "engine2040-snapshot-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_write_creates_the_snapshot_and_passes() {
        let dir = Temp::new();
        check_snapshot_in(&dir.0, "first", "alpha\n", Mode::Check).unwrap();
        let snapshot = dir.0.join("first.snap");
        assert_eq!(fs::read_to_string(&snapshot).unwrap(), "alpha\n");
        assert!(!dir.0.join("first.snap.new").exists());
    }

    #[test]
    fn mismatch_writes_the_pending_path_and_names_it() {
        let dir = Temp::new();
        fs::write(dir.0.join("receipt.snap"), "old value\n").unwrap();
        let error = check_snapshot_in(&dir.0, "receipt", "new value\n", Mode::Check).unwrap_err();
        let pending = dir.0.join("receipt.snap.new");
        assert_eq!(fs::read_to_string(&pending).unwrap(), "new value\n");
        let message = error.to_string();
        assert!(
            message.contains(&pending.display().to_string()),
            "message must name the pending path: {message}"
        );
        assert!(
            message.contains(ACCEPT_ENV),
            "message must name the accept flag: {message}"
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("receipt.snap")).unwrap(),
            "old value\n",
            "a check never rewrites the reviewed file"
        );
    }

    #[test]
    fn accept_rewrites_the_snapshot_and_clears_the_pending_file() {
        let dir = Temp::new();
        fs::write(dir.0.join("receipt.snap"), "old value\n").unwrap();
        fs::write(dir.0.join("receipt.snap.new"), "stale\n").unwrap();
        check_snapshot_in(&dir.0, "receipt", "new value\n", Mode::Accept).unwrap();
        assert_eq!(
            fs::read_to_string(dir.0.join("receipt.snap")).unwrap(),
            "new value\n"
        );
        assert!(!dir.0.join("receipt.snap.new").exists());
    }

    #[test]
    fn match_passes_and_clears_a_stale_pending_file() {
        let dir = Temp::new();
        fs::write(dir.0.join("same.snap"), "kept\n").unwrap();
        fs::write(dir.0.join("same.snap.new"), "stale\n").unwrap();
        check_snapshot_in(&dir.0, "same", "kept\n", Mode::Check).unwrap();
        assert!(!dir.0.join("same.snap.new").exists());
    }

    #[test]
    fn normalize_strips_crlf_trailing_space_and_extra_blank_lines() {
        assert_eq!(normalize("a  \r\nb\r\n\r\n\r\n"), "a\nb\n");
    }

    #[test]
    fn redact_hides_the_home_directory_and_the_temp_dir() {
        let (home, expected) = if cfg!(windows) {
            ("C:\\Users\\alice\\work", "<home>\\work")
        } else {
            ("/home/alice/work", "<home>/work")
        };
        assert_eq!(redact(home), expected);
        assert!(!redact(home).contains("alice"));
        if let Some(temp) = env::temp_dir().to_str() {
            assert_eq!(redact(&format!("{temp}\\x")), "<temp>\\x");
        }
    }

    #[test]
    fn only_the_value_one_accepts() {
        assert_eq!(Mode::from_value(Some("1")), Mode::Accept);
        for other in [None, Some("0"), Some("true"), Some("yes")] {
            assert_eq!(Mode::from_value(other), Mode::Check);
        }
    }
}
