from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from types import ModuleType, SimpleNamespace
from typing import Any

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


def test_known_schema_seven_baseline_projects_legacy_post_send_field() -> None:
    output: dict[str, Any] = {
        "schema_version": 7,
        "records": [{"bookkeeping_duration_us": 4}],
    }

    normalized = ACCEPTANCE._normalize_historical_native_trace(
        output,
        native_build_commit=ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA,
    )

    assert normalized["records"][0]["core_post_send_duration_us"] == 4
    assert "core_post_send_duration_us" not in output["records"][0]


def test_unknown_schema_seven_does_not_project_legacy_post_send_field() -> None:
    output: dict[str, Any] = {
        "schema_version": 7,
        "records": [{"bookkeeping_duration_us": 4}],
    }

    normalized = ACCEPTANCE._normalize_historical_native_trace(
        output,
        native_build_commit="unknown-sha",
    )

    assert normalized is output
    assert "core_post_send_duration_us" not in output["records"][0]


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


def test_schema_five_baseline_requires_matching_timing_domain_and_config() -> None:
    config = {
        "backend": "mock",
        "game_fps": 60,
        "rt_priority_mode": "off",
        "adaptive_spin": True,
        "waitable_timer": True,
        "event_wait": True,
        "mock_base_latency_us": 80,
        "mock_per_key_latency_us": 40,
        "actions": 128,
        "polyphony": [1, 2, 3, 5, 8, 15],
        "lead_mode": "fixed",
        "fixed_lead_us": 0,
        "gap_profile": "hot",
        "warmup_cycles": 8,
    }
    report = {
        "benchmark_schema_version": 5,
        "candidate_sha": "candidate-sha",
        "reference_sha": ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA,
        "comparison_role": ACCEPTANCE.SAME_SEMANTICS,
        "timeline_semantics_version": 2,
        "command_timing_domain": "native_qpc_v1",
        "latency_segment_domain": "native_trace_v1",
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
    baseline = {
        **report,
        "candidate_sha": ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA,
        "reference_sha": ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA,
    }
    ACCEPTANCE._assert_baseline_compatible(report, baseline)

    legacy = {"command_timing_domain": "native_qpc"}
    with pytest.raises(SystemExit, match="legacy baseline"):
        ACCEPTANCE._assert_baseline_compatible(report, legacy)

    mismatched = dict(baseline)
    mismatched["benchmark_config"] = {**config, "rt_priority_mode": "auto"}
    with pytest.raises(SystemExit, match="fingerprint mismatch"):
        ACCEPTANCE._assert_baseline_compatible(report, mismatched)


def test_timeline_semantics_contract_rejects_cross_version_same_semantics() -> None:
    candidate = {
        "benchmark_schema_version": 5,
        "candidate_sha": "candidate-sha",
        "reference_sha": ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA,
        "comparison_role": ACCEPTANCE.SAME_SEMANTICS,
        "timeline_semantics_version": 2,
    }
    baseline = {
        "benchmark_schema_version": 5,
        "candidate_sha": ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA,
        "timeline_semantics_version": 1,
    }
    with pytest.raises(SystemExit, match="SAME_SEMANTICS"):
        ACCEPTANCE._assert_comparison_contract(candidate, baseline)


def test_transport_reference_allows_v2_candidate_against_known_v1() -> None:
    candidate = {
        "candidate_sha": "candidate-sha",
        "reference_sha": ACCEPTANCE.TRANSPORT_REFERENCE_SHA,
        "comparison_role": ACCEPTANCE.TRANSPORT_REFERENCE,
        "timeline_semantics_version": 2,
    }
    baseline = {"candidate_sha": ACCEPTANCE.TRANSPORT_REFERENCE_SHA}
    ACCEPTANCE._assert_comparison_contract(candidate, baseline)


def test_unknown_missing_timeline_semantics_is_rejected() -> None:
    with pytest.raises(SystemExit, match="unknown SHA"):
        ACCEPTANCE._timeline_semantics_version({"candidate_sha": "unknown-sha"})


def test_known_historical_sha_maps_missing_timeline_semantics() -> None:
    assert ACCEPTANCE._timeline_semantics_version(
        {"candidate_sha": ACCEPTANCE.TRANSPORT_REFERENCE_SHA}
    ) == 1
    assert ACCEPTANCE._timeline_semantics_version(
        {"candidate_sha": ACCEPTANCE.SAME_SEMANTICS_REFERENCE_SHA}
    ) == 2


def test_absolute_fixed_hot_wake_slo_is_exact() -> None:
    config = {"game_fps": 60, "gap_profile": "hot", "lead_mode": "fixed"}
    for value in (300,):
        ACCEPTANCE._assert_absolute_wake_slo(
            {
                "benchmark_config": config,
                "wake_error_us": {"absolute": {"p99": value}},
            }
        )
    with pytest.raises(SystemExit, match="absolute wake SLO"):
        ACCEPTANCE._assert_absolute_wake_slo(
            {
                "benchmark_config": config,
                "wake_error_us": {"absolute": {"p99": 301}},
            }
        )


def test_correctness_is_checked_before_percentiles() -> None:
    with pytest.raises(SystemExit, match="correctness failure"):
        ACCEPTANCE._assert_report_correctness(
            {"correctness": {"chord_integrity_lost": 1}}
        )


def test_p999_uses_nearest_rank() -> None:
    assert ACCEPTANCE._percentile([1, 2, 3, 4], 0.999) == 4


def test_signed_metrics_split_late_and_early_by_magnitude() -> None:
    report = ACCEPTANCE._completion_error_report_pairs(
        [("down", -7), ("down", 3), ("up", -2), ("up", 5)]
    )
    assert report["signed"]["p50"] == -2
    assert report["absolute"]["max"] == 7
    assert report["late"]["n"] == 2
    assert report["late"]["max"] == 5
    assert report["early"]["n"] == 2
    assert report["early"]["max"] == 7


def _trace_fixture(**overrides: object) -> SimpleNamespace:
    values: dict[str, Any] = {
        "kind": "down",
        "wake_us": 100,
        "wake_error_us": -2,
        "sender_started_us": 110,
        "sender_completed_us": 130,
        "sendinput_call_duration_us": 20,
        "core_post_send_duration_us": 4,
        "sender_completion_error_us": 3,
        "native_polyphony": 1,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def test_trace_metrics_calculate_pre_send_latency() -> None:
    rows = ACCEPTANCE._trace_metric_rows([_trace_fixture()])
    assert rows["pre_send_software_latency_us"] == [("down", 10)]


def test_trace_metrics_reject_invalid_timestamp_ordering() -> None:
    with pytest.raises(RuntimeError, match="timestamp ordering"):
        ACCEPTANCE._trace_metric_rows([_trace_fixture(sender_started_us=90)])


def test_trace_metrics_reject_missing_required_field() -> None:
    with pytest.raises(RuntimeError, match="wake_error_us"):
        ACCEPTANCE._trace_metric_rows([_trace_fixture(wake_error_us=None)])


def test_hot_and_cold_action_spacing() -> None:
    hot = ACCEPTANCE._actions(2, 1, gap_profile="hot")
    cold = ACCEPTANCE._actions(2, 1, gap_profile="cold")
    assert hot[2][2] - hot[0][2] == 19_667
    assert hot[1][2] - hot[0][2] == 9_833
    assert cold[2][2] - cold[0][2] == 60_000
    assert cold[1][2] - cold[0][2] == 30_000
    assert cold[2][2] - cold[1][2] > ACCEPTANCE.SEND_COLD_THRESHOLD_US


def test_hot_action_spacing_is_frame_safe_at_supported_fps() -> None:
    for fps in (30, 60, 120, 240):
        actions = ACCEPTANCE._actions(2, 15, gap_profile="hot", game_fps=fps)
        assert actions[2][2] - actions[0][2] >= (
            (1_000_000 + fps - 1) // fps
            + 500
            + ACCEPTANCE.BENCHMARK_HOLD_GUARD_US
        )


def test_warmup_records_are_integrity_input_but_measurement_slice_excludes_them() -> None:
    actions = ACCEPTANCE._actions(3, 1)
    assert len(actions) == 6
    records = [SimpleNamespace(event_index=index, kind=kind) for index, kind, *_ in actions]
    diagnostics = ACCEPTANCE._validate_telemetry_integrity(
        actions=actions,
        telemetry={"attempted": 6, "accepted": 6, "dropped": 0, "truncated": False},
        records=records,
        polyphony=1,
    )
    assert diagnostics["records_count"] == 6
    assert [record.event_index for record in records[2:]] == [2, 3, 4, 5]


def test_fixed_and_adaptive_lead_arguments_are_strict() -> None:
    assert ACCEPTANCE._resolve_lead_config(
        SimpleNamespace(lead_mode="fixed", fixed_lead_us=0)
    ) == ("fixed", 0)
    assert ACCEPTANCE._resolve_lead_config(
        SimpleNamespace(lead_mode="adaptive", fixed_lead_us=0)
    ) == ("adaptive", 0)
    with pytest.raises(SystemExit, match="must be 0"):
        ACCEPTANCE._resolve_lead_config(
            SimpleNamespace(lead_mode="adaptive", fixed_lead_us=1)
        )


def test_cpu_ratio_is_computed_per_run() -> None:
    assert ACCEPTANCE._ratio_ppm(50, 100) == 500_000


def test_threshold_uses_relative_bound_and_absolute_floor() -> None:
    assert ACCEPTANCE.allowed_value(
        100,
        relative_fraction=0.05,
        absolute_floor=5,
    ) == 105
    assert ACCEPTANCE.allowed_value(
        1,
        relative_fraction=0.05,
        absolute_floor=5,
    ) == 6


def test_rss_regression_uses_two_mib_floor() -> None:
    baseline = {"peak_rss_bytes": {"max": 100}}
    report = {"peak_rss_bytes": {"max": 2 * 1024 * 1024 + 100}}
    ACCEPTANCE._assert_metric_threshold(
        report,
        baseline,
        ("peak_rss_bytes", "max"),
        relative_fraction=0.05,
        absolute_floor=2 * 1024 * 1024,
    )


def test_no_outlier_exclusion_contract() -> None:
    stats = ACCEPTANCE._stats([1, 2, 100])
    assert stats["n"] == 3
    assert stats["max"] == 100


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
    assert "scripts/bench_native_ab.py" in workflow
    assert "fetch-depth: 0" in workflow
    assert "baseline_comparison=unavailable_initial_push" in workflow
    assert "adaptive-cold" in workflow


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


@pytest.mark.windows
@pytest.mark.parametrize(
    ("fault_mode", "expected_outcome", "counter"),
    [
        ("none", "finished", None),
        ("zero_progress_recovered", "finished", None),
        ("zero_progress_failed", "error", "sendinput_zero_progress_failures"),
        ("partial", "error", "sendinput_partial_events"),
    ],
)
def test_test_support_fault_matrix_publishes_terminal_counters(
    fault_mode: str,
    expected_outcome: str,
    counter: str | None,
) -> None:
    if not callable(getattr(sky_player_rs, "TestDispatchSession", None)):
        pytest.skip("requires the test-support native wheel")

    actions = ACCEPTANCE._actions(4, 3, gap_profile="hot", game_fps=60)
    session = sky_player_rs.TestDispatchSession(  # type: ignore[attr-defined]
        actions,
        list(ACCEPTANCE.SKY_15_SCAN_CODES),
        min_hold_us=100,
        game_fps=60,
        mock_latency_base_us=0,
        mock_latency_per_key_us=0,
        telemetry_capacity=256,
        fault_mode=fault_mode,
    )
    session.start()
    deadline = ACCEPTANCE.time.perf_counter() + 5.0
    while not bool(dict(session.snapshot()).get("is_finished")):
        session.heartbeat()
        if ACCEPTANCE.time.perf_counter() >= deadline:
            session.panic_release()
            session.quit()
            session.join(timeout_ms=5_000)
            raise AssertionError(f"fault mode did not finish: {fault_mode}")
        ACCEPTANCE.time.sleep(0.001)
    assert session.join(timeout_ms=5_000) is True
    snapshot = dict(session.snapshot())
    assert snapshot["outcome"] == expected_outcome
    session.take_telemetry_json()
    if counter is None:
        assert snapshot["sendinput_partial_events"] == 0
        assert snapshot["sendinput_zero_progress_failures"] == 0
        assert snapshot["chords_rejected"] == 0
        assert snapshot["authored_keys_rejected"] == 0
    else:
        assert int(snapshot[counter]) > 0


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
