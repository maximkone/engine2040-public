//! Diagnostic render vectors: graphical refusal shape, std-only.
//!
//! Consumer: the MCP door reviewer and any weak model that must pick the
//! refusal cause in one round trip. Each vector starts from one real
//! [`engine_endpoint::guards::GuardError`], renders a small graphical block
//! (header plus labelled snippet plus help), and asserts the block names the
//! machine `code`, shows a human `label`, quotes the request `snippet`, and
//! never leaks simulation tick bytes. Data owner stays the endpoint guard
//! module; this file owns only the presentation shape and the seeded table.
//! Units: refusal cases and rendered lines. Determinism: seed 2040 always
//! builds the same 20 refusals on every machine; integer math only, no wall
//! clock, no float, no network. Errors are panics via asserts.
//!
//! Attribution: shape idea (header line, gutter box, span pointer, help
//! footer) follows zkat/miette `src/handlers/graphical.rs` (Apache-2.0,
//! license verified at <https://github.com/zkat/miette>). Every line below
//! is fresh Rust written against our `GuardError` API; no donor code is
//! copied, vendored, or quoted.
//!
//! Cause-pick lift: the rendered block puts the machine code in the header
//! and points one span line at the culprit snippet, so the picker reads the
//! cause without reopening the request.
use engine_core::Rng;
use engine_endpoint::guards::{self, GuardError, VerbClass};
const SEED: u64 = 2040;
const VECTOR_COUNT: usize = 20;
struct Vector {
    error: GuardError,
    snippet: String,
}
fn label_of(error: &GuardError) -> String {
    let first = error.message.split(';').next().unwrap_or(&error.message);
    let one_line: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.len() > 120 {
        let mut end = 120;
        while !one_line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &one_line[..end])
    } else {
        one_line
    }
}
fn render_graphical(error: &GuardError, snippet: &str) -> String {
    let label = label_of(error);
    let mut out = String::new();
    out.push_str(&format!("error[{}]\n", error.code));
    out.push_str(&format!("  x {label}\n"));
    out.push_str("   open-[request snippet]\n");
    out.push_str(&format!("   | {snippet}\n"));
    out.push_str(&format!("   +-> {} here\n", error.code));
    out.push_str("   end-> see the snippet line above, then re-run\n");
    out.push_str(&format!("  help: {}\n", error.message));
    out
}
fn refusal_for(draw: u64, index: usize) -> GuardError {
    match draw % 7 {
        0 => guards::check_intent("").expect_err("empty intent refuses"),
        1 => {
            let long = "w".repeat(guards::INTENT_MAX_BYTES + 1 + (index % 8));
            guards::check_intent(&long).expect_err("over-long intent refuses")
        }
        2 => {
            let bad = format!("bad-{index}-{draw}");
            guards::check_dry_run_id(&bad).expect_err("bad receipt refuses")
        }
        3 => {
            let mut id = String::from(guards::DRY_RUN_ID_PREFIX);
            while id.len() <= guards::DRY_RUN_ID_MAX_BYTES + (index % 4) {
                id.push('r');
            }
            guards::check_dry_run_id(&id).expect_err("over-long receipt refuses")
        }
        4 => guards::check_args_ops(0).expect_err("empty batch refuses"),
        5 => guards::check_args_ops(guards::ARGS_MAX_OPS + 1 + (index % 3))
            .expect_err("oversized batch refuses"),
        _ => guards::check_policy(VerbClass::Destructive, false, 0)
            .expect_err("low tier refuses deletes"),
    }
}
fn snippet_for(draw: u64, index: usize) -> String {
    let ops = draw % 300;
    format!(
        "case {index} intent_len={} receipt=rcpt_{} ops={ops} tier=0",
        (draw >> 16) % 2100,
        draw % 1000
    )
}
fn seeded_vectors() -> Vec<Vector> {
    let mut rng = Rng::new(SEED);
    let mut out = Vec::with_capacity(VECTOR_COUNT);
    for index in 0..VECTOR_COUNT {
        let draw = rng.next_u64();
        let error = refusal_for(draw, index);
        let snippet = snippet_for(draw, index);
        out.push(Vector { error, snippet });
    }
    out
}
#[test]
fn twenty_seeded_refusals_render_code_label_and_snippet() {
    let vectors = seeded_vectors();
    assert_eq!(vectors.len(), VECTOR_COUNT, "table holds 20 refusals");
    let again = seeded_vectors();
    for (first, second) in vectors.iter().zip(again.iter()) {
        assert_eq!(first.error, second.error, "same seed, same refusal");
        assert_eq!(first.snippet, second.snippet, "same seed, same snippet");
    }
    let mut codes: Vec<&str> = vectors.iter().map(|v| v.error.code).collect();
    codes.sort_unstable();
    codes.dedup();
    assert!(
        codes.len() >= 4,
        "seed 2040 must cover 4+ codes, got: {codes:?}"
    );
    let mut rendered_count = 0usize;
    for vector in &vectors {
        let rendered = render_graphical(&vector.error, &vector.snippet);
        assert!(
            rendered.contains(&format!("error[{}]", vector.error.code)),
            "render names the code header: {rendered}"
        );
        assert!(
            rendered.contains(vector.error.code),
            "render names the code: {rendered}"
        );
        let label = label_of(&vector.error);
        assert!(!label.is_empty(), "label stays non-empty");
        assert!(
            rendered.contains(&label),
            "render shows the label: {rendered}"
        );
        assert!(
            rendered.contains(&vector.snippet),
            "render quotes the snippet: {rendered}"
        );
        assert!(
            !rendered.contains("tick"),
            "render leaks no tick bytes: {rendered}"
        );
        let second = render_graphical(&vector.error, &vector.snippet);
        assert_eq!(rendered, second, "same refusal twice, same text");
        rendered_count += 1;
    }
    assert_eq!(rendered_count, 20, "all 20 seeded refusals render");
}
#[test]
fn graphical_shape_has_header_gutter_span_and_help() {
    let vectors = seeded_vectors();
    let first = vectors.first().expect("table holds 20 refusals");
    let rendered = render_graphical(&first.error, &first.snippet);
    assert!(
        rendered.starts_with(&format!("error[{}]", first.error.code)),
        "header names the code first: {rendered}"
    );
    assert!(rendered.contains("open-"), "snippet box opens: {rendered}");
    assert!(rendered.contains('|'), "snippet gutter quotes: {rendered}");
    assert!(
        rendered.contains("->"),
        "span pointer picks the cause: {rendered}"
    );
    assert!(
        rendered.contains(&format!("{} here", first.error.code)),
        "span names the cause code: {rendered}"
    );
    assert!(
        rendered.contains("end->"),
        "next-step line rides: {rendered}"
    );
    assert!(rendered.contains("help:"), "help footer rides: {rendered}");
    assert!(
        rendered.contains(&first.error.message),
        "help carries the full detail: {rendered}"
    );
    let second = vectors.get(1).expect("table holds 20 refusals");
    let other = render_graphical(&second.error, &second.snippet);
    assert!(
        other.starts_with(&format!("error[{}]", second.error.code)),
        "second header names its code: {other}"
    );
    assert!(
        other.contains('|') && other.contains("help:"),
        "shape repeats: {other}"
    );
}
#[test]
fn cause_pick_points_at_culprit_and_names_recovery() {
    const RECOVERIES: [&str; 5] = ["re-run", "preview", "split", "shorten", "session"];
    let vectors = seeded_vectors();
    for vector in &vectors {
        let rendered = render_graphical(&vector.error, &vector.snippet);
        assert!(
            rendered.contains(&vector.snippet),
            "cause points at the snippet: {rendered}"
        );
        let help = rendered
            .lines()
            .find(|line| line.trim_start().starts_with("help:"))
            .expect("help footer rides");
        assert!(
            RECOVERIES.iter().any(|verb| help.contains(verb)),
            "help names one recovery: {rendered}"
        );
    }
}
