"""Harness tests: recorded-session replay shape over fixture sessions.

Run: `python test_replay.py` from this directory (stdlib only). The
live test replays the real `session.json` written by `run_session.py`;
the fixture tests mutate a copy in a temp dir and prove each mismatch
class fails replay (shape, order, license, digest, verdict).
"""

import copy
import hashlib
import json
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from replay import ReplayFailure, replay  # noqa: E402


def read_session():
    with open(os.path.join(HERE, "session.json"), encoding="utf-8") as fh:
        return json.load(fh)


def write_copy(tmp, session, report, provenance, blob):
    with open(os.path.join(tmp, "session.json"), "w", encoding="utf-8") as fh:
        json.dump(session, fh)
    with open(os.path.join(tmp, "cook_report.json"), "w", encoding="utf-8") as fh:
        json.dump(report, fh)
    with open(os.path.join(tmp, "provenance.json"), "w", encoding="utf-8") as fh:
        json.dump(provenance, fh)
    os.makedirs(os.path.join(tmp, "fixture"), exist_ok=True)
    with open(os.path.join(tmp, "fixture", "prop.glb"), "wb") as fh:
        fh.write(blob)


class ReplayShapeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with open(os.path.join(HERE, "cook_report.json"), encoding="utf-8") as fh:
            cls.report = json.load(fh)
        with open(os.path.join(HERE, "provenance.json"), encoding="utf-8") as fh:
            cls.provenance = json.load(fh)
        with open(
            os.path.join(HERE, "fixture", "prop.glb"), "rb"
        ) as fh:
            cls.blob = fh.read()

    def fixture(self):
        tmp = tempfile.mkdtemp(prefix="ct4-replay-")
        write_copy(tmp, read_session(), self.report, self.provenance, self.blob)
        return tmp

    def test_live_session_replays(self):
        summary = replay(HERE)
        self.assertEqual(summary["license"], "CC0-1.0")
        self.assertGreater(summary["steps"], 0)
        self.assertGreater(summary["bytes"], 0)

    def test_fixture_copy_replays(self):
        summary = replay(self.fixture())
        self.assertEqual(summary["license"], "CC0-1.0")

    def test_tampered_bytes_fail(self):
        tmp = self.fixture()
        with open(os.path.join(tmp, "fixture", "prop.glb"), "ab") as fh:
            fh.write(b"\x00")
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_unlicensed_generator_fails(self):
        tmp = self.fixture()
        session = read_session()
        session["generator"] = {"name": "mystery-gen", "license": ""}
        write_copy(tmp, session, self.report, self.provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_forbidden_license_fails(self):
        tmp = self.fixture()
        session = read_session()
        session["generator"] = {
            "name": "nc-gen",
            "license": "CC-BY-NC-4.0",
            "license_source": "x",
        }
        session["license_tag"] = "CC-BY-NC-4.0"
        report = copy.deepcopy(self.report)
        report["manifest"]["license"] = "CC-BY-NC-4.0"
        provenance = copy.deepcopy(self.provenance)
        provenance["license"] = "CC-BY-NC-4.0"
        write_copy(tmp, session, report, provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_failed_step_fails(self):
        tmp = self.fixture()
        session = read_session()
        session["steps"][3]["ok"] = False
        write_copy(tmp, session, self.report, self.provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_reordered_steps_fail(self):
        tmp = self.fixture()
        session = read_session()
        session["steps"][0], session["steps"][1] = (
            session["steps"][1],
            session["steps"][0],
        )
        write_copy(tmp, session, self.report, self.provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_dirty_spatial_fails(self):
        tmp = self.fixture()
        report = copy.deepcopy(self.report)
        report["spatial"] = {"verdict": "red", "new_findings": 1, "hash": "00"}
        write_copy(tmp, read_session(), report, self.provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_receipt_mismatch_fails(self):
        tmp = self.fixture()
        provenance = copy.deepcopy(self.provenance)
        provenance["receipt"] = "deadbeefdeadbeef"
        write_copy(tmp, read_session(), self.report, provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_missing_session_shape_fails(self):
        tmp = self.fixture()
        session = read_session()
        del session["steps"]
        write_copy(tmp, session, self.report, self.provenance, self.blob)
        with self.assertRaises(ReplayFailure):
            replay(tmp)

    def test_sha256_recorded_is_real(self):
        session = read_session()
        self.assertEqual(
            session["artifact"]["sha256"], hashlib.sha256(self.blob).hexdigest()
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
