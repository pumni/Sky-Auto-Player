import json
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from sky_music.config import resolve_game_fps


@dataclass(frozen=True, slots=True)
class CalibrationInput:
    profile_name: str
    tempo_scale: float
    fps: int
    p95_lateness_us: int
    p99_lateness_us: int
    p95_send_duration_us: int
    late_over_10ms: int
    impossible_same_key_repeats: int
    risky_same_key_repeats: int
    failed_release_count: int
    infeasible_same_key_repeats: int = 0
    compressed_holds: int = 0
    max_polyphony: int = 0
    note_count: int = 0

    def __post_init__(self) -> None:
        if self.infeasible_same_key_repeats == 0 and self.impossible_same_key_repeats > 0:
            object.__setattr__(
                self,
                "infeasible_same_key_repeats",
                self.impossible_same_key_repeats,
            )

@dataclass(frozen=True, slots=True)
class CalibrationRecommendation:
    profile_name: str
    tempo_scale: float
    hold_us: int
    reason: str
    severity: Literal["ok", "moderate", "severe"]


def _read_summary_file(path: Path) -> dict | None:
    try:
        with path.open("r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else None
    except Exception:
        return None


def load_latest_telemetry_summary(logs_dir: Path | str = Path("logs")) -> dict | None:
    """Load the newest telemetry companion summary from a logs directory."""
    path = Path(logs_dir)
    summaries = sorted(path.glob("*.summary.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not summaries:
        return None
    return _read_summary_file(summaries[0])


def load_telemetry_summary(target: Path | str | None = None) -> dict | None:
    """Load a specific summary file, or the latest summary from a directory/logs."""
    if target is None:
        return load_latest_telemetry_summary()
    path = Path(target)
    if path.is_dir():
        return load_latest_telemetry_summary(path)
    if path.suffix == ".csv":
        path = path.with_suffix(".summary.json")
    return _read_summary_file(path)

def calibrate_profile(inp: CalibrationInput) -> CalibrationRecommendation:
    """
    Analyzes high-precision telemetry loops and returns targeted calibration parameter proposals.
    """
    from sky_music.config import load_config
    from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy

    cfg = load_config()

    p99 = inp.p99_lateness_us
    late_10ms = inp.late_over_10ms
    
    schedule_stress = (
        inp.infeasible_same_key_repeats > 0
        or inp.risky_same_key_repeats > 5
        or inp.compressed_holds > 10
    )
    dense_polyphony = inp.max_polyphony > 8
    stress_rate = (
        (inp.infeasible_same_key_repeats + inp.risky_same_key_repeats) / inp.note_count
        if inp.note_count > 0
        else 0.0
    )

    # 2. Timing Profile and Tempo Scale calibration decision tree
    if inp.failed_release_count > 0 or inp.infeasible_same_key_repeats > 0 or p99 > 15000 or late_10ms > 5:
        severity = "severe"
        rec_profile = "audience-safe" if inp.fps <= 30 and not schedule_stress else "local-precise"
        rec_tempo = round(inp.tempo_scale * (0.88 if stress_rate > 0.03 else 0.90), 2)
        reason = (
            f"Severe timing or schedule stress detected "
            f"(p99={p99/1000:.1f}ms, late >10ms count={late_10ms}, "
            f"infeasible same-key repeats={inp.infeasible_same_key_repeats}). "
            "Recommend safe/dense playback and scaling down tempo."
        )
    elif p99 > 8000 or late_10ms > 0 or schedule_stress or dense_polyphony:
        severity = "moderate"
        rec_profile = "local-precise" if schedule_stress else ("audience-safe" if dense_polyphony else "balanced")
        rec_tempo = round(inp.tempo_scale * 0.95, 2)
        reason = (
            f"Moderate timing or density stress detected "
            f"(p99={p99/1000:.1f}ms, same-key compressed holds={inp.risky_same_key_repeats}, "
            f"compressed holds={inp.compressed_holds}, max polyphony={inp.max_polyphony}). "
            "Recommend a safer profile and slight tempo reduction."
        )
    elif p99 < 3000:
        severity = "ok"
        rec_profile = "local-precise"
        rec_tempo = inp.tempo_scale
        reason = f"Excellent timing performance (p99={p99/1000:.1f}ms). High precision profiles can be safely used."
    else:
        severity = "ok"
        rec_profile = inp.profile_name
        rec_tempo = inp.tempo_scale
        reason = "Good timing performance. Current parameters are well-calibrated."

    # 3. Hold duration via the same FrameTimingPolicy path as playback scheduling
    base = TimingPolicy.from_profile_name(rec_profile, cfg)
    effective = FrameTimingPolicy.from_timing_policy(
        base,
        fps=resolve_game_fps(inp.fps),
        profile_name=rec_profile,
        **cfg.frame_timing.as_policy_kwargs(),  # type: ignore[arg-type]
    )
    recommended_hold = effective.hold_us

    return CalibrationRecommendation(
        profile_name=rec_profile,
        tempo_scale=rec_tempo,
        hold_us=recommended_hold,
        reason=reason,
        severity=severity
    )


def calibration_input_from_summary(summary: dict) -> CalibrationInput:
    """Build CalibrationInput from a telemetry *.summary.json payload."""
    lat = summary.get("lateness_us", {})
    dur = summary.get("send_duration_us", {})
    backend = summary.get("backend", {})
    fps_val = resolve_game_fps(summary.get("fps"))
    sched = summary.get("schedule", {})

    return CalibrationInput(
        profile_name=str(summary.get("profile", "balanced")),
        tempo_scale=float(summary.get("tempo_scale", 1.0)),
        fps=fps_val,
        p95_lateness_us=int(lat.get("p95_us", 0)),
        p99_lateness_us=int(lat.get("p99_us", 0)),
        p95_send_duration_us=int(dur.get("p95_us", 0)),
        late_over_10ms=int(lat.get("over_10ms", 0)),
        impossible_same_key_repeats=int(sched.get("impossible_same_key_repeats", 0)),
        infeasible_same_key_repeats=int(
            sched.get("infeasible_same_key_repeats", sched.get("impossible_same_key_repeats", 0))
        ),
        risky_same_key_repeats=int(sched.get("risky_same_key_repeats", 0)),
        failed_release_count=int(backend.get("panic_release_failures", 0)),
        compressed_holds=int(sched.get("compressed_holds", 0)),
        max_polyphony=int(sched.get("max_polyphony", 0)),
        note_count=int(sched.get("note_count", summary.get("total_events", 0))),
    )
