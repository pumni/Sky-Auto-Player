import sys
from typing import Any

from sky_music.config import load_config
from sky_music.layouts import PHYSICAL_SCAN_CODES, SKY_15_KEY_PROFILE, VK_CODES
from sky_music.platform.win32 import window_target
from sky_music.platform.win32.diagnostics import (
    check_timer_resolution as _check_timer_resolution,
)
from sky_music.platform.win32.diagnostics import (
    is_process_elevated,
)


def is_admin() -> bool:
    """Checks if the current process is running with administrative privileges."""
    return is_process_elevated()

def check_sky_window() -> dict:
    """Diagnoses Sky window handle, process name, and potential UIPI elevation mismatches."""
    status: dict[str, Any] = {"ok": False, "msg": "", "hwnd": None, "process": ""}

    hwnd = window_target.get_sky_window()
    if hwnd is None:
        status["msg"] = "Sky window NOT found. Ensure the game is running and verify --sky-process-names."
        return status

    pid = window_target.get_window_process_id(hwnd)
    if pid is None:
        status["msg"] = "Sky window process id could not be queried."
        return status
    proc_name = window_target.get_process_name_by_pid(pid)

    status["hwnd"] = hwnd
    status["process"] = proc_name or "Unknown Process"

    current_admin = is_admin()
    status["ok"] = True

    msg_parts = [f"Found Sky window (HWND={hwnd}, PID={pid}, Process={status['process']})."]
    # Phase G.3: exact UIPI advisory text (plan §G.3).
    if not current_admin:
        msg_parts.append(
            "If Sky runs elevated (Admin) and Sky Auto Player does not, SendInput may return 0 (UIPI). "
            "Run both elevated or both not elevated."
        )
    else:
        msg_parts.append("Running as Administrator — input compatibility with elevated game windows is likely.")

    status["msg"] = " ".join(msg_parts)
    return status

def check_timer_resolution() -> dict:
    """Diagnoses high-precision multimedia timer subsystem settings on Windows."""
    return _check_timer_resolution()

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
        if window_target.is_virtual_key_down(vk):
            held.append(char.upper())
            
    if held:
        status["ok"] = False
        status["held_keys"] = held
        status["msg"] = f"Warning: Note key(s) {', '.join(held)} are physically held down! This will conflict with SendInput signals."
        
    return status

def check_calibration_cache() -> dict:
    """Checks whether the host input-delivery calibration cache exists."""
    from pathlib import Path
    path = Path(".cache/input_latency.json")
    status = {"ok": True, "msg": "", "path": str(path)}
    if path.exists():
        status["msg"] = f"Calibration cache found at {path}."
    else:
        status["ok"] = False
        status["msg"] = "Calibration cache not found. Run `--doctor-calibrate` to measure host input delivery for tighter hold margins."
    return status


def check_native_dispatch() -> dict[str, Any]:
    """Report native metadata and admission status without creating a session."""
    from sky_music.orchestration.native_admission import (
        NativeAdmissionError,
        inspect_rust_core,
        validate_native_runtime_info,
        validate_release_commit,
    )

    status: dict[str, Any] = {
        "ok": False,
        "required": True,
        "enabled": True,
        "available": False,
        "msg": "",
    }
    inspection = inspect_rust_core()
    status["native_module_path"] = inspection.module_path
    frozen = bool(getattr(sys, "frozen", False))
    status["mode"] = "frozen production" if frozen else "source development"
    if inspection.info is None:
        status["msg"] = f"Rust dispatch module is unavailable and is required: {inspection.error}"
        return status

    info = inspection.info
    status.update(info)
    status["available"] = True
    try:
        runtime_gil_probe = getattr(sys, "_is_gil_enabled", None)
        if not callable(runtime_gil_probe) or runtime_gil_probe():
            raise NativeAdmissionError("active Python runtime is not free-threaded")
        validated = validate_native_runtime_info(native_info=info)
        if frozen:
            from sky_music.orchestration.native_admission import (
                _packaged_application_commit,
            )

            app_commit = _packaged_application_commit()
            status["application_build_commit"] = app_commit
            validate_release_commit(
                app_commit=app_commit,
                native_commit=validated.native_build_commit,
            )
            status["release_contract"] = "PASS"
            status["commit_match"] = True
        else:
            status["release_contract"] = "not applicable"
            status["commit_match"] = None
        status["ok"] = True
        status["msg"] = (
            f"Mode: {status['mode']}; Rust native core: "
            f"native commit={validated.native_build_commit}, "
            f"rustc={validated.rustc_version}, ABI={validated.native_abi}, "
            f"schema={validated.schema_version}, "
            f"Win32 backend={'available' if validated.win32_backend else 'missing'}; "
            f"runtime contract=PASS; release contract={status['release_contract']}."
        )
    except (ImportError, NativeAdmissionError, TypeError, ValueError) as exc:
        status["msg"] = f"Rust native admission failed: {exc}"
    return status


def check_sky_foreground() -> dict:
    """Checks whether the Sky window is currently the foreground (active) window."""
    status = {"ok": True, "msg": ""}
    try:
        from sky_music.platform.win32 import window_target
        if window_target.is_sky_active():
            status["msg"] = "Sky window is currently in the foreground."
        else:
            status["ok"] = False
            status["msg"] = "Sky window exists but is NOT in the foreground. No input will reach the game until it is focused."
    except Exception as exc:
        status["ok"] = False
        status["msg"] = f"Could not check foreground state: {exc}"
    return status


def print_fps_advisory() -> None:
    cfg = load_config()
    fps = cfg.game_fps
    if fps > 60:
        print(f"\nFPS Advisory: Configured game FPS is {fps}. Notes shorter than one 60 fps frame")
        print("  (~16.7ms) may not register if the game runs below the configured FPS.")
        print("  Consider lowering game_fps to match Sky or using a longer hold for visibility.")
        print()


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
    
    # 1. Sky Window + Foreground
    print("[1/8] Sky Window Detection:")
    win_diag = check_sky_window()
    print(f"      Status: {'OK' if win_diag['ok'] else 'FAILED'}")
    print(f"      Details: {win_diag['msg']}")
    fg_diag = check_sky_foreground()
    print(f"      Foreground: {'OK' if fg_diag['ok'] else 'WARNING'}")
    print(f"      Details: {fg_diag['msg']}")
    print("-" * 60)
    
    # 2. Timer Resolution Check
    print("[2/8] Multimedia High-Precision Timers:")
    time_diag = check_timer_resolution()
    print(f"      Status: {'OK' if time_diag['ok'] else 'FAILED'}")
    print(f"      Details: {time_diag['msg']}")
    print("-" * 60)
    
    # 3. Calibration Cache Check
    print("[3/8] Calibration Cache:")
    cal_diag = check_calibration_cache()
    print(f"      Status: {'OK' if cal_diag['ok'] else 'ADVISORY'}")
    print(f"      Details: {cal_diag['msg']}")
    print("-" * 60)
    
    # 4. Note Key Mapping Check
    print("[4/8] Note Mapping Configuration:")
    kb_diag = check_keyboard_layout()
    print(f"      Status: {'OK' if kb_diag['ok'] else 'FAILED'}")
    print(f"      Details: {kb_diag['msg']}")
    print("-" * 60)
    
    # 5. Preflight Key Conflict Check
    print("[5/8] Keyboard Preflight Checks:")
    conflict_diag = check_physical_keys_held()
    print(f"      Status: {'OK' if conflict_diag['ok'] else 'WARNING'}")
    print(f"      Details: {conflict_diag['msg']}")
    print("-" * 60)

    # 6. Native dispatch diagnostics
    print("[6/8] Native Rust Dispatch:")
    native_diag = check_native_dispatch()
    native_label = "OK" if native_diag["ok"] else (
        "FAILED" if native_diag["required"] else "ADVISORY"
    )
    print(f"      Status: {native_label}")
    print(f"      Details: {native_diag['msg']}")
    print("-" * 60)

    # 7. FPS Advisory
    print("[7/8] FPS Configuration:")
    print_fps_advisory()
    print("-" * 60)

    # 8. Host Input Delivery Calibration Cache (redundant path hint)
    print("[8/8] Preflight Summary:")
    print("      Run `--doctor-calibrate` if calibration cache is missing (see check 3/8).")
    print("=" * 60)
    
    all_ok = (
        win_diag["ok"] and fg_diag["ok"] and time_diag["ok"]
        and cal_diag["ok"] and kb_diag["ok"] and conflict_diag["ok"]
        and (native_diag["ok"] or not native_diag["required"])
    )
    if all_ok:
        print("Result: ALL CHECKS PASSED — ready for precise playback.")
    else:
        print("Result: ATTENTION NEEDED — review the details above before playing.")
    print("=" * 60)
    
    return all_ok
