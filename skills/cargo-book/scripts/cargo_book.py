"""Run every good/bad pair of references/pairs.json with real cargo.

Each side runs in its own fresh scratch directory under $TEMP/opencode
(never in the repo, never in another repo's tree) with CARGO_NET_OFFLINE=1,
so every fixture is dependency-free and no network is touched. A good side
must exit as named and print the fragments it promises; a bad side must
fail with the real cargo error it names.

    python skills/cargo-book/scripts/cargo_book.py          # one line per pair
    python skills/cargo-book/scripts/cargo_book.py --json   # machine readable
    python skills/cargo-book/scripts/cargo_book.py --pair cb-p01

Exit code 0 when every good side prints what it promises and every bad side
fails as named.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PAIRS = HERE.parent / "references" / "pairs.json"


def scratch_root(tag: str) -> Path:
    base = Path(os.environ.get("TEMP") or tempfile.gettempdir()) / "opencode"
    base.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix=f"cb-{tag}-", dir=str(base)))


def run_step(step: dict, root: Path, cargo: str, env: dict) -> tuple[bool, str]:
    """Run one step (argv, write, or assertion) inside root. Returns (ok, note)."""
    if "write" in step:
        for rel, body in step["write"].items():
            target = root / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(body, encoding="utf-8", newline="\n")
        return True, f"wrote {len(step['write'])} file(s)"
    saw_assert = False
    for rel in step.get("assert_exists", []):
        saw_assert = True
        if not (root / rel).exists():
            return False, f"missing {rel}"
    for rel in step.get("assert_absent", []):
        saw_assert = True
        if (root / rel).exists():
            return False, f"should be absent but exists: {rel}"
    if "assert_contains" in step:
        spec = step["assert_contains"]
        path = root / spec["file"]
        if not path.is_file():
            return False, f"missing {spec['file']}"
        text = path.read_text(encoding="utf-8", errors="replace").lower()
        for fragment in spec.get("fragments", []):
            if fragment.lower() not in text:
                return False, f"{spec['file']} lacks {fragment!r}"
        return True, f"{spec['file']} holds {len(spec.get('fragments', []))} fragment(s)"
    if "argv" in step:
        cwd = root / step.get("cwd", ".")
        cwd.mkdir(parents=True, exist_ok=True)
        argv = list(step["argv"])
        if argv and argv[0] == "cargo":
            argv = [cargo, *argv[1:]]
        proc = subprocess.run(
            argv, cwd=cwd, env=env,
            capture_output=True, text=True, encoding="utf-8", errors="replace",
            stdin=subprocess.DEVNULL, timeout=300,
        )
        combined = (proc.stdout or "") + "\n" + (proc.stderr or "")
        lowered = combined.lower()
        want_exit = step.get("expect_exit", 0)
        if proc.returncode != want_exit:
            return False, f"exit {proc.returncode} != {want_exit}: {combined.strip()[-300:]}"
        for fragment in step.get("expect", []):
            if fragment.lower() not in lowered:
                return False, f"output lacks {fragment!r}: {combined.strip()[-300:]}"
        for fragment in step.get("absent", []):
            if fragment.lower() in lowered:
                return False, f"output should lack {fragment!r}"
        if "count" in step:
            frag, times = step["count"]["fragment"], step["count"]["times"]
            got = lowered.count(frag.lower())
            if got != times:
                return False, f"{frag!r} appears {got}x, want {times}x"
        return True, f"exit {proc.returncode}"
    if saw_assert:
        return True, "assertions hold"
    return False, f"unknown step {sorted(step)}"


def run_side(pair: dict, side: str, root: Path, cargo: str, env: dict) -> tuple[bool, str]:
    """Replay one side (good or bad) of a pair in a fresh scratch dir."""
    spec = pair.get(side)
    if spec is None:
        return True, "no side"
    work = root / f"{pair['id']}-{side}"
    work.mkdir(parents=True)
    for rel, body in spec.get("fixture", {}).items():
        target = work / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(body, encoding="utf-8", newline="\n")
    for n, step in enumerate(spec.get("steps", []), 1):
        ok, note = run_step(step, work, cargo, env)
        if not ok:
            return False, f"step {n}: {note}"
    return True, f"{len(spec.get('steps', []))} step(s)"


def run_pairs(doc: dict) -> list[dict]:
    cargo = shutil.which("cargo")
    if not cargo:
        raise RuntimeError("cargo is not installed")
    env = dict(os.environ, CARGO_NET_OFFLINE="1")
    root = scratch_root("pairs")
    results = []
    for pair in doc["pairs"]:
        good_ok, good_note = run_side(pair, "good", root, cargo, env)
        bad_ok, bad_note = run_side(pair, "bad", root, cargo, env)
        ok = good_ok and bad_ok
        results.append({
            "id": pair["id"], "title": pair.get("title", ""),
            "good_ok": good_ok, "good_note": good_note,
            "bad_ok": bad_ok, "bad_note": bad_note, "ok": ok,
        })
    return results


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--json", action="store_true", help="machine-readable results")
    ap.add_argument("--pair", default=None, help="run one pair by id")
    args = ap.parse_args(argv)
    doc = json.loads(PAIRS.read_text(encoding="utf-8"))
    if args.pair:
        doc["pairs"] = [p for p in doc["pairs"] if p["id"] == args.pair]
        if not doc["pairs"]:
            print(f"unknown pair {args.pair}")
            return 2
    results = run_pairs(doc)
    if args.json:
        print(json.dumps(results, indent=2))
    else:
        for r in results:
            if r["ok"]:
                print(f"PASS {r['id']}: {r['title']}")
            else:
                bad = "" if r["bad_ok"] else f" bad[{r['bad_note']}]"
                good = "" if r["good_ok"] else f" good[{r['good_note']}]"
                print(f"FAIL {r['id']}:{good}{bad}")
    return 0 if all(r["ok"] for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
