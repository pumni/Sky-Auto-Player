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
        --repeats 3 --actions 512 --output native-followup.json

The completion-error percentiles are a host-side proxy. They do not measure
when a game samples Windows input, renders a frame, or produces audio.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import time
from pathlib import Path
from typing import Any

from sky_music.layouts import SKY_15_SCAN_CODES


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
) -> Any:
    import sky_player_rs

    return sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        actions,
        list(SKY_15_SCAN_CODES),
        min_hold_us=100,
        max_lead_us=2_000,
        mock_backend=backend == "mock",
        mock_latency_base_us=mock_base_latency_us,
        mock_latency_per_key_us=mock_per_key_latency_us,
        require_focus=False,
        telemetry_enabled=True,
        telemetry_capacity=max(1_024, len(actions) + 16),
        rt_priority_mode="off",
        enable_waitable_timer=True,
        enable_event_wait=True,
        enable_adaptive_spin=adaptive_spin,
        enable_spin_reprobe=adaptive_spin,
        enable_adaptive_lead=True,
    )


def _run_dispatch(
    actions: list[tuple[int, str, int, list[int], str]],
    polyphony: int,
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
) -> dict[str, Any]:
    session = _new_session(
        actions,
        backend=backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=adaptive_spin,
    )
    started_ns = time.perf_counter_ns()
    session.start()
    if not session.join(timeout_ms=60_000):
        raise RuntimeError("native acceptance session exceeded 60 seconds")
    wall_us = (time.perf_counter_ns() - started_ns) // 1_000
    snapshot = dict(session.snapshot())
    telemetry = json.loads(session.take_telemetry_json())
    records = telemetry.get("records", [])
    sender_errors = [int(record["visible_lateness_us"]) for record in records]
    lead_by_polyphony = {
        str(len(record.get("scan_codes", []))): int(record.get("applied_lead_us", 0))
        for record in records
        if record.get("kind") == "down" and record.get("scan_codes")
    }
    peak_rss = _peak_working_set_bytes()
    result: dict[str, Any] = {
        "polyphony": polyphony,
        "wall_us": wall_us,
        "_sender_error_values": sender_errors,
        "sender_completion_error_us": _stats(sender_errors),
        "spin_cpu_time_us": int(snapshot.get("spin_time_us", 0)),
        "peak_rss_bytes": peak_rss,
        "keys_dropped": int(snapshot.get("keys_dropped", 0)),
        "failed_release_count": int(snapshot.get("failed_release_count", 0)),
        "chord_split_events": int(snapshot.get("chord_split_events", 0)),
        "sendinput_partial_events": int(snapshot.get("sendinput_partial_events", 0)),
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
    )
    session.start()
    deadline = time.perf_counter() + 2.0
    while not bool(dict(session.snapshot()).get("is_running")):
        if time.perf_counter() >= deadline:
            raise RuntimeError("native worker did not enter running state")
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
    parser.add_argument("--actions", type=int, default=512)
    parser.add_argument("--repeats", type=int, default=3)
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
    parser.add_argument("--mock-base-latency-us", type=int, default=80)
    parser.add_argument("--mock-per-key-latency-us", type=int, default=40)
    parser.add_argument(
        "--no-adaptive-spin",
        action="store_true",
        help="disable adaptive wait probing; adaptive lead remains enabled",
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


def main() -> int:
    args = _parse_args()
    if os.name != "nt":
        raise SystemExit("this acceptance benchmark requires Windows")
    if args.actions <= 0 or args.repeats <= 0:
        raise SystemExit("--actions and --repeats must be positive")
    if args.backend == "sendinput" and not args.allow_real_input:
        raise SystemExit("--backend sendinput requires --allow-real-input")
    if args.mock_base_latency_us < 0 or args.mock_per_key_latency_us < 0:
        raise SystemExit("mock latency values must be non-negative")
    if args.backend == "sendinput" and (
        args.mock_base_latency_us or args.mock_per_key_latency_us
    ):
        raise SystemExit("mock latency values are only valid with --backend mock")

    polyphonies = _parse_polyphony(args.polyphony)
    dispatch_runs: list[dict[str, Any]] = []
    by_polyphony: dict[str, Any] = {}
    for polyphony in polyphonies:
        actions = _actions(args.actions, polyphony)
        runs = [
            _run_dispatch(
                actions,
                polyphony,
                backend=args.backend,
                mock_base_latency_us=args.mock_base_latency_us,
                mock_per_key_latency_us=args.mock_per_key_latency_us,
                adaptive_spin=not args.no_adaptive_spin,
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
            "spin_cpu_time_us": _stats([run["spin_cpu_time_us"] for run in runs]),
            "peak_rss_bytes": _stats(
                [run["peak_rss_bytes"] for run in runs if run["peak_rss_bytes"] is not None]
            ),
            "keys_dropped": sum(run["keys_dropped"] for run in runs),
            "failed_release_count": sum(run["failed_release_count"] for run in runs),
            "chord_split_events": sum(run["chord_split_events"] for run in runs),
            "sendinput_partial_events": sum(
                run["sendinput_partial_events"] for run in runs
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
    interrupt_runs = [
        _measure_command_interrupt(
            backend=args.backend,
            mock_base_latency_us=args.mock_base_latency_us,
            mock_per_key_latency_us=args.mock_per_key_latency_us,
            adaptive_spin=not args.no_adaptive_spin,
        )
        for _ in range(args.repeats)
    ]
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
        "sender_completion_error_us": {
            key: _stats(sender_errors)[key]
            for key in ("p50", "p95", "p99", "max")
        },
        "spin_cpu_time_us": _stats([run["spin_cpu_time_us"] for run in dispatch_runs]),
        "peak_rss_bytes": _stats(
            [run["peak_rss_bytes"] for run in dispatch_runs if run["peak_rss_bytes"] is not None]
        ),
        "command_interrupt_latency_us": _stats(interrupt_runs),
        "keys_dropped": sum(run["keys_dropped"] for run in dispatch_runs),
        "failed_release_count": sum(run["failed_release_count"] for run in dispatch_runs),
        "chord_split_events": sum(run["chord_split_events"] for run in dispatch_runs),
        "sendinput_partial_events": sum(
            run["sendinput_partial_events"] for run in dispatch_runs
        ),
        "positive_residual_at_cap": sum(
            run["positive_residual_at_cap"] for run in dispatch_runs
        ),
        "outcomes": sorted({run["outcome"] for run in dispatch_runs}),
        "mock_latency_model": {
            "base_us": args.mock_base_latency_us,
            "per_key_us": args.mock_per_key_latency_us,
        }
        if args.backend == "mock"
        else None,
        "by_polyphony": by_polyphony,
        "evidence_scope": (
            "sender_side_polyphony_latency_model"
            if args.backend == "mock"
            else "sender_side_real_sendinput_fixed_host"
        ),
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
