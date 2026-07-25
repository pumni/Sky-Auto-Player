"""Win32 console capability queries kept behind the platform boundary."""
from __future__ import annotations

import ctypes
import sys
from ctypes import wintypes

_ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x0004
_STD_OUTPUT_HANDLE = -11


def virtual_terminal_processing_enabled() -> bool | None:
    """Return the console VT capability, or ``None`` when it cannot be queried."""
    if sys.platform != "win32":
        return None
    try:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.GetStdHandle.argtypes = (wintypes.DWORD,)
        kernel32.GetStdHandle.restype = wintypes.HANDLE
        kernel32.GetConsoleMode.argtypes = (wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD))
        kernel32.GetConsoleMode.restype = wintypes.BOOL
        handle = kernel32.GetStdHandle(wintypes.DWORD(_STD_OUTPUT_HANDLE & 0xFFFFFFFF))
        mode = wintypes.DWORD()
        if not kernel32.GetConsoleMode(handle, ctypes.byref(mode)):
            return None
        return bool(mode.value & _ENABLE_VIRTUAL_TERMINAL_PROCESSING)
    except OSError:
        return None
