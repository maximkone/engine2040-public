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

#[cfg(test)]
mod tests {
    use super::*;

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
}
