# Skill-on vs skill-off pairs: sky-bake-checks

Three review situations. "Off" is what a reviewer says from habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it prevents, and the file or pixel proof it asks for.

## P1: bake on disk but the frame ignores it (items 6, 7)

Situation: A builder runs `bake_sky` with `SkyParams::midday` at 64x32 and the bytes match x2, but `fs_sky` in `crates/render/src/shaders/forward.wgsl` still mixes `fog_fwd.color` to `fog_fwd.zenith` with no texture, bound through `FogFwd::day` in `presenter.rs` from two colours only. The builder calls LF1 done.

Off: "The bake is deterministic and the sky looks blue, so the sky pass is live."

On: "Not live. Item 6: `fs_sky` must sample the sky-view texture with `fog_factor_infinite`, bound via `FogFwd::day` from `look_fog_color` bake, not two colours. Item 7: sky region on-vs-off must differ above the EG8 epsilon and the fully fogged composite must sit within `FOG_SKY_RAMP_TOL` of the fog color. Proof: sky pixels differ on vs off, capture x2 equal, `arena per-pass` sky above 0 ms. Search `node tools/books.mjs search \"outdoor scene lighting skylight\"` and cite [gems1 ch.15]."

What skill-less misses: it treats a passing bake as a live pass. It never checks the texture bind or the pixels, so identical frames pass as sky.

## P2: hazy air plus white sand blows out (items 2, 8, 9)

Situation: A builder sets turbidity 18.0 with ground albedo 1.0/1.0/1.0 and bakes exposure into the texels, then tone maps again in `fs_sky`. Noon sand glares white and dusk goes red-black, but the unit test on one noon texel passes.

Off: "Turbidity is just haze and white sand is brightest, so the glare is correct. Exposure in the bake is simpler."

On: "Three findings. Item 2: `turbidity` must stay inside `SKY_TURBIDITY_MIN..=SKY_TURBIDITY_MAX` with the day rig near 2.0-3.0 (`ARENA_DAY_TURBIDITY` 3.0); 18.0 breaks the scatter. Item 9: `ground_albedo` must sit in 0..=1 with plausible sand (`arena_day.rs` 0.42/0.32/0.20), never 1.0 white. Item 8: `ev100_to_exposure` must carry `exposure_ev100` 12.0 to the uniforms with one tone map in post, never baked into texels. Proof: `SkyParams::validate` rejects both inputs, noon and dusk captures differ sanely. Search `node tools/books.mjs search \"outdoor scene lighting skylight\"` and cite [gems1 ch.15]."

What skill-less misses: input ranges and the exposure split. One noon sample hides the haze blowout and the double tone map, so every time of day ships wrong.

## P3: fog tinted by zenith plus drifting bytes (items 4, 5, 3)

Situation: A builder averages the whole `SkyCapture` for fog so zenith blue tints the sand haze, rebakes 512x256 every frame, and sums texels in f32 so `to_bytes` drifts between runs. Fog looks blue and the frame hitches.

Off: "Fog from the sky average is fine and bigger bakes are sharper. Byte drift is float noise."

On: "Three findings. Item 5: `fog_color` must average the equator rows only into `look_fog_color` and `FogParams::from_sky_capture`; zenith and ground must never tint it. Item 3: `bake_sky` must stay 64x32 under `SKY_CAPTURE_MAX_DIM` inside `SKY_BUDGET_MS` 0.4 ms, cached until time moves via `spawn_look_bake`. Item 4: `to_bytes` must match x2 with row-major order and f64 accumulation in `mean_radiance`. Proof: fog composite within `FOG_SKY_RAMP_TOL` of horizon, bake x2 identical, no per-frame rebake. Search `node tools/books.mjs search \"outdoor scene lighting skylight\"` and cite [gems1 ch.15]."

What skill-less misses: which rows feed fog, the bake budget, and determinism. Without them fog is blue, frames hitch, and reruns drift.
