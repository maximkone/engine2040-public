# Sources

This skill is a derived work of the ggrs documentation, changelog and manifest and of the GGPO developer guide. The rules are condensed and reworded; a few short code lines are kept from the ggrs pages.

- Origin 1: https://github.com/gschup/ggrs at commit e97e3d2416cc68af2d2876d41180d950c2939b6e (branch main): the docs folder, CHANGELOG.md and Cargo.toml.
- Version: Cargo.toml at that commit says `version = "0.13.0"` (and `rust-version = "1.87"`) and CHANGELOG.md has a `## 0.13.0` section, so the skill targets ggrs 0.13 and tells you to write `ggrs = "0.13"`. docs/setup.md at the same commit still prints `ggrs = "0.12"`. That line is not the crate version, it lags the manifest: the 0.13.0 changelog section itself records the docs change that set the guide to 0.12, and the guide was not moved to 0.13 when the manifest was. The other docs pages already describe 0.13 behaviour (changing input delay mid-session, `advance_frame_with_wait`, the `NotEnoughData` handling, `with_num_players` re-validating handles). The version of a library comes from its manifest at the pinned commit, not from the prose of its docs. The `## Unreleased` section of the changelog lists changes made after 0.13.0 (late spectators, a spectator rollback fix); the skill marks them as such. An earlier text of this skill followed the guide's line and said 0.12; it was corrected on 2026-10-11.
- Licence 1: dual MIT OR Apache-2.0 ("GGRS is dual-licensed under either" in README.md, with LICENSE-MIT and LICENSE-APACHE; Cargo.toml says `license = "MIT OR Apache-2.0"`), verified 2026-10-11 from https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/LICENSE . This skill takes the MIT option. The repository has no separate licence for its docs folder.
- Origin 2: https://github.com/pond3r/ggpo at commit 7ddadef8546a7d99ff0b3530c6056bc8ee4b9c0a (branch master), the file doc/DeveloperGuide.md.
- Licence 2: MIT, "Copyright (c) 2009-2019 GroundStorm Studios, LLC." (https://github.com/pond3r/ggpo/blob/7ddadef8546a7d99ff0b3530c6056bc8ee4b9c0a/LICENSE ; README.md: "GGPO is available under The MIT License"), verified 2026-10-11.
- Attribution: rules and short code derived from the ggrs docs and changelog by the ggrs authors (MIT OR Apache-2.0) and from the GGPO developer guide by GroundStorm Studios, LLC. (MIT). The MIT permission notices stay at the licence links above and in `THIRD_PARTY_NOTICES.md`. Changes: condensed into rules, reworded, short code lines kept. No endorsement by the authors is implied.
- Supply copies: the docs pages below are kept in git-ignored `work/ggrs-rollback-rules/src/` and `work/ggpo-guide/src/`, each with a `MANIFEST.json` that holds the pinned commit and a size per file; CHANGELOG.md and Cargo.toml at the same pin are kept in git-ignored `work/ggrs-rollback-rules/extra/` with a `PIN.txt`. `tests/test_ggrs_rollback_rules.py` checks every API name a rule cites against the page the rule names (live test).

## Pages used (all at the pinned commits)

- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/Cargo.toml
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/CHANGELOG.md
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/docs/setup.md
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/docs/sessions.md
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/docs/main-loop.md
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/docs/requests-and-events.md
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/docs/sparse-saving.md
- https://github.com/gschup/ggrs/blob/e97e3d2416cc68af2d2876d41180d950c2939b6e/docs/time-synchronization.md
- https://github.com/pond3r/ggpo/blob/7ddadef8546a7d99ff0b3530c6056bc8ee4b9c0a/doc/DeveloperGuide.md

## Measured

The skill was built because a bare-model baseline probe on the 12-task sheet `evals/ggrs-rollback-rules_trials.jsonl` failed the topic (see `research/BASELINE-PROBES.md`). Its lift with and without the skill, from live model runs, is in `references/trial-proof.json`; the sheet and the skill say 0.13 since the 2026-10-11 correction (the ledger lists what changed).
