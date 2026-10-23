//! Reimport watch (E10): file snapshots compared at tick boundaries.
//!
//! The dock watches the folders an artist drops assets from. No threads and no
//! notify crate: the caller calls [`Watch::poll`] once per tick boundary with
//! the current tick, and a change is reported only after it has sat still for
//! the coalescing window, so a DCC save that rewrites several files in one
//! breath costs one reimport.
//! Units: `now_ticks` is a caller tick; a snapshot is `(modified, length)`.
//! Determinism: entries poll in watch order, so a report list follows that
//! order and never a hash order.
//! Errors: [`WatchError`]; a refused watch leaves the list untouched.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Most files one watch holds.
pub const MAX_WATCHED: usize = 256;
/// Most changes one poll reports; the rest wait for the next tick.
pub const MAX_REPORTS_PER_POLL: usize = 32;
/// Coalescing window a caller gets when it asks for none: two ticks.
pub const DEFAULT_WINDOW_TICKS: u64 = 2;

/// One file snapshot: modified time (absent when the file is missing) and
/// length in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snapshot {
    mtime: Option<SystemTime>,
    len: u64,
}

impl Snapshot {
    /// Stat one path; a missing or unreadable file is a `(None, 0)` snapshot
    /// so its later appearance counts as a change.
    fn read(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) => Self {
                mtime: meta.modified().ok(),
                len: meta.len(),
            },
            Err(_) => Self {
                mtime: None,
                len: 0,
            },
        }
    }
}

/// One watched file: its path, last snapshot and the tick its current change
/// was first seen.
#[derive(Clone, Debug)]
struct Watched {
    path: PathBuf,
    seen: Snapshot,
    changed_at: Option<u64>,
    mtime_changed: bool,
    size_changed: bool,
}

/// One reported change, ready for a reimport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    /// File that changed.
    pub path: PathBuf,
    /// Tick the change was first seen.
    pub first_tick: u64,
    /// True when the modified time moved.
    pub mtime_changed: bool,
    /// True when the byte length moved.
    pub size_changed: bool,
}

/// A bounded set of watched files with a coalescing window.
#[derive(Clone, Debug)]
pub struct Watch {
    entries: Vec<Watched>,
    window_ticks: u64,
    reports: u64,
    deferred: u64,
}

impl Watch {
    /// Empty watch with a coalescing window in ticks (`0` reports on the first
    /// poll that sees a change).
    pub fn new(window_ticks: u64) -> Self {
        Self {
            entries: Vec::new(),
            window_ticks,
            reports: 0,
            deferred: 0,
        }
    }

    /// Coalescing window in ticks.
    pub fn window_ticks(&self) -> u64 {
        self.window_ticks
    }

    /// Watched file count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no file is watched.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many changes have been reported.
    pub fn reports(&self) -> u64 {
        self.reports
    }

    /// How many changes sat over [`MAX_REPORTS_PER_POLL`] in one poll.
    pub fn deferred(&self) -> u64 {
        self.deferred
    }

    /// Watch one file. False when it is already watched; an empty path and the
    /// watch bound refuse without change.
    pub fn watch(&mut self, path: &Path) -> Result<bool, WatchError> {
        if path.as_os_str().is_empty() {
            return Err(WatchError::EmptyPath);
        }
        if self.entries.iter().any(|entry| entry.path == path) {
            return Ok(false);
        }
        if self.entries.len() >= MAX_WATCHED {
            return Err(WatchError::Full {
                len: self.entries.len(),
                max: MAX_WATCHED,
            });
        }
        self.entries.push(Watched {
            path: path.to_path_buf(),
            seen: Snapshot::read(path),
            changed_at: None,
            mtime_changed: false,
            size_changed: false,
        });
        Ok(true)
    }

    /// Snapshot every file at `now_ticks` and return the changes that have sat
    /// still for the whole window.
    ///
    /// A change seen for the first time starts the window; it is reported on
    /// the first poll at least `window_ticks` later, so a burst is one report.
    /// Past [`MAX_REPORTS_PER_POLL`] the rest stay pending for the next poll.
    pub fn poll(&mut self, now_ticks: u64) -> Vec<Changed> {
        for entry in &mut self.entries {
            let now = Snapshot::read(&entry.path);
            if now != entry.seen {
                entry.mtime_changed |= now.mtime != entry.seen.mtime;
                entry.size_changed |= now.len != entry.seen.len;
                entry.seen = now;
                if entry.changed_at.is_none() {
                    entry.changed_at = Some(now_ticks);
                }
            }
        }
        let mut out = Vec::new();
        for entry in &mut self.entries {
            let Some(first_tick) = entry.changed_at else {
                continue;
            };
            if now_ticks.saturating_sub(first_tick) < self.window_ticks {
                continue;
            }
            if out.len() >= MAX_REPORTS_PER_POLL {
                self.deferred = self.deferred.saturating_add(1);
                continue;
            }
            entry.changed_at = None;
            out.push(Changed {
                path: entry.path.clone(),
                first_tick,
                mtime_changed: entry.mtime_changed,
                size_changed: entry.size_changed,
            });
            entry.mtime_changed = false;
            entry.size_changed = false;
            self.reports = self.reports.saturating_add(1);
        }
        out
    }
}

/// What refused a watch. Nothing was watched on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchError {
    /// The path held no text.
    EmptyPath,
    /// The watch already holds [`MAX_WATCHED`] files.
    Full {
        /// Files held now.
        len: usize,
        /// The bound.
        max: usize,
    },
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => f.write_str("watch path holds no text; pass a file path"),
            Self::Full { len, max } => write!(
                f,
                "watch holds {len} of {max} files; unwatch a folder or watch one file at a time"
            ),
        }
    }
}

impl std::error::Error for WatchError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_writes_one_report_after_the_window() {
        let dir = std::env::temp_dir().join(format!("e2040-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prop.glb");
        std::fs::write(&path, b"a").unwrap();

        let mut watch = Watch::new(2);
        assert!(watch.watch(&path).unwrap());
        assert!(!watch.watch(&path).unwrap(), "the second watch is a no-op");
        assert_eq!(watch.watch(Path::new("")), Err(WatchError::EmptyPath));

        // First write: seen but still inside the window.
        std::fs::write(&path, b"bb").unwrap();
        assert!(watch.poll(0).is_empty());
        // Window over: one report, naming the tick it was first seen.
        let changes = watch.poll(2);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, path);
        assert_eq!(changes[0].first_tick, 0);
        assert!(changes[0].size_changed);
        assert_eq!(watch.reports(), 1);
        // Nothing new: the next poll is quiet.
        assert!(watch.poll(2).is_empty());

        // Second write: reported again after its own window.
        std::fs::write(&path, b"ccc").unwrap();
        assert!(watch.poll(3).is_empty());
        assert_eq!(watch.poll(5).len(), 1);
        assert_eq!(watch.reports(), 2);

        // A missing file then appears: that is a change too.
        let fresh = dir.join("new.glb");
        watch.watch(&fresh).unwrap();
        assert!(watch.poll(6).is_empty());
        std::fs::write(&fresh, b"x").unwrap();
        assert!(watch.poll(7).is_empty());
        assert_eq!(watch.poll(9).len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_watch_list_is_bounded() {
        let mut watch = Watch::new(DEFAULT_WINDOW_TICKS);
        for n in 0..MAX_WATCHED {
            watch
                .watch(Path::new(&format!("C:/props/{n}.glb")))
                .unwrap();
        }
        assert_eq!(watch.len(), MAX_WATCHED);
        assert_eq!(
            watch.watch(Path::new("C:/props/one-too-many.glb")),
            Err(WatchError::Full {
                len: MAX_WATCHED,
                max: MAX_WATCHED
            })
        );
    }
}
