use super::*;

/// One planned prop: stable id, catalog size and solver wants with
/// object targets resolved to actor ids.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedProp {
    /// Prop name from the brief.
    pub name: String,
    /// Catalog kind from the brief.
    pub kind: String,
    /// Stable actor id (name-sorted 1-based).
    pub id: u32,
    /// Solver wants in brief line order.
    pub wants: Vec<Want>,
    /// Source want lines in the same order (conflicts echo these).
    pub sources: Vec<String>,
}

/// Relation plan: every prop plus the nav spawn. Name-sorted, so brief
/// line order never moves the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationPlan {
    /// Props in name order.
    pub props: Vec<PlannedProp>,
    /// Actor id the nav routes start from.
    pub spawn_id: u32,
}

/// Typed constraint category: every compiled want lands in exactly one
/// per-category vector before placement solves. Counts are deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConstraintCategory {
    /// Prop declarations (`prop <kind> <name>`).
    Declare,
    /// Relation targets without ranges.
    Relate,
    /// Distance bounds, one per relate entry.
    Bound,
    /// Nav spawn (`spawn <name>`, default first prop).
    Spawn,
}

impl ConstraintCategory {
    /// Stable tag for notes and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declare => "declare",
            Self::Relate => "relate",
            Self::Bound => "bound",
            Self::Spawn => "spawn",
        }
    }
}

/// One declared prop: catalog kind plus its stable actor id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclareConstraint {
    /// Prop name from the brief.
    pub name: String,
    /// Catalog kind from the brief.
    pub kind: String,
    /// Stable actor id (name-sorted 1-based).
    pub id: u32,
}

/// One relation want without its range: typed target, source line kept
/// so conflicts echo the brief.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelateConstraint {
    /// Owning prop actor id.
    pub prop: u32,
    /// Relation word.
    pub relation: Relation,
    /// Resolved target (object names become ids here).
    pub target: Target,
    /// Source brief line.
    pub source: String,
}

/// One range bound for a relate entry: explicit or solver-defaulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundConstraint {
    /// Owning prop actor id.
    pub prop: u32,
    /// Index into the prop's relate entries.
    pub want_idx: usize,
    /// Inclusive lower bound in cm.
    pub lo: i32,
    /// Inclusive upper bound in cm.
    pub hi: i32,
    /// True when the brief gave no range and the solver default applies.
    pub defaulted: bool,
}

/// Semantic want bucket: every compiled relate lands in exactly one
/// bucket before the placement solver runs, so pre-solve validation and
/// repair budget account per bucket. Pure tag over relation plus target;
/// the same relate names the same bucket on every machine, and the
/// solver never sees this tag. Repair budget is shared: at most
/// `MAX_REPAIRS` mutating repairs across all four buckets, so no single
/// bucket can exceed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WantCategory {
    /// Grounded anchors: `next_to` a wall or a room point.
    Placement,
    /// Prop separation: `next_to` another prop; spacing feeds nav
    /// clearance.
    SeparationNav,
    /// Stacking and containment: `inside`, `on_top_of`, `supports`.
    SupportSurface,
    /// Approach and reach: `facing` any target, plus the spawn directive
    /// (counted here by `want_histogram`); these feed route reachability.
    SpawnReach,
}

impl WantCategory {
    /// Stable tag for notes and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Placement => "placement",
            Self::SeparationNav => "separation_nav",
            Self::SupportSurface => "support_surface",
            Self::SpawnReach => "spawn_reach",
        }
    }

    /// Histogram slot for this bucket, in
    /// `[Placement, SeparationNav, SupportSurface, SpawnReach]` order.
    /// Named conversion so histogram indexing never casts.
    pub fn index(self) -> usize {
        match self {
            Self::Placement => 0,
            Self::SeparationNav => 1,
            Self::SupportSurface => 2,
            Self::SpawnReach => 3,
        }
    }
}

/// Tag one relate by relation plus target. Total: every relation word
/// the brief grammar accepts lands in exactly one bucket. Deterministic
/// and allocation-free.
pub fn categorize_want(relation: Relation, target: Target) -> WantCategory {
    match (relation, target) {
        (Relation::Inside | Relation::OnTopOf | Relation::Supports, _) => {
            WantCategory::SupportSurface
        }
        (Relation::Facing, _) => WantCategory::SpawnReach,
        (Relation::NextTo, Target::Object(_)) => WantCategory::SeparationNav,
        (Relation::NextTo, _) => WantCategory::Placement,
    }
}

/// Constraint-first compile of one brief: typed per-category constraints
/// before placement solves. Bounded diagnose-and-repair loop with greedy
/// fixes only (no optimizer): dedup, room clamp and self-target drop,
/// at most `MAX_REPAIRS` mutating repairs. Deterministic: same brief
/// text gives the same categories, order and notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledBrief {
    /// Declares in prop-id order.
    pub declares: Vec<DeclareConstraint>,
    /// Relates grouped by prop: walls, points, objects, facing last.
    pub relates: Vec<RelateConstraint>,
    /// One bound per relate entry, same order.
    pub bounds: Vec<BoundConstraint>,
    /// Actor id the nav routes start from.
    pub spawn_id: u32,
    /// Mutating compile repairs consumed (at most `MAX_REPAIRS`).
    pub compile_repairs: u8,
    /// One note per normalization or repair, in apply order.
    pub notes: Vec<String>,
}

impl CompiledBrief {
    /// Count constraints in one category (spawn counts one).
    pub fn category_count(&self, category: ConstraintCategory) -> usize {
        match category {
            ConstraintCategory::Declare => self.declares.len(),
            ConstraintCategory::Relate => self.relates.len(),
            ConstraintCategory::Bound => self.bounds.len(),
            ConstraintCategory::Spawn => 1,
        }
    }

    /// Total constraints across all four categories.
    pub fn constraint_count(&self) -> usize {
        self.declares.len() + self.relates.len() + self.bounds.len() + 1
    }

    /// Semantic bucket per relate entry, in relate order.
    pub fn want_categories(&self) -> Vec<WantCategory> {
        self.relates
            .iter()
            .map(|relate| categorize_want(relate.relation, relate.target))
            .collect()
    }

    /// Histogram over the four semantic buckets in
    /// `[Placement, SeparationNav, SupportSurface, SpawnReach]` order;
    /// the spawn directive counts one in `SpawnReach`. Deterministic:
    /// the same compiled brief gives the same counts on every machine.
    pub fn want_histogram(&self) -> [usize; 4] {
        let mut counts = [0usize; 4];
        for relate in &self.relates {
            counts[categorize_want(relate.relation, relate.target).index()] += 1;
        }
        counts[WantCategory::SpawnReach.index()] += 1;
        counts
    }

    /// Pre-solve validation over the compiled form: relates and bounds
    /// stay paired, every relate and bound names a declared prop, and
    /// the spawn names a declared prop. Runs after compile, before the
    /// first placement solve; a broken compile refuses here and places
    /// nothing.
    pub fn validate(&self) -> Result<(), LevelError> {
        if self.relates.len() != self.bounds.len() {
            return Err(LevelError::BadLine {
                line: 0,
                reason: format!(
                    "compiled {} relates against {} bounds; refusing before solve",
                    self.relates.len(),
                    self.bounds.len()
                ),
            });
        }
        let declared: BTreeSet<u32> = self.declares.iter().map(|declare| declare.id).collect();
        for relate in &self.relates {
            if !declared.contains(&relate.prop) {
                return Err(LevelError::BadLine {
                    line: 0,
                    reason: format!(
                        "relate names undeclared prop {}; refusing before solve",
                        relate.prop
                    ),
                });
            }
        }
        for bound in &self.bounds {
            if !declared.contains(&bound.prop) {
                return Err(LevelError::BadLine {
                    line: 0,
                    reason: format!(
                        "bound names undeclared prop {}; refusing before solve",
                        bound.prop
                    ),
                });
            }
        }
        if !declared.contains(&self.spawn_id) {
            return Err(LevelError::BadLine {
                line: 0,
                reason: format!(
                    "spawn id {} names no declared prop; refusing before solve",
                    self.spawn_id
                ),
            });
        }
        Ok(())
    }
}

/// Room half-extent in cm for point clamps (the 10 m square shell).
pub(super) const ROOM_HALF_CM: i32 = 500;

pub(super) fn target_rank(target: &Target) -> u8 {
    match target {
        Target::Wall(_) => 0,
        Target::Point { .. } => 1,
        Target::Object(_) => 2,
    }
}

pub(super) fn clamp_room(value: i32) -> i32 {
    value.clamp(-ROOM_HALF_CM, ROOM_HALF_CM)
}

/// One compiled want entry: relation, resolved target, optional range
/// in cm, and the source brief line conflicts echo.
pub(super) type WantEntry = (Relation, Target, Option<(i32, i32)>, String);
/// Dedup key for one want entry: the entry without its source line.
pub(super) type WantKey = (Relation, Target, Option<(i32, i32)>);

pub(super) struct PropDraft {
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) id: u32,
    pub(super) entries: Vec<WantEntry>,
}

pub(super) fn draft_entries(
    parsed: &ParsedBrief,
    name: &str,
    index_of: &BTreeMap<&str, u32>,
) -> Result<Vec<WantEntry>, LevelError> {
    let raw_wants: &[RawWant] = parsed.wants.get(name).map_or(&[], Vec::as_slice);
    if raw_wants.len() > WANTS_MAX {
        return Err(LevelError::BadLine {
            line: 0,
            reason: format!(
                "prop `{name}` holds {} wants; at most {WANTS_MAX}",
                raw_wants.len()
            ),
        });
    }
    let mut out = Vec::with_capacity(raw_wants.len());
    for raw in raw_wants {
        let target = match &raw.target {
            RawTarget::Wall(wall) => Target::Wall(*wall),
            RawTarget::Point(x, z) => Target::Point { x_cm: *x, z_cm: *z },
            RawTarget::Object(other) => {
                let Some(id) = index_of.get(other.as_str()).copied() else {
                    return Err(LevelError::UnknownName {
                        name: other.clone(),
                    });
                };
                Target::Object(id)
            }
        };
        out.push((raw.relation, target, raw.range, raw.source.clone()));
    }
    Ok(out)
}

/// Greedy compile repair over one prop's entries: dedup exact repeats
/// (normalization, uncounted), then bounded self-target drop and room
/// clamp (counted, at most what is left of `MAX_REPAIRS`). Order is
/// preserved here; the caller sorts after repair.
pub(super) fn repair_entries(
    prop: &str,
    prop_id: u32,
    entries: Vec<WantEntry>,
    repairs: &mut u8,
    notes: &mut Vec<String>,
) -> Vec<WantEntry> {
    let mut seen: Vec<WantKey> = Vec::new();
    let mut unique = Vec::with_capacity(entries.len());
    for entry in entries {
        let (relation, target, range, source) = &entry;
        let key = (*relation, *target, *range);
        if seen.contains(&key) {
            notes.push(format!("dedup {prop} {source}"));
        } else {
            seen.push(key);
            unique.push(entry);
        }
    }
    let mut out = Vec::with_capacity(unique.len());
    for (relation, target, range, source) in unique {
        if matches!(target, Target::Object(id) if id == prop_id) && *repairs < MAX_REPAIRS {
            *repairs += 1;
            notes.push(format!("drop-self {prop} {source}"));
            continue;
        }
        if let Target::Point { x_cm, z_cm } = target {
            let (cx, cz) = (clamp_room(x_cm), clamp_room(z_cm));
            if (cx, cz) != (x_cm, z_cm) && *repairs < MAX_REPAIRS {
                *repairs += 1;
                notes.push(format!("clamp-point {prop} [{x_cm},{z_cm}]"));
                out.push((
                    relation,
                    Target::Point { x_cm: cx, z_cm: cz },
                    range,
                    source,
                ));
                continue;
            }
        }
        out.push((relation, target, range, source));
    }
    out
}

/// Stable want order per prop: walls, points, objects, facing last.
/// Line order survives inside each rank, so reordered brief lines still
/// compile identically and the solver sees the cheapest constraints
/// first. Pure order, never a counted repair.
pub(super) fn order_entries(entries: Vec<WantEntry>) -> Vec<WantEntry> {
    let mut ranked: Vec<(u8, u8, usize)> = entries
        .iter()
        .enumerate()
        .map(|(index, (relation, target, _, _))| {
            let facing = u8::from(*relation == Relation::Facing);
            (facing, target_rank(target), index)
        })
        .collect();
    // Index keys are unique, so the order is total and the unstable
    // sort repeats byte-identically on every machine.
    ranked.sort_unstable();
    ranked
        .into_iter()
        .map(|(_, _, index)| entries[index].clone())
        .collect()
}

/// Compile the envelope text into typed per-category constraints.
/// Grammar failures refuse exactly like `plan_brief` did; no actor is
/// placed here and no provider is consulted.
pub fn compile_brief(env: &BriefEnvelope) -> Result<CompiledBrief, LevelError> {
    let parsed = parse_brief(env.text())?;
    let mut names: Vec<&String> = parsed.kinds.keys().collect();
    names.sort();
    let index_of: BTreeMap<&str, u32> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), u32::try_from(index + 1).unwrap_or(u32::MAX)))
        .collect();
    let mut drafts = Vec::with_capacity(names.len());
    for name in &names {
        let id = index_of[name.as_str()];
        if id == u32::MAX {
            return Err(LevelError::TooManyProps { count: names.len() });
        }
        let entries = draft_entries(&parsed, name, &index_of)?;
        drafts.push(PropDraft {
            name: (*name).clone(),
            kind: parsed.kinds[*name].clone(),
            id,
            entries,
        });
    }
    let spawn_name: &str = match &parsed.spawn {
        Some(name) => name,
        None => names[0],
    };
    let spawn_id = index_of[spawn_name];
    let mut repairs = 0u8;
    let mut notes = Vec::new();
    let mut declares = Vec::with_capacity(drafts.len());
    let mut relates = Vec::new();
    let mut bounds = Vec::new();
    for draft in &drafts {
        declares.push(DeclareConstraint {
            name: draft.name.clone(),
            kind: draft.kind.clone(),
            id: draft.id,
        });
    }
    for draft in drafts {
        let fixed = repair_entries(
            &draft.name,
            draft.id,
            draft.entries,
            &mut repairs,
            &mut notes,
        );
        let ordered = order_entries(fixed);
        for (want_idx, (relation, target, range, source)) in ordered.into_iter().enumerate() {
            let (lo, hi, defaulted) = match range {
                Some((lo, hi)) => (lo, hi, false),
                None => (0, 0, true),
            };
            relates.push(RelateConstraint {
                prop: draft.id,
                relation,
                target,
                source,
            });
            bounds.push(BoundConstraint {
                prop: draft.id,
                want_idx,
                lo,
                hi,
                defaulted,
            });
        }
    }
    Ok(CompiledBrief {
        declares,
        relates,
        bounds,
        spawn_id,
        compile_repairs: repairs,
        notes,
    })
}

/// Lower compiled constraints to the solver plan. Pure reshape:
/// props in id order, wants in compiled order, defaulted bounds become
/// `None` so the solver applies its per-target defaults.
pub fn plan_from_compiled(compiled: &CompiledBrief) -> RelationPlan {
    let kinds: BTreeMap<u32, (&str, &str)> = compiled
        .declares
        .iter()
        .map(|declare| (declare.id, (declare.name.as_str(), declare.kind.as_str())))
        .collect();
    let mut ids: Vec<u32> = kinds.keys().copied().collect();
    ids.sort_unstable();
    let mut props = Vec::with_capacity(ids.len());
    for id in ids {
        let (name, kind) = kinds[&id];
        let mut wants = Vec::new();
        let mut sources = Vec::new();
        for (relate, bound) in compiled.relates.iter().zip(compiled.bounds.iter()) {
            if relate.prop != id {
                continue;
            }
            wants.push(Want {
                relation: relate.relation,
                target: relate.target,
                distance_cm: if bound.defaulted {
                    None
                } else {
                    Some((bound.lo, bound.hi))
                },
            });
            sources.push(relate.source.clone());
        }
        props.push(PlannedProp {
            name: name.to_owned(),
            kind: kind.to_owned(),
            id,
            wants,
            sources,
        });
    }
    RelationPlan {
        props,
        spawn_id: compiled.spawn_id,
    }
}

/// Parse the envelope text into a relation plan via the constraint-first
/// compile. Grammar failures refuse with the line and the legal values;
/// the compiled form validates before lowering, so a broken compile
/// refuses before the first solve. No actor is placed here.
pub fn plan_brief(env: &BriefEnvelope) -> Result<RelationPlan, LevelError> {
    let compiled = compile_brief(env)?;
    compiled.validate()?;
    Ok(plan_from_compiled(&compiled))
}
