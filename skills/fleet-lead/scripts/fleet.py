"""Fleet monitor for a lead session: what every helper agent is doing right now.

    python -I fleet.py fleet <lead_session_id> [--extra <file of session ids>]
    python -I fleet.py live <session_id> [n]       last n items: reasoning, text, tool calls with state
    python -I fleet.py report <session_id> ...     full last text of each (a report the lead never got)

Read-only (sqlite mode=ro) over OpenCode v2's own store. The OpenChamber session.messages
action shows only finished text, so a 15-minute gh-check looks like a dead agent there;
session_message holds every tool call with its running/completed state. Never write here.
"""
import json
import sqlite3
import sys
import time
from pathlib import Path

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
DB = (Path.home() / ".local/share/opencode/opencode.db").as_posix()
c = sqlite3.connect(f"file:{DB}?mode=ro", uri=True, timeout=15)
now = time.time() * 1000
# A tool call running longer than this, or a busy agent silent this long, gets a flag.
LONG_RUN_MIN = 30
SILENT_MIN = 15


def mins(ms):
    return round((now - ms) / 60000) if ms else None


def items(sid, n_msgs=3):
    """Content items of the newest messages, oldest first: (time, kind, item)."""
    out = []
    rows = c.execute(
        "select type, time_updated, data from session_message where session_id=? order by seq desc limit ?",
        (sid, n_msgs),
    ).fetchall()
    for (kind, ts, data) in reversed(rows):
        try:
            d = json.loads(data)
        except ValueError:
            continue
        if kind != "assistant":
            out.append((ts, kind, d))
            continue
        for it in d.get("content", []):
            out.append((it.get("time", {}).get("created") or ts, it.get("type"), it))
    return out


def tool_line(it):
    st = it.get("state", {})
    inp = st.get("input", {})
    arg = inp.get("command") or inp.get("path") or inp.get("pattern") or inp.get("description") or json.dumps(inp)
    return f"{it.get('name')}[{st.get('status')}] {str(arg)[:150]}"


def live(sid, n):
    for (ts, kind, it) in items(sid, 6)[-n:]:
        if kind == "tool":
            print(f"{mins(ts)}m TOOL {tool_line(it)}")
        elif kind in ("text", "reasoning"):
            print(f"{mins(ts)}m {kind.upper()} {str(it.get('text'))[:300]!r}")
        else:
            print(f"{mins(ts)}m {kind}")


def report(sids):
    for sid in sids:
        text = None
        rows = c.execute(
            "select data from session_message where session_id=? and type='assistant' order by seq desc limit 6",
            (sid,),
        ).fetchall()
        for (data,) in rows:
            texts = [it.get("text") for it in json.loads(data).get("content", []) if it.get("type") == "text" and it.get("text")]
            if texts:
                text = texts[-1]
                break
        print(f"===== {sid}\n{text}\n")


def fleet(lead, extra):
    ids = [r[0] for r in c.execute("select id from session_v2 where parent_id=? and time_archived is null", (lead,))]
    if extra and Path(extra).exists():
        ids += [x for x in Path(extra).read_text().split() if x.startswith("ses_")]
    rows = []
    for sid in dict.fromkeys(ids):
        s = c.execute(
            "select title, time_created, tokens_output, tokens_cache_read, cost, parent_id from session_v2 where id=?",
            (sid,),
        ).fetchone()
        if not s:
            continue
        title, created, out, cache, cost, parent = s
        its = items(sid)
        last = its[-1] if its else (created, "-", {})
        running = [it for (_, k, it) in its if k == "tool" and it.get("state", {}).get("status") == "running"]
        flag = ""
        if running:
            t = running[-1].get("time", {})
            run_m = mins(t.get("ran") or t.get("created"))
            doing = f"RUN {run_m}m {tool_line(running[-1])}"
            if run_m and run_m > LONG_RUN_MIN:
                flag = "LONG "
        elif last[1] == "idle":
            doing = f"idle {mins(last[0])}m (turn ended: has its report reached you?)"
        else:
            doing = f"{last[1]} {mins(last[0])}m ago"
            if (mins(last[0]) or 0) > SILENT_MIN:
                flag = "SILENT "
        where = "child" if parent == lead else "OTHER"
        line = (f"{flag}{sid[-8:]} {where} age={mins(created)}m out={out} cacheM={round((cache or 0) / 1e6, 1)} "
                f"${round(cost or 0, 2)} | {title[:34]:34} | {doing}")
        rows.append((flag == "", line))
    for _, line in sorted(rows):
        print(line)


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        print(__doc__)
    elif args[0] == "live":
        live(args[1], int(args[2]) if len(args) > 2 else 6)
    elif args[0] == "report":
        report(args[1:])
    elif args[0] == "fleet":
        extra = args[args.index("--extra") + 1] if "--extra" in args else None
        fleet(args[1], extra)
    else:
        print(__doc__)
