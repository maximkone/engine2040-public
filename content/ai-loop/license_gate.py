"""Generator license gate for the CT4 AI modeling loop.

Every generated asset carries generator+license data; a generator
without a readable license is refused before Blender runs, before the
cook runs, and before placement. Refusals are typed
(`LicenseRefusal` with a stable machine `code`), never silent skips.

Readable means: the license names a known shippable grant (CC0-1.0,
CC-BY-4.0, CC-BY-SA-4.0, MIT, Apache-2.0) AND states where its text
was read (a `license_source` string: file path, URL, or `LICENSES/`
row). Unknown generators, empty licenses, non-commercial /
no-derivatives grants, Unreal-only or other EULA-bound sources, and
copyleft engine-infecting code licenses (GPL/AGPL) are refused with
their own codes. Blender itself is GPL, but modeling *output* is the
artist's own bytes (the plan's route 2), so a Blender-built prop with
a stated CC0 grant passes; Blender *code* would never enter the repo.
"""

from dataclasses import dataclass

# Grants the engine may ship generated content under. Each needs a
# readable source named in the record (checked separately).
SHIPPABLE = frozenset({"CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0", "MIT", "Apache-2.0"})

# Grants that forbid redistribution or commercial use: refused loudly.
NON_SHIPPABLE = frozenset({
    "CC-BY-NC-4.0",
    "CC-BY-ND-4.0",
    "CC-BY-NC-SA-4.0",
    "CC-BY-NC-ND-4.0",
    "Fab-UE-Only",
    "Unreal-EULA",
    "NonCommercial",
})

# Code licenses that must never infect the engine or its content
# pipeline: ideas may be reimplemented, never vendored or generated
# into shipped bytes.
INFECTING = frozenset({"GPL-2.0", "GPL-3.0", "AGPL-3.0", "LGPL-3.0", "MPL-2.0"})


@dataclass(frozen=True)
class LicenseRefusal(Exception):
    """Typed refusal: stable code plus human reason."""

    code: str
    generator: str
    reason: str

    def __str__(self) -> str:  # pragma: no cover - trivial
        return f"[{self.code}] generator={self.generator!r}: {self.reason}"


def check_generator(record):
    """Gate one generator record; returns the normalized license tag.

    Raises `LicenseRefusal` with one of UNKNOWN_GENERATOR,
    UNREADABLE_LICENSE, FORBIDDEN_LICENSE or INFECTING_LICENSE.
    """
    if not isinstance(record, dict):
        raise LicenseRefusal("UNKNOWN_GENERATOR", str(record), "not a record")
    name = record.get("name", "")
    if not isinstance(name, str) or not name.strip():
        raise LicenseRefusal("UNKNOWN_GENERATOR", str(name), "generator unnamed")
    license_tag = record.get("license", "")
    if not isinstance(license_tag, str) or not license_tag.strip():
        raise LicenseRefusal(
            "UNREADABLE_LICENSE", name, "no readable license tag on record"
        )
    license_tag = license_tag.strip()
    source = record.get("license_source", "")
    if license_tag in SHIPPABLE:
        if not isinstance(source, str) or not source.strip():
            raise LicenseRefusal(
                "UNREADABLE_LICENSE",
                name,
                f"{license_tag} names no readable source (file, URL or ledger row)",
            )
        return license_tag
    if license_tag in NON_SHIPPABLE:
        raise LicenseRefusal(
            "FORBIDDEN_LICENSE", name, f"{license_tag} forbids redistribution"
        )
    if license_tag in INFECTING:
        raise LicenseRefusal(
            "INFECTING_LICENSE", name, f"{license_tag} code must never ship"
        )
    raise LicenseRefusal(
        "UNREADABLE_LICENSE", name, f"unknown license tag {license_tag!r}"
    )
