"""RegisterHotKey-backed global playback controls.

This module is the only low-level binding used by the playback hotkey path.
It deliberately has no keyboard hook and no polling loop: Windows delivers a
``WM_HOTKEY`` message to one dedicated thread, which forwards the action to a
thread-safe queue.
"""

from __future__ import annotations

import ctypes
import queue
import sys
import threading
from collections.abc import Mapping
from ctypes import wintypes
from dataclasses import dataclass
from typing import Protocol

WM_HOTKEY = 0x0312
WM_QUIT = 0x0012
MOD_ALT = 0x0001
MOD_CONTROL = 0x0002
MOD_SHIFT = 0x0004
MOD_NOREPEAT = 0x4000
HOTKEY_ID_BASE = 0x5A00


class GlobalHotkeyError(RuntimeError):
    """Base error for registration and message-loop failures."""


class GlobalHotkeyConflictError(GlobalHotkeyError):
    """Raised when Windows refuses one of the requested registrations."""


@dataclass(frozen=True, slots=True)
class HotkeyRegistration:
    """Validated data needed by ``RegisterHotKey``."""

    action: str
    virtual_key: int
    modifiers: int
    identifier: int

    def __post_init__(self) -> None:
        if not self.action or not self.action.replace("_", "").isalnum():
            raise ValueError("hotkey action must be a non-empty identifier")
        if type(self.virtual_key) is not int or not 1 <= self.virtual_key <= 0xFF:
            raise ValueError("virtual_key must be in 1..255")
        if type(self.modifiers) is not int or self.modifiers & ~(
            MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT
        ):
            raise ValueError("unsupported RegisterHotKey modifier")
        if type(self.identifier) is not int or not HOTKEY_ID_BASE <= self.identifier < HOTKEY_ID_BASE + 0x100:
            raise ValueError("hotkey identifier is outside the reserved range")


class _BindingLike(Protocol):
    @property
    def key_code(self) -> int: ...

    @property
    def ctrl(self) -> bool: ...

    @property
    def alt(self) -> bool: ...

    @property
    def shift(self) -> bool: ...


def modifiers_for_binding(binding: _BindingLike) -> int:
    """Convert a validated ``HotkeyBinding``-like object to Win32 flags."""

    modifiers = MOD_NOREPEAT
    if binding.ctrl:
        modifiers |= MOD_CONTROL
    if binding.alt:
        modifiers |= MOD_ALT
    if binding.shift:
        modifiers |= MOD_SHIFT
    return modifiers


def build_registrations(bindings: Mapping[str, _BindingLike]) -> tuple[HotkeyRegistration, ...]:
    """Create deterministic registrations and reject duplicate actions/IDs."""

    if not bindings:
        return ()
    registrations: list[HotkeyRegistration] = []
    seen_actions: set[str] = set()
    seen_chords: set[tuple[int, int]] = set()
    for index, (action, binding) in enumerate(bindings.items()):
        if action in seen_actions:
            raise ValueError(f"duplicate hotkey action: {action}")
        registration = HotkeyRegistration(
            action=action,
            virtual_key=binding.key_code,
            modifiers=modifiers_for_binding(binding),
            identifier=HOTKEY_ID_BASE + index,
        )
        chord = (registration.modifiers, registration.virtual_key)
        if chord in seen_chords:
            raise ValueError(f"duplicate hotkey chord for action: {action}")
        seen_actions.add(action)
        seen_chords.add(chord)
        registrations.append(registration)
    return tuple(registrations)


if sys.platform == "win32":
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    user32.RegisterHotKey.argtypes = (
        wintypes.HWND,
        ctypes.c_int,
        wintypes.UINT,
        wintypes.UINT,
    )
    user32.RegisterHotKey.restype = wintypes.BOOL
    user32.UnregisterHotKey.argtypes = (wintypes.HWND, ctypes.c_int)
    user32.UnregisterHotKey.restype = wintypes.BOOL
    user32.GetMessageW.argtypes = (
        ctypes.POINTER(wintypes.MSG),
        wintypes.HWND,
        wintypes.UINT,
        wintypes.UINT,
    )
    user32.GetMessageW.restype = ctypes.c_int
    user32.PeekMessageW.argtypes = (
        ctypes.POINTER(wintypes.MSG),
        wintypes.HWND,
        wintypes.UINT,
        wintypes.UINT,
        wintypes.UINT,
    )
    user32.PeekMessageW.restype = wintypes.BOOL
    user32.PostThreadMessageW.argtypes = (
        wintypes.DWORD,
        wintypes.UINT,
        wintypes.WPARAM,
        wintypes.LPARAM,
    )
    user32.PostThreadMessageW.restype = wintypes.BOOL
    kernel32.GetCurrentThreadId.argtypes = ()
    kernel32.GetCurrentThreadId.restype = wintypes.DWORD
else:
    user32 = None
    kernel32 = None


class GlobalHotkeyListener:
    """One blocking ``GetMessageW`` thread for a complete action set."""

    def __init__(self, registrations: tuple[HotkeyRegistration, ...]) -> None:
        self._registrations = registrations
        self._by_id = {item.identifier: item.action for item in registrations}
        self._events: queue.Queue[str] = queue.Queue()
        self._thread: threading.Thread | None = None
        self._thread_id = 0
        self._ready = threading.Event()
        self._closed = False
        self._error: GlobalHotkeyError | None = None
        self._lifecycle_lock = threading.Lock()

    @classmethod
    def from_bindings(cls, bindings: Mapping[str, _BindingLike]) -> GlobalHotkeyListener:
        return cls(build_registrations(bindings))

    def start(self) -> None:
        with self._lifecycle_lock:
            if self._thread is not None:
                if self._error is not None:
                    raise self._error
                return
            self._closed = False
            self._thread = threading.Thread(
                target=self._run,
                name="sky-global-hotkeys",
                daemon=True,
            )
            self._thread.start()
        self._ready.wait(timeout=2.0)
        if not self._ready.is_set():
            self.close()
            raise GlobalHotkeyError("global hotkey thread did not initialize")
        if self._error is not None:
            error = self._error
            self.close()
            raise error

    def poll(self) -> str | None:
        try:
            return self._events.get_nowait()
        except queue.Empty:
            return None

    def close(self) -> None:
        with self._lifecycle_lock:
            thread = self._thread
            thread_id = self._thread_id
            if thread is None:
                return
            self._closed = True
        if sys.platform == "win32" and thread_id:
            if not bool(user32.PostThreadMessageW(thread_id, WM_QUIT, 0, 0)):
                # The thread may already have left after a registration error.
                pass
        if thread is not threading.current_thread():
            thread.join(timeout=2.0)
        with self._lifecycle_lock:
            if not thread.is_alive():
                self._thread = None

    def _run(self) -> None:
        registered: list[int] = []
        try:
            if sys.platform != "win32":
                self._ready.set()
                return

            self._thread_id = int(kernel32.GetCurrentThreadId())
            # Creating the message queue before signalling readiness makes an
            # immediate close() reliable (PostThreadMessage otherwise races
            # queue creation).
            message = wintypes.MSG()
            user32.PeekMessageW(ctypes.byref(message), None, 0, 0, 0)
            for registration in self._registrations:
                if not bool(
                    user32.RegisterHotKey(
                        None,
                        registration.identifier,
                        registration.modifiers,
                        registration.virtual_key,
                    )
                ):
                    raise GlobalHotkeyConflictError(
                        f"RegisterHotKey failed for {registration.action} "
                        f"(Win32 error {ctypes.get_last_error()})"
                    )
                registered.append(registration.identifier)
            self._ready.set()
            while not self._closed:
                result = int(user32.GetMessageW(ctypes.byref(message), None, 0, 0))
                if result == -1:
                    raise GlobalHotkeyError(
                        f"GetMessageW failed (Win32 error {ctypes.get_last_error()})"
                    )
                if result == 0:
                    break
                if message.message == WM_HOTKEY:
                    action = self._by_id.get(int(message.wParam))
                    if action is not None:
                        self._events.put(action)
        except GlobalHotkeyError as exc:
            self._error = exc
            self._ready.set()
        except Exception as exc:
            self._error = GlobalHotkeyError(f"global hotkey loop failed: {exc}")
            self._ready.set()
        finally:
            if sys.platform == "win32":
                for identifier in registered:
                    user32.UnregisterHotKey(None, identifier)
            self._ready.set()
