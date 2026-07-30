from __future__ import annotations

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


def _production_engine() -> PlaybackEngine:
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
    )


def test_native_dispatch_feature_flag_selects_real_windows_path(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.setenv("SKY_USE_RUST_DISPATCH", "1")
    monkeypatch.setattr(native_dispatch, "is_native_dispatch_available", lambda: True)
    assert _production_engine()._should_use_native_dispatch() is True


def test_native_dispatch_missing_extension_fails_closed(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.setenv("SKY_USE_RUST_DISPATCH", "1")
    monkeypatch.setattr(native_dispatch, "is_native_dispatch_available", lambda: False)
    with pytest.raises(RuntimeError, match="Native Rust dispatch is unavailable"):
        _production_engine()._should_use_native_dispatch()


def test_python_dispatch_requires_explicit_rollback_switch(monkeypatch) -> None:
    monkeypatch.setenv("SKY_USE_RUST_DISPATCH", "1")
    monkeypatch.setenv("SKY_USE_PYTHON_DISPATCH", "1")
    monkeypatch.setattr(native_dispatch, "is_native_dispatch_available", lambda: True)
    assert _production_engine()._should_use_native_dispatch() is False


def test_python_dispatch_remains_default_until_soak_signoff(monkeypatch) -> None:
    monkeypatch.delenv("SKY_USE_PYTHON_DISPATCH", raising=False)
    monkeypatch.delenv("SKY_USE_RUST_DISPATCH", raising=False)
    monkeypatch.setattr(native_dispatch, "is_native_dispatch_available", lambda: True)
    assert _production_engine()._should_use_native_dispatch() is False
