from __future__ import annotations

import sky_music.infrastructure.doctor as doctor


def run_doctor_command(
    *,
    full: bool,
    timing: bool,
    input_check: bool,
    calibrate: bool = False,
    song_path: str | None = None,
) -> int:
    if full:
        doctor.run_all_doctor_checks()
    elif timing:
        print("=" * 60)
        print("         SKY MUSIC PLAYER — TIMING CHECK")
        print("=" * 60)
        diag = doctor.check_timer_resolution()
        print(f"Status: {'OK' if diag['ok'] else 'FAILED'}\nDetails: {diag['msg']}")
        print("=" * 60)
    elif input_check:
        print("=" * 60)
        print("         SKY MUSIC PLAYER — INPUT CHECK")
        print("=" * 60)
        kb_diag = doctor.check_keyboard_layout()
        conflict_diag = doctor.check_physical_keys_held()
        print(f"Layout Mapping : {'OK' if kb_diag['ok'] else 'FAILED'} - {kb_diag['msg']}")
        print(f"Key Conflicts  : {'OK' if conflict_diag['ok'] else 'WARNING'} - {conflict_diag['msg']}")
        print("=" * 60)
    elif calibrate:
        print("=" * 60)
        print("    SKY MUSIC PLAYER — INPUT DELIVERY LATENCY CALIBRATION")
        print("=" * 60)
        from sky_music.platform.win32 import inputs
        if inputs.get_sky_window() is not None:
            print("Error: Sky process is currently running.")
            print("Please close the game entirely before running input calibration.")
            print("=" * 60)
            return 1
        
        print("Creating calibration window. Please keep the window focused.")
        print("Injecting down/up keystrokes and measuring raw input delivery...")
        try:
            from sky_music.platform.win32.calibration import (
                calibrate_input_latency_harness,
            )
            res = calibrate_input_latency_harness()
            print("Calibration complete successfully!")
            print(f"Sampled Down Latency (us): p50={res['down_us']['p50']}, p90={res['down_us']['p90']}, p99={res['down_us']['p99']}")
            print(f"Sampled Up Latency   (us): p50={res['up_us']['p50']}, p90={res['up_us']['p90']}, p99={res['up_us']['p99']}")
            print("Calibration saved to .cache/input_latency.json")
        except Exception as exc:
            print(f"Calibration failed: {exc}")
            print("=" * 60)
            return 1
        print("=" * 60)
    return 0
