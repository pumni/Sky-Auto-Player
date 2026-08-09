"""Tests for the RegisterHotKey playback control contract."""

from __future__ import annotations

import pytest

from sky_music.infrastructure.hotkeys import PlaybackControls, parse_hotkey
from sky_music.platform.win32.global_hotkeys import (
    HOTKEY_ID_BASE,
    MOD_CONTROL,
    MOD_NOREPEAT,
    GlobalHotkeyConflictError,
    GlobalHotkeyError,
    GlobalHotkeyListener,
    build_registrations,
)


def test_build_registrations_is_deterministic_and_uses_norepeat() -> None:
    registrations = build_registrations(
        {
            "pause": parse_hotkey("ctrl+f8"),
            "quit": parse_hotkey("f10"),
        }
    )
    assert [item.action for item in registrations] == ["pause", "quit"]
    assert registrations[0].identifier == HOTKEY_ID_BASE
    assert registrations[0].modifiers == MOD_CONTROL | MOD_NOREPEAT


def test_duplicate_global_chord_is_rejected_before_registration() -> None:
    with pytest.raises(ValueError, match="duplicate hotkey chord"):
        build_registrations(
            {
                "pause": parse_hotkey("f8"),
                "quit": parse_hotkey("f8"),
            }
        )


def test_listener_queue_is_consumed_without_key_state_polling() -> None:
    listener = GlobalHotkeyListener.from_bindings({"pause": parse_hotkey("f8")})
    listener._events.put("pause")  # type: ignore[attr-defined]
    assert listener.poll() == "pause"
    assert listener.poll() is None


def test_registration_conflicts_have_a_specific_error_type() -> None:
    assert issubclass(GlobalHotkeyConflictError, RuntimeError)


def _controls() -> PlaybackControls:
    return PlaybackControls(
        pause=parse_hotkey("f8"),
        skip=parse_hotkey("f9"),
        quit=parse_hotkey("f10"),
        refocus=parse_hotkey("f11"),
        panic=parse_hotkey("f12"),
    )


def test_playback_controls_require_registration_before_polling() -> None:
    controls = _controls()
    with pytest.raises(GlobalHotkeyError, match="not registered"):
        controls.poll()


def test_registration_conflict_is_a_startup_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    class ConflictListener:
        @classmethod
        def from_bindings(cls, _bindings: object) -> ConflictListener:
            return cls()

        def start(self) -> None:
            raise GlobalHotkeyConflictError("F8 is already registered")

    monkeypatch.setattr("sky_music.infrastructure.hotkeys.GlobalHotkeyListener", ConflictListener)
    with pytest.raises(GlobalHotkeyConflictError, match="already registered"):
        _controls().start()


def test_message_loop_failure_is_not_silently_ignored() -> None:
    listener = GlobalHotkeyListener.from_bindings({"pause": parse_hotkey("f8")})
    listener._error = GlobalHotkeyError("message loop failed")  # type: ignore[attr-defined]
    with pytest.raises(GlobalHotkeyError, match="message loop failed"):
        listener.poll()
