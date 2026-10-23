//! pipeline_warmup: count every GPU pipeline the presenter builds and say loudly when one is
//! built after the loading screen (WEB13).
//!
//! A pipeline built on first use is the usual first-minute stutter: the browser (or the driver)
//! compiles the shader while a frame waits. The rule is that everything a frame draws with is
//! built before the first frame, behind the loader. This module is the proof of it: the presenter
//! and its submitters build their pipelines through [`CountedPipelines`]
//! (`CountedPipelines`, `_counted` twins of the wgpu calls), each build is written to a per-thread ledger, and the
//! loader calls [`seal`] once the warm-up is done. A build after the seal is a *late compile*:
//! [`late_line`] turns it into the loud `late_compile` line the web page prints and the headless
//! check fails on.
//!
//! wgpu 30 has no async pipeline call (`createRenderPipelineAsync` is only a raw binding behind
//! `Device::as_webgpu`, and its result cannot become a `wgpu::RenderPipeline`), so on the web the
//! warm-up is the closest non-blocking thing: build every pipeline, draw the real frame a few
//! times and `await` the queue's work-done signal before the page says it is ready. The browser
//! then compiles on its own threads behind the loading screen.
//!
//! Units: counts and labels. Determinism: no clock, no hash; the ledger is presentation
//! bookkeeping and never enters a simulation hash. The ledger is per thread (the web page is one
//! thread; each test runs on its own), so parallel tests do not see each other's builds.
//! Caller: `present/gpu_present` and the submitters it owns, `crates/web` (the loader line).

use std::cell::RefCell;

/// The builds seen on one thread and where the warm-up ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    created: Vec<String>,
    sealed_at: Option<usize>,
}

impl Ledger {
    /// Write one build. Unlabeled pipelines are counted under `unlabeled`.
    pub fn note(&mut self, label: Option<&str>) {
        self.created.push(label.unwrap_or("unlabeled").to_string());
    }

    /// End the warm-up: every build from now on is late. Returns the builds so far. A second
    /// call keeps the first seal (the warm-up ended once).
    pub fn seal(&mut self) -> usize {
        *self.sealed_at.get_or_insert(self.created.len())
    }

    /// Every build so far, warm-up and late.
    #[must_use]
    pub fn total(&self) -> usize {
        self.created.len()
    }

    /// True after [`Ledger::seal`].
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.sealed_at.is_some()
    }

    /// Builds before the seal (all of them while unsealed).
    #[must_use]
    pub fn warm(&self) -> usize {
        self.sealed_at.unwrap_or(self.created.len())
    }

    /// Labels of the builds after the seal, in order.
    #[must_use]
    pub fn late(&self) -> &[String] {
        match self.sealed_at {
            Some(at) => &self.created[at..],
            None => &[],
        }
    }

    /// Labels of every build, in order.
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.created
    }
}

thread_local! {
    static LEDGER: RefCell<Ledger> = RefCell::new(Ledger::default());
}

/// Note one build on this thread's ledger.
pub fn note_created(label: Option<&str>) {
    LEDGER.with(|l| l.borrow_mut().note(label));
}

/// End the warm-up on this thread: later builds are late. Returns the builds so far.
pub fn seal() -> usize {
    LEDGER.with(|l| l.borrow_mut().seal())
}

/// A copy of this thread's ledger.
#[must_use]
pub fn snapshot() -> Ledger {
    LEDGER.with(|l| l.borrow().clone())
}

/// Forget everything on this thread (a new presenter, or a test).
pub fn reset() {
    LEDGER.with(|l| *l.borrow_mut() = Ledger::default());
}

/// The loader line: `pipelines=<n> ms=<t>`, `n` the builds before the seal and `t` the
/// milliseconds the warm-up took (whole milliseconds, as the page prints them).
#[must_use]
pub fn ready_line(ledger: &Ledger, ms: f64) -> String {
    let ms = if ms.is_finite() && ms > 0.0 { ms } else { 0.0 };
    format!("pipelines={} ms={ms:.0}", ledger.warm())
}

/// `None` while no pipeline was built late; otherwise the loud line naming the first few.
#[must_use]
pub fn late_line(ledger: &Ledger) -> Option<String> {
    let late = ledger.late();
    if late.is_empty() {
        return None;
    }
    let shown: Vec<&str> = late.iter().take(4).map(String::as_str).collect();
    Some(format!(
        "late_compile count={} first={}",
        late.len(),
        shown.join(",")
    ))
}

/// `Device::create_render_pipeline` and `create_compute_pipeline` with a ledger entry each.
/// Import it with `use crate::pipeline_warmup::CountedPipelines as _;` and call the `_counted`
/// twin where a pipeline is built: the call keeps the shape of the wgpu call it replaces.
#[cfg(feature = "gpu")]
pub trait CountedPipelines {
    /// `create_render_pipeline`, counted.
    fn create_render_pipeline_counted(
        &self,
        desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> wgpu::RenderPipeline;
    /// `create_compute_pipeline`, counted.
    fn create_compute_pipeline_counted(
        &self,
        desc: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> wgpu::ComputePipeline;
}

#[cfg(feature = "gpu")]
impl CountedPipelines for wgpu::Device {
    fn create_render_pipeline_counted(
        &self,
        desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> wgpu::RenderPipeline {
        note_created(desc.label);
        self.create_render_pipeline(desc)
    }

    fn create_compute_pipeline_counted(
        &self,
        desc: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> wgpu::ComputePipeline {
        note_created(desc.label);
        self.create_compute_pipeline(desc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_warmup_counts_builds_before_the_seal_and_flags_the_rest() {
        let mut l = Ledger::default();
        for name in ["forward", "sky", "blit"] {
            l.note(Some(name));
        }
        assert_eq!((l.total(), l.warm(), l.late().len()), (3, 3, 0));
        assert!(!l.is_sealed());
        assert_eq!(l.seal(), 3);
        assert!(late_line(&l).is_none(), "nothing late yet");
        l.note(Some("overlay"));
        l.note(None);
        assert_eq!((l.total(), l.warm()), (5, 3));
        assert_eq!(l.late(), ["overlay", "unlabeled"]);
        let line = late_line(&l).expect("two builds came late");
        assert_eq!(line, "late_compile count=2 first=overlay,unlabeled");
        // The warm-up ended once: a second seal does not move it.
        assert_eq!(l.seal(), 3);
        assert_eq!(l.late().len(), 2);
    }

    #[test]
    fn pipeline_warmup_ready_line_has_the_two_fields_the_page_prints() {
        let mut l = Ledger::default();
        l.note(Some("a"));
        l.note(Some("b"));
        l.seal();
        assert_eq!(ready_line(&l, 412.6), "pipelines=2 ms=413");
        assert_eq!(ready_line(&l, f64::NAN), "pipelines=2 ms=0");
        assert_eq!(ready_line(&l, -3.0), "pipelines=2 ms=0");
    }

    #[test]
    fn pipeline_warmup_thread_ledger_resets_and_is_per_thread() {
        reset();
        note_created(Some("x"));
        assert_eq!(seal(), 1);
        note_created(Some("late"));
        let s = snapshot();
        assert_eq!((s.warm(), s.late().len()), (1, 1));
        // Another thread starts empty: parallel tests never see each other's builds.
        let other = std::thread::spawn(|| snapshot().total())
            .join()
            .expect("thread joins");
        assert_eq!(other, 0);
        reset();
        assert_eq!(snapshot().total(), 0);
        assert!(!snapshot().is_sealed());
    }

    /// The live presenter path builds its pipelines only through the counted wrappers. A new
    /// direct `device.create_*_pipeline` in one of these files would hide a build from the
    /// ledger, so it fails here.
    #[test]
    fn pipeline_warmup_every_live_path_build_goes_through_the_ledger() {
        let files: [(&str, &str); 13] = [
            ("fog.rs", include_str!("fog.rs")),
            ("decals.rs", include_str!("decals.rs")),
            ("gtao.rs", include_str!("gtao.rs")),
            ("ibl.rs", include_str!("ibl.rs")),
            ("material_layer.rs", include_str!("material_layer.rs")),
            ("particles_gpu.rs", include_str!("particles_gpu.rs")),
            ("sky.rs", include_str!("sky.rs")),
            ("submit.rs", include_str!("present/gpu_present/submit.rs")),
            ("live.rs", include_str!("present/gpu_present/live.rs")),
            (
                "sun_shadow.rs",
                include_str!("present/gpu_present/sun_shadow.rs"),
            ),
            (
                "gtao_live.rs",
                include_str!("present/gpu_present/gtao_live.rs"),
            ),
            (
                "bloom_dual.rs",
                include_str!("present/gpu_present/bloom_dual.rs"),
            ),
            (
                "presenter.rs",
                include_str!("present/gpu_present/presenter.rs"),
            ),
        ];
        for (name, src) in files {
            // Only the shipped code: the test modules build their own fixtures.
            let cut = src
                .match_indices("#[cfg(test)]")
                .map(|(at, tag)| (at, src[at + tag.len()..].trim_start()))
                .find(|(_, rest)| rest.starts_with("mod ") || rest.starts_with("#[cfg(feature"))
                .map_or(src.len(), |(at, _)| at);
            let shipped = &src[..cut];
            for call in [
                "device.create_render_pipeline(",
                "device.create_compute_pipeline(",
            ] {
                assert!(
                    !shipped.contains(call),
                    "{name} builds a pipeline with `{call}` outside the ledger: use pipeline_warmup::CountedPipelines"
                );
            }
        }
    }
}
