"""Responsive / scroll behaviour of the command palette in short terminals.

The command modal is built around ``CommandPaletteList`` (an ``OptionList``
subclass) so that Textual gives us a real scrollbar, mouse-wheel, PageUp /
PageDown, Home / End and automatic keep-highlight-in-view. These tests assert
the *contract* the user actually relies on when the terminal is too small to
show the entire option list at once:

- The modal still mounts and exposes a ``CommandPaletteList`` with the full
  set of commands even when the viewport is only ~15 rows tall.
- Highlighting the bottom-most command (here ``update``) scrolls it into view
  rather than letting it sit invisibly below the viewport.
- ``PageDown`` / ``PageUp`` remain routed to OptionList (we explicitly do NOT
  stop those keys in ``CommandModal.on_key``), proving the modern scroll UX is
  preserved by the modal even though the filter input is the focused widget.

Driving these through ``app.run_test(size=...)`` is what makes ``size`` real
on the rendered widgets — ``monkeypatch.setattr(SkyPickerApp, "size", ...)``
fakes the value but does not flow down to ``CommandPaletteList`` size which is
what determines the scrollbar visibility.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from textual.widgets import Input

from sky_music.config import AppConfig
from sky_music.ui.textual_app import app as app_module
from sky_music.ui.textual_app.app import SkyPickerApp
from sky_music.ui.textual_app.keymap import COMMANDS
from sky_music.ui.textual_app.widgets import CommandPaletteList


class _ShortTerminalMetadataCoordinator:
    """Minimal stand-in for ``MetadataCoordinator`` so we can mount the picker
    app headlessly without touching real metadata refresh. Mirrors the shape
    of the FakeMetadataCoordinator used in test_textual_picker / main selftest.
    """

    instances: list[_ShortTerminalMetadataCoordinator] = []

    def __init__(self, *_args: Any, **_kwargs: Any) -> None:
        self.refreshed: list[list[Path]] = []
        self.close_waits: list[bool] = []
        self.shutdown_started = False
        self.closed = False
        self.instances.append(self)

    @property
    def name(self) -> str:
        return "responsive-test-metadata"

    @property
    def phase(self) -> str:
        return "picker"

    def refresh(self, paths: list[Path]) -> None:
        self.refreshed.append(paths)

    def cancel(self) -> None:
        self.shutdown_started = True

    def close(self, *, wait: bool = False) -> None:
        self.close_waits.append(wait)
        self.shutdown_started = True
        if wait:
            self.closed = True

    def snapshot(self) -> Any:
        from sky_music.infrastructure.background import WorkerSnapshot

        return WorkerSnapshot(
            name=self.name,
            phase=self.phase,
            closed=self.closed,
            pending_count=0,
            running_count=0,
        )


def _build_app(monkeypatch: Any, *, headless_songs: bool = True) -> SkyPickerApp:
    """Construct a SkyPickerApp with the song metadata fake replaced so the
    command palette can be opened without a real songs background worker.
    """
    _ShortTerminalMetadataCoordinator.instances.clear()
    monkeypatch.setattr(
        app_module, "get_song_choices", lambda force_refresh=False: []
    )
    monkeypatch.setattr(
        app_module, "MetadataCoordinator", _ShortTerminalMetadataCoordinator
    )
    return SkyPickerApp(initial_dry_run=True, cfg=AppConfig())


# ── Mount + content presence in a 15-row terminal ──────────────────────────


def test_command_modal_mounts_in_short_terminal(monkeypatch: Any) -> None:
    """A 15-row terminal must still render the modal and its command palette
    with every command reachable via scrolling. The test only verifies mount +
    presence — visual layout is asserted by inspecting options rather than
    the rendered surface (which depends on Textual rendering internals).
    """
    import asyncio

    app = _build_app(monkeypatch)

    async def run() -> None:
        async with app.run_test(size=(80, 15)) as pilot:
            await pilot.pause()
            app.action_open_commands()
            await pilot.pause()
            assert type(app.screen).__name__ == "CommandModal"
            palette = app.screen.query_one("#modal-options", CommandPaletteList)
            # Every command is, at minimum, *present* in the option list —
            # scrolling reveals them, not "they got dropped".
            option_ids = [getattr(opt, "id", "") or "" for opt in palette._options]  # type: ignore[attr-defined]
            for cmd in COMMANDS:
                assert f"cmd:{cmd.id}" in option_ids, f"missing option for {cmd.id}"
            await pilot.press("escape")

    asyncio.run(run())
    assert app.return_value is None


# ── Highlight + auto-scroll in short terminals ──────────────────────────────


def test_bottom_command_scrolls_into_view_in_short_terminal(monkeypatch: Any) -> None:
    """Highlighting the last command on a short terminal must auto-scroll it
    into view — this is the core UX guarantee ``watch_highlighted -> scroll_to_highlight``
    provides for any OptionList subclass once the palette itself has a
    scrollbar. ``CommandModal.on_modal_mounted`` sizes the palette's
    ``max-height`` against the viewport so an OptionList-native scrollbar
    appears (rather than the palette overflow into the modal-content area).
    """
    import asyncio

    app = _build_app(monkeypatch)

    async def run() -> None:
        async with app.run_test(size=(80, 15)) as pilot:
            await pilot.pause()
            app.action_open_commands()
            await pilot.pause()
            palette = app.screen.query_one("#modal-options", CommandPaletteList)
            # The palette must overflow on a 15-row terminal: OptionList's
            # native scrollbar kicks in, otherwise scroll_to_highlight has
            # nowhere to scroll to.
            assert palette.max_scroll_y > 0, "palette must scroll when options overflow the short viewport"
            starting_scroll = palette.scroll_y
            # Jump the OptionList highlight to the last selectable option.
            palette.action_last()
            await pilot.pause()
            assert palette.highlighted is not None
            cmd = palette._command_for_option_index(palette.highlighted)
            assert cmd is not None and cmd.id == "update"
            # scroll_to_highlight (fired from watch_highlighted) must have
            # advanced the viewport so the bottom row is in view.
            assert palette.scroll_y > starting_scroll, (
                "highlight-at-tail did not auto-scroll the palette viewport"
            )
            await pilot.press("escape")

    asyncio.run(run())
    assert app.return_value is None


# ── PageDown scrolls the palette rather than rerouting ─────────────────────


def test_pagedown_from_filter_input_moves_palette_view(monkeypatch: Any) -> None:
    """PageDown pressed while the filter input has focus must still bubble down
    to the OptionList so the user can page through commands — proving the
    modal's ``on_key`` does not swallow scroll keys it should not own.
    ``CommandModal.on_key`` forwards PageUp/PageDown/Home/End to OptionList's
    own actions while focus sits on the filter input.
    """
    import asyncio

    app = _build_app(monkeypatch)

    async def run() -> None:
        async with app.run_test(size=(80, 20)) as pilot:
            await pilot.pause()
            app.action_open_commands()
            await pilot.pause()
            palette = app.screen.query_one("#modal-options", CommandPaletteList)
            # Default focus is the filter input (per CommandModal.on_modal_mounted).
            assert isinstance(app.screen.focused, Input)
            assert palette.max_scroll_y > 0, "test requires a scrollable palette"
            starting_scroll = palette.scroll_y
            starting_highlight = palette.highlighted
            await pilot.press("pagedown")
            await pilot.pause()
            # PageDown must either advance the viewport, the highlighted row,
            # or both — at least one of these should differ if PageDown acted.
            assert palette.scroll_y > starting_scroll or (
                palette.highlighted is not None and palette.highlighted != starting_highlight
            ), "PageDown did not move the palette cursor or viewport"
            await pilot.press("escape")

    asyncio.run(run())
    assert app.return_value is None


# ── Home / End owned by OptionList when palette has focus ───────────────────


def test_home_end_jump_to_first_and_last_command(monkeypatch: Any) -> None:
    """Home / End while focus is in the palette must use OptionList's own
    action_first / action_last so the modern 'jump to top / bottom' UX works
    just like in plain OptionList modals.
    """
    import asyncio

    app = _build_app(monkeypatch)

    async def run() -> None:
        async with app.run_test(size=(80, 20)) as pilot:
            await pilot.pause()
            app.action_open_commands()
            await pilot.pause()
            palette = app.screen.query_one("#modal-options", CommandPaletteList)
            # Give focus to the palette itself — then OptionList owns the keys.
            app.screen.set_focus(palette)
            await pilot.pause()
            await pilot.press("end")
            await pilot.pause()
            assert palette.highlighted is not None
            cmd = palette._command_for_option_index(palette.highlighted)
            assert cmd is not None and cmd.id == "update"
            await pilot.press("home")
            await pilot.pause()
            assert palette.highlighted is not None
            cmd_top = palette._command_for_option_index(palette.highlighted)
            assert cmd_top is not None and cmd_top.id == COMMANDS[0].id
            await pilot.press("escape")

    asyncio.run(run())
    assert app.return_value is None
