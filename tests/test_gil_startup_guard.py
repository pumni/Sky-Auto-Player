"""Tests for the no-GIL startup fail-fast guard (review of main@7c548527 §3).

The free-threaded-interpreter pair (``.python-version`` ↔ ``pyproject.toml
requires-python``) is an architecture invariant: the dispatch thread and the Textual UI
thread must not contend on the GIL. The legacy code only recorded GIL state to telemetry
and tuned switch-interval accordingly; it never refused playback. These tests cover the
``assert_free_threaded_runtime`` guard that fails fast BEFORE the UI/backend are built:

    * free-threaded build + GIL runtime-disabled (the happy path) → no exception.
    * non-free-threaded build (``Py_GIL_DISABLED`` != 1) → FreeThreadedRuntimeError.
    * free-threaded build but GIL runtime-on (an incompatible extension or startup flag)
      → FreeThreadedRuntimeError.

We monkey-patch ``sysconfig.get_config_var`` and ``sys._is_gil_enabled`` (visible to
``realtime._gil_enabled`` via ``getattr(sys, "_is_gil_enabled")``) — production callers see
the real interpreter, tests see our synthetic models.
"""
from __future__ import annotations

import pytest

from sky_music.infrastructure.realtime import (
    FreeThreadedRuntimeError,
    assert_free_threaded_runtime,
)


def _set_env(monkeypatch: pytest.MonkeyPatch, *, build_disabled: int | None, runtime_gil: bool) -> None:
    """Patch the two probes ``assert_free_threaded_runtime`` consults.

    ``sysconfig.get_config_var`` is imported lazily inside the guard, so we patch the real
    ``sysconfig`` module (the guard will see the patched version when it executes
    ``import sysconfig``). ``realtime._gil_enabled()`` reads via ``getattr(sys,
    "_is_gil_enabled", None)``; we install a callable on ``sys`` so it comes back through.
    """
    import sys
    import sysconfig

    monkeypatch.setattr(sysconfig, "get_config_var", lambda _name: build_disabled, raising=True)

    def fake_is_gil_enabled() -> bool:
        return runtime_gil

    monkeypatch.setattr(sys, "_is_gil_enabled", fake_is_gil_enabled, raising=False)


def test_assert_passes_when_build_is_free_threaded_and_gil_disabled(monkeypatch: pytest.MonkeyPatch) -> None:
    _set_env(monkeypatch, build_disabled=1, runtime_gil=False)
    # Must not raise — the happy path under ``uv run`` and on the free-threaded build.
    assert_free_threaded_runtime()


def test_assert_fails_when_build_is_not_free_threaded(monkeypatch: pytest.MonkeyPatch) -> None:
    # A normal CPython build (Py_GIL_DISABLED == 0). Surfaces on developer machines that
    # forgot to honour the .python-version pin via uv.
    _set_env(monkeypatch, build_disabled=0, runtime_gil=True)
    with pytest.raises(FreeThreadedRuntimeError, match="Py_GIL_DISABLED"):
        assert_free_threaded_runtime()


def test_assert_fails_when_build_is_free_threaded_but_gil_reenabled(monkeypatch: pytest.MonkeyPatch) -> None:
    # A free-threaded build that nonetheless has the GIL enabled at runtime — an
    # incompatible C extension can force this, or a startup flag like PYTHON_GIL=1.
    _set_env(monkeypatch, build_disabled=1, runtime_gil=True)
    with pytest.raises(FreeThreadedRuntimeError, match="re-enabled"):
        assert_free_threaded_runtime()


def test_assert_fails_when_sysconfig_var_is_missing(monkeypatch: pytest.MonkeyPatch) -> None:
    # Older / exotic interpreters return None for the var — treat as not-free-threaded,
    # fail fast with the actionable build message.
    _set_env(monkeypatch, build_disabled=None, runtime_gil=True)
    with pytest.raises(FreeThreadedRuntimeError, match="Py_GIL_DISABLED"):
        assert_free_threaded_runtime()
