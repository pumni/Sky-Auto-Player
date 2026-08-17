"""Validated Windows target-window and keyboard-state seam.

Rust owns all playback input injection. This module contains only window
discovery/focus and read-only keyboard-state queries needed by the UI/doctor.
"""

from __future__ import annotations

import ctypes
import sys
import time
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
PROCESS_IMAGE_NAME_BUFFER_CHARS = 4096
FOCUS_VERIFY_TIMEOUT_MS = 100
FOCUS_VERIFY_POLL_MS = 5

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
user32.ShowWindow.argtypes = (wintypes.HWND, ctypes.c_int)
user32.ShowWindow.restype = wintypes.BOOL
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
    """Attempt the minimal explicit user-requested focus operation.

    Windows may refuse foreground activation due to its foreground-lock
    policy. We report that refusal and let the UI ask the user to click Sky;
    we do not attach input queues, raise z-order, or force an active window.
    """
    if not is_sky_window_valid() or _target_hwnd is None:
        return False
    user32.ShowWindow(_target_hwnd, SW_RESTORE)
    result = bool(user32.SetForegroundWindow(_target_hwnd))
    if PLAYBACK_DEBUG:
        debug_log(f"[focus] SetForegroundWindow target={_target_hwnd} result={result}")
    return result


def is_sky_active() -> bool:
    return is_sky_window_valid() and bool(_target_hwnd) and user32.GetForegroundWindow() == _target_hwnd


def is_foreground_cached_hwnd() -> bool:
    return bool(_target_hwnd) and user32.GetForegroundWindow() == _target_hwnd


def is_hwnd_foreground(hwnd: int) -> bool:
    """Check if the given HWND is currently the foreground window."""
    if type(hwnd) is not int or hwnd <= 0:
        raise ValueError("hwnd must be a positive integer")
    return int(user32.GetForegroundWindow() or 0) == hwnd


is_exact_hwnd_foreground = is_hwnd_foreground


def wait_for_foreground_hwnd(
    hwnd: int,
    *,
    timeout_ms: int = FOCUS_VERIFY_TIMEOUT_MS,
    poll_ms: int = FOCUS_VERIFY_POLL_MS,
) -> bool:
    """Wait for an exact target HWND to become the active foreground window.

    Performs bounded read-only polling without repeating SetForegroundWindow,
    ShowWindow, or target cache discovery.
    """
    if type(hwnd) is not int or hwnd <= 0:
        raise ValueError("hwnd must be a positive integer")
    if type(timeout_ms) is not int or timeout_ms < 0:
        raise ValueError("timeout_ms must be a non-negative integer")
    if type(poll_ms) is not int or poll_ms <= 0:
        raise ValueError("poll_ms must be a positive integer")

    if is_hwnd_foreground(hwnd):
        if PLAYBACK_DEBUG:
            debug_log(f"[focus] target={hwnd} already foreground")
        return True

    deadline_ns = time.monotonic_ns() + timeout_ms * 1_000_000
    poll_s = poll_ms / 1000.0

    while True:
        if _target_hwnd != hwnd:
            if PLAYBACK_DEBUG:
                debug_log(
                    f"[focus] target changed during verification: "
                    f"expected={hwnd}, current={_target_hwnd}"
                )
            return False

        if not bool(user32.IsWindow(hwnd)):
            if PLAYBACK_DEBUG:
                debug_log(f"[focus] target window destroyed during verification: hwnd={hwnd}")
            return False

        foreground = int(user32.GetForegroundWindow() or 0)
        if foreground == hwnd:
            if PLAYBACK_DEBUG:
                debug_log(f"[focus] verified target={hwnd} foreground after bounded wait")
            return True

        if time.monotonic_ns() >= deadline_ns:
            break

        time.sleep(poll_s)

    if (
        _target_hwnd == hwnd
        and bool(user32.IsWindow(hwnd))
        and int(user32.GetForegroundWindow() or 0) == hwnd
    ):
        if PLAYBACK_DEBUG:
            debug_log(f"[focus] verified target={hwnd} foreground at deadline")
        return True

    if PLAYBACK_DEBUG:
        current_fg = int(user32.GetForegroundWindow() or 0)
        debug_log(f"[focus] verification timeout target={hwnd} foreground={current_fg}")
    return False


wait_for_exact_foreground = wait_for_foreground_hwnd


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
