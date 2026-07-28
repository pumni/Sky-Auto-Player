"""conftest.py — pytest root configuration.

Ensures that ``src/`` is on ``sys.path`` for every test session so individual
test files do not need to call ``sys.path.insert`` themselves.

Also autouse-mocks the calibration loader so non-calibration tests are
isolated from a real ``.cache/input_latency.json`` artefact in the working
tree. The three calibration-focused tests opt out via the nodeid check
below so they can exercise the real loader.
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

    We patch ``sky_music.infrastructure.calibration_loader.load_calibrated_margin_recommendation``
    (the source module) so every consumer -- the orchestration
    ``RuntimeSessionState.apply_session`` and any test that imports the
    loader directly -- sees the mock. Tests whose nodeid contains one of
    the three calibration-loader opt-out markers run unmocked.
    """
    nodeid = request.node.nodeid
    if (
        "test_calibrated_margin_resolution" in nodeid
        or "test_calibrated_margin_recommendation_poison_cases" in nodeid
        or "test_calibrated_margin_rejects_low_sample_count" in nodeid
    ):
        return
    import sky_music.infrastructure.calibration_loader as loader_module
    monkeypatch.setattr(
        loader_module,
        "load_calibrated_margin_recommendation",
        lambda: (None, "default_500"),
    )


