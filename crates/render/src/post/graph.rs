use super::*;

// ---------------------------------------------------------------------------
// RP1 graph passes: the six post stages as RA2 data for WIN1/the viewport.
// The `window` `gpu::Rp1Submitter` records the same six in compiled order
// on a live device; this builder is its CPU-side declaration so order,
// aliasing and barriers come from RA2, never raw.
// ---------------------------------------------------------------------------

/// RP1 TAA resolve pass name on the frame graph.
pub const RP1_PASS_TAA: &str = "rp1-taa";
/// RP1 temporal upsample pass name on the frame graph.
pub const RP1_PASS_TAAU: &str = "rp1-taau";
/// RP1 bloom pass name on the frame graph.
pub const RP1_PASS_BLOOM: &str = "rp1-bloom";
/// RP1 exposure pass name on the frame graph.
pub const RP1_PASS_EXPOSURE: &str = "rp1-exposure";
/// RP1 ACES pass name on the frame graph.
pub const RP1_PASS_ACES: &str = "rp1-aces";
/// RP1 grading pass name on the frame graph.
pub const RP1_PASS_GRADE: &str = "rp1-grade";
/// Bytes per HDR texel in the transient plan (`Rgba32Float` history).
pub const RP1_HISTORY_BYTES_PER_TEXEL: u64 = 16;
/// Bytes per LDR staging texel (`Rgba16Float` post target).
pub const RP1_POST_BYTES_PER_TEXEL: u64 = 8;

/// The six RP1 pass names in execution order.
pub fn rp1_pass_names() -> [&'static str; 6] {
    [
        RP1_PASS_TAA,
        RP1_PASS_TAAU,
        RP1_PASS_BLOOM,
        RP1_PASS_EXPOSURE,
        RP1_PASS_ACES,
        RP1_PASS_GRADE,
    ]
}

/// Transient backing estimate in bytes for an RP1 frame: history plus
/// resolved (render size, 16 B/texel) plus the present-size stage plus
/// the bloom mip chain (present size halved per level, 16 B/texel).
/// Integer math only; used by the WIN1 budget check before submit.
pub fn rp1_transient_bytes(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    bloom_levels: u32,
) -> Result<u64, PostError> {
    check_dims(render_w, render_h)?;
    check_dims(present_w, present_h)?;
    if bloom_levels == 0 || bloom_levels > MAX_BLOOM_LEVELS {
        return Err(PostError::Invalid(format!(
            "rp1 bloom levels {bloom_levels} outside 1..={MAX_BLOOM_LEVELS}"
        )));
    }
    let render = render_w as u64 * render_h as u64;
    let mut total = render * RP1_HISTORY_BYTES_PER_TEXEL * 2;
    total = total.saturating_add(present_w as u64 * present_h as u64 * RP1_POST_BYTES_PER_TEXEL);
    let (mut mw, mut mh) = (present_w as u64, present_h as u64);
    for _ in 0..bloom_levels {
        mw = mw.div_ceil(2).max(1);
        mh = mh.div_ceil(2).max(1);
        total = total.saturating_add(mw * mh * RP1_HISTORY_BYTES_PER_TEXEL);
        if mw == 1 && mh == 1 {
            break;
        }
    }
    Ok(total)
}

/// Declare the six RP1 passes on a fresh RA2 frame graph for the WIN1
/// match frame (and the ED1 viewport on the same shape): TAA at the
/// render size, TAAU to the presentation size, then the four chain
/// stages. Resource flow infers `taa -> taau -> bloom -> exposure ->
/// aces -> grade`; explicit edges pin the same order so a resource-flow
/// change can never reorder the chain. Validates every dimension and the
/// chain config before adding any pass.
pub fn build_rp1_graph(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<crate::graph::FrameGraph, PostError> {
    chain.validate()?;
    check_dims(render_w, render_h)?;
    check_dims(present_w, present_h)?;
    let render_px = render_w as u64 * render_h as u64;
    let present_px = present_w as u64 * present_h as u64;
    let mut graph = crate::graph::FrameGraph::new();
    let taa = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_TAA)
            .input("rp1-current")
            .input("rp1-history")
            .input("rp1-motion")
            .input("rp1-jitter")
            .output("rp1-resolved")
            .transient("rp1-history-bytes", render_px * RP1_HISTORY_BYTES_PER_TEXEL)
            .work(1, 1),
    );
    let taau = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_TAAU)
            .input("rp1-resolved")
            .output("rp1-upsampled")
            .work(1, 1),
    );
    let bloom = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_BLOOM)
            .input("rp1-upsampled")
            .output("rp1-flared")
            .transient("rp1-bloom-bytes", present_px * RP1_HISTORY_BYTES_PER_TEXEL)
            .work(1, 1),
    );
    let exposure = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_EXPOSURE)
            .input("rp1-flared")
            .output("rp1-exposed")
            .work(1, 1),
    );
    let aces = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_ACES)
            .input("rp1-exposed")
            .output("rp1-toned")
            .work(1, 1),
    );
    let grade = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_GRADE)
            .input("rp1-toned")
            .output("rp1-ldr")
            .work(1, 1),
    );
    let order = [
        (taa, taau),
        (taau, bloom),
        (bloom, exposure),
        (exposure, aces),
        (aces, grade),
    ];
    for (from, to) in order {
        graph
            .add_edge(from, to)
            .map_err(|e| PostError::Invalid(format!("rp1 graph edge: {e}")))?;
    }
    Ok(graph)
}

/// WIN1 wiring: validate the match frame sizes plus the chain, declare
/// the six RP1 graph passes, compile, and check the compiled order names
/// the six stages in order. Returns the compiled graph the window submit
/// records on the device. Fails loudly on bad sizes or config.
pub fn compile_rp1_for_present(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<crate::graph::CompiledGraph, PostError> {
    let graph = build_rp1_graph(render_w, render_h, present_w, present_h, chain)?;
    let compiled = graph
        .compile()
        .map_err(|e| PostError::Invalid(format!("rp1 graph compile: {e}")))?;
    let names: Vec<&str> = compiled
        .order()
        .iter()
        .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
        .collect();
    let want = rp1_pass_names();
    if names != want {
        return Err(PostError::Invalid(format!(
            "rp1 compiled order {names:?} differs from {want:?}"
        )));
    }
    Ok(compiled)
}

// ---------------------------------------------------------------------------
// RP5 scale seam: post/TAAU production wiring over the RP4 TAA-off graph.
// The `aa_scale` API stays frozen: this seam only feeds the live
// `ScaleController` (hysteresis on PF2 frame ms) into
// `compile_taa_off_for_scale` and maps its errors into `PostError`, then
// re-checks the SMAA-first order through the `graph.rs` seam so the
// SMAA-first graph (never TAA) drives the presented size. Consumer: the
// WIN1 TAA-off loop (RP6 selection rides on top of this seam).
// Budget: `tick_scale_for_present` is the per-present call; the hysteresis
// tick itself allocates nothing, the compile allocates its RA2 plan only.
// ---------------------------------------------------------------------------

impl From<crate::aa_scale::AaScaleError> for PostError {
    fn from(e: crate::aa_scale::AaScaleError) -> Self {
        match e {
            crate::aa_scale::AaScaleError::Invalid(m) => Self::Invalid(m),
            crate::aa_scale::AaScaleError::BudgetExceeded { requested, allowed } => {
                Self::BudgetExceeded { requested, allowed }
            }
            crate::aa_scale::AaScaleError::Device(m) => Self::Device(m),
            crate::aa_scale::AaScaleError::Validation(m) => Self::Validation(m),
        }
    }
}

/// Render size the controller drives for this present: scaled, rounded,
/// forced even (the TAAU path stays exact), clamped to the present size.
/// Same inputs give the same size; errors map into `PostError`.
pub fn taa_off_render_size_for_scale(
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
) -> Result<(u32, u32), PostError> {
    controller
        .render_size(present_w, present_h)
        .map_err(PostError::from)
}

/// Production TAA-off compile at the controller scale: sizes from the
/// controller, declares the SMAA-first graph via
/// `aa_scale::compile_taa_off_for_scale` (frozen API), then re-checks the
/// compiled order through the `graph.rs` SMAA-first seam. Returns the
/// compiled graph plus the render size it was built at. Fails loudly on
/// bad sizes or a non-SMAA-first order.
pub fn compile_taa_off_for_scale(
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<(crate::graph::CompiledGraph, u32, u32), PostError> {
    let (compiled, render_w, render_h) =
        crate::aa_scale::compile_taa_off_for_scale(controller, present_w, present_h, chain)
            .map_err(PostError::from)?;
    if !compiled.order_names_match(&crate::aa_scale::taa_off_pass_names()) {
        return Err(PostError::Invalid(format!(
            "taa-off compiled order is not smaa-first at {render_w}x{render_h}"
        )));
    }
    Ok((compiled, render_w, render_h))
}

/// Per-present production tick: feed one PF2 frame time through the
/// controller hysteresis (fast step-down past target, slow step-up with
/// headroom, band resets so borderline frames never flap), then compile
/// the SMAA-first graph at the new scale. Returns the compiled graph, the
/// render size it was built at, and the scale for the frame report.
pub fn tick_scale_for_present(
    controller: &mut crate::aa_scale::ScaleController,
    frame_ms: f64,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<(crate::graph::CompiledGraph, u32, u32, f32), PostError> {
    let scale = controller.update(frame_ms).map_err(PostError::from)?;
    let (compiled, render_w, render_h) =
        compile_taa_off_for_scale(controller, present_w, present_h, chain)?;
    Ok((compiled, render_w, render_h, scale))
}

/// CPU TAA-off frame at the controller scale: sizes from the controller,
/// then runs the frozen `aa_scale::run_taa_off_frame` oracle (SMAA at the
/// render size, RP1 TAAU plus chain after it, never a second chain). Same
/// complete inputs give bit-identical pixels; only `ms` varies.
pub fn run_taa_off_frame_for_scale(
    hdr: &[[f32; 3]],
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
    smaa: &crate::aa_scale::SmaaParams,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<crate::aa_scale::TaaOffOutput, PostError> {
    let (render_w, render_h) = taa_off_render_size_for_scale(controller, present_w, present_h)?;
    crate::aa_scale::run_taa_off_frame(
        hdr, render_w, render_h, present_w, present_h, smaa, chain, lights,
    )
    .map_err(PostError::from)
}
