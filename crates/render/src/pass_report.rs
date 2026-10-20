//! pass_report: what every render pass reads, writes and did.
//!
//! A frame's passes are declared as data before they run: each names the
//! resources it reads and writes and whether it ran, was culled or failed.
//! [`Plan::validate`] walks the list in order with a set of available
//! resources seeded from the plan's external inputs; a pass whose read has no
//! earlier writer fails with the pass and the resource named, so a silently
//! dropped pass can never leave the frame "merely wrong".
//!
//! The report is a small, fixed JSON in declaration order: no map, no clock,
//! so the same plan prints the same bytes twice. Presentation only; the sim
//! never reads it.
//!
//! Donor shape: uinosoft/zenfg render graph, Themaister/Granite
//! `render_graph.cpp` validate_passes (method only, zero bytes vendored).

use std::fmt;

/// What one pass did this frame. A culled or failed pass publishes none of
/// its writes, so a later read of them fails validation and names the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ran {
    /// The pass ran and its writes are available downstream.
    Yes,
    /// The pass did not run; the reason is printed in the report.
    Culled(&'static str),
    /// The pass was attempted and failed; the reason is printed.
    Failed(&'static str),
}

/// One declared pass: its name, the resources it reads and writes, and what
/// happened when the frame was built.
#[derive(Clone, Copy, Debug)]
pub struct Pass {
    /// Pass name as printed in the report and in errors.
    pub name: &'static str,
    /// Resources read; every one must be a plan input or an earlier write.
    pub reads: &'static [&'static str],
    /// Resources published downstream when the pass ran.
    pub writes: &'static [&'static str],
    /// Ran, culled or failed with a reason.
    pub run: Ran,
}

/// A frame's pass list plus the resources that exist before any pass runs
/// (the mesh and camera inputs of the forward pass, for example).
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// Resources available before the first pass.
    pub inputs: &'static [&'static str],
    /// Passes in the order they would run.
    pub passes: &'static [Pass],
}

/// Why a plan cannot be reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// A pass reads a resource no earlier pass wrote and the plan does not
    /// declare as an input.
    MissingWriter {
        /// The reading pass.
        pass: &'static str,
        /// The resource with no writer.
        resource: &'static str,
    },
    /// Two passes publish the same resource in one frame.
    DuplicateWriter {
        /// The second writer.
        pass: &'static str,
        /// The resource written twice.
        resource: &'static str,
    },
    /// A pass reads a resource whose only writer was culled or failed.
    WriterDidNotRun {
        /// The reading pass.
        pass: &'static str,
        /// The resource with no live writer.
        resource: &'static str,
        /// The writer pass that did not run.
        writer: &'static str,
    },
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWriter { pass, resource } => {
                write!(f, "pass {pass} reads {resource}, but no pass writes it")
            }
            Self::DuplicateWriter { pass, resource } => {
                write!(f, "pass {pass} writes {resource}, which is already written")
            }
            Self::WriterDidNotRun {
                pass,
                resource,
                writer,
            } => write!(
                f,
                "pass {pass} reads {resource}, but its writer {writer} did not run"
            ),
        }
    }
}

impl std::error::Error for ReportError {}

impl Ran {
    /// The report word: `ran`, `culled` or `failed`.
    pub fn word(self) -> &'static str {
        match self {
            Self::Yes => "ran",
            Self::Culled(_) => "culled",
            Self::Failed(_) => "failed",
        }
    }

    /// The reason printed beside a culled or failed pass; empty when ran.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Yes => "",
            Self::Culled(reason) | Self::Failed(reason) => reason,
        }
    }
}

impl Plan {
    /// Walk the passes in order. Every enabled pass must find each read among
    /// the plan inputs or an earlier pass's writes; every resource has at most
    /// one live writer. The first broken read is the error.
    pub fn validate(&self) -> Result<(), ReportError> {
        // Live resources: available before the pass that reads them. `None`
        // marks a plan input (no pass wrote it); `Some` names the pass that
        // ran and published it.
        let mut live: Vec<(&'static str, Option<&'static str>)> =
            self.inputs.iter().map(|input| (*input, None)).collect();
        // Resources a culled or failed pass would have written: a later read
        // names that writer instead of reporting a missing one.
        let mut dead: Vec<(&'static str, &'static str)> = Vec::new();
        for pass in self.passes {
            for resource in pass.reads {
                if live.iter().any(|(name, _)| name == resource) {
                    continue;
                }
                if let Some((_, writer)) = dead.iter().find(|(name, _)| name == resource) {
                    return Err(ReportError::WriterDidNotRun {
                        pass: pass.name,
                        resource,
                        writer,
                    });
                }
                return Err(ReportError::MissingWriter {
                    pass: pass.name,
                    resource,
                });
            }
            match pass.run {
                Ran::Yes => {
                    for resource in pass.writes {
                        if live.iter().any(|(name, _)| name == resource) {
                            return Err(ReportError::DuplicateWriter {
                                pass: pass.name,
                                resource,
                            });
                        }
                        live.push((resource, Some(pass.name)));
                    }
                }
                Ran::Culled(_) | Ran::Failed(_) => {
                    for resource in pass.writes {
                        if !live.iter().any(|(name, _)| name == resource)
                            && !dead.iter().any(|(name, _)| name == resource)
                        {
                            dead.push((resource, pass.name));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The report: one fixed JSON object with the passes in declaration order
    /// and a state word plus reason for each. Byte-equal for the same plan.
    pub fn report_json(&self) -> Result<String, ReportError> {
        self.validate()?;
        let mut out = String::with_capacity(64 + self.passes.len() * 64);
        out.push_str("{\"passes\":[");
        for (i, pass) in self.passes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":\"");
            out.push_str(pass.name);
            out.push_str("\",\"state\":\"");
            out.push_str(pass.run.word());
            out.push('"');
            if !pass.run.reason().is_empty() {
                out.push_str(",\"reason\":\"");
                out.push_str(pass.run.reason());
                out.push('"');
            }
            out.push('}');
        }
        out.push_str("]}");
        Ok(out)
    }

    /// One line naming every pass and its state, in order: the human half of
    /// [`Plan::report_json`].
    pub fn report_line(&self) -> Result<String, ReportError> {
        self.validate()?;
        let mut out = String::with_capacity(32 + self.passes.len() * 24);
        out.push_str("passes: ");
        for (i, pass) in self.passes.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(pass.name);
            out.push('=');
            out.push_str(pass.run.word());
            if !pass.run.reason().is_empty() {
                out.push('(');
                out.push_str(pass.run.reason());
                out.push(')');
            }
        }
        Ok(out)
    }
}

impl Pass {
    /// A pass that ran.
    pub const fn ran(
        name: &'static str,
        reads: &'static [&'static str],
        writes: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            reads,
            writes,
            run: Ran::Yes,
        }
    }

    /// A pass that was culled with a reason.
    pub const fn culled(
        name: &'static str,
        reads: &'static [&'static str],
        writes: &'static [&'static str],
        reason: &'static str,
    ) -> Self {
        Self {
            name,
            reads,
            writes,
            run: Ran::Culled(reason),
        }
    }

    /// A pass that was attempted and failed with a reason.
    pub const fn failed(
        name: &'static str,
        reads: &'static [&'static str],
        writes: &'static [&'static str],
        reason: &'static str,
    ) -> Self {
        Self {
            name,
            reads,
            writes,
            run: Ran::Failed(reason),
        }
    }
}

/// The arena's pass plan: the six passes the window runs today, in order,
/// plus fog declared and culled because the live window keeps it off (fog is
/// a fragment function, not a pass, in this build). The forward pass seeds
/// `depth` and `hdr`; sky and IBL feed the frame, shadow and GTAO read the
/// stored depth, bloom and TAA read the HDR target.
pub const ARENA_PLAN: Plan = Plan {
    inputs: &["mesh", "camera"],
    passes: &[
        Pass::ran("forward", &["mesh", "camera"], &["depth", "hdr"]),
        Pass::ran("sky", &["hdr"], &["sky"]),
        Pass::ran("ibl", &["sky"], &["ibl"]),
        Pass::ran("shadow", &["depth"], &["shadow"]),
        Pass::ran("gtao", &["depth"], &["ao"]),
        Pass::ran("bloom", &["hdr"], &["bloom"]),
        Pass::ran("taa", &["hdr"], &["history"]),
        Pass::culled("fog", &["depth"], &["fog"], "fog is a fragment function"),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arena_plan_names_six_passes_ran_and_culls_fog() {
        let json = ARENA_PLAN.report_json().expect("the arena plan is valid");
        for name in ["forward", "sky", "ibl", "shadow", "gtao", "bloom", "taa"] {
            assert!(
                json.contains(&format!("{{\"name\":\"{name}\",\"state\":\"ran\"}}")),
                "{name} ran: {json}"
            );
        }
        assert!(
            json.contains(
                "{\"name\":\"fog\",\"state\":\"culled\",\"reason\":\"fog is a fragment function\"}"
            ),
            "fog is culled with its reason: {json}"
        );
        let line = ARENA_PLAN.report_line().expect("line");
        assert!(line.starts_with("passes: forward=ran, sky=ran"), "{line}");
        assert!(
            line.ends_with("fog=culled(fog is a fragment function)"),
            "{line}"
        );
    }

    #[test]
    fn a_read_with_no_writer_names_the_pass_and_the_texture() {
        const PLAN: Plan = Plan {
            inputs: &[],
            passes: &[Pass::ran("gtao", &["depth"], &["ao"])],
        };
        let err = PLAN.report_json().expect_err("no writer for depth");
        assert_eq!(
            err,
            ReportError::MissingWriter {
                pass: "gtao",
                resource: "depth"
            }
        );
        assert_eq!(
            err.to_string(),
            "pass gtao reads depth, but no pass writes it",
            "the message names both"
        );
    }

    #[test]
    fn a_culled_writer_fails_its_reader_with_the_writer_named() {
        const PLAN: Plan = Plan {
            inputs: &[],
            passes: &[
                Pass::culled("ibl", &[], &["ibl"], "no probe"),
                Pass::ran("forward", &["ibl"], &["hdr"]),
            ],
        };
        let err = PLAN.report_json().expect_err("ibl never published");
        assert_eq!(
            err,
            ReportError::WriterDidNotRun {
                pass: "forward",
                resource: "ibl",
                writer: "ibl"
            }
        );
        assert_eq!(
            err.to_string(),
            "pass forward reads ibl, but its writer ibl did not run"
        );
    }

    #[test]
    fn the_same_plan_prints_the_same_json_twice() {
        let a = ARENA_PLAN.report_json().expect("first");
        let b = ARENA_PLAN.report_json().expect("second");
        assert_eq!(a, b, "byte-equal twice");
        assert!(a.starts_with("{\"passes\":[{\"name\":\"forward\""), "{a}");
    }

    #[test]
    fn a_failed_pass_prints_failed_with_its_reason() {
        const PLAN: Plan = Plan {
            inputs: &["hdr"],
            passes: &[Pass::failed("bloom", &["hdr"], &["bloom"], "no hdr view")],
        };
        let json = PLAN.report_json().expect("a failed pass still reports");
        assert!(
            json.contains("{\"name\":\"bloom\",\"state\":\"failed\",\"reason\":\"no hdr view\"}"),
            "{json}"
        );
        assert_eq!(
            PLAN.report_line().expect("line"),
            "passes: bloom=failed(no hdr view)"
        );
    }

    #[test]
    fn two_live_writers_of_one_resource_fail() {
        const PLAN: Plan = Plan {
            inputs: &["mesh"],
            passes: &[
                Pass::ran("forward", &["mesh"], &["hdr"]),
                Pass::ran("sky", &["mesh"], &["hdr"]),
            ],
        };
        let err = PLAN.validate().expect_err("hdr has two writers");
        assert_eq!(
            err,
            ReportError::DuplicateWriter {
                pass: "sky",
                resource: "hdr"
            }
        );
    }
}
