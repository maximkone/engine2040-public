"""Minimal MCP stdio client for the CT4 modeling loop.

Speaks newline-delimited JSON-RPC to the `mcp-for-blender` server
(FastMCP stdio framing): initialize, notifications/initialized,
tools/list, tools/call. Only what the session needs; stdlib only.
"""

import json
import subprocess


class McpError(Exception):
    pass


class McpClient:
    def __init__(self, argv, env=None):
        self.proc = subprocess.Popen(
            argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=env,
            text=True,
            bufsize=1,
        )
        self._next_id = 0

    def _send(self, method, params=None, notification=False):
        self._next_id += 1
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if not notification:
            msg["id"] = self._next_id
            want = self._next_id
        else:
            want = None
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()
        if notification:
            return None
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise McpError(f"server closed stdio during {method}")
            try:
                resp = json.loads(line)
            except ValueError:
                continue  # log noise on stdout is skipped, never parsed
            if resp.get("id") != want:
                continue  # notification or foreign reply: ignore
            if "error" in resp:
                raise McpError(f"{method}: {resp['error']}")
            return resp.get("result")

    def initialize(self):
        result = self._send(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "ct4-ai-loop", "version": "1"},
            },
        )
        self._send("notifications/initialized", notification=True)
        return result

    def list_tools(self):
        return self._send("tools/list", {})

    def call_tool(self, name, arguments):
        result = self._send("tools/call", {"name": name, "arguments": arguments})
        parts = []
        for item in result.get("content", []):
            if item.get("type") == "text":
                parts.append(item.get("text", ""))
        return "\n".join(parts)

    def close(self):
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        self.proc.wait(timeout=30)
