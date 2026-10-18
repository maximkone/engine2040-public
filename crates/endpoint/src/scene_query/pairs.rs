//! Pair and measure questions for `scene.query`: sees, why_hidden, clearance,
//! slide and ground. Each answers from one `engine-spatial` call and returns
//! the figures an agent acts on (percent seen, blocker id, gap and push in
//! cm). Split out of the parent file to keep it under the size ratchet.

use super::*;
use engine_spatial::measure::clearance;
use engine_spatial::sees::{sees_pair, FAN_RAYS};
use engine_spatial::slide::{ground_below, slide, Dir};
use engine_spatial::why_hidden::{why_hidden, Draw, Hidden};

/// Anchor ids, hits and conflicts of one pair answer.
pub(super) type Found = (Vec<u32>, Vec<Hit>, Vec<Conflict>);

pub(super) fn row(a: &Actor, matched: bool, reason: String) -> Hit {
    Hit {
        id: a.id,
        x: a.x,
        y: a.y,
        z: a.z,
        matched,
        reason,
    }
}

pub(super) fn other_actor<'a>(
    actors: &'a [Actor],
    args: &QueryArgs,
) -> Result<&'a Actor, QueryError> {
    args.other
        .and_then(|id| actor_by_id(actors, id))
        .ok_or_else(|| {
            QueryError::new(
                "unknown_actor",
                "this question needs an `other` object",
                valid_ids_hint(actors),
            )
        })
}

fn sees_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let a = check_anchor(actors, args.anchor)?;
    let b = other_actor(actors, args)?;
    let seen = sees_pair(&SightWorld::build(actors), a.id, b.id).map_err(|_| {
        QueryError::new(
            "unknown_actor",
            "degenerate objects index no sight box",
            "re-read scene.brief",
        )
    })?;
    let word = if seen.visible { "visible" } else { "hidden" };
    let mut hits = vec![row(
        b,
        seen.visible,
        format!(
            "sees: {word} {}% ({}/{FAN_RAYS} rays)",
            seen.fraction_pct(),
            seen.clear_rays
        ),
    )];
    let blocker = seen
        .blocker
        .and_then(|bl| actor_by_id(actors, bl.id).map(|actor| (actor, bl.dist_cm)));
    if let Some((actor, dist_cm)) = blocker {
        hits.push(row(
            actor,
            false,
            format!("blocks the view at {dist_cm} cm"),
        ));
    }
    Ok((vec![a.id, b.id], hits, Vec::new()))
}

fn why_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let target = check_anchor(actors, args.anchor)?;
    let eye = check_eye(args.eye_m)?;
    let yaw = args.yaw.unwrap_or(0.0);
    let why = why_hidden(actors, Draw::default(), target.id, eye, yaw).map_err(|_| {
        QueryError::new(
            "bad_point",
            "eye or yaw is not finite",
            "pass finite meters",
        )
    })?;
    let reason = format!(
        "why_hidden: {} (evidence {})",
        why.verdict.word(),
        why.evidence
    );
    let hit = row(target, why.verdict == Hidden::Visible, reason);
    Ok((vec![target.id], vec![hit], Vec::new()))
}

fn clearance_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let a = check_anchor(actors, args.anchor)?;
    let need = check_radius(args.radius_cm.unwrap_or(60))?;
    let found = clearance(actors, a.id, need)
        .and_then(|c| actor_by_id(actors, c.neighbour).map(|n| (n, c)));
    let Some((neighbour, c)) = found else {
        let none = Conflict {
            code: "no_neighbour",
            message: "nothing else physical stands near the anchor".to_owned(),
            hint: "add objects or ask another kind".to_owned(),
        };
        return Ok((vec![a.id], Vec::new(), vec![none]));
    };
    let reason = format!(
        "clearance: {} gap {} cm, need {need}; push_pm {:?}",
        c.kind.word(),
        c.gap.gap_cm,
        c.gap.push_pm
    );
    Ok((
        vec![a.id],
        vec![row(neighbour, c.holds, reason)],
        Vec::new(),
    ))
}

fn slide_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let a = check_anchor(actors, args.anchor)?;
    let word = args.dir.as_deref().unwrap_or_default();
    let dir = Dir::parse(word).ok_or_else(|| {
        QueryError::new(
            "bad_dir",
            format!("dir `{word}` is unknown"),
            format!("legal dirs: {}", Dir::words()),
        )
    })?;
    let range = check_radius(args.radius_cm.unwrap_or(500))?;
    let slid = slide(actors, a.id, dir, range).ok_or_else(|| {
        QueryError::new(
            "unknown_actor",
            "anchor has no finite box",
            "re-read scene.brief",
        )
    })?;
    let end = match (slid.already_blocked, slid.capped) {
        (true, _) => "already blocked",
        (_, true) => "clear to range",
        _ => "stops at a blocker",
    };
    let mut hits = vec![row(
        a,
        slid.free_cm > 0,
        format!("slide {word}: free {} cm of {range}; {end}", slid.free_cm),
    )];
    if let Some(blocker) = slid.blocker.and_then(|id| actor_by_id(actors, id)) {
        hits.push(row(blocker, false, format!("blocks the slide {word}")));
    }
    Ok((vec![a.id], hits, Vec::new()))
}

fn ground_rows(actors: &[Actor], args: &QueryArgs) -> Result<Found, QueryError> {
    let at = check_eye(args.eye_m)?;
    let found = ground_below(actors, to_cm(at[0]), to_cm(at[2]), to_cm(at[1]))
        .and_then(|(top, id)| actor_by_id(actors, id).map(|a| (top, a)));
    Ok(match found {
        Some((top, a)) => (
            Vec::new(),
            vec![row(a, true, format!("ground: top {top} cm"))],
            Vec::new(),
        ),
        None => {
            let none = Conflict {
                code: "no_ground",
                message: "no solid lies under that point".to_owned(),
                hint: "lower the y or pick a point over an object".to_owned(),
            };
            (Vec::new(), Vec::new(), vec![none])
        }
    })
}

/// Answer one of the five pair kinds over the staged scene.
pub(super) fn run_pair_kind(
    actors: &[Actor],
    args: &QueryArgs,
    budget: usize,
) -> Result<QueryAnswer, QueryError> {
    let (anchors, hits, conflicts) = match args.kind {
        QueryKind::Sees => sees_rows(actors, args)?,
        QueryKind::WhyHidden => why_rows(actors, args)?,
        QueryKind::Clearance => clearance_rows(actors, args)?,
        QueryKind::Slide => slide_rows(actors, args)?,
        _ => ground_rows(actors, args)?,
    };
    Ok(render(args.kind, &anchors, hits, conflicts, budget))
}
