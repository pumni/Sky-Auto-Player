"""PostMessage viability probe — does Sky register key input delivered via window messages?

WHY: SendInput goes through the GLOBAL system input queue + Raw Input Thread + every installed
low-level keyboard hook (RGB/macro/overlay/Filter Keys). That shared, serialized chokepoint is what
throttles injection on otherwise-fast machines. PostMessage(WM_KEYDOWN/WM_KEYUP) delivers straight to
the target window's message queue, bypassing that chokepoint entirely — BUT it only works if the game
reads window messages (not Raw Input / GetAsyncKeyState polling).

This probe sends an ascending run of note keys to the Sky window via PostMessage. Run it with Sky open
and a harp/instrument focused, then LISTEN:
  - You hear the notes  -> Sky reads window messages -> PostMessage is a viable bottleneck-free channel.
  - Silence             -> Sky uses Raw Input / key-state polling -> PostMessage is NOT viable.

Run:  uv run python tests/probe_postmessage.py
"""
from __future__ import annotations

import contextlib
import sys
import time
from ctypes import wintypes

WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101

# (char, scancode, virtual-key) — top instrument row, ascending. Resolved from the project's layout.
NOTES = [("y", 21, 89), ("u", 22, 85), ("i", 23, 73), ("o", 24, 79), ("p", 25, 80)]


def main() -> int:
    with contextlib.suppress(Exception):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    from sky_music.platform.win32 import inputs

    user32 = inputs.user32
    user32.PostMessageW.argtypes = (wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)
    user32.PostMessageW.restype = wintypes.BOOL

    hwnd = inputs.get_sky_window()
    if not hwnd:
        print("Sky window not found. Open Sky and try again (or check --sky-process-names).")
        return 2
    print(f"Sky window = {hwnd}. Sending 5 notes via PostMessage in 3s — focus a harp and LISTEN...")
    time.sleep(3.0)

    for char, scancode, vk in NOTES:
        down_lparam = (scancode << 16) | 0x0001
        up_lparam = (scancode << 16) | 0xC0000001  # bits 30+31 set = key-up transition
        ok_down = user32.PostMessageW(hwnd, WM_KEYDOWN, vk, down_lparam)
        time.sleep(0.06)  # ~60ms hold, well above one frame
        ok_up = user32.PostMessageW(hwnd, WM_KEYUP, vk, up_lparam)
        print(f"  sent {char!r} (vk={vk} sc={scancode})  post_down={bool(ok_down)} post_up={bool(ok_up)}")
        time.sleep(0.35)  # gap so each note is distinct

    print(
        "\nDid you HEAR the 5 notes in Sky?\n"
        "  YES -> PostMessage works; we can build a bottleneck-free channel that bypasses hooks.\n"
        "  NO  -> Sky reads Raw Input / polls key state; PostMessage is out, SendInput is the only channel."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
