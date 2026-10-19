//! Measure, room, glance, coverage and the sense text verbs of `scene.query`:
//! `measure`, `room_of`, `in_room`, `exits_of`, `glance`, `coverage`, and the
//! SP69 text verbs `view`, `lidar`, `map` and `pixel`. Each answers from one
//! `engine-spatial` call and returns the figures an agent acts on (signed gap
//! and shortfall in cm, room numbers with exits, permille coverage per object,
//! and plain sense text rows). Split out of the parent file to keep it under
//! the size ratchet.

use super::pairs::{other_actor, row, Found};
use super::*;
use engine_spatial::measure::{measure_pair, MeasureArgs, What};
use engine_spatial::rooms::{find_rooms, Rooms, CAPSULE_CM};
use engine_spatial::sense_text::{
    camera_yaw_deg, coverage, glance, glance_text, lidar, map, pixel, view, Camera, SenseError,
    DEFAULT_GLANCE_GRID, HIDDEN_PERMILLE,
};

/// Extra fields of these questions, all optional on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpaceArgs {
    /// `gap`, `overlap`, `align` or `contain` (measure).
    pub what: Option<String>,
    /// Room number as `room_of` names it (in_room, exits_of).
    pub room: Option<u32>,
    /// Camera pitch in whole degrees, positive looks up (coverage), default 0.
    pub pitch_deg: Option<i32>,
}

/// The measure question: `what`, with `radius_cm` as the gap or margin it must
/// meet; contact tolerance 1 cm, all axes, centers for alignment.
fn measure_args(args: &QueryArgs) -> Result<(MeasureArgs, &str), QueryError> {
    let word = args.space.what.as_deref().unwrap_or("gap");
    let what = match word {
        "gap" => What::Gap,
        "overlap" => What::Overlap,
        "align" => What::Align,
        "contain" => What::Contain,
        _ => {
            return Err(QueryError::new(
                "bad_what",
                format!("what `{word}` is unknown"),
                "legal: gap, overlap, align, contain",
            ))
        }
    };
    let margs = MeasureArgs {
        what,
        min_clearance_cm: args.radius_cm.unwrap_or(0),
        ..MeasureArgs::default()
    };
    Ok((margs, word))
}

fn measure_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let a = check_anchor(actors, args.anchor)?;
    let b = other_actor(actors, args)?;
    let (margs, word) = measure_args(args)?;
    let m = measure_pair(actors, a.id, b.id, &margs).ok_or_else(|| {
        QueryError::new(
            "unknown_actor",
            "degenerate objects have no box to measure",
            "re-read scene.brief",
        )
    })?;
    let verdict = if m.passed {
        "passed".to_owned()
    } else {
        format!("short by {} cm", m.shortfall_cm)
    };
    let reason = format!(
        "measure {word}: gap {} cm, axis_gap {:?}, {}, penetration {} pm, {verdict}",
        m.gap_cm,
        m.axis_gap,
        m.relation.word(),
        m.penetration_pm
    );
    Ok((vec![a.id], vec![row(b, m.passed, reason)], Vec::new()))
}

/// Room number holding the plan point of `a`, when it stands in one.
fn room_of_actor(rooms: &Rooms, a: &Actor) -> Option<u32> {
    rooms.room_of(to_cm(a.x), to_cm(a.z))
}

fn room_name(id: u32) -> String {
    if id == 0 {
        "outside".to_owned()
    } else {
        format!("room_{id}")
    }
}

/// The room a question names: `room` as given, else the room around `anchor`.
fn target_room(actors: &[Actor], rooms: &Rooms, args: &QueryArgs) -> Result<u32, QueryError> {
    if let Some(n) = args.space.room {
        if rooms.rooms.iter().any(|r| r.id == n) {
            return Ok(n);
        }
        return Err(QueryError::new(
            "bad_room",
            format!(
                "room {n} does not exist; the scene has {} room(s)",
                rooms.rooms.len()
            ),
            "ask room_of for an object first, or pass a room from 1",
        ));
    }
    let a = check_anchor(actors, args.anchor)?;
    room_of_actor(rooms, a).ok_or_else(|| {
        QueryError::new(
            "no_room",
            format!("{} stands in no room (outside, or on a wall)", a.name),
            "pass `room` instead, or move the object inside walls",
        )
    })
}

/// Whole cm as meters; the world bound keeps it inside i16.
fn meters(cm: i32) -> f32 {
    i16::try_from(cm).map_or(0.0, f32::from) / 100.0
}

fn exit_hits(rooms: &Rooms, id: u32) -> Vec<Hit> {
    rooms
        .exits_of(id)
        .iter()
        .zip(0u32..)
        .map(|(e, k)| {
            let other = if e.a == id { e.b } else { e.a };
            Hit {
                id: SAMPLE_ACTOR_BASE + k,
                x: meters(e.center_cm[0]),
                y: 0.0,
                z: meters(e.center_cm[1]),
                matched: true,
                reason: format!(
                    "exit: {} to {}, {} cm wide",
                    room_name(id),
                    room_name(other),
                    e.width_cm
                ),
            }
        })
        .collect()
}

fn room_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let rooms = find_rooms(actors, CAPSULE_CM);
    match args.kind {
        QueryKind::RoomOf => {
            let a = check_anchor(actors, args.anchor)?;
            let room =
                room_of_actor(&rooms, a).and_then(|id| rooms.rooms.iter().find(|r| r.id == id));
            let reason = match room {
                Some(room) => format!(
                    "room_of: {} area {} m2, center ({}, {}) cm, {} exit(s), {} member(s)",
                    room_name(room.id),
                    room.area_cm2 / 10_000,
                    room.center_cm[0],
                    room.center_cm[1],
                    rooms.exits_of(room.id).len(),
                    room.members.len()
                ),
                None => "room_of: outside every room".to_owned(),
            };
            Ok((vec![a.id], vec![row(a, room.is_some(), reason)], Vec::new()))
        }
        QueryKind::InRoom => {
            let id = target_room(actors, &rooms, args)?;
            let members = rooms.rooms.iter().find(|r| r.id == id).map(|r| &r.members);
            let hits = members
                .into_iter()
                .flatten()
                .filter_map(|m| actor_by_id(actors, *m))
                .map(|a| row(a, true, format!("in_room: {}", room_name(id))))
                .collect();
            Ok((Vec::new(), hits, Vec::new()))
        }
        _ => {
            let id = target_room(actors, &rooms, args)?;
            Ok((Vec::new(), exit_hits(&rooms, id), Vec::new()))
        }
    }
}

fn glance_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let eye = check_eye(args.eye_m)?;
    let cam = sense_camera(args)?;
    let fov = args
        .half_angle_deg
        .map_or(90, |half| half.saturating_mul(2));
    let grid = DEFAULT_GLANCE_GRID;
    let g = glance(&SightWorld::build(actors), &cam, fov, grid)
        .map_err(|e| QueryError::new("bad_view", e.to_string(), "grid is 1..=64, fov 2..=170"))?;
    let mut hits = Vec::new();
    for s in &g.visible {
        if let Some(a) = actor_by_id(actors, s.id) {
            let reason = format!(
                "glance: {} pm, near {} cm, far {} cm",
                s.coverage_pm, s.near_cm, s.far_cm
            );
            hits.push(row(a, true, reason));
        }
    }
    for id in &g.occluded {
        if let Some(a) = actor_by_id(actors, *id) {
            hits.push(row(a, false, "glance: occluded".to_owned()));
        }
    }
    let depth = g.depth_cm.map_or_else(
        || "none".to_owned(),
        |[near, far, mean]| format!("{near}..{far} cm mean {mean}"),
    );
    let summary = format!(
        "glance {grid}x{grid} fov {fov}: background {} pm, more {} pm, depth {depth}, \
         ~{} tokens as sense text",
        g.background_pm,
        g.more_pm,
        engine_spatial::estimate_tokens(&glance_text(&g, &cam, fov))
    );
    hits.push(Hit {
        id: SAMPLE_ACTOR_BASE,
        x: eye[0],
        y: eye[1],
        z: eye[2],
        matched: true,
        reason: summary,
    });
    Ok((Vec::new(), hits, Vec::new()))
}

fn coverage_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let subject = check_anchor(actors, args.anchor)?;
    let cam = sense_camera(args)?;
    let seen = coverage(
        &SightWorld::build(actors),
        &cam,
        args.space.pitch_deg.unwrap_or(0),
        args.half_angle_deg
            .map_or(60, |half| half.saturating_mul(2)),
        subject.id,
    )
    .map_err(|e| {
        QueryError::new(
            "bad_view",
            e.to_string(),
            "pitch is -180..=180 degrees, fov 2..=170",
        )
    })?;
    let verdict = match (seen.looks_away, seen.flipped) {
        (true, true) => "looks_away flipped",
        (true, false) => "looks_away",
        (false, true) => "flipped",
        (false, false) => "ok",
    };
    let mut reason = format!("coverage: {} pm, {verdict}", seen.permille);
    if !seen.next.is_empty() {
        reason = format!("{reason}; {}", seen.next);
    }
    let shown = seen.permille >= HIDDEN_PERMILLE;
    Ok((
        vec![subject.id],
        vec![row(subject, shown, reason)],
        Vec::new(),
    ))
}

/// The camera one door question names: `eye_m` in meters with `yaw` radians
/// (0 faces +x), refused when the yaw is not finite.
fn sense_camera(args: &QueryArgs) -> Result<Camera, QueryError> {
    let eye = check_eye(args.eye_m)?;
    let yaw = args.yaw.unwrap_or(0.0);
    if !yaw.is_finite() {
        return Err(QueryError::new(
            "bad_point",
            "yaw is not finite".to_owned(),
            "pass radians",
        ));
    }
    Ok(Camera {
        eye_cm: eye.map(to_cm),
        yaw_deg: camera_yaw_deg(yaw),
    })
}

/// View or pixel box `[w, h, x, y]` in cells, with the per-kind default.
fn sense_cells(args: &QueryArgs, kind: QueryKind) -> [i32; 4] {
    args.cells.unwrap_or(match kind {
        QueryKind::Pixel => [24, 16, 12, 8],
        _ => [24, 16, 0, 0],
    })
}

/// Door refusal for a sense verb: the typed code plus one recovery.
fn sense_refusal(error: SenseError) -> QueryError {
    match error {
        SenseError::BadSize => QueryError::new(
            "bad_view",
            error.to_string(),
            "view and pixel edges are 1..=128 cells",
        ),
        SenseError::OutOfImage => QueryError::new(
            "bad_pixel",
            error.to_string(),
            "x runs 0..w and y runs 0..h inside the view",
        ),
        SenseError::BadRadius => QueryError::new(
            "bad_radius",
            error.to_string(),
            "map radius is 50..=1500 cm",
        ),
        _ => QueryError::new(
            "bad_view",
            error.to_string(),
            "glance and coverage refuse their own shapes",
        ),
    }
}

/// The SP69 text verbs at the door: `view`, `lidar`, `map` and `pixel` answer
/// as rows of the same plain text `engine-spatial` prints, one row per line,
/// plus the token cost of the whole text. Text only: no render, no vision
/// model and no state. Every answer replays byte for byte.
fn sense_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let cam = sense_camera(args)?;
    let world = SightWorld::build(actors);
    let text = match args.kind {
        QueryKind::View => {
            let [w, h, _, _] = sense_cells(args, args.kind);
            view(&world, &cam, w, h)
        }
        QueryKind::Lidar => Ok(lidar(&world, cam.eye_cm, cam.yaw_deg)),
        QueryKind::Map => map(&world, cam.eye_cm, args.radius_cm.unwrap_or(600)),
        _ => {
            let [w, h, x, y] = sense_cells(args, args.kind);
            pixel(&world, &cam, w, h, x, y)
        }
    }
    .map_err(sense_refusal)?;
    let eye_m = args.eye_m.unwrap_or([0.0; 3]);
    let mut hits: Vec<Hit> = text
        .lines()
        .enumerate()
        .map(|(line, text)| Hit {
            id: SAMPLE_ACTOR_BASE.saturating_add(u32::try_from(line).unwrap_or(u32::MAX)),
            x: eye_m[0],
            y: eye_m[1],
            z: eye_m[2],
            matched: true,
            reason: text.to_owned(),
        })
        .collect();
    let tokens = engine_spatial::estimate_tokens(&text);
    hits.push(Hit {
        id: SAMPLE_ACTOR_BASE.saturating_add(u32::try_from(hits.len()).unwrap_or(u32::MAX)),
        x: eye_m[0],
        y: eye_m[1],
        z: eye_m[2],
        matched: true,
        reason: format!("tokens {tokens}"),
    });
    Ok((Vec::new(), hits, Vec::new()))
}

/// Answer one of the measure, room, glance, coverage or sense text kinds over
/// the staged scene.
pub(super) fn run_space_kind(
    actors: &[Actor],
    args: &QueryArgs,
    budget: usize,
) -> Result<QueryAnswer, QueryError> {
    let (anchors, hits, conflicts) = match args.kind {
        QueryKind::Measure => measure_rows(actors, args)?,
        QueryKind::Glance => glance_rows(actors, args)?,
        QueryKind::Coverage => coverage_rows(actors, args)?,
        QueryKind::View | QueryKind::Lidar | QueryKind::Map | QueryKind::Pixel => {
            sense_rows(actors, args)?
        }
        _ => room_rows(actors, args)?,
    };
    Ok(render(args.kind, &anchors, hits, conflicts, budget))
}
