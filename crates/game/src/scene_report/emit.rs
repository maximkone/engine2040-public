//! emit: the scene report as one JSON line and as readable text.
//!
//! Both forms are written from the same measured rows, so they cannot
//! disagree. The JSON embeds the text (`text`, one string per line) so a
//! consumer that reads one line of a wire gets both. Numbers print to three
//! places (centimeter and percent fields to one), a non-finite number prints
//! as `null`, ids and names are quoted and escaped. Same input, same bytes.

use std::fmt::Write as _;

use super::geom::{scale, sub, unit, P3};
use super::{joint_at, pct, sun_elev_deg, Row, SceneInput, Shape, Thing, SCHEMA, STUCK_BONE_CM};
use crate::view_truth::{cover, screen_rect};

/// A float to three places, or `null`.
pub fn n3(v: f32) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        "null".to_owned()
    }
}

/// A float to one place, or `null`.
fn n1(v: f32) -> String {
    if v.is_finite() {
        format!("{v:.1}")
    } else {
        "null".to_owned()
    }
}

/// A vector to three places.
pub fn v3(v: P3) -> String {
    format!("[{},{},{}]", n3(v[0]), n3(v[1]), n3(v[2]))
}

/// A rect to three places.
fn r4(v: [f32; 4]) -> String {
    format!("[{},{},{},{}]", n3(v[0]), n3(v[1]), n3(v[2]), n3(v[3]))
}

/// A JSON string with the escapes the format needs.
pub fn q(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A `[..]` of already-rendered items.
pub fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
}

/// `{"k":v,..}` from rendered pairs.
pub fn obj(pairs: &[(&str, String)]) -> String {
    let body: Vec<String> = pairs.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
    format!("{{{}}}", body.join(","))
}

/// NDC rect to a pixel rect `[left, top, right, bottom]`, y down.
fn px_rect(rect: [f32; 4], size: (u32, u32)) -> [f32; 4] {
    let (w, h) = (
        f32::from(u16::try_from(size.0).unwrap_or(u16::MAX)),
        f32::from(u16::try_from(size.1).unwrap_or(u16::MAX)),
    );
    [
        (rect[0] + 1.0) * 0.5 * w,
        (1.0 - (rect[3] + 1.0) * 0.5) * h,
        (rect[2] + 1.0) * 0.5 * w,
        (1.0 - (rect[1] + 1.0) * 0.5) * h,
    ]
}

/// Heading of a direction in degrees, from +z toward +x.
fn heading_deg(d: P3) -> f32 {
    d[0].atan2(d[2]).to_degrees()
}

/// A pixel rect for text: `[l,t,r,b]` to whole pixels.
fn px_text(rect: [f32; 4]) -> String {
    format!(
        "[{:.0},{:.0},{:.0},{:.0}]",
        rect[0], rect[1], rect[2], rect[3]
    )
}

/// The camera object.
fn camera_json(input: &SceneInput) -> String {
    let cam = &input.camera;
    let fwd = unit(sub(cam.target.to_array(), cam.eye.to_array())).unwrap_or([0.0, 0.0, -1.0]);
    obj(&[
        ("eye", v3(cam.eye.to_array())),
        ("target", v3(cam.target.to_array())),
        ("up", v3(cam.up.to_array())),
        ("fov_deg", n1(cam.fov_y.to_degrees())),
        ("aspect", n3(cam.aspect)),
        ("near", n3(cam.near)),
        ("far", n3(cam.far)),
        ("forward", v3(fwd)),
        ("heading_deg", n1(heading_deg(fwd))),
        ("pitch_deg", n1(fwd[1].clamp(-1.0, 1.0).asin().to_degrees())),
        ("shot", q(&input.shot)),
        ("mode", q(&input.mode)),
    ])
}

/// The light, the sun map and the pass switches.
fn light_json(input: &SceneInput) -> String {
    let sun = input.light.sun_dir;
    let map = &input.shadow;
    obj(&[
        ("sun_dir", v3(sun)),
        ("sun_elev_deg", n1(sun_elev_deg(sun))),
        (
            "sky_sun_dir",
            input
                .light
                .sky_sun_dir
                .map_or_else(|| "null".to_owned(), v3),
        ),
        ("sun_heading_deg", n1(heading_deg(sun))),
        ("color", v3(input.light.color)),
        ("intensity", n3(input.light.intensity)),
        (
            "shadow_map",
            obj(&[
                ("on", map.on.to_string()),
                ("edge_px", map.edge_px.to_string()),
                ("box_m", n1(map.half_extent_m * 2.0)),
                ("texel_cm", n1(map.texel_m * 100.0)),
            ]),
        ),
        (
            "renderer",
            q(if input.renderer_live {
                "live"
            } else {
                "shipped_defaults"
            }),
        ),
        ("contact_shadow", n3(input.contact)),
        (
            "passes",
            obj(&input
                .passes
                .iter()
                .map(|(name, on)| (name.as_str(), on.to_string()))
                .collect::<Vec<_>>()),
        ),
    ])
}

/// `[{"id":..,"share":..},..]` for blockers.
fn blockers(input: &SceneInput, by: &[(usize, f32)]) -> String {
    list(
        by.iter()
            .map(|(j, s)| obj(&[("id", q(&input.things[*j].id)), ("share", n3(*s))])),
    )
}

/// The screen block of a thing, or `null` when it has no rect.
fn screen_json(input: &SceneInput, row: &Row) -> String {
    match row.screen {
        None => "null".to_owned(),
        Some(rect) => obj(&[
            ("ndc", r4(rect)),
            ("px", r4(px_rect(rect, input.viewport))),
            ("cover", n3(row.cover)),
        ]),
    }
}

/// The sun block of a thing.
fn sun_json(input: &SceneInput, row: &Row) -> String {
    let sun = &row.sun;
    let shadow = sun.footprint.map_or_else(
        || "null".to_owned(),
        |f| {
            obj(&[
                ("xz", r4(f.xz)),
                ("len_m", n3(f.len_m)),
                ("screen", f.screen.map_or_else(|| "null".to_owned(), r4)),
            ])
        },
    );
    obj(&[
        ("lit", n3(sun.lit)),
        ("shaded", n3(sun.blocked)),
        ("shaded_by", blockers(input, &sun.by)),
        ("map", q(sun.casts)),
        ("shadow_on_sand", shadow),
    ])
}

/// The visibility block of a thing.
fn vis_json(input: &SceneInput, row: &Row) -> String {
    obj(&[
        ("visible", n3(row.vis.visible)),
        ("hidden", n3(row.vis.occluded)),
        ("outside", n3(row.vis.offscreen)),
        ("hidden_by", blockers(input, &row.vis.by)),
    ])
}

/// The touching list of a thing.
fn touch_json(input: &SceneInput, row: &Row) -> String {
    list(row.touch.iter().map(|t| {
        obj(&[
            ("id", q(&input.things[t.other].id)),
            ("gap_cm", n1(t.gap_cm)),
            ("overlap_cm", n1(t.overlap_cm)),
        ])
    }))
}

/// The fixed joint set a fighter prints.
const KEY_JOINTS: [&str; 7] = [
    "head", "torso", "pelvis", "palm_r", "palm_l", "foot_r", "foot_l",
];

/// The parts skinned to a hand, each with its bounds and screen rect: the
/// weapons and shields a fighter carries, measured like any other thing.
fn held_parts_json(input: &SceneInput, f: &super::FighterFacts) -> String {
    list(f.held.iter().map(|part| {
        let (lo, hi) = part.bounds;
        let rect = screen_rect(&input.camera, lo, hi);
        obj(&[
            ("name", q(&part.name)),
            ("hand", q(&part.hand.to_string())),
            ("min", v3(lo)),
            ("max", v3(hi)),
            ("len_m", n3(part.len_m)),
            (
                "screen",
                rect.map_or_else(
                    || "null".to_owned(),
                    |r| obj(&[("ndc", r4(r)), ("cover", n3(cover(r)))]),
                ),
            ),
        ])
    }))
}

/// The fighter-only block.
fn fighter_json(input: &SceneInput, thing: &Thing, row: &Row) -> Vec<(&'static str, String)> {
    let Some(f) = thing.fighter.as_ref() else {
        return Vec::new();
    };
    let joints = obj(&KEY_JOINTS
        .iter()
        .filter_map(|name| joint_at(thing, name).map(|j| (*name, v3(j.at))))
        .collect::<Vec<_>>());
    let floor = row.floor.map_or_else(
        || "null".to_owned(),
        |fl| {
            obj(&[
                ("grounded", fl.grounded.to_string()),
                ("min_y_cm", n1(fl.min_y_cm)),
                (
                    "standing_on",
                    fl.standing_on
                        .map_or_else(|| q("ground"), |j| q(&input.things[j].id)),
                ),
                (
                    "feet_cm",
                    obj(&[
                        ("l", fl.feet_cm[0].map_or_else(|| "null".to_owned(), n1)),
                        ("r", fl.feet_cm[1].map_or_else(|| "null".to_owned(), n1)),
                    ]),
                ),
            ])
        },
    );
    let hands = list(row.hands.iter().map(|h| {
        obj(&[
            ("hand", q(&h.hand.to_string())),
            ("at", h.at.map_or_else(|| "null".to_owned(), v3)),
            (
                "holds",
                list(
                    h.holds
                        .iter()
                        .map(|(item, how)| obj(&[("item", q(item)), ("how", q(how))])),
                ),
            ),
        ])
    }));
    let size = thing.bounds.map_or([0.0; 3], |(lo, hi)| sub(hi, lo));
    vec![
        ("slot", f.slot.to_string()),
        ("act", q(&f.act)),
        ("clip", f.clip.to_string()),
        ("tint", f.tinted.to_string()),
        ("ragdoll", f.ragdolled.to_string()),
        (
            "hp",
            f.hp.map_or_else(|| "null".to_owned(), |h| h.to_string()),
        ),
        (
            "armed",
            f.armed.as_deref().map_or_else(|| "null".to_owned(), q),
        ),
        ("root", v3(f.root)),
        ("yaw_deg", n1(f.yaw.to_degrees())),
        ("size", v3(size)),
        ("joints", joints),
        ("floor", floor),
        ("hands", hands),
        ("held_parts", held_parts_json(input, f)),
        (
            "slide_cm",
            obj(&[
                (
                    "l",
                    f.foot_slide_m[0].map_or_else(|| "null".to_owned(), |m| n1(m * 100.0)),
                ),
                (
                    "r",
                    f.foot_slide_m[1].map_or_else(|| "null".to_owned(), |m| n1(m * 100.0)),
                ),
            ]),
        ),
        ("error_parts", f.error_parts.to_string()),
        ("nonfinite", f.nonfinite.to_string()),
    ]
}

/// The spear-only block.
fn spear_json(input: &SceneInput, thing: &Thing, row: &Row) -> Vec<(&'static str, String)> {
    let Some(s) = thing.spear.as_ref() else {
        return Vec::new();
    };
    let tail = sub(s.tip, scale(s.dir, s.len_m));
    let stuck = row.stuck.as_ref().map_or_else(
        || "null".to_owned(),
        |st| match st.fighter {
            Some(v) => obj(&[
                ("kind", q("fighter")),
                ("id", q(&input.things[v].id)),
                (
                    "bone",
                    st.bone.as_deref().map_or_else(|| "null".to_owned(), q),
                ),
                ("bone_cm", n1(st.dist_cm)),
                ("tip_inside_bounds", st.tip_inside.to_string()),
            ]),
            None => obj(&[
                ("kind", q("world")),
                (
                    "near",
                    st.near
                        .map_or_else(|| "null".to_owned(), |j| q(&input.things[j].id)),
                ),
                ("near_cm", n1(st.dist_cm)),
            ]),
        },
    );
    vec![
        ("owner", s.owner.to_string()),
        ("state", q(s.state.name())),
        ("tip", v3(s.tip)),
        ("tail", v3(tail)),
        ("dir", v3(s.dir)),
        ("len_m", n3(s.len_m)),
        (
            "held_by",
            s.held_by
                .map_or_else(|| "null".to_owned(), |h| h.to_string()),
        ),
        ("stuck", stuck),
    ]
}

/// One thing as a JSON object.
fn thing_json(input: &SceneInput, i: usize, row: &Row) -> String {
    let thing = &input.things[i];
    let (min, max) = thing.bounds.map_or_else(
        || ("null".to_owned(), "null".to_owned()),
        |(lo, hi)| (v3(lo), v3(hi)),
    );
    let mut pairs: Vec<(&str, String)> = vec![
        ("id", q(&thing.id)),
        ("kind", q(thing.kind.name())),
        ("label", q(&thing.label)),
        ("solid", (!matches!(thing.shape, Shape::Hollow)).to_string()),
        ("drawn", thing.drawn.to_string()),
        ("min", min),
        ("max", max),
        ("depth_m", row.depth_m.map_or_else(|| "null".to_owned(), n3)),
        ("screen", screen_json(input, row)),
        ("vis", vis_json(input, row)),
        ("sun", sun_json(input, row)),
        ("touch", touch_json(input, row)),
    ];
    pairs.extend(fighter_json(input, thing, row));
    pairs.extend(spear_json(input, thing, row));
    if let Some(px) = row.px {
        pairs.push(("px", px.to_string()));
    }
    obj(&pairs)
}

/// The whole report as one JSON line.
pub(super) fn json(input: &SceneInput, rows: &[Row], findings: &[String], text: &str) -> String {
    let things = list(
        rows.iter()
            .enumerate()
            .map(|(i, row)| thing_json(input, i, row)),
    );
    let laws = obj(&[
        ("line", q(&input.laws.line)),
        ("verdict", q(&input.laws.verdict)),
        ("warnings", list(input.laws.warnings.iter().map(|w| q(w)))),
    ]);
    let pixels = input.pixels.as_ref().map_or_else(
        || "null".to_owned(),
        |p| {
            let by: Vec<(&str, String)> = p
                .by_id
                .iter()
                .map(|(id, n)| (id.as_str(), n.to_string()))
                .collect();
            obj(&[
                ("total", p.total.to_string()),
                ("clear", p.clear.to_string()),
                ("by", obj(&by)),
            ])
        },
    );
    obj(&[
        ("schema", SCHEMA.to_string()),
        ("source", q(input.source)),
        ("tick", input.tick.to_string()),
        ("seed", q(&input.seed.to_string())),
        (
            "viewport",
            format!("[{},{}]", input.viewport.0, input.viewport.1),
        ),
        ("camera", camera_json(input)),
        ("light", light_json(input)),
        ("things", things),
        ("pixels", pixels),
        ("laws", laws),
        ("findings", list(findings.iter().map(|f| q(f)))),
        ("text", list(text.lines().map(q))),
    ])
}

/// A vector for text, one place.
fn t3(v: P3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v[0], v[1], v[2])
}

/// The passes line: `name=on|off` per pass.
fn passes_text(input: &SceneInput) -> String {
    input
        .passes
        .iter()
        .map(|(name, on)| format!("{name}={}", if *on { "on" } else { "off" }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Blockers for a sentence: `prop_3 40%, fighter_1 10%`.
fn blockers_text(input: &SceneInput, by: &[(usize, f32)]) -> String {
    by.iter()
        .map(|(j, s)| format!("{} {}", input.things[*j].id, pct(*s)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The text lines of one fighter beyond the common ones.
fn fighter_text(input: &SceneInput, thing: &Thing, row: &Row, out: &mut Vec<String>) {
    let Some(f) = thing.fighter.as_ref() else {
        return;
    };
    if let Some(fl) = row.floor {
        let on = fl
            .standing_on
            .map_or("ground", |j| input.things[j].id.as_str());
        let feet = |v: Option<f32>| v.map_or_else(|| "-".to_owned(), |c| format!("{c:.1}"));
        out.push(format!(
            "    floor: {} on {on} (lowest point {:+.1} cm, ankles L {} R {} cm)",
            if fl.grounded {
                "grounded"
            } else {
                "NOT grounded"
            },
            fl.min_y_cm,
            feet(fl.feet_cm[0]),
            feet(fl.feet_cm[1]),
        ));
    }
    for hand in &row.hands {
        let at = hand.at.map_or_else(|| "no palm joint".to_owned(), t3);
        let holds = if hand.holds.is_empty() {
            "nothing".to_owned()
        } else {
            hand.holds
                .iter()
                .map(|(item, how)| format!("{item} ({how})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push(format!("    hand {}: palm {at} holds {holds}", hand.hand));
    }
    let armed = f.armed.as_deref().unwrap_or("bare");
    out.push(format!(
        "    armed: {armed}; error_parts={} nonfinite={}",
        f.error_parts, f.nonfinite
    ));
}

/// The text line of a stuck spear.
fn stuck_text(input: &SceneInput, row: &Row) -> Option<String> {
    let st = row.stuck.as_ref()?;
    Some(match st.fighter {
        Some(v) => format!(
            "    stuck in {}: nearest bone {} at {:.1} cm (limit {STUCK_BONE_CM:.0}), tip {} the body bounds",
            input.things[v].id,
            st.bone.as_deref().unwrap_or("?"),
            st.dist_cm,
            if st.tip_inside { "inside" } else { "OUTSIDE" },
        ),
        None => format!(
            "    stuck in the world: nearest prop {} at {:.1} cm",
            st.near.map_or("none", |j| input.things[j].id.as_str()),
            st.dist_cm
        ),
    })
}

/// The lines of one thing.
fn thing_text(input: &SceneInput, i: usize, row: &Row, out: &mut Vec<String>) {
    let thing = &input.things[i];
    let mut head = format!("{} [{}]", thing.id, thing.label);
    if !thing.drawn {
        head.push_str(" NOT DRAWN");
    }
    if let Some(f) = thing.fighter.as_ref() {
        let hp = f.hp.map_or_else(String::new, |h| format!(" hp={h}"));
        let _ = write!(
            head,
            " {} clip={}{hp}{}{} root {}",
            f.act,
            f.clip,
            if f.ragdolled { " ragdoll" } else { "" },
            if f.tinted { " tinted" } else { "" },
            t3(f.root)
        );
    }
    if let Some(s) = thing.spear.as_ref() {
        let _ = write!(
            head,
            " {} owner={} tip {}",
            s.state.name(),
            s.owner,
            t3(s.tip)
        );
    }
    if let Some((lo, hi)) = thing.bounds {
        let _ = write!(head, " bounds {} .. {}", t3(lo), t3(hi));
    }
    out.push(head);
    match row.screen {
        Some(rect) => out.push(format!(
            "    screen: {} of the picture, px {}, {:.1} m from the eye{}",
            pct(row.cover),
            px_text(px_rect(rect, input.viewport)),
            row.depth_m.unwrap_or(f32::NAN),
            row.px
                .map_or_else(String::new, |p| format!(", id buffer {p} px")),
        )),
        None => out.push("    screen: not in the picture".to_owned()),
    }
    let hid = if row.vis.by.is_empty() {
        String::new()
    } else {
        format!(" by {}", blockers_text(input, &row.vis.by))
    };
    out.push(format!(
        "    seen: visible {}, hidden {}{hid}, outside the frame {}",
        pct(row.vis.visible),
        pct(row.vis.occluded),
        pct(row.vis.offscreen)
    ));
    let shade = if row.sun.by.is_empty() {
        String::new()
    } else {
        format!(" by {}", blockers_text(input, &row.sun.by))
    };
    let on_sand = row.sun.footprint.map_or_else(String::new, |f| {
        format!(
            "; shadow on the sand {:.1} m long at x {:.1}..{:.1} z {:.1}..{:.1}",
            f.len_m, f.xz[0], f.xz[2], f.xz[1], f.xz[3]
        )
    });
    out.push(format!(
        "    sun: lit {}, shaded {}{shade}; in the sun map: {}{on_sand}",
        pct(row.sun.lit),
        pct(row.sun.blocked),
        row.sun.casts
    ));
    if !row.touch.is_empty() {
        let items: Vec<String> = row
            .touch
            .iter()
            .map(|t| {
                let how = if t.overlap_cm > 0.0 {
                    format!("overlap {:.1} cm", t.overlap_cm)
                } else {
                    format!("gap {:.1} cm", t.gap_cm)
                };
                format!("{} ({how})", input.things[t.other].id)
            })
            .collect();
        out.push(format!("    touching: {}", items.join(", ")));
    }
    fighter_text(input, thing, row, out);
    out.extend(stuck_text(input, row));
}

/// The readable form.
pub(super) fn text(input: &SceneInput, rows: &[Row], findings: &[String]) -> String {
    let cam = &input.camera;
    let fwd = unit(sub(cam.target.to_array(), cam.eye.to_array())).unwrap_or([0.0, 0.0, -1.0]);
    let sun = input.light.sun_dir;
    let mut out = vec![
        format!(
            "scene tick={} seed={} source={}{} viewport={}x{}",
            input.tick,
            input.seed,
            input.source,
            match input.source {
                "headless" => " (a windowless bout on the empty input tape)",
                "headless_showcase" => " (a windowless run of the showcase tour)",
                _ => "",
            },
            input.viewport.0,
            input.viewport.1
        ),
        format!(
            "camera eye {} -> target {} fov {:.0} deg, heading {:.0} deg, pitch {:.0} deg, shot {} ({})",
            t3(cam.eye.to_array()),
            t3(cam.target.to_array()),
            cam.fov_y.to_degrees(),
            heading_deg(fwd),
            fwd[1].clamp(-1.0, 1.0).asin().to_degrees(),
            input.shot,
            input.mode
        ),
        format!(
            "sun elevation {:.0} deg, heading {:.0} deg; sun map {} ({} px over {:.0} m, texel {:.1} cm); contact shadow {:.2}",
            sun_elev_deg(sun),
            heading_deg(sun),
            if input.shadow.on { "on" } else { "OFF" },
            input.shadow.edge_px,
            input.shadow.half_extent_m * 2.0,
            input.shadow.texel_m * 100.0,
            input.contact
        ),
        if input.renderer_live {
            format!("passes: {}", passes_text(input))
        } else {
            "passes: not read (a headless run has no renderer; the sun map is the shipped one)".to_owned()
        },
    ];
    for (i, row) in rows.iter().enumerate() {
        thing_text(input, i, row, &mut out);
    }
    out.push(format!(
        "laws: {} => {}",
        if input.laws.line.is_empty() {
            "(no frame folded yet)"
        } else {
            &input.laws.line
        },
        if input.laws.verdict.is_empty() {
            "unknown"
        } else {
            &input.laws.verdict
        }
    ));
    for warning in &input.laws.warnings {
        out.push(format!("  {warning}"));
    }
    if findings.is_empty() {
        out.push("findings: none".to_owned());
    } else {
        out.push(format!("findings ({}):", findings.len()));
        out.extend(findings.iter().map(|f| format!("  - {f}")));
    }
    out.join("\n")
}
