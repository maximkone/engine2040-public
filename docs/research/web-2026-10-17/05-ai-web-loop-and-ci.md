# AI web dev and test loop (scout, 2026-10-17)

Read on 2026-10-17. GUESS = not verified. "(snippet)" = search result text, not a fetched page. No runner proof was found for any WebGPU flag line.

## Ideas

| Idea | Source + URL | Licence | Copy or do | Row | Effort |
|---|---|---|---|---|---|
| WebGPU flags with SwiftShader adapter | Chromium web_tests `webgpu-swiftshader` args: https://chromium.googlesource.com/chromium/src/+/HEAD/third_party/blink/web_tests/FlagSpecificConfig | Chromium, BSD-3 (not read on page) | Use these args in the WEB4 launch; print the adapter line. Full line in Top 5 | WEB4 | S |
| Canvas readback, not screenshot | agent-browser.dev/webgpu: headless canvas capture is black on Linux and Windows; luma.gl GPU buffers docs (snippet): row stride is a multiple of 256 | not checked | Copy the frame to a buffer, `mapAsync`, hash the bytes, write the PNG from them. The gate reads the hash | WEB5, AIS2 browser.capture | S |
| Door browser verbs | github.com/microsoft/playwright-mcp (read): navigate, snapshot, take_screenshot, evaluate, console_messages, network_requests, start/stop_tracing, resize, press_key, mouse_click_xy, mouse_wheel. Also github.com/ChromeDevTools/chrome-devtools-mcp (read): trace, network, console | Apache-2.0 (both) | Copy the verb names. Copy `--headless`, `--isolated`, `--output-dir`; without `--headless` the browser runs headed | AIS2 | M |
| Grouped door tools | github.com/playcanvas/editor-mcp-server (read): about 130 tools in groups; a runtime group covers launch, screenshot, logs, live state, input | MIT | Group door verbs into packs, each behind a scope gate | AIS2, AIS4 | S |
| wasm tests in headless Chrome | rustwasm.github.io/wasm-bindgen/wasm-bindgen-test/browsers.html (read) | not checked | `wasm-pack test --headless --chrome`; Chrome switches go in `webdriver.json` under `goog:chromeOptions` args. Run the WEB7 input-tape test there | WEB7, WEB3 | S |
| Readable wasm panics | github.com/rustwasm/console_error_panic_hook (read) | Apache-2.0 or MIT | Archived 2025-08-25, read-only: vendor the hook with `panic::set_hook`. Build with `--keep-debug` (CLI doc, snippet) | WEB4 | S |
| Pages from an Actions workflow | docs.github.com Pages custom workflow (read) | GitHub docs | upload-pages-artifact, then deploy-pages on default-branch push only; PR runs skip deploy | WEB10 | S |
| PR previews | github.com/rossjrw/pr-preview-action (read) | not on page; check its LICENSE | Previews under `pr-preview/pr-N`. Needs Pages source "Deploy from branch"; the Actions source does not work with it. One source per site | NEW: WEB10 PR previews | M |
| Real COOP/COEP headers | developers.cloudflare.com/pages/configuration/headers (read); github.com/gzuidhof/coi-serviceworker (read) | Cloudflare docs; MIT | GitHub Pages cannot set headers (staff reply 2023, no ETA). For threads: Cloudflare `_headers` (100 rules) or coi-serviceworker (reloads on first visit) | NEW: WEB12 threads host | S |
| Visual tolerance | vitest.dev/guide/browser/visual-regression-testing (read); playwright.dev/docs/test-snapshots (read) | Vitest not stated on page | `allowedMismatchedPixelRatio` (example 0.01) or Playwright `maxDiffPixels` with threshold 0.2. One baseline set per runner image | VR1, WEB5 | S |
| Start templates | github.com/TheBevyFlock/bevy_new_2d (read); lab.rosebud.ai blog (vendor claims) | bevy_new_2d CC0, MIT or Apache-2.0; Rosebud not checked | Copy the layout: screens, menus, dev tools behind a key, asset tracking. Rosebud flow: start from a template, ask for a change, read the code | AIS1 | M |

## Top 5 this week

1. WEB4 launch line (GUESS, parts sourced): `--headless=new --enable-unsafe-webgpu --use-webgpu-adapter=swiftshader --enable-dawn-features=allow_unsafe_apis --disable-dawn-features=use_dxc --enable-webgpu-developer-features --use-gpu-in-tests --enable-features=Vulkan --use-angle=vulkan --use-vulkan=swiftshader --disable-vulkan-surface`. The first part is Chromium's config; the Linux tail is agent-browser's preset. Print `chrome://gpu` WebGPU status and the adapter name in the receipt.
2. Gate on readback bytes (WEB5, AIS2): the frame hash plus a non-blank check, not a screenshot.
3. Door verbs named after Playwright MCP, grouped into packs (AIS2); pass `--headless` explicitly.
4. wasm-bindgen-test with `webdriver.json` for WEB7, and a vendored panic hook for WEB4.
5. WEB10: deploy from Actions on default-branch push only; `curl -I` must show `application/wasm` before any `.br` file ships.

## Traps

- A green "adapter ok" does not mean pixels: headless WebGPU canvas capture is black per agent-browser, and it knows no flag that fixes it.
- The adapter can be null: Chrome's troubleshooting page (snippet) lists blocklisted GPUs and option mismatches; Linux needs `chrome://flags/#enable-vulkan`. No source found for `windows-latest` getting a SwiftShader adapter; test it.
- `--use-vulkan=swiftshader` needs the `enable_swiftshader_vulkan` build feature (Chromium SwiftShader doc). Check the runner's Chrome has it. The WebGL SwiftShader fallback is deprecated.
- WEB5's SSIM 0.98 against the native twin compares SwiftShader with a real GPU. Vitest and Playwright both say renders vary by GPU, driver, OS and headless mode. Measure before setting the bar (GUESS: it fails as written).
- Playwright's comparator is pixelmatch, not SSIM; a SSIM check needs its own comparator.
- Playwright MCP runs headed by default; on a runner it needs xvfb or `--headless`.
- WebGPU init is async; a load-time screenshot can be blank with no error (snippet). Print the receipt after the device is ready.
- The GitHub Pages pages I read say nothing about headers or MIME types. A hand-made `.br` failed on github.io (Godot forum snippet). Check with curl.
