import json
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from sky_music.config import resolve_game_fps
from sky_music.domain.hold_timing import normalize_hold_frames
from sky_music.domain.scheduler_types import FrameTimingPolicy


@dataclass(frozen=True, slots=True)
class CalibrationInput:
    hold_frames: float
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
        object.__setattr__(self, "hold_frames", normalize_hold_frames(self.hold_frames))
        if self.infeasible_same_key_repeats == 0 and self.impossible_same_key_repeats > 0:
            object.__setattr__(self, "infeasible_same_key_repeats", self.impossible_same_key_repeats)


@dataclass(frozen=True, slots=True)
class CalibrationRecommendation:
    hold_frames: float
    tempo_scale: float
    recommended_hold_us: int
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
    path = Path(logs_dir)
    summaries = sorted(path.glob("*.summary.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    return _read_summary_file(summaries[0]) if summaries else None


def load_telemetry_summary(target: Path | str | None = None) -> dict | None:
    if target is None:
        return load_latest_telemetry_summary()
    path = Path(target)
    if path.is_dir():
        return load_latest_telemetry_summary(path)
    if path.suffix == ".csv":
        path = path.with_suffix(".summary.json")
    return _read_summary_file(path)


def calibrate_timing(inp: CalibrationInput) -> CalibrationRecommendation:
    stress_repeats = inp.risky_same_key_repeats > 5 or inp.compressed_holds > 10
    timing_failure = inp.failed_release_count > 0 or inp.p99_lateness_us > 15_000 or inp.late_over_10ms > 5
    if inp.infeasible_same_key_repeats > 0 and not timing_failure:
        hold = 1.0
        tempo = round(min(inp.tempo_scale, 0.90), 2)
        severity = "moderate"
        reason = "Infeasible repeat stress detected; use the shortest hold and reduce tempo."
    elif timing_failure:
        hold = 1.5
        tempo = round(min(inp.tempo_scale, 0.90), 2)
        severity = "severe"
        reason = "Delivery degradation or completion lateness detected; use a longer hold and reduce tempo."
    elif stress_repeats:
        hold = 1.0
        tempo = round(min(inp.tempo_scale, 0.95), 2)
        severity = "moderate"
        reason = "Repeat/compression stress detected; use a shorter hold."
    elif inp.p99_lateness_us > 8_000 or inp.late_over_10ms > 0:
        hold = 1.25
        tempo = round(min(inp.tempo_scale, 0.95), 2)
        severity = "moderate"
        reason = "Moderate timing stress detected; use a visibility cushion and reduce tempo."
    elif inp.max_polyphony >= 5:
        hold = 1.5
        tempo = inp.tempo_scale
        severity = "moderate"
        reason = "Dense polyphony detected without repeat stress; use a longer hold for visibility."
    else:
        hold = inp.hold_frames
        tempo = inp.tempo_scale
        severity = "ok"
        reason = "Timing is good; retain the selected hold."

    effective = FrameTimingPolicy.from_hold_frames(hold, resolve_game_fps(inp.fps))
    return CalibrationRecommendation(hold, tempo, int(effective.hold_us), reason, severity)


def calibration_input_from_summary(summary: dict) -> CalibrationInput:
    lat = summary.get("lateness_us", {})
    dur = summary.get("send_duration_us", {})
    backend = summary.get("backend", {})
    sched = summary.get("schedule", {})
    legacy = str(summary.get("profile", "balanced")).lower().replace("-", "_")
    legacy_hold = 1.5 if legacy in {"audience_safe", "remote_safe", "online_audible_safe", "online_audible"} else 1.0
    return CalibrationInput(
        hold_frames=normalize_hold_frames(summary.get("hold_frames", legacy_hold)),
        tempo_scale=float(summary.get("tempo_scale", 1.0)),
        fps=resolve_game_fps(summary.get("fps")),
        p95_lateness_us=int(lat.get("p95_us", 0)),
        p99_lateness_us=int(lat.get("p99_us", 0)),
        p95_send_duration_us=int(dur.get("p95_us", 0)),
        late_over_10ms=int(lat.get("over_10ms", 0)),
        impossible_same_key_repeats=int(sched.get("impossible_same_key_repeats", 0)),
        infeasible_same_key_repeats=int(sched.get("infeasible_same_key_repeats", sched.get("impossible_same_key_repeats", 0))),
        risky_same_key_repeats=int(sched.get("risky_same_key_repeats", 0)),
        failed_release_count=int(backend.get("panic_release_failures", 0)),
        compressed_holds=int(sched.get("compressed_holds", 0)),
        max_polyphony=int(sched.get("max_polyphony", 0)),
        note_count=int(sched.get("note_count", summary.get("total_events", 0))),
    )
