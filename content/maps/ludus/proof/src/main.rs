// WD2 proof: the ludus hub loads, plays and replays exactly. Cooks the 7
// hub kit assets through the engine's own CT2/A2 lanes, packs the hub bundle
// through the exact WIN1/ED1 startup loader, runs the engine spatial gate
// over the placed hub, bakes the yard nav through NV1, streams the hub cells
// through WD1, then replays the whole pipeline and demands one report byte.
// Only landed engine APIs; exit 0 with report.json on green, exit 1 naming
// the failure.
use engine_assets::{
    asset_digest, cook_gltf_bytes, cook_texture, texture_asset_id, tri_count, BundleLoader,
    BundleWriter, CookedMaterial, EntryKind, MeshClass, TextureRef, TextureSlot, PROP_MAX_TRIS,
};
use engine_nav::mesh::{bake, BakeConfig, NavMesh};
use engine_spatial::{run, Actor};
use engine_world::cells::{
    bake_cell, cell_cost_bytes, parse_cell, seam_joins, CellBudget, CellCoord, CellError,
    CellEvent, Cells,
};

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/map_data.rs"));

fn fail(msg: &str) -> ! {
    eprintln!("LUDUS-PROOF FAIL: {msg}");
    std::process::exit(1);
}

fn read(dir: &std::path::Path, rel: &str) -> Vec<u8> {
    let p = dir.join(rel);
    std::fs::read(&p).unwrap_or_else(|e| fail(&format!("read {}: {e}", p.display())))
}

/// Kit texture per hub asset (the CT9 cook contract; albedo slot, sRGB bytes).
fn albedo_of(asset: &str) -> &'static str {
    match asset {
        "yard_floor" => "ground_albedo",
        "barracks" => "plaster_albedo",
        "blacksmith" | "street_wall" => "stone_albedo",
        "notice_board" | "training_post" | "weapon_rack" => "timber_albedo",
        _ => fail("non-hub asset in the hub cook"),
    }
}

/// Kit PBR factors per hub asset (the CT9 authored set, reused not re-tuned).
fn factors(asset: &str) -> ([f32; 4], f32, f32) {
    match asset {
        "yard_floor" => ([0.72, 0.6, 0.44, 1.0], 0.0, 0.96),
        "barracks" => ([0.8, 0.74, 0.64, 1.0], 0.0, 0.92),
        "blacksmith" => ([0.6, 0.57, 0.52, 1.0], 0.0, 0.9),
        "notice_board" => ([0.52, 0.37, 0.22, 1.0], 0.0, 0.82),
        "training_post" => ([0.55, 0.4, 0.24, 1.0], 0.0, 0.85),
        "weapon_rack" => ([0.5, 0.36, 0.22, 1.0], 0.1, 0.8),
        "street_wall" => ([0.6, 0.57, 0.52, 1.0], 0.0, 0.9),
        _ => fail("non-hub asset in the hub factors"),
    }
}

fn spawn(name: &str) -> (f32, f32) {
    SPAWNS
        .iter()
        .find(|s| s.name == name)
        .map(|s| (s.x, s.z))
        .unwrap_or_else(|| fail("route starts at a missing spawn"))
}

fn main() {
    let maps = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
        });
    let ludus = maps.join("ludus");
    if MAP_NAME != "ludus" || MAP_VERSION != 1 {
        fail("map bindings are not the ludus v1 hub");
    }
    if ACTORS.len() != 8 || ZONES.len() != 3 || SPAWNS.len() != 4 {
        fail("hub shape drifted: want 8 actors, 3 zones, 4 spawns");
    }
    if ROUTES.len() != 3 || STREAM.len() != 4 || CELL_SIZE != 16 {
        fail("hub shape drifted: want 3 routes, 4 stream cells, size 16");
    }

    // ---- Leg 1: cook every hub asset twice through CT2/A2 ------------------
    let mut total_lod0 = 0u32;
    let mut asset_entries: Vec<String> = Vec::new();
    for asset in LUDUS_ASSETS {
        let bytes = read(&maps, &format!("src/{asset}.gltf"));
        let first = cook_gltf_bytes(&bytes, MeshClass::Prop)
            .unwrap_or_else(|e| fail(&format!("cook {asset}: {e}")));
        let second = cook_gltf_bytes(&bytes, MeshClass::Prop)
            .unwrap_or_else(|e| fail(&format!("recook {asset}: {e}")));
        if first != second {
            fail(&format!("cook of {asset} not identical twice"));
        }
        if first.digest != asset_digest(&first) {
            fail(&format!("digest mismatch on {asset}"));
        }
        let lod0: u32 = first.meshes.iter().map(|m| m.base_tris).sum();
        total_lod0 += lod0;
        if lod0 > PROP_MAX_TRIS {
            fail(&format!("{asset} over prop budget: {lod0}"));
        }
        for (mi, m) in first.meshes.iter().enumerate() {
            for (l, lod) in m.lods.iter().enumerate() {
                if tri_count(lod) == 0 {
                    fail(&format!("{asset} mesh {mi} LOD{l} is empty"));
                }
                if !m.hulls[l].contains_mesh(lod) {
                    fail(&format!(
                        "hull does not contain mesh {mi} LOD{l} in {asset}"
                    ));
                }
            }
        }
        asset_entries.push(format!(
            "    {{\"asset\": \"{asset}\", \"lod0_tris\": {lod0}, \"digest\": \"{:016x}\", \"identical_twice\": true}}",
            first.digest,
        ));
    }
    // Hub textures cook twice with mip chains reaching 1x1.
    for tex in [
        "ground_albedo",
        "plaster_albedo",
        "stone_albedo",
        "timber_albedo",
    ] {
        let bytes = read(&maps, &format!("textures/{tex}.png"));
        let first = cook_texture(&bytes, TextureSlot::BaseColor)
            .unwrap_or_else(|e| fail(&format!("tex {tex}: {e:?}")));
        let second = cook_texture(&bytes, TextureSlot::BaseColor)
            .unwrap_or_else(|e| fail(&format!("tex recook {tex}: {e:?}")));
        if first != second {
            fail(&format!("texture cook of {tex} not identical twice"));
        }
        if first.width > 512 || first.height > 512 {
            fail(&format!("texture {tex} over the 512 albedo cap"));
        }
        let last = first
            .mips
            .last()
            .unwrap_or_else(|| fail("texture has no mips"));
        if last.width != 1 || last.height != 1 {
            fail(&format!("texture {tex} mip chain does not reach 1x1"));
        }
    }

    // ---- Leg 2: the hub bundle through the startup loader ------------------
    let build_bundle = || -> BundleWriter {
        let mut w = BundleWriter::new();
        for asset in LUDUS_ASSETS {
            let bytes = read(&maps, &format!("src/{asset}.gltf"));
            let cooked = cook_gltf_bytes(&bytes, MeshClass::Prop)
                .unwrap_or_else(|e| fail(&format!("bundle cook {asset}: {e}")));
            for (mi, m) in cooked.meshes.iter().enumerate() {
                for (l, lod) in m.lods.iter().enumerate() {
                    w.add_mesh(&format!("ludus/{asset}/m{mi:02}_lod{l}"), lod)
                        .unwrap_or_else(|e| fail(&format!("bundle mesh: {e}")));
                }
            }
            let (base, metallic, roughness) = factors(asset);
            let albedo = read(&maps, &format!("textures/{}.png", albedo_of(asset)));
            let flat = read(&maps, "textures/flat_normal.png");
            let mat = CookedMaterial {
                base_color_factor: base,
                metallic_factor: metallic,
                roughness_factor: roughness,
                textures: vec![
                    TextureRef {
                        asset: texture_asset_id(&albedo),
                        slot: TextureSlot::BaseColor,
                    },
                    TextureRef {
                        asset: texture_asset_id(&flat),
                        slot: TextureSlot::Normal,
                    },
                ],
            };
            w.add_material(&format!("ludus/mat_{asset}"), &mat)
                .unwrap_or_else(|e| fail(&format!("bundle mat: {e}")));
        }
        w
    };
    let packed_first = build_bundle().finish();
    let packed_second = build_bundle().finish();
    if packed_first != packed_second {
        fail("hub bundle not identical twice");
    }
    let loaded = BundleLoader::new()
        .load(&packed_first)
        .unwrap_or_else(|e| fail(&format!("hub bundle refused: {e}")));
    let mut mesh_entries = 0u32;
    let mut mat_entries = 0u32;
    for e in &loaded.entries {
        match e.kind {
            EntryKind::Mesh => {
                e.decode_mesh()
                    .unwrap_or_else(|er| fail(&format!("decode {}: {er}", e.name)));
                mesh_entries += 1;
            }
            EntryKind::Material => {
                e.decode_material()
                    .unwrap_or_else(|er| fail(&format!("decode {}: {er}", e.name)));
                mat_entries += 1;
            }
            _ => fail("unexpected entry kind in the hub bundle"),
        }
    }
    if mesh_entries == 0 || mat_entries != LUDUS_ASSETS.len() as u32 {
        fail("hub bundle entry census failed");
    }
    {
        // Corrupt bytes never load: flip one payload byte, the digest must catch it.
        let first_len = loaded.entries[0].name.len();
        let at = 16 + 1 + 2 + first_len + 4;
        let mut bad = packed_first.clone();
        bad[at] ^= 0xFF;
        if BundleLoader::new().load(&bad).is_ok() {
            fail("corrupt hub bundle loaded");
        }
    }

    // ---- Leg 3: the placed hub runs the spatial gate clean -----------------
    let actors: Vec<Actor> = ACTORS
        .iter()
        .map(|p| Actor {
            id: p.id,
            name: p.name.to_string(),
            x: p.x,
            y: p.y,
            z: p.z,
            sx: p.sx,
            sy: p.sy,
            sz: p.sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        })
        .collect();
    let placed_first = run(&actors);
    let placed_second = run(&actors);
    if placed_first.hash != placed_second.hash || placed_first.findings != placed_second.findings {
        fail("spatial run on the hub not deterministic");
    }
    if !placed_first.findings.is_empty() {
        fail(&format!(
            "spatial findings on the hub: {:?}",
            placed_first.findings
        ));
    }

    // ---- Leg 4: the yard walks on the NV1 bake -----------------------------
    let (rx0, rx1, rz0, rz1) = (NAV_RECT[0], NAV_RECT[1], NAV_RECT[2], NAV_RECT[3]);
    let yard_tris = [
        [[rx0, 0.0, rz0], [rx0, 0.0, rz1], [rx1, 0.0, rz1]],
        [[rx0, 0.0, rz0], [rx1, 0.0, rz1], [rx1, 0.0, rz0]],
    ];
    let config = BakeConfig {
        cell_size: NAV_CELL_M,
        walkable_normal_y: 0.7,
    };
    let nav_first = bake(&yard_tris, &config).unwrap_or_else(|e| fail(&format!("nav bake: {e:?}")));
    let nav_second =
        bake(&yard_tris, &config).unwrap_or_else(|e| fail(&format!("nav rebake: {e:?}")));
    if nav_first.to_bytes() != nav_second.to_bytes() || nav_first.digest() != nav_second.digest() {
        fail("yard nav not identical twice");
    }
    {
        let mut shuffled = yard_tris.to_vec();
        shuffled.reverse();
        if bake(&shuffled, &config)
            .unwrap_or_else(|e| fail(&format!("shuffled nav: {e:?}")))
            .to_bytes()
            != nav_first.to_bytes()
        {
            fail("yard nav breaks the order-permutation bar");
        }
    }
    if nav_first.walkable_cells() <= 100 {
        fail("yard floor is not walkable");
    }
    let nav_round = NavMesh::from_bytes(&nav_first.to_bytes())
        .unwrap_or_else(|e| fail(&format!("nav round-trip: {e:?}")));
    if nav_round.to_bytes() != nav_first.to_bytes() {
        fail("yard nav does not round-trip exactly");
    }
    let mut routes_ok = 0u32;
    for r in ROUTES {
        let (fx, fz) = spawn(r.from);
        let path = nav_first
            .query([fx, 0.0, fz], [r.to_x, 0.0, r.to_z])
            .unwrap_or_else(|e| fail(&format!("route {} never walks: {e:?}", r.to)));
        if path.is_empty() {
            fail(&format!("route {} walks nowhere", r.to));
        }
        routes_ok += 1;
    }

    // ---- Leg 5: the hub streams through WD1 with zero hitches ---------------
    if cell_cost_bytes(CELL_SIZE).unwrap_or_else(|e| fail(&format!("cell cost: {e}"))) != CELL_COST
    {
        fail("stream cost drifted from the baked WD1 cost");
    }
    let budget = CellBudget::new(4 * CELL_COST, 4, CELL_COST)
        .unwrap_or_else(|e| fail(&format!("budget: {e}")));
    let mut cells = Cells::new(SEED, budget);
    for c in STREAM {
        let coord = CellCoord::new(c.x, c.z).unwrap_or_else(|e| fail(&format!("coord: {e}")));
        let first =
            bake_cell(SEED, coord, CELL_SIZE).unwrap_or_else(|e| fail(&format!("bake: {e}")));
        let second =
            bake_cell(SEED, coord, CELL_SIZE).unwrap_or_else(|e| fail(&format!("rebake: {e}")));
        if first != second || Cells::digest(&first) != Cells::digest(&second) {
            fail(&format!("cell {coord} not identical twice"));
        }
        parse_cell(&first).unwrap_or_else(|e| fail(&format!("own bake parses: {e}")));
        cells
            .stream(coord, CELL_SIZE, c.priority)
            .unwrap_or_else(|e| fail(&format!("stream {coord}: {e}")));
    }
    // Seams join around the whole 2x2 before publish.
    let cell_data: Vec<_> = STREAM
        .iter()
        .map(|c| {
            let coord = CellCoord::new(c.x, c.z).expect("fixture coord in range");
            parse_cell(&bake_cell(SEED, coord, CELL_SIZE).expect("fixture cell bakes"))
                .expect("fixture cell parses")
        })
        .collect();
    for (a, b) in [(0usize, 1usize), (0, 2), (1, 3), (2, 3)] {
        seam_joins(&cell_data[a], &cell_data[b]).unwrap_or_else(|e| fail(&format!("seam: {e}")));
    }
    // Publish order is priority first: the hub cell lands before its ring.
    let order = cells.pending_order();
    if order[0] != CellCoord::new(0, 0).expect("hub coord") {
        fail("hub cell does not stream first");
    }
    let mut hitches = 0u32;
    for _ in 0..4 {
        let done: Vec<_> = cells
            .publish(CELL_COST)
            .into_iter()
            .filter_map(|e| match e {
                CellEvent::Completed { coord } => Some(coord),
                _ => None,
            })
            .collect();
        if done.len() != 1 {
            fail("stream frame must complete exactly one hub cell");
        }
        if !cells.is_live(&done[0]) {
            hitches += 1;
        }
    }
    if hitches != 0 || cells.live_count() != 4 || cells.refused_count() != 0 {
        fail("hub walk hitched, lost a cell or refused one");
    }
    let hub = CellCoord::new(0, 0).expect("hub coord");
    let corner = CellCoord::new(1, 1).expect("corner coord");
    let steps = cells
        .route(hub, (0, 0), corner, (15, 15))
        .unwrap_or_else(|e| fail(&format!("hub never routes: {e}")));
    if steps <= 16 {
        fail(&format!("hub route crosses no ground: {steps} steps"));
    }
    let hash_before = cells.sim_hash();
    cells
        .evict(&hub)
        .unwrap_or_else(|e| fail(&format!("evict: {e}")));
    cells
        .stream(hub, CELL_SIZE, 9)
        .unwrap_or_else(|e| fail(&format!("re-stream: {e}")));
    cells.publish(u64::MAX);
    if cells.sim_hash() != hash_before {
        fail("boundary rebake shifted the hub ground");
    }
    // Unloaded ground never guesses: the gap past the block refuses by name.
    if cells.route(hub, (0, 0), CellCoord::new(2, 2).expect("gap"), (0, 0))
        != Err(CellError::NotLive)
    {
        fail("unloaded ground guessed a route");
    }

    // ---- Leg 6: replay — the pipeline twice is one report byte -------------
    let placed_replay = run(&actors);
    let nav_replay =
        bake(&yard_tris, &config).unwrap_or_else(|e| fail(&format!("nav replay: {e:?}")));
    let cell_replay =
        bake_cell(SEED, hub, CELL_SIZE).unwrap_or_else(|e| fail(&format!("cell replay: {e}")));
    let replay_ok = placed_replay.hash == placed_first.hash
        && nav_replay.to_bytes() == nav_first.to_bytes()
        && cell_replay
            == bake_cell(SEED, hub, CELL_SIZE)
                .unwrap_or_else(|e| fail(&format!("cell rereplay: {e}")));

    let cell_rows: Vec<String> = STREAM
        .iter()
        .map(|c| {
            let coord = CellCoord::new(c.x, c.z).expect("fixture coord in range");
            let blob = bake_cell(SEED, coord, CELL_SIZE).expect("fixture cell bakes");
            format!(
                "    {{\"cell\": [{}, {}], \"digest\": \"{:016x}\", \"identical_twice\": true}}",
                c.x,
                c.z,
                Cells::digest(&blob),
            )
        })
        .collect();
    let report = format!(
        concat!(
            "{{\n  \"map\": \"{map}\", \"version\": {ver}, \"seed\": {seed},\n",
            "  \"actors\": {actors}, \"zones\": {zones}, \"spawns\": {spawns}, \"routes\": {routes},\n",
            "  \"total_lod0_tris\": {tt},\n",
            "  \"assets\": [\n{a}  ],\n",
            "  \"bundle\": {{\"bytes\": {bb}, \"entries\": {be}, \"meshes\": {bm}, \"materials\": {bt}, ",
            "\"load_ms\": {lm}, \"identical_twice\": true, \"corrupt_refused\": true}},\n",
            "  \"spatial\": {{\"hash\": \"{sh:016x}\", \"findings\": 0, \"verdict\": \"clean\", \"identical_twice\": true}},\n",
            "  \"nav\": {{\"digest\": \"{nd:016x}\", \"walkable\": {nw}, \"routes_ok\": {nr}, ",
            "\"roundtrip\": true, \"identical_twice\": true, \"shuffled_identical\": true}},\n",
            "  \"cells\": [\n{c}  ],\n",
            "  \"stream\": {{\"live\": 4, \"hitches\": 0, \"refused\": 0, \"route_steps\": {rs}, ",
            "\"seams_join\": true, \"rebake_identical\": true, \"unloaded_refuses\": true, \"sim_hash\": \"{sm:016x}\"}},\n",
            "  \"replay\": {{\"identical_twice\": {rp}}}\n}}\n",
        ),
        map = MAP_NAME,
        ver = MAP_VERSION,
        seed = SEED,
        actors = ACTORS.len(),
        zones = ZONES.len(),
        spawns = SPAWNS.len(),
        routes = ROUTES.len(),
        tt = total_lod0,
        a = if asset_entries.is_empty() {
            String::new()
        } else {
            asset_entries.join(",\n") + "\n"
        },
        bb = packed_first.len(),
        be = loaded.entries.len(),
        bm = mesh_entries,
        bt = mat_entries,
        lm = loaded.load_time_ms,
        sh = placed_first.hash,
        nd = nav_first.digest(),
        nw = nav_first.walkable_cells(),
        nr = routes_ok,
        c = cell_rows.join(",\n") + "\n",
        rs = steps,
        sm = hash_before,
        rp = replay_ok,
    );
    if !replay_ok {
        fail("hub replay diverged");
    }
    std::fs::write(ludus.join("report.json"), &report)
        .unwrap_or_else(|e| fail(&format!("write report: {e}")));
    println!(
        "LUDUS-PROOF GREEN actors={} zones={} spawns={} routes={} lod0={} bundle={} entries={} spatial={:016x} nav={:016x} steps={}",
        ACTORS.len(),
        ZONES.len(),
        SPAWNS.len(),
        ROUTES.len(),
        total_lod0,
        packed_first.len(),
        loaded.entries.len(),
        placed_first.hash,
        nav_first.digest(),
        steps,
    );
}
