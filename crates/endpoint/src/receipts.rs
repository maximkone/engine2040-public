//! receipts: every mutation returns proof. ok:true only on landed.

/// Call outcome: whether the door answered only or landed state.
/// Units are counts of effects. No errors: refusals ride on the receipt.
/// Budget is one effect per call.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Answered,
    Landed,
}

/// Call receipt: success flag, effect, hash, undo revision and recovery.
/// Units are hash and revision numbers. Errors are the `code` plus two
/// recovery verbs. Budget is one receipt per call.
#[derive(Debug)]
pub struct Receipt {
    pub ok: bool,
    pub effect: Effect,
    pub hash: u64,
    pub undo_to: u64,
    /// Machine code: "ok" | "arg_bind_failed" | "handler_threw".
    pub code: &'static str,
    /// True only for caller-fixable binds.
    pub retryable: bool,
    /// Always >=2 recovery verbs.
    pub recovery: [&'static str; 2],
}

/// Most change rows a receipt lists before it clips to counts. An agent
/// reads the head of the list; the counts stay the whole truth.
pub const MAX_CHANGES: usize = 60;

/// Bounded, read-only "what changed" summary of one landed write.
///
/// Consumer: the `tx.apply` JSON receipt, so an agent reads which objects a
/// call added, removed or moved instead of re-reading the scene. The counts
/// are the full totals; `rows` keeps at most [`MAX_CHANGES`] of them and
/// `truncated` says when it clipped, so a count never lies.
/// Units are counts and revision numbers. Budget is one bounded list per call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeList {
    /// Document revision before the write.
    pub revision_before: u64,
    /// Document revision after the write.
    pub revision_after: u64,
    /// Objects present after only.
    pub added: u32,
    /// Objects present before only.
    pub removed: u32,
    /// Objects on both sides whose world pose moved.
    pub moved: u32,
    /// One row per change, clipped to [`MAX_CHANGES`].
    pub rows: Vec<String>,
    /// True when `rows` dropped entries; the counts stay the full totals.
    pub truncated: bool,
}

impl ChangeList {
    /// Clip a full change list: `rows` keeps the head, the counts keep the
    /// whole. The caller lists one row per change, so `rows` clips to
    /// [`MAX_CHANGES`] and `truncated` is true exactly when it did.
    pub fn bounded(
        revision_before: u64,
        revision_after: u64,
        added: u32,
        removed: u32,
        moved: u32,
        mut rows: Vec<String>,
    ) -> Self {
        let truncated = rows.len() > MAX_CHANGES;
        rows.truncate(MAX_CHANGES);
        Self {
            revision_before,
            revision_after,
            added,
            removed,
            moved,
            rows,
            truncated,
        }
    }

    /// True when the write moved nothing.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && self.moved == 0
    }
}

impl Receipt {
    /// landed: state changed, hash + undo revision attached.
    pub fn landed(hash: u64, undo_to: u64) -> Self {
        Self {
            ok: true,
            effect: Effect::Landed,
            hash,
            undo_to,
            code: "ok",
            retryable: false,
            recovery: ["scene.read", "log.query"],
        }
    }
    /// answered: question only, nothing changed. ok is still true —
    /// the CALL succeeded — but effect proves no mutation happened.
    pub fn answered() -> Self {
        Self {
            ok: true,
            effect: Effect::Answered,
            hash: 0,
            undo_to: 0,
            code: "ok",
            retryable: false,
            recovery: ["scene.read", "log.query"],
        }
    }
    /// refused bind: yours, fix args + retry once.
    pub fn refused_bind() -> Self {
        Self {
            ok: false,
            effect: Effect::Answered,
            hash: 0,
            undo_to: 0,
            code: "arg_bind_failed",
            retryable: true,
            recovery: ["api.search", "api.describe"],
        }
    }
    /// handler threw: ours, report, never blind retry.
    pub fn failed_handler() -> Self {
        Self {
            ok: false,
            effect: Effect::Answered,
            hash: 0,
            undo_to: 0,
            code: "handler_threw",
            retryable: false,
            recovery: ["log.query", "rpc.discover"],
        }
    }
}

/// What an agent claims about its own work (AD21). `Landed` says the write
/// changed the document, `Confirmed` says a later check read it back clean,
/// `Viewed` says a capture or view read showed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Landed,
    Confirmed,
    Viewed,
}

/// What the journal and the read log hold for the receipt key a claim rests
/// on. A key the journal no longer holds is `Unknown`, never backed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRecord {
    /// The journal holds no entry for this key.
    Unknown,
    /// A preview key: staged for a dry run, never applied.
    Preview,
    /// An apply receipt: the revision it landed at, the staged content digest
    /// and the digest read back after the write.
    Applied {
        revision: u64,
        staged: [u8; 32],
        after: [u8; 32],
    },
}

/// One read the log kept: the revision it read at, how many checks ran, how
/// many findings they raised, and whether it was a capture or view read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadRecord {
    pub revision: u64,
    pub checks_run: u32,
    pub findings: u32,
    pub view: bool,
}

/// The answer to a claim check: backed, unknown, or refused with the missing
/// proof named and one live verb to go and get it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimVerdict {
    Backed,
    /// The key is not in the journal any more: nothing to check against.
    Unknown,
    /// The claim is not backed: `missing` names the proof, `next` the verb
    /// that produces it.
    Refused {
        missing: &'static str,
        next: &'static str,
    },
}

/// Refusal code for a claim with no backing proof.
pub const CLAIM_NOT_BACKED: &str = "claim_not_backed";

/// Check one claim against what the journal and the read log hold (AD21).
///
/// Pure and read-only: the caller gathers the record and the reads, this
/// function answers. `landed` needs an apply receipt whose staged and read
/// back digests are equal; a preview key never claims it. `confirmed` needs
/// an apply at revision `r` and a read at `r` with checks run and no
/// findings. `viewed` needs a capture or view read at the key's revision.
pub fn claim_check(claim: Claim, key: KeyRecord, reads: &[ReadRecord]) -> ClaimVerdict {
    let refused =
        |missing: &'static str, next: &'static str| ClaimVerdict::Refused { missing, next };
    match (claim, key) {
        (_, KeyRecord::Unknown) => ClaimVerdict::Unknown,
        (Claim::Landed, KeyRecord::Preview) => refused(
            "an apply receipt for this key; a preview key never lands",
            "tx.apply",
        ),
        (Claim::Landed, KeyRecord::Applied { staged, after, .. }) => {
            if staged == after {
                ClaimVerdict::Backed
            } else {
                refused(
                    "an apply whose read-back digest equals the staged digest",
                    "scene.read",
                )
            }
        }
        (Claim::Confirmed | Claim::Viewed, KeyRecord::Preview) => refused(
            "an apply receipt for this key before any read of it",
            "tx.apply",
        ),
        (Claim::Confirmed, KeyRecord::Applied { revision, .. }) => {
            let clean = reads.iter().any(|read| {
                !read.view && read.revision == revision && read.checks_run > 0 && read.findings == 0
            });
            if clean {
                ClaimVerdict::Backed
            } else {
                refused(
                    "a later read at this revision with checks run and no findings",
                    "scene.spatial",
                )
            }
        }
        (Claim::Viewed, KeyRecord::Applied { revision, .. }) => {
            let viewed = reads
                .iter()
                .any(|read| read.view && read.revision == revision);
            if viewed {
                ClaimVerdict::Backed
            } else {
                refused("a capture or view read at this revision", "capture")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(revision: u64, staged: u8, after: u8) -> KeyRecord {
        KeyRecord::Applied {
            revision,
            staged: [staged; 32],
            after: [after; 32],
        }
    }

    fn checked_read(revision: u64) -> ReadRecord {
        ReadRecord {
            revision,
            checks_run: 4,
            findings: 0,
            view: false,
        }
    }

    #[test]
    fn claim_check_preview_key_never_claims_landed() {
        let verdict = claim_check(Claim::Landed, KeyRecord::Preview, &[]);
        assert!(
            matches!(verdict, ClaimVerdict::Refused { .. }),
            "{verdict:?}"
        );
        assert_ne!(verdict, ClaimVerdict::Backed);
    }

    #[test]
    fn claim_check_apply_with_equal_digests_is_backed() {
        assert_eq!(
            claim_check(Claim::Landed, applied(3, 7, 7), &[]),
            ClaimVerdict::Backed
        );
    }

    #[test]
    fn claim_check_confirmed_needs_a_later_checked_read() {
        let key = applied(3, 7, 7);
        let none = claim_check(Claim::Confirmed, key, &[]);
        assert!(
            matches!(none, ClaimVerdict::Refused { .. }),
            "no read: {none:?}"
        );
        let found = claim_check(Claim::Confirmed, key, &[checked_read(3)]);
        assert_eq!(found, ClaimVerdict::Backed);
        // A read at another revision, or with findings, does not back it.
        let other_rev = claim_check(Claim::Confirmed, key, &[checked_read(2)]);
        assert!(matches!(other_rev, ClaimVerdict::Refused { .. }));
        let dirty = ReadRecord {
            findings: 1,
            ..checked_read(3)
        };
        assert!(matches!(
            claim_check(Claim::Confirmed, key, &[dirty]),
            ClaimVerdict::Refused { .. }
        ));
    }

    #[test]
    fn claim_check_one_flipped_digest_byte_flips_the_answer() {
        let mut after = [7u8; 32];
        after[31] ^= 1;
        let flipped = KeyRecord::Applied {
            revision: 3,
            staged: [7u8; 32],
            after,
        };
        assert_eq!(
            claim_check(Claim::Landed, applied(3, 7, 7), &[]),
            ClaimVerdict::Backed
        );
        assert!(matches!(
            claim_check(Claim::Landed, flipped, &[]),
            ClaimVerdict::Refused { .. }
        ));
    }

    #[test]
    fn claim_check_unknown_key_answers_unknown_never_backed() {
        for claim in [Claim::Landed, Claim::Confirmed, Claim::Viewed] {
            let verdict = claim_check(claim, KeyRecord::Unknown, &[checked_read(3)]);
            assert_eq!(verdict, ClaimVerdict::Unknown, "{claim:?}");
        }
    }

    #[test]
    fn claim_check_every_refusal_names_a_live_verb() {
        let keys = [KeyRecord::Preview, applied(3, 7, 8), applied(3, 7, 7)];
        for key in keys {
            for claim in [Claim::Landed, Claim::Confirmed, Claim::Viewed] {
                if let ClaimVerdict::Refused { missing, next } = claim_check(claim, key, &[]) {
                    assert!(!missing.is_empty(), "the proof is named");
                    assert!(
                        crate::tools::lookup(next).is_some(),
                        "next `{next}` is a registered verb"
                    );
                }
            }
        }
        assert_eq!(CLAIM_NOT_BACKED, "claim_not_backed");
    }

    #[test]
    fn claim_check_equal_bytes_twice() {
        let a = format!("{:?}", claim_check(Claim::Viewed, applied(3, 7, 7), &[]));
        let b = format!("{:?}", claim_check(Claim::Viewed, applied(3, 7, 7), &[]));
        assert_eq!(a, b);
    }

    #[test]
    fn envelope_carries_recovery() {
        for r in [
            Receipt::landed(7, 3),
            Receipt::answered(),
            Receipt::refused_bind(),
            Receipt::failed_handler(),
        ] {
            assert_eq!(r.recovery.len(), 2);
        }
        assert!(Receipt::landed(1, 0).ok);
        assert!(!Receipt::refused_bind().ok);
        assert!(Receipt::refused_bind().retryable);
        assert!(!Receipt::failed_handler().retryable);
        assert_eq!(Receipt::refused_bind().code, "arg_bind_failed");
        assert_eq!(Receipt::failed_handler().code, "handler_threw");
    }

    #[test]
    fn recovery_verbs_name_live_tools() {
        for r in [
            Receipt::landed(7, 3),
            Receipt::answered(),
            Receipt::refused_bind(),
            Receipt::failed_handler(),
        ] {
            for verb in r.recovery {
                assert!(
                    crate::tools::lookup(verb).is_some(),
                    "recovery `{verb}` must be a registered tool"
                );
            }
        }
    }

    #[test]
    fn change_list_reports_exactly_the_changes_it_was_given() {
        let list = ChangeList::bounded(3, 4, 1, 0, 0, vec!["added 7".to_string()]);
        assert_eq!(list.revision_before, 3);
        assert_eq!(list.revision_after, 4);
        assert_eq!((list.added, list.removed, list.moved), (1, 0, 0));
        assert_eq!(list.rows, vec!["added 7".to_string()]);
        assert!(!list.truncated, "one change is under the cap");
        assert!(!list.is_empty());
        // A write that moved nothing reads empty, never a phantom row.
        let none = ChangeList::bounded(4, 4, 0, 0, 0, Vec::new());
        assert!(none.is_empty());
        assert!(none.rows.is_empty());
        assert!(!none.truncated);
    }

    #[test]
    fn change_list_cap_truncates_rows_but_counts_stay_exact() {
        let rows: Vec<String> = (0..MAX_CHANGES + 7).map(|i| format!("moved {i}")).collect();
        let total = u32::try_from(rows.len()).expect("fits in u32");
        let list = ChangeList::bounded(0, 1, 0, 0, total, rows);
        assert!(list.truncated, "over the cap must flag truncation");
        assert_eq!(list.rows.len(), MAX_CHANGES, "rows clip to the cap");
        assert_eq!(list.moved, total, "the count stays the full total");
        // Exactly at the cap is not a truncation: rows <= MAX_CHANGES.
        let rows: Vec<String> = (0..MAX_CHANGES).map(|i| format!("moved {i}")).collect();
        let exact = ChangeList::bounded(0, 1, 0, 0, MAX_CHANGES as u32, rows);
        assert!(!exact.truncated, "the cap itself never truncates");
        assert_eq!(exact.rows.len(), MAX_CHANGES);
    }
}
