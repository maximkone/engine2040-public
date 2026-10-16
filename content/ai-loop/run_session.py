"""Recorded-session driver for the CT4 AI modeling loop.

One MCP client drives the outside tool (Blender through
`mcp-for-blender`, MIT) while the engine side (cook, spatial check,
place, provenance) runs in `cook_and_place` + the registry/ManifestRow
path. Every step appends to `transcript.jsonl`; the session summary
lands in `session.json` for `replay.py`.

Usage: `python run_session.py` from this directory. Starts headless
Blender itself, stops it at the end, and refuses to model when the
generator license gate fails. Network use: none past the already
installed `mcp-for-blender` tool (no Poly Haven / generator downloads;
`Never download models without a readable license`).
"""

import hashlib
import json
import os
import socket
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from license_gate import LicenseRefusal, check_generator  # noqa: E402
from mcp_client import McpClient, McpError  # noqa: E402

SESSION_VERSION = 1
USER_GOAL = "CT4: model a 0.5 m beveled crate prop and export it as GLB"
GENERATOR = {
    "name": "ct4-manual-prop",
    "license": "CC0-1.0",
    "license_source": "content/ai-loop/run_session.py: operator-authored Blender build code, output dedicated to CC0-1.0",
    "tool": "mcp-for-blender over Blender 5.2 LTS (tool GPL; model output is artist bytes)",
}
PROP_NAME = "CT4_Crate"
MCP_PORT = 9876


def probe(exe, args):
    try:
        out = subprocess.run(
            [exe] + args, capture_output=True, text=True, timeout=60
        )
        text = (out.stdout + out.stderr).strip().splitlines()
        return {"present": True, "first_line": text[0] if text else ""}
    except (OSError, subprocess.SubprocessError) as exc:
        return {"present": False, "first_line": f"{type(exc).__name__}: {exc}"}


def wait_for_port(port, timeout_s=180):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        sock = socket.socket()
        sock.settimeout(1.0)
        try:
            if sock.connect_ex(("localhost", port)) == 0:
                return True
        finally:
            sock.close()
        time.sleep(1.0)
    return False


def main():
    t0 = time.time()
    transcript_path = os.path.join(HERE, "transcript.jsonl")
    session_path = os.path.join(HERE, "session.json")
    fixture_dir = os.path.join(HERE, "fixture")
    os.makedirs(fixture_dir, exist_ok=True)
    export_path = os.path.join(fixture_dir, "prop.glb")
    stop_file = os.path.join(
        os.environ.get("TEMP", os.environ.get("TMP", ".")), "ct4-blender.stop"
    )
    try:
        os.remove(stop_file)
    except OSError:
        pass

    steps = []

    def record(tool, arguments, ok, summary):
        entry = {
            "seq": len(steps) + 1,
            "tool": tool,
            "arguments": arguments,
            "ok": ok,
            "summary": summary,
        }
        steps.append(entry)
        with open(transcript_path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry) + "\n")
        return entry

    try:
        os.remove(transcript_path)
    except OSError:
        pass

    blender_probe = probe("blender", ["--version"])
    uvx_probe = probe("uvx", ["--version"])
    record("probe", {"exe": "blender"}, blender_probe["present"], blender_probe)
    record("probe", {"exe": "uvx"}, uvx_probe["present"], uvx_probe)
    if not blender_probe["present"]:
        raise McpError("blender absent: cannot run the live modeling half")

    # License gate runs BEFORE any modeling output exists.
    try:
        license_tag = check_generator(GENERATOR)
    except LicenseRefusal as exc:
        record("license_gate", {"generator": GENERATOR.get("name")}, False, str(exc))
        raise
    record("license_gate", {"generator": GENERATOR["name"]}, True, license_tag)

    bridge = os.path.join(HERE, "blender_bridge.py")
    telemetry_note = "not attempted"
    blender_proc = subprocess.Popen(
        [
            "blender",
            "--background",
            "--python",
            bridge,
            "--",
            str(MCP_PORT),
            stop_file,
            "1200",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        if not wait_for_port(MCP_PORT):
            raise McpError("blender bridge never opened the socket")
        record("bridge_start", {"port": MCP_PORT}, True, "socket open")

        env = dict(os.environ)
        env["BLENDER_MCP_SAFE_MODE"] = "1"
        env["BLENDER_PORT"] = str(MCP_PORT)
        client = McpClient(["uvx", "mcp-for-blender"], env=env)
        try:
            hello = client.initialize()
            record("mcp_initialize", {}, True, str(hello.get("serverInfo", hello))[:200])
            tools = client.list_tools()
            names = sorted(t["name"] for t in tools.get("tools", []))
            record("mcp_tools_list", {}, True, ",".join(names))
            for needed in ("execute_blender_code", "export_scene"):
                if needed not in names:
                    raise McpError(f"mcp server lacks {needed}")

            out = client.call_tool("disable_telemetry", {"user_prompt": USER_GOAL})
            record("disable_telemetry", {}, True, out[:200])
            telemetry_note = out[:200]
            status = client.call_tool("get_addon_status", {"user_prompt": USER_GOAL})
            record("get_addon_status", {}, True, status[:300])
            scene = client.call_tool("get_scene_info", {"user_prompt": USER_GOAL})
            record("get_scene_info", {}, True, scene[:300])

            with open(os.path.join(HERE, "build_prop.py"), encoding="utf-8") as fh:
                build_code = fh.read()
            out = client.call_tool(
                "execute_blender_code",
                {"code": build_code, "user_prompt": USER_GOAL},
            )
            record("execute_blender_code", {"file": "build_prop.py"}, True, out[:300])
            if PROP_NAME not in out:
                raise McpError(f"build step did not report {PROP_NAME}: {out[:200]}")

            info = client.call_tool(
                "get_object_info",
                {"object_name": PROP_NAME, "user_prompt": USER_GOAL},
            )
            record("get_object_info", {"object": PROP_NAME}, True, info[:300])

            out = client.call_tool(
                "export_scene",
                {
                    "filepath": export_path,
                    "format": "glb",
                    "object_names": [PROP_NAME],
                    "apply_modifiers": True,
                    "user_prompt": USER_GOAL,
                },
            )
            record("export_scene", {"filepath": export_path}, True, out[:300])
        finally:
            client.close()
    finally:
        with open(stop_file, "w", encoding="utf-8") as fh:
            fh.write("stop")
        blender_proc.wait(timeout=120)

    with open(export_path, "rb") as fh:
        blob = fh.read()
    artifact = {
        "path": "fixture/prop.glb",
        "bytes": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
    }
    record("artifact_hash", artifact, True, artifact["sha256"][:16])

    session = {
        "session_version": SESSION_VERSION,
        "user_goal": USER_GOAL,
        "safe_mode": True,
        "telemetry": telemetry_note,
        "blender": blender_probe["first_line"][:120],
        "generator": GENERATOR,
        "license_tag": license_tag,
        "steps": steps,
        "artifact": artifact,
        "elapsed_s": round(time.time() - t0, 1),
    }
    with open(session_path, "w", encoding="utf-8") as fh:
        json.dump(session, fh, indent=2)
    print(f"session ok: {len(steps)} steps, {artifact['bytes']} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
