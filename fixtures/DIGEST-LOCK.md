# DIGEST-LOCK — clients/python 3-episode digest

Date: 2026-10-05T20:54Z.
Tree: b219b26.
Binary: present (target/debug/e2040-mcp.exe, .opencode/bin/e2040-mcp.exe). No NEED-BUILD.

## Command

```powershell
python clients/python/agent_loop.py --seed 7 --episodes 3 --ticks 600 --out rows.jsonl
python clients/python/replay.py --in rows.jsonl
python clients/python/replay.py --in rows.jsonl
```

Shim: embedded deterministic mirror, stdlib only. No Rust changes.

## Result

- Seeds: 7, 8, 9 (one per episode).
- Rows: 1800 (3 episodes x 600 ticks).
- Digest: 0x420c6001f231b568 (4759284467698349416).
- Replay x2: identical (0x420c6001f231b568, 0x420c6001f231b568).
- Run output: episodes=3 ticks=600 digest=0x420c6001f231b568.
