import ctypes
import queue
import threading
from ctypes import wintypes
from typing import Any

from sky_music.platform.win32.inputs import SKY_PLAYER_SIGNATURE

# Windows API constants
WH_KEYBOARD_LL = 13
WM_KEYDOWN = 0x0100
WM_SYSKEYDOWN = 0x0104
WM_KEYUP = 0x0101
WM_SYSKEYUP = 0x0105

# Virtual Key Codes
VK_LSHIFT = 0xA0
VK_RSHIFT = 0xA1
VK_LCONTROL = 0xA2
VK_RCONTROL = 0xA3
VK_LMENU = 0xA4
VK_RMENU = 0xA5

class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("vkCode", wintypes.DWORD),
        ("scanCode", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))
    ]

HOOKPROC = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_int, wintypes.WPARAM, ctypes.POINTER(KBDLLHOOKSTRUCT))

user32 = ctypes.WinDLL('user32', use_last_error=True)

user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wintypes.HINSTANCE, wintypes.DWORD]
user32.SetWindowsHookExW.restype = wintypes.HHOOK

user32.UnhookWindowsHookEx.argtypes = [wintypes.HHOOK]
user32.UnhookWindowsHookEx.restype = wintypes.BOOL

user32.CallNextHookEx.argtypes = [wintypes.HHOOK, ctypes.c_int, wintypes.WPARAM, ctypes.POINTER(KBDLLHOOKSTRUCT)]
user32.CallNextHookEx.restype = ctypes.c_long

user32.GetMessageW.argtypes = [ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT, wintypes.UINT]
user32.GetMessageW.restype = wintypes.BOOL

user32.TranslateMessage.argtypes = [ctypes.POINTER(wintypes.MSG)]
user32.TranslateMessage.restype = wintypes.BOOL

user32.DispatchMessageW.argtypes = [ctypes.POINTER(wintypes.MSG)]
user32.DispatchMessageW.restype = ctypes.c_long

user32.PostThreadMessageW.argtypes = [wintypes.DWORD, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.PostThreadMessageW.restype = wintypes.BOOL

WM_QUIT = 0x0012

class HotkeyHook:
    def __init__(self, controls: Any):
        self.controls = controls
        # Bound against keypress floods (DoS of RAM). Drop-on-full in _hook_proc.
        self.event_queue: queue.Queue[str] = queue.Queue(maxsize=64)
        self._hook_id: Any = None
        self._thread: threading.Thread | None = None
        self._thread_id: int = 0
        self._hook_proc_ref: Any = None
        
        # Track modifier state
        self._ctrl_down = False
        self._alt_down = False
        self._shift_down = False

    def _hook_proc(self, nCode: int, wParam: Any, lParam: Any) -> int:
        if nCode < 0:
            return user32.CallNextHookEx(self._hook_id, nCode, wParam, lParam)

        info = lParam.contents
        
        # Feedback loop guard: ignore our own injected keys
        if info.dwExtraInfo:
            extra = info.dwExtraInfo[0]
            if (extra & SKY_PLAYER_SIGNATURE) == SKY_PLAYER_SIGNATURE:
                return user32.CallNextHookEx(self._hook_id, nCode, wParam, lParam)

        vkCode = info.vkCode
        is_down = wParam in (WM_KEYDOWN, WM_SYSKEYDOWN)
        
        if vkCode in (VK_LCONTROL, VK_RCONTROL):
            self._ctrl_down = is_down
        elif vkCode in (VK_LMENU, VK_RMENU):
            self._alt_down = is_down
        elif vkCode in (VK_LSHIFT, VK_RSHIFT):
            self._shift_down = is_down

        if is_down and self.controls.enabled:
            # Check against hotkeys
            for action, binding in (
                ("quit", self.controls.quit),
                ("skip", self.controls.skip),
                ("pause", self.controls.pause),
                ("refocus", self.controls.refocus),
                ("panic", self.controls.panic),
            ):
                if vkCode == binding.key_code and (
                    self._ctrl_down == binding.ctrl and 
                    self._alt_down == binding.alt and 
                    self._shift_down == binding.shift
                ):
                    # Swallow it; drop queue write on flood (never block the OS hook chain).
                    try:
                        self.event_queue.put_nowait(action)
                    except queue.Full:
                        pass
                    return 1

        return user32.CallNextHookEx(self._hook_id, nCode, wParam, lParam)

    def _run_pump(self) -> None:
        self._thread_id = threading.get_native_id()
        self._hook_proc_ref = HOOKPROC(self._hook_proc)
        self._hook_id = user32.SetWindowsHookExW(WH_KEYBOARD_LL, self._hook_proc_ref, None, 0)
        
        if not self._hook_id:
            # Hook never installed — safe to drop the ctypes wrapper immediately.
            self._hook_id = None
            self._hook_proc_ref = None
            return

        msg = wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))
            
        user32.UnhookWindowsHookEx(self._hook_id)
        # Null only after UnhookWindowsHookEx — the OS must not call through a freed wrapper.
        self._hook_id = None
        self._hook_proc_ref = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self._run_pump, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        if self._thread and self._thread.is_alive() and self._thread_id:
            user32.PostThreadMessageW(self._thread_id, WM_QUIT, 0, 0)
            self._thread.join(timeout=2.0)
        # _hook_proc_ref / _hook_id are nulled inside _run_pump after UnhookWindowsHookEx.
        self._thread = None
            
    def poll(self) -> str | None:
        try:
            return self.event_queue.get_nowait()
        except queue.Empty:
            return None
