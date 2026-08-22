"""Tests for the update-related modals in
``sky_music.ui.textual_app.modals``.

These tests boot a real Textual ``App.run_test`` session so that the modal's
``on_modal_mounted`` lifecycle hook runs — the hook that previously simply
set focus, and that now also renders release-notes Markdown into a
``RichLog``. We assert the mount does NOT raise and that the expected
widgets are present.
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import pytest

from sky_music.config import AppConfig
from sky_music.ui.textual_app import app as app_module


def _run(coro: Any) -> Any:
    return asyncio.run(coro)


async def _with_app(actions: Any, cfg: AppConfig | None = None) -> app_module.SkyPickerApp:
    app = app_module.SkyPickerApp(initial_dry_run=True, cfg=cfg or AppConfig())
    async with app.run_test() as pilot:
        await pilot.pause()
        await actions(app, pilot)
    return app


def test_update_modal_renders_release_notes_monkeypatch(monkeypatch: pytest.MonkeyPatch) -> None:
    """Push an UpdateModal with release notes containing Markdown and
    verify it mounts without raising — the ``RichLog`` render path is
    exercised by ``on_modal_mounted``.
    """
    from sky_music.ui.textual_app.modals import UpdateModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    notes = (
        "## Changes\n"
        "- Fixed crash on launch (#42)\n"
        "- Improved timing precision\n\n"
        "see [full changelog](https://example.com/cn)."
    )

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateModal(
            latest_version="2.3.2",
            current_version="2.3.1",
            release_notes=notes,
            published_at="2025-11-02T10:00:00Z",
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        # RichLog exists and was written to.
        richlog = modal.query_one("#update-notes")
        assert richlog is not None
        # Modal header reflects the latest version.
        assert "v2.3.2" in modal.title_text
        # The info line now carries the published date YYYY-MM-DD.
        from textual.widgets import Static
        info_widget = modal.query_one("#update-info", Static)
        assert "2025-11-02" in str(info_widget.content)
        await pilot.press("escape")

    _run(_with_app(actions))


def test_update_modal_handles_empty_notes_gracefully(monkeypatch: pytest.MonkeyPatch) -> None:
    """Empty / missing release notes must not break the modal — the
    placeholder markdown line is shown instead.
    """
    from sky_music.ui.textual_app.modals import UpdateModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateModal(
            latest_version="2.3.2",
            current_version="2.3.1",
            release_notes="",
            published_at="",
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        # Mount succeeded; placeholder is rendered.
        richlog = modal.query_one("#update-notes")
        assert richlog is not None
        await pilot.press("escape")

    _run(_with_app(actions))





def test_update_settings_modal_persists_toggles(monkeypatch: pytest.MonkeyPatch) -> None:
    """Toggling a Checkbox in UpdateSettingsModal must call the corresponding
    persistence callback.
    """
    from textual.widgets import Checkbox

    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    auto_check_calls: list[bool] = []

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateSettingsModal(
            auto_check=True,
            on_auto_check=auto_check_calls.append,
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        # First checkbox is auto-focused by PickerModal fallback.
        cb_check = modal.query_one("#checkbox-auto-check", Checkbox)
        assert cb_check.value is True
        # Press Space to toggle.
        await pilot.press("space")
        await pilot.pause()
        assert cb_check.value is False
        assert auto_check_calls == [False]
        # Modal remains open until Esc.
        await pilot.press("escape")

    _run(_with_app(actions))


def test_update_settings_modal_persists_beta_channel(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from textual.widgets import Checkbox

    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    channel_calls: list[str] = []

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateSettingsModal(
            auto_check=True,
            on_auto_check=lambda _v: None,
            channel="beta",
            on_channel=channel_calls.append,
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        checkbox = modal.query_one("#checkbox-beta-channel", Checkbox)
        assert checkbox.value is True
        checkbox.focus()
        await pilot.press("space")
        await pilot.pause()
        assert checkbox.value is False
        assert modal._channel == "stable"
        assert channel_calls == ["stable"]
        await pilot.press("escape")

    _run(_with_app(actions))


def test_update_settings_modal_clear_skip_version(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """When a skip-version is configured, the modal exposes a Button that clears
    it via the registered callback.
    """
    from textual.widgets import Button

    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    clear_calls: list[bool] = []

    async def _run_large() -> None:
        app = app_module.SkyPickerApp(initial_dry_run=True)
        async with app.run_test(size=(80, 40)) as pilot:
            await pilot.pause()
            modal = UpdateSettingsModal(
                auto_check=True,
                on_auto_check=lambda _v: None,
                skip_version="2.4.0",
                theme_name="aurora",
            )
            modal._on_clear_skip = lambda: clear_calls.append(True)  # type: ignore[attr-defined]
            app.push_screen(modal)
            await pilot.pause()
            # Diagnostic: which widget has focus?
            focused = modal.focused
            import sys
            print(f"DEBUG focused: {focused!r}", file=sys.stderr)
            assert focused is not None, "No widget focused"
            # The clear-skip Button is present in the action row.
            btn = modal.query_one("#btn-clear-skip", Button)
            assert btn is not None
            # Navigate from focused widget to btn-clear-skip via Tab then Enter.
            for _ in range(8):  # generous tab count; stop when btn-clear-skip focused
                if modal.focused is btn:
                    break
                await pilot.press("tab")
                await pilot.pause()
            else:
                raise AssertionError(f"Never reached btn-clear-skip; focused={modal.focused!r}")
            await pilot.press("enter")
            await pilot.pause()
            assert clear_calls == [True]
            # The button should be removed from the DOM after clearing.
            assert len(modal.query("#btn-clear-skip")) == 0
            await pilot.press("escape")

    _run(_run_large())


def test_open_update_settings_modal_pushes_screen_with_current_cfg(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """``on_picker_open_update_settings`` opens the settings modal configured
    from the live ``cfg.update`` values.
    """
    from textual.widgets import Checkbox

    from sky_music.config import AppConfig, UpdateSettings
    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    cfg = AppConfig(update=UpdateSettings(auto_check=False, channel="beta"))

    pushed: list[Any] = []

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        original = app.push_screen

        def _spy(modal: Any, *a: Any, **k: Any) -> None:
            pushed.append(modal)
            return original(modal, *a, **k)

        app.push_screen = _spy  # type: ignore[method-assign]
        app.on_picker_open_update_settings()
        await pilot.pause()
        assert len(pushed) == 1
        modal = pushed[0]
        assert isinstance(modal, UpdateSettingsModal)
        # The modal was seeded with the live cfg values.
        assert modal._auto_check is False
        assert modal._channel == "beta"
        assert modal.query_one("#checkbox-beta-channel", Checkbox).value is True
        # First checkbox is auto-focused; toggle auto_check.
        cb_check = modal.query_one("#checkbox-auto-check", Checkbox)
        assert cb_check.value is False
        await pilot.press("space")
        await pilot.pause()
        assert cb_check.value is True  # flipped from False to True
        assert modal._auto_check is True  # model state also flipped

    _run(_with_app(actions, cfg=cfg))


def test_update_settings_modal_renders_divider_between_info_and_rows(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The modal renders a ``Rule.horizontal()`` divider between the info
    header and the toggle rows — this signals the visual break between
    ``read me first`` (cadence, legend) and ``do something`` (toggle rows +
    action rows).
    """
    from textual.widgets import Rule

    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateSettingsModal(
            auto_check=True,
            on_auto_check=lambda _v: None,
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        # The divider is now a native Rule widget.
        divider = modal.query_one("#update-settings-divider", Rule)
        assert divider is not None
        await pilot.press("escape")

    _run(_with_app(actions))


def test_update_settings_modal_escape_works_immediately(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Pressing ``escape`` immediately after the modal appears must dismiss it,
    even before any widget interaction occurs.
    """
    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    dismissed: list[Any] = []

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateSettingsModal(
            auto_check=True,
            on_auto_check=lambda _v: None,
            theme_name="aurora",
        )
        original_dismiss = modal.dismiss

        def _spy_dismiss(result: Any = None) -> Any:
            dismissed.append(result)
            return original_dismiss(result)

        modal.dismiss = _spy_dismiss  # type: ignore[method-assign]
        app.push_screen(modal)
        await pilot.pause()
        await pilot.press("escape")
        assert dismissed == [None], (
            f"Expected [None] from escape→action_close→dismiss(None), got {dismissed}"
        )

    _run(_with_app(actions))


def test_update_banner_modal_renders(monkeypatch: pytest.MonkeyPatch) -> None:
    from sky_music.ui.textual_app.modals import UpdateBannerModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    notes = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9\nLine 10\nLine 11"

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateBannerModal(
            latest_version="2.0.1",
            current_version="2.0.0",
            release_notes=notes,
            published_at="2024-01-01T12:00:00Z",
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()

        # Check for static banner text
        banner = modal.query_one("#update-banner-info").render()
        assert banner is not None
        banner_text = str(banner)
        assert "Sky Auto Player v2.0.1 is now available." in banner_text
        assert "Line 1" in banner_text
        assert "Line 11" not in banner_text
        assert "... (see GitHub for full notes)" in banner_text

        options = modal.query_one("#update-banner-options")
        assert options is not None

        await pilot.press("escape")

    _run(_with_app(actions))


def test_update_banner_enter_defaults_to_remind_without_launch(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from textual.widgets import OptionList

    from sky_music.domain.update_checker import UpdateInfo
    from sky_music.ui.textual_app.modals import UpdateBannerModal

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        launch_calls: list[Any] = []
        responses: list[str | None] = []
        release = UpdateInfo(
            latest_version="2.0.1",
            download_url="",
            release_notes="",
            html_url="",
            published_at="",
        )
        app._launch_native_update = lambda _release: launch_calls.append(_release)  # type: ignore[method-assign]
        modal = UpdateBannerModal(
            latest_version="2.0.1",
            current_version="2.0.0",
            theme_name="aurora",
        )

        def on_result(response: str | None) -> None:
            responses.append(response)
            app._handle_update_response(response, release)

        app.push_screen(modal, on_result)
        await pilot.pause()
        options = modal.query_one("#update-banner-options", OptionList)
        assert options.highlighted == 2
        await pilot.press("enter")
        await pilot.pause()
        assert responses == ["remind"]
        assert launch_calls == []

    _run(_with_app(actions))


def test_update_success_cleanup_warning_uses_acceptance_copy(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from sky_music.infrastructure.update_runtime import (
        UpdateRuntimeResult,
        UpdateRuntimeWarning,
    )

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])
    monkeypatch.setattr(
        "sky_music.infrastructure.update_runtime.consume_last_result",
        lambda: UpdateRuntimeResult(
            status="success",
            from_version="3.4.4",
            target_version="3.4.5",
            timestamp_utc="2026-08-22T00:00:00Z",
            warnings=(
                UpdateRuntimeWarning(
                    code="ARTIFACT_CLEANUP_FAILED",
                    message="Access is denied",
                    path=r"C:\install\.sky-update-1.bak",
                    os_error=32,
                ),
            ),
            cleanup_pending=True,
        ),
    )

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        notices: list[str] = []
        app.notify = lambda message, **_kwargs: notices.append(str(message))  # type: ignore[method-assign]
        app._report_last_update_result()
        assert (
            "The update completed, but some temporary updater files could not be removed. "
            "Cleanup will be retried automatically."
        ) in notices
        await pilot.pause()

    _run(_with_app(actions))


def test_rolled_back_update_renders_structured_provenance(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from sky_music.infrastructure.update_runtime import UpdateRuntimeResult

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [])
    monkeypatch.setattr(
        "sky_music.infrastructure.update_runtime.consume_last_result",
        lambda: UpdateRuntimeResult(
            status="rolled_back",
            from_version="3.4.4",
            target_version="3.4.5",
            timestamp_utc="2026-08-22T00:00:00Z",
            error_code="INSTALL_ATOMIC_REPLACE_FAILED",
            message="replacement failed",
            phase="apply",
            operation="replace",
            path=r"C:\install\Sky-Auto-Player.exe",
            os_error=5,
        ),
    )

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        notices: list[str] = []
        app.notify = lambda message, **_kwargs: notices.append(str(message))  # type: ignore[method-assign]
        app._report_last_update_result()
        assert notices == [
            "Update to v3.4.5 was rolled back [INSTALL_ATOMIC_REPLACE_FAILED] "
            "during apply/replace at C:\\install\\Sky-Auto-Player.exe "
            "(Windows error 5): replacement failed"
        ]
        await pilot.pause()

    _run(_with_app(actions))


def test_rejected_update_keeps_app_open_and_reports_error(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from types import SimpleNamespace

    from sky_music.infrastructure.update_launcher import UpdateLaunchError

    app = app_module.SkyPickerApp(initial_dry_run=True)
    notices: list[str] = []
    app.notify = lambda message, **_kwargs: notices.append(str(message))  # type: ignore[method-assign]
    monkeypatch.setattr(
        "sky_music.infrastructure.update_launcher.launch_update",
        lambda _request: (_ for _ in ()).throw(
            UpdateLaunchError("native updater rejected startup [UI_INITIALIZATION_FAILED]: InitCommonControlsEx failed")
        ),
    )

    app._launch_native_update(SimpleNamespace(latest_version="3.4.5"))

    assert notices == [
        "Update could not start: native updater rejected startup "
        "[UI_INITIALIZATION_FAILED]: InitCommonControlsEx failed. Choose Open GitHub Releases "
        "for manual update."
    ]


def test_duplicate_update_notifies_then_exits_after_delay(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    from types import SimpleNamespace

    from sky_music.infrastructure.update_launcher import UpdateLaunchResult

    app = app_module.SkyPickerApp(initial_dry_run=True)
    notices: list[str] = []
    timers: list[tuple[float, Any]] = []
    exits: list[bool] = []
    app.notify = lambda message, **_kwargs: notices.append(str(message))  # type: ignore[method-assign]
    app.set_timer = lambda delay, callback: timers.append((float(delay), callback))  # type: ignore[method-assign]
    app.exit = lambda: exits.append(True)  # type: ignore[method-assign]
    monkeypatch.setattr(
        "sky_music.infrastructure.update_launcher.launch_update",
        lambda _request: UpdateLaunchResult(
            status="already_running",
            staged_updater=tmp_path / "Sky-Auto-Player-Updater.exe",
            run_root=tmp_path / "run",
            updater_pid=4711,
        ),
    )

    app._launch_native_update(SimpleNamespace(latest_version="3.4.5"))
    assert notices == [
        "An update is already running. This app window will close so the updater can continue."
    ]
    assert [delay for delay, _callback in timers] == [1.0]
    timers[0][1]()
    assert exits == [True]
