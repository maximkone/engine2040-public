//! Small pure helpers every spatial-facing door shares: quaternion yaw
//! conversion, brief finding lines and conflict-to-source-text mapping.

/// Y yaw extracted from a world quaternion (`[x, y, z, w]` like Transform).
/// Identity reads zero; pure-Y rotations read their angle exactly. The caller
/// guarantees finite lanes. Test-only: the one scene adapter
/// ([`engine_editor::spatial_view`]) carries yaw now, so production code never
/// reads a quaternion back.
#[cfg(test)]
pub(crate) fn yaw_of_quat(q: [f32; 4]) -> f32 {
    let [x, y, z, w] = q;
    (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// Pure-Y rotation quaternion for a solved yaw.
pub(crate) fn quat_of_yaw(yaw: f32) -> [f32; 4] {
    let half = 0.5 * yaw;
    [0.0, half.sin(), 0.0, half.cos()]
}

/// One finding rendered exactly like the SP4 brief lines (`finding <check>
/// <a> <b|-> <severity> <pct>` plus newline), so spatial rows compare
/// directly against brief text.
pub(crate) fn finding_line(finding: &engine_spatial::Finding) -> String {
    format!("{}\n", finding.row())
}

/// A parsed place want plus its source text (conflicts echo source texts).
pub(crate) struct ParsedWant {
    pub(crate) want: engine_spatial::Want,
    pub(crate) source: String,
}

/// Unmet solver wants back to source texts. The conflict set is a
/// subsequence of the intent in intent order, so an ordered multiset
/// subtraction finds each entry exactly once.
pub(crate) fn conflict_texts(parsed: &[ParsedWant], unmet: &[engine_spatial::Want]) -> Vec<String> {
    let mut rest: Vec<engine_spatial::Want> = unmet.to_vec();
    let mut out = Vec::new();
    for entry in parsed {
        if let Some(position) = rest.iter().position(|other| *other == entry.want) {
            rest.remove(position);
            out.push(entry.source.clone());
        }
    }
    out
}
