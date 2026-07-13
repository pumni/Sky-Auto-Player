"""Tests for ``CommandPaletteList`` built on Textual's :class:`OptionList`.

Now that the widget subclasses OptionList, we get scrollbar, mouse-wheel,
PageUp/Down, Home/End, and "keep-highlight-in-view" for free from Textual.
These tests verify the *behaviour we layered on top of* OptionList:

- Group headers render as disabled options interspersed with the commands.
- Filtering reduces the visible options.
- Highlight navigation skips header rows.
- ``CommandSelected`` is posted on Enter/click with the right `CommandSpec`.
- All commands (including ``update``, which the group lives at the bottom)
  remain reachable without scrolling manually — OptionList keeps the
  highlighted row in view automatically.

We drive the widget directly (no Textual app) to keep tests fast and
deterministic. ``app`` is a read-only property on Textual widgets so we
stuff a stub into the instance dict to bypass the descriptor.
"""

from __future__ import annotations

from typing import Any

from sky_music.ui.textual_app.keymap import COMMANDS
from sky_music.ui.textual_app.widgets import CommandPaletteList


def _make_widget() -> CommandPaletteList:
    widget = CommandPaletteList(COMMANDS, id="modal-options")
    widget.on_mount()  # type: ignore[attr-defined]
    return widget


def _option_ids(widget: CommandPaletteList) -> list[str]:
    """All option ids currently in the OptionList, in render order."""
    return [getattr(opt, "id", None) or "" for opt in widget._options]  # type: ignore[attr-defined]


def _command_option_id(cmd_id: str) -> str:
    return f"cmd:{cmd_id}"


# ── All commands present by default ──────────────────────────────────────────


def test_all_commands_present_after_mount() -> None:
    widget = _make_widget()
    ids = _option_ids(widget)
    for cmd in COMMANDS:
        assert _command_option_id(cmd.id) in ids, f"Missing option for {cmd.id}"
    # Update command is specifically there
    assert _command_option_id("update") in ids


def test_group_headers_are_disabled_options() -> None:
    """Group headers render as disabled Options so they can never be
    highlighted/selectable but still travel with the scrollable surface.
    """
    widget = _make_widget()
    for opt in widget._options:  # type: ignore[attr-defined]
        opt_id = getattr(opt, "id", None) or ""
        if opt_id.startswith("__header__:"):
            assert getattr(opt, "disabled", False) is True
        else:
            assert getattr(opt, "disabled", False) is False


def test_group_headers_interleave_with_commands_in_group_order() -> None:
    widget = _make_widget()
    ids = _option_ids(widget)
    # First non-header option must be a command from the first group
    # that has commands (View, per GROUP_ORDER + COMMANDS contents).
    first_cmd_idx = next(i for i, x in enumerate(ids) if x.startswith("cmd:"))
    header_id = ids[first_cmd_idx - 1]
    assert header_id.startswith("__header__:")
    first_cmd = COMMANDS[0]
    header_group = header_id.split(":", 1)[1]
    assert first_cmd.group == header_group


# ── Update command is reachable ──────────────────────────────────────────────


def test_update_command_is_last_selectable_option() -> None:
    """End-to-end proof the user can scroll/select the bottom-most command.
    The ``update`` entry lives at the tail of the list (it's a System command
    added last). With OptionList, ``action_select_last`` jumps to the last
    *selectable* option — verifying it lands on `update` proves both that
    update is present AND that disabled header rows at the tail don't block.
    """
    widget = _make_widget()
    widget.action_last()  # OptionList built-in
    assert widget.highlighted is not None
    cmd = widget._command_for_option_index(widget.highlighted)
    assert cmd is not None
    assert cmd.id == "update"


def test_enter_on_update_selects_it() -> None:
    """Highlighting the last command and pressing Enter must post a
    ``CommandSelected`` message carrying the ``update`` CommandSpec.
    """
    widget = _make_widget()
    captured: list[str] = []

    def _capture(msg: Any) -> None:
        if isinstance(msg, CommandPaletteList.CommandSelected):
            captured.append(msg.command.id)

    widget.post_message = _capture  # type: ignore[method-assign]
    widget.action_last()
    # Selecting via OptionList's highlighteds dispatches through OptionList's
    # built-in action_select — emulate Enter by calling on_option_list_option_selected
    widget.select_highlighted()

    assert captured == ["update"]


# ── Filter behaviour ─────────────────────────────────────────────────────────


def test_set_filter_with_match_keeps_only_matching_commands() -> None:
    widget = _make_widget()
    widget.set_filter("update")
    ids = _option_ids(widget)
    # Only the System header + the update command should remain.
    assert _command_option_id("update") in ids
    for cmd in COMMANDS:
        if cmd.id != "update":
            assert _command_option_id(cmd.id) not in ids


def test_set_filter_resets_highlight_to_first_command() -> None:
    widget = _make_widget()
    widget.action_last()
    assert widget.highlighted_index > 0

    widget.set_filter("update")
    # After filter, only one command matches → it's the first.
    assert widget.highlighted_index == 0


def test_set_filter_with_no_match_yields_empty_option_list() -> None:
    widget = _make_widget()
    widget.set_filter("zzzzz-no-such-command")
    assert len(widget._options) == 0  # type: ignore[attr-defined]


# ── highlighted_index property translation ──────────────────────────────────


def test_highlighted_index_translates_optionlist_highlight_into_cmd_index() -> None:
    """The exposed ``highlighted_index`` skips header rows so callers that
    index into ``selectable_commands`` see the right value regardless of how
    many header rows precede the highlighted option.
    """
    widget = _make_widget()
    widget.action_first()
    assert widget.highlighted_index == 0

    widget.action_last()
    assert widget.highlighted_index == len(COMMANDS) - 1


def test_move_highlight_skips_header_rows() -> None:
    """Moving by ±1 must skip past any header rows. Given every group has
    exactly one header before its commands, a single ``move_highlight(1)``
    from the last command of a group should land on the first command of the
    next group, not get stuck on the intervening disabled header.
    """
    widget = _make_widget()
    widget.action_first()
    widget.move_highlight(1)
    # Highlighted option (raw OptionList index) must be a `cmd:*` row, not a header
    assert widget.highlighted is not None
    opt_id = getattr(widget._options[widget.highlighted], "id", "")  # type: ignore[attr-defined]
    assert opt_id.startswith("cmd:")


def test_move_highlight_wraps_from_last_to_first() -> None:
    widget = _make_widget()
    widget.action_last()
    assert widget.highlighted_index == len(COMMANDS) - 1
    widget.move_highlight(1)  # wraps
    assert widget.highlighted_index == 0
    widget.move_highlight(-1)  # wraps back
    assert widget.highlighted_index == len(COMMANDS) - 1


# ── Update is reachable from any arbitrary earlier position ─────────────────


def test_arrow_down_repeatedly_reaches_update() -> None:
    """Simulate pressing Down enough times to walk the entire list.

    OptionList's ``action_cursor_down`` would do this natively, but we use
    our ``move_highlight`` shim that wraps headers — proving the helper
    reaches the bottom without skipping or stalling.
    """
    widget = _make_widget()
    widget.action_first()
    last_idx = len(COMMANDS) - 1
    for _ in range(last_idx):
        widget.move_highlight(1)
    assert widget.highlighted_index == last_idx
    cmd = widget._command_for_option_index(widget.highlighted)
    assert cmd is not None and cmd.id == "update"
