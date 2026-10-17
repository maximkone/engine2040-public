use super::*;

/// Scalar header of one spec: identity, seed, shape and phase lengths. Split
/// out of [`parse_spec`] so each fn stays a named step.
pub(super) struct SpecHead {
    pub(super) id: String,
    pub(super) claim: String,
    pub(super) seed: u64,
    pub(super) fighters: u8,
    pub(super) half_extent_m: f32,
    pub(super) prefix_ticks: u32,
    pub(super) window_ticks: u32,
}

pub(super) fn parse_spec_head(
    entries: &[(String, Json)],
    stem: &str,
) -> Result<SpecHead, ParseError> {
    let version = field(entries, "version")
        .and_then(|value| as_u32(value, "version", u32::from(u16::MAX)))?;
    if version != u32::from(SPEC_VERSION) {
        return Err(ParseError::Mismatch {
            what: format!("unsupported spec version {version}"),
        });
    }
    let id = field(entries, "id").and_then(|value| as_str(value, "id"))?;
    if id != stem {
        return Err(ParseError::Mismatch {
            what: format!("id `{id}` must match file stem `{stem}`"),
        });
    }
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
    {
        return Err(ParseError::Mismatch {
            what: format!("bad id `{id}`"),
        });
    }
    let claim = field(entries, "claim").and_then(|value| as_str(value, "claim"))?;
    if claim.is_empty() || claim.chars().count() > MAX_CLAIM_CHARS {
        return Err(ParseError::OutOfRange {
            what: "claim length out of range".to_owned(),
        });
    }
    let seed = field(entries, "seed").and_then(|value| as_i64(value, "seed"))?;
    if seed < 0 {
        return Err(ParseError::OutOfRange {
            what: "seed must be non-negative".to_owned(),
        });
    }
    let fighters = field(entries, "fighters").and_then(|value| as_u32(value, "fighters", 255))?;
    if fighters < 2 || fighters > u32::from(engine_arena::MAX_PARTICIPANTS) {
        return Err(ParseError::OutOfRange {
            what: format!("fighters {fighters} outside 2..=4"),
        });
    }
    let half_extent_m = field(entries, "half_extent_m").and_then(|value| match value {
        Json::Int(number) => {
            i32::try_from(*number)
                .map(f64::from)
                .map_err(|_| ParseError::WrongType {
                    field: "half_extent_m".to_owned(),
                    want: "a number",
                })
        }
        Json::Float(number) => Ok(*number),
        _ => Err(ParseError::WrongType {
            field: "half_extent_m".to_owned(),
            want: "a number",
        }),
    })?;
    if !(5.0..=100.0).contains(&half_extent_m) {
        return Err(ParseError::OutOfRange {
            what: "half_extent_m outside 5.0..=100.0".to_owned(),
        });
    }
    let prefix_ticks = field(entries, "prefix_ticks")
        .and_then(|value| as_u32(value, "prefix_ticks", MAX_PREFIX_TICKS))?;
    let window_ticks = field(entries, "window_ticks")
        .and_then(|value| as_u32(value, "window_ticks", MAX_WINDOW_TICKS))?;
    if window_ticks == 0 {
        return Err(ParseError::OutOfRange {
            what: "window_ticks must be positive".to_owned(),
        });
    }
    Ok(SpecHead {
        id: id.to_owned(),
        claim: claim.to_owned(),
        seed: u64::try_from(seed).map_err(|_| ParseError::OutOfRange {
            what: "seed must be non-negative".to_owned(),
        })?,
        fighters: u8::try_from(fighters).map_err(|_| ParseError::OutOfRange {
            what: format!("fighters {fighters} outside 2..=4"),
        })?,
        // Range-checked to 5.0..=100.0 above, so the narrowing stays finite
        // and deterministic.
        half_extent_m: half_extent_m as f32,
        prefix_ticks,
        window_ticks,
    })
}

pub(super) fn parse_spec(value: &Json, stem: &str) -> Result<KeypointSpec, ParseError> {
    let entries = as_obj(value)?;
    let head = parse_spec_head(entries, stem)?;
    let setup = field(entries, "setup").and_then(|value| as_arr(value, "setup"))?;
    let setup = parse_inputs(setup, 0, head.prefix_ticks, head.fighters, "setup")
        .map_err(|error| ParseError::context("setup", error))?;
    let window = field(entries, "window").and_then(|value| as_arr(value, "window"))?;
    let window = parse_inputs(
        window,
        head.prefix_ticks,
        head.prefix_ticks.saturating_add(head.window_ticks),
        head.fighters,
        "window",
    )
    .map_err(|error| ParseError::context("window", error))?;
    let freeze = match optional(entries, "freeze")? {
        None => Vec::new(),
        Some(value) => {
            let items = as_arr(value, "freeze")?;
            parse_slots(items)?
        }
    };
    if freeze.iter().any(|slot| *slot >= head.fighters) {
        return Err(ParseError::OutOfRange {
            what: "freeze slot outside fighters".to_owned(),
        });
    }
    let asserts = match optional(entries, "assert")? {
        None => KeypointAsserts::default(),
        Some(value) => parse_asserts(value, head.fighters, head.window_ticks)
            .map_err(|error| ParseError::context("assert", error))?,
    };
    let spine = match optional(entries, "track")? {
        None => false,
        Some(value) => match as_str(value, "track")? {
            "fork" => false,
            "spine" => true,
            other => {
                return Err(ParseError::Mismatch {
                    what: format!("unknown track `{other}`"),
                })
            }
        },
    };
    if spine && !(asserts.damage.is_empty() && asserts.stuck.is_empty() && freeze.is_empty()) {
        return Err(ParseError::Mismatch {
            what: "spine track cannot assert damage or stuck, or freeze".to_owned(),
        });
    }
    if !spine && asserts.reads_spine() {
        return Err(ParseError::Mismatch {
            what: "state, stamina and reach_mm need track `spine`".to_owned(),
        });
    }
    if entries.iter().any(|(key, _)| {
        ![
            "version",
            "id",
            "claim",
            "track",
            "seed",
            "fighters",
            "half_extent_m",
            "prefix_ticks",
            "window_ticks",
            "setup",
            "window",
            "freeze",
            "assert",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ParseError::Unknown {
            what: "unknown field".to_owned(),
        });
    }
    Ok(KeypointSpec {
        spine,
        id: head.id,
        claim: head.claim,
        seed: head.seed,
        fighters: head.fighters,
        half_extent_m: head.half_extent_m,
        prefix_ticks: head.prefix_ticks,
        setup,
        window_ticks: head.window_ticks,
        window,
        freeze,
        asserts,
    })
}

pub(super) fn parse_slots(items: &[Json]) -> Result<Vec<u8>, ParseError> {
    let mut slots = Vec::new();
    for item in items {
        let slot = as_u32(item, "freeze slot", 255)?;
        let slot = u8::try_from(slot).map_err(|_| ParseError::OutOfRange {
            what: "freeze slot out of range".to_owned(),
        })?;
        if slots.contains(&slot) {
            return Err(ParseError::Mismatch {
                what: "duplicate freeze slot".to_owned(),
            });
        }
        slots.push(slot);
    }
    Ok(slots)
}

/// One validated script entry: fighter, wishes, facing and buttons. Split out
/// of [`parse_inputs`] so each fn stays a named step.
pub(super) struct ScriptShape {
    pub(super) fighter: u8,
    pub(super) mx: i8,
    pub(super) mz: i8,
    pub(super) facing: i16,
    pub(super) buttons: u8,
}

pub(super) fn parse_script_shape(
    entries: &[(String, Json)],
    fighters: u8,
) -> Result<ScriptShape, ParseError> {
    let fighter = field(entries, "fighter").and_then(|value| as_u32(value, "fighter", 255))?;
    if fighter >= u32::from(fighters) {
        return Err(ParseError::OutOfRange {
            what: format!("fighter {fighter} outside slots"),
        });
    }
    let mx = field(entries, "mx").and_then(|value| as_i64(value, "mx"))?;
    let mz = field(entries, "mz").and_then(|value| as_i64(value, "mz"))?;
    if !(-100..=100).contains(&mx) || !(-100..=100).contains(&mz) {
        return Err(ParseError::OutOfRange {
            what: "wish outside -100..=100".to_owned(),
        });
    }
    let facing = field(entries, "facing").and_then(|value| as_i64(value, "facing"))?;
    if !(-18_000..=18_000).contains(&facing) {
        return Err(ParseError::OutOfRange {
            what: "facing outside -18000..=18000".to_owned(),
        });
    }
    let buttons = field(entries, "buttons").and_then(|value| as_u32(value, "buttons", 255))?;
    if buttons > 7 {
        return Err(ParseError::OutOfRange {
            what: format!("buttons {buttons} use unknown bits"),
        });
    }
    let narrow_fighter = u8::try_from(fighter).map_err(|_| ParseError::OutOfRange {
        what: format!("fighter {fighter} outside slots"),
    })?;
    let narrow_mx = i8::try_from(mx).map_err(|_| ParseError::OutOfRange {
        what: "wish outside -100..=100".to_owned(),
    })?;
    let narrow_mz = i8::try_from(mz).map_err(|_| ParseError::OutOfRange {
        what: "wish outside -100..=100".to_owned(),
    })?;
    let narrow_facing = i16::try_from(facing).map_err(|_| ParseError::OutOfRange {
        what: "facing outside -18000..=18000".to_owned(),
    })?;
    let narrow_buttons = u8::try_from(buttons).map_err(|_| ParseError::OutOfRange {
        what: format!("buttons {buttons} use unknown bits"),
    })?;
    Ok(ScriptShape {
        fighter: narrow_fighter,
        mx: narrow_mx,
        mz: narrow_mz,
        facing: narrow_facing,
        buttons: narrow_buttons,
    })
}

pub(super) fn push_script_tick(
    out: &mut Vec<ScriptInput>,
    shape: &ScriptShape,
    tick: u32,
    lower: u32,
    upper: u32,
) -> Result<(), ParseError> {
    if tick < lower || tick >= upper {
        return Err(ParseError::OutOfRange {
            what: format!("tick {tick} outside phase"),
        });
    }
    out.push(ScriptInput {
        tick,
        fighter: shape.fighter,
        mx: shape.mx,
        mz: shape.mz,
        facing: shape.facing,
        buttons: shape.buttons,
    });
    if out.len() > 4096 {
        return Err(ParseError::OutOfRange {
            what: "too many inputs".to_owned(),
        });
    }
    Ok(())
}

/// Expand one script entry: a single `tick` or an inclusive `from`/`to` hold.
pub(super) fn expand_script_entry(
    entries: &[(String, Json)],
    shape: &ScriptShape,
    lower: u32,
    upper: u32,
    out: &mut Vec<ScriptInput>,
) -> Result<(), ParseError> {
    match (optional(entries, "tick")?, optional(entries, "from")?) {
        (Some(tick), None) => {
            let tick = as_u32(tick, "tick", u32::MAX)?;
            push_script_tick(out, shape, tick, lower, upper)?;
        }
        (None, Some(from)) => {
            let to = field(entries, "to")?;
            let from = as_u32(from, "from", u32::MAX)?;
            let to = as_u32(to, "to", u32::MAX)?;
            if from > to {
                return Err(ParseError::OutOfRange {
                    what: format!("hold from {from} past to {to}"),
                });
            }
            if to - from > 512 {
                return Err(ParseError::OutOfRange {
                    what: "hold over 512 ticks".to_owned(),
                });
            }
            for tick in from..=to {
                push_script_tick(out, shape, tick, lower, upper)?;
            }
        }
        _ => {
            return Err(ParseError::Mismatch {
                what: "entry needs `tick` or `from`+`to`".to_owned(),
            });
        }
    }
    if entries.iter().any(|(key, _)| {
        ![
            "fighter", "mx", "mz", "facing", "buttons", "tick", "from", "to",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ParseError::Unknown {
            what: "unknown entry field".to_owned(),
        });
    }
    Ok(())
}

/// Parse one script phase: single-tick entries carry `tick`, hold entries
/// carry inclusive `from`/`to`. Bounds are phase-relative so a window entry
/// can never name a setup tick.
pub(super) fn parse_inputs(
    items: &[Json],
    lower: u32,
    upper: u32,
    fighters: u8,
    phase: &str,
) -> Result<Vec<ScriptInput>, ParseError> {
    let mut out = Vec::new();
    for item in items {
        let entries = as_obj(item).map_err(|error| ParseError::context(phase, error))?;
        let shape = parse_script_shape(entries, fighters)
            .map_err(|error| ParseError::context(phase, error))?;
        expand_script_entry(entries, &shape, lower, upper, &mut out)
            .map_err(|error| ParseError::context(phase, error))?;
    }
    out.sort_by_key(|input| (input.tick, input.fighter));
    let mut seen = BTreeMap::new();
    for input in &out {
        if seen.insert((input.tick, input.fighter), true).is_some() {
            return Err(ParseError::context(
                phase,
                ParseError::Mismatch {
                    what: format!(
                        "duplicate input at tick {} fighter {}",
                        input.tick, input.fighter
                    ),
                },
            ));
        }
    }
    Ok(out)
}

pub(super) fn parse_pairs(
    items: &[Json],
    key: &str,
    max: u32,
) -> Result<Vec<(u8, u32)>, ParseError> {
    let mut out = Vec::new();
    for item in items {
        let pair = as_arr(item, key).map_err(|error| ParseError::context(key, error))?;
        if pair.len() != 2 {
            return Err(ParseError::Mismatch {
                what: format!("{key}: pair must have two entries"),
            });
        }
        let slot = as_u32(&pair[0], key, 255)?;
        let value = as_u32(&pair[1], key, max)?;
        let slot = u8::try_from(slot).map_err(|_| ParseError::OutOfRange {
            what: format!("{key}: slot out of range"),
        })?;
        if out.iter().any(|(known, _)| *known == slot) {
            return Err(ParseError::Mismatch {
                what: format!("{key}: duplicate slot {slot}"),
            });
        }
        out.push((slot, value));
    }
    out.sort_by_key(|(slot, _)| *slot);
    Ok(out)
}

/// One checked `(slot, value)` assert list: optional field, pair shape and
/// fighter range in one step. Split out of [`parse_asserts`].
pub(super) fn checked_pairs(
    entries: &[(String, Json)],
    key: &str,
    fighters: u8,
    max: u32,
) -> Result<Vec<(u8, u32)>, ParseError> {
    let Some(value) = optional(entries, key)? else {
        return Ok(Vec::new());
    };
    let items = as_arr(value, key).map_err(|error| ParseError::context("assert", error))?;
    let pairs =
        parse_pairs(items, key, max).map_err(|error| ParseError::context("assert", error))?;
    for (slot, _) in &pairs {
        if *slot >= fighters {
            return Err(ParseError::OutOfRange {
                what: format!("assert {key} slot outside fighters"),
            });
        }
    }
    Ok(pairs)
}

/// Narrow one `(slot, value)` list to `u16` values for the report.
pub(super) fn narrow_pairs(pairs: &[(u8, u32)], key: &str) -> Result<Vec<(u8, u16)>, ParseError> {
    let mut out = Vec::with_capacity(pairs.len());
    for (slot, value) in pairs {
        let narrowed = u16::try_from(*value).map_err(|_| ParseError::OutOfRange {
            what: format!("assert {key} value {value} outside u16"),
        })?;
        out.push((*slot, narrowed));
    }
    Ok(out)
}

/// Parse the window-relative damage ticks, exact and in range.
pub(super) fn parse_damage_ticks(
    items: &[Json],
    window_ticks: u32,
) -> Result<Vec<u32>, ParseError> {
    let mut list = Vec::with_capacity(items.len());
    for item in items {
        let tick = as_u32(item, "damage_ticks", u32::MAX)?;
        if tick >= window_ticks {
            return Err(ParseError::OutOfRange {
                what: "assert damage tick outside window".to_owned(),
            });
        }
        if list.contains(&tick) {
            return Err(ParseError::Mismatch {
                what: "assert duplicate damage tick".to_owned(),
            });
        }
        list.push(tick);
    }
    list.sort_unstable();
    Ok(list)
}

pub(super) fn parse_asserts(
    value: &Json,
    fighters: u8,
    window_ticks: u32,
) -> Result<KeypointAsserts, ParseError> {
    let entries = as_obj(value)?;
    let hp = narrow_pairs(&checked_pairs(entries, "hp", fighters, 1000)?, "hp")?;
    let damage = narrow_pairs(
        &checked_pairs(entries, "damage", fighters, 65_535)?,
        "damage",
    )?;
    let stuck = checked_pairs(entries, "stuck", fighters, u32::MAX)?;
    let state = narrow_pairs(&checked_pairs(entries, "state", fighters, 31)?, "state")?;
    let stamina = narrow_pairs(
        &checked_pairs(entries, "stamina", fighters, 65_535)?,
        "stamina",
    )?;
    let mut asserts = KeypointAsserts {
        hp,
        damage,
        stuck,
        state: state
            .into_iter()
            .map(|(slot, bits)| (slot, u8::try_from(bits).unwrap_or(u8::MAX)))
            .collect(),
        stamina,
        reach_mm: checked_pairs(entries, "reach_mm", fighters, 1_000_000)?,
        ..KeypointAsserts::default()
    };
    if let Some(killed) = optional(entries, "killed")? {
        asserts.killed = Some(as_bool(killed, "killed")?);
    }
    if let Some(ticks) = optional(entries, "damage_ticks")? {
        let items = as_arr(ticks, "damage_ticks")?;
        asserts.damage_ticks = Some(parse_damage_ticks(items, window_ticks)?);
    }
    if let Some(ttk) = optional(entries, "ticks_to_kill")? {
        match ttk {
            Json::Null => asserts.ticks_to_kill = Some(None),
            _ => {
                let tick = as_u32(ttk, "ticks_to_kill", u32::MAX)?;
                if tick == 0 || tick > window_ticks + 1 {
                    return Err(ParseError::OutOfRange {
                        what: "assert kill tick outside window".to_owned(),
                    });
                }
                asserts.ticks_to_kill = Some(Some(tick));
            }
        }
    }
    if entries.iter().any(|(key, _)| {
        ![
            "hp",
            "damage",
            "stuck",
            "killed",
            "damage_ticks",
            "ticks_to_kill",
            "state",
            "stamina",
            "reach_mm",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ParseError::context(
            "assert",
            ParseError::Unknown {
                what: "unknown field".to_owned(),
            },
        ));
    }
    Ok(asserts)
}

// ---- tape building ----------------------------------------------------------

/// Canonical template input every synthetic tape derives from via
/// struct-update syntax, so this module never mirrors button constants.
pub(super) fn template_input() -> FighterInput {
    static TEMPLATE: OnceLock<FighterInput> = OnceLock::new();
    *TEMPLATE.get_or_init(|| {
        let config = ArenaConfig {
            seed: 2040,
            ticks: 8,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        run_dataset(&config, &FighterTape::default())
            .and_then(|dataset| dataset.rows.into_iter().next().map(|row| row.action.input))
            .unwrap_or_else(|| FighterInput {
                tick: 0,
                fighter: engine_arena::FighterId::new(0).unwrap_or(engine_arena::FighterId(0)),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Default::default(),
            })
    })
}

pub(super) fn make_input(
    tick: u32,
    fighter: u8,
    mx: i8,
    mz: i8,
    facing: i16,
    buttons: u8,
) -> Option<FighterInput> {
    let id = engine_arena::FighterId::new(fighter)?;
    let template = template_input();
    let mut input = template;
    input.tick = tick;
    input.fighter = id;
    input.mx = mx;
    input.mz = mz;
    input.facing = facing;
    input.buttons.0 = buttons;
    input.validate().ok()?;
    Some(input)
}

/// Materialize full-coverage tapes: every `(tick, slot)` runs the scripted
/// override or idle, so brains are never consulted and the run is exact.
pub(super) fn materialize(
    overrides: &BTreeMap<(u32, u8), ScriptInput>,
    lower: u32,
    upper: u32,
    fighters: u8,
) -> Option<FighterTape> {
    let mut inputs = Vec::new();
    for tick in lower..upper {
        for slot in 0..fighters {
            let (mx, mz, facing, buttons) = match overrides.get(&(tick, slot)) {
                Some(entry) => (entry.mx, entry.mz, entry.facing, entry.buttons),
                None => (0, 0, 0, 0),
            };
            inputs.push(make_input(tick, slot, mx, mz, facing, buttons)?);
        }
    }
    let tape = FighterTape(inputs);
    tape.validate().ok()?;
    Some(tape)
}
