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

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

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

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

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

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

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


def test_update_settings_modal_clear_skip_version(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """When a skip-version is configured, the modal exposes a Button that clears
    it via the registered callback.
    """
    from textual.widgets import Button

    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

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

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

    cfg = AppConfig(update=UpdateSettings(auto_check=False))

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

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

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

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

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
