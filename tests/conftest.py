"""conftest.py — pytest root configuration.

Ensures that ``src/`` is on ``sys.path`` for every test session so individual
test files do not need to call ``sys.path.insert`` themselves.

Also autouse-mocks the calibration resolver so non-calibration tests are
isolated from a real ``.cache/input_latency.json`` artefact in the working
tree. Calibration-focused tests opt out via the nodeid check below.
"""
from __future__ import annotations

import sys
from pathlib import Path

# Add the src/ directory to sys.path once for the entire session.
# This replaces the per-file `sys.path.insert(0, str(src_dir))` pattern.
_src = Path(__file__).parent.parent / "src"
if str(_src) not in sys.path:
    sys.path.insert(0, str(_src))


import pytest  # noqa: E402


@pytest.fixture(autouse=True)
def isolate_input_latency_cache(request, monkeypatch):
    """Isolate every test from a real ``.cache/input_latency.json`` so a
    cached run cannot pollute a test that does not opt in.

    We patch both the source module and the binding captured by
    ``calibrated_policy`` so every production consumer sees the mock even when
    that module was imported during pytest collection.
    """
    nodeid = request.node.nodeid
    if (
        "test_calibrated_margin_resolution" in nodeid
        or "test_calibrated_margin_recommendation_poison_cases" in nodeid
        or "test_calibrated_margin_rejects_low_sample_count" in nodeid
        or "test_calibration_regression" in nodeid
        or "test_load_calibration_resolution_states" in nodeid
    ):
        return
    import sky_music.infrastructure.calibration_loader as loader_module
    import sky_music.orchestration.calibrated_policy as calibrated_policy_module

    def _default_resolution(*_args: object, **_kwargs: object):
        return loader_module.CalibrationLoadResult(
            status=loader_module.CalibrationStatus.UNCALIBRATED,
            resolved_margin_us=500,
            margin_source=loader_module.SOURCE_DEFAULT_500,
            summary=None,
        )

    monkeypatch.setattr(
        loader_module,
        "load_calibration_resolution",
        _default_resolution,
    )
    monkeypatch.setattr(
        calibrated_policy_module,
        "load_calibration_resolution",
        _default_resolution,
    )
