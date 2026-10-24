// forward_torch_vertex.wgsl: the potato tier's vertex-rate torch lighting
// (SW5). Compiled AFTER `forward.wgsl` (the crate's shader gate composes the
// two, and the potato pipeline pairs these entries with the shipped bind
// groups), so every struct, binding and helper below comes from that file.
//
// Why: `forward.wgsl` `fs` sums the eight frame torches per fragment. The
// potato tier pays that sum once per vertex instead (`vs_torch`), writes it as
// one interpolant, and `fs_torch` adds the interpolated value: 8 lights at 4
// vertices per card, not per pixel. Measured on the CPU twin
// (`crate::torch_vertex`): a 15 cm card keeps the highlight within 1% of the
// per-fragment loop; a 1 m quad shows ~13%, which is why this variant stays on
// the potato row alone (`quality::TorchLighting::Vertex`).
//
// Two differences from `forward.wgsl` `fs`, both named here:
//   1. the torch term is `in.torch_sum`, not a per-fragment loop;
//   2. the vertex albedo is the material factor (`mat.base_color`), because the
//      base map is not sampled in the vertex stage.
// Everything else (sun shadow, IBL, GTAO, contact, fog, motion) is the same
// math in the same order as `fs`.

// The vertex-stage output: the frame's own interpolants plus the torch sum.
struct TorchVOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) draw_id: u32,
    @location(4) tangent: vec4<f32>,
    @location(5) cur_clip: vec4<f32>,
    @location(6) prev_clip: vec4<f32>,
    // SW5: the eight-torch sum at this vertex, interpolated across the card.
    @location(7) torch_sum: vec3<f32>,
};

// The potato tier's static mesh entry: `vs` plus the torch sum at the vertex.
@vertex
fn vs_torch(in: VIn, @builtin(instance_index) ii: u32) -> TorchVOut {
    var out: TorchVOut;
    let wp = vec4<f32>(in.pos, 1.0);
    out.world_pos = wp.xyz;
    out.world_n = in.normal;
    out.uv = in.uv;
    out.tangent = in.tangent;
    out.clip = cam.view_proj * wp;
    out.draw_id = ii;
    out.cur_clip = out.clip;
    out.prev_clip = out.clip;
    // SW5: the one sum the fragment stage reads instead of running itself.
    out.torch_sum = torch_frame_sum(wp.xyz, normalize(in.normal), mat.base_color);
    return out;
}

// The potato tier's window entry: `forward.wgsl` `fs` with the torch term
// replaced by the interpolated vertex sum. Linear HDR radiance, exposure only;
// the blit tone-maps.
@fragment
fn fs_torch(in: TorchVOut) -> FsOut {
    contact_live = contact_from_feet(in.world_pos);
    let detail = surface_detail();
    var base_texel: vec3<f32>;
    var map_texel: vec3<f32>;
    var orm: vec3<f32>;
    if (detail != 0u) {
        let dx = dpdx(in.uv);
        let dy = dpdy(in.uv);
        let cell = hex_cell(in.uv);
        base_texel = hex_sample(base_map, base_smp, cell, dx, dy);
        map_texel = hex_sample(normal_map, normal_smp, cell, dx, dy);
        orm = hex_sample(orm_map, normal_smp, cell, dx, dy);
    } else {
        base_texel = textureSample(base_map, base_smp, in.uv).rgb;
        map_texel = textureSample(normal_map, normal_smp, in.uv).rgb;
        orm = textureSample(orm_map, normal_smp, in.uv).rgb;
    }
    let albedo = mat.base_color * base_texel * surface_macro(detail, in.world_pos, normalize(in.world_n));
    let sun_vis = sun_shadow_at(in.world_pos, in.world_n);
    let map_n = eg12_decode_normal(map_texel, normalize(in.world_n));
    let mapped_n = eg12_world_normal(normalize(in.world_n), in.tangent, map_n);
    let n_ibl = normalize(mapped_n);
    let v_ibl = normalize(cam.camera_pos - in.world_pos);
    orm_scale = orm;
    spec_aa_k = spec_aa_kernel(dpdx(n_ibl), dpdy(n_ibl));
    let rough_eff = spec_aa_perceptual(clamp(mat.roughness * orm.y, 0.0, 1.0), spec_aa_k);
    let metal_eff = clamp(mat.metallic * orm.z, 0.0, 1.0);
    let f0_ibl = f0_term(albedo, metal_eff);
    let ibl_diffuse_term = ibl_diffuse(n_ibl, albedo, metal_eff);
    let ibl_spec_term = ibl_specular(n_ibl, v_ibl, f0_ibl, rough_eff);
    let occlusion: f32 = sun_contact_factor(in.world_pos);
    let ao = ao_at(in.clip.xy);
    let ao_all = ao * orm.x;
    let spec_ao = spec_occlusion(clamp(dot(n_ibl, v_ibl), 0.0, 1.0), ao_all, rough_eff * rough_eff);
    // SW5: the one difference in the light sum: the vertex-rate torch term.
    let lit = direct_light(in.world_pos, mapped_n, in.tangent, albedo, sun_vis) + ibl_diffuse_term * multi_bounce_ao(ao_all, albedo) * occlusion + ibl_spec_term * spec_ao + in.torch_sum;
    let radiance = max(lit * mat.exposure, vec3<f32>(0.0));
    let fog_f = fog_factor_finite(fog_fwd, cam.camera_pos, in.world_pos);
    var out: FsOut;
    out.color = vec4<f32>(mix(radiance, fog_fwd.color, fog_f), 1.0);
    out.id = in.draw_id;
    let cur_ndc = in.cur_clip.xy / max(in.cur_clip.w, 1e-6);
    var prev_ndc = cur_ndc;
    if (in.prev_clip.w > 1e-6) {
        prev_ndc = in.prev_clip.xy / in.prev_clip.w;
    }
    out.motion = (cur_ndc - prev_ndc) * vec2<f32>(0.5, -0.5);
    return out;
}
