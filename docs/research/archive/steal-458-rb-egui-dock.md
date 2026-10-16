# Steal 458 RB egui_dock — Shell-owned serializable dock-layout record (ED detail)

Goal (part + question): FOCUS Now pillar 3, Speed (edit-to-play iteration) —
how `egui_dock` structures a restorable multi-leaf dock layout (tree splits,
per-leaf active tabs, shell-authoritative open/close), and what transfers to
our open editor gap (`crates/editor/src/shell.rs` + `shell_ui.rs`: the dock
rebuilds from `open_panels` every launch with no owned layout record) without
vendoring a byte or touching the tick.
Scope: `docs/research/` owned file only
(`steal-458-rb-egui-dock.md` + one LADDER line); code read-only
(`crates/editor/src/shell.rs`, `shell_ui.rs`, `Cargo.toml`, `docs/plan/world.md`).
Proof (read live 2026-10-04, rev + license + paths below): repo record
(license.key `mit`, default branch main, 640 stars, pushed 2026-09-23),
pinned ref `cdcdb359af4b74f8cc7921091db7217f0759e4f7`, `LICENSE` MIT text
(blob `d6a85a6f`), `src/` + `src/dock_state/` listings (tree/, surface.rs,
window_state.rs); engine baseline via local Grep/Read. No 403/429 on any
host (wrapper searches returned empty, host NOT ended — known repo confirmed
via repository record instead, no repeated query).
Stop: 3 searches + reads over the 12 guide (stopped here per stop rule); no
test run here; the spike is proposed with numbers and a kill rule, not claimed.

Angles used (narrow): (1) `node tools/gh-search.mjs --type repos`
`egui_dock docking tabs egui` (EMPTY — no hits, no second search);
(2) `node tools/web-search.mjs --query "egui_dock docking tabs egui
Adanos020"` (EMPTY — 0 items, host not ended, no repeat); (3) repository
record search `egui_dock docking egui` (HIT: `anhosh/egui_dock` —
license.key mit, default_branch main, 640 stars, pushed 2026-09-23,
updated 2026-10-04); (4) root listing read live (ref `cdcdb359af`,
`LICENSE` blob `d6a85a6f`, `Cargo.toml`, `src/`, `examples/`);
(5) `LICENSE` full text read live (MIT, Copyright (c) 2022 Adam Gasior);
(6) `src/` listing read live (`lib.rs` blob `2c1100c1`, `style.rs`,
`utils.rs`, `widgets/`, `dock_state/`); (7) `src/dock_state/` listing read
live (`mod.rs` blob `d877c3ad`, `surface.rs` blob `39f29593`,
`surface_index.rs`, `translations.rs`, `tree/`, `window_state.rs` blob
`03b3fc10`, `error.rs`).

## Engine baseline (local grep/read, 2026-10-04)

- Wired: `Cargo.toml:203` (`egui_dock = "0.21"`, `Cargo.lock` pins it);
  `crates/editor/Cargo.toml:11,19,44` (NE1 dock shell, `ui` feature pulls
  `dep:egui_dock`, headless stays std-only without it);
  `docs/DEPENDENCIES.md:24` (egui_dock 0.21 MIT, editor `ui`, dock panels).
- Shell owns panels, not layout: `crates/editor/src/shell.rs:53-98`
  (`Panel` 7 variants — Hierarchy, Inspector, Log, Viewport, Findings,
  Tuning, Templates — `all()` in default dock order; "order of
  `open_panels` is the render order").
- Dock rebuilds every launch, first-leaf pile: `shell_ui.rs:48-51`
  (`ShellUi::new` = `DockState::new(shell.open_panels.clone())` — flat tab
  list, no splits); `shell_ui.rs:94-100` (per frame, missing tabs go to
  `push_to_first_leaf`); close reports back via `OnCloseResponse` so the
  shell stays authoritative (`shell_ui.rs:13-14,68-75`). The two methods
  read own no restore path: splits, fractions, per-leaf actives and
  floating windows are recomputed defaults each launch.
- Prior take HAVE IT, no dupe: `docs/plan/world.md:292-294` (egui_dock MIT
  read live 2026-09-26) + `world.md:324-326` (REPO18-1 HAVE IT — ED9 auto
  inspector ticked; this card owns only the layout-record half, never the
  inspector).

## CARD 1 — Shell-owned serializable dock-layout record over the egui_dock tree shape (ED detail)

- Source:
  `PORT(anhosh/egui_dock@cdcdb359af4b74f8cc7921091db7217f0759e4f7:src/dock_state)`
  (`mod.rs` blob `d877c3ad`, `surface.rs` blob `39f29593`,
  `window_state.rs` blob `03b3fc10`, `tree/` dir, all read live 2026-10-04
  via contents listing at the pinned ref) + `src/lib.rs` blob `2c1100c1`.
- License: **MIT** — repo-record `license.key` mit read live 2026-10-04
  plus full `LICENSE` MIT text (Copyright 2022 Adam Gasior, blob
  `d6a85a6f`) read live at the pinned rev; on the `deny.toml` allow-list
  (`"MIT"`). Reimplement shape only, zero bytes vendored, attribution in
  the file header. (Repo-record `spdx_id` field itself was truncated in
  transit — key + full license text both say MIT; re-verify `spdx_id`
  before any vendoring, though none is proposed.)
- What it does — HOW THEY HANDLE it, in own words: (a) the live dock is a
  tree of split nodes with fractional sizes plus leaf tab lists with one
  active tab each (`tree/` + `surface.rs` shape), floating windows tracked
  separately (`window_state.rs`); (b) the state object is the single thing
  the draw call borrows each frame (`DockArea::new(dock).show_inside`,
  the exact call our `shell_ui.rs:125` already makes), so layout and
  drawing never diverge; (c) tab open/close round-trips through the owner
  (`TabViewer` + `OnCloseResponse` — the shape our shell already mirrors
  with `open_panel`/`close_panel` authority).
- Take: a Shell-owned layout record (named split tree over `Panel` ids:
  node fractions + leaf tab order + active per leaf), serialized beside
  `open_panels`, restored into the `DockState` at `ShellUi::new` and
  re-synced per frame with the existing first-leaf + close-report loop;
  reimplemented std-only over our `Panel` enum, no egui types in the
  record, no new dependency edge.
- Do not take: any donor source bytes, `style.rs` theming, the example
  apps, a `egui_dock` version bump, floating-window support (record it as
  closed; follow-up card only if a spike needs it), any write path around
  `Shell` batch entrypoints.
- Spike (proposed, not run — no builds per researcher contract):
  headless `Shell` fixture with all 7 panels and a 3-leaf target record
  (Viewport 70% left, right column Hierarchy/Inspector/Findings/Tuning/
  Templates, bottom Log): flat `DockState::new` baseline vs record
  restore; metrics tab placement (0 misplaced) + record bytes x2
  identical + `headless_dock_frame_smoke` green; adverse input: record
  naming a panel the shell build closed — restore must drop it, never
  resurrect.
- Kill: one misplaced tab, one record-byte drift x2, one red headless
  dock test, one write bypassing `Shell` batch entrypoints, one
  resurrected closed panel, or any golden `d57b5c8ae3096614` drift —
  then the card closes REJECTED and the flat first-leaf loop stands.
- Integration home: ED detail, no new row (REPO18-1 HAVE IT; home
  `crates/editor/src/shell.rs` layout record + `shell_ui.rs`
  restore/sync beside the frozen `open_panels` order; consumer ED rows
  via the editor plan owner). Pillar: FOCUS Now 3 (Speed —
  edit-to-play iteration: the workspace survives relaunch). EMPIRE
  outcome: AI builds inside (the editor is the AI workbench the crew
  dogfoods through the MCP door daily).

## EVIDENCE row proposal (builder appends to `docs/plan/world.md` §EB-73 list)

| `anhosh/egui_dock` | MIT (repo-record license.key mit + `LICENSE` MIT text blob `d6a85a6f` read live 2026-10-04 at `cdcdb359af`, main, 640 stars) | split-tree + per-leaf-active-tab layout shape for a Shell-owned serializable dock record, reimplemented std-only over `Panel` (`PORT(anhosh/egui_dock@cdcdb359:src/dock_state)`, no bytes vendored) | donor sources, style.rs, examples, version bump, floating windows |

## Triage

- Answer: one CARD (Shell-owned dock-layout record), zero rejects.
- Evidence: `Cargo.toml:203` (0.21 pin), `crates/editor/src/shell.rs:53-98`
  (7 panels, order = render order), `shell_ui.rs:48-51,94-100,125`
  (flat rebuild + first-leaf pile, no restore path in methods read),
  `docs/plan/world.md:292-294,324-326` (MIT verified 2026-09-26, REPO18-1
  HAVE IT bounds the dupe), donor ref/blobs/listings read live above.
- Existing owner: editor crate (`shell.rs` + `shell_ui.rs`, ED rows).
- Missing coverage: the layout record itself (no serializable split
  state on disk per methods read).
- Smallest packet: EXPERIMENT spike running the 3-leaf restore fixture
  with the numbers above, then a CODE packet for `shell.rs` record +
  `shell_ui.rs` restore only on ADOPT.
- Acceptance: `cargo test -p engine-editor --lib shell_ui` green incl.
  placement + bytes-x2 + closed-panel-drop fixtures.
- Unknowns: exact record schema for fractions (spike decides; f32
  outside tick is fine — record lives in the editor host, never the
  sim); floating windows stay closed by kill rule.
