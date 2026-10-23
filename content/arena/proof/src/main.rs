// CT8 proof binary: cooks the arena kit through the engine's own lanes
// (CT2 mesh cook, A2 texture cook, AS1 bundle) and runs the engine spatial
// gate over the placed kit. No new engine code: only calls landed APIs.
// Exit 0 with report.json + previews on green; exit 1 naming the failure.
use engine_assets::{
    cook_gltf_bytes, cook_texture, texture_asset_id, tri_count, BundleEntry,
    BundleLoader, BundleWriter, CookedMaterial, EntryKind, MeshClass, TextureRef, TextureSlot,
};
use engine_spatial::{run, Actor};

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/placement_data.rs"));

fn fail(msg: &str) -> ! {
    eprintln!("CT8-PROOF FAIL: {msg}");
    std::process::exit(1);
}

fn read(dir: &std::path::Path, rel: &str) -> Vec<u8> {
    let p = dir.join(rel);
    std::fs::read(&p).unwrap_or_else(|e| fail(&format!("read {}: {e}", p.display())))
}

// ---- minimal deterministic PNG (RGB8, filter 0, stored deflate) ----
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    c ^ 0xFFFF_FFFF
}
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
fn png_chunk(ty: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut c = Vec::with_capacity(4 + data.len());
    c.extend_from_slice(ty);
    c.extend_from_slice(data);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}
fn write_png(path: &std::path::Path, w: u32, h: u32, rgb: &[u8]) {
    let mut raw = Vec::with_capacity((h * (1 + w * 3)) as usize);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgb[(y * w * 3) as usize..((y + 1) * w * 3) as usize]);
    }
    let mut def = vec![0x78, 0x01];
    let mut rest = raw.as_slice();
    while !rest.is_empty() {
        let n = rest.len().min(65535);
        def.push(if n == rest.len() { 1 } else { 0 });
        def.extend_from_slice(&(n as u16).to_le_bytes());
        def.extend_from_slice(&(!n as u16).to_le_bytes());
        def.extend_from_slice(&rest[..n]);
        rest = &rest[n..];
    }
    def.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(b"IHDR", &ihdr, &mut png);
    png_chunk(b"IDAT", &def, &mut png);
    png_chunk(b"IEND", &[], &mut png);
    std::fs::write(path, png).unwrap_or_else(|e| fail(&format!("write png: {e}")));
}

// ---- tiny orthographic preview renderer over cooked LOD0 triangles ----
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}
impl Canvas {
    fn new(w: u32, h: u32, bg: [u8; 3]) -> Self {
        let mut px = vec![0u8; (w * h * 3) as usize];
        for c in px.chunks_exact_mut(3) {
            c.copy_from_slice(&bg);
        }
        Self { w, h, px }
    }
    fn dot(&mut self, x: i32, y: i32, col: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = ((y as u32 * self.w + x as u32) * 3) as usize;
            self.px[i..i + 3].copy_from_slice(&col);
        }
    }
    fn tri(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), col: [u8; 3]) {
        let (x0, x1) = (a.0.min(b.0).min(c.0).floor() as i32, a.0.max(b.0).max(c.0).ceil() as i32);
        let (y0, y1) = (a.1.min(b.1).min(c.1).floor() as i32, a.1.max(b.1).max(c.1).ceil() as i32);
        let edge = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
            (r.0 - p.0) * (q.1 - p.1) - (r.1 - p.1) * (q.0 - p.0)
        };
        let area = edge(a, b, c);
        if area.abs() < 1e-9 {
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(b, c, p) / area;
                let w1 = edge(c, a, p) / area;
                let w2 = edge(a, b, p) / area;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    self.dot(x, y, col);
                }
            }
        }
    }
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 3]) {
        for y in y0.min(y1)..=y0.max(y1) {
            for x in x0.min(x1)..=x0.max(x1) {
                self.dot(x, y, col);
            }
        }
    }
    fn disc(&mut self, cx: i32, cy: i32, r: i32, col: [u8; 3]) {
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r {
                    self.dot(x, y, col);
                }
            }
        }
    }
}

const REF_H: f32 = 1.8; // 1.8 m reference figure next to every asset.
const AMBER: [u8; 3] = [230, 160, 40];
const GROUND: [u8; 3] = [70, 130, 80];

#[allow(clippy::too_many_lines)]
fn preview(
    path: &std::path::Path,
    tris: &[(u32, Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>)],
) -> ([f32; 3], [f32; 3], f32) {
    // Gather bounds over all LOD0 meshes.
    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    for (_, pos, _, _) in tris {
        for p in pos {
            for i in 0..3 {
                mn[i] = mn[i].min(p[i]);
                mx[i] = mx[i].max(p[i]);
            }
        }
    }
    let w = (mx[0] - mn[0]).max(0.01);
    let h = (mx[1] - mn[1]).max(0.01);
    let d = (mx[2] - mn[2]).max(0.01);
    let need_h = h.max(d).max(REF_H + 0.1);
    let need_w = w.max(d).max(0.6) + 1.2; // room for the reference figure.
    let s = (224.0 / need_w).min(284.0 / need_h);
    let mut cv = Canvas::new(720, 300, [24, 26, 32]);
    // Views: front (x,y), side (z,y), top (x,z). Shade reads the face normal
    // on the view axis, so bumps and steps read as shading, not just outline.
    for (view, axis) in [(0, 2), (1, 0), (2, 1)] {
        let ox = 8.0 + view as f32 * 240.0;
        let ch = if axis == 0 { (mn[2] + mx[2]) / 2.0 } else { (mn[0] + mx[0]) / 2.0 };
        let cv2 = if axis == 1 { (mn[2] + mx[2]) / 2.0 } else { (mn[1] + mx[1]) / 2.0 };
        let to_px = |a: f32, b: f32| (ox + 112.0 + (a - ch) * s, 292.0 - (b - cv2) * s);
        if axis != 1 {
            // Ground line at y = 0 for the elevation views.
            let gy = (292.0 - (0.0 - mn[1]) * s) as i32;
            for x in (ox as i32)..(ox as i32 + 224) {
                cv.dot(x, gy, GROUND);
            }
        }
        let mut order: Vec<(f32, f32, (f32, f32), (f32, f32), (f32, f32))> = Vec::new();
        for (_, pos, nrm, idx) in tris {
            for t in idx.chunks_exact(3) {
                let p = |i: u32| pos[i as usize];
                let (pa, pb, pc) = (p(t[0]), p(t[1]), p(t[2]));
                let n = nrm[t[0] as usize];
                let (qa, qb, qc, depth) = match axis {
                    2 => {
                        let q = |v: [f32; 3]| to_px(v[0], v[1]);
                        (q(pa), q(pb), q(pc), pa[2] + pb[2] + pc[2])
                    }
                    0 => {
                        let q = |v: [f32; 3]| to_px(v[2], v[1]);
                        (q(pa), q(pb), q(pc), pa[0] + pb[0] + pc[0])
                    }
                    _ => {
                        let q = |v: [f32; 3]| to_px(v[0], v[2]);
                        (q(pa), q(pb), q(pc), -(pa[1] + pb[1] + pc[1]))
                    }
                };
                order.push((depth, n[axis as usize].abs(), qa, qb, qc));
            }
        }
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, facing, a, b, c) in order {
            let shade = (110.0 + 80.0 * facing.clamp(0.0, 1.0)) as u8;
            cv.tri(a, b, c, [shade, shade, (shade as f32 * 0.96) as u8]);
        }
        // 1.8 m reference figure left of the asset (elevations) / marker (top).
        let rx = ox + 112.0 - (w.max(d) / 2.0 + 0.7) * s;
        if axis == 1 {
            cv.disc(rx as i32, (292.0 - cv2 * s) as i32, (0.25 * s).max(1.5) as i32, AMBER);
        } else {
            let base = 292.0 - (0.0 - mn[1]) * s;
            let rw = (0.36 * s).max(2.0);
            cv.rect(
                (rx - rw / 2.0) as i32,
                (base - 1.4 * s) as i32,
                (rx + rw / 2.0) as i32,
                base as i32,
                AMBER,
            );
            cv.disc(rx as i32, (base - 1.62 * s) as i32, (0.13 * s).max(1.5) as i32, AMBER);
        }
    }
    write_png(path, cv.w, cv.h, &cv.px);
    (mn, mx, s)
}

fn main() {
    let arena = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
        });
    let prev_dir = arena.join("previews");
    std::fs::create_dir_all(&prev_dir).unwrap_or_else(|e| fail(&format!("mkdir previews: {e}")));

    let mut assets_json = String::new();
    let mut asset_entries: Vec<String> = Vec::new();
    let mut total_lod0 = 0u32;
    // Bundle pass: identical writer built twice, bytes compared.
    let mut bundle_names: Vec<(EntryKind, String)> = Vec::new();
    let build_bundle = || -> BundleWriter {
        let mut w = BundleWriter::new();
        for asset in ASSETS {
            let bytes = read(&arena, &format!("src/{asset}.gltf"));
            let cooked = cook_gltf_bytes(&bytes, MeshClass::Prop)
                .unwrap_or_else(|e| fail(&format!("cook {asset}: {e}")));
            for (mi, m) in cooked.meshes.iter().enumerate() {
                for (l, lod) in m.lods.iter().enumerate() {
                    w.add_mesh(&format!("arena/{asset}/m{mi:02}_lod{l}"), lod)
                        .unwrap_or_else(|e| fail(&format!("bundle mesh: {e}")));
                }
            }
            for ch in &cooked.chunks {
                let piece = ch.name.strip_prefix("FRAC_").unwrap_or(&ch.name);
                w.add_mesh(&format!("arena/chunk_{piece}"), &ch.mesh)
                    .unwrap_or_else(|e| fail(&format!("bundle chunk: {e}")));
            }
            let f = factors(asset);
            let tex_name: &str = ASSET_TEX
                .iter()
                .find(|(a, _)| *a == *asset)
                .map(|(_, t)| *t)
                .unwrap_or(asset);
            let albedo = read(&arena, &format!("textures/{tex_name}.png"));
            let flat = read(&arena, "textures/flat_normal.png");
            let mat = CookedMaterial {
                base_color_factor: f.base,
                metallic_factor: f.metallic,
                roughness_factor: f.roughness,
                textures: vec![
                    TextureRef { asset: texture_asset_id(&albedo), slot: TextureSlot::BaseColor },
                    TextureRef { asset: texture_asset_id(&flat), slot: TextureSlot::Normal },
                ],
                kind: engine_assets::cooked::MATERIAL_KIND_STONE,
            };
            w.add_material(&format!("arena/mat_{asset}"), &mat)
                .unwrap_or_else(|e| fail(&format!("bundle mat: {e}")));
        }
        for (name, _) in TEXTURES {
            let bytes = read(&arena, &format!("textures/{name}.png"));
            let slot = match TEXTURES.iter().find(|(n, _)| *n == *name).unwrap().1 {
                "BaseColor" => TextureSlot::BaseColor,
                "MetallicRoughness" => TextureSlot::MetallicRoughness,
                _ => TextureSlot::Normal,
            };
            let tex = cook_texture(&bytes, slot)
                .unwrap_or_else(|e| fail(&format!("cook tex {name}: {e:?}")));
            w.add_texture(&format!("arena/tex_{name}"), &tex)
                .unwrap_or_else(|e| fail(&format!("bundle tex: {e}")));
        }
        w
    };
    let b1 = build_bundle().finish();
    let b2 = build_bundle().finish();
    if b1 != b2 {
        fail("bundle cook not identical twice");
    }
    // Consumer load path: the exact loader WIN1/ED1 use at startup.
    let loaded = BundleLoader::new().load(&b1).unwrap_or_else(|e| fail(&format!("load: {e}")));
    for e in &loaded.entries {
        bundle_names.push((e.kind, e.name.clone()));
        match e.kind {
            EntryKind::Mesh => {
                e.decode_mesh().unwrap_or_else(|er| fail(&format!("decode {}: {er}", e.name)));
            }
            EntryKind::Material => {
                e.decode_material().unwrap_or_else(|er| fail(&format!("decode {}: {er}", e.name)));
            }
            EntryKind::Texture => {
                e.decode_texture().unwrap_or_else(|er| fail(&format!("decode {}: {er}", e.name)));
            }
            _ => fail("unexpected entry kind"),
        }
    }
    // Corrupt bundle must be refused, never loaded.
    {
        let first_len = loaded.entries[0].name.len();
        let at = 16 + 1 + 2 + first_len + 4;
        let mut bad = b1.clone();
        bad[at] ^= 0xFF;
        if BundleLoader::new().load(&bad).is_ok() {
            fail("corrupt bundle loaded");
        }
    }

    for asset in ASSETS {
        let bytes = read(&arena, &format!("src/{asset}.gltf"));
        let a1 = cook_gltf_bytes(&bytes, MeshClass::Prop)
            .unwrap_or_else(|e| fail(&format!("cook {asset}: {e}")));
        let a2 = cook_gltf_bytes(&bytes, MeshClass::Prop)
            .unwrap_or_else(|e| fail(&format!("recook {asset}: {e}")));
        if a1 != a2 {
            fail(&format!("cook of {asset} not identical twice"));
        }
        if a1.digest != engine_assets::asset_digest(&a1) {
            fail(&format!("digest mismatch on {asset}"));
        }
        let lod0: u32 = a1.meshes.iter().map(|m| m.base_tris).sum();
        total_lod0 += lod0;
        if lod0 > engine_assets::PROP_MAX_TRIS {
            fail(&format!("{asset} over prop budget: {lod0}"));
        }
        let mut lod_tris = Vec::new();
        let mut hull_ok = true;
        let mut min_area = f32::INFINITY;
        let mut uv_min = [f32::INFINITY; 2];
        let mut uv_max = [f32::NEG_INFINITY; 2];
        let mut render: Vec<(u32, Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>)> = Vec::new();
        for (mi, m) in a1.meshes.iter().enumerate() {
            for (l, lod) in m.lods.iter().enumerate() {
                lod_tris.push(tri_count(lod));
                if tri_count(lod) == 0 {
                    fail(&format!("{asset} mesh {mi} LOD{l} is empty (invisible at distance)"));
                }
                if !m.hulls[l].contains_mesh(lod) {
                    hull_ok = false;
                }
            }
            if a1.meshes.len() > 1 {
                let _ = mi;
            }
            let base = &m.lods[0];
            for t in base.indices.chunks_exact(3) {
                let (p0, p1, p2) = (
                    base.positions[t[0] as usize],
                    base.positions[t[1] as usize],
                    base.positions[t[2] as usize],
                );
                let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                let cr = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let area = 0.5 * (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt();
                if area.is_finite() && area < min_area {
                    min_area = area;
                }
            }
            for uv in &base.uv0 {
                for i in 0..2 {
                    uv_min[i] = uv_min[i].min(uv[i]);
                    uv_max[i] = uv_max[i].max(uv[i]);
                }
            }
            if col_meshes(asset).contains(&mi) {
                continue; // COL_ proxies are collision-only, never rendered.
            }
            render.push((mi as u32, base.positions.clone(), base.normals.clone(), base.indices.clone()));
        }
        if !hull_ok {
            fail(&format!("hull does not contain mesh in {asset}"));
        }
        if asset == &"crate" && a1.chunks.len() != 6 {
            fail(&format!("crate chunks = {}, want 6", a1.chunks.len()));
        }
        let (mn, mx, ppm) = preview(&prev_dir.join(format!("preview_{asset}.png")), &render);
        asset_entries.push(format!(
            "    {{\"asset\": \"{a}\", \"meshes\": {m}, \"chunks\": {c}, \"lod0_tris\": {t}, \"lod_tris\": [{l}], \"digest\": \"{d:016x}\", \"hulls_contain\": true, \"bbox_min\": [{x0:.3}, {y0:.3}, {z0:.3}], \"bbox_max\": [{x1:.3}, {y1:.3}, {z1:.3}], \"min_tri_area_m2\": {ar:.3e}, \"uv_min\": [{u0:.3}, {u1:.3}], \"uv_max\": [{v0:.3}, {v1:.3}], \"preview_px_per_m\": {s:.1}}}",
            a = asset,
            m = a1.meshes.len(),
            c = a1.chunks.len(),
            t = lod0,
            l = lod_tris.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
            d = a1.digest,
            x0 = mn[0],
            y0 = mn[1],
            z0 = mn[2],
            x1 = mx[0],
            y1 = mx[1],
            z1 = mx[2],
            ar = min_area,
            u0 = uv_min[0],
            u1 = uv_min[1],
            v0 = uv_max[0],
            v1 = uv_max[1],
            s = ppm,
        ));
    }
    assets_json.push_str(&asset_entries.join(",\n"));

    let mut tex_entries: Vec<String> = Vec::new();
    for (name, slot_name) in TEXTURES {
        let bytes = read(&arena, &format!("textures/{name}.png"));
        let slot = match *slot_name {
            "BaseColor" => TextureSlot::BaseColor,
            "MetallicRoughness" => TextureSlot::MetallicRoughness,
            _ => TextureSlot::Normal,
        };
        let t1 = cook_texture(&bytes, slot).unwrap_or_else(|e| fail(&format!("tex {name}: {e:?}")));
        let t2 = cook_texture(&bytes, slot).unwrap_or_else(|e| fail(&format!("tex recook {name}: {e:?}")));
        if t1 != t2 {
            fail(&format!("texture cook of {name} not identical twice"));
        }
        if t1.width > 2048 || t1.height > 2048 {
            fail(&format!("texture {name} over 2k budget"));
        }
        let last = t1.mips.last().unwrap();
        if last.width != 1 || last.height != 1 {
            fail(&format!("texture {name} mip chain does not reach 1x1"));
        }
        tex_entries.push(format!(
            "    {{\"texture\": \"{name}\", \"slot\": \"{slot_name}\", \"size\": [{w}, {h}], \"mips\": {m}}}",
            name = name,
            slot_name = slot_name,
            w = t1.width,
            h = t1.height,
            m = t1.mips.len(),
        ));
    }
    let mut tex_json = String::new();
    tex_json.push_str(&tex_entries.join(",\n"));

    // Spatial gate over the placed kit: zero findings required.
    let actors: Vec<Actor> = PLACEMENT
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
    let r1 = run(&actors);
    let r2 = run(&actors);
    if r1.hash != r2.hash || r1.findings != r2.findings {
        fail("spatial run not deterministic");
    }
    if !r1.findings.is_empty() {
        fail(&format!("spatial findings: {:?}", r1.findings));
    }

    let entry_list: Vec<String> = bundle_names
        .iter()
        .map(|(k, n)| format!("      {{\"kind\": \"{k:?}\", \"name\": \"{n}\"}}"))
        .collect();
    let report = format!(
        concat!(
            "{{\n  \"kit\": \"ct8-arena-kit\",\n  \"total_lod0_tris\": {tt},\n",
            "  \"bundle\": {{\"bytes\": {bb}, \"entries\": {be}, \"load_ms\": {lm}, ",
            "\"identical_twice\": true, \"corrupt_refused\": true}},\n",
            "  \"spatial\": {{\"actors\": {sa}, \"findings\": 0, \"verdict\": \"clean\", ",
            "\"hash\": \"{sh:016x}\", \"identical_twice\": true}},\n",
            "  \"assets\": [\n{a}  ],\n  \"textures\": [\n{t}  ],\n",
            "  \"bundle_entries\": [\n{e}\n  ]\n}}\n"
        ),
        tt = total_lod0,
        bb = b1.len(),
        be = loaded.entries.len(),
        lm = loaded.load_time_ms,
        sa = actors.len(),
        sh = r1.hash,
        a = assets_json,
        t = tex_json,
        e = entry_list.join(",\n"),
    );
    // Keep an unused-import firewall green: every bundle entry kind decoded above.
    let _: &Vec<BundleEntry> = &loaded.entries;
    std::fs::write(arena.join("report.json"), &report)
        .unwrap_or_else(|e| fail(&format!("write report: {e}")));
    println!("CT8-PROOF GREEN total_lod0={total_lod0} bundle_bytes={} entries={} spatial_hash={:016x}",
        b1.len(), loaded.entries.len(), r1.hash);
}
