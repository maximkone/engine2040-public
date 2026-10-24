use super::*;
use engine_editor::capability::Capability;
use rmcp::handler::server::router::tool::SyncTool;
use rmcp::serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

const CONDITION: &str = "tencent-hunyuan-community: eu-excluded";

fn server() -> McpServer {
    McpServer::new(71, Capability::unrestricted(71))
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "asset_intake_{tag}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp folder");
    dir
}

/// A bumpy grid with 2 * n * n triangles, as a GLB with positions and indices only.
fn grid_glb(n: u32) -> Vec<u8> {
    let side = n + 1;
    let mut pos: Vec<[f32; 3]> = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let (fx, fy) = (
                f32::from(u16::try_from(x).unwrap_or(0)),
                f32::from(u16::try_from(y).unwrap_or(0)),
            );
            let bump = ((fx * 0.37).fract() + (fy * 0.21).fract()) * 0.2;
            pos.push([fx * 0.02, fy * 0.02, bump]);
        }
    }
    let mut idx: Vec<u32> = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let a = y * side + x;
            idx.extend_from_slice(&[a, a + 1, a + side, a + 1, a + side + 1, a + side]);
        }
    }
    let mut bin = Vec::new();
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in &pos {
        for k in 0..3 {
            bin.extend_from_slice(&p[k].to_le_bytes());
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let pos_len = bin.len();
    for i in &idx {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let idx_len = bin.len() - pos_len;
    let json = format!(
        "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0}}],\
\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1}}]}}],\
\"buffers\":[{{\"byteLength\":{}}}],\
\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{pos_len}}},{{\"buffer\":0,\"byteOffset\":{pos_len},\"byteLength\":{idx_len}}}],\
\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{:?},{:?},{:?}],\"max\":[{:?},{:?},{:?}]}},\
{{\"bufferView\":1,\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}]}}",
        bin.len(), pos.len(), lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], idx.len()
    );
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let word = |v: usize| u32::try_from(v).unwrap_or(0).to_le_bytes();
    let mut out = b"glTF".to_vec();
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&word(total));
    out.extend_from_slice(&word(json.len()));
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&word(bin.len()));
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}

/// A runner folder: one GLB plus the `kaggle-lane.json` the center job writes.
fn runner_folder(
    tag: &str,
    triposg_license: &str,
    extra_component: Option<(&str, &str)>,
) -> (PathBuf, PathBuf, PathBuf) {
    let dir = folder(tag);
    let glb = dir.join("mesh-01-barrel.glb");
    std::fs::write(&glb, grid_glb(60)).expect("glb");
    let mut models = vec![
        json!({"key": "birefnet", "id": "ZhengPeng7/BiRefNet", "license": "MIT", "commercial": true, "conditions": []}),
        json!({"key": "triposg", "id": "VAST-AI/TripoSG", "license": triposg_license, "commercial": true, "conditions": [CONDITION]}),
        json!({"key": "dinov2-large", "id": "facebook/dinov2-large", "license": "Apache-2.0", "commercial": true, "conditions": []}),
    ];
    if let Some((id, license)) = extra_component {
        models.push(json!({"key": "extra", "id": id, "license": license, "commercial": false, "conditions": []}));
    }
    let lane = json!({
        "tool": "center/kaggle.mjs", "job": "mesh", "models": models,
        "items": [{
            "file": "mesh-01-barrel.glb", "model": "triposg", "seed": 4243,
            "source_sha256": "9f2c41d0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c",
            "code_commit": "fc5c40990181e2a756c4e0b1c2f4d6b5202faf8c", "conditions": [CONDITION], "faces": 60000
        }]
    });
    let lane_path = dir.join("kaggle-lane.json");
    std::fs::write(&lane_path, lane.to_string()).expect("lane json");
    (dir, glb, lane_path)
}

fn args(dir: &Path, glb: &Path, lane: &Path, accept: &[&str]) -> IntakeArgs {
    IntakeArgs {
        name: "barrel".to_owned(),
        glb_path: glb.display().to_string(),
        lane_json: Some(lane.display().to_string()),
        accept_conditions: accept.iter().map(|s| (*s).to_owned()).collect(),
        out_dir: Some(dir.join("out").display().to_string()),
        ..IntakeArgs::default()
    }
}

#[test]
fn the_door_cooks_a_generated_glb_and_the_cache_returns_the_same_hash() {
    let (dir, glb, lane) = runner_folder("cook", "MIT", None);
    let door = server();
    // The declared Tencent condition must be accepted by name first.
    let refused = IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[])).expect_err("conditions");
    assert_eq!(refused.code, "conditions_unaccepted");
    assert!(
        refused.message.contains(CONDITION) && refused.message.contains("Next:"),
        "{}",
        refused.message
    );
    assert!(
        !dir.join("out").join("barrel.e204bndl").exists(),
        "a refusal writes no bundle"
    );

    let first = IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION])).expect("intake");
    assert!(!first.cache_hit);
    assert_eq!(first.tris_in, 7200);
    assert!(
        first.tris <= 5000 && first.tris > 500,
        "tris {}",
        first.tris
    );
    assert!(first.hull.radius > 0.0 && !first.hull.support.is_empty());
    assert_eq!(first.provenance["model"]["id"], "VAST-AI/TripoSG");
    assert_eq!(first.provenance["model"]["license"], "MIT");
    assert_eq!(first.provenance["seed"], 4243);
    assert_eq!(
        first.provenance["source_digest"],
        "9f2c41d0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c"
    );
    assert!(first.provenance["components"]
        .as_array()
        .is_some_and(|c| c.len() == 2));
    let bytes = std::fs::read(&first.bundle_path).expect("bundle written");
    assert_eq!(engine_scene::ids::content_digest(&bytes), first.bundle_hash);
    let row = std::fs::read_to_string(&first.row_path).expect("row written");
    assert!(row.contains(&first.bundle_hash));

    let again = IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION])).expect("cache");
    assert!(again.cache_hit);
    assert_eq!(again.bundle_hash, first.bundle_hash);
    assert_eq!(again.hull, first.hull);

    // Cook x2 from scratch: a fresh output folder gives the same bundle bytes.
    let mut fresh = args(&dir, &glb, &lane, &[CONDITION]);
    fresh.out_dir = Some(dir.join("out2").display().to_string());
    let second = IntakeTool::invoke(&door, fresh).expect("fresh cook");
    assert!(!second.cache_hit);
    assert_eq!(
        second.bundle_hash, first.bundle_hash,
        "cook x2 byte-identical"
    );
    assert_eq!(std::fs::read(&second.bundle_path).expect("bundle 2"), bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_commercial_models_and_parts_are_refused_and_write_nothing() {
    let door = server();
    let (dir, glb, lane) = runner_folder("nc-model", "CC-BY-NC-4.0", None);
    let err =
        IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION])).expect_err("nc licence");
    assert_eq!(err.code, "license_refused");
    assert!(err.message.contains("non-commercial"), "{}", err.message);
    assert!(!dir.join("out").join("barrel.e204bndl").exists());
    let _ = std::fs::remove_dir_all(&dir);

    let (dir, glb, lane) = runner_folder("rmbg", "MIT", Some(("briaai/RMBG-1.4", "MIT")));
    let err =
        IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION])).expect_err("rmbg part");
    assert_eq!(err.code, "model_restricted");
    assert!(!dir.join("out").join("barrel.e204bndl").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The retry answers a scripted client sends after the licence ask.
fn licence_answer(licence: &str) -> rmcp::model::InputResponses {
    rmcp::serde_json::from_value(json!({
        mcp_input::LICENCE_REQUEST_ID: {"action": "accept", "content": {"licence": licence}}
    }))
    .expect("answers decode")
}

#[test]
fn input_required_an_unknown_licence_asks_before_it_refuses() {
    let door = server();
    let (dir, glb, lane) = runner_folder("ask", "see the card!", None);
    let asked =
        IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION])).expect_err("an ask");
    assert_eq!(asked.code, mcp_input::CODE);
    assert!(
        !dir.join("out").join("barrel.e204bndl").exists(),
        "an ask writes no bundle"
    );
    let response = mcp_input::into_response(&asked).expect("the ask is a typed result");
    let rmcp::model::CallToolResponse::InputRequired(result) = response else {
        panic!("an unknown licence answers input_required");
    };
    assert_eq!(
        result
            .input_requests
            .as_ref()
            .map(rmcp::model::InputRequests::len),
        Some(1),
        "one request, the licence"
    );
    let log = door.audit_log().expect("the door audit reads");
    assert_eq!(log.len(), 1, "the ask seals one record");
    let entries: Vec<&crate::audit::LedgerEntry> = log.entries().collect();
    assert_eq!(entries.len(), 1, "one call, one ledger entry");
    let entry = entries[0];
    assert_eq!(entry.rounds, 1, "an unanswered ask is still one round");
    assert_eq!(entry.outcome, crate::audit::LedgerOutcome::InputRequired);
    assert_eq!(
        entry.request_state.as_deref(),
        Some(mcp_input::REQUEST_STATE),
        "the entry is keyed by the state a retry echoes"
    );
    assert!(entry.answer.is_none(), "nothing was answered yet");
    log.verify().expect("the ask chain verifies");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn input_required_a_scripted_retry_with_the_licence_lands_the_cook() {
    let door = server();
    let (dir, glb, lane) = runner_folder("retry", "unknown", None);
    let a = args(&dir, &glb, &lane, &[CONDITION]);
    let asked = IntakeTool::invoke(&door, a.clone()).expect_err("an ask");
    assert_eq!(asked.code, mcp_input::CODE);

    // The retry the door would receive: the asked state echoed back plus the
    // licence answer. Both ride the call; the ledger folds it into the ask.
    door.set_retry_answers(licence_answer("MIT"));
    door.set_round(Some(crate::audit::RoundNote {
        request_state: mcp_input::REQUEST_STATE.to_owned(),
        answer: Some("MIT".to_owned()),
    }));
    let landed = IntakeTool::invoke(&door, a);
    door.set_round(None);
    door.clear_retry_answers();
    let landed = landed.expect("the retry carrying the licence lands the cook");
    assert_eq!(landed.provenance["model"]["license"], "MIT");
    assert_eq!(landed.provenance["model"]["id"], "VAST-AI/TripoSG");
    let bytes = std::fs::read(&landed.bundle_path).expect("bundle written");
    assert_eq!(
        engine_scene::ids::content_digest(&bytes),
        landed.bundle_hash
    );

    // One call in two rounds: one ledger entry, the ask folded with the retry.
    let log = door.audit_log().expect("the door audit reads");
    assert_eq!(log.len(), 2, "two sealed records, one ask and one retry");
    let entries: Vec<&crate::audit::LedgerEntry> = log.entries().collect();
    assert_eq!(entries.len(), 1, "one call, one ledger entry");
    let entry = entries[0];
    assert_eq!(entry.rounds, 2, "the ask, then the landed retry");
    assert_eq!(
        entry.outcome,
        crate::audit::LedgerOutcome::Ok,
        "the outcome is the last round's"
    );
    assert_eq!(entry.seq, 2, "the entry points at the retry's record");
    assert_eq!(
        entry.request_state.as_deref(),
        Some(mcp_input::REQUEST_STATE)
    );
    assert_eq!(
        entry.answer.as_deref(),
        Some("MIT"),
        "the licence answer is recorded on the second round"
    );
    assert_eq!(entry.row(), "2 2 ok MIT asset.intake:licence");
    log.verify().expect("the two-round chain still verifies");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn input_required_a_compliant_licence_never_asks() {
    let door = server();
    let (dir, glb, lane) = runner_folder("compliant", "MIT", None);
    let first = IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION]))
        .expect("a compliant licence cooks on the first call");
    assert_eq!(first.provenance["model"]["license"], "MIT");
    assert_eq!(
        door.audit_log().expect("the door audit reads").len(),
        1,
        "one round, no ask"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// One plain intake record's canonical entry bytes, pinned off HEAD's
/// `entry_bytes` (session 71, `asset.intake`, revision 0, Ok, detail 0, seq 1).
/// The ledger layer folds rounds beside the chain, so these bytes must not
/// move: a plain call reads exactly as it did before MCP31.
const PLAIN_ENTRY_HEX: &str = "47000000000000000c0000000000000061737365742e696e74616b65\
0000000000000000000000000000000000000100000000000000";
/// The chain digest that record seals: `digest(zeros || entry || seq 1)`.
const PLAIN_CHAIN_HEX: &str = "20dfe2853cc12a9f0016a2386ac007a58f4a3e8aa60775bbe1d2327c864c4b70";

/// Read a 32-byte chain digest from its lowercase hex.
fn chain_from_hex(text: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(pair).expect("hex pair");
        *slot = u8::from_str_radix(text, 16).expect("hex pair");
    }
    out
}

#[test]
fn a_compliant_asset_intake_keeps_its_one_round_record_byte_equal() {
    let row = |tag: &str| {
        let door = server();
        let (dir, glb, lane) = runner_folder(tag, "MIT", None);
        IntakeTool::invoke(&door, args(&dir, &glb, &lane, &[CONDITION])).expect("a plain cook");
        let log = door.audit_log().expect("the door audit reads");
        let record = log.iter().next().expect("one sealed record");
        let assert_plain = (
            record.session,
            record.tool.clone(),
            record.revision,
            record.seq,
        );
        assert_eq!(
            assert_plain,
            (71, "asset.intake".to_owned(), 0, 1),
            "the plain call's record identity"
        );
        assert_eq!(
            record.canonical_hex(),
            PLAIN_ENTRY_HEX,
            "a plain record's canonical bytes are unchanged"
        );
        assert_eq!(
            record.chain,
            chain_from_hex(PLAIN_CHAIN_HEX),
            "a plain record chains to the pinned digest"
        );
        let entries: Vec<crate::audit::LedgerEntry> = log.entries().cloned().collect();
        assert_eq!(entries.len(), 1, "one plain call, one entry");
        let entry = &entries[0];
        assert_eq!(entry.rounds, 1, "a plain call stays one round");
        assert_eq!(entry.outcome, crate::audit::LedgerOutcome::Ok);
        assert!(
            entry.request_state.is_none() && entry.answer.is_none(),
            "a plain call asked nothing and answered nothing"
        );
        log.verify().expect("the plain chain verifies");
        let _ = std::fs::remove_dir_all(&dir);
        entry.row()
    };
    // Same call twice: the same ledger row bytes, on two fresh doors.
    assert_eq!(row("byte-a"), row("byte-b"), "same bytes twice");
    assert_eq!(row("byte-c"), "1 1 ok - -", "the pinned plain row");
}

#[test]
fn explicit_facts_work_without_a_lane_file_and_bad_input_names_the_argument() {
    let door = server();
    let (dir, glb, _lane) = runner_folder("explicit", "MIT", None);
    let mut a = IntakeArgs {
        name: "crate-01".to_owned(),
        glb_path: glb.display().to_string(),
        model: Some(ModelArg {
            id: "VAST-AI/TripoSG".to_owned(),
            license: "MIT".to_owned(),
        }),
        prompt: Some("a wooden crate".to_owned()),
        seed: Some(7),
        out_dir: Some(dir.join("out").display().to_string()),
        up_axis: Some("z".to_owned()),
        size_mm: Some(1500),
        ..IntakeArgs::default()
    };
    let out = IntakeTool::invoke(&door, a.clone()).expect("explicit facts");
    assert_eq!(
        out.provenance["source_digest"],
        digest_text("a wooden crate")
    );
    assert_eq!(out.provenance["settings"]["up"], "z");
    a.up_axis = Some("w".to_owned());
    assert_eq!(
        IntakeTool::invoke(&door, a.clone()).expect_err("axis").code,
        "bad_input"
    );
    a.up_axis = None;
    a.model = None;
    assert_eq!(
        IntakeTool::invoke(&door, a.clone())
            .expect_err("model")
            .code,
        "bad_input"
    );
    a.glb_path = "model.fbx".to_owned();
    assert_eq!(
        IntakeTool::invoke(&door, a).expect_err("ext").code,
        "bad_input"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_pack_carries_both_verbs_with_the_right_cards() {
    let pack = super::super::pack().expect("pack");
    let names = pack.verb_names();
    assert!(names.contains(&"asset.intake") && names.contains(&"asset.generate"));
    for verb in pack.verbs() {
        if verb.name() == "asset.intake" {
            assert_eq!(
                verb.card(),
                crate::tool_capabilities::CapabilityCard::Commit
            );
        }
    }
}

#[test]
#[ignore = "needs the real generated props: set E2040_MAKE_RUN to the folder that `kaggle.mjs run mesh` wrote"]
fn real_generated_props_cook_through_the_door() {
    let run = PathBuf::from(
        std::env::var("E2040_MAKE_RUN").expect("E2040_MAKE_RUN names the runner folder"),
    );
    let lane = run.join("kaggle-lane.json");
    for fresh in ["intake", "intake-x2"] {
        let _ = std::fs::remove_dir_all(run.join(fresh));
    }
    let mut cooked = 0;
    for entry in std::fs::read_dir(&run).expect("runner folder") {
        let path = entry.expect("entry").path();
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if !file.starts_with("mesh-") || !file.ends_with(".glb") {
            continue;
        }
        let name = format!("prop-{}", &file[5..7]);
        let mk = |out: &str| IntakeArgs {
            name: name.clone(),
            glb_path: path.display().to_string(),
            lane_json: Some(lane.display().to_string()),
            accept_conditions: vec![CONDITION.to_owned()],
            out_dir: Some(run.join(out).display().to_string()),
            ..IntakeArgs::default()
        };
        let first = IntakeTool::invoke(&server(), mk("intake")).expect("first cook");
        let second = IntakeTool::invoke(&server(), mk("intake")).expect("cache");
        let again =
            IntakeTool::invoke(&server(), mk("intake-x2")).expect("second cook from scratch");
        assert!(
            first.tris <= 5000 && first.tris > 500,
            "{file}: {} tris",
            first.tris
        );
        assert!(first.hull.radius > 0.0 && !first.hull.support.is_empty());
        assert_eq!(first.provenance["model"]["license"], "MIT");
        assert!(second.cache_hit && second.bundle_hash == first.bundle_hash);
        assert!(
            !again.cache_hit && again.bundle_hash == first.bundle_hash,
            "cook x2 byte-identical"
        );
        assert_eq!(
            std::fs::read(&first.bundle_path).expect("bundle"),
            std::fs::read(&again.bundle_path).expect("bundle x2")
        );
        println!(
            "{file}: tris_in {} -> {} (lods {:?}), hull radius {:.3} m with {} support points, bundle {}, specks {}",
            first.tris_in, first.tris, first.lod_tris, first.hull.radius, first.hull.support.len(), first.bundle_hash,
            first.provenance["repair"]["specks"]
        );
        cooked += 1;
    }
    assert!(cooked >= 2, "cooked {cooked} props; want at least 2");
}
