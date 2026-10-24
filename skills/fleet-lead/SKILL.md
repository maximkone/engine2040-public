---
name: fleet-lead
description: Use when you lead helper agents in OpenChamber or OpenCode — spawn children, queue rows, watch stuck or slow agents, recover lost reports, judge and tick rows, merge hunter rows, archive finished sessions.
---

# fleet-lead

One lead runs 5-20 helper agents on one shared tree. The lead plans, splits work by file, judges every report
and ticks rows in batches. A helper codes one row from `skills/fleet-lead/brief.md`; a hunter is read-only and
writes rows from `skills/fleet-lead/hunt.md`. Merge, archive, processes, secrets and the master-red drill are
in `skills/fleet-lead/reference.md`; read it once before the first spawn.

## Shape

- You own the order of work, the row split and every board edit. A helper owns one row and one file group.
- Split by file, not by subsystem: two agents on one hot file cost more than they save.
- Judge each report against the row's Accept; a pass needs a run id and a test count above 0. Tick in batches.
- Tell the owner what landed and what is stuck in a few lines, often.

## Spawn

- Only the subagent tool, with `background: true`. Never the openchamber `session.create` action: it makes a
  sidebar session the owner must archive by hand.
- Prompt: read brief.md (hunt.md too for a hunter), the row id, the files to avoid (other agents' files), the
  report shape. Nothing else; the brief is the contract.
- 5-20 coders at once, one file group each. A big shared file (`crates/game/src/bin/arena.rs`,
  `crates/game/src/fighter_look.rs`, `crates/render/src/quality.rs`, `crates/game/src/lib.rs`) gets
  hunk-sized edits only: one field, one `mod` line, one call.

## Talk

- `tools.openchamber({action:"session.send", parameters:{sessionId, prompt}})`.
- To a BUSY session the message is a steer: OpenCode's `session_inbox` delivers it into the running turn at the
  next step, not after it. That is how a hung or drifting agent gets corrected.
- To an idle child it starts a new turn, and that turn's end comes back as a subagent notification.
- A session that is not your child needs `returnResult: true` or its answer never reaches you.
- One message per agent. Never broadcast to read-only hunters.

## Watch (every 15-20 minutes)

- `python -I skills/fleet-lead/scripts/fleet.py fleet <lead session id> --extra <file of other session ids>`;
  `live <id> [n]` for the last tool calls; `report <id>...` for the last full text.
- Why: the openchamber `session.messages` action shows finished text only, so a 15-minute gh-check looks dead
  there. The script reads OpenCode v2's DB read-only (tables `session_v2`, `session_message`, `session_inbox` in
  `~/.local/share/opencode/opencode.db`); never write to it.
- `LONG` = a tool call over 30 min (an engine-game test run of 15-20 min is normal; a Start-Sleep loop or a local
  cargo is not). `SILENT` = busy but nothing new for 15 min: the model call may hang, send a short steer.
- `idle` = the turn ended. If its report did not reach you (a report that lands during a context summary is
  lost), read it with `report`.
