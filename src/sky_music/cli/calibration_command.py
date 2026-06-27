from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from sky_music.config import AppConfig, persist_calibration_defaults
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    apply_recommendation_to_context,
    resolve_game_fps,
)
from sky_music.orchestration.runtime_session import RuntimeSessionState


@dataclass(frozen=True, slots=True)
class CalibrationCommandResult:
    exit_code: int
    applied: bool = False

def run_auto_calibrate(summary_path: Path | str | None = None) -> int:
    ANSI_RESET  = "\033[0m"
    ANSI_BOLD   = "\033[1m"
    ANSI_CYAN   = "\033[36m"
    ANSI_YELLOW = "\033[33m"

    from sky_music.orchestration.calibration import (
        calibrate_profile,
        calibration_input_from_summary,
        load_telemetry_summary,
    )

    summary = load_telemetry_summary(summary_path)
    if summary is None:
        target = summary_path if summary_path is not None else "logs/"
        print(f"\n  {ANSI_YELLOW}No telemetry summary found at {target}.{ANSI_RESET}")
        print("  Run a playback with --debug-csv first to generate telemetry.")
        return 1

    print()
    label = str(summary_path) if summary_path is not None else "latest telemetry"
    print(f"  {ANSI_BOLD}{ANSI_CYAN}Auto-Calibrate — analysing: {label}{ANSI_RESET}")
    print()

    inp = calibration_input_from_summary(summary)
    rec = calibrate_profile(inp)
    lat = summary.get("lateness_us", {})
    dur = summary.get("send_duration_us", {})
    print(f"  Song          : {summary.get('song', 'unknown')}")
    print(f"  Profile used  : {inp.profile_name}")
    print(f"  FPS           : {inp.fps}")
    print(f"  p95 lateness  : {lat.get('p95_us', 0) / 1000:.1f} ms")
    print(f"  p99 lateness  : {lat.get('p99_us', 0) / 1000:.1f} ms")
    print(f"  p95 send      : {dur.get('p95_us', 0) / 1000:.1f} ms")
    print()
    print("  Calibration Recommendation:")
    print(f"    Suggested Profile : {rec.profile_name}")
    print(f"    Suggested Tempo   : {rec.tempo_scale:.2f}x")
    print(f"    Hold Target       : {rec.hold_us / 1000:.1f} ms")
    print(f"    Severity          : {rec.severity.upper()}")
    print(f"    Reason            : {rec.reason}")
    print()
    print()
    return 0


def apply_calibration_from_telemetry(
    cfg: AppConfig,
    runtime_state: RuntimeSessionState,
    *,
    persist: bool = False,
    summary_path: Path | str | None = None,
) -> CalibrationCommandResult:
    ANSI_RESET  = "\033[0m"
    ANSI_BOLD   = "\033[1m"
    ANSI_CYAN   = "\033[36m"
    ANSI_YELLOW = "\033[33m"
    ANSI_GREEN  = "\033[32m"
    ANSI_DIM    = "\033[2m"

    from sky_music.orchestration.calibration import (
        calibrate_profile,
        calibration_input_from_summary,
        load_telemetry_summary,
    )

    summary = load_telemetry_summary(summary_path)
    if summary is None:
        target = summary_path if summary_path is not None else "logs/"
        print(f"\n  {ANSI_YELLOW}No telemetry summary found at {target}.{ANSI_RESET}")
        print("  Run a playback with --debug-csv first to generate telemetry.")
        print()
        return CalibrationCommandResult(exit_code=1, applied=False)

    inp = calibration_input_from_summary(summary)
    rec = calibrate_profile(inp)
    base = runtime_state.session or PlaybackSessionContext.balanced(
        tempo_scale=cfg.default_tempo_scale,
        fps=resolve_game_fps(cfg.game_fps),
    )
    updated = apply_recommendation_to_context(base, rec)
    updated = updated.with_fps(resolve_game_fps(inp.fps))
    
    runtime_state.apply_session(updated, cfg)
    
    print()
    print(f"  {ANSI_BOLD}{ANSI_CYAN}Applied calibration to session{ANSI_RESET}")
    print(f"    Profile     : {rec.profile_name}")
    print(f"    Tempo scale : {rec.tempo_scale:.2f}x")
    print(f"    Hold target : {rec.hold_us / 1000:.1f} ms ({ANSI_DIM}via FrameTimingPolicy{ANSI_RESET})")
    print(f"    Severity    : {rec.severity.upper()}")
    print(f"    Reason      : {rec.reason}")
    if persist:
        persist_calibration_defaults(
            cfg,
            profile_name=rec.profile_name,
            tempo_scale=rec.tempo_scale,
            fps=inp.fps,
        )
        print(f"  {ANSI_GREEN}Saved calibration defaults to config.json.{ANSI_RESET}")
    else:
        print(f"  {ANSI_GREEN}In-memory only — config.json not modified.{ANSI_RESET}")
    print()
    return CalibrationCommandResult(exit_code=0, applied=True)
