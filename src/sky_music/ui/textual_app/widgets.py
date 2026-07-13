from __future__ import annotations

from typing import TYPE_CHECKING, Any

from rich.markup import escape
from rich.table import Table
from rich.text import Text
from textual import events
from textual.message import Message
from textual.widgets import OptionList, Static
from textual.widgets.option_list import Option

from sky_music.ui.picker_theme import pad_text, remove_accents
from sky_music.ui.text_render import cell_width
from sky_music.ui.textual_app.keymap import PICKER_HINTS, CommandSpec, KeyHint

if TYPE_CHECKING:
    from sky_music.ui.textual_app.app import SkyPickerApp


class GridRenderable(Table):
    """A helper class to wrap a Table with a .plain property for test compatibility."""

    def __init__(self, *args: Any, plain_text: str = "", **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._plain_text = plain_text

    @property
    def plain(self) -> str:
        return self._plain_text


class AppFooter(Static):
    """Clean app action bar/footer for the main picker screen."""

    def __init__(self, hints: list[KeyHint], *, key_color: str = "#facc15", muted_color: str = "#6b7a93", **kwargs: Any) -> None:
        Static.__init__(self, "", markup=True, **kwargs)
        self.hints = hints
        self.key_color = key_color
        self.muted_color = muted_color
        self._click_spans: list[tuple[int, int, str]] = []

    def set_theme(self, key_color: str, muted_color: str) -> AppFooter:
        self.key_color = key_color
        self.muted_color = muted_color
        self.refresh()
        return self

    def _render_hint(self, hint: KeyHint) -> Text:
        text = Text()
        text.append(" ")
        text.append(hint.key, style=f"bold {self.key_color}")
        text.append(" ")
        text.append(" ")
        text.append(hint.label, style=self.muted_color)
        return text

    def render(self) -> Text:
        right_text = Text()
        self._click_spans = []
        for index, hint in enumerate(self.hints):
            if index:
                right_text.append("  ·  ", style=self.muted_color)
            start = cell_width(right_text.plain)
            right_text.append_text(self._render_hint(hint))
            end = cell_width(right_text.plain)
            if hint.action is not None:
                self._click_spans.append((start, end, hint.action))
        return right_text

    def on_click(self, event: events.Click) -> None:
        for start, end, action in self._click_spans:
            if start <= event.x < end:
                event.stop()
                handler_name = f"action_{action.rsplit('.', 1)[-1]}"
                handler = getattr(self.app, handler_name, None)
                if callable(handler):
                    handler()
                return


class ModalHintBar(Static):
    """Muted instruction footer/hint bar for modals."""

    def __init__(self, hints: list[KeyHint], *, key_color: str = "#6b7a93", muted_color: str = "#6b7a93", **kwargs: Any) -> None:
        Static.__init__(self, "", markup=True, **kwargs)
        self.hints = hints
        self.key_color = key_color
        self.muted_color = muted_color
        self._update_markup()

    def set_theme(self, key_color: str, muted_color: str) -> ModalHintBar:
        self.key_color = key_color
        self.muted_color = muted_color
        self._update_markup()
        return self

    def _update_markup(self) -> None:
        markup_str = "  ·  ".join(self._render_hint(h) for h in self.hints)
        self.update(markup_str)

    def _render_hint(self, hint: KeyHint) -> str:
        key_display = hint.key.lower()
        label_display = hint.label.lower()
        return f"[bold {self.key_color}] {escape(key_display)} [/][{self.muted_color}] {escape(label_display)}[/]"


class CustomFooter(AppFooter):
    """A custom, clean footer that displays a concise hint of the most important keys."""

    def __init__(self, **kwargs: Any) -> None:
        AppFooter.__init__(self, PICKER_HINTS, **kwargs)


class CommandPaletteList(OptionList):
    """Grouped, filterable command palette built on :class:`OptionList`.

    Subclassing ``OptionList`` (rather than drawing into a ``Static`` surface)
    buys us modern Textual scroll behaviour for free: native scrollbar,
    mouse-wheel, click-to-scroll, PageUp/PageDown, Home/End, and automatic
    keep-highlight-in-view — all the UX the previous Static-based widget
    was missing when the terminal was shorter than the option list.

    Layout
    ------
    For each group (in ``GROUP_ORDER``) we emit a *disabled* Option whose
    prompt is the group header, followed by an Option per command in that
    group. Disabled options cannot be highlighted/selected by ``OptionList``
    but are still part of the scrollable surface, so headers travel with their
    commands when the user scrolls.

    Message compatibility
    ---------------------
    ``CommandHighlighted`` and ``CommandSelected`` messages are posted so
    ``CommandModal`` keeps working unchanged — we just bridge OptionList's
    built-in ``OptionSelected`` / ``HighlightChanged`` events onto them.
    """

    app: SkyPickerApp
    GROUP_ORDER = ["View", "Playback", "Interface", "Library", "System"]

    class CommandHighlighted(Message):
        def __init__(self, command: CommandSpec) -> None:
            super().__init__()
            self.command = command

    class CommandSelected(Message):
        def __init__(self, command: CommandSpec) -> None:
            super().__init__()
            self.command = command

    def __init__(self, commands: list[CommandSpec], **kwargs: Any) -> None:
        self.commands = commands
        self.filter_text = ""
        self.selectable_commands: list[CommandSpec] = self._filtered_commands()
        # Build initial option rows (headers + commands) and delegate to the
        # OptionList constructor.
        OptionList.__init__(self, *self._build_options(), **kwargs)
        # Modern, scrollbar-on-demand scroll behaviour — the whole point of
        # subclassing OptionList is so the user can wheel/drag to reveal
        # commands hidden past the visible viewport. ``allow_vertical_scroll``
        # is a read-only derived property (``is_scrollable and
        # show_vertical_scrollbar``); OptionList is always scrollable when it
        # has options, so flipping ``show_vertical_scrollbar`` is enough.
        self.show_vertical_scrollbar = True

    # ── Construction helpers ────────────────────────────────────────────────

    def _build_options(self) -> list[Option]:
        """Materialise the visible options: group header (disabled) +
        one Option per matched command, in GROUP_ORDER order.
        """
        grouped: dict[str, list[CommandSpec]] = {}
        for cmd in self.selectable_commands:
            grouped.setdefault(cmd.group, []).append(cmd)

        options: list[Option] = []
        for group_name in self.GROUP_ORDER:
            if group_name not in grouped:
                continue
            header_prompt = Text(group_name.upper(), style="bold dim")
            options.append(Option(header_prompt, id=f"__header__:{group_name}", disabled=True))
            options.extend(
                Option(self._format_command_prompt(cmd), id=f"cmd:{cmd.id}")
                for cmd in grouped[group_name]
            )
        return options

    @staticmethod
    def _format_command_prompt(cmd: CommandSpec) -> Text:
        """Render a single command row as a styled Text prompt.

        Modern ''command palette'' layout — pattern shared by VS Code /
        Linear / Raycast:

            <key> <label> · <description>

        - ``key``   is padded to a fixed 8-cell column so single-char keys
          (``v``, ``p``) align with multi-char ones (``Ctrl+R``, ``F3``);
          rendered bold so it reads as a keyboard shortcut chip.
        - ``label`` sits flush against the key (no padding) so the entry
          reads naturally; ``·`` separators keep ``description`` cleanly
          detached — no column alignment needed for the description, which
          means descriptions of any length stay on one row.
        - ``description`` is dim so it visually de-emphasises the help text
          without losing readability.
        """
        key_str = pad_text(cmd.key, 8)
        line = Text()
        line.append("  ")
        line.append(key_str, style="bold")
        line.append(cmd.label, style="bold")
        line.append(" · ", style="dim")
        line.append(cmd.description, style="dim")
        return line

    # ── Filter ──────────────────────────────────────────────────────────────

    def set_filter(self, value: str) -> None:
        """Rebuild the option list with the given filter applied.

        Resetting options via :meth:`OptionList.clear_options` + ``add_options``
        is the canonical Textual API here — it preserves scroll/selection
        bookkeeping that a hand-rolled refresh would not.
        """
        self.filter_text = value.strip()
        self.selectable_commands = self._filtered_commands()
        self.clear_options()
        new_opts = self._build_options()
        if new_opts:
            self.add_options(new_opts)
            # Highlight the first real (non-disabled) command.
            first_idx = self._first_command_index()
            if first_idx is not None:
                self.highlighted = first_idx
        if self.selectable_commands:
            self.post_message(self.CommandHighlighted(self.selectable_commands[0]))

    def _first_command_index(self) -> int | None:
        """Index of the first selectable (non-header) option, or None."""
        for i, opt in enumerate(self._options):  # type: ignore[attr-defined]
            opt_id = getattr(opt, "id", None) or ""
            if opt_id.startswith("cmd:"):
                return i
        return None

    def _matches_filter(self, command: CommandSpec) -> bool:
        query = remove_accents(self.filter_text).casefold()
        if not query:
            return True
        haystack = " ".join(
            (command.id, command.key, command.label, command.description, command.group)
        )
        return query in remove_accents(haystack).casefold()

    def _filtered_commands(self) -> list[CommandSpec]:
        matched = [cmd for cmd in self.commands if self._matches_filter(cmd)]
        return sorted(
            matched,
            key=lambda cmd: (
                self.GROUP_ORDER.index(cmd.group) if cmd.group in self.GROUP_ORDER else len(self.GROUP_ORDER),
                self.commands.index(cmd),
            ),
        )

    # ── Bridging OptionList events onto Command{Highlighted,Selected} ───────

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        """Map OptionList click/Enter to ``CommandSelected``."""
        cmd = self._command_for_option_index(getattr(event, "option_index", None))
        if cmd is not None:
            event.stop()
            self.post_message(self.CommandSelected(cmd))

    def on_option_list_option_highlighted(  # type: ignore[override]
        self, event: OptionList.OptionHighlighted
    ) -> None:
        cmd = self._command_for_option_index(getattr(event, "option_index", None))
        if cmd is not None:
            event.stop()
            self.post_message(self.CommandHighlighted(cmd))

    def _command_for_option_index(self, idx: int | None) -> CommandSpec | None:
        """Translate an OptionList row index (which includes header rows)
        into the corresponding ``CommandSpec``. Returns None for headers
        and out-of-range indices.
        """
        if idx is None:
            return None
        try:
            opt = self._options[idx]  # type: ignore[attr-defined]
        except (IndexError, AttributeError):
            return None
        opt_id = getattr(opt, "id", None) or ""
        if not opt_id.startswith("cmd:"):
            return None
        cmd_id = opt_id[len("cmd:") :]
        for c in self.selectable_commands:
            if c.id == cmd_id:
                return c
        return None

    # ── External API used by tests / callers ───────────────────────────────

    @property
    def highlighted_index(self) -> int:
        """Index into ``selectable_commands`` of the currently highlighted
        command (skipping header rows), or 0 if nothing is highlighted.
        """
        raw = self.highlighted
        if raw is None:
            return 0
        cmd = self._command_for_option_index(raw)
        if cmd is None:
            return 0
        try:
            return self.selectable_commands.index(cmd)
        except ValueError:
            return 0

    @highlighted_index.setter
    def highlighted_index(self, value: int) -> None:
        if not self.selectable_commands:
            return
        idx = max(0, min(value, len(self.selectable_commands) - 1))
        cmd = self.selectable_commands[idx]
        # Find the OptionList row index that carries this command.
        for i, opt in enumerate(self._options):  # type: ignore[attr-defined]
            if (getattr(opt, "id", None) or "") == f"cmd:{cmd.id}":
                self.highlighted = i
                return

    def move_highlight(self, delta: int) -> None:
        """Move the highlight by ``delta`` skipping header rows."""
        n = len(self.selectable_commands)
        if n == 0:
            return
        self.highlighted_index = (self.highlighted_index + delta) % n

    def select_highlighted(self) -> None:
        if self.selectable_commands:
            self.post_message(self.CommandSelected(self.selectable_commands[self.highlighted_index]))

    # ── Render is handled by OptionList itself; we only mark up prompts ─────

    # No custom render(): OptionList handles scrollbar, viewport, etc.
