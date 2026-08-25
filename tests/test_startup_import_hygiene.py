"""Startup import-hygiene regression tests.

Frozen startup pays every top-level import before the first picker frame,
so an eagerly re-added import on the launch path is a silent startup-time
regression. These tests pin the two contracts that keep the pre-picker
import graph lean:

1. ``sky_music`` resolves its version from ``_version.py`` without importing
   ``importlib.metadata`` (whose chain — zipfile/shutil/bz2 — is paid on
   every launch otherwise).
2. Importing any ``sky_music.orchestration`` submodule must not drag in the
   dispatch engine; the engine loads at first playback use.

Both are verified in clean subprocess interpreters so the assertions cannot
be polluted by modules the pytest process already imported.
"""

from __future__ import annotations

import importlib
import os
import subprocess
import sys
from pathlib import Path

import pytest

_REPO_ROOT = Path(__file__).resolve().parent.parent


def _run_clean_python(code: str) -> None:
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(
        [str(_REPO_ROOT / "src"), env.get("PYTHONPATH", "")]
    ).rstrip(os.pathsep)
    result = subprocess.run(
        [sys.executable, "-c", code],
        cwd=str(_REPO_ROOT),
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert result.returncode == 0, (
        f"clean-interpreter probe failed:\n{code}\nstdout: {result.stdout}\nstderr: {result.stderr}"
    )


def test_import_sky_music_skips_importlib_metadata_when_version_module_present() -> None:
    """``_version.py`` (always baked into frozen builds) must resolve the
    version without importing ``importlib.metadata``."""
    try:
        importlib.import_module("sky_music._version")
    except ImportError:
        pytest.skip("_version.py not generated (bare checkout)")
    _run_clean_python(
        "import sys\n"
        "import sky_music\n"
        "assert 'importlib.metadata' not in sys.modules, (\n"
        "    'importlib.metadata was eagerly imported; keep it inside the'\n"
        "    ' _resolve_version fallback branch'\n"
        ")\n"
    )


def test_orchestration_submodule_import_does_not_load_engine() -> None:
    """Any orchestration submodule import (e.g. ``native_admission``, used by
    the startup admission gate) must leave the engine unimported."""
    _run_clean_python(
        "import sys\n"
        "from sky_music.orchestration.native_admission import require_rust_core\n"
        "assert 'sky_music.orchestration.engine' not in sys.modules, (\n"
        "    'orchestration/__init__ eagerly imported the engine; keep the'\n"
        "    ' re-exports lazy (PEP 562)'\n"
        ")\n"
    )


def test_orchestration_lazy_exports_resolve() -> None:
    """The PEP 562 re-exports must keep resolving for package-root callers."""
    import sky_music.orchestration as orchestration
    from sky_music.orchestration import (
        PLAYBACK_QUIT,
        PlaybackEngine,
        TelemetryLogger,
    )

    assert isinstance(PLAYBACK_QUIT, str)
    assert callable(PlaybackEngine)
    assert TelemetryLogger.__name__ == "TelemetryLogger"
    assert set(orchestration.__all__) == orchestration._ALL_EXPORTS


def test_orchestration_lazy_getattr_raises_for_unknown_name() -> None:
    import sky_music.orchestration as orchestration

    with pytest.raises(AttributeError, match="no attribute"):
        orchestration.does_not_exist  # noqa: B018
