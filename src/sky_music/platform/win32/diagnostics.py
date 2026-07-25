"""Windows diagnostics kept behind the platform boundary."""
from __future__ import annotations

import ctypes
import sys
from typing import Any

_winmm: Any | None = None


def _get_winmm() -> Any:
    global _winmm
    if _winmm is None:
        _winmm = ctypes.WinDLL("winmm", use_last_error=True)
    return _winmm


def is_process_elevated() -> bool:
    """Return whether this process has administrator privileges."""
    if sys.platform != "win32":
        return False
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except (AttributeError, OSError):
        return False


def check_timer_resolution() -> dict[str, object]:
    """Probe the multimedia timer API without leaving its period changed."""
    status: dict[str, object] = {
        "ok": True,
        "msg": "Windows Multimedia high-precision timers are active (resolution: 1ms expected).",
    }
    if sys.platform != "win32":
        status["ok"] = False
        status["msg"] = "Multimedia timer check requires Windows."
        return status
    try:
        winmm = _get_winmm()
        result = winmm.timeBeginPeriod(1)
        if result == 0:
            winmm.timeEndPeriod(1)
        else:
            status["ok"] = False
            status["msg"] = f"winmm.timeBeginPeriod failed with status code: {result}"
    except (AttributeError, OSError) as exc:
        status["ok"] = False
        status["msg"] = f"Multimedia timer check failed: {exc}"
    return status
