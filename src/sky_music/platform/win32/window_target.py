"""Validated Windows target-window and keyboard-state seam.

Rust owns all playback input injection. This module contains only window
discovery/focus and read-only keyboard-state queries needed by the UI/doctor.
"""

from __future__ import annotations

import ctypes
import sys
from collections.abc import Iterable
from ctypes import wintypes
from pathlib import Path

if sys.platform == "win32":
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
else:
    class _MockFunction:
        def __init__(self, name: str) -> None:
            self.name = name
            self.argtypes = None
            self.restype = None

        def __call__(self, *_args: object, **_kwargs: object) -> int:
            return 0

    class _MockDll:
        def __getattr__(self, name: str) -> _MockFunction:
            return _MockFunction(name)

    user32 = _MockDll()
    kernel32 = _MockDll()

SW_RESTORE = 9
SWP_NOMOVE = 0x0002
SWP_NOSIZE = 0x0001
SWP_SHOWWINDOW = 0x0040
HWND_TOP = 0
PROCESS_IMAGE_NAME_BUFFER_CHARS = 4096

user32.MapVirtualKeyW.argtypes = (wintypes.UINT, wintypes.UINT)
user32.MapVirtualKeyW.restype = wintypes.UINT
user32.GetWindowThreadProcessId.argtypes = (wintypes.HWND, ctypes.POINTER(wintypes.DWORD))
user32.GetWindowThreadProcessId.restype = wintypes.DWORD
user32.GetForegroundWindow.argtypes = ()
user32.GetForegroundWindow.restype = wintypes.HWND
user32.GetAsyncKeyState.argtypes = (ctypes.c_int,)
user32.GetAsyncKeyState.restype = ctypes.c_short
user32.IsWindow.argtypes = (wintypes.HWND,)
user32.IsWindow.restype = wintypes.BOOL
user32.IsWindowVisible.argtypes = (wintypes.HWND,)
user32.IsWindowVisible.restype = wintypes.BOOL
user32.GetWindowTextLengthW.argtypes = (wintypes.HWND,)
user32.GetWindowTextLengthW.restype = ctypes.c_int
user32.GetWindowTextW.argtypes = (wintypes.HWND, wintypes.LPWSTR, ctypes.c_int)
user32.GetWindowTextW.restype = ctypes.c_int
user32.EnumWindows.argtypes = (
    ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM),
    wintypes.LPARAM,
)
user32.EnumWindows.restype = wintypes.BOOL
user32.SetForegroundWindow.argtypes = (wintypes.HWND,)
user32.SetForegroundWindow.restype = wintypes.BOOL
user32.BringWindowToTop.argtypes = (wintypes.HWND,)
user32.BringWindowToTop.restype = wintypes.BOOL
user32.SetActiveWindow.argtypes = (wintypes.HWND,)
user32.SetActiveWindow.restype = wintypes.HWND
user32.AttachThreadInput.argtypes = (wintypes.DWORD, wintypes.DWORD, wintypes.BOOL)
user32.AttachThreadInput.restype = wintypes.BOOL
user32.SetWindowPos.argtypes = (
    wintypes.HWND,
    wintypes.HWND,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.UINT,
)
user32.SetWindowPos.restype = wintypes.BOOL
kernel32.GetCurrentThreadId.argtypes = ()
kernel32.GetCurrentThreadId.restype = wintypes.DWORD
kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
kernel32.CloseHandle.restype = wintypes.BOOL
kernel32.QueryFullProcessImageNameW.argtypes = (
    wintypes.HANDLE,
    wintypes.DWORD,
    wintypes.LPWSTR,
    ctypes.POINTER(wintypes.DWORD),
)
kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL

from sky_music.config import DEFAULT_SKY_PROCESS_NAMES  # noqa: E402

EXPECTED_PROCESS_NAMES: set[str] = set(DEFAULT_SKY_PROCESS_NAMES)
ALLOW_TITLE_FALLBACK = False
PLAYBACK_DEBUG = False
REJECTED_WINDOW_WARNINGS: set[int] = set()
_debug_log_callback = None
_target_hwnd: int | None = None


def debug_log(message: str) -> None:
    if _debug_log_callback is not None:
        _debug_log_callback(message)


def set_expected_process_names(names: Iterable[str]) -> None:
    normalized = {name.strip() for name in names if name.strip()}
    if not normalized:
        raise ValueError("Expected process names cannot be empty")
    global EXPECTED_PROCESS_NAMES
    EXPECTED_PROCESS_NAMES = normalized


def set_title_fallback(enabled: bool) -> None:
    global ALLOW_TITLE_FALLBACK
    ALLOW_TITLE_FALLBACK = bool(enabled)


def reset_window_cache() -> None:
    global _target_hwnd
    _target_hwnd = None


def cached_target_hwnd() -> int:
    return _target_hwnd or 0


def map_virtual_key(vk: int) -> int:
    if type(vk) is not int or not 0 <= vk <= 0xFF:
        raise ValueError("vk must be an integer in the range 0..255")
    return int(user32.MapVirtualKeyW(vk, 0))


def get_window_process_id(hwnd: int) -> int | None:
    if type(hwnd) is not int or hwnd <= 0:
        raise ValueError("hwnd must be a positive integer")
    process_id = wintypes.DWORD()
    if not user32.GetWindowThreadProcessId(hwnd, ctypes.byref(process_id)):
        return None
    return int(process_id.value)


def get_process_name_by_pid(pid: int) -> str | None:
    if type(pid) is not int or pid <= 0:
        raise ValueError("pid must be a positive integer")
    handle = kernel32.OpenProcess(0x1000, False, pid)
    if not handle:
        return None
    try:
        size = wintypes.DWORD(PROCESS_IMAGE_NAME_BUFFER_CHARS)
        buffer = ctypes.create_unicode_buffer(PROCESS_IMAGE_NAME_BUFFER_CHARS)
        if kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
            return Path(buffer.value).name
        return None
    finally:
        kernel32.CloseHandle(handle)


def get_sky_window() -> int | None:
    found: int | None = None

    def visit(hwnd: int, _lparam: int) -> bool:
        nonlocal found
        if not user32.IsWindowVisible(hwnd):
            return True
        title_length = user32.GetWindowTextLengthW(hwnd)
        if title_length <= 0:
            return True
        title_buffer = ctypes.create_unicode_buffer(title_length + 1)
        user32.GetWindowTextW(hwnd, title_buffer, title_length + 1)
        title = title_buffer.value
        if title != "Sky" and not title.startswith("Sky"):
            return True
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        process_name = get_process_name_by_pid(int(pid.value))
        if process_name in EXPECTED_PROCESS_NAMES or not EXPECTED_PROCESS_NAMES or ALLOW_TITLE_FALLBACK:
            found = int(hwnd)
            return False
        if int(hwnd) not in REJECTED_WINDOW_WARNINGS:
            REJECTED_WINDOW_WARNINGS.add(int(hwnd))
            if PLAYBACK_DEBUG:
                debug_log(
                    f"[window] rejected candidate: title={title!r}, "
                    f"pid={pid.value}, process={process_name!r}"
                )
        return True

    callback_type = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows(callback_type(visit), 0)
    return found


def is_sky_window_valid() -> bool:
    global _target_hwnd
    if _target_hwnd is None or not user32.IsWindow(_target_hwnd):
        _target_hwnd = get_sky_window()
        return _target_hwnd is not None
    pid = get_window_process_id(_target_hwnd)
    process_name = get_process_name_by_pid(pid) if pid is not None else None
    if process_name in EXPECTED_PROCESS_NAMES or ALLOW_TITLE_FALLBACK:
        return True
    _target_hwnd = get_sky_window()
    return _target_hwnd is not None


def focus_window() -> bool:
    if not is_sky_window_valid() or _target_hwnd is None:
        return False
    foreground_thread = user32.GetWindowThreadProcessId(user32.GetForegroundWindow(), None)
    current_thread = kernel32.GetCurrentThreadId()
    attached = bool(foreground_thread and foreground_thread != current_thread)
    if attached:
        user32.AttachThreadInput(current_thread, foreground_thread, True)
    try:
        user32.ShowWindow(_target_hwnd, SW_RESTORE)
        user32.SetWindowPos(
            _target_hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )
        user32.BringWindowToTop(_target_hwnd)
        success = bool(user32.SetForegroundWindow(_target_hwnd))
        user32.SetActiveWindow(_target_hwnd)
        return success
    finally:
        if attached:
            user32.AttachThreadInput(current_thread, foreground_thread, False)


def is_sky_active() -> bool:
    return is_sky_window_valid() and bool(_target_hwnd) and user32.GetForegroundWindow() == _target_hwnd


def is_foreground_cached_hwnd() -> bool:
    return bool(_target_hwnd) and user32.GetForegroundWindow() == _target_hwnd


def is_virtual_key_down(key_code: int) -> bool:
    if type(key_code) is not int or not 0 <= key_code <= 0xFFFF:
        raise ValueError("key_code must be an integer in the range 0..65535")
    return bool(user32.GetAsyncKeyState(key_code) & 0x8000)


def describe_target() -> str:
    return (
        f"target_hwnd={_target_hwnd}, "
        f"foreground_hwnd={user32.GetForegroundWindow()}, "
        f"target_active={is_sky_active()}"
    )


# Explicitly named alias for older UI focus call sites during the file migration.
focusWindow = focus_window
