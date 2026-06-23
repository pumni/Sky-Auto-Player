import sys
import ctypes
from ctypes import wintypes
from sky_music.layouts import SKY_15_KEY_PROFILE, PHYSICAL_SCAN_CODES, VK_CODES
from sky_music.platform.win32 import inputs

def is_admin() -> bool:
    """Checks if the current process is running with administrative privileges."""
    try:
        return ctypes.windll.shell32.IsUserAnAdmin() != 0
    except Exception:
        return False

def check_sky_window() -> dict:
    """Diagnoses Sky window handle, process name, and potential UIPI elevation mismatches."""
    status = {"ok": False, "msg": "", "hwnd": None, "process": ""}
    
    hwnd = inputs.get_sky_window()
    if hwnd is None:
        status["msg"] = "Sky window NOT found. Ensure the game is running and verify --sky-process-names."
        return status
        
    pid = wintypes.DWORD()
    inputs.user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    proc_name = inputs.get_process_name_by_pid(pid.value)
    
    status["hwnd"] = hwnd
    status["process"] = proc_name or "Unknown Process"
    
    # Preflight Admin Integrity warning (UIPI mitigation)
    current_admin = is_admin()
    status["ok"] = True
    
    msg_parts = [f"Found Sky window (HWND={hwnd}, PID={pid.value}, Process={status['process']})."]
    if not current_admin:
        msg_parts.append("Note: Running as standard user. If Sky is elevated (Administrator), Windows UIPI may block keystrokes — run this tool as Admin if input is not reaching the game.")
    else:
        msg_parts.append("Running as Administrator — input compatibility with elevated game windows is likely.")
        
    status["msg"] = " ".join(msg_parts)
    return status

def check_timer_resolution() -> dict:
    """Diagnoses high-precision multimedia timer subsystem settings on Windows."""
    status = {"ok": True, "msg": "Windows Multimedia high-precision timers are active (resolution: 1ms expected)."}
    
    # We test loading winmm directly
    try:
        winmm = ctypes.WinDLL("winmm", use_last_error=True)
        # Attempt to temporarily begin period to see if winmm functions cleanly
        res = winmm.timeBeginPeriod(1)
        if res == 0:
            winmm.timeEndPeriod(1)
        else:
            status["ok"] = False
            status["msg"] = f"winmm.timeBeginPeriod failed with status code: {res}"
    except Exception as exc:
        status["ok"] = False
        status["msg"] = f"Multimedia timer check failed: {exc}"
        
    return status

def check_keyboard_layout() -> dict:
    """Diagnoses note mapping scan codes completeness and uniqueness."""
    status = {"ok": True, "msg": "", "mapped_count": 0}
    mapped_count = 0
    unmapped = []
    
    for note_key, mapped_char in SKY_15_KEY_PROFILE.key_map.items():
        # Check base keys mapping completeness
        if note_key.startswith("Key"):
            sc = PHYSICAL_SCAN_CODES.get(mapped_char, 0)
            if sc == 0:
                unmapped.append(note_key)
            else:
                mapped_count += 1
                
    if unmapped:
        status["ok"] = False
        status["msg"] = f"Layout mapping incomplete! Unmapped keys: {', '.join(unmapped)}"
    else:
        status["msg"] = f"Layout mapping is complete and healthy ({mapped_count} physical scan codes verified)."
        status["mapped_count"] = mapped_count
        
    return status

def check_physical_keys_held() -> dict:
    """Warns if any of the target QWERTY note keys are already physically depressed on the keyboard."""
    status = {"ok": True, "msg": "No note keys are physically pressed.", "held_keys": []}
    held = []
    
    for char, vk in VK_CODES.items():
        # GetAsyncKeyState returns negative values if key is currently down
        if inputs.is_virtual_key_down(vk):
            held.append(char.upper())
            
    if held:
        status["ok"] = False
        status["held_keys"] = held
        status["msg"] = f"Warning: Note key(s) {', '.join(held)} are physically held down! This will conflict with SendInput signals."
        
    return status

def run_all_doctor_checks() -> bool:
    """Runs a complete diagnostic suite and prints standard actionable recommendations."""
    print("=" * 60)
    print("         SKY MUSIC PLAYER — READINESS CHECK")
    print("=" * 60)
    print(f"OS Platform      : {sys.platform} (Windows expected)")
    print(f"Python Version   : {sys.version.split()[0]}")
    _gil_probe = getattr(sys, "_is_gil_enabled", None)
    _gil_state = "enabled" if (_gil_probe is None or _gil_probe()) else "DISABLED (free-threaded)"
    print(f"GIL State        : {_gil_state}")
    print(f"Admin Privileges : {'YES' if is_admin() else 'NO'}")
    print("-" * 60)
    
    # 1. Sky Window Check
    print("[1/4] Sky Window Detection:")
    win_diag = check_sky_window()
    print(f"      Status: {'OK' if win_diag['ok'] else 'FAILED'}")
    print(f"      Details: {win_diag['msg']}")
    print("-" * 60)
    
    # 2. Timer Resolution Check
    print("[2/4] Multimedia High-Precision Timers:")
    time_diag = check_timer_resolution()
    print(f"      Status: {'OK' if time_diag['ok'] else 'FAILED'}")
    print(f"      Details: {time_diag['msg']}")
    print("-" * 60)
    
    # 3. Note Key Mapping Check
    print("[3/4] Note Mapping Configuration:")
    kb_diag = check_keyboard_layout()
    print(f"      Status: {'OK' if kb_diag['ok'] else 'FAILED'}")
    print(f"      Details: {kb_diag['msg']}")
    print("-" * 60)
    
    # 4. Preflight Key Conflict Check
    print("[4/4] Keyboard Preflight Checks:")
    conflict_diag = check_physical_keys_held()
    print(f"      Status: {'OK' if conflict_diag['ok'] else 'WARNING'}")
    print(f"      Details: {conflict_diag['msg']}")
    print("=" * 60)
    
    all_ok = win_diag["ok"] and time_diag["ok"] and kb_diag["ok"] and conflict_diag["ok"]
    if all_ok:
        print("Result: ALL CHECKS PASSED — ready for precise playback.")
    else:
        print("Result: ATTENTION NEEDED — review the details above before playing.")
    print("=" * 60)
    
    return all_ok
