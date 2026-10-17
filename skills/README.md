# skills/

Task skills, one folder per job. Each folder holds one `SKILL.md` (frontmatter `name` = folder name, `description` starting "Use when", under 300 characters) and, for long tables, a `reference.md` or `references/`.
Open `skills/SKILLS.md` first for the map, then read the one skill whose trigger matches.
Rules for a skill file (checked by `tools/skills-index.test.mjs`): `SKILL.md` at most 250 lines, no `path:line` cites (cite `path` or `path::symbol`), every cited repo path exists or the same line says `cut` (removed file) or `git-ignored` (output dir).
Add or rename a skill: update `skills/SKILLS.md` in the same commit.
