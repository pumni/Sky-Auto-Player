"""Bounded Windows acceptance benchmark for the native dispatch worker.

The default benchmark uses the Rust worker with an explicit, polyphony-aware
mock latency model. It never emits input to a game or another process. This
isolates estimator behaviour while still exercising non-zero completion
latency. ``--backend sendinput --allow-real-input`` is an explicit fixed-host
run against the real SendInput seam; it must not be used while an unintended
foreground application can receive the test keys.

Run the same command on the baseline and follow-up revisions, then compare the
JSON files::

    uv run --env-file .env python scripts/bench_native_acceptance.py \
        --repeats 2 --actions 128 --output native-followup.json

The completion-error percentiles are a host-side proxy. They do not measure
when a game samples Windows input, renders a frame, or produces audio.
"""

from __future__ import annotations

import argparse
import collections
import json
import math
import os
import platform
import subprocess
import time
from pathlib import Path
from typing import Any

from sky_music.layouts import SKY_15_SCAN_CODES
from sky_music.orchestration.telemetry import (
    TelemetryRecord,
    materialize_native_trace,
)

REPOSITORY_ROOT = Path(__file__).resolve().parents[1]


def _actions(count: int, polyphony: int) -> list[tuple[int, str, int, list[int], str]]:
    if not 1 <= polyphony <= len(SKY_15_SCAN_CODES):
        raise ValueError(f"polyphony must be in 1..{len(SKY_15_SCAN_CODES)}")
    actions: list[tuple[int, str, int, list[int], str]] = []
    for index in range(count):
        scan_codes = [
            int(SKY_15_SCAN_CODES[(index * polyphony + offset) % len(SKY_15_SCAN_CODES)])
            for offset in range(polyphony)
        ]
        # Leave a generous multi-millisecond off-gap between cycles. The benchmark is
        # a clean chord-integrity gate; sub-millisecond same-key feasibility
        # belongs to the dedicated conflict/recovery tests, not this sender
        # timing baseline.
        at_us = index * 10_000
        actions.append((index * 2, "down", at_us, scan_codes, "bench-down"))
        actions.append((index * 2 + 1, "up", at_us + 5_000, scan_codes, "bench-up"))
    return actions


def _percentile(values: list[int], fraction: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * fraction) - 1)
    return ordered[min(index, len(ordered) - 1)]


def _stats(values: list[int]) -> dict[str, int]:
    return {
        "n": len(values),
        "p50": _percentile(values, 0.50),
        "p95": _percentile(values, 0.95),
        "p99": _percentile(values, 0.99),
        "max": max(values, default=0),
    }


def _required_stats(values: list[int], name: str) -> dict[str, int]:
    if not values:
        raise RuntimeError(f"required metric {name} has no measurements")
    return _stats(values)


class TelemetryIntegrityError(RuntimeError):
    """Raised when a benchmark trace cannot prove one record per authored action."""

    def __init__(self, diagnostics: dict[str, Any]) -> None:
        self.diagnostics = diagnostics
        super().__init__(
            "native telemetry integrity failure: "
            + json.dumps(diagnostics, sort_keys=True)
        )


def _validate_telemetry_integrity(
    *,
    actions: list[tuple[int, str, int, list[int], str]],
    telemetry: dict[str, Any],
    records: list[TelemetryRecord],
    polyphony: int,
) -> dict[str, Any]:
    """Prove that the compact native trace is a complete authored action set.

    A matching record count is insufficient: a duplicate event index can hide a
    missing action.  This validation deliberately checks the exact authored
    index and kind mapping before a repetition is eligible for statistics.
    """

    expected_indices = [int(action[0]) for action in actions]
    expected_kinds = {int(action[0]): str(action[1]) for action in actions}
    actual_indices = [int(record.event_index) for record in records]
    counts = collections.Counter(actual_indices)
    duplicate_indices = sorted(index for index, count in counts.items() if count > 1)
    expected_set = set(expected_indices)
    actual_set = set(actual_indices)
    missing_indices = sorted(expected_set - actual_set)
    unexpected_indices = sorted(actual_set - expected_set)
    kind_mismatches = [
        {
            "event_index": record.event_index,
            "expected": expected_kinds[record.event_index],
            "actual": record.kind,
        }
        for record in records
        if record.event_index in expected_kinds
        and record.kind != expected_kinds[record.event_index]
    ]
    diagnostics: dict[str, Any] = {
        "polyphony": polyphony,
        "expected_count": len(actions),
        "records_count": len(records),
        "attempted": telemetry.get("attempted"),
        "accepted": telemetry.get("accepted"),
        "dropped": telemetry.get("dropped"),
        "truncated": telemetry.get("truncated"),
        "expected_indices": expected_indices,
        "actual_indices": actual_indices,
        "missing_indices": missing_indices,
        "duplicate_indices": duplicate_indices,
        "unexpected_indices": unexpected_indices,
        "kind_mismatches": kind_mismatches,
    }
    valid = (
        diagnostics["attempted"] == len(actions)
        and diagnostics["accepted"] == len(actions)
        and diagnostics["dropped"] == 0
        and diagnostics["truncated"] is False
        and len(records) == len(actions)
        and not missing_indices
        and not duplicate_indices
        and not unexpected_indices
        and not kind_mismatches
    )
    if not valid:
        raise TelemetryIntegrityError(diagnostics)
    return diagnostics


def _failed_run_artifact_path(output: Path | None, run_index: int) -> Path:
    """Choose a unique diagnostic path without overwriting an earlier failure."""

    stem = (output or (REPOSITORY_ROOT / "native-acceptance.json")).resolve()
    candidate = stem.with_name(f"{stem.stem}-failed-run-{run_index}.json")
    attempt = 1
    while candidate.exists():
        attempt += 1
        candidate = stem.with_name(
            f"{stem.stem}-failed-run-{run_index}-attempt-{attempt}.json"
        )
    return candidate


def _write_failed_run_artifact(
    path: Path,
    *,
    git_info: dict[str, Any],
    native_info: dict[str, Any],
    host_info: dict[str, Any],
    run_index: int,
    polyphony: int,
    actions: list[tuple[int, str, int, list[int], str]],
    snapshot: dict[str, Any] | None,
    telemetry: dict[str, Any] | None,
    diagnostics: dict[str, Any] | None,
    exception: BaseException,
) -> Path:
    """Persist every available failed-run diagnostic and never replace a file."""

    if path.exists():
        raise FileExistsError(f"refusing to overwrite failed-run artifact {path}")
    payload = {
        "git_provenance": git_info,
        "native_provenance": native_info,
        "host_fingerprint": host_info,
        "run_index": run_index,
        "polyphony": polyphony,
        "actions": actions,
        "session_snapshot": snapshot,
        "raw_telemetry": telemetry,
        "validation_diagnostics": diagnostics,
        "exception": f"{type(exception).__name__}: {exception}",
        "command_line": list(os.sys.argv),
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, default=str) + "\n", encoding="utf-8")
    return path


def _git_provenance() -> dict[str, Any]:
    try:
        sha_result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        status_result = subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"could not read Git provenance: {exc}") from exc
    sha = sha_result.stdout.strip()
    if not sha:
        raise RuntimeError("Git HEAD is empty")
    dirty = bool(status_result.stdout.strip())
    if dirty:
        raise RuntimeError("acceptance evidence requires a clean worktree")
    return {"git_sha": sha, "dirty_worktree": False}


def _native_provenance(expected_commit: str) -> dict[str, Any]:
    import sky_player_rs

    info = dict(sky_player_rs.build_info())
    required = (
        "native_build_commit",
        "rustc_version",
        "schema_version",
        "native_abi",
        "qpc_frequency_hz",
    )
    for name in required:
        value = info.get(name)
        if value in (None, "", "unknown"):
            raise RuntimeError(f"native build provenance is missing {name}")
    if info["native_build_commit"] != expected_commit:
        raise RuntimeError(
            "native build commit does not match the current checkout: "
            f"native={info['native_build_commit']} expected={expected_commit}"
        )
    return info


def _host_fingerprint(native_info: dict[str, Any]) -> dict[str, Any]:
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "windows_build": platform.version(),
        "qpc_frequency_hz": int(native_info["qpc_frequency_hz"]),
    }


def _completion_error_report(records: list[TelemetryRecord]) -> dict[str, Any]:
    """Return signed, absolute, early and late error distributions.

    Signed aggregate percentiles are retained for report compatibility, but
    regression gates must inspect all three non-negative views.  The per-kind
    split prevents a Down-only or Up-only regression from being diluted by the
    other half of the timeline.
    """

    def values_for(rows: list[dict[str, Any]]) -> list[int]:
        values: list[int] = []
        for record in rows:
            value = record.sender_completion_error_us
            if not isinstance(value, int) or isinstance(value, bool):
                raise RuntimeError(
                    "sender telemetry is missing exact sender_completion_error_us"
                )
            values.append(value)
        return values

    if not records:
        raise RuntimeError("required sender telemetry has no records")

    def report_for(rows: list[dict[str, Any]], name: str) -> dict[str, Any]:
        signed = values_for(rows)
        return {
            "signed": _required_stats(signed, f"{name}.signed"),
            "absolute": _required_stats([abs(value) for value in signed], f"{name}.absolute"),
            "late": _required_stats([max(value, 0) for value in signed], f"{name}.late"),
            "early": _required_stats([max(-value, 0) for value in signed], f"{name}.early"),
        }

    by_kind = {
        kind: report_for([record for record in records if record.kind == kind], kind)
        for kind in ("down", "up")
    }
    result = report_for(records, "all")
    result["by_kind"] = by_kind
    return result


def _peak_working_set_bytes() -> int | None:
    """Read this benchmark process's peak working set without Python Win32 ctypes."""
    if os.name != "nt":
        return None
    process_id = str(os.getpid())
    command = f"(Get-Process -Id {process_id}).PeakWorkingSet64"
    for executable in ("powershell", "pwsh"):
        try:
            result = subprocess.run(
                [executable, "-NoProfile", "-NonInteractive", "-Command", command],
                capture_output=True,
                text=True,
                check=True,
                timeout=5,
            )
        except (OSError, subprocess.SubprocessError):
            continue
        try:
            return int(result.stdout.strip().splitlines()[-1])
        except (IndexError, ValueError):
            return None
    return None


def _new_session(
    actions: list[tuple[int, str, int, list[int], str]],
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
) -> Any:
    import sky_player_rs

    if backend == "mock":
        test_session = getattr(sky_player_rs, "TestDispatchSession", None)
        if not callable(test_session):
            raise RuntimeError(
                "mock acceptance requires a test-support native wheel; "
                "production wheels expose only DispatchSession"
            )
        return test_session(
            actions,
            list(SKY_15_SCAN_CODES),
            min_hold_us=100,
            mock_latency_base_us=mock_base_latency_us,
            mock_latency_per_key_us=mock_per_key_latency_us,
            telemetry_capacity=min(1_024, max(1, len(actions))),
            rt_priority_mode=rt_priority_mode,
            enable_waitable_timer=True,
            enable_event_wait=True,
            enable_adaptive_spin=adaptive_spin,
            enable_adaptive_lead=True,
        )
    return sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        actions,
        list(SKY_15_SCAN_CODES),
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            min_hold_us=100,
            require_focus=False,
            telemetry=True,
            profile="production",
        ),
    )


def _run_dispatch(
    actions: list[tuple[int, str, int, list[int], str]],
    polyphony: int,
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
    timeout_ms: int = 60_000,
) -> dict[str, Any]:
    session = _new_session(
        actions,
        backend=backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=adaptive_spin,
        rt_priority_mode=rt_priority_mode,
    )
    started_ns = time.perf_counter_ns()
    session.start()
    if not session.join(timeout_ms=timeout_ms):
        session.panic_release()
        session.quit()
        session.join(timeout_ms=5_000)
        raise RuntimeError("native acceptance session exceeded its hard budget")
    wall_us = (time.perf_counter_ns() - started_ns) // 1_000
    snapshot = dict(session.snapshot())
    telemetry = json.loads(session.take_telemetry_json())
    records = materialize_native_trace(telemetry)
    _validate_telemetry_integrity(
        actions=actions,
        telemetry=telemetry,
        records=records,
        polyphony=polyphony,
    )
    sender_errors = [
        record.sender_completion_error_us
        for record in records
        if isinstance(record.sender_completion_error_us, int)
        and not isinstance(record.sender_completion_error_us, bool)
    ]
    if len(sender_errors) != len(records):
        raise RuntimeError("required sender telemetry has missing exact completion errors")
    lead_by_polyphony = {
        str(record.native_polyphony): int(record.applied_lead_us)
        for record in records
        if record.kind == "down" and record.native_polyphony is not None
    }
    peak_rss = _peak_working_set_bytes()
    result: dict[str, Any] = {
        "polyphony": polyphony,
        "wall_us": wall_us,
        "_sender_error_values": sender_errors,
        "_records": records,
        "sender_completion_error_us": _required_stats(sender_errors, "sender_completion_error_us"),
        "completion_error_us": _completion_error_report(records),
        "spin_cpu_time_us": int(snapshot.get("spin_time_us", 0)),
        "worker_cpu_time_us": int(snapshot.get("worker_cpu_time_us", 0)),
        "process_cpu_time_us": int(snapshot.get("process_cpu_time_us", 0)),
        "playback_wall_time_us": int(snapshot.get("playback_wall_time_us", 0)),
        "spin_duty_cycle_ppm": int(snapshot.get("spin_duty_cycle_ppm", 0)),
        "peak_rss_bytes": peak_rss,
        "keys_dropped": int(snapshot.get("keys_dropped", 0)),
        "failed_release_count": int(snapshot.get("failed_release_count", 0)),
        "chord_split_events": int(snapshot.get("chord_split_events", 0)),
        "sendinput_partial_events": int(snapshot.get("sendinput_partial_events", 0)),
        "sendinput_zero_progress_failures": int(
            snapshot.get("sendinput_zero_progress_failures", 0)
        ),
        "lead_saturation_count_down": list(
            snapshot.get("lead_saturation_count_down", [])
        ),
        "lead_saturation_count_up": list(snapshot.get("lead_saturation_count_up", [])),
        "positive_residual_at_cap": int(snapshot.get("positive_residual_at_cap", 0)),
        "lead_by_polyphony": lead_by_polyphony,
        "generation_status_counts": dict(snapshot.get("generation_status_counts", {})),
        "outcome": snapshot.get("outcome"),
    }
    return result


def _measure_command_interrupt(
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
) -> int:
    # The deadline is intentionally far away; the only expected wake is the
    # command event. No input can be emitted before the pause is observed.
    actions = [(0, "down", 10_000_000, [int(SKY_15_SCAN_CODES[0])], "interrupt")]
    session = _new_session(
        actions,
        backend=backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=adaptive_spin,
        rt_priority_mode=rt_priority_mode,
    )
    session.start()
    deadline = time.perf_counter() + 2.0
    while not bool(dict(session.snapshot()).get("is_running")):
        if time.perf_counter() >= deadline:
            raise RuntimeError("native worker did not enter running state")
        time.sleep(0.001)

    # The lifecycle flag is published before the worker finishes its bounded
    # wake-probe/admission setup. Do not charge that startup work to the
    # command interrupt measurement.
    while dict(session.snapshot()).get("rt_priority_acquired") == "pending":
        if time.perf_counter() >= deadline:
            raise RuntimeError("native worker did not finish startup admission")
        time.sleep(0.001)

    started_ns = time.perf_counter_ns()
    session.pause()
    while not bool(dict(session.snapshot()).get("is_paused")):
        if time.perf_counter() >= deadline + 2.0:
            session.quit()
            session.join(timeout_ms=5_000)
            raise RuntimeError("native pause command was not observed")
        time.sleep(0.001)
    elapsed_us = (time.perf_counter_ns() - started_ns) // 1_000
    session.quit()
    if not session.join(timeout_ms=5_000):
        raise RuntimeError("native command-interrupt session did not terminate")
    return elapsed_us


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--actions", type=int, default=128)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument(
        "--budget-seconds",
        type=float,
        default=120.0,
        help="hard whole-command budget in seconds (1..120; default: 120)",
    )
    parser.add_argument("--label", default="native")
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--polyphony",
        default="1,2,3,5,8,15",
        help="comma-separated chord sizes to exercise (default: 1,2,3,5,8,15)",
    )
    parser.add_argument(
        "--backend",
        choices=("mock", "sendinput"),
        default="mock",
        help="mock = injected polyphony-aware latency (default), sendinput = real fixed-host seam",
    )
    parser.add_argument(
        "--allow-real-input",
        action="store_true",
        help="required with --backend sendinput; keys may reach the foreground window",
    )
    parser.add_argument(
        "--mock-base-latency-us",
        type=int,
        default=None,
        help="mock backend base latency in microseconds (default: 80; mock only)",
    )
    parser.add_argument(
        "--mock-per-key-latency-us",
        type=int,
        default=None,
        help="mock backend per-key latency in microseconds (default: 40; mock only)",
    )
    parser.add_argument(
        "--no-adaptive-spin",
        action="store_true",
        help="disable adaptive wait probing; adaptive lead remains enabled",
    )
    parser.add_argument(
        "--rt-priority-mode",
        choices=("auto", "mmcss", "time_critical", "highest", "off"),
        default="off",
        help="real-time priority policy (default: off)",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        help="optional JSON report with sender-side regression thresholds",
    )
    return parser.parse_args()


def _parse_polyphony(raw: str) -> list[int]:
    try:
        values = [int(part.strip()) for part in raw.split(",") if part.strip()]
    except ValueError as exc:
        raise SystemExit("--polyphony must contain comma-separated integers") from exc
    if not values or len(set(values)) != len(values):
        raise SystemExit("--polyphony must contain at least one unique value")
    if any(value < 1 or value > len(SKY_15_SCAN_CODES) for value in values):
        raise SystemExit(f"--polyphony values must be in 1..{len(SKY_15_SCAN_CODES)}")
    return values


def _resolve_mock_latency_values(
    *,
    backend: str,
    mock_base_latency_us: int | None,
    mock_per_key_latency_us: int | None,
) -> tuple[int, int]:
    if backend == "sendinput" and (
        mock_base_latency_us is not None or mock_per_key_latency_us is not None
    ):
        raise SystemExit("mock latency values are only valid with --backend mock")

    base_latency_us = (
        80 if mock_base_latency_us is None else mock_base_latency_us
    )
    per_key_latency_us = (
        40 if mock_per_key_latency_us is None else mock_per_key_latency_us
    )
    if base_latency_us < 0 or per_key_latency_us < 0:
        raise SystemExit("mock latency values must be non-negative")
    if backend == "sendinput":
        return 0, 0
    return base_latency_us, per_key_latency_us


def _assert_correctness(run: dict[str, Any]) -> None:
    if run["outcome"] != "finished":
        raise SystemExit(f"native acceptance outcome was {run['outcome']!r}")
    if run["keys_dropped"] or run["failed_release_count"] or run["chord_split_events"]:
        raise SystemExit(
            "native acceptance correctness failure: "
            f"polyphony={run['polyphony']} "
            f"keys_dropped={run['keys_dropped']} "
            f"failed_release_count={run['failed_release_count']} "
            f"chord_split_events={run['chord_split_events']}"
        )
    statuses = run["generation_status_counts"]
    nonterminal = sum(int(statuses.get(name, 0)) for name in ("scheduled", "active", "release_pending"))
    if nonterminal:
        raise SystemExit(
            f"native acceptance left {nonterminal} nonterminal generations "
            f"for polyphony={run['polyphony']}"
        )


def _assert_baseline(report: dict[str, Any], baseline_path: Path) -> None:
    try:
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"cannot read benchmark baseline {baseline_path}: {exc}") from exc

    checks = [
        ("sender_completion_error_us", "p95", 1.25),
        ("sender_completion_error_us", "p99", 1.25),
        ("command_interrupt_latency_us", "p95", 1.50),
        ("peak_rss_bytes", "max", 1.25),
    ]
    for section, field, ratio in checks:
        observed = float(report[section][field])
        expected = float(baseline.get(section, {}).get(field, 0))
        if expected <= 0:
            continue
        if observed > expected * ratio:
            raise SystemExit(
                f"native benchmark regression in {section}.{field}: "
                f"observed={observed:g}, baseline={expected:g}, allowed={expected * ratio:g}"
            )

    for polyphony, observed in report.get("by_polyphony", {}).items():
        expected_poly = baseline.get("by_polyphony", {}).get(polyphony) or baseline.get(
            "by_polyphony", {}
        ).get("default", {})
        expected_errors = expected_poly.get("completion_error_us", {})
        observed_errors = observed.get("completion_error_us", {})
        for dimension in ("absolute", "late", "early"):
            for field in ("p95", "p99"):
                expected = float(expected_errors.get(dimension, {}).get(field, 0))
                observed_value = float(observed_errors.get(dimension, {}).get(field, 0))
                if expected <= 0:
                    continue
                if observed_value > expected * 1.25:
                    raise SystemExit(
                        "native benchmark regression in "
                        f"by_polyphony.{polyphony}.completion_error_us.{dimension}.{field}: "
                        f"observed={observed_value:g}, baseline={expected:g}, "
                        f"allowed={expected * 1.25:g}"
                    )
            for kind in ("down", "up"):
                expected_kind = expected_errors.get("by_kind", {}).get(kind, {})
                observed_kind = observed_errors.get("by_kind", {}).get(kind, {})
                for dimension in ("absolute", "late", "early"):
                    for field in ("p95", "p99"):
                        expected = float(expected_kind.get(dimension, {}).get(field, 0))
                        observed_value = float(observed_kind.get(dimension, {}).get(field, 0))
                        if expected <= 0:
                            continue
                        if observed_value > expected * 1.25:
                            raise SystemExit(
                                "native benchmark regression in "
                                f"by_polyphony.{polyphony}.completion_error_us.by_kind."
                                f"{kind}.{dimension}.{field}: observed={observed_value:g}, "
                                f"baseline={expected:g}, allowed={expected * 1.25:g}"
                            )


def main() -> int:
    args = _parse_args()
    if os.name != "nt":
        raise SystemExit("this acceptance benchmark requires Windows")
    if args.actions <= 0 or args.repeats <= 0:
        raise SystemExit("--actions and --repeats must be positive")
    if (
        isinstance(args.budget_seconds, bool)
        or not math.isfinite(args.budget_seconds)
        or not 1.0 <= args.budget_seconds <= 120.0
    ):
        raise SystemExit("--budget-seconds must be between 1 and 120 seconds")
    if args.backend == "sendinput" and not args.allow_real_input:
        raise SystemExit("--backend sendinput requires --allow-real-input")
    mock_base_latency_us, mock_per_key_latency_us = _resolve_mock_latency_values(
        backend=args.backend,
        mock_base_latency_us=args.mock_base_latency_us,
        mock_per_key_latency_us=args.mock_per_key_latency_us,
    )

    git_info = _git_provenance()
    native_info = _native_provenance(git_info["git_sha"])
    if native_info["native_build_commit"] != git_info["git_sha"]:
        raise RuntimeError(
            "native build provenance does not match Git HEAD: "
            f"native={native_info['native_build_commit']} git={git_info['git_sha']}"
        )
    host_info = _host_fingerprint(native_info)

    polyphonies = _parse_polyphony(args.polyphony)
    run_deadline = time.monotonic() + args.budget_seconds

    def next_timeout_ms() -> int:
        remaining = run_deadline - time.monotonic() - 5.0
        if remaining <= 0:
            raise RuntimeError("native acceptance budget expired before cleanup reserve")
        return max(1_000, min(60_000, math.ceil(remaining * 1_000)))

    dispatch_runs: list[dict[str, Any]] = []
    by_polyphony: dict[str, Any] = {}
    for polyphony in polyphonies:
        actions = _actions(args.actions, polyphony)
        runs = [
            _run_dispatch(
                actions,
                polyphony,
                backend=args.backend,
                mock_base_latency_us=mock_base_latency_us,
                mock_per_key_latency_us=mock_per_key_latency_us,
                adaptive_spin=not args.no_adaptive_spin,
                rt_priority_mode=args.rt_priority_mode,
                timeout_ms=next_timeout_ms(),
            )
            for _ in range(args.repeats)
        ]
        for run in runs:
            _assert_correctness(run)
        values = [value for run in runs for value in run["_sender_error_values"]]
        poly_report = {
            "polyphony": polyphony,
            "actions": len(actions),
            "sender_completion_error_us": {
                key: _stats(values)[key] for key in ("p50", "p95", "p99", "max")
            },
            "completion_error_us": _completion_error_report(
                [record for run in runs for record in run["_records"]]
            ),
            "spin_cpu_time_us": _stats([run["spin_cpu_time_us"] for run in runs]),
            "worker_cpu_time_us": _stats([run["worker_cpu_time_us"] for run in runs]),
            "process_cpu_time_us": _stats([run["process_cpu_time_us"] for run in runs]),
            "playback_wall_time_us": _stats([run["playback_wall_time_us"] for run in runs]),
            "spin_duty_cycle_ppm": _stats([run["spin_duty_cycle_ppm"] for run in runs]),
            "peak_rss_bytes": _required_stats(
                [run["peak_rss_bytes"] for run in runs if run["peak_rss_bytes"] is not None],
                "peak_rss_bytes",
            ),
            "keys_dropped": sum(run["keys_dropped"] for run in runs),
            "failed_release_count": sum(run["failed_release_count"] for run in runs),
            "chord_split_events": sum(run["chord_split_events"] for run in runs),
            "sendinput_partial_events": sum(
                run["sendinput_partial_events"] for run in runs
            ),
            "sendinput_zero_progress_failures": sum(
                run["sendinput_zero_progress_failures"] for run in runs
            ),
            "positive_residual_at_cap": sum(
                run["positive_residual_at_cap"] for run in runs
            ),
            "lead_by_polyphony": {
                key: max(
                    int(run["lead_by_polyphony"].get(key, 0)) for run in runs
                )
                for key in {str(polyphony)}
            },
            "outcomes": sorted({run["outcome"] for run in runs}),
        }
        by_polyphony[str(polyphony)] = poly_report
        dispatch_runs.extend(runs)
    interrupt_runs = []
    for _ in range(args.repeats):
        if run_deadline - time.monotonic() <= 5.0:
            raise RuntimeError("native acceptance budget expired before interrupt checks")
        interrupt_runs.append(
            _measure_command_interrupt(
                backend=args.backend,
                mock_base_latency_us=mock_base_latency_us,
                mock_per_key_latency_us=mock_per_key_latency_us,
                adaptive_spin=not args.no_adaptive_spin,
                rt_priority_mode=args.rt_priority_mode,
            )
        )
    sender_errors = [
        value
        for run in dispatch_runs
        for value in run["_sender_error_values"]
    ]
    report: dict[str, Any] = {
        "label": args.label,
        "backend": args.backend,
        "actions_per_polyphony": args.actions * 2,
        "polyphony": polyphonies,
        "repeats": args.repeats,
        "budget_seconds": args.budget_seconds,
        "sender_completion_error_us": {
            key: _stats(sender_errors)[key]
            for key in ("p50", "p95", "p99", "max")
        },
        "completion_error_us": _completion_error_report(
            [record for run in dispatch_runs for record in run["_records"]]
        ),
        "spin_cpu_time_us": _stats([run["spin_cpu_time_us"] for run in dispatch_runs]),
        "peak_rss_bytes": _required_stats(
            [run["peak_rss_bytes"] for run in dispatch_runs if run["peak_rss_bytes"] is not None],
            "peak_rss_bytes",
        ),
        "command_interrupt_latency_us": _stats(interrupt_runs),
        "keys_dropped": sum(run["keys_dropped"] for run in dispatch_runs),
        "failed_release_count": sum(run["failed_release_count"] for run in dispatch_runs),
        "chord_split_events": sum(run["chord_split_events"] for run in dispatch_runs),
        "sendinput_partial_events": sum(
            run["sendinput_partial_events"] for run in dispatch_runs
        ),
        "sendinput_zero_progress_failures": sum(
            run["sendinput_zero_progress_failures"] for run in dispatch_runs
        ),
        "positive_residual_at_cap": sum(
            run["positive_residual_at_cap"] for run in dispatch_runs
        ),
        "outcomes": sorted({run["outcome"] for run in dispatch_runs}),
        "mock_latency_model": {
            "base_us": mock_base_latency_us,
            "per_key_us": mock_per_key_latency_us,
        }
        if args.backend == "mock"
        else None,
        "by_polyphony": by_polyphony,
        "evidence_scope": "sender_completion",
        "git_sha": git_info["git_sha"],
        "native_build_commit": native_info["native_build_commit"],
        "rustc_version": native_info["rustc_version"],
        "schema_version": native_info["schema_version"],
        "backend_evidence": "real_sendinput_sender_completion"
        if args.backend == "sendinput"
        else "deterministic_coordinator_delivery_simulation",
        "host_fingerprint": host_info,
        "dirty_worktree": git_info["dirty_worktree"],
        "command_line": list(os.sys.argv),
    }
    if args.baseline is not None:
        _assert_baseline(report, args.baseline)
    encoded = json.dumps(report, indent=2)
    print(encoded)
    if args.output is not None:
        args.output.write_text(encoded + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
