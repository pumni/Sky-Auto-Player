from __future__ import annotations

import sky_music.infrastructure.doctor as doctor


def run_doctor_command(
    *,
    full: bool,
    timing: bool,
    input_check: bool,
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
    return 0
