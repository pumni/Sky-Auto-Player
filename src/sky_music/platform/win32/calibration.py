import ctypes
import json
import threading
import time
from ctypes import wintypes
from datetime import datetime
from pathlib import Path
from typing import Any

# Win32 Constants
CS_HREDRAW = 0x0002
CS_VREDRAW = 0x0001
IDC_ARROW = 32512
COLOR_WINDOW = 5
WS_OVERLAPPEDWINDOW = 0x00CF0000
SW_SHOW = 5
WM_DESTROY = 0x0002
WM_PAINT = 0x000F
WM_INPUT = 0x00FF
RID_INPUT = 0x10000003
RIDEV_INPUTSINK = 0x00000100
RIM_TYPEKEYBOARD = 1
WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101

# SendInput Constants
INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008
SKY_PLAYER_SIGNATURE = 0x5C1B9111

# Paint / DrawText Constants
DT_CENTER = 0x00000001
DT_VCENTER = 0x00000004
DT_WORDBREAK = 0x00000010

# Types and structs
WNDPROC = ctypes.WINFUNCTYPE(wintypes.LPARAM, wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)

class WNDCLASSEXW(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.UINT),
        ("style", wintypes.UINT),
        ("lpfnWndProc", WNDPROC),
        ("cbClsExtra", ctypes.c_int),
        ("cbWndExtra", ctypes.c_int),
        ("hInstance", wintypes.HINSTANCE),
        ("hIcon", wintypes.HICON),
        ("hCursor", wintypes.HCURSOR),
        ("hbrBackground", wintypes.HBRUSH),
        ("lpszMenuName", wintypes.LPCWSTR),
        ("lpszClassName", wintypes.LPCWSTR),
        ("hIconSm", wintypes.HICON),
    ]

class PAINTSTRUCT(ctypes.Structure):
    _fields_ = [
        ("hdc", wintypes.HDC),
        ("fErase", wintypes.BOOL),
        ("rcPaint", wintypes.RECT),
        ("fRestore", wintypes.BOOL),
        ("fIncUpdate", wintypes.BOOL),
        ("rgbReserved", wintypes.BYTE * 32),
    ]

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

class RAWINPUTDEVICE(ctypes.Structure):
    _fields_ = [
        ("usUsagePage", wintypes.USHORT),
        ("usUsage", wintypes.USHORT),
        ("dwFlags", wintypes.DWORD),
        ("hwndTarget", wintypes.HWND),
    ]

class RAWINPUTHEADER(ctypes.Structure):
    _fields_ = [
        ("dwType", wintypes.DWORD),
        ("dwSize", wintypes.DWORD),
        ("hDevice", wintypes.HANDLE),
        ("wParam", wintypes.WPARAM),
    ]

class RAWKEYBOARD(ctypes.Structure):
    _fields_ = [
        ("MakeCode", wintypes.USHORT),
        ("Flags", wintypes.USHORT),
        ("Reserved", wintypes.USHORT),
        ("VKey", wintypes.USHORT),
        ("Message", wintypes.UINT),
        ("ExtraInformation", wintypes.ULONG),
    ]

class RAWINPUT(ctypes.Structure):
    _fields_ = [
        ("header", RAWINPUTHEADER),
        ("keyboard", RAWKEYBOARD),
    ]

# Setup Windows APIs
user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

user32.RegisterClassExW.argtypes = (ctypes.POINTER(WNDCLASSEXW),)
user32.RegisterClassExW.restype = wintypes.ATOM

user32.CreateWindowExW.argtypes = (
    wintypes.DWORD,
    wintypes.LPCWSTR,
    wintypes.LPCWSTR,
    wintypes.DWORD,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.HWND,
    wintypes.HMENU,
    wintypes.HINSTANCE,
    wintypes.LPVOID,
)
user32.CreateWindowExW.restype = wintypes.HWND

user32.ShowWindow.argtypes = (wintypes.HWND, ctypes.c_int)
user32.ShowWindow.restype = wintypes.BOOL

user32.UpdateWindow.argtypes = (wintypes.HWND,)
user32.UpdateWindow.restype = wintypes.BOOL

user32.InvalidateRect.argtypes = (wintypes.HWND, wintypes.LPRECT, wintypes.BOOL)
user32.InvalidateRect.restype = wintypes.BOOL

user32.GetMessageW.argtypes = (ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT, wintypes.UINT)
user32.GetMessageW.restype = wintypes.BOOL

user32.TranslateMessage.argtypes = (ctypes.POINTER(wintypes.MSG),)
user32.TranslateMessage.restype = wintypes.BOOL

user32.DispatchMessageW.argtypes = (ctypes.POINTER(wintypes.MSG),)
user32.DispatchMessageW.restype = wintypes.LPARAM

user32.PostQuitMessage.argtypes = (ctypes.c_int,)
user32.PostQuitMessage.restype = None

user32.DefWindowProcW.argtypes = (wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)
user32.DefWindowProcW.restype = wintypes.LPARAM

user32.GetForegroundWindow.argtypes = ()
user32.GetForegroundWindow.restype = wintypes.HWND

user32.RegisterRawInputDevices.argtypes = (ctypes.POINTER(RAWINPUTDEVICE), wintypes.UINT, wintypes.UINT)
user32.RegisterRawInputDevices.restype = wintypes.BOOL

user32.GetRawInputData.argtypes = (
    wintypes.HANDLE,
    wintypes.UINT,
    ctypes.c_void_p,
    ctypes.POINTER(wintypes.UINT),
    wintypes.UINT,
)
user32.GetRawInputData.restype = wintypes.UINT

user32.BeginPaint.argtypes = (wintypes.HWND, ctypes.POINTER(PAINTSTRUCT))
user32.BeginPaint.restype = wintypes.HDC

user32.EndPaint.argtypes = (wintypes.HWND, ctypes.POINTER(PAINTSTRUCT))
user32.EndPaint.restype = wintypes.BOOL

user32.GetClientRect.argtypes = (wintypes.HWND, ctypes.POINTER(wintypes.RECT))
user32.GetClientRect.restype = wintypes.BOOL

user32.DrawTextW.argtypes = (wintypes.HDC, wintypes.LPCWSTR, ctypes.c_int, ctypes.POINTER(wintypes.RECT), wintypes.UINT)
user32.DrawTextW.restype = ctypes.c_int

user32.SendInput.argtypes = (wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int)
user32.SendInput.restype = wintypes.UINT

kernel32.GetModuleHandleW.argtypes = (wintypes.LPCWSTR,)
kernel32.GetModuleHandleW.restype = wintypes.HINSTANCE


_WND_PROC_REFS: list[Any] = []


class CalibrationHarness:
    def __init__(self, scancode: int = 0x1E):
        self.scancode = scancode
        self.hwnd: Any = None
        self.injections_done = threading.Event()
        self.down_latencies_us: list[float] = []
        self.up_latencies_us: list[float] = []
        self.last_send_time_ns: int = 0
        self.last_send_type: str | None = None
        self.input_event = threading.Event()
        self.aborted = False
        self.abort_reason = ""


def percentile(data: list[float], pct: float) -> float:
    if not data:
        return 0.0
    sorted_data = sorted(data)
    idx = (len(sorted_data) - 1) * (pct / 100.0)
    floor_idx = int(idx)
    ceil_idx = min(floor_idx + 1, len(sorted_data) - 1)
    weight = idx - floor_idx
    return sorted_data[floor_idx] * (1.0 - weight) + sorted_data[ceil_idx] * weight


def run_calibration_loop(harness: CalibrationHarness) -> None:
    # Give the user a moment to focus the window, or wait if needed
    for _ in range(100):
        if harness.aborted:
            return
        if user32.GetForegroundWindow() == harness.hwnd:
            break
        time.sleep(0.1)
    else:
        harness.aborted = True
        harness.abort_reason = "Calibration window did not gain focus."
        harness.injections_done.set()
        if harness.hwnd:
            user32.PostMessageW(harness.hwnd, WM_DESTROY, 0, 0)
        return

    # Warmup sleeps
    time.sleep(0.5)

    try:
        for _i in range(200):
            if harness.aborted:
                break

            # --- DOWN injection ---
            if user32.GetForegroundWindow() != harness.hwnd:
                harness.aborted = True
                harness.abort_reason = "Window lost focus during calibration."
                break

            harness.input_event.clear()
            harness.last_send_type = "down"

            inputs_array = (INPUT * 1)()
            inputs_array[0].type = INPUT_KEYBOARD
            inputs_array[0].ki.wVk = 0
            inputs_array[0].ki.wScan = harness.scancode
            inputs_array[0].ki.dwFlags = KEYEVENTF_SCANCODE
            inputs_array[0].ki.time = 0
            inputs_array[0].ki.dwExtraInfo = SKY_PLAYER_SIGNATURE

            # Trigger SendInput and capture timestamp immediately on return
            res = user32.SendInput(1, inputs_array, ctypes.sizeof(INPUT))
            t_send = time.perf_counter_ns()
            if res != 1:
                harness.aborted = True
                harness.abort_reason = f"SendInput failed with return code {res}."
                break

            harness.last_send_time_ns = t_send

            # Wait for WM_INPUT confirmation
            if not harness.input_event.wait(timeout=1.0):
                harness.aborted = True
                harness.abort_reason = "Timeout waiting for raw input Down event."
                break

            # Safe spacing (at least 20ms, using 25ms to be safe)
            time.sleep(0.025)

            # --- UP injection ---
            if user32.GetForegroundWindow() != harness.hwnd:
                harness.aborted = True
                harness.abort_reason = "Window lost focus during calibration."
                break

            harness.input_event.clear()
            harness.last_send_type = "up"

            inputs_array[0].ki.dwFlags = KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP

            res = user32.SendInput(1, inputs_array, ctypes.sizeof(INPUT))
            t_send = time.perf_counter_ns()
            if res != 1:
                harness.aborted = True
                harness.abort_reason = f"SendInput failed with return code {res}."
                break

            harness.last_send_time_ns = t_send

            if not harness.input_event.wait(timeout=1.0):
                harness.aborted = True
                harness.abort_reason = "Timeout waiting for raw input Up event."
                break

            time.sleep(0.025)

    finally:
        harness.injections_done.set()
        # Shut down window message pump
        if harness.hwnd:
            user32.PostMessageW(harness.hwnd, WM_DESTROY, 0, 0)


def calibrate_input_latency_harness(scancode: int = 0x1E) -> dict[str, Any]:
    """Execute raw input latency calibration on an app-owned window."""
    harness = CalibrationHarness(scancode)
    
    # We must keep references to delegates so they aren't garbage collected
    def wnd_proc(hwnd: int, msg: int, wparam: int, lparam: int) -> int:
        t_recv = time.perf_counter_ns()
        if msg == WM_INPUT:
            cbSize = wintypes.UINT()
            user32.GetRawInputData(
                lparam,
                0x10000003,  # RID_INPUT
                None,
                ctypes.byref(cbSize),
                ctypes.sizeof(RAWINPUTHEADER)
            )
            if cbSize.value > 0:
                buffer = ctypes.create_string_buffer(cbSize.value)
                if user32.GetRawInputData(
                    lparam,
                    0x10000003,
                    buffer,
                    ctypes.byref(cbSize),
                    ctypes.sizeof(RAWINPUTHEADER)
                ) == cbSize.value:
                    raw = RAWINPUT.from_buffer(buffer)
                    if raw.header.dwType == RIM_TYPEKEYBOARD:
                        kb = raw.keyboard
                        if kb.MakeCode == harness.scancode:
                            is_up = bool(kb.Flags & 1)
                            if is_up and harness.last_send_type == "up":
                                # ns delta / 1000 → microseconds
                                latency = float(t_recv - harness.last_send_time_ns) / 1000.0
                                harness.up_latencies_us.append(latency)
                                harness.input_event.set()
                            elif not is_up and harness.last_send_type == "down":
                                latency = float(t_recv - harness.last_send_time_ns) / 1000.0
                                harness.down_latencies_us.append(latency)
                                harness.input_event.set()
                                # Progress text is only drawn on WM_PAINT; without this the
                                # window stays at "0 / 200" until a resize/uncover forces paint.
                                user32.InvalidateRect(hwnd, None, True)
            return user32.DefWindowProcW(hwnd, msg, wparam, lparam)

        if msg == WM_PAINT:
            ps = PAINTSTRUCT()
            hdc = user32.BeginPaint(hwnd, ctypes.byref(ps))
            rect = wintypes.RECT()
            user32.GetClientRect(hwnd, ctypes.byref(rect))
            
            # Simple text rendering
            text = (
                "Sky Auto Player Latency Calibration\n\n"
                "Measuring input delivery latency...\n"
                "Please keep this window focused.\n\n"
                f"Progress: {len(harness.down_latencies_us)} / 200"
            )
            user32.DrawTextW(hdc, text, -1, ctypes.byref(rect), DT_CENTER | DT_VCENTER | DT_WORDBREAK)
            user32.EndPaint(hwnd, ctypes.byref(ps))
            return 0

        if msg == WM_DESTROY:
            user32.PostQuitMessage(0)
            return 0

        return user32.DefWindowProcW(hwnd, msg, wparam, lparam)

    # Window creation thread
    window_ready = threading.Event()
    window_error: list[Exception | None] = [None]
    global _WND_PROC_REFS
    _WND_PROC_REFS = []

    def create_window_and_pump():
        try:
            h_inst = kernel32.GetModuleHandleW(None)
            
            wnd_proc_delegate = WNDPROC(wnd_proc)
            # Keep reference alive
            _WND_PROC_REFS.append(wnd_proc_delegate)

            class_name = "SkyPlayerCalibrationWindow"
            wcex = WNDCLASSEXW()
            wcex.cbSize = ctypes.sizeof(WNDCLASSEXW)
            wcex.style = CS_HREDRAW | CS_VREDRAW
            wcex.lpfnWndProc = wnd_proc_delegate
            wcex.cbClsExtra = 0
            wcex.cbWndExtra = 0
            wcex.hInstance = h_inst
            wcex.hIcon = 0
            wcex.hCursor = user32.LoadCursorW(0, IDC_ARROW)
            wcex.hbrBackground = COLOR_WINDOW + 1
            wcex.lpszMenuName = None
            wcex.lpszClassName = class_name
            wcex.hIconSm = 0

            if not user32.RegisterClassExW(ctypes.byref(wcex)):
                raise OSError("RegisterClassExW failed.")

            hwnd = user32.CreateWindowExW(
                0,
                class_name,
                "Sky Auto Player Input Latency Calibration",
                WS_OVERLAPPEDWINDOW,
                100, 100, 400, 300,
                0, 0, h_inst, None
            )

            if not hwnd:
                raise OSError("CreateWindowExW failed.")

            harness.hwnd = hwnd

            # Register Raw Input Device
            rid = RAWINPUTDEVICE()
            rid.usUsagePage = 1
            rid.usUsage = 6
            rid.dwFlags = RIDEV_INPUTSINK
            rid.hwndTarget = hwnd

            if not user32.RegisterRawInputDevices(ctypes.byref(rid), 1, ctypes.sizeof(RAWINPUTDEVICE)):
                raise OSError("RegisterRawInputDevices failed.")

            user32.ShowWindow(hwnd, SW_SHOW)
            user32.UpdateWindow(hwnd)
            user32.SetForegroundWindow(hwnd)

            window_ready.set()

            # Message pump loop
            msg = wintypes.MSG()
            while user32.GetMessageW(ctypes.byref(msg), 0, 0, 0):
                user32.TranslateMessage(ctypes.byref(msg))
                user32.DispatchMessageW(ctypes.byref(msg))

        except Exception as exc:
            window_error[0] = exc
            window_ready.set()

    t = threading.Thread(target=create_window_and_pump, daemon=True)
    t.start()

    window_ready.wait()
    if window_error[0]:
        raise window_error[0]

    # Run injection loop on main thread (or another thread)
    inj_thread = threading.Thread(target=run_calibration_loop, args=(harness,), daemon=True)
    inj_thread.start()

    # Wait for completion or timeout
    harness.injections_done.wait()

    if harness.aborted:
        raise RuntimeError(f"Calibration aborted: {harness.abort_reason}")

    # Compute latency stats (microseconds)
    down_lat = harness.down_latencies_us
    up_lat = harness.up_latencies_us

    result = {
        "version": 1,
        "down_us": {
            "p50": round(percentile(down_lat, 50.0)),
            "p90": round(percentile(down_lat, 90.0)),
            "p99": round(percentile(down_lat, 99.0)),
        },
        "up_us": {
            "p50": round(percentile(up_lat, 50.0)),
            "p90": round(percentile(up_lat, 90.0)),
            "p99": round(percentile(up_lat, 99.0)),
        },
        "sampled_at": datetime.now().isoformat(),
        "n": len(down_lat),
    }

    # Save to .cache/input_latency.json
    cache_dir = Path(".cache")
    cache_dir.mkdir(exist_ok=True)
    with open(cache_dir / "input_latency.json", "w", encoding="utf-8") as f:
        json.dump(result, f, indent=4)

    return result
