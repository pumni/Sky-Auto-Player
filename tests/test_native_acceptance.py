from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from types import ModuleType, SimpleNamespace

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]


def _load_acceptance_module() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "bench_native_acceptance.py"
    spec = importlib.util.spec_from_file_location("native_acceptance_under_test", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


ACCEPTANCE = _load_acceptance_module()


def test_production_wheel_omits_optional_calibration_surface() -> None:
    names = {name for name in dir(sky_player_rs) if not name.startswith("_")}
    assert "run_calibration_rs" not in names
    assert "calibration_schema_version" not in names
    assert "calibration_schema_version" not in sky_player_rs.build_info()  # type: ignore[attr-defined]


def test_real_backend_without_mock_options_uses_zero_mock_latency() -> None:
    assert ACCEPTANCE._resolve_mock_latency_values(
        backend="sendinput",
        mock_base_latency_us=None,
        mock_per_key_latency_us=None,
    ) == (0, 0)


def test_mock_backend_defaults_preserve_latency_model() -> None:
    assert ACCEPTANCE._resolve_mock_latency_values(
        backend="mock",
        mock_base_latency_us=None,
        mock_per_key_latency_us=None,
    ) == (80, 40)


def test_mock_latency_overrides_are_preserved_for_mock_backend() -> None:
    assert ACCEPTANCE._resolve_mock_latency_values(
        backend="mock",
        mock_base_latency_us=100,
        mock_per_key_latency_us=25,
    ) == (100, 25)


@pytest.mark.parametrize("budget_seconds", [1.0, 120.0, 300.0, 600.0])
def test_long_benchmark_budget_is_allowed_for_native_sample_runs(
    budget_seconds: float,
) -> None:
    assert (
        ACCEPTANCE.MIN_BENCHMARK_BUDGET_SECONDS
        <= budget_seconds
        <= ACCEPTANCE.MAX_BENCHMARK_BUDGET_SECONDS
    )


def test_benchmark_budget_cap_leaves_room_for_release_command_samples() -> None:
    assert ACCEPTANCE.MAX_BENCHMARK_BUDGET_SECONDS >= 300.0


def test_benchmark_default_priority_policy_is_off(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(sys, "argv", ["bench_native_acceptance.py"])
    assert ACCEPTANCE._parse_args().rt_priority_mode == "off"


def test_repeats_alias_cannot_be_combined_with_dispatch_repeats() -> None:
    args = SimpleNamespace(repeats=2, dispatch_repeats=3, command_samples=4)
    with pytest.raises(SystemExit, match="ambiguous"):
        ACCEPTANCE._resolve_repeat_counts(args)


def test_schema_two_baseline_requires_matching_timing_domain_and_config() -> None:
    config = {
        "backend": "mock",
        "rt_priority_mode": "off",
        "adaptive_spin": True,
        "waitable_timer": True,
        "event_wait": True,
        "mock_base_latency_us": 80,
        "mock_per_key_latency_us": 40,
        "actions": 128,
        "polyphony": [1, 2, 3, 5, 8, 15],
    }
    report = {
        "benchmark_schema_version": 2,
        "command_timing_domain": "native_qpc_v1",
        "benchmark_config": config,
        "statistics_eligible": True,
        "excluded_runs": 0,
        "sender_completion_error_us": {"p50": 1, "p99": 1, "max": 1},
        "command_observation_latency_us": {"p99": 1},
        "command_completion_latency_us": {"p99": 1},
        "worker_cpu_ratio_ppm": {"p50": 1},
        "process_cpu_ratio_ppm": {"p50": 1},
        "spin_cpu_ratio_ppm": {"p50": 1},
        "peak_rss_bytes": {"max": 1},
    }
    ACCEPTANCE._assert_baseline_compatible(report, dict(report))

    legacy = {"command_timing_domain": "native_qpc"}
    with pytest.raises(SystemExit, match="legacy baseline"):
        ACCEPTANCE._assert_baseline_compatible(report, legacy)

    mismatched = dict(report)
    mismatched["benchmark_config"] = {**config, "rt_priority_mode": "auto"}
    with pytest.raises(SystemExit, match="fingerprint mismatch"):
        ACCEPTANCE._assert_baseline_compatible(report, mismatched)


def test_workflow_dispatch_marks_validation_relevant_before_path_diff() -> None:
    workflow = (
        Path(__file__).parents[1] / ".github" / "workflows" / "ci.yml"
    ).read_text(encoding="utf-8")
    manual_branch = 'if [[ "$EVENT_NAME" == "workflow_dispatch" ]]'
    path_diff = 'changed_files="$(git diff --name-only "$BEFORE_SHA" "$CURRENT_SHA")"'
    assert workflow.index(manual_branch) < workflow.index(path_diff)
    assert "--dispatch-repeats 3" in workflow
    assert "--command-samples 100" in workflow
    assert "--rt-priority-mode off" in workflow


@pytest.mark.windows
def test_test_support_pause_timing_smoke_uses_100_fresh_sessions() -> None:
    if not callable(getattr(sky_player_rs, "TestDispatchSession", None)):
        pytest.skip("requires the test-support native wheel")
    for _ in range(100):
        result = ACCEPTANCE._measure_command_interrupt(
            backend="mock",
            mock_base_latency_us=80,
            mock_per_key_latency_us=40,
            adaptive_spin=True,
            rt_priority_mode="off",
        )
        assert result["requested_ticks"] <= result["observed_ticks"]
        assert result["observed_ticks"] <= result["acknowledged_ticks"]
    assert result["generation"] > 0


@pytest.mark.windows
def test_test_support_pause_timing_waits_for_startup_ready_boundary() -> None:
    if not callable(getattr(sky_player_rs, "TestDispatchSession", None)):
        pytest.skip("requires the test-support native wheel")
    result = ACCEPTANCE._measure_command_interrupt(
        backend="mock",
        mock_base_latency_us=80,
        mock_per_key_latency_us=40,
        adaptive_spin=True,
        rt_priority_mode="off",
    )
    assert result["requested_ticks"] <= result["observed_ticks"]


@pytest.mark.parametrize(
    ("base_latency_us", "per_key_latency_us"),
    [(0, 1), (1, 0), (80, 40)],
)
def test_explicit_mock_options_are_rejected_for_real_backend(
    base_latency_us: int, per_key_latency_us: int
) -> None:
    with pytest.raises(SystemExit, match="only valid with --backend mock"):
        ACCEPTANCE._resolve_mock_latency_values(
            backend="sendinput",
            mock_base_latency_us=base_latency_us,
            mock_per_key_latency_us=per_key_latency_us,
        )


def test_negative_mock_latency_is_rejected() -> None:
    with pytest.raises(SystemExit, match="non-negative"):
        ACCEPTANCE._resolve_mock_latency_values(
            backend="mock",
            mock_base_latency_us=-1,
            mock_per_key_latency_us=None,
        )


def _integrity_fixture() -> tuple[list[tuple[int, str, int, list[int], str]], dict, list]:
    actions = [
        (10, "down", 0, [21], "down"),
        (11, "up", 5_000, [21], "up"),
    ]
    telemetry = {
        "attempted": 2,
        "accepted": 2,
        "dropped": 0,
        "truncated": False,
    }
    records = [
        SimpleNamespace(event_index=10, kind="down"),
        SimpleNamespace(event_index=11, kind="up"),
    ]
    return actions, telemetry, records


def test_complete_native_telemetry_passes_one_to_one_validation() -> None:
    actions, telemetry, records = _integrity_fixture()

    diagnostics = ACCEPTANCE._validate_telemetry_integrity(
        actions=actions, telemetry=telemetry, records=records, polyphony=1
    )

    assert diagnostics["missing_indices"] == []
    assert diagnostics["duplicate_indices"] == []
    assert diagnostics["kind_mismatches"] == []


@pytest.mark.parametrize(
    ("mutate", "diagnostic"),
    [
        (lambda actions, telemetry, records: records.pop(), "missing_indices"),
        (
            lambda actions, telemetry, records: records.__setitem__(
                1, SimpleNamespace(event_index=10, kind="down")
            ),
            "duplicate_indices",
        ),
        (
            lambda actions, telemetry, records: records.__setitem__(
                1, SimpleNamespace(event_index=12, kind="up")
            ),
            "unexpected_indices",
        ),
        (
            lambda actions, telemetry, records: records.__setitem__(
                1, SimpleNamespace(event_index=11, kind="down")
            ),
            "kind_mismatches",
        ),
        (lambda actions, telemetry, records: telemetry.__setitem__("dropped", 1), "dropped"),
        (
            lambda actions, telemetry, records: telemetry.__setitem__("truncated", True),
            "truncated",
        ),
        (
            lambda actions, telemetry, records: telemetry.__setitem__("attempted", 1),
            "attempted",
        ),
        (
            lambda actions, telemetry, records: telemetry.__setitem__("accepted", 1),
            "accepted",
        ),
    ],
)
def test_invalid_native_telemetry_fails_closed(mutate, diagnostic: str) -> None:
    actions, telemetry, records = _integrity_fixture()
    mutate(actions, telemetry, records)

    with pytest.raises(ACCEPTANCE.TelemetryIntegrityError) as raised:
        ACCEPTANCE._validate_telemetry_integrity(
            actions=actions, telemetry=telemetry, records=records, polyphony=1
        )

    assert raised.value.diagnostics[diagnostic]


def test_failed_run_artifact_contains_raw_diagnostics_and_is_not_overwritten(
    tmp_path: Path,
) -> None:
    actions, telemetry, records = _integrity_fixture()
    path = ACCEPTANCE._failed_run_artifact_path(tmp_path / "acceptance.json", 3)
    exception = RuntimeError("synthetic failure")

    written = ACCEPTANCE._write_failed_run_artifact(
        path,
        git_info={"git_sha": "git"},
        native_info={"native_build_commit": "native"},
        host_info={"platform": "test"},
        run_index=3,
        polyphony=1,
        actions=actions,
        snapshot={"outcome": "error"},
        telemetry=telemetry,
        diagnostics={"records": len(records)},
        exception=exception,
    )

    assert written == path
    payload = json.loads(path.read_text(encoding="utf-8"))
    assert payload["raw_telemetry"] == telemetry
    assert payload["validation_diagnostics"] == {"records": 2}
    assert payload["exception"] == "RuntimeError: synthetic failure"
    assert ACCEPTANCE._failed_run_artifact_path(tmp_path / "acceptance.json", 3) != path


def test_partial_repetition_set_is_invalid_and_not_statistics_eligible() -> None:
    results = [
        ACCEPTANCE.BenchmarkRunResult(index, 0, {"ok": True}, None)
        for index in range(4)
    ]
    results.append(
        ACCEPTANCE.BenchmarkRunResult(
            4, 1, None, {"error": "telemetry incomplete"}
        )
    )

    summary = ACCEPTANCE._run_validity_summary(5, results)

    assert summary == {
        "requested_runs": 5,
        "successful_runs": 4,
        "failed_runs": 1,
        "run_validity": "invalid",
    }
