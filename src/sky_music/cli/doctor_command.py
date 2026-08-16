from __future__ import annotations

import sky_music.infrastructure.doctor as doctor


def run_doctor_command(
    *,
    full: bool,
    timing: bool,
    input_check: bool,
    calibrate: bool = False,
    song_path: str | None = None,  # noqa: ARG001
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
        from sky_music.platform.win32 import window_target
        if window_target.get_sky_window() is not None:
            print("Error: Sky process is currently running.")
            print("Please close the game entirely before running input calibration.")
            print("=" * 60)
            return 1
        
        print("Creating calibration window. Please keep the window focused.")
        print("Injecting balanced Down/Up pairs and measuring host-side Raw Input delivery...")
        print("This is a SendInput -> app-owned WM_INPUT delivery proxy, not game/audio onset truth.")
        try:
            from sky_music.platform.win32.native_calibration import (
                run_published_native_calibration,
            )

            res = run_published_native_calibration()
            print("Calibration complete successfully!")
            print(
                "Host hold-shrink p99 (us): "
                f"{res.global_shrink_p99_us} (worst bucket: {res.worst_bucket})"
            )
            print(f"Recommended margin (us): {res.margin_us}")
            print(f"Clean pairs per bucket: {res.sample_count}")
            print("Calibration saved to .cache/input_latency.json")
        except Exception as exc:
            print(f"Calibration failed: {exc}")
            print("=" * 60)
            return 1
        print("=" * 60)
    return 0
