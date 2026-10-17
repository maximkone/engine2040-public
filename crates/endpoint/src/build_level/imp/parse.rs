use super::*;

pub(super) enum RawTarget {
    Wall(Wall),
    Point(i32, i32),
    Object(String),
}

pub(super) struct RawWant {
    pub(super) relation: Relation,
    pub(super) target: RawTarget,
    pub(super) range: Option<(i32, i32)>,
    pub(super) source: String,
}

pub(super) struct ParsedBrief {
    pub(super) kinds: BTreeMap<String, String>,
    pub(super) wants: BTreeMap<String, Vec<RawWant>>,
    pub(super) spawn: Option<String>,
}

pub(super) fn bad_line(line_no: usize, detail: &str) -> LevelError {
    LevelError::BadLine {
        line: line_no,
        reason: format!(
            "{detail}; grammar is `prop <kind> <name>`, \
                 `want <name> <relation> <target> [<lo>,<hi>]?` or `spawn <name>` with \
                 target `wall:<north|south|east|west>`, `object:<name>` or \
                 `point:<x_cm>,<z_cm>`"
        ),
    }
}

pub(super) fn parse_relation_word(word: &str) -> Option<Relation> {
    match word {
        "next_to" => Some(Relation::NextTo),
        "facing" => Some(Relation::Facing),
        "inside" => Some(Relation::Inside),
        "on_top_of" => Some(Relation::OnTopOf),
        "supports" => Some(Relation::Supports),
        _ => None,
    }
}

pub(super) fn parse_wall_word(word: &str) -> Option<Wall> {
    match word {
        "north" => Some(Wall::North),
        "south" => Some(Wall::South),
        "east" => Some(Wall::East),
        "west" => Some(Wall::West),
        _ => None,
    }
}

pub(super) fn parse_int_word(word: &str, line_no: usize, what: &str) -> Result<i32, LevelError> {
    word.parse::<i32>().map_err(|_| {
        bad_line(
            line_no,
            &format!("bad {what} `{word}` (expected an integer)"),
        )
    })
}

/// Brief grammar, one directive per line, `#` comments ignored. Prop and
/// want order never matters: props sort by name, wants keep line order per
/// prop, so reordered brief lines plan identically.
pub(super) fn parse_brief(text: &str) -> Result<ParsedBrief, LevelError> {
    let mut kinds: BTreeMap<String, String> = BTreeMap::new();
    let mut wants: BTreeMap<String, Vec<RawWant>> = BTreeMap::new();
    let mut spawn: Option<String> = None;
    let mut lines = 0usize;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        lines += 1;
        if lines > BRIEF_MAX_LINES {
            return Err(bad_line(
                line_no,
                &format!("more than {BRIEF_MAX_LINES} directives"),
            ));
        }
        if line.len() > LINE_MAX_CHARS {
            return Err(bad_line(
                line_no,
                &format!("longer than {LINE_MAX_CHARS} chars"),
            ));
        }
        let mut parts = line.split_whitespace();
        let head = parts.next().unwrap_or("");
        match head {
            "prop" => {
                let kind = parts
                    .next()
                    .ok_or_else(|| bad_line(line_no, "prop needs `<kind> <name>`"))?;
                let name = parts
                    .next()
                    .ok_or_else(|| bad_line(line_no, "prop needs `<kind> <name>`"))?;
                if parts.next().is_some() {
                    return Err(bad_line(line_no, "prop takes exactly `<kind> <name>`"));
                }
                if kind_size(kind).is_none() {
                    return Err(LevelError::UnknownKind {
                        kind: kind.to_owned(),
                    });
                }
                if !valid_name(name) {
                    return Err(bad_line(
                        line_no,
                        &format!(
                            "bad name `{name}` (lowercase letters, digits and `_`, \
                                 no leading digit, at most {NAME_MAX} chars)"
                        ),
                    ));
                }
                if kinds.contains_key(name) {
                    return Err(LevelError::DuplicateName {
                        name: name.to_owned(),
                    });
                }
                kinds.insert(name.to_owned(), kind.to_owned());
            }
            "want" => {
                let name = parts
                    .next()
                    .ok_or_else(|| bad_line(line_no, "want needs a prop name"))?;
                let relation_word = parts
                    .next()
                    .ok_or_else(|| bad_line(line_no, "want needs a relation"))?;
                let target_word = parts
                    .next()
                    .ok_or_else(|| bad_line(line_no, "want needs a target"))?;
                let range_word = parts.next();
                if parts.next().is_some() {
                    return Err(bad_line(line_no, "want takes at most one range"));
                }
                if !valid_name(name) {
                    return Err(bad_line(line_no, &format!("bad name `{name}`")));
                }
                let relation = parse_relation_word(relation_word).ok_or_else(|| {
                    bad_line(
                        line_no,
                        &format!(
                            "unknown relation `{relation_word}`; submit one of {}",
                            RELATIONS.join(", ")
                        ),
                    )
                })?;
                let target = match target_word.split_once(':') {
                    Some(("wall", word)) => RawTarget::Wall(
                        parse_wall_word(word)
                            .ok_or_else(|| bad_line(line_no, &format!("unknown wall `{word}`")))?,
                    ),
                    Some(("object", word)) => {
                        if !valid_name(word) {
                            return Err(bad_line(line_no, &format!("bad object name `{word}`")));
                        }
                        RawTarget::Object(word.to_owned())
                    }
                    Some(("point", coords)) => {
                        let (x_word, z_word) = coords
                            .split_once(',')
                            .ok_or_else(|| bad_line(line_no, "point needs `<x_cm>,<z_cm>`"))?;
                        RawTarget::Point(
                            parse_int_word(x_word, line_no, "x_cm")?,
                            parse_int_word(z_word, line_no, "z_cm")?,
                        )
                    }
                    _ => {
                        return Err(bad_line(
                            line_no,
                            "target needs a `wall:|object:|point:` kind",
                        ));
                    }
                };
                let range = match range_word {
                    None => None,
                    Some(word) => {
                        let inner = word
                            .strip_prefix('[')
                            .and_then(|text| text.strip_suffix(']'))
                            .ok_or_else(|| bad_line(line_no, "range needs `[<lo>,<hi>]`"))?;
                        let (lo_word, hi_word) = inner
                            .split_once(',')
                            .ok_or_else(|| bad_line(line_no, "range needs `[<lo>,<hi>]`"))?;
                        let lo = parse_int_word(lo_word, line_no, "lo")?;
                        let hi = parse_int_word(hi_word, line_no, "hi")?;
                        if lo < 0 || hi < lo {
                            return Err(bad_line(
                                line_no,
                                &format!("bad range `[{lo},{hi}]` (need 0 <= lo <= hi)"),
                            ));
                        }
                        Some((lo, hi))
                    }
                };
                wants.entry(name.to_owned()).or_default().push(RawWant {
                    relation,
                    target,
                    range,
                    source: line.to_owned(),
                });
            }
            "spawn" => {
                let name = parts
                    .next()
                    .ok_or_else(|| bad_line(line_no, "spawn needs a prop name"))?;
                if parts.next().is_some() {
                    return Err(bad_line(line_no, "spawn takes exactly one name"));
                }
                if !valid_name(name) {
                    return Err(bad_line(line_no, &format!("bad name `{name}`")));
                }
                if spawn.is_some() {
                    return Err(bad_line(line_no, "spawn takes exactly one prop"));
                }
                spawn = Some(name.to_owned());
            }
            other => {
                return Err(bad_line(
                    line_no,
                    &format!("unknown directive `{other}` (submit prop, want or spawn)"),
                ));
            }
        }
    }
    if kinds.len() > PROPS_MAX {
        return Err(LevelError::TooManyProps { count: kinds.len() });
    }
    for name in wants.keys() {
        if !kinds.contains_key(name) {
            return Err(LevelError::UnknownName { name: name.clone() });
        }
    }
    if let Some(name) = &spawn {
        if !kinds.contains_key(name) {
            return Err(LevelError::UnknownSpawn { name: name.clone() });
        }
    }
    if kinds.is_empty() {
        return Err(LevelError::EmptyBrief);
    }
    Ok(ParsedBrief {
        kinds,
        wants,
        spawn,
    })
}
