//! `asset` pack: the engine's code and content references, and generated props.
//!
//! Holds the reverse callers graph and asset reference lookup, plus the
//! generated-prop path: `asset.generate` explains the center command that runs an
//! image-to-3D job on the free Kaggle lane (no runner lives in the engine), and
//! `asset.intake` (feature `intake`) cooks the GLB that job made into a game-ready
//! prop: licence gate, repair, at most 5,000 triangles, LOD chain, collision hull,
//! one E204BNDL bundle, a provenance row and a cache, through
//! `engine_assets::gen_intake`. `import.fbx` lands here as its row closes.
//! Errors: [`McpToolError`] with the intake's own code (`license_refused`,
//! `model_restricted`, `conditions_unaccepted`, `bad_provenance`, `import_failed`,
//! `cook_failed`, `over_budget`, ...), the cause and one next step.

use super::{PackDef, PackError, VerbSpec};
use crate::insight::mcp_tools::{
    AssetRefsTool, CodeCallersTool, TOOL_ASSET_REFS, TOOL_CODE_CALLERS,
};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "asset";
/// Verb: explain how to generate a prop (the runner lives in center).
pub const TOOL_ASSET_GENERATE: &str = "asset.generate";
/// Verb: cook a generated GLB into a game-ready prop with provenance.
pub const TOOL_ASSET_INTAKE: &str = "asset.intake";

/// The center command that runs the generator on the free GPU lane.
pub const GENERATE_COMMAND: &str =
    "node kaggle.mjs run mesh --repo engine2040 --in <image or folder> --out <folder> [--n N] [--faces 60000]  (run in the center repo)";

/// Args for `asset.generate`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GenerateArgs {
    /// What the prop should be (a picture of it comes first: use the center
    /// `run image` job for a text prompt).
    #[serde(default)]
    pub prompt: Option<String>,
}

/// Result of `asset.generate`: no run happens, the steps are returned.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GenerateOutput {
    /// Always false: the engine has no generator runner.
    pub runs_here: bool,
    /// True when this door was built with `asset.intake`.
    pub intake_built: bool,
    /// The steps in order: picture, mesh job, intake.
    pub steps: Vec<String>,
    /// Licences the chain was read against.
    pub licenses: Vec<String>,
}

fn generate(_server: &McpServer, args: &GenerateArgs) -> Result<GenerateOutput, McpToolError> {
    let picture = match &args.prompt {
        Some(prompt) => format!(
            "node kaggle.mjs run image --repo engine2040 --out <folder> --prompt {prompt:?}  (run in the center repo)"
        ),
        None => "make or pick one clear picture of the object on a plain background".to_owned(),
    };
    Ok(GenerateOutput {
        runs_here: false,
        intake_built: cfg!(feature = "intake"),
        steps: vec![
            picture,
            GENERATE_COMMAND.to_owned(),
            "asset.intake {name, glb_path, lane_json: <folder>/kaggle-lane.json, accept_conditions: [...]}".to_owned(),
        ],
        licenses: vec![
            "BiRefNet cutout: MIT".to_owned(),
            "TripoSG shape: MIT weights; its transformer code carries Tencent Hunyuan Community terms (EU excluded): intake asks you to accept that condition by name".to_owned(),
            "refused outright: briaai RMBG (non-commercial), diso (CC BY-NC), Hunyuan3D, any non-SPDX or non-allow-listed licence".to_owned(),
        ],
    })
}

crate::e2_tool!(
    /// `asset.generate`: explain the generator command; nothing runs here.
    GenerateTool,
    name = TOOL_ASSET_GENERATE,
    description = "How to make a prop from a picture: returns the free center GPU command (BiRefNet cutout, TripoSG shape) and the intake step. Nothing runs in the engine.",
    readonly = true,
    args = GenerateArgs,
    output = GenerateOutput,
    invoke = generate
);

#[cfg(feature = "intake")]
mod intake_verb;
#[cfg(feature = "intake")]
pub use intake_verb::{IntakeArgs, IntakeOutputDoc, IntakeTool, ModelArg};

/// The asset pack: callers, references, the generate explainer and (feature
/// `intake`) the intake, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "code and content references: reverse callers, which code and scenes touch one \
         asset; generated props: asset.generate explains the free GPU command, asset.intake \
         cooks its GLB to at most 5,000 triangles with a hull, provenance and a cache",
        false,
    );
    pack.add(VerbSpec::of::<CodeCallersTool>(
        TOOL_CODE_CALLERS,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<AssetRefsTool>(
        TOOL_ASSET_REFS,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<GenerateTool>(
        TOOL_ASSET_GENERATE,
        PACK,
        CapabilityCard::Observe,
    )?);
    #[cfg(feature = "intake")]
    pack.add(VerbSpec::of::<IntakeTool>(
        TOOL_ASSET_INTAKE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    #[test]
    fn generate_explains_the_center_command_and_runs_nothing() {
        let server = McpServer::new(61, Capability::unrestricted(61));
        let out = GenerateTool::invoke(
            &server,
            GenerateArgs {
                prompt: Some("a barrel".to_owned()),
            },
        )
        .expect("explains");
        assert!(!out.runs_here);
        assert_eq!(out.intake_built, cfg!(feature = "intake"));
        assert!(out.steps[0].contains("kaggle.mjs run image") && out.steps[0].contains("a barrel"));
        assert!(out.steps[1].contains("kaggle.mjs run mesh"));
        assert!(out.steps[2].contains("asset.intake"));
        assert!(out.licenses.iter().any(|l| l.contains("briaai")));
    }

    #[test]
    fn pack_lists_its_verbs_with_annotations() {
        let pack = pack().expect("registers");
        let names = pack.verb_names();
        assert!(names.contains(&TOOL_ASSET_GENERATE));
        assert_eq!(names.contains(&TOOL_ASSET_INTAKE), cfg!(feature = "intake"));
        for verb in pack.verbs() {
            assert_ne!(verb.card(), CapabilityCard::Deny);
            assert!(
                verb.model().annotations.is_some(),
                "{} is annotated",
                verb.name()
            );
        }
        assert!(super::super::PackRegistry::shipped().errors().is_empty());
    }
}
