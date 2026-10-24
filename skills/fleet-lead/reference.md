# fleet-lead reference

Detail behind `skills/fleet-lead/SKILL.md`. Read once before the first spawn, then when a case comes up.

## Speed rules

- 45-minute box per row. An agent past it with no commit gets "land what is proven now, report".
- Flash agents end turns with "waiting on the runner": steer them to run
  `node tools/gh-check.mjs --wait <wip id> --budget 900` and not end the reply until the report is final.
- Blocked on a dirty shared file: tell them to take their own hunk with `--pick` (brief.md, Edit).
- Context bloat: `cacheM` (millions of cached tokens) grows every turn; past about 30M a child gets slow, so start
  a fresh child for the next big row instead of steering the old one.

## Master red from a half commit

- A commit that uses a symbol defined only in an uncommitted file breaks that crate for every agent.
- Find it with `git grep -n <symbol> HEAD -- crates` against the working tree. The owner of the missing file
  lands it; otherwise the lead lands it with a proof.
- Real case (2026-10-24): `PLAY_JOB_ENDED` in `crates/observe`, `note_play_end` in
  `crates/endpoint/src/log_tools.rs`.

## Judge and tick

- Judge: `git show --stat <sha>`; the run id and test count are above 0; the files are the agent's row;
  `ACCEPT LEFT:` is honest (a frame capture still owed stays owed).
- `node skills/fleet-lead/scripts/tick.mjs <ID> x "Landed <sha7> ..."` for a row that passed its Accept, or
  `node skills/fleet-lead/scripts/tick.mjs <ID> p "<progress>"` for a dated note.
- Then `node tools/workspace-check.mjs`, then commit the shard by path.

## Hunter rows

- Give each hunter one id range and one rows file, in the shape of `docs/SPRINT-39-steal-wave.md`.
- Merge with `node skills/fleet-lead/scripts/merge-rows.mjs <config.json> <rows dir> <shard.md>`; the config drops
  duplicates and overrides `needs:` lines. Then add the new ids to the FOCUS list.
- Read every hunter row before merging: a row with no proof grep of today's code is a guess, not a row.

## Archive

- No tool archives or deletes a session: the openchamber tool cannot, and OpenCode v2 `PATCH /api/session/{id}`
  takes only title, metadata and permissions.
- Rename finished ones with `tools.opencode.session_rename` to "DONE · archive · <what>".
- Tell the owner: project menu, "Archive all sessions" archives everything except pinned, running and In work
  sessions. Subagents go with their parent.

## The CLI and processes

- The OpenCode CLI (`C:\opencode\opencode.exe api get /api/info`) reaches the background service, not the
  OpenChamber-managed server that runs the agents (that one needs a password the agent env does not carry), so
  do not interrupt or compact through it.
- `Get-Process | Sort-Object WorkingSet64 -Descending` shows what eats RAM.
- Long-lived and fine: git fsmonitor daemon, OpenChamber, opencode serve. Stop only processes you started; the
  owner's (yt-dlp, a Blender the owner opened) are theirs.

## Secrets

- Asset API keys live in Windows user env vars (SKETCHFAB_TOKEN, FREESOUND_API_KEY); read with
  `[Environment]::GetEnvironmentVariable('<name>','User')`.
- Never print a key, never put one in a prompt, a file or a commit.
