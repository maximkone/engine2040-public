"""Harness tests: the generator license gate refuses unlicensed input.

Run: `python test_license_gate.py` from this directory (stdlib only).
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from license_gate import LicenseRefusal, check_generator


def record(name, license_tag, source="content/ai-loop/run_session.py"):
    return {"name": name, "license": license_tag, "license_source": source}


class LicenseGateTests(unittest.TestCase):
    def test_cc0_with_source_passes(self):
        self.assertEqual(
            check_generator(record("ct4-manual-prop", "CC0-1.0")), "CC0-1.0"
        )

    def test_mit_with_source_passes(self):
        self.assertEqual(
            check_generator(record("mcp-for-blender", "MIT", "LICENSE read 2026-09-23")),
            "MIT",
        )

    def test_unknown_generator_refused(self):
        for bad in ({}, {"license": "CC0-1.0"}, {"name": ""}, {"name": "  "}, "nope"):
            with self.assertRaises(LicenseRefusal) as ctx:
                check_generator(bad)
            self.assertEqual(ctx.exception.code, "UNKNOWN_GENERATOR")

    def test_missing_license_refused(self):
        for bad_license in ("", "  ", None):
            with self.assertRaises(LicenseRefusal) as ctx:
                check_generator(record("gen-x", bad_license))
            self.assertEqual(ctx.exception.code, "UNREADABLE_LICENSE")

    def test_license_without_readable_source_refused(self):
        rec = {"name": "gen-x", "license": "CC0-1.0", "license_source": "  "}
        with self.assertRaises(LicenseRefusal) as ctx:
            check_generator(rec)
        self.assertEqual(ctx.exception.code, "UNREADABLE_LICENSE")

    def test_unknown_license_tag_refused(self):
        with self.assertRaises(LicenseRefusal) as ctx:
            check_generator(record("gen-x", "CC-SOMEDAY-9.9"))
        self.assertEqual(ctx.exception.code, "UNREADABLE_LICENSE")

    def test_noncommercial_refused(self):
        for tag in ("CC-BY-NC-4.0", "CC-BY-ND-4.0", "CC-BY-NC-SA-4.0"):
            with self.assertRaises(LicenseRefusal) as ctx:
                check_generator(record("gen-nc", tag))
            self.assertEqual(ctx.exception.code, "FORBIDDEN_LICENSE")

    def test_eula_source_refused(self):
        with self.assertRaises(LicenseRefusal) as ctx:
            check_generator(record("ue-pack", "Fab-UE-Only"))
        self.assertEqual(ctx.exception.code, "FORBIDDEN_LICENSE")

    def test_infecting_code_license_refused(self):
        for tag in ("GPL-3.0", "AGPL-3.0"):
            with self.assertRaises(LicenseRefusal) as ctx:
                check_generator(record("gen-gpl", tag))
            self.assertEqual(ctx.exception.code, "INFECTING_LICENSE")

    def test_refusal_is_typed_with_generator(self):
        try:
            check_generator(record("sketchfab-scrape", "NonCommercial"))
        except LicenseRefusal as exc:
            self.assertEqual(exc.code, "FORBIDDEN_LICENSE")
            self.assertEqual(exc.generator, "sketchfab-scrape")
            self.assertIn("NonCommercial", exc.reason)
        else:
            self.fail("expected LicenseRefusal")


if __name__ == "__main__":
    unittest.main(verbosity=2)
