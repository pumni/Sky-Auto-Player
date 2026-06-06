import ctypes
from ctypes import wintypes
import time
from pathlib import Path
from collections.abc import Callable

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
winmm = ctypes.WinDLL("winmm", use_last_error=True)
try:
    avrt = ctypes.WinDLL("avrt", use_last_error=True)
except OSError:
    avrt = None

SW_RESTORE = 9
SWP_NOMOVE = 0x0002
SWP_NOSIZE = 0x0001
SWP_SHOWWINDOW = 0x0040
HWND_TOP = 0
INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008
CREATE_WAITABLE_TIMER_HIGH_RESOLUTION = 0x00000002
TIMER_ALL_ACCESS = 0x001F0003
INFINITE = 0xFFFFFFFF

class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.c_size_t),
    ]

class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.c_size_t),
    ]

class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [
        ("uMsg", wintypes.DWORD),
        ("wParamL", wintypes.WORD),
        ("wParamH", wintypes.WORD),
    ]

class INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]

class INPUT(ctypes.Structure):
    _anonymous_ = ("union",)
    _fields_ = [("type", wintypes.DWORD), ("union", INPUTUNION)]

user32.SendInput.argtypes = (wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int)
user32.SendInput.restype = wintypes.UINT
user32.MapVirtualKeyW.argtypes = (wintypes.UINT, wintypes.UINT)
user32.MapVirtualKeyW.restype = wintypes.UINT
user32.EnumWindows.argtypes = (ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM), wintypes.LPARAM)
user32.EnumWindows.restype = wintypes.BOOL
user32.GetWindowTextLengthW.argtypes = (wintypes.HWND,)
user32.GetWindowTextLengthW.restype = ctypes.c_int
user32.GetWindowTextW.argtypes = (wintypes.HWND, wintypes.LPWSTR, ctypes.c_int)
user32.GetWindowTextW.restype = ctypes.c_int
user32.IsWindowVisible.argtypes = (wintypes.HWND,)
user32.IsWindowVisible.restype = wintypes.BOOL
user32.ShowWindow.argtypes = (wintypes.HWND, ctypes.c_int)
user32.ShowWindow.restype = wintypes.BOOL
user32.SetForegroundWindow.argtypes = (wintypes.HWND,)
user32.SetForegroundWindow.restype = wintypes.BOOL
user32.BringWindowToTop.argtypes = (wintypes.HWND,)
user32.BringWindowToTop.restype = wintypes.BOOL
user32.SetActiveWindow.argtypes = (wintypes.HWND,)
user32.SetActiveWindow.restype = wintypes.HWND
user32.GetWindowThreadProcessId.argtypes = (wintypes.HWND, ctypes.POINTER(wintypes.DWORD))
user32.GetWindowThreadProcessId.restype = wintypes.DWORD
kernel32.GetCurrentThreadId.argtypes = ()
kernel32.GetCurrentThreadId.restype = wintypes.DWORD
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
user32.GetForegroundWindow.argtypes = ()
user32.GetForegroundWindow.restype = wintypes.HWND
user32.GetAsyncKeyState.argtypes = (ctypes.c_int,)
user32.GetAsyncKeyState.restype = ctypes.c_short
user32.IsWindow.argtypes = (wintypes.HWND,)
user32.IsWindow.restype = wintypes.BOOL
kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
kernel32.CloseHandle.restype = wintypes.BOOL
if hasattr(kernel32, "CreateWaitableTimerExW"):
    kernel32.CreateWaitableTimerExW.argtypes = (
        ctypes.c_void_p,
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
    )
    kernel32.CreateWaitableTimerExW.restype = wintypes.HANDLE
kernel32.SetWaitableTimer.argtypes = (
    wintypes.HANDLE,
    ctypes.POINTER(ctypes.c_longlong),
    wintypes.LONG,
    ctypes.c_void_p,
    ctypes.c_void_p,
    wintypes.BOOL,
)
kernel32.SetWaitableTimer.restype = wintypes.BOOL
kernel32.WaitForSingleObject.argtypes = (wintypes.HANDLE, wintypes.DWORD)
kernel32.WaitForSingleObject.restype = wintypes.DWORD
kernel32.QueryFullProcessImageNameW.argtypes = (wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD))
kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
winmm.timeBeginPeriod.argtypes = (wintypes.UINT,)
winmm.timeBeginPeriod.restype = wintypes.UINT
winmm.timeEndPeriod.argtypes = (wintypes.UINT,)
winmm.timeEndPeriod.restype = wintypes.UINT
if avrt is not None:
    avrt.AvSetMmThreadCharacteristicsW.argtypes = (
        wintypes.LPCWSTR,
        ctypes.POINTER(wintypes.DWORD),
    )
    avrt.AvSetMmThreadCharacteristicsW.restype = wintypes.HANDLE
    avrt.AvRevertMmThreadCharacteristics.argtypes = (wintypes.HANDLE,)
    avrt.AvRevertMmThreadCharacteristics.restype = wintypes.BOOL

TIMER_RESOLUTION_MS = 1
PROCESS_IMAGE_NAME_BUFFER_CHARS = 4096
_timer_resolution_enabled: bool = False

# Global configuration variables to be updated by main.py
from sky_music.config import DEFAULT_SKY_PROCESS_NAMES

EXPECTED_PROCESS_NAMES: set[str] = set(DEFAULT_SKY_PROCESS_NAMES)
ALLOW_TITLE_FALLBACK: bool = False
PLAYBACK_DEBUG: bool = False
REJECTED_WINDOW_WARNINGS: set[int] = set()
sky: int | None = None

# We dynamically hook debug_log to avoid circular dependency
_debug_log_callback: Callable[[str], None] | None = None

def debug_log(message: str) -> None:
    if _debug_log_callback is not None:
        _debug_log_callback(message)

def enable_high_precision_timers() -> None:
    global _timer_resolution_enabled
    if _timer_resolution_enabled:
        return
    result = winmm.timeBeginPeriod(TIMER_RESOLUTION_MS)
    if result != 0:
        raise OSError(f"timeBeginPeriod({TIMER_RESOLUTION_MS}) failed with MMRESULT {result}")
    _timer_resolution_enabled = True

def disable_high_precision_timers() -> None:
    global _timer_resolution_enabled
    if not _timer_resolution_enabled:
        return
    winmm.timeEndPeriod(TIMER_RESOLUTION_MS)
    _timer_resolution_enabled = False


class _HighResolutionTimerScope:
    """Defensive 1ms timer-resolution guard for the dispatch thread.

    timeBeginPeriod/timeEndPeriod are refcounted by the OS, so this raw begin/end pair is safe to
    nest inside the process-wide ``enable_high_precision_timers`` window. It guarantees the dispatch
    loop always runs at 1ms granularity — the safety net for the ``RealSleeper`` fallback when the
    high-resolution waitable timer is unavailable — independent of the module's on/off flag, which
    could otherwise be left coarse by an unbalanced enable/disable elsewhere in the session.
    """

    __slots__ = ("_active",)

    def __enter__(self) -> "_HighResolutionTimerScope":
        self._active = False
        try:
            if winmm.timeBeginPeriod(TIMER_RESOLUTION_MS) == 0:
                self._active = True
        except Exception as exc:
            debug_log(f"[realtime] timeBeginPeriod guard failed: {exc}")
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        if not self._active:
            return
        try:
            winmm.timeEndPeriod(TIMER_RESOLUTION_MS)
        except Exception as exc:
            debug_log(f"[realtime] timeEndPeriod guard failed: {exc}")
        finally:
            self._active = False


def high_resolution_timer_scope() -> _HighResolutionTimerScope:
    """Return a context manager guaranteeing 1ms timer resolution for its body."""
    return _HighResolutionTimerScope()


def reset_window_cache() -> None:
    """Drop the cached Sky HWND so the next lookup re-enumerates the live window.

    Called at the start of every real playback so a window handle that went stale during the
    session (game restarted, window re-created, focus juggled) can never carry into the next run —
    the class of volatile-state fault that otherwise only clears on a full player restart.
    """
    global sky
    sky = None


def describe_input_target() -> str:
    """One-line snapshot of the volatile input-targeting state for play-start diagnostics."""
    try:
        foreground = user32.GetForegroundWindow()
    except Exception:
        foreground = None
    try:
        active = is_sky_active()
    except Exception:
        active = None
    return (
        f"sky_hwnd={sky}, foreground_hwnd={foreground}, sky_active={active}, "
        f"timer_res_enabled={_timer_resolution_enabled}"
    )

def create_high_resolution_waitable_timer() -> int | None:
    create_timer = getattr(kernel32, "CreateWaitableTimerExW", None)
    if create_timer is None:
        return None
    handle = create_timer(
        None,
        None,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
        TIMER_ALL_ACCESS,
    )
    if not handle:
        return None
    return int(handle)

def set_waitable_timer_relative_us(handle: int, delay_us: int) -> bool:
    # Negative due time requests a relative interval in 100ns units.
    due_time = ctypes.c_longlong(-max(1, int(delay_us) * 10))
    return bool(
        kernel32.SetWaitableTimer(
            wintypes.HANDLE(handle),
            ctypes.byref(due_time),
            0,
            None,
            None,
            False,
        )
    )

def wait_for_timer(handle: int) -> None:
    kernel32.WaitForSingleObject(wintypes.HANDLE(handle), INFINITE)

def close_handle(handle: int) -> None:
    kernel32.CloseHandle(wintypes.HANDLE(handle))

def av_set_mm_thread_characteristics(task_name: str) -> int | None:
    if avrt is None:
        return None
    task_index = wintypes.DWORD(0)
    handle = avrt.AvSetMmThreadCharacteristicsW(task_name, ctypes.byref(task_index))
    if not handle:
        return None
    return int(handle)

def av_revert_mm_thread_characteristics(handle: int) -> None:
    if avrt is None:
        return
    avrt.AvRevertMmThreadCharacteristics(wintypes.HANDLE(handle))

def _retry_wait_seconds(seconds: float) -> None:
    if seconds <= 0:
        return
    deadline = time.perf_counter() + seconds
    while True:
        remaining = deadline - time.perf_counter()
        if remaining <= 0:
            return
        if remaining > 0.020:
            time.sleep(remaining - 0.005)
        elif remaining > 0.003:
            time.sleep(0.001)
        elif remaining > 0.0008:
            time.sleep(0)
        else:
            pass

def send_input_batch(inputs: list[INPUT]) -> None:
    if not inputs:
        return
    pending_inputs = list(inputs)
    retries_without_progress = 0
    total_inputs = len(inputs)
    while pending_inputs:
        input_array = (INPUT * len(pending_inputs))(*pending_inputs)
        sent = user32.SendInput(len(pending_inputs), input_array, ctypes.sizeof(INPUT))
        if sent == len(pending_inputs):
            return
        if sent > 0:
            pending_inputs = pending_inputs[sent:]
            retries_without_progress = 0
            continue
        retries_without_progress += 1
        if retries_without_progress >= 3:
            err_code = ctypes.get_last_error()
            raise OSError(
                f"SendInput failure: sent {total_inputs - len(pending_inputs)}/{total_inputs} actions. "
                f"Windows Error Code: {err_code} ({ctypes.FormatError(err_code).strip()}). "
                f"Possible reasons: Sky is elevated (Admin) while this script is not (UIPI mismatch), "
                f"or target window handles became invalid."
            )
        _retry_wait_seconds(0.002)

# Cache one immutable INPUT per (scan_code, flags). Sky uses at most ~15 keys x {down, up}, so this
# is a tiny fixed-size table. Building the KEYBDINPUT/INPUT structs is the bulk of the per-event
# Python cost (~50-60% for chords); SendInput copies these by value into the batch array, so the
# cached entries are never mutated and the partial-send retry in send_input_batch still operates on
# the copied array.
_INPUT_CACHE: dict[tuple[int, int], INPUT] = {}

def _cached_key_input(scan_code: int, flags: int) -> INPUT:
    cache_key = (scan_code, flags)
    cached = _INPUT_CACHE.get(cache_key)
    if cached is None:
        cached = INPUT(type=INPUT_KEYBOARD)
        cached.ki = KEYBDINPUT(0, scan_code, flags, 0, 0)
        _INPUT_CACHE[cache_key] = cached
    return cached

def send_scan_code_batch(scan_codes: tuple[int, ...] | list[int], key_up: bool = False) -> None:
    if not scan_codes:
        return
    # Keep this boundary defensive: release/retry paths and direct callers do not all originate
    # from the scheduler's already-deduplicated KeyAction batches.
    scan_codes = tuple(dict.fromkeys(scan_codes))
    flags = KEYEVENTF_SCANCODE | (KEYEVENTF_KEYUP if key_up else 0)
    key_inputs = [_cached_key_input(scan_code, flags) for scan_code in scan_codes]
    send_input_batch(key_inputs)

def get_process_name_by_pid(pid: int) -> str | None:
    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    h_process = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h_process:
        return None
    try:
        size = wintypes.DWORD(PROCESS_IMAGE_NAME_BUFFER_CHARS)
        buffer = ctypes.create_unicode_buffer(PROCESS_IMAGE_NAME_BUFFER_CHARS)
        if kernel32.QueryFullProcessImageNameW(h_process, 0, buffer, ctypes.byref(size)):
            path = buffer.value
            return Path(path).name
    except Exception:
        pass
    finally:
        kernel32.CloseHandle(h_process)
    return None

def get_sky_window() -> int | None:
    found_window = wintypes.HWND()
    rejected_candidates = []

    def enum_window_callback(hwnd: int, _lparam: int) -> bool:
        if not user32.IsWindowVisible(hwnd):
            return True
        title_length = user32.GetWindowTextLengthW(hwnd)
        if title_length == 0:
            return True
        title_buffer = ctypes.create_unicode_buffer(title_length + 1)
        user32.GetWindowTextW(hwnd, title_buffer, title_length + 1)
        title = title_buffer.value

        if title == "Sky" or title.startswith("Sky"):
            pid = wintypes.DWORD()
            user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            proc_name = get_process_name_by_pid(pid.value)
            
            if proc_name in EXPECTED_PROCESS_NAMES:
                found_window.value = hwnd
                return False
            if not EXPECTED_PROCESS_NAMES or ALLOW_TITLE_FALLBACK:
                found_window.value = hwnd
                return False
            
            rejected_candidates.append((hwnd, title, pid.value, proc_name))
            if PLAYBACK_DEBUG:
                debug_log(
                    f"[window] rejected candidate: title={title!r}, "
                    f"pid={pid.value}, process={proc_name!r}"
                )
        return True

    callback_type = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    callback = callback_type(enum_window_callback)
    user32.EnumWindows(callback, 0)
    res = found_window.value or None
    if res is not None:
        if PLAYBACK_DEBUG:
            pid = wintypes.DWORD()
            user32.GetWindowThreadProcessId(res, ctypes.byref(pid))
            proc_name = get_process_name_by_pid(pid.value)
            title_len = user32.GetWindowTextLengthW(res)
            title_buf = ctypes.create_unicode_buffer(title_len + 1)
            user32.GetWindowTextW(res, title_buf, title_len + 1)
            debug_log(f"Detected Sky window: Title='{title_buf.value}', PID={pid.value}, ProcessName='{proc_name}'")
    else:
        for hwnd, title, pid_val, proc_name in rejected_candidates:
            if hwnd not in REJECTED_WINDOW_WARNINGS:
                REJECTED_WINDOW_WARNINGS.add(hwnd)
                print(
                    f"Rejected Sky-like window (untrusted process): Title={title!r}, "
                    f"PID={pid_val}, ProcessName={proc_name!r}"
                )
                print(
                    "If this is your actual game window, rerun with "
                    "--allow-title-fallback or set --sky-process-names correctly."
                )
    return res

def is_sky_window_valid() -> bool:
    global sky
    if sky is None or not user32.IsWindow(sky):
        sky = get_sky_window()
        return sky is not None

    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(sky, ctypes.byref(pid))
    proc_name = get_process_name_by_pid(pid.value)
    if proc_name in EXPECTED_PROCESS_NAMES:
        return True
    if EXPECTED_PROCESS_NAMES and not ALLOW_TITLE_FALLBACK:
        sky = get_sky_window()
        return sky is not None

    title_length = user32.GetWindowTextLengthW(sky)
    if title_length > 0:
        title_buffer = ctypes.create_unicode_buffer(title_length + 1)
        user32.GetWindowTextW(sky, title_buffer, title_length + 1)
        title = title_buffer.value
        if title == "Sky" or title.startswith("Sky"):
            return True

    sky = get_sky_window()
    return sky is not None

def focusWindow() -> bool:
    global sky
    if not is_sky_window_valid():
        return False
    foreground_window = user32.GetForegroundWindow()
    foreground_thread_id = user32.GetWindowThreadProcessId(foreground_window, None)
    current_thread_id = kernel32.GetCurrentThreadId()
    attached = False
    if foreground_thread_id != 0 and foreground_thread_id != current_thread_id:
        attached = bool(user32.AttachThreadInput(current_thread_id, foreground_thread_id, True))
    try:
        user32.ShowWindow(sky, SW_RESTORE)
        user32.SetWindowPos(sky, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW)
        user32.BringWindowToTop(sky)
        success = bool(user32.SetForegroundWindow(sky))
        user32.SetActiveWindow(sky)
        if not success and PLAYBACK_DEBUG:
            debug_log("[window] SetForegroundWindow failed to set Sky in foreground")
        return success
    finally:
        if attached:
            user32.AttachThreadInput(current_thread_id, foreground_thread_id, False)

def is_sky_active() -> bool:
    global sky
    return is_sky_window_valid() and user32.GetForegroundWindow() == sky

def is_virtual_key_down(key_code: int) -> bool:
    return bool(user32.GetAsyncKeyState(key_code) & 0x8000)
