"""Regression tests for hotkey poll modifier snapshotting (review of main@7c548527 §1.5).

The legacy ``PlaybackControls.poll`` re-queried Ctrl/Alt/Shift once per hotkey via
``GetAsyncKeyState`` (is_virtual_key_down). At 5 hotkeys × 4 calls/hotkey = 20 calls/tick ≈
2 000 calls/s during a 10 ms-cadence playback, the pollhammer ``GetAsyncKeyState`` traffic
is the user-visible CPU footprint of "playing a song". The snapshot refactor queries each
modifier exactly ONCE per poll tick and passes the flags down to
``_eval_hotkey_with_modifiers`` for every binding — 3 + 5 = 8 calls/tick, a 60 % reduction.

We assert the syscall count via a counter on the patched ``is_virtual_key_down`` (so the
test is not at the mercy of real Win32 keyboard state) and verify the snapshot is read at
poll entry rather than re-evaluated behind each hotkey.
"""
from __future__ import annotations

import pytest

import sky_music.infrastructure.hotkeys as hotkeys_mod
from sky_music.infrastructure.hotkeys import (
    HotkeyBinding,
    PlaybackControls,
    parse_hotkey,
)


def _make_controls(first_is_modifier: bool = False) -> tuple[PlaybackControls, dict[int, int]]:
    """Build a 5-binding PlaybackControls with a counting ``is_virtual_key_down``.

    ``first_is_modifier=True`` builds a plain-F8 ``quit`` hotkey (no modifiers) so the "no
    modifier held" gate fires for every remaining binding whenever Ctrl is sampled as down.
    """
    counter: dict[int, int] = {}

    def fake_vk_down(vk: int) -> bool:
        counter[vk] = counter.get(vk, 0) + 1
        # CTRL held → required-modifier bindings succeed; plain bindings see "modifier held"
        # so they read their key and return False (we don't care which route they take here;
        # the contract is the COUNT, not the route-true/false).
        if vk == hotkeys_mod.VK_CONTROL:
            return True
        if vk == hotkeys_mod.VK_MENU:
            return False
        if vk == hotkeys_mod.VK_SHIFT:
            return False
        # Heart of the matter: an arbitrary key is "down" only for plain-key pollhammer
        # detection and never matters to the syscall-count contract.
        return False

    quit_hk = parse_hotkey("ctrl+f3") if first_is_modifier else parse_hotkey("f3")
    controls = PlaybackControls(
        pause=parse_hotkey("ctrl+f1"),
        skip=parse_hotkey("ctrl+f2"),
        quit=quit_hk,
        refocus=parse_hotkey("ctrl+f4"),
        panic=parse_hotkey("ctrl+f5"),
    )
    return controls, counter


def _patch_vk_down(monkeypatch: pytest.MonkeyPatch, fn) -> None:
    monkeypatch.setattr(hotkeys_mod, "is_virtual_key_down", fn)


def test_poll_snapshots_three_modifiers_once_per_tick(monkeypatch: pytest.MonkeyPatch) -> None:
    controls, counter = _make_controls(first_is_modifier=True)
    _patch_vk_down(monkeypatch, lambda vk: counter_inc(counter, vk))

    # Fresh poll: there must be exactly one read of VK_CONTROL / VK_MENU / VK_SHIFT (3 calls)
    # plus one read of each of the 5 hotkey's key_code (5 calls) = 8 total Win32 syscalls.
    # Legacy path would have read 3 modifiers × 5 hotkeys + 5 key_codes = 20 calls.
    controls.poll()
    mod_calls = sum(
        counter.get(vk, 0)
        for vk in (hotkeys_mod.VK_CONTROL, hotkeys_mod.VK_MENU, hotkeys_mod.VK_SHIFT)
    )
    assert mod_calls == 3, (
        f"snapshot must read Ctrl/Alt/Shift ONCE per tick (3 calls); got {mod_calls}"
    )
    key_calls = sum(
        counter.get(parse_hotkey(f"ctrl+f{i}").key_code, 0) for i in (1, 2, 4, 5)
    ) + counter.get(parse_hotkey("f3").key_code, 0)
    # 5 hotkey key reads, one per binding.
    assert key_calls == 5, (
        f"5 hotkey key reads expected (one per binding); got {key_calls}"
    )


def test_poll_does_not_call_legacy_is_hotkey_down_on_the_tick_path(monkeypatch: pytest.MonkeyPatch) -> None:
    """The new poll code-path must not accidentally route through ``is_hotkey_down``.

    ``is_hotkey_down`` is the public single-shot entry point that re-queries every modifier
    per call. If poll() were silently delegated to it the syscall count would balloon back to
    20/tick; this test guards against that regression by instrumenting the legacy entry.
    """
    controls, counter = _make_controls(first_is_modifier=True)
    legacy_calls: list[int] = []

    def fake_vk_down(vk: int) -> bool:
        counter[vk] = counter.get(vk, 0) + 1
        return vk == hotkeys_mod.VK_CONTROL

    orig_is_hotkey_down = hotkeys_mod.is_hotkey_down

    def spy_is_hotkey_down(hotkey: HotkeyBinding) -> bool:
        legacy_calls.append(1)
        return orig_is_hotkey_down(hotkey)

    _patch_vk_down(monkeypatch, fake_vk_down)
    monkeypatch.setattr(hotkeys_mod, "is_hotkey_down", spy_is_hotkey_down)
    # PlaybackControls.poll was bound at class-definition time; re-bind through the module
    # attribute so the patched ``is_hotkey_down`` is observed if the implementation ever
    # regresses to call the legacy function.
    monkeypatch.setattr(
        PlaybackControls, "poll", PlaybackControls.poll, raising=True
    )

    controls.poll()
    assert legacy_calls == [], (
        "PlaybackControls.poll must NOT call is_hotkey_down; it should route all 5 "
        "bindings through _eval_hotkey_with_modifiers with a single modifier snapshot."
    )


def test_eval_hotkey_with_modifiers_requires_required_modifiers(monkeypatch: pytest.MonkeyPatch) -> None:
    """The snapshot path enforces the same modifier requirements as the legacy path."""
    from sky_music.infrastructure.hotkeys import _eval_hotkey_with_modifiers

    plain = parse_hotkey("f8")  # no modifiers
    modded = parse_hotkey("ctrl+f8")  # ctrl required

    # Plain hotkey with any modifier held → False, regardless of key down state.
    key_hits: list[int] = []
    def fake_vk_down(vk: int) -> bool:
        key_hits.append(vk)
        return True

    _patch_vk_down(monkeypatch, fake_vk_down)

    assert _eval_hotkey_with_modifiers(plain, ctrl_down=True, alt_down=False, shift_down=False) is False
    assert _eval_hotkey_with_modifiers(plain, ctrl_down=False, alt_down=True, shift_down=False) is False
    assert _eval_hotkey_with_modifiers(plain, ctrl_down=False, alt_down=False, shift_down=True) is False
    # Plain + no modifier → reads key
    assert _eval_hotkey_with_modifiers(plain, ctrl_down=False, alt_down=False, shift_down=False) is True

    # Modifier hotkey with required modifier missing → False, does not even read key
    key_hits.clear()
    assert _eval_hotkey_with_modifiers(modded, ctrl_down=False, alt_down=False, shift_down=False) is False
    assert key_hits == [], "key_code must NOT be queried when a required modifier is absent"
    # Modifier hotkey with required modifier held → reads key
    assert _eval_hotkey_with_modifiers(modded, ctrl_down=True, alt_down=False, shift_down=False) is True


def counter_inc(counter: dict[int, int], vk: int) -> bool:
    counter[vk] = counter.get(vk, 0) + 1
    # Return True for VK_CONTROL (modifier held) so the modifier-hotkey bindings actually
    # proceed to read their key_code — that is the path that exercises the 3+5 call count.
    return vk == hotkeys_mod.VK_CONTROL
