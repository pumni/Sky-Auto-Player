from __future__ import annotations

import sys
from types import SimpleNamespace

import pytest

from sky_music.domain import Song
from sky_music.domain.scheduler_types import (
    ActionKind,
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.infrastructure.backend import WinSendInputBackend
from sky_music.orchestration import native_dispatch
from sky_music.orchestration.engine import PlaybackEngine


def _available_probe() -> native_dispatch.NativeProbeResult:
    return native_dispatch.NativeProbeResult(
        available=True,
        reason=native_dispatch.NativeProbeReason.AVAILABLE,
        detail="test native dispatch",
    )


def _production_engine(*, chord_stagger_us: int = 0) -> PlaybackEngine:
    return PlaybackEngine(
        song=Song(name="native-selection", notes=()),
        actions=(
            KeyAction(
                kind=ActionKind.DOWN,
                scan_codes=(ScanCode(0x15),),
                at_us=Microseconds(1_000),
            ),
        ),
        backend=WinSendInputBackend(),
        require_focus=False,
        chord_stagger_us=chord_stagger_us,
    )


def test_native_dispatch_legacy_feature_flag_selects_real_windows_path(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.setenv("SKY_USE_RUST_DISPATCH", "1")
    monkeypatch.setattr(native_dispatch, "probe_native_dispatch", lambda: _available_probe())
    assert _production_engine()._should_use_native_dispatch() is True


def test_native_dispatch_missing_extension_fails_closed_by_default(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.delenv("SKY_REQUIRE_RUST_DISPATCH", raising=False)
    monkeypatch.setattr(
        native_dispatch,
        "probe_native_dispatch",
        lambda: native_dispatch.NativeProbeResult(
            available=False,
            reason=native_dispatch.NativeProbeReason.MODULE_NOT_FOUND,
            detail="test missing native module",
        ),
    )
    with pytest.raises(RuntimeError, match="Native Rust dispatch is unavailable"):
        _production_engine()._should_use_native_dispatch()


def test_native_dispatch_required_mode_fails_closed(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.setenv("SKY_REQUIRE_RUST_DISPATCH", "1")
    monkeypatch.setattr(
        native_dispatch,
        "probe_native_dispatch",
        lambda: native_dispatch.NativeProbeResult(
            available=False,
            reason=native_dispatch.NativeProbeReason.MODULE_NOT_FOUND,
            detail="test missing native module",
        ),
    )
    with pytest.raises(RuntimeError, match="Native Rust dispatch is unavailable"):
        _production_engine()._should_use_native_dispatch()


def test_python_dispatch_requires_explicit_rollback_switch(monkeypatch) -> None:
    monkeypatch.setenv("SKY_USE_RUST_DISPATCH", "1")
    monkeypatch.setenv("SKY_USE_PYTHON_DISPATCH", "1")
    monkeypatch.setattr(native_dispatch, "probe_native_dispatch", lambda: _available_probe())
    assert _production_engine()._should_use_native_dispatch() is False


def test_native_dispatch_is_default_when_eligible(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.delenv("SKY_USE_RUST_DISPATCH", raising=False)
    monkeypatch.setattr(native_dispatch, "probe_native_dispatch", lambda: _available_probe())
    assert _production_engine()._should_use_native_dispatch() is True


def test_auto_dispatch_uses_python_for_staggered_chords(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.delenv("SKY_USE_RUST_DISPATCH", raising=False)
    monkeypatch.setattr(native_dispatch, "is_native_dispatch_available", lambda: True)
    engine = _production_engine(chord_stagger_us=100)
    assert engine._should_use_native_dispatch() is False
    assert (
        engine.telemetry.runtime_options["rust_dispatch_fallback_reason"]
        == "chord_stagger_us is unsupported by the native atomic-chord capability"
    )


def test_native_dispatch_rejects_stale_build_id(monkeypatch) -> None:
    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(
            build_info=lambda: {
                "schema_version": 1,
                "native_schema_version": 1,
                "native_abi": "cp314t-win_amd64",
                "native_build_commit": "old-commit",
                "native_source_fingerprint": "fingerprint",
                "free_threaded": True,
                "win32_backend": True,
            }
        ),
    )
    monkeypatch.setattr(native_dispatch, "_expected_native_build_id", lambda: "new-commit")
    monkeypatch.setattr(native_dispatch, "_expected_native_source_fingerprint", lambda: "fingerprint")
    monkeypatch.setattr(sys, "_is_gil_enabled", lambda: False, raising=False)
    native_dispatch.reset_native_dispatch_availability_cache()
    assert native_dispatch.is_native_dispatch_available() is False


def test_native_dispatch_accepts_exact_build_id_and_abi(monkeypatch) -> None:
    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(
            build_info=lambda: {
                "schema_version": 1,
                "native_schema_version": 1,
                "native_abi": "cp314t-win_amd64",
                "native_build_commit": "new-commit",
                "native_source_fingerprint": "fingerprint",
                "free_threaded": True,
                "win32_backend": True,
            }
        ),
    )
    monkeypatch.setattr(native_dispatch, "_expected_native_build_id", lambda: "new-commit")
    monkeypatch.setattr(native_dispatch, "_expected_native_source_fingerprint", lambda: "fingerprint")
    monkeypatch.setattr(sys, "_is_gil_enabled", lambda: False, raising=False)
    native_dispatch.reset_native_dispatch_availability_cache()
    assert native_dispatch.is_native_dispatch_available() is True
