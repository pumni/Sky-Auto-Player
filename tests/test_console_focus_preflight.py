from __future__ import annotations

from unittest.mock import Mock

from rich.style import Style


def _patch_preflight(monkeypatch, console_playback, active: bool) -> Mock:
    monkeypatch.setattr(
        console_playback,
        "_resolve_cli_theme_style",
        lambda: Style.parse("#22c55e"),
    )
    monkeypatch.setattr(
        console_playback.doctor,
        "check_sky_window",
        lambda: {"ok": True, "msg": "Sky found"},
    )
    monkeypatch.setattr(
        console_playback.doctor,
        "check_timer_resolution",
        lambda: {"ok": True, "msg": "Timer active"},
    )
    monkeypatch.setattr(
        console_playback.doctor,
        "check_physical_keys_held",
        lambda: {"ok": True, "held_keys": []},
    )
    monkeypatch.setattr(console_playback, "_build_preflight_panel", lambda *_args: None)
    monkeypatch.setattr(console_playback._window_target, "is_sky_active", lambda: active)
    focus_window = Mock()
    monkeypatch.setattr(console_playback._window_target, "focus_window", focus_window)
    monkeypatch.setattr(console_playback.time, "sleep", lambda _seconds: None)
    return focus_window


def test_startup_preflight_does_not_focus_window(monkeypatch) -> None:
    import sky_music.cli.console_playback as console_playback

    focus_window = _patch_preflight(monkeypatch, console_playback, active=True)

    assert console_playback._mini_preflight(False) is True
    focus_window.assert_not_called()


def test_explicit_retry_can_refocus(monkeypatch) -> None:
    import sky_music.cli.console_playback as console_playback

    focus_window = _patch_preflight(monkeypatch, console_playback, active=False)
    active_states = iter((False, True))
    monkeypatch.setattr(
        console_playback._window_target,
        "is_sky_active",
        lambda: next(active_states),
    )
    monkeypatch.setattr("builtins.input", lambda _prompt: "r")

    assert console_playback._mini_preflight(False) is True
    focus_window.assert_called_once_with()
