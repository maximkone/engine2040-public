use super::*;

/// Sorted-id snapshot of the live document as fixed solver actors: every
/// node with a finite world position becomes one unit cube, like the
/// `scene.place` mapping. Live actors constrain the solve but are never
/// moved and never culprits.
pub(super) fn snapshot_live(doc: &engine_editor::commands::Document) -> Vec<Actor> {
    let mut order: Vec<ObjectId> = Vec::new();
    let mut stack = doc.hierarchy.roots();
    while let Some(id) = stack.pop() {
        order.push(id);
        stack.extend(doc.hierarchy.children(id));
    }
    order.sort_by_key(|id| id.to_string());
    let mut actors = Vec::new();
    let mut next_id = LIVE_BASE;
    for id in order {
        if next_id >= SHELL_BASE {
            break;
        }
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
        actors.push(Actor {
            id: next_id,
            name: format!("live_{next_id:06}"),
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
        next_id += 1;
    }
    actors
}

pub(super) fn bad_brief(error: LevelError) -> McpToolError {
    McpToolError::new("bad_brief", error.to_string())
}

/// Door args for `level.build`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BuildLevelArgs {
    /// Brief text in the `prop`/`want`/`spawn` grammar.
    pub brief: String,
    /// Build seed naming this attempt.
    pub seed: u64,
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Retry identity for the landed batch.
    pub idempotency_key: u64,
}

/// One landed prop with its live document id.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlacedDto {
    /// Live document id (UUID text).
    pub id: String,
    /// Prop name from the brief.
    pub name: String,
    /// Position meters.
    pub x: f32,
    /// Grounded height meters.
    pub y: f32,
    /// Position meters.
    pub z: f32,
    /// Facing yaw radians.
    pub yaw: f32,
}

/// Door output for `level.build`: solved placements plus the AB4 input
/// line, or the refusal with conflicts and findings. Undo rides the
/// existing `tx.undo` tool with `undo_token`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BuildLevelOutput {
    /// Document revision after the call (unchanged on refusal).
    pub revision: u64,
    /// True when the level placed.
    pub solved: bool,
    /// Landed props with live ids.
    pub placements: Vec<PlacedDto>,
    /// Gate finding lines (empty on a pass).
    pub findings: Vec<String>,
    /// Culprit want source lines on a refusal.
    pub conflicts: Vec<String>,
    /// Why the build refused, empty on a pass.
    pub reason: String,
    /// Repair rounds consumed.
    pub repairs_used: u8,
    /// `"pass"` or `"refused"`.
    pub verdict: String,
    /// AB4 input line on a pass, empty on a refusal.
    pub next_ab4: String,
    /// Simulated cost units.
    pub cost_units: u64,
    /// Landed batch key, zero on a refusal.
    pub key: u64,
    /// Stable undo identity for `tx.undo`, zero on a refusal.
    pub undo_token: u64,
    /// Shared LW6 envelope record line (`BriefEnvelope::record_line`).
    /// Every door write carries it so the audit traces end to end.
    #[serde(default)]
    #[schemars(default)]
    pub record_line: String,
    /// Relation changes this edit made (ARC12); empty on a refusal.
    #[serde(default)]
    #[schemars(default)]
    pub relation_diff: Vec<String>,
}

/// Run the level chain against the live document through the relation-diff
/// edit path (ARC12): plan the brief into relation requests, solve with SP5
/// against the live snapshot, then land the edit's spine writes as one
/// gated batch. Denied, stale and refused calls write nothing; a refusal
/// answers with the SP5 conflict set beside the gate findings. The raw
/// coordinate table stays the ablation arm (`ablation_numbers`), never the
/// payload the caller reads.
pub(super) fn build_level_on_state(
    state: &mut ServerState,
    args: &BuildLevelArgs,
) -> Result<BuildLevelOutput, McpToolError> {
    if args.expected_revision != state.doc.revision {
        return Err(McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {}, intent expects {}; \
                     re-read scene.brief, then build again",
                state.doc.revision, args.expected_revision
            ),
        ));
    }
    let env = envelope(&args.brief, args.seed).map_err(bad_brief)?;
    let live = snapshot_live(&state.doc);
    match relation_edit_on(&env, &live).map_err(bad_brief)? {
        EditOutcome::Refused(refusal) => {
            // Conflicts keep the culprit wants first, then add the SP5
            // minimal subset the caller must fix (deduplicated).
            let mut conflicts = refusal.conflicts;
            for line in refusal.conflict_set {
                if !conflicts.contains(&line) {
                    conflicts.push(line);
                }
            }
            Ok(BuildLevelOutput {
                revision: state.doc.revision,
                solved: false,
                placements: Vec::new(),
                findings: refusal.findings,
                conflicts,
                reason: refusal.reason,
                repairs_used: refusal.repairs_used,
                verdict: "refused".to_owned(),
                next_ab4: String::new(),
                cost_units: 0,
                key: 0,
                undo_token: 0,
                record_line: env.record_line(),
                relation_diff: Vec::new(),
            })
        }
        EditOutcome::Edited(edit) => {
            let mut ids = Vec::with_capacity(edit.writes.len());
            let mut ops = Vec::with_capacity(edit.writes.len());
            for write in &edit.writes {
                let id = ObjectId::new();
                ids.push((id, write.name.clone()));
                ops.push(Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new(write.pos, quat_of_yaw(write.yaw), write.scale),
                });
            }
            let batch = Batch {
                ops,
                base_revision: state.doc.revision,
                idempotency_key: args.idempotency_key,
                provenance: Some(ProvenanceStamp::script("level.build")),
            };
            let _preview =
                commands::validate(&state.doc, &batch).map_err(|error| batch_error(&error))?;
            let approval = approve(
                state.session,
                &batch,
                state.doc.revision,
                state.doc.revision,
            );
            let granted = granted_ops(&state.cap);
            let applied = capability::apply_gated(
                &mut state.doc,
                &mut state.journal,
                &batch,
                &mut state.cap,
                &approval,
                &mut state.ledger,
            )
            .map_err(|error| cap_error(&error, &granted))?;
            // A landed level is a watch event: move the session cache.
            state.refresh_watch();
            let placements: Vec<PlacedDto> = ids
                .iter()
                .zip(edit.report.placements.iter())
                .map(|((id, name), placement)| PlacedDto {
                    id: id.to_string(),
                    name: name.clone(),
                    x: placement.x,
                    y: placement.y,
                    z: placement.z,
                    yaw: placement.yaw,
                })
                .collect();
            Ok(BuildLevelOutput {
                revision: applied.revision,
                solved: true,
                placements,
                findings: Vec::new(),
                conflicts: Vec::new(),
                reason: String::new(),
                repairs_used: edit.report.repairs_used,
                verdict: "pass".to_owned(),
                next_ab4: edit.report.next_ab4.clone(),
                cost_units: edit.report.cost.units,
                key: applied.key,
                undo_token: applied.key,
                record_line: env.record_line(),
                relation_diff: edit.changes,
            })
        }
    }
}

/// `level.build` door tool: brief to placed level through the command
/// gate, dispatched like every sibling so the taxonomy fronts the name
/// and the audit seals one record per call. Undo rides `tx.undo`.
pub struct BuildLevelTool;

impl ToolBase for BuildLevelTool {
    type Parameter = BuildLevelArgs;
    type Output = BuildLevelOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_BUILD_LEVEL.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Build a level from a text brief: relation plan, SP5 placement and cell \
                 layout with nav checks, applied through the command gate; conflicts and \
                 findings on refusal."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::write_annotations(true)
    }
}

impl SyncTool<McpServer> for BuildLevelTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_BUILD_LEVEL, |state| {
            build_level_on_state(state, &param)
        })
    }
}
