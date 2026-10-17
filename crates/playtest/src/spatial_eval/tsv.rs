use super::*;

// ---- TSV codec (versioned, strict, integer-only) -------------------------------

/// TSV column header: id, kind, scene, tool, params, gold, vision.
pub(super) const TSV_HEADER: &str = "id\tkind\tscene\ttool\tparams\tgold\tvision";
/// Comment marker for the version line.
pub(super) const TSV_VERSION_LINE: &str = "# spatial-gold v1";

pub(super) fn encode_ids(ids: &[u32]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn encode_params(params: &Params) -> String {
    match params {
        Params::Place {
            x_cm,
            z_cm,
            hx_cm,
            hz_cm,
        } => format!("x={x_cm};z={z_cm};hx={hx_cm};hz={hz_cm}"),
        Params::Near { anchor, radius_cm } => format!("anchor={anchor};radius_cm={radius_cm}"),
        Params::Between { a, b } => format!("a={a};b={b}"),
        Params::Visible {
            ex_cm,
            ez_cm,
            yaw_oct,
        } => format!("ex={ex_cm};ez={ez_cm};oct={yaw_oct}"),
        Params::Reachable { anchor } => format!("anchor={anchor}"),
        Params::CoverNear {
            anchor,
            tx_cm,
            tz_cm,
            crouch,
            radius_cm,
        } => {
            format!(
                "anchor={anchor};tx={tx_cm};tz={tz_cm};crouch={};radius_cm={radius_cm}",
                u8::from(*crouch)
            )
        }
        Params::LargestOpen { capsule_cm } => format!("capsule_cm={capsule_cm}"),
        Params::FreeSpace {
            capsule_cm,
            gap_cm,
            max_n,
            seed,
        } => {
            format!("capsule_cm={capsule_cm};gap_cm={gap_cm};max_n={max_n};seed={seed}")
        }
    }
}

pub(super) fn encode_gold(gold: &Gold) -> String {
    match gold {
        Gold::Verdict { clean, conflicts } => format!(
            "clean={};conflicts={}",
            u8::from(*clean),
            encode_ids(conflicts)
        ),
        Gold::Ids(ids) => format!("ids={}", encode_ids(ids)),
        Gold::Repair {
            dx_cm,
            dz_cm,
            x_cm,
            z_cm,
        } => format!("repair={dx_cm},{dz_cm};at={x_cm},{z_cm}"),
        Gold::Samples {
            expect_empty,
            min_clear_cm,
        } => {
            format!("empty={};min_clear={min_clear_cm}", u8::from(*expect_empty))
        }
    }
}

/// Render the frozen hundred as the checked-in TSV.
pub fn encode_tsv(tasks: &[Task]) -> String {
    let mut out = String::from(TSV_VERSION_LINE);
    out.push('\n');
    out.push_str(TSV_HEADER);
    out.push('\n');
    for task in tasks {
        out.push_str(&task.id);
        out.push('\t');
        out.push_str(task.kind.as_str());
        out.push('\t');
        out.push_str(&task.scene);
        out.push('\t');
        out.push_str(&task.tool);
        out.push('\t');
        out.push_str(&encode_params(&task.params));
        out.push('\t');
        out.push_str(&encode_gold(&task.gold));
        out.push('\t');
        out.push_str(&task.vision.note);
        out.push(';');
        out.push_str(&task.vision.capture);
        out.push('\n');
    }
    out
}

pub(super) fn parse_i32(text: &str, key: &str) -> Result<i32, ParseError> {
    text.parse::<i32>().map_err(|_| ParseError::WrongType {
        field: format!("{key}={text}"),
        want: "an i32 cm value",
    })
}

pub(super) fn parse_u32(text: &str, key: &str) -> Result<u32, ParseError> {
    text.parse::<u32>().map_err(|_| ParseError::WrongType {
        field: format!("{key}={text}"),
        want: "a u32 id",
    })
}

pub(super) fn check_coord(value: i32, key: &str) -> Result<i32, ParseError> {
    if (-MAX_COORD_CM..=MAX_COORD_CM).contains(&value) {
        Ok(value)
    } else {
        Err(ParseError::OutOfRange {
            what: format!("`{key}={value}` outside -{MAX_COORD_CM}..={MAX_COORD_CM} cm"),
        })
    }
}

pub(super) fn parse_id_list(text: &str) -> Result<Vec<u32>, ParseError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for part in text.split(',') {
        let id = parse_u32(part, "id")?;
        if id >= 1_000_000 {
            return Err(ParseError::OutOfRange {
                what: format!("id `{id}` past the 1000000 actor bound"),
            });
        }
        if ids.contains(&id) {
            return Err(ParseError::Mismatch {
                what: format!("duplicate id `{id}`; gold lists stay sorted and unique"),
            });
        }
        ids.push(id);
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    if ids != sorted {
        return Err(ParseError::Mismatch {
            what: "id list is not ascending; gold lists stay sorted".to_owned(),
        });
    }
    Ok(ids)
}

/// Parse one `x/z/hx/hz` footprint: the gold set places 80 cm crates.
pub(super) fn parse_footprint(pairs: &[(String, String)]) -> Result<Params, ParseError> {
    kv_expect_keys(pairs, &["x", "z", "hx", "hz"])?;
    let x = check_coord(parse_i32(kv_get(pairs, "x")?, "x")?, "x")?;
    let z = check_coord(parse_i32(kv_get(pairs, "z")?, "z")?, "z")?;
    let hx = parse_i32(kv_get(pairs, "hx")?, "hx")?;
    let hz = parse_i32(kv_get(pairs, "hz")?, "hz")?;
    if hx != 40 || hz != 40 {
        return Err(ParseError::Mismatch {
            what: format!("footprint hx={hx} hz={hz}; the gold set places 80 cm crates"),
        });
    }
    Ok(Params::Place {
        x_cm: x,
        z_cm: z,
        hx_cm: hx,
        hz_cm: hz,
    })
}

/// Parse one `radius_cm` against the world bound both near shapes share.
pub(super) fn check_radius(raw: &str) -> Result<i32, ParseError> {
    let radius = parse_i32(raw, "radius_cm")?;
    if !(1..=MAX_RADIUS_CM).contains(&radius) {
        return Err(ParseError::OutOfRange {
            what: format!("radius_cm={radius} outside 1..={MAX_RADIUS_CM}"),
        });
    }
    Ok(radius)
}

/// Parse the anchor-led query shapes: reachable, near, or cover-near when a
/// threat rides along.
pub(super) fn parse_anchor_params(
    pairs: &[(String, String)],
    anchor_id: u32,
) -> Result<Params, ParseError> {
    if let Ok(radius_raw) = kv_get(pairs, "radius_cm") {
        // Near, or cover-near when a threat rides along.
        if kv_get(pairs, "tx").is_ok() {
            kv_expect_keys(pairs, &["anchor", "tx", "tz", "crouch", "radius_cm"])?;
            let tx = check_coord(parse_i32(kv_get(pairs, "tx")?, "tx")?, "tx")?;
            let tz = check_coord(parse_i32(kv_get(pairs, "tz")?, "tz")?, "tz")?;
            let crouch = match kv_get(pairs, "crouch")? {
                "0" => false,
                "1" => true,
                other => {
                    return Err(ParseError::Unknown {
                        what: format!("crouch={other}; want 0 or 1"),
                    });
                }
            };
            return Ok(Params::CoverNear {
                anchor: anchor_id,
                tx_cm: tx,
                tz_cm: tz,
                crouch,
                radius_cm: check_radius(radius_raw)?,
            });
        }
        kv_expect_keys(pairs, &["anchor", "radius_cm"])?;
        return Ok(Params::Near {
            anchor: anchor_id,
            radius_cm: check_radius(radius_raw)?,
        });
    }
    kv_expect_keys(pairs, &["anchor"])?;
    Ok(Params::Reachable { anchor: anchor_id })
}

/// Parse one `ex/ez/oct` visibility question.
pub(super) fn parse_visible_params(pairs: &[(String, String)]) -> Result<Params, ParseError> {
    kv_expect_keys(pairs, &["ex", "ez", "oct"])?;
    let ex = check_coord(parse_i32(kv_get(pairs, "ex")?, "ex")?, "ex")?;
    let ez = check_coord(parse_i32(kv_get(pairs, "ez")?, "ez")?, "ez")?;
    let oct_raw = kv_get(pairs, "oct")?;
    let oct = oct_raw.parse::<u8>().map_err(|_| ParseError::OutOfRange {
        what: format!("oct={oct_raw} past 0..=7"),
    })?;
    if oct > 7 {
        return Err(ParseError::OutOfRange {
            what: format!("oct={oct} past 0..=7"),
        });
    }
    Ok(Params::Visible {
        ex_cm: ex,
        ez_cm: ez,
        yaw_oct: oct,
    })
}

/// Parse the sampler questions: one capsule, or capsule plus gap, count and
/// seed for free-space sampling.
pub(super) fn parse_space_params(pairs: &[(String, String)]) -> Result<Params, ParseError> {
    if kv_get(pairs, "gap_cm").is_ok() {
        kv_expect_keys(pairs, &["capsule_cm", "gap_cm", "max_n", "seed"])?;
        let capsule = parse_i32(kv_get(pairs, "capsule_cm")?, "capsule_cm")?;
        let gap = parse_i32(kv_get(pairs, "gap_cm")?, "gap_cm")?;
        let max_n =
            kv_get(pairs, "max_n")?
                .parse::<usize>()
                .map_err(|_| ParseError::WrongType {
                    field: "max_n".to_owned(),
                    want: "a count 1..=64",
                })?;
        let seed = kv_get(pairs, "seed")?
            .parse::<u64>()
            .map_err(|_| ParseError::WrongType {
                field: "seed".to_owned(),
                want: "a u64",
            })?;
        if !(0..=MAX_RADIUS_CM).contains(&capsule) {
            return Err(ParseError::OutOfRange {
                what: format!("capsule_cm={capsule} outside 0..={MAX_RADIUS_CM}"),
            });
        }
        if !(1..=MAX_RADIUS_CM).contains(&gap) {
            return Err(ParseError::OutOfRange {
                what: format!("gap_cm={gap} outside 1..={MAX_RADIUS_CM}"),
            });
        }
        if !(1..=MAX_IDS).contains(&max_n) {
            return Err(ParseError::OutOfRange {
                what: format!("max_n={max_n} outside 1..={MAX_IDS}"),
            });
        }
        return Ok(Params::FreeSpace {
            capsule_cm: capsule,
            gap_cm: gap,
            max_n,
            seed,
        });
    }
    kv_expect_keys(pairs, &["capsule_cm"])?;
    let capsule = parse_i32(kv_get(pairs, "capsule_cm")?, "capsule_cm")?;
    if !(0..=MAX_RADIUS_CM).contains(&capsule) {
        return Err(ParseError::OutOfRange {
            what: format!("capsule_cm={capsule} outside 0..={MAX_RADIUS_CM}"),
        });
    }
    Ok(Params::LargestOpen {
        capsule_cm: capsule,
    })
}

/// Parse one query params map by its leading key.
pub(super) fn parse_query_params(pairs: &[(String, String)]) -> Result<Params, ParseError> {
    if let Ok(anchor_raw) = kv_get(pairs, "anchor") {
        let anchor_id = parse_u32(anchor_raw, "anchor")?;
        return parse_anchor_params(pairs, anchor_id);
    }
    if kv_get(pairs, "a").is_ok() {
        kv_expect_keys(pairs, &["a", "b"])?;
        return Ok(Params::Between {
            a: parse_u32(kv_get(pairs, "a")?, "a")?,
            b: parse_u32(kv_get(pairs, "b")?, "b")?,
        });
    }
    if kv_get(pairs, "ex").is_ok() {
        return parse_visible_params(pairs);
    }
    if kv_get(pairs, "capsule_cm").is_ok() {
        return parse_space_params(pairs);
    }
    Err(ParseError::Mismatch {
        what: "params carry no known key; want anchor/a/ex/capsule_cm".to_owned(),
    })
}

pub(super) fn parse_params(kind: TaskKind, text: &str) -> Result<Params, ParseError> {
    let pairs = split_kv(text)?;
    match kind {
        TaskKind::Place | TaskKind::Fix => parse_footprint(&pairs),
        TaskKind::Query => parse_query_params(&pairs),
    }
}

/// Parse one `dx,dz`-shaped fix gold pair.
pub(super) fn parse_fix_pair(
    pairs: &[(String, String)],
    key: &str,
) -> Result<(i32, i32), ParseError> {
    let raw = kv_get(pairs, key)?;
    let (first, second) = raw.split_once(',').ok_or_else(|| ParseError::Mismatch {
        what: format!("`{key}={raw}` wants two comma ints"),
    })?;
    Ok((parse_i32(first, key)?, parse_i32(second, key)?))
}

pub(super) fn parse_gold(kind: TaskKind, text: &str) -> Result<Gold, ParseError> {
    let pairs = split_kv(text)?;
    match kind {
        TaskKind::Place => {
            if pairs.len() != 2
                || kv_get(&pairs, "clean").is_err()
                || kv_get(&pairs, "conflicts").is_err()
            {
                return Err(ParseError::Mismatch {
                    what: "place gold wants clean=0/1;conflicts=id,..".to_owned(),
                });
            }
            let clean = match kv_get(&pairs, "clean")? {
                "0" => false,
                "1" => true,
                other => {
                    return Err(ParseError::Unknown {
                        what: format!("clean={other}; want 0 or 1"),
                    });
                }
            };
            Ok(Gold::Verdict {
                clean,
                conflicts: parse_id_list(kv_get(&pairs, "conflicts")?)?,
            })
        }
        TaskKind::Query => {
            if kv_get(&pairs, "ids").is_ok() {
                if pairs.len() != 1 {
                    return Err(ParseError::Mismatch {
                        what: "query id gold wants exactly ids=..".to_owned(),
                    });
                }
                return Ok(Gold::Ids(parse_id_list(kv_get(&pairs, "ids")?)?));
            }
            if kv_get(&pairs, "empty").is_ok() {
                if pairs.len() != 2 || kv_get(&pairs, "min_clear").is_err() {
                    return Err(ParseError::Mismatch {
                        what: "query sample gold wants empty=0/1;min_clear=N".to_owned(),
                    });
                }
                let empty = match kv_get(&pairs, "empty")? {
                    "0" => false,
                    "1" => true,
                    other => {
                        return Err(ParseError::Unknown {
                            what: format!("empty={other}; want 0 or 1"),
                        });
                    }
                };
                let min_clear = parse_i32(kv_get(&pairs, "min_clear")?, "min_clear")?;
                if min_clear < 0 {
                    return Err(ParseError::OutOfRange {
                        what: format!("min_clear={min_clear}; want 0 or more"),
                    });
                }
                return Ok(Gold::Samples {
                    expect_empty: empty,
                    min_clear_cm: min_clear,
                });
            }
            Err(ParseError::Mismatch {
                what: "query gold wants ids=.. or empty=..;min_clear=..".to_owned(),
            })
        }
        TaskKind::Fix => {
            if pairs.len() != 2
                || kv_get(&pairs, "repair").is_err()
                || kv_get(&pairs, "at").is_err()
            {
                return Err(ParseError::Mismatch {
                    what: "fix gold wants repair=dx,dz;at=x,z".to_owned(),
                });
            }
            let (dx, dz) = parse_fix_pair(&pairs, "repair")?;
            let (x, z) = parse_fix_pair(&pairs, "at")?;
            check_coord(x, "at.x")?;
            check_coord(z, "at.z")?;
            Ok(Gold::Repair {
                dx_cm: dx,
                dz_cm: dz,
                x_cm: x,
                z_cm: z,
            })
        }
    }
}

pub(super) fn parse_vision(text: &str) -> Result<Vision, ParseError> {
    let (note, capture) = text.rsplit_once(';').ok_or_else(|| ParseError::Mismatch {
        what: "vision wants note;capture-hex".to_owned(),
    })?;
    if !note.starts_with("advisory:") {
        return Err(ParseError::Mismatch {
            what: format!("vision `{note}` must start with advisory:"),
        });
    }
    if capture.len() != 16 || !capture.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ParseError::Mismatch {
            what: format!("capture `{capture}` must be 16 hex digits"),
        });
    }
    if note.len() > 128 {
        return Err(ParseError::OutOfRange {
            what: "vision note past 128 chars".to_owned(),
        });
    }
    Ok(Vision {
        note: note.to_owned(),
        capture: capture.to_owned(),
    })
}

/// Parse one TSV row (seven tab fields) into a task.
pub(super) fn parse_row(line: &str) -> Result<Task, ParseError> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 7 {
        return Err(ParseError::Mismatch {
            what: format!("want 7 tab fields, saw {}", fields.len()),
        });
    }
    let id = fields[0];
    if id.len() != 8 || !id.starts_with("sp12-") || id[5..].parse::<u32>().is_err() {
        return Err(ParseError::Mismatch {
            what: format!("id `{id}`; want sp12-001..sp12-100"),
        });
    }
    let kind = TaskKind::parse(fields[1])?;
    let scene = scene_of(fields[2]).ok_or_else(|| ParseError::Unknown {
        what: format!(
            "scene `{}`; want room_pillar, room_open, room_tight",
            fields[2]
        ),
    })?;
    let tool = fields[3];
    let legal_tool = match kind {
        TaskKind::Place => tool == "scene.place",
        TaskKind::Fix => tool == "tx.preview",
        TaskKind::Query => tool.starts_with("scene.query "),
    };
    if !legal_tool {
        return Err(ParseError::Mismatch {
            what: format!("tool `{tool}` mismatches the task kind"),
        });
    }
    let params = parse_params(kind, fields[4])?;
    // Query tool words must name the params they carry.
    if kind == TaskKind::Query {
        let word = match &params {
            Params::Near { .. } => "near",
            Params::Between { .. } => "between",
            Params::Visible { .. } => "visible",
            Params::Reachable { .. } => "reachable",
            Params::CoverNear { .. } => "cover_near",
            Params::LargestOpen { .. } => "largest_open",
            Params::FreeSpace { .. } => "free_space",
            Params::Place { .. } => "near",
        };
        if tool != format!("scene.query {word}") {
            return Err(ParseError::Mismatch {
                what: format!("tool `{tool}` mismatches params for `{word}`"),
            });
        }
        let anchors_ok = match &params {
            Params::Near { anchor, .. } | Params::Reachable { anchor } => {
                actor_of(scene, *anchor).is_some()
            }
            Params::Between { a, b } => {
                actor_of(scene, *a).is_some() && actor_of(scene, *b).is_some()
            }
            Params::CoverNear { anchor, .. } => actor_of(scene, *anchor).is_some(),
            _ => true,
        };
        if !anchors_ok {
            return Err(ParseError::Mismatch {
                what: "anchor names no scene actor; re-read the scene".to_owned(),
            });
        }
    }
    let gold = parse_gold(kind, fields[5])?;
    // Gold ids must name scene actors.
    let gold_ids_ok = match &gold {
        Gold::Verdict { conflicts, .. } => {
            conflicts.iter().all(|id| actor_of(scene, *id).is_some())
        }
        Gold::Ids(ids) => ids.iter().all(|id| actor_of(scene, *id).is_some()),
        Gold::Repair { .. } | Gold::Samples { .. } => true,
    };
    if !gold_ids_ok {
        return Err(ParseError::Mismatch {
            what: "gold names an actor outside the scene".to_owned(),
        });
    }
    let vision = parse_vision(fields[6])?;
    if vision.capture != capture_hex(id) {
        return Err(ParseError::Mismatch {
            what: format!("capture stamp mismatch for `{id}`"),
        });
    }
    Ok(Task {
        id: id.to_owned(),
        scene: scene.name.to_owned(),
        kind,
        tool: tool.to_owned(),
        params,
        gold,
        vision,
    })
}

/// Parse checked-in TSV text into tasks. Strict: version line, header,
/// exactly [`TASK_COUNT`] rows with sequential ids and the 30/40/30 split.
/// Refusals are typed [`ParseError`]; callers at the IO boundary render the
/// reason into [`SpatialError`].
pub fn parse_tsv(text: &str) -> Result<Vec<Task>, ParseError> {
    let mut lines = text.lines();
    let version = lines.next().ok_or_else(|| ParseError::Mismatch {
        what: "empty fixture; want the v1 version line".to_owned(),
    })?;
    if version.trim_end_matches('\r') != TSV_VERSION_LINE {
        return Err(ParseError::Mismatch {
            what: format!("version `{version}`; want `{TSV_VERSION_LINE}`"),
        });
    }
    let header = lines.next().ok_or_else(|| ParseError::Mismatch {
        what: "missing TSV header".to_owned(),
    })?;
    if header.trim_end_matches('\r') != TSV_HEADER {
        return Err(ParseError::Mismatch {
            what: "header mismatch; want id/kind/scene/tool/params/gold/vision".to_owned(),
        });
    }
    let mut tasks = Vec::with_capacity(TASK_COUNT + 1);
    for line in lines {
        let row = line.trim_end_matches('\r');
        if row.is_empty() || row.starts_with('#') {
            continue;
        }
        let task = parse_row(row)
            .map_err(|error| ParseError::context(format!("row {}", tasks.len() + 1), error))?;
        tasks.push(task);
    }
    if tasks.len() != TASK_COUNT {
        return Err(ParseError::Mismatch {
            what: format!("saw {} tasks; want {TASK_COUNT}", tasks.len()),
        });
    }
    for (i, task) in tasks.iter().enumerate() {
        let want = format!("sp12-{:03}", i + 1);
        if task.id != want {
            return Err(ParseError::Mismatch {
                what: format!("row {} id `{}`; want `{want}`", i + 1, task.id),
            });
        }
    }
    let (mut place, mut query, mut fix) = (0, 0, 0);
    for task in &tasks {
        match task.kind {
            TaskKind::Place => place += 1,
            TaskKind::Query => query += 1,
            TaskKind::Fix => fix += 1,
        }
    }
    if (place, query, fix) != (PLACE_COUNT, QUERY_COUNT, FIX_COUNT) {
        return Err(ParseError::Mismatch {
            what: format!(
                "split place/query/fix {place}/{query}/{fix}; want {PLACE_COUNT}/{QUERY_COUNT}/{FIX_COUNT}"
            ),
        });
    }
    Ok(tasks)
}
