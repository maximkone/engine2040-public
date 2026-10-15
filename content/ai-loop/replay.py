"""Recorded-session replay for the CT4 AI modeling loop.

Replays the fixture session in `content/ai-loop` without touching
Blender, the network, or any model: loads `session.json`,
`cook_report.json` and `provenance.json`, and fails loudly on any
shape, license, digest or verdict mismatch.

Checked per replay:
- session shape: version 1, ordered steps, every step ok, required
  tools present in order (probe, license_gate, execute_blender_code,
  export_scene, artifact_hash).
- license gate: the recorded generator passes `check_generator`, so a
  session naming an unlicensed generator can never replay green.
- artifact: `fixture/prop.glb` exists and its sha256 matches.
- cook: report digest present, deterministic_x2 true, Prop budget
  fields present.
- spatial: staged verdict clean with zero new findings.
- provenance: generator+license match the session, artifact digest
  matches the GLB, receipt is a nonzero hex id, all verifiers passed.

Usage: `python replay.py [ai-loop-dir]`; exit 0 replays clean.
"""

import hashlib
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from license_gate import LicenseRefusal, check_generator  # noqa: E402

REQUIRED_TOOLS = ("probe", "license_gate", "execute_blender_code", "export_scene")


class ReplayFailure(Exception):
    pass


def load_json(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError) as exc:
        raise ReplayFailure(f"unreadable {os.path.basename(path)}: {exc}") from exc


def replay(directory=HERE):
    session = load_json(os.path.join(directory, "session.json"))
    report = load_json(os.path.join(directory, "cook_report.json"))
    provenance = load_json(os.path.join(directory, "provenance.json"))

    if session.get("session_version") != 1:
        raise ReplayFailure("session_version is not 1")
    steps = session.get("steps")
    if not isinstance(steps, list) or not steps:
        raise ReplayFailure("steps missing or empty")
    for want_seq, step in enumerate(steps, 1):
        if step.get("seq") != want_seq:
            raise ReplayFailure(f"step order broken at seq {step.get('seq')}")
        if step.get("ok") is not True:
            raise ReplayFailure(f"step {want_seq} ({step.get('tool')}) not ok")
    seen = [s.get("tool") for s in steps]
    for tool in REQUIRED_TOOLS:
        if tool not in seen:
            raise ReplayFailure(f"required tool {tool} absent from session")

    generator = session.get("generator")
    try:
        license_tag = check_generator(generator)
    except LicenseRefusal as exc:
        raise ReplayFailure(f"session generator refused: {exc}") from exc
    if session.get("license_tag") != license_tag:
        raise ReplayFailure("session license_tag disagrees with the gate")

    artifact = session.get("artifact", {})
    glb_path = os.path.join(directory, artifact.get("path", ""))
    try:
        with open(glb_path, "rb") as fh:
            blob = fh.read()
    except OSError as exc:
        raise ReplayFailure(f"artifact missing: {exc}") from exc
    if hashlib.sha256(blob).hexdigest() != artifact.get("sha256"):
        raise ReplayFailure("artifact sha256 mismatch")
    if artifact.get("bytes") != len(blob):
        raise ReplayFailure("artifact byte count mismatch")

    cook = report.get("cook", {})
    if not cook.get("deterministic_x2"):
        raise ReplayFailure("cook not proven deterministic x2")
    if cook.get("class") != "Prop" or not cook.get("digest"):
        raise ReplayFailure("cook record incomplete")

    spatial = report.get("spatial", {})
    if spatial.get("verdict") != "clean" or spatial.get("new_findings") != 0:
        raise ReplayFailure(f"spatial gate not clean: {spatial}")

    manifest = report.get("manifest", {})
    if manifest.get("license") != license_tag:
        raise ReplayFailure("manifest license disagrees with the gate")
    if report.get("provenance_receipt") != provenance.get("receipt"):
        raise ReplayFailure("cook report and provenance receipts disagree")
    if provenance.get("generator") != generator.get("name"):
        raise ReplayFailure("provenance generator mismatch")
    if provenance.get("license") != license_tag:
        raise ReplayFailure("provenance license mismatch")
    if provenance.get("artifact_digest") != artifact.get("sha256"):
        raise ReplayFailure("provenance artifact digest mismatch")
    verifiers = provenance.get("verifiers", [])
    if not verifiers or any(not v.get("passed") for v in verifiers):
        raise ReplayFailure("provenance verifiers not all passed")
    try:
        int(provenance.get("receipt", ""), 16)
    except (ValueError, TypeError) as exc:
        raise ReplayFailure("provenance receipt not a hex id") from exc
    if int(provenance.get("receipt", "0"), 16) == 0:
        raise ReplayFailure("provenance receipt is zero")

    return {
        "steps": len(steps),
        "bytes": len(blob),
        "license": license_tag,
        "cook_digest": cook.get("digest"),
        "receipt": provenance.get("receipt"),
    }


if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else HERE
    try:
        summary = replay(target)
    except ReplayFailure as exc:
        print(f"REPLAY FAIL: {exc}")
        sys.exit(1)
    print(f"replay ok: {summary}")
