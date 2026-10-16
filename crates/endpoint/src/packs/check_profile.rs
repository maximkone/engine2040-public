//! Door check/profile/overlay helpers: hinted refusals plus staged costs.
//! Consumer, units, determinism, errors: door pre-checks, integer math only.
/// Check verb name.
pub const TOOL_DOOR_CHECK: &str = "door.check";
/// Profile verb name.
pub const TOOL_DOOR_PROFILE: &str = "door.profile";
/// Overlay verb name.
pub const TOOL_DOOR_OVERLAY: &str = "door.debug_overlay";
/// One hinted refusal: code plus path and hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckIssue {
    /// Machine code.
    pub code: &'static str,
    /// Rule path.
    pub path: String,
    /// Fix hint with one recovery.
    pub hint: String,
}
impl CheckIssue {
    /// New hinted issue via the guard tables.
    pub fn hinted(code: &'static str) -> Self {
        Self {
            code,
            path: super::refusal_path(code).to_owned(),
            hint: super::hint_line(code),
        }
    }
}
/// Profile verdict: stages plus whether they sum within 5%.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileVerdict {
    /// Stage costs: sim, present, fighters. Units: us.
    pub stages: [u32; 3],
    /// Whole-frame cost. Units: us.
    pub whole_us: u32,
    /// True when stages sum to whole within 5%.
    pub within_5pct: bool,
}
/// Overlay request shape. Pixels stay in render; the door files this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRequest {
    /// Draw collider boxes.
    pub colliders: bool,
    /// Draw velocity arrows.
    pub velocity_arrows: bool,
    /// Draw target rings.
    pub targets: bool,
    /// PNG path the render worker writes.
    pub png_path: String,
}
/// Check intent plus payload as hinted issues.
pub fn door_check(intent: &str, payload: &str) -> Vec<CheckIssue> {
    let mut out = Vec::new();
    if super::check_intent(intent).is_err() {
        let code = if intent.is_empty() {
            "missing_intent"
        } else {
            "intent_too_long"
        };
        out.push(CheckIssue::hinted(code));
    }
    if let Some(r) = crate::sandbox::scan::scan_payload(payload) {
        out.push(CheckIssue::hinted(r.code));
    }
    #[cfg(feature = "mcp")]
    let _ = engine_spatial::checks::SPATIAL_CHECKS.len();
    out
}
/// Profile stages against the whole frame.
pub fn door_profile(stages: [u32; 3], whole_us: u32) -> ProfileVerdict {
    #[cfg(feature = "mcp")]
    if let Ok(s) = engine_observe::fps::FpsFrameSplit::from_stages(0, stages) {
        return ProfileVerdict {
            stages,
            whole_us,
            within_5pct: s.sums_within(whole_us),
        };
    }
    let sum = u64::from(stages[0]) + u64::from(stages[1]) + u64::from(stages[2]);
    let whole = u64::from(whole_us);
    let within = if whole == 0 {
        sum == 0
    } else {
        sum.abs_diff(whole).saturating_mul(100) <= whole.saturating_mul(5)
    };
    ProfileVerdict {
        stages,
        whole_us,
        within_5pct: within,
    }
}
/// File an overlay request; render owns pixels.
pub fn file_overlay_request(png_path: &str) -> OverlayRequest {
    OverlayRequest {
        colliders: true,
        velocity_arrows: true,
        targets: true,
        png_path: png_path.to_owned(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hinted_refusals_carry_code_path_hint() {
        let markers = crate::sandbox::scan::SCAN_MARKERS;
        for marker in markers.iter().take(12) {
            let issues = door_check("move pawn north", marker);
            assert_eq!(issues.len(), 1, "marker `{marker}` must refuse");
            assert_eq!(issues[0].code, "hostile_text");
            assert!(issues[0].path.starts_with("door.check/"));
            assert!(issues[0].hint.contains("re-run"));
        }
        let bad = door_check("", "fresh bread at dawn");
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].code, "missing_intent");
        assert!(door_check("move pawn north", "fresh bread").is_empty());
    }
    #[test]
    fn profile_sums_within_five_percent() {
        assert!(door_profile([120, 2300, 400], 2820).within_5pct);
        assert!(!door_profile([100, 100, 100], 1000).within_5pct);
    }
}
