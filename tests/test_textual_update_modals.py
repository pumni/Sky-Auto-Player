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


def test_update_progress_modal_mounts_without_total(monkeypatch: pytest.MonkeyPatch) -> None:
    """``UpdateProgressModal`` must mount cleanly when total size is unknown
    (e.g. server omitted Content-Length). The progress bar uses
    indeterminate advance in that case.
    """
    from sky_music.ui.textual_app.modals import UpdateProgressModal

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateProgressModal(
            latest_version="2.3.2",
            current_version="2.3.1",
            total=None,
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        # update_progress with unknown total must not raise.
        modal.update_progress(2 * 1024 * 1024, None)
        modal.update_progress(4 * 1024 * 1024, None)
        bar = modal.query_one("#update-progress-bar")
        assert bar is not None
        await pilot.press("escape")

    _run(_with_app(actions))


def test_update_settings_modal_persists_toggles(monkeypatch: pytest.MonkeyPatch) -> None:
    """Toggling a row in UpdateSettingsModal must call the corresponding
    persistence callback and re-render the row with the new checkbox.
    """
    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

    auto_check_calls: list[bool] = []
    auto_apply_calls: list[bool] = []

    async def actions(app: app_module.SkyPickerApp, pilot: Any) -> None:
        modal = UpdateSettingsModal(
            auto_check=True,
            auto_apply=False,
            on_auto_check=auto_check_calls.append,
            on_auto_apply=auto_apply_calls.append,
            theme_name="aurora",
        )
        app.push_screen(modal)
        await pilot.pause()
        # Highlight is on row 0 (auto_check) — press enter to toggle to False.
        await pilot.press("enter")
        assert auto_check_calls == [False]
        # Move to row 1 and toggle auto_apply True.
        await pilot.press("down")
        await pilot.press("enter")
        assert auto_apply_calls == [True]
        # Modal remains open until Esc.
        await pilot.press("escape")

    _run(_with_app(actions))


def test_open_update_settings_modal_pushes_screen_with_current_cfg(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """``on_picker_open_update_settings`` opens the settings modal configured
    from the live ``cfg.update`` values.
    """
    from sky_music.config import AppConfig, UpdateSettings
    from sky_music.ui.textual_app.modals import UpdateSettingsModal

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [])

    cfg = AppConfig(update=UpdateSettings(auto_check=False, auto_apply=True))

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
        assert modal._auto_apply is True
        # Toggle auto_check path: the callbacks are wired to persist_* — just
        # verify they don't raise when toggled from app context.
        modal._toggle_current()
        assert modal._auto_check is True  # flipped from False to True

    _run(_with_app(actions, cfg=cfg))
