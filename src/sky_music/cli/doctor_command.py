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
        print("    SKY MUSIC PLAYER — HOST INPUT DELIVERY CALIBRATION")
        print("=" * 60)
        from sky_music.platform.win32 import window_target
        if window_target.get_sky_window() is not None:
            print("Error: Sky process is currently running.")
            print("Please close the game entirely before running input calibration.")
            print("=" * 60)
            return 1
        
        print("Creating calibration window. Please keep the window focused.")
        print("Injecting balanced Down/Up pairs and measuring the host-side total hold proxy...")
        print("This is SendInput target-to-receipt evidence, not game/audio onset truth.")
        try:
            from sky_music.infrastructure.calibration_loader import CalibrationStatus
            from sky_music.platform.win32.native_calibration import (
                run_published_native_calibration,
            )

            res = run_published_native_calibration()
            print(
                "Host total-hold proxy shrink p99 (us): "
                f"{res.global_shrink_p99_us} (worst bucket: {res.worst_bucket})"
            )
            print(f"Candidate hold correction (us): {res.candidate_margin_us}")
            print(f"Policy guard (us): {res.guard_us}")
            print(f"Trusted correction ceiling (us): {res.ceiling_us}")
            print(f"Evidence: {res.evidence_kind}")
            print(f"Cache: {res.cache_path.as_posix()}")
            if res.status is CalibrationStatus.VALID:
                assert res.margin_us is not None
                print("Host Input Delivery Calibration: VALID")
                print(f"Applied hold margin (us): {res.margin_us}")
                print(f"Materialized authored hold (us): {res.effective_min_hold_us}")
            else:
                print("Host Input Delivery Calibration: OUT OF ENVELOPE")
                print("Applied calibrated margin: NONE")
                print("Playback hold fallback: 500 us (not calibrated)")
                print("Calibration measurement completed, but host qualification failed.")
            print(f"Clean pairs per bucket: {res.sample_count}")
            print("Note-On timestamps: unchanged")
            print("=" * 60)
            return 0 if res.status is CalibrationStatus.VALID else 1
        except Exception as exc:
            print(f"Calibration failed: {exc}")
            print("=" * 60)
            return 1
        print("=" * 60)
    return 0
