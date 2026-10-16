//! MV1 workflow receipt: the M1 harness record, std-only.
//!
//! Consumer: the MV1 gate (`tests/m1_workflow.rs` runs the real workflow
//! through the assets/editor/scene/render APIs and reports each step here).
//! Data owner: this module owns only the receipt shape and its hand-rolled
//! text encoding — no fixture bytes, no scene graph, no renderer logic.
//! Determinism scope: [`emit_receipt`] is a pure function of its input; the
//! same steps and digests always print the same text. Errors: none — every
//! input is bounded (names truncated, details truncated, steps capped), so
//! hostile or chatty callers can never flood a log.
//!
//! The receipt is JSON-ish text written by hand (no new dependencies): an
//! object with the workflow name, the fixture row, one entry per step with
//! its pass/fail bit, and the recorded artifact digests.

/// Workflow name stamped on every MV1 receipt.
pub const WORKFLOW: &str = "m1_workflow";
/// Largest accepted step count; the workflow has eight steps.
pub const MAX_STEPS: usize = 16;
/// Largest accepted digest count.
pub const MAX_DIGESTS: usize = 16;
/// Upper bound on any name/digest-key/value string, in chars.
pub const MAX_TEXT_CHARS: usize = 256;
/// Upper bound on any step detail string, in chars.
pub const MAX_DETAIL_CHARS: usize = 512;

/// One workflow step plus its pass/fail bit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepOutcome {
    /// Step name (`import_fixture`, `place_edit`, ...).
    pub name: String,
    /// True when the step passed on the licensed fixture.
    pub pass: bool,
    /// Bounded human-readable summary (counts, revisions, ids avoided).
    pub detail: String,
}

/// One recorded artifact digest: what was hashed plus its hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestRow {
    /// Artifact label (`fixture_gltf`, `import_mesh`, ...).
    pub label: String,
    /// Hex digest of the artifact bytes.
    pub hex: String,
}

/// The MV1 receipt: workflow name, fixture row, steps and digests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    /// Always [`WORKFLOW`]; carried so the text is self-describing.
    pub workflow: String,
    /// Licensed fixture row (`<scene>: <license>`), bounded.
    pub fixture: String,
    /// One entry per workflow step, in run order.
    pub steps: Vec<StepOutcome>,
    /// Artifact digests recorded along the way, in run order.
    pub digests: Vec<DigestRow>,
}

impl Receipt {
    /// Empty receipt stamped with the workflow name.
    pub fn new(fixture: &str) -> Self {
        Self {
            workflow: WORKFLOW.to_string(),
            fixture: bound_text(fixture, MAX_TEXT_CHARS),
            steps: Vec::new(),
            digests: Vec::new(),
        }
    }

    /// Record one step outcome; extras past [`MAX_STEPS`] are ignored so a
    /// chatty harness can never grow the receipt without bound.
    pub fn push_step(&mut self, name: &str, pass: bool, detail: &str) {
        if self.steps.len() >= MAX_STEPS {
            return;
        }
        self.steps.push(StepOutcome {
            name: bound_text(name, MAX_TEXT_CHARS),
            pass,
            detail: bound_text(detail, MAX_DETAIL_CHARS),
        });
    }

    /// Record one artifact digest; extras past [`MAX_DIGESTS`] are ignored.
    pub fn push_digest(&mut self, label: &str, hex: &str) {
        if self.digests.len() >= MAX_DIGESTS {
            return;
        }
        self.digests.push(DigestRow {
            label: bound_text(label, MAX_TEXT_CHARS),
            hex: bound_text(hex, MAX_TEXT_CHARS),
        });
    }

    /// True when every recorded step passed (and at least one step ran).
    pub fn all_pass(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|step| step.pass)
    }
}

/// FNV-1a 64 hex of `bytes` (16 lowercase digits).
///
/// Std-only digest for receipt unit tests and tiny harness inputs. The
/// workflow itself records the assets crate's content-hash hex for fixture
/// bytes (same algorithm, same canon); this helper exists so the receipt
/// module needs no crate edge at all.
pub fn fnv_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Emit the receipt as JSON-ish text: one object with `workflow`, `fixture`,
/// `steps` (each with `name`, `pass`, `detail`) and `digests` (each with
/// `label`, `hex`), plus the aggregate `all_pass` bit.
///
/// Hand-rolled on purpose (no new dependencies): strings are escaped
/// (`"`, `\`, controls) and every field was already bounded at record time.
pub fn emit_receipt(receipt: &Receipt) -> String {
    let mut out = String::from("{\"workflow\":\"");
    escape_into(&mut out, &receipt.workflow);
    out.push_str("\",\"fixture\":\"");
    escape_into(&mut out, &receipt.fixture);
    out.push_str("\",\"all_pass\":");
    out.push_str(if receipt.all_pass() { "true" } else { "false" });
    out.push_str(",\"steps\":[");
    for (index, step) in receipt.steps.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":\"");
        escape_into(&mut out, &step.name);
        out.push_str("\",\"pass\":");
        out.push_str(if step.pass { "true" } else { "false" });
        out.push_str(",\"detail\":\"");
        escape_into(&mut out, &step.detail);
        out.push_str("\"}");
    }
    out.push_str("],\"digests\":[");
    for (index, digest) in receipt.digests.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"label\":\"");
        escape_into(&mut out, &digest.label);
        out.push_str("\",\"hex\":\"");
        escape_into(&mut out, &digest.hex);
        out.push_str("\"}");
    }
    out.push_str("]}");
    out
}

/// Truncate `text` to at most `max` chars at a char boundary.
fn bound_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

/// Append `text` with JSON string escaping.
fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitter_names_workflow_steps_and_digests() {
        let mut receipt = Receipt::new("Triangle.gltf: CC0-1.0");
        receipt.push_step("import_fixture", true, "1 mesh, 3 verts");
        receipt.push_step("place_edit", true, "revision 4, 3 nodes");
        receipt.push_digest("fixture_gltf", &fnv_hex(b"abc"));
        let text = emit_receipt(&receipt);
        assert!(text.contains("\"workflow\":\"m1_workflow\""));
        assert!(text.contains("\"all_pass\":true"));
        assert!(text.contains("\"name\":\"import_fixture\",\"pass\":true"));
        assert!(text.contains("\"name\":\"place_edit\",\"pass\":true"));
        assert!(text.contains(&fnv_hex(b"abc")));
        // Deterministic: same input, same text.
        assert_eq!(text, emit_receipt(&receipt));
    }

    #[test]
    fn failing_step_clears_the_aggregate_bit() {
        let mut receipt = Receipt::new("fixture");
        receipt.push_step("import_fixture", true, "ok");
        receipt.push_step("child_play", false, "stub refused to die");
        assert!(!receipt.all_pass());
        let text = emit_receipt(&receipt);
        assert!(text.contains("\"all_pass\":false"));
        assert!(text.contains("\"name\":\"child_play\",\"pass\":false"));
    }

    #[test]
    fn empty_receipt_never_reports_all_pass() {
        let receipt = Receipt::new("fixture");
        assert!(!receipt.all_pass());
        assert!(emit_receipt(&receipt).contains("\"steps\":[]"));
    }

    #[test]
    fn hostile_strings_escape_and_stay_bounded() {
        let mut receipt = Receipt::new("fix\"ture\\with\ncontrols\u{1}");
        receipt.push_step("step\"\\", true, &"d".repeat(MAX_DETAIL_CHARS + 100));
        receipt.push_digest(&"l".repeat(MAX_TEXT_CHARS + 40), "aa\"bb");
        let text = emit_receipt(&receipt);
        assert!(text.contains("fix\\\"ture\\\\with\\ncontrols\\u0001"));
        assert!(text.contains("aa\\\"bb"));
        for step in &receipt.steps {
            assert!(step.detail.chars().count() <= MAX_DETAIL_CHARS);
        }
        for digest in &receipt.digests {
            assert!(digest.label.chars().count() <= MAX_TEXT_CHARS);
        }
    }

    #[test]
    fn overfull_receipt_keeps_first_entries_only() {
        let mut receipt = Receipt::new("fixture");
        for i in 0..(MAX_STEPS + 5) {
            receipt.push_step(&format!("step{i}"), true, "ok");
        }
        for i in 0..(MAX_DIGESTS + 5) {
            receipt.push_digest(&format!("label{i}"), "00");
        }
        assert_eq!(receipt.steps.len(), MAX_STEPS);
        assert_eq!(receipt.digests.len(), MAX_DIGESTS);
        assert!(receipt.all_pass());
    }

    #[test]
    fn fnv_hex_is_stable_and_sensitive() {
        assert_eq!(fnv_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv_hex(b"a"), fnv_hex(b"a"));
        assert_ne!(fnv_hex(b"a"), fnv_hex(b"b"));
    }
}
