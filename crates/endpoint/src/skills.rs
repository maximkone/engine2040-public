//! Published skills: the repo's `skills/*/SKILL.md` files become MCP prompts.
//!
//! A skill is data, not a tool: listing it costs no tool budget, and a client
//! can read one procedure without loading every verb. The front matter is the
//! same `name` and `description` the coding agent already uses; the body is
//! the prompt text. Only shipped skills are served, and a malformed file is
//! skipped rather than taking the door down.

use rmcp::model::Prompt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Hard cap for one prompt body. A skill is a procedure, not a manual.
pub const MAX_BODY_BYTES: usize = 32_768;
/// Hard cap for a skill name.
pub const MAX_NAME_CHARS: usize = 64;

/// One parsed skill file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Shipped skills, sorted by name. Unknown or malformed files are skipped.
#[derive(Clone, Debug, Default)]
pub struct SkillRegistry {
    skills: Vec<SkillSpec>,
}

impl SkillRegistry {
    /// Read `<dir>/*/SKILL.md`. A missing directory is an empty registry.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        let mut skills = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return Self { skills };
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(skill) = parse_skill(&text) else {
                continue;
            };
            if skills
                .iter()
                .any(|seen: &SkillSpec| seen.name == skill.name)
            {
                continue;
            }
            skills.push(skill);
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Self { skills }
    }

    /// The repo's shipped skills, loaded once. Try the working directory first
    /// (the door runs from the repo root), then the compile-time source root.
    #[must_use]
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<SkillRegistry> = OnceLock::new();
        SHIPPED.get_or_init(|| {
            let cwd = std::env::current_dir().unwrap_or_default();
            let local = cwd.join("skills");
            let root = if local.is_dir() {
                local
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("skills")
            };
            Self::load(&root)
        })
    }

    /// True when at least one skill parsed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Number of published skills.
    #[must_use]
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Prompt descriptors for `prompts/list`.
    #[must_use]
    pub fn prompts(&self) -> Vec<Prompt> {
        self.skills
            .iter()
            .map(|skill| Prompt::new(skill.name.clone(), Some(skill.description.clone()), None))
            .collect()
    }

    /// Prompt body for `prompts/get`; `None` for an unknown name.
    #[must_use]
    pub fn body(&self, name: &str) -> Option<&str> {
        self.skills
            .iter()
            .find(|skill| skill.name == name)
            .map(|skill| skill.body.as_str())
    }
}

/// Parse one `SKILL.md`: YAML-style front matter with `name` and
/// `description`, then the markdown body. Returns `None` when the required
/// front matter or body is missing or over the cap.
#[must_use]
pub fn parse_skill(text: &str) -> Option<SkillSpec> {
    let normalized = text.replace("\r\n", "\n");
    let rest = normalized.strip_prefix("---\n")?;
    let (front, body) = rest.split_once("\n---")?;
    let body = body.trim_start_matches('\n').trim();
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return None;
    }
    let mut name = None;
    let mut description = None;
    for line in front.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(unquote(value));
        }
    }
    let name = name?;
    if !valid_name(&name) {
        return None;
    }
    let description = description.filter(|text| !text.is_empty())?;
    Some(SkillSpec {
        name,
        description,
        body: body.to_owned(),
    })
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_owned()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: ai-door\ndescription: Use when the door changes.\n---\n# Door\n\nRead this first.\n";

    #[test]
    fn parses_front_matter_and_body() {
        let skill = parse_skill(SAMPLE).expect("sample parses");
        assert_eq!(skill.name, "ai-door");
        assert_eq!(skill.description, "Use when the door changes.");
        assert!(skill.body.starts_with("# Door"));
    }

    #[test]
    fn parses_crlf_and_quoted_values() {
        let text = SAMPLE.replace('\n', "\r\n").replace(
            "description: Use when the door changes.",
            "description: \"Use when the door changes.\"",
        );
        let skill = parse_skill(&text).expect("CRLF parses");
        assert_eq!(skill.description, "Use when the door changes.");
    }

    #[test]
    fn rejects_missing_name_bad_name_and_empty_body() {
        assert!(parse_skill("---\ndescription: x\n---\nbody").is_none());
        assert!(parse_skill("---\nname: Bad Name\ndescription: x\n---\nbody").is_none());
        assert!(parse_skill("---\nname: ok\ndescription: x\n---\n").is_none());
        assert!(parse_skill("no front matter").is_none());
    }

    #[test]
    fn load_sorts_skips_duplicates_and_ignores_missing_dir() {
        let dir = std::env::temp_dir().join(format!("e2040-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("beta")).expect("beta dir");
        fs::create_dir_all(dir.join("alpha")).expect("alpha dir");
        fs::write(dir.join("beta/SKILL.md"), SAMPLE.replace("ai-door", "beta"))
            .expect("beta skill");
        fs::write(
            dir.join("alpha/SKILL.md"),
            SAMPLE.replace("ai-door", "alpha"),
        )
        .expect("alpha skill");
        let registry = SkillRegistry::load(&dir);
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.prompts()[0].name, "alpha");
        assert_eq!(registry.prompts()[1].name, "beta");
        assert!(registry
            .body("alpha")
            .is_some_and(|body| body.contains("Read this first")));
        assert!(!registry.is_empty());
        assert!(SkillRegistry::load(&dir.join("missing")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_repo_skills_are_published() {
        let registry = SkillRegistry::shipped();
        assert!(
            registry.len() >= 30,
            "expected the shipped skill set, got {}",
            registry.len()
        );
        assert!(
            registry.body("ai-door").is_some(),
            "ai-door must be published"
        );
        assert!(registry.body("not-a-skill").is_none());
        let prompts = registry.prompts();
        assert!(prompts.iter().all(|prompt| prompt.description.is_some()));
        assert!(prompts.iter().any(|prompt| prompt.name == "ai-door"));
    }
}
