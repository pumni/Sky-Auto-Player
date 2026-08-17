from __future__ import annotations

import ctypes
from ctypes import wintypes

from sky_music.platform.win32 import window_target


def test_show_window_binding_declares_explicit_win32_prototype() -> None:
    assert window_target.user32.ShowWindow.argtypes == (wintypes.HWND, ctypes.c_int)
    assert window_target.user32.ShowWindow.restype is wintypes.BOOL
