"""conftest.py — pytest root configuration.

Ensures that ``src/`` is on ``sys.path`` for every test session so individual
test files do not need to call ``sys.path.insert`` themselves.
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

import sky_music.domain.scheduler_types  # noqa: E402


@pytest.fixture(autouse=True)
def isolate_input_latency_cache(request, monkeypatch):
    if "test_calibrated_margin_resolution" not in request.node.nodeid:
        monkeypatch.setattr(
            sky_music.domain.scheduler_types,
            "get_calibrated_margin_recommendation",
            lambda: None
        )


