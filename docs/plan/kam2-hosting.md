# KAM2 hosting: the browser build on another PC with no setup

KAM9, 2026-10-20. `tools/kam2-host.mjs` serves the KAM8 build (`target/kam2web`)
on this PC and opens a free Cloudflare quick tunnel, so a work PC needs only a
browser: no install, no account, no data pack. The tunnel is the mechanism
`kam-web` uses (`docs/DEPLOY.md` section 3, `cloudflared tunnel --url ...`).

## Start it

```powershell
cd C:\engine2040
node tools/kam2-host.mjs
```

It reuses a complete `target/kam2web` (or builds it the way
`tools/kam2-web.mjs` does), serves it on `http://127.0.0.1:8136`, starts
`cloudflared`, waits until the public page answers, and prints one line:

```
KAM9 HOST local=http://127.0.0.1:8136/index.html public=https://<random>.trycloudflare.com/index.html
```

Send the public URL to the other PC and open it. The address is random and dies
with the process, so a new start means a new URL.

`cloudflared.exe` is found from `KAM9_CLOUDFLARED`, then `%TEMP%\opencode`,
`%LOCALAPPDATA%\cloudflared`, `tools/`, then `PATH`. It is free and needs no
account. This PC must stay on and online; every viewer's traffic passes through
it and through Cloudflare's edge. Nothing is uploaded or stored: the tunnel only
forwards requests for the static folder.

## What it exposes

Only the static page and its files: `index.html`, `app.js`, `kam2web.js`,
`kam2web_bg.wasm` (156 KB), the two manifests and two atlases. Read-only. There
is no relay, no multiplayer, no file access, no secret, no write path. The local
server binds `127.0.0.1` only, so the LAN cannot reach it directly.

The port is fixed at 8136 (`--port` changes it). If it is busy, the start fails
loudly. 8135 is left to the separate `kam-web` tunnel on this machine.

## Stop it

Press `Ctrl+C` in the window that runs it. It stops the `cloudflared` child it
started and closes the server; the public URL stops answering. Nothing else is
touched. No background process is needed or left behind.

## Prove it

```powershell
node tools/kam2-host.mjs --check
```

The check starts the same server and tunnel, fetches `index.html` on the local
URL, fetches it on the public URL, then loads
`<public>/index.html?frames=60` in headless Chrome, waits for the page to freeze
at exactly 60 frames, and writes `target/kam2host.png`. It prints:

```
KAM9 OK local=http://127.0.0.1:8136/index.html public=https://<random>.trycloudflare.com/index.html frames=60 png=C:\engine2040\target\kam2host.png bytes=<n>
```

and then stops its own server, tunnel and Chrome. Logs:
`target/kam2host-tunnel.log`, `target/kam2host-chrome.log`.

## If a tunnel ever needs a key

A quick tunnel needs no account and no key; that is why KAM9 uses it. It also
gives no stable hostname and no access control. A named Cloudflare tunnel fixes
both and needs things only the owner can provide:

- a Cloudflare account and a domain on Cloudflare (free plan is enough);
- a tunnel token (or the tunnel's credentials JSON), created in that account;
- a decision on who may open the page (Cloudflare Access, or the URL is public).

The token is a secret: keep it out of git and out of logs, pass it by environment
variable, and never print it. The owner's command would then be
`cloudflared tunnel run --token <token>` (or `--config` with the credentials file)
instead of the quick-tunnel line. Until the owner supplies those, the quick
tunnel is the whole story and it costs nothing.

## Limits

- One static page, no relay, so no two-player match yet.
- The URL changes every start; a named tunnel is the fix.
- The host PC is the server: sleep, logout or network loss ends the page for the
  other PC.
- Cloudflare's free quick tunnel has no uptime promise; a dropped connection is
  retried by `cloudflared` and usually recovers in seconds.
