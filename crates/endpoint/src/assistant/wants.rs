use super::*;

/// Document snapshot as spatial actors plus the index-to-UUID table. Every
/// node with a finite world position becomes one unit cube on its world
/// position (the CAP1/SP1 convention); node order is the id-text sort, so
/// the same scene always maps to the same actor ids.
pub(super) struct ActorSnapshot {
    pub(super) actors: Vec<engine_spatial::Actor>,
    pub(super) ids: Vec<ObjectId>,
}

pub(super) fn snapshot_actors(doc: &Document) -> ActorSnapshot {
    let mut order: Vec<ObjectId> = Vec::new();
    let mut stack = doc.hierarchy.roots();
    while let Some(id) = stack.pop() {
        order.push(id);
        stack.extend(doc.hierarchy.children(id));
    }
    order.sort_by_key(|id| id.to_string());
    let mut actors = Vec::new();
    let mut ids = Vec::new();
    for id in order {
        let Some(world) = doc.hierarchy.world(id) else {
            continue;
        };
        if !world.position.iter().all(|lane| lane.is_finite()) {
            continue;
        }
        let yaw = if world.rotation.iter().all(|lane| lane.is_finite()) {
            yaw_of_quat(world.rotation)
        } else {
            0.0
        };
        let Ok(index) = u32::try_from(actors.len()) else {
            break;
        };
        ids.push(id);
        actors.push(engine_spatial::Actor {
            id: index,
            name: format!("prop_{index:04}"),
            x: world.position[0],
            y: world.position[1],
            z: world.position[2],
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw,
        });
    }
    ActorSnapshot { actors, ids }
}

pub(super) fn bad_want(source: &str, detail: &str) -> ProposeError {
    ProposeError::PreviewFailed {
        code: "bad_want",
        message: format!(
            "want `{source}` is malformed ({detail}); \
             want grammar is `<relation> <target> [<lo>,<hi>]?` with \
             target `wall:<north|south|east|west>`, `object:<uuid>` or \
             `point:<x_cm>,<z_cm>`"
        ),
    }
}

pub(super) fn parse_relation(text: &str) -> Result<engine_spatial::Relation, ProposeError> {
    use engine_spatial::Relation;
    match text {
        "supports" => Ok(Relation::Supports),
        "on_top_of" => Ok(Relation::OnTopOf),
        "inside" => Ok(Relation::Inside),
        "next_to" => Ok(Relation::NextTo),
        "facing" => Ok(Relation::Facing),
        other => Err(ProposeError::PreviewFailed {
            code: "bad_relation",
            message: format!(
                "relation `{other}` is unknown; \
                 submit supports, on_top_of, inside, next_to or facing"
            ),
        }),
    }
}

pub(super) fn parse_wall(text: &str) -> Result<engine_spatial::Wall, ProposeError> {
    use engine_spatial::Wall;
    match text {
        "north" => Ok(Wall::North),
        "south" => Ok(Wall::South),
        "east" => Ok(Wall::East),
        "west" => Ok(Wall::West),
        other => Err(ProposeError::PreviewFailed {
            code: "bad_wall",
            message: format!("wall `{other}` is unknown; submit north, south, east or west"),
        }),
    }
}

/// A parsed want, or a want that names an unknown target id (a conflict,
/// not an error: impossible intents answer with the conflict shape).
pub(super) enum WantOutcome {
    Ready(engine_spatial::Want),
    MissingTarget { reason: String },
}

pub(super) fn parse_int(value: &str, what: &str, source: &str) -> Result<i32, ProposeError> {
    value.parse::<i32>().map_err(|_| {
        bad_want(
            source,
            &format!("bad {what} `{value}` (expected an integer)"),
        )
    })
}

/// Want grammar, one line per want: `<relation> <target> [<lo>,<hi>]?`.
/// Distances are inclusive integer centimeters.
pub(super) fn parse_want(
    source: &str,
    index_of: &BTreeMap<String, u32>,
) -> Result<WantOutcome, ProposeError> {
    let mut parts = source.split_whitespace();
    let relation_text = parts.next().ok_or_else(|| bad_want(source, "empty"))?;
    let target_text = parts
        .next()
        .ok_or_else(|| bad_want(source, "missing target"))?;
    let range_text = parts.next();
    if parts.next().is_some() {
        return Err(bad_want(source, "too many parts"));
    }
    let relation = parse_relation(relation_text).map_err(|error| match error {
        ProposeError::PreviewFailed { message, .. } => ProposeError::PreviewFailed {
            code: "bad_relation",
            message: format!("want `{source}`: {message}"),
        },
        other => other,
    })?;
    let target = match target_text.split_once(':') {
        Some(("wall", name)) => {
            let wall = parse_wall(name).map_err(|error| match error {
                ProposeError::PreviewFailed { message, .. } => ProposeError::PreviewFailed {
                    code: "bad_wall",
                    message: format!("want `{source}`: {message}"),
                },
                other => other,
            })?;
            engine_spatial::Target::Wall(wall)
        }
        Some(("object", id)) => {
            let target_id = ObjectId::parse(id).map_err(|_| ProposeError::PreviewFailed {
                code: "bad_id",
                message: format!(
                    "want `{source}`: target `{id}` is not an object id (expected UUID text)"
                ),
            })?;
            match index_of.get(&target_id.to_string()).copied() {
                Some(index) => engine_spatial::Target::Object(index),
                None => {
                    return Ok(WantOutcome::MissingTarget {
                        reason: format!(
                            "unknown-target {id}: no staged node with that id; \
                             re-read scene.brief for the staged ids"
                        ),
                    });
                }
            }
        }
        Some(("point", coords)) => {
            let (x_text, z_text) = coords
                .split_once(',')
                .ok_or_else(|| bad_want(source, "point needs `<x_cm>,<z_cm>`"))?;
            engine_spatial::Target::Point {
                x_cm: parse_int(x_text, "x_cm", source)?,
                z_cm: parse_int(z_text, "z_cm", source)?,
            }
        }
        _ => {
            return Err(bad_want(
                source,
                "target needs a `wall:|object:|point:` kind",
            ));
        }
    };
    let distance_cm = match range_text {
        None => None,
        Some(range) => {
            let inner = range
                .strip_prefix('[')
                .and_then(|text| text.strip_suffix(']'))
                .ok_or_else(|| bad_want(source, "range needs `[<lo>,<hi>]`"))?;
            let (lo_text, hi_text) = inner
                .split_once(',')
                .ok_or_else(|| bad_want(source, "range needs `[<lo>,<hi>]`"))?;
            Some((
                parse_int(lo_text, "lo", source)?,
                parse_int(hi_text, "hi", source)?,
            ))
        }
    };
    Ok(WantOutcome::Ready(engine_spatial::Want {
        relation,
        target,
        distance_cm,
    }))
}

/// Deterministic check-row text for a receipt.
pub(super) fn receipt_text(receipt: &PreviewReceipt) -> String {
    format!(
        "check ok rev{} {}",
        receipt.resulting_revision,
        receipt.effects.join(" | ")
    )
}

/// Deterministic check-row text for a conflict.
pub(super) fn conflict_text(conflict: &ProposalConflict) -> String {
    format!(
        "check conflict {} | {}",
        conflict.reason,
        conflict.conflicts.join(" + ")
    )
}

pub(super) fn batch_error(error: &BatchError) -> ProposeError {
    let code = match error {
        BatchError::StaleRevision { .. } => "stale_revision",
        BatchError::DuplicateKey { .. } => "duplicate_key",
        BatchError::UndoneKey { .. } => "undone_key",
        _ => "invalid_batch",
    };
    ProposeError::PreviewFailed {
        code,
        message: error.to_string(),
    }
}
