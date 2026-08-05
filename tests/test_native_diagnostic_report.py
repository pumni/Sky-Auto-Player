from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest


def _load_reporter():
    path = Path(__file__).parents[1] / "scripts" / "report_native_diagnostic.py"
    spec = importlib.util.spec_from_file_location("native_diagnostic_report", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


REPORTER = _load_reporter()


def test_reporter_preserves_sender_game_boundary(tmp_path: Path) -> None:
    artifact = tmp_path / "run.json"
    artifact.write_text(
        '{"snapshot": {"outcome": "finished", "chords_rejected": 0}}',
        encoding="utf-8",
    )

    report = REPORTER.build_report([(1.0, artifact)])

    assert report["evidence_scope"] == "sender_completion"
    assert report["game_input_observed"] is False
    assert report["runs"][0]["diagnosis"] == "clean_native_delivery"


def test_reporter_rejects_duplicate_hold_profiles(tmp_path: Path) -> None:
    artifact = tmp_path / "run.json"
    artifact.write_text("{}", encoding="utf-8")
    with pytest.raises(ValueError, match="once"):
        REPORTER.build_report([(1.0, artifact), (1.0, artifact)])
