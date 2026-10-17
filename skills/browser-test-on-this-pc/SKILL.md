---
name: browser-test-on-this-pc
description: "Use when you changed the web build and must see it run in a real WebGPU browser: open the GitHub Pages URL in the built-in browser pane (Claude desktop app), read the golden, report and frame lines, and check the page is the commit you pushed."
---

# Test the web build in the built-in browser

The Claude desktop app has a browser pane with tools named `mcp__Claude_Browser__*` (skill `anthropic-skills:built-in-browser`). It uses this PC's real GPU, so the page gets a real adapter (2026-10-17: vendor nvidia, ampere). The headless Chrome proof (skill `web-wasm`, section 5) uses a software adapter on a runner. Use both: the runner proves the build, the pane proves it on real hardware.

It cannot open `file://` or a `localhost` server your shell started. So test the PUBLISHED page, not `dist/web` on disk. (For a local `dist/web`, use `node tools/web-browse.mjs`.)

## 1. Make sure the page is your build

1. Push the committed tree to the mirror: `node tools/public-export.mjs` then `node tools/public-export.mjs --commit --push` (skill `github-check`). Pages deploys only on a push to mirror `main`.
2. `gh run list -R maximkone/engine2040-public --workflow pages.yml -L 3`: the top row must be `completed success` and its title `snapshot <date> of <your sha7>`. `in_progress` means wait. `failure`: `gh run view <id> -R maximkone/engine2040-public --log-failed` after it ends.
3. The mirror head: `gh api repos/maximkone/engine2040-public/commits/main --jq '.sha[0:12]'`. The page prints the same 12 characters in `data-commit` (step 3). If they differ, you are looking at an old deploy: reload with a new query (`?r=2`) or wait.

## 2. Open and read

1. If the browser tools are deferred, load them in ONE call: ToolSearch with query `mcp__Claude_Browser__` and `max_results: 64`.
2. `navigate` to `https://maximkone.github.io/engine2040-public/` (handbook `/docs/`, rustdoc `/docs/api/`; `?square` opens the plain WebGPU shell instead of the arena).
3. `get_page_text`. Read text, not a screenshot. Then `javascript_tool` with: `JSON.stringify({state:document.body.dataset.state,golden:document.body.dataset.golden,commit:document.body.dataset.commit,gen:document.body.dataset.generation,gpu:!!navigator.gpu})`.

## 3. What the lines mean (page source `crates/web/index.html`, `crates/web/src/shell.rs`)

| line | good | bad |
|---|---|---|
| `frame N \| .. ms \| cpu .. ms` | N grows between two reads | N stays 1: the pane is hidden (section 4) or frames stopped |
| `golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK <n> ms` | `data-golden` is `ok` | `golden MISMATCH got ... expected ...` (`data-golden=mismatch`): the wasm sim diverged, skill `determinism-debug` |
| `tape 600 ticks in .. ms, .. ms per tick` | present | missing: golden did not run |
| first JSON line | `"ok":true`, `backend` `BrowserWebGpu`, adapter vendor, `deviceGeneration` 1 | `"ok":false` with a `reason` (`no-webgpu`, `load-failed`) and `data-state=fatal` with the red panel |
| `arena ready in .. ms` | present | `arena: starting` forever: init failed, read the console |
| `adapter= .. sky=..ms layers=.. ibl=.. fog=.. gtao=.. shadow=.. validation_errors=0` | `validation_errors=0`; passes with 0.000 ms are not drawn yet | any validation error |
| last JSON line | `"frames"` rising, `frameErrors`, `validationErrors`, `deferredValidationErrors`, `uncapturedErrors` all 0, `draws` above 0 | a non-zero error count |
| `input: kb=.. mouse=.. pad=..`, `audio: state=suspended` | audio stays suspended until a key press (a gesture) | audio running before any gesture |

`data-state`: `starting`, `running`, `fatal`. `data-generation` rises by one after a lost device. Expected values of the golden are fixed; any other hash means the engine changed (skill `determinism-debug`; never re-pin).

## 4. When it looks stuck

- `tabs_context` ends by saying whether the Browser pane is displayed or hidden. A hidden pane pauses animation frames: the HUD stays at `frame 1` even though `document.visibilityState` still says `visible` (seen 2026-10-17). Ask the person to bring the pane back (Ctrl+Shift+B in the Claude app), then read the text twice, a few seconds apart, and quote both.
- `read_console_messages` with `onlyErrors: true` shows wasm panics and WebGPU errors.
- A screenshot (`computer`, action `screenshot`) is a second opinion only: a WebGPU canvas can capture black. The numbers in the last JSON line are the proof.
- A site approval prompt: wait for the person, do not work around it. Do not sign in anywhere.

## 5. Report

Quote: the URL, `data-commit` against the mirror head, the golden line, the first and last JSON line (frames, errors, draws), the adapter vendor, and what you could not see (hidden pane, no input test). A test of keys and mouse needs a person or `node tools/web-input-check.mjs` on a runner.
