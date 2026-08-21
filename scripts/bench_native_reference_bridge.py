"""Compatibility bridge for real-SendInput A/B runs against historical native wheels.

The candidate checkout owns the benchmark harness. Historical native reference
commits predate the current ``DispatchSession`` Python ABI and retain adaptive
runtime policies that must be reported, not rewritten. This bridge patches only
the harness-facing compatibility seams for those exact reference SHAs; current
candidate behavior remains owned by ``bench_native_acceptance``.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

import bench_native_acceptance as acceptance

from sky_music.layouts import SKY_15_SCAN_CODES

BRIDGE_SCHEMA_VERSION = 8
LEGACY_ALLOWED_SCAN_CODES_API = "legacy_allowed_scan_codes_v1"
CURRENT_SESSION_API = "current_registry_v2"
HISTORICAL_REFERENCE_SHAS = frozenset(
    {
        acceptance.SAME_SEMANTICS_REFERENCE_SHA,
        acceptance.TRANSPORT_REFERENCE_SHA,
    }
)


def _arg_value(flag: str) -> str | None:
    try:
        index = sys.argv.index(flag)
    except ValueError:
        return None
    if index + 1 >= len(sys.argv):
        raise SystemExit(f"{flag} requires a value")
    return sys.argv[index + 1].strip().lower()


def _session_api_for_build(native_build_commit: str) -> str:
    if native_build_commit in HISTORICAL_REFERENCE_SHAS:
        return LEGACY_ALLOWED_SCAN_CODES_API
    return CURRENT_SESSION_API


def _timeline_semantics_for_build(native_build_commit: str) -> int:
    return int(
        acceptance.KNOWN_TIMELINE_SEMANTICS.get(
            native_build_commit, acceptance.TIMELINE_SEMANTICS_VERSION
        )
    )


def _native_policy(
    *,
    backend: str,
    native_build_commit: str,
    config: dict[str, Any],
) -> dict[str, Any]:
    if backend != "sendinput":
        return {
            "session_api": "test_support",
            "build_flavor": config.get("native_build_flavor"),
            "profile": config.get("native_profile"),
            "priority_mode": config.get("rt_priority_mode"),
            "waitable_timer": config.get("waitable_timer"),
            "event_wait": config.get("event_wait"),
            "adaptive_spin": config.get("adaptive_spin"),
            "lead_mode": config.get("lead_mode"),
            "fixed_lead_us": config.get("fixed_lead_us"),
        }
    historical = native_build_commit in HISTORICAL_REFERENCE_SHAS
    return {
        "session_api": _session_api_for_build(native_build_commit),
        "build_flavor": "production",
        "profile": "strict_timing_diagnostic",
        "priority_mode": "auto",
        "waitable_timer": True,
        "event_wait": True,
        "adaptive_spin": historical,
        "lead_mode": "adaptive" if historical else "fixed",
        "fixed_lead_us": 0,
        "adaptive_lead": historical,
        "spin_policy": "adaptive_floor_700_threshold_150" if historical else "fixed_700",
    }


def _workload_config(config: dict[str, Any]) -> dict[str, Any]:
    keys = (
        "backend",
        "game_fps",
        "actions",
        "polyphony",
        "scenario",
        "gap_profile",
        "warmup_cycles",
        "start_delay_us",
        "native_profile",
        "native_build_flavor",
        "require_focus",
        "materialized_min_hold_us",
        "mock_base_latency_us",
        "mock_per_key_latency_us",
    )
    return {key: config.get(key) for key in keys}


def _normalize_historical_trace(
    output: dict[str, Any], *, native_build_commit: str
) -> dict[str, Any]:
    if (
        native_build_commit not in HISTORICAL_REFERENCE_SHAS
        or output.get("schema_version") != 8
    ):
        return output
    records = output.get("records")
    if not isinstance(records, list):
        return output
    normalized_records: list[dict[str, Any]] = []
    changed = False
    for record in records:
        if not isinstance(record, dict):
            return output
        normalized = dict(record)
        if (
            "core_post_send_duration_us" not in normalized
            and "bookkeeping_duration_us" in normalized
        ):
            normalized["core_post_send_duration_us"] = normalized[
                "bookkeeping_duration_us"
            ]
            changed = True
        normalized_records.append(normalized)
    if not changed:
        return output
    return {**output, "records": normalized_records}


def _create_real_dispatch_session(
    *,
    sky_player_rs: Any,
    actions: list[tuple[int, str, int, list[int], str]],
    config: Any,
    native_build_commit: str,
) -> Any:
    if native_build_commit in HISTORICAL_REFERENCE_SHAS:
        return sky_player_rs.DispatchSession(
            actions,
            list(SKY_15_SCAN_CODES),
            config=config,
        )
    return sky_player_rs.DispatchSession(actions, config=config)


def _assert_bridge_baseline_compatible(
    report: dict[str, Any], baseline: dict[str, Any]
) -> None:
    if baseline.get("benchmark_schema_version") != BRIDGE_SCHEMA_VERSION:
        raise SystemExit(
            f"legacy baseline is incompatible; regenerate with benchmark schema version {BRIDGE_SCHEMA_VERSION}"
        )
    if report.get("benchmark_schema_version") != BRIDGE_SCHEMA_VERSION:
        raise SystemExit("candidate benchmark schema version is invalid")
    if baseline.get("command_timing_domain") != acceptance.COMMAND_TIMING_DOMAIN:
        raise SystemExit("baseline command timing domain is incompatible")
    if report.get("command_timing_domain") != acceptance.COMMAND_TIMING_DOMAIN:
        raise SystemExit("candidate command timing domain is incompatible")
    if baseline.get("latency_segment_domain") != acceptance.LATENCY_SEGMENT_DOMAIN:
        raise SystemExit("baseline latency segment domain is incompatible")
    if report.get("latency_segment_domain") != acceptance.LATENCY_SEGMENT_DOMAIN:
        raise SystemExit("candidate latency segment domain is incompatible")
    acceptance._assert_comparison_contract(report, baseline)
    if report.get("candidate_sha") != acceptance._report_sha(report):
        raise SystemExit("candidate_sha must identify the candidate artifact")
    baseline_config = baseline.get("benchmark_config")
    report_config = report.get("benchmark_config")
    if not isinstance(baseline_config, dict) or not isinstance(report_config, dict):
        raise SystemExit("benchmark_config is required for A/B comparison")
    baseline_workload = baseline_config.get("workload")
    report_workload = report_config.get("workload")
    if not isinstance(baseline_workload, dict) or not isinstance(report_workload, dict):
        raise SystemExit("benchmark_config.workload is required for A/B comparison")
    if baseline_workload != report_workload:
        raise SystemExit("benchmark workload mismatch; refusing to compare metrics")
    for name, config in (("baseline", baseline_config), ("candidate", report_config)):
        policy = config.get("native_policy")
        if not isinstance(policy, dict) or not policy.get("session_api"):
            raise SystemExit(f"{name} benchmark native_policy provenance is missing")
    if baseline.get("statistics_eligible") is not True:
        raise SystemExit("baseline is incompatible; it is not statistics-eligible")
    if baseline.get("excluded_runs") != 0:
        raise SystemExit("baseline is incompatible; excluded runs are not allowed")
    if report.get("statistics_eligible") is not True:
        raise SystemExit("candidate must be statistics-eligible")


def _install_patches(native_build_commit: str) -> None:
    original_benchmark_config = acceptance._benchmark_config
    original_new_session = acceptance._new_session
    original_normalize = acceptance._normalize_historical_native_trace
    original_comparison_metadata = acceptance._comparison_metadata
    original_wake_slo = acceptance._assert_absolute_wake_slo
    original_pre_call_slo = acceptance._assert_absolute_pre_call_slo

    acceptance.BENCHMARK_SCHEMA_VERSION = BRIDGE_SCHEMA_VERSION

    def benchmark_config(*, args, polyphonies, mock_base_latency_us, mock_per_key_latency_us):
        config = original_benchmark_config(
            args=args,
            polyphonies=polyphonies,
            mock_base_latency_us=mock_base_latency_us,
            mock_per_key_latency_us=mock_per_key_latency_us,
        )
        policy = _native_policy(
            backend=args.backend,
            native_build_commit=native_build_commit,
            config=config,
        )
        if args.backend == "sendinput":
            config.update(
                {
                    "rt_priority_mode": policy["priority_mode"],
                    "adaptive_spin": policy["adaptive_spin"],
                    "lead_mode": policy["lead_mode"],
                    "fixed_lead_us": policy["fixed_lead_us"],
                    "native_profile": policy["profile"],
                }
            )
        config["workload"] = _workload_config(config)
        config["native_policy"] = policy
        return config

    def new_session(actions, **kwargs):
        if kwargs.get("backend") != "sendinput" or native_build_commit not in HISTORICAL_REFERENCE_SHAS:
            return original_new_session(actions, **kwargs)
        import sky_player_rs

        if callable(getattr(sky_player_rs, "TestDispatchSession", None)):
            raise RuntimeError(
                "SendInput qualification requires a production native wheel; "
                "test-support is not a valid physical timing path"
            )
        game_fps = int(kwargs.get("game_fps", 60))
        gap_profile = str(kwargs.get("gap_profile", "hot"))
        materialized_min_hold_us = acceptance._materialized_hold_us(
            game_fps=game_fps,
            gap_profile=gap_profile,
        )
        require_focus = bool(kwargs.get("require_focus", True))
        target_hwnd = acceptance._real_input_target_hwnd(require_focus=require_focus)
        config = sky_player_rs.SessionConfig(
            game_fps=game_fps,
            min_hold_us=materialized_min_hold_us,
            require_focus=require_focus,
            target_hwnd=target_hwnd,
            telemetry=True,
            profile="strict_timing_diagnostic",
        )
        return _create_real_dispatch_session(
            sky_player_rs=sky_player_rs,
            actions=actions,
            config=config,
            native_build_commit=native_build_commit,
        )

    def normalize(output: dict[str, Any], *, native_build_commit: str):
        normalized = _normalize_historical_trace(
            output,
            native_build_commit=native_build_commit,
        )
        return original_normalize(
            normalized,
            native_build_commit=native_build_commit,
        )

    def comparison_metadata(candidate_sha: str, baseline_path: Path | None):
        metadata = original_comparison_metadata(candidate_sha, baseline_path)
        metadata["timeline_semantics_version"] = _timeline_semantics_for_build(candidate_sha)
        return metadata

    def wake_slo(report: dict[str, Any]) -> None:
        if native_build_commit in HISTORICAL_REFERENCE_SHAS:
            return
        original_wake_slo(report)

    def pre_call_slo(report: dict[str, Any]) -> None:
        if native_build_commit in HISTORICAL_REFERENCE_SHAS:
            return
        original_pre_call_slo(report)

    acceptance._benchmark_config = benchmark_config
    acceptance._new_session = new_session
    acceptance._normalize_historical_native_trace = normalize
    acceptance._comparison_metadata = comparison_metadata
    acceptance._assert_baseline_compatible = _assert_bridge_baseline_compatible
    acceptance._assert_absolute_wake_slo = wake_slo
    acceptance._assert_absolute_pre_call_slo = pre_call_slo


def main() -> int:
    native_build_commit = _arg_value("--expected-native-build-commit")
    if not native_build_commit:
        raise SystemExit(
            "historical reference bridge requires --expected-native-build-commit"
        )
    _install_patches(native_build_commit)
    return acceptance.main()


if __name__ == "__main__":
    raise SystemExit(main())
