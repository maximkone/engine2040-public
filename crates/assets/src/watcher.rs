//! watcher: std-only poll-driven file watch over mtimes.
//!
//! The caller owns the poll cadence (per frame, on demand, or from the
//! practice-mode loop) and calls [`Watcher::poll`]; the watcher diffs
//! `std::fs::metadata` snapshots (mtime plus length) against the previous
//! poll. No threads, no async, no `notify` dependency: a polling scan keeps
//! the headless default std-only and deterministic in ordering (events come
//! out in [`WatchId`] order, one per changed watch).
//!
//! Future optimization (not this task): replace the mtime scan with the
//! `notify` crate for OS push events. The [`FileEvent`] shape is the seam:
//! only the inside of `poll` changes.
//!
//! Running-game latency acceptance (edit visible within 1 s on the small
//! fixture) belongs to the hot-reload consumer, not here: this crate reports transitions, it
//! never publishes into a live match. Consumers own publication boundaries.

use std::collections::BTreeMap;
use std::time::UNIX_EPOCH;

/// Opaque watch handle, minted by [`Watcher::watch`] in increasing order.
pub type WatchId = u64;

/// What changed for one watched path since the previous [`Watcher::poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// No snapshot existed before and the file exists now.
    Created,
    /// The file existed before and its mtime or length changed.
    Modified,
    /// A snapshot existed before and the file is gone (or unreadable) now.
    Removed,
}

/// One path transition. The producer of the watch loop forwards these to the
/// recook queue; publication into a running game stays with the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEvent {
    /// Watched path, exactly as passed to [`Watcher::watch`].
    pub path: String,
    /// The observed transition.
    pub kind: FileKind,
}

/// Per-watch options, taken by value at [`Watcher::watch`] time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatchSettings {
    /// When true, the first [`Watcher::poll`] after `watch` reports an
    /// already-existing file as [`FileKind::Created`]. When false (default)
    /// the first poll only records the baseline silently.
    pub report_existing: bool,
}

/// Snapshot of one watched path: mtime plus length. Length rides along
/// because some filesystems tick mtime coarsely; either half changing is a
/// [`FileKind::Modified`] event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Signature {
    /// Seconds since the Unix epoch; `0` when the mtime is unavailable.
    secs: u64,
    /// Sub-second nanos; `0` when the mtime is unavailable.
    nanos: u32,
    /// `metadata.len()` at snapshot time.
    len: u64,
}

/// Read the current snapshot; `None` when the path has no readable metadata.
fn signature(path: &str) -> Option<Signature> {
    let meta = std::fs::metadata(path).ok()?;
    let (secs, nanos) = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos()));
    Some(Signature {
        secs,
        nanos,
        len: meta.len(),
    })
}

/// One active watch: the path plus its last-polled snapshot (`None` while the
/// file is absent). `seen` distinguishes the first-ever poll (baseline, silent
/// unless `report_existing`) from a later reappearance (a `Created` event).
#[derive(Clone, Debug)]
struct Watch {
    path: String,
    known: Option<Signature>,
    seen: bool,
    report_existing: bool,
}

/// Poll-driven file watcher. Owns the watch set; the caller owns the cadence.
/// Event order is deterministic: watches are visited in [`WatchId`] order.
#[derive(Clone, Debug, Default)]
pub struct Watcher {
    next: WatchId,
    watches: BTreeMap<WatchId, Watch>,
    last_dirty: bool,
}

/// Hot-reload hook the export path reuses: true when `events` is non-empty. Dev-only.
pub fn needs_recook(events: &[FileEvent]) -> bool {
    !events.is_empty()
}

impl Watcher {
    /// Empty watcher: no watches, first id is `0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Watch count.
    pub fn len(&self) -> usize {
        self.watches.len()
    }

    /// True when no paths are watched.
    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    /// Start watching `path`. Always succeeds and mints a fresh id; a path
    /// with no readable metadata simply reports nothing until it appears.
    /// The baseline is taken on the next [`Watcher::poll`].
    pub fn watch(&mut self, path: &str, settings: WatchSettings) -> WatchId {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        self.watches.insert(
            id,
            Watch {
                path: path.to_string(),
                known: None,
                seen: false,
                report_existing: settings.report_existing,
            },
        );
        id
    }

    /// Stop watching `id`. True when a watch was removed.
    pub fn unwatch(&mut self, id: WatchId) -> bool {
        self.watches.remove(&id).is_some()
    }

    /// True when `id` names an active watch.
    pub fn is_watching(&self, id: WatchId) -> bool {
        self.watches.contains_key(&id)
    }

    /// Diff every watch against its last snapshot, oldest watch first.
    /// At most one event per watch per call.
    pub fn poll(&mut self) -> Vec<FileEvent> {
        let mut out = Vec::new();
        for watch in self.watches.values_mut() {
            let current = signature(&watch.path);
            if !watch.seen {
                // Baseline poll: record, report only when asked.
                watch.seen = true;
                watch.known = current;
                if watch.report_existing && current.is_some() {
                    out.push(FileEvent {
                        path: watch.path.clone(),
                        kind: FileKind::Created,
                    });
                }
                continue;
            }
            match (watch.known, current) {
                (None, None) => {}
                (None, Some(seen)) => {
                    // Absent at the last poll, present now: a creation.
                    // (First sightings never reach here; they are baselined.)
                    watch.known = Some(seen);
                    out.push(FileEvent {
                        path: watch.path.clone(),
                        kind: FileKind::Created,
                    });
                }
                (Some(_), None) => {
                    watch.known = None;
                    out.push(FileEvent {
                        path: watch.path.clone(),
                        kind: FileKind::Removed,
                    });
                }
                (Some(before), Some(seen)) => {
                    if before != seen {
                        watch.known = Some(seen);
                        out.push(FileEvent {
                            path: watch.path.clone(),
                            kind: FileKind::Modified,
                        });
                    }
                }
            }
        }
        self.last_dirty = needs_recook(&out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    /// Unique scratch path per test (unit tests run in parallel threads).
    fn scratch(name: &str) -> String {
        let n = SCRATCH.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "engine-assets-hr1-{name}-{}-{n}.tmp",
            std::process::id()
        ));
        path.to_string_lossy().into_owned()
    }

    fn remove(path: &str) {
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn created_modified_removed_lifecycle() {
        let path = scratch("lifecycle");
        remove(&path);
        let mut watcher = Watcher::new();
        let id = watcher.watch(&path, WatchSettings::default());
        assert!(watcher.is_watching(id));
        assert_eq!(watcher.len(), 1);

        // Absent file: no baseline event.
        assert!(watcher.poll().is_empty());

        // Appear: Created (length differs from "absent", so it is visible).
        std::fs::write(&path, b"v1").unwrap();
        let events = watcher.poll();
        assert_eq!(
            events,
            vec![FileEvent {
                path: path.clone(),
                kind: FileKind::Created,
            }]
        );

        // Quiet poll: nothing.
        assert!(watcher.poll().is_empty());

        // Rewrite with a different length: Modified even on coarse mtimes.
        std::fs::write(&path, b"v1-plus-more-bytes").unwrap();
        let events = watcher.poll();
        assert_eq!(
            events,
            vec![FileEvent {
                path: path.clone(),
                kind: FileKind::Modified,
            }]
        );
        assert!(watcher.poll().is_empty());

        // Delete: Removed.
        remove(&path);
        let events = watcher.poll();
        assert_eq!(
            events,
            vec![FileEvent {
                path: path.clone(),
                kind: FileKind::Removed,
            }]
        );
        assert!(watcher.poll().is_empty());
        remove(&path);
    }

    #[test]
    fn report_existing_controls_first_poll() {
        let loud = scratch("loud");
        let quiet = scratch("quiet");
        std::fs::write(&loud, b"loud-bytes").unwrap();
        std::fs::write(&quiet, b"quiet-bytes").unwrap();

        let mut watcher = Watcher::new();
        watcher.watch(
            &loud,
            WatchSettings {
                report_existing: true,
            },
        );
        watcher.watch(&quiet, WatchSettings::default());

        // Visited in WatchId order: the loud watch first.
        let events = watcher.poll();
        assert_eq!(
            events,
            vec![FileEvent {
                path: loud.clone(),
                kind: FileKind::Created,
            }]
        );
        remove(&loud);
        remove(&quiet);
    }

    #[test]
    fn unwatch_stops_events_and_unknown_is_false() {
        let path = scratch("unwatch");
        remove(&path);
        let mut watcher = Watcher::new();
        let id = watcher.watch(&path, WatchSettings::default());
        assert!(!watcher.unwatch(id + 1000));
        assert!(watcher.unwatch(id));
        assert!(!watcher.is_watching(id));
        assert!(watcher.is_empty());

        // Changes after unwatch report nothing.
        std::fs::write(&path, b"late-bytes").unwrap();
        assert!(watcher.poll().is_empty());
        remove(&path);
    }

    #[test]
    fn unwatched_missing_path_stays_silent() {
        let path = scratch("ghost");
        remove(&path);
        let mut watcher = Watcher::new();
        watcher.watch(&path, WatchSettings::default());
        // Never appears: every poll is empty, never an error.
        assert!(watcher.poll().is_empty());
        assert!(watcher.poll().is_empty());
    }

    #[test]
    fn recook_signal_follows_poll_output() {
        assert!(!needs_recook(&[]));
        let one = FileEvent {
            path: "a".to_string(),
            kind: FileKind::Modified,
        };
        assert!(needs_recook(std::slice::from_ref(&one)));
    }

    #[test]
    fn poll_records_recook_before_staging() {
        let path = scratch("recook-flag");
        remove(&path);
        let mut watcher = Watcher::new();
        watcher.watch(&path, WatchSettings::default());
        assert!(watcher.poll().is_empty());
        assert!(!watcher.last_dirty);
        std::fs::write(&path, b"v1").unwrap();
        let events = watcher.poll();
        assert!(needs_recook(&events));
        assert!(watcher.last_dirty);
        remove(&path);
    }
}
