//! The judge's JSON body: fixed field order, integers only.
//!
//! Consumer: `super::judge`, which writes the body and digests it. Data
//! owner: the judge owns the field order; this module only renders. Units:
//! the same integers the typed report carries. Determinism: the same report
//! gives byte-identical text twice. Errors: none: the render cannot fail.
//!
//! The body is a separate module so the judge file stays inside the file
//! size the quality ratchet sets; every function here is one step of the
//! same render.

use std::fmt;

use super::judge::{FrameReport, LookJudgement, Q1Match, RubricItem, Sequence};
use super::metric::Comparison;

/// A quoted JSON string.
fn json_str(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One value or `null`.
fn json_opt<T: fmt::Display>(out: &mut String, value: Option<T>) {
    match value {
        Some(v) => out.push_str(&v.to_string()),
        None => out.push_str("null"),
    }
}

/// A list, each item written by `write`.
fn json_list<T>(out: &mut String, items: &[T], mut write: impl FnMut(&mut String, &T)) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write(out, item);
    }
    out.push(']');
}

/// One rubric item.
fn write_item(out: &mut String, item: &RubricItem) {
    out.push_str(&format!("{{\"id\":\"{}\",\"name\":\"{}\",\"applicable\":{},\"score\":{},\"pass\":{},\"frames_passing\":{},\"detail\":", item.id, item.name, item.applicable, item.score_permille, item.pass, item.frames_passing));
    json_str(out, &item.detail);
    out.push('}');
}

/// One metric comparison.
fn write_comparison(out: &mut String, c: &Comparison) {
    out.push_str(&format!(
        "{{\"ssim\":{},\"mean_de\":{},\"hist_tv\":{},\"hist_emd\":{},\"flip\":{}}}",
        c.ssim_x10000, c.mean_de_x100, c.hist_tv_x10000, c.hist_emd_x10000, c.flip_x10000
    ));
}

/// One Q1 match, with the second metric beside the SSIM score.
fn write_q1(out: &mut String, q: &Q1Match) {
    out.push_str("{\"name\":");
    json_str(out, &q.name);
    out.push_str(",\"comparison\":");
    match &q.comparison {
        Some(c) => write_comparison(out, c),
        None => out.push_str("null"),
    }
    out.push_str(",\"ssimulacra2\":");
    json_opt(out, q.ssimulacra2_x10000);
    out.push('}');
}

/// One frame report.
fn write_frame(out: &mut String, f: &FrameReport) {
    out.push_str("{\"name\":");
    json_str(out, &f.name);
    out.push_str(&format!(
        ",\"png_fnv\":\"{}\",\"pixel_fnv\":\"{}\",\"width\":{},\"height\":{},\"fault\":",
        f.png_fnv, f.pixel_fnv, f.width, f.height
    ));
    match &f.fault {
        Some(words) => json_str(out, words),
        None => out.push_str("null"),
    }
    out.push_str(",\"manifest_tick\":");
    json_opt(out, f.manifest_tick);
    out.push_str(",\"manifest_png_ok\":");
    json_opt(out, f.manifest_png_ok);
    out.push_str(",\"scores\":");
    json_list(out, &f.scores, |o, s| json_opt(o, *s));
    out.push_str(",\"raw\":");
    json_list(out, &f.raw, |o, r| json_opt(o, *r));
    out.push_str(&format!(
        ",\"fighters\":{{\"count\":{},\"pixels\":{},\"regions\":",
        f.fighters.count, f.fighters.pixels
    ));
    json_list(out, &f.fighters.regions, |o, r| {
        o.push_str("{\"entity\":");
        json_opt(o, r.entity);
        o.push_str(&format!(
            ",\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"pixels\":{}}}",
            r.x, r.y, r.w, r.h, r.pixels
        ));
    });
    out.push_str(",\"manifest_slots\":");
    json_list(out, &f.fighters.manifest_slots, |o, s| {
        o.push_str(&s.to_string());
    });
    out.push_str(",\"manifest_unseen\":");
    json_list(out, &f.fighters.manifest_unseen, |o, s| {
        o.push_str(&s.to_string());
    });
    out.push_str("},\"preview\":");
    match &f.preview {
        Some(p) => {
            out.push_str("{\"name\":");
            json_str(out, &p.name);
            out.push_str(&format!(
                ",\"emd\":{},\"mean_de\":{}}}",
                p.emd_x10000, p.mean_de_x100
            ));
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"q1\":");
    match &f.q1 {
        Some(q) => write_q1(out, q),
        None => out.push_str("null"),
    }
    out.push('}');
}

/// The sequence findings.
fn write_sequence(out: &mut String, s: &Sequence) {
    out.push_str(&format!(
        "{{\"frames\":{},\"distinct\":{},\"frozen\":{},\"motion_ssim\":",
        s.frames, s.distinct, s.frozen
    ));
    json_list(out, &s.motion_ssim_x10000, |o, m| json_opt(o, *m));
    out.push_str(&format!(",\"flicker\":{}}}", s.flicker_permille));
}

/// Every JSON member after the digest, without the braces: the bytes the
/// digest covers.
#[must_use]
pub fn body(judgement: &LookJudgement) -> String {
    let mut out = format!(
        "\"pass\":{},\"score\":{},\"integrity_ok\":{},\"rubric\":",
        judgement.pass, judgement.score_permille, judgement.integrity_ok
    );
    json_list(&mut out, &judgement.rubric, write_item);
    out.push_str(",\"sequence\":");
    write_sequence(&mut out, &judgement.sequence);
    out.push_str(",\"frames\":");
    json_list(&mut out, &judgement.frames, write_frame);
    out.push_str(",\"notes\":");
    json_list(&mut out, &judgement.notes, |o, n| json_str(o, n));
    out
}
