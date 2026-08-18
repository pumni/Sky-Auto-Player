from __future__ import annotations

import importlib.util
import sys
from collections.abc import Mapping
from pathlib import Path
from types import SimpleNamespace

import pytest

_SCRIPTS = Path(__file__).parents[1] / "scripts"
sys.path.insert(0, str(_SCRIPTS))
_SPEC = importlib.util.spec_from_file_location(
    "bench_native_reference_bridge", _SCRIPTS / "bench_native_reference_bridge.py"
)
assert _SPEC is not None and _SPEC.loader is not None
BRIDGE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(BRIDGE)


def test_historical_references_use_legacy_dispatch_session_api() -> None:
    assert (
        BRIDGE._session_api_for_build(BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA)
        == BRIDGE.LEGACY_ALLOWED_SCAN_CODES_API
    )
    assert (
        BRIDGE._session_api_for_build(BRIDGE.acceptance.TRANSPORT_REFERENCE_SHA)
        == BRIDGE.LEGACY_ALLOWED_SCAN_CODES_API
    )
    assert BRIDGE._session_api_for_build("candidate-sha") == BRIDGE.CURRENT_SESSION_API


def test_transport_reference_keeps_timeline_semantics_v1() -> None:
    assert (
        BRIDGE._timeline_semantics_for_build(BRIDGE.acceptance.TRANSPORT_REFERENCE_SHA)
        == 1
    )
    assert (
        BRIDGE._timeline_semantics_for_build(BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA)
        == 2
    )
    assert BRIDGE._timeline_semantics_for_build("candidate-sha") == 2


def test_historical_schema_eight_trace_projects_bookkeeping_field() -> None:
    payload: dict[str, object] = {
        "schema_version": 8,
        "records": [{"bookkeeping_duration_us": 17}],
    }
    normalized = BRIDGE._normalize_historical_trace(
        payload,
        native_build_commit=BRIDGE.acceptance.TRANSPORT_REFERENCE_SHA,
    )
    records = normalized.get("records")
    assert isinstance(records, list)
    assert records and isinstance(records[0], dict)
    assert records[0]["core_post_send_duration_us"] == 17

    source_records = payload["records"]
    assert isinstance(source_records, list)
    assert source_records and isinstance(source_records[0], dict)
    assert "core_post_send_duration_us" not in source_records[0]


def test_legacy_dispatch_session_receives_explicit_allowlist() -> None:
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def dispatch_session(*args, **kwargs):
        calls.append((args, kwargs))
        return object()

    fake_native = SimpleNamespace(DispatchSession=dispatch_session)
    actions = [(0, "down", 0, [0x10], "test")]
    config = object()
    BRIDGE._create_real_dispatch_session(
        sky_player_rs=fake_native,
        actions=actions,
        config=config,
        native_build_commit=BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA,
    )
    args, kwargs = calls[-1]
    assert args[0] == actions
    assert isinstance(args[1], list)
    assert args[1] == list(BRIDGE.SKY_15_SCAN_CODES)
    assert kwargs == {"config": config}


def test_current_dispatch_session_does_not_receive_legacy_allowlist() -> None:
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def dispatch_session(*args, **kwargs):
        calls.append((args, kwargs))
        return object()

    fake_native = SimpleNamespace(DispatchSession=dispatch_session)
    actions = [(0, "down", 0, [0x10], "test")]
    config = object()
    BRIDGE._create_real_dispatch_session(
        sky_player_rs=fake_native,
        actions=actions,
        config=config,
        native_build_commit="candidate-sha",
    )
    args, kwargs = calls[-1]
    assert args == (actions,)
    assert kwargs == {"config": config}


def _report(
    *,
    sha: str,
    workload: Mapping[str, object],
    policy: Mapping[str, object],
) -> dict[str, object]:
    return {
        "benchmark_schema_version": BRIDGE.BRIDGE_SCHEMA_VERSION,
        "candidate_sha": sha,
        "reference_sha": BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA,
        "comparison_role": BRIDGE.acceptance.SAME_SEMANTICS,
        "timeline_semantics_version": 2,
        "command_timing_domain": BRIDGE.acceptance.COMMAND_TIMING_DOMAIN,
        "latency_segment_domain": BRIDGE.acceptance.LATENCY_SEGMENT_DOMAIN,
        "benchmark_config": {
            "workload": dict(workload),
            "native_policy": dict(policy),
        },
        "statistics_eligible": True,
        "excluded_runs": 0,
    }


def test_comparison_accepts_same_workload_with_different_native_policy(monkeypatch) -> None:
    workload = {"backend": "sendinput", "game_fps": 60}
    baseline = _report(
        sha=BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA,
        workload=workload,
        policy={"session_api": BRIDGE.LEGACY_ALLOWED_SCAN_CODES_API, "adaptive_spin": True},
    )
    candidate = _report(
        sha="candidate-sha",
        workload=workload,
        policy={"session_api": BRIDGE.CURRENT_SESSION_API, "adaptive_spin": False},
    )
    monkeypatch.setattr(BRIDGE.acceptance, "_assert_comparison_contract", lambda report, baseline: None)
    monkeypatch.setattr(BRIDGE.acceptance, "_report_sha", lambda report: str(report["candidate_sha"]))
    BRIDGE._assert_bridge_baseline_compatible(candidate, baseline)


def test_comparison_rejects_workload_mismatch(monkeypatch) -> None:
    baseline = _report(
        sha=BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA,
        workload={"backend": "sendinput", "game_fps": 60},
        policy={"session_api": BRIDGE.LEGACY_ALLOWED_SCAN_CODES_API},
    )
    candidate = _report(
        sha="candidate-sha",
        workload={"backend": "sendinput", "game_fps": 120},
        policy={"session_api": BRIDGE.CURRENT_SESSION_API},
    )
    monkeypatch.setattr(BRIDGE.acceptance, "_assert_comparison_contract", lambda report, baseline: None)
    monkeypatch.setattr(BRIDGE.acceptance, "_report_sha", lambda report: str(report["candidate_sha"]))
    with pytest.raises(SystemExit, match="workload mismatch"):
        BRIDGE._assert_bridge_baseline_compatible(candidate, baseline)


def test_ab_real_backend_routes_through_reference_bridge() -> None:
    import bench_native_ab  # pyright: ignore[reportMissingImports]

    args = SimpleNamespace(
        backend="sendinput",
        actions=128,
        dispatch_repeats=2,
        command_samples=0,
        polyphony="1,5,15",
        game_fps=60,
        lead_mode="fixed",
        fixed_lead_us=0,
        gap_profile="hot",
        warmup_cycles=8,
        rt_priority_mode="off",
        budget_seconds=120.0,
        label="baseline",
        allow_real_input=True,
    )
    command = bench_native_ab._benchmark_command(
        output=Path("baseline.json"),
        expected_native_commit=BRIDGE.acceptance.SAME_SEMANTICS_REFERENCE_SHA,
        args=args,
    )
    assert "scripts/bench_native_reference_bridge.py" in command


def test_ab_real_backend_matrix_defers_native_policy_to_each_leg() -> None:
    import bench_native_ab  # pyright: ignore[reportMissingImports]

    args = SimpleNamespace(
        backend="sendinput",
        actions=128,
        dispatch_repeats=2,
        command_samples=0,
        polyphony="1,5,15",
        game_fps=60,
        lead_mode="fixed",
        fixed_lead_us=0,
        gap_profile="hot",
        warmup_cycles=8,
        rt_priority_mode="off",
        budget_seconds=120.0,
        allow_real_input=True,
    )
    matrix = bench_native_ab._benchmark_matrix(args)
    assert matrix["native_policy_source"] == "per-leg benchmark report"
    assert "adaptive_spin" not in matrix
    assert "lead_mode" not in matrix
    assert "rt_priority_mode" not in matrix