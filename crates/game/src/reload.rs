//! Hot-reload gate: staged generations applied at frame boundaries.
//!
//! Consumer: the window session polls its watcher once per presented frame
//! and settles here (`bin/arena.rs`). Units: frames/ticks as indices,
//! generations as opaque `u64`; drain order is note order, no clock.
//! Errors: [`ReloadReason`]; the old generation stays live.

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
}
