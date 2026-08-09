from __future__ import annotations

import json
from pathlib import Path

from packaging.version import Version

CORPUS = (
    Path(__file__).resolve().parents[1]
    / "rust"
    / "crates"
    / "sky_updater"
    / "tests"
    / "pep440_ordering.json"
)


def test_pep440_corpus_matches_python_packaging() -> None:
    cases = json.loads(CORPUS.read_text(encoding="utf-8"))
    for case in cases:
        left = Version(case["left"])
        right = Version(case["right"])
        expected = case["ordering"]
        python_order = -1 if left < right else 1 if left > right else 0
        assert python_order == expected, case
