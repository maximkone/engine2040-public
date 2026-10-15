"""Blender-side bridge for the CT4 AI modeling loop.

Runs inside headless Blender (`blender --background --python`):
enables the bundled `blender_mcp` addon, starts its localhost socket
server, then pumps the command queue on the main thread until a stop
file appears or the runtime guard expires.

Why the manual pump: the addon drains its socket queue through
`bpy.app.timers`, which never tick in background mode, so this script
(the main thread) drains the queue itself at ~20 Hz. No modeling lives
here; the MCP client sends all scene work as `execute_code` steps.
"""
import os
import sys
import time

PORT = int(sys.argv[sys.argv.index("--") + 1]) if "--" in sys.argv else 9876
ARGS = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
STOP_FILE = ARGS[1] if len(ARGS) > 1 else ""
MAX_SECONDS = float(ARGS[2]) if len(ARGS) > 2 else 1200.0

import bpy

# A stale `blender_mcp/` package install shadows the fresh `blender_mcp.py`
# on `import blender_mcp` (packages win over same-named modules), so load
# the freshly installed file by explicit path. The user config is left
# untouched; nothing here writes outside the session stop file.
import importlib.util

# Addon file resolution: env override first, then APPDATA-derived
# candidates (current default included), first existing wins.
_BLENDER_ADDON_DEFAULT = (
    "C:\\Users\\me\\AppData\\Roaming\\Blender Foundation\\Blender\\5.2"
    "\\scripts\\addons\\blender_mcp.py"
)


def _addon_candidates():
    cands = []
    env_addon = os.environ.get("BLENDER_ADDON_PATH", "").strip()
    if env_addon:
        cands.append(env_addon)
    env_blender = os.environ.get("BLENDER_PATH", "").strip()
    if env_blender and env_blender.lower().endswith(".py"):
        cands.append(env_blender)
    appdata = os.environ.get("APPDATA") or os.path.join(
        os.path.expanduser("~"), "AppData", "Roaming"
    )
    cands.append(
        os.path.join(
            appdata,
            "Blender Foundation",
            "Blender",
            "5.2",
            "scripts",
            "addons",
            "blender_mcp.py",
        )
    )
    try:
        vendor = os.path.join(appdata, "Blender Foundation", "Blender")
        for name in sorted(os.listdir(vendor), reverse=True):
            p = os.path.join(vendor, name, "scripts", "addons", "blender_mcp.py")
            if p not in cands:
                cands.append(p)
    except OSError:
        pass
    if _BLENDER_ADDON_DEFAULT not in cands:
        cands.append(_BLENDER_ADDON_DEFAULT)
    return cands


def _resolve_addon_file():
    tried = _addon_candidates()
    for candidate in tried:
        try:
            if os.path.exists(candidate):
                return candidate
        except OSError:
            continue
    raise RuntimeError(
        "blender_mcp.py not found. Set BLENDER_ADDON_PATH. Tried:\n"
        + "\n".join(tried)
    )


_ADDON_FILE = _resolve_addon_file()
_spec = importlib.util.spec_from_file_location("ct4_blender_mcp Fresh", _ADDON_FILE)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)

BlenderMCPServer = _mod.BlenderMCPServer
# Register scene properties/handlers the same way enabling the addon
# would; execute_code and the generator tools read them.
_mod.register()
server = BlenderMCPServer(host="localhost", port=PORT)


def bridge_start(server):
    """Start the bridge without the background-mode guard.

    `BlenderMCPServer.start` returns early under `blender -b` because its
    queue drain rides `bpy.app.timers`, which never tick headless. This
    bridge pumps `server._drain_command_queue()` itself (same function,
    same thread rule: all bpy access on the main thread), so perform the
    same startup the guard skips: bind, listen, serve thread. No timer is
    registered; the pump loop below replaces it.
    """
    import socket
    import threading

    if server.running:
        return
    server.running = True
    try:
        server.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.socket.bind((server.host, server.port))
        server.socket.listen(5)
        server.server_thread = threading.Thread(target=server._server_loop)
        server.server_thread.daemon = True
        server.server_thread.start()
        print(f"CT4 bridge serving on {server.host}:{server.port}", flush=True)
    except Exception as exc:
        print(f"CT4 bridge failed to start: {exc}", flush=True)
        server.stop()
        raise


bridge_start(server)

deadline = time.time() + MAX_SECONDS
while time.time() < deadline:
    server._drain_command_queue()
    if STOP_FILE:
        try:
            with open(STOP_FILE, "rb"):
                pass
            break
        except OSError:
            pass
    time.sleep(0.05)

server.stop()
try:
    _mod.unregister()
except Exception as exc:  # never fail teardown on handler state
    print(f"CT4 bridge unregister note: {exc}", flush=True)
print("CT4 bridge stopped", flush=True)
