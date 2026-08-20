from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any, cast

import pytest

from sky_music.infrastructure import calibration_loader as loader
from sky_music.platform.win32 import native_calibration


def _quantiles(p99: int = 6) -> dict[str, int]:
    if p99 < 0:
        return {"min": p99 - 6, "p50": p99 - 4, "p90": p99 - 3, "p95": p99 - 2, "p99": p99, "max": p99 + 2, "mean": p99 - 3}
    return {
        "min": -4,
        "p50": -1,
        "p90": 1,
        "p95": 3,
        "p99": p99,
        "max": p99 + 2,
        "mean": 0,
    }


def _host_fingerprint() -> dict[str, object]:
    return {
        "host_fingerprint_version": 2,
        "qpc_frequency_hz": 10_000_000,
        "win32_build": "Windows 11 test",
        "processor_architecture": "AMD64",
        "cpu_vendor": "GenuineIntel",
        "cpu_family": 6,
        "cpu_model": 183,
        "cpu_stepping": 1,
        "logical_processor_count": 16,
        "processor_group_count": 1,
        "cpu_set_efficiency_classes": [8, 8, 16, 16],
        "highest_efficiency_class": 16,
        "lowest_efficiency_class": 8,
        "sampled_at_us": 123,
    }


def _scheduling_aids() -> dict[str, object]:
    return {
        "mmcss_acquired": "mmcss:Games",
        "mmcss_active": True,
        "power_throttling_active": True,
        "waiter_mode": "event+high_resolution_timer",
    }


def _bucket(*, clean: int = 100, attempted: int | None = None, p99: int = 6) -> dict[str, object]:
    total = clean if attempted is None else attempted
    rejected = total - clean
    return {
        "attempted": total,
        "clean": clean,
        "clean_sample_count": clean,
        "rejected": rejected,
        "sample_count": total,
        "timeout_count": 0,
        "anomaly_count": 0,
        "class_mismatch_count": 0,
        "partial_send": 0,
        "down_call_duration_us": {"min": 1, "p50": 1, "p90": 2, "p95": 2, "p99": 3, "max": 3, "mean": 1},
        "up_call_duration_us": {"min": 1, "p50": 1, "p90": 2, "p95": 2, "p99": 3, "max": 3, "mean": 1},
        "pair_sender_hold_shrink_us": _quantiles(p99),
        "scheduler_shrink_us": _quantiles(6),
        "sendinput_shrink_us": _quantiles(6),
    }


def _configuration(*, polyphonies: list[int], samples: int = 100, budget: int = 120) -> dict[str, object]:
    return {
        "polyphonies": polyphonies,
        "samples_per_hot_bucket": samples,
        "samples_per_cold_bucket": samples,
        "warmup_samples": 4,
        "hot_gap_target_us": 5_000,
        "cold_threshold_us": 20_000,
        "cold_idle_gap_us": 25_000,
        "budget_seconds": budget,
    }


def _pair_bucket_result(*, polyphony: int, class_name: str, samples: int = 100) -> dict[str, object]:
    return {
        "version": 14,
        "measurement_protocol_version": 10,
        "evidence_kind": "sender_completion_hold_shrink",
        "source_git_sha": "test-sha",
        "native_build_id": "test-sha",
        "dirty_worktree": False,
        "native_source_fingerprint": "test-fingerprint",
        "rustc_version": "rustc test",
        "host_fingerprint": _host_fingerprint(),
        "scheduling_aids": _scheduling_aids(),
        "configuration": _configuration(polyphonies=[polyphony], samples=samples),
        "class": class_name,
        "polyphony": polyphony,
        "attempted_pairs": samples,
        "warmup_pairs": 4,
        "warmup_rejected": 0,
        "pair_bucket": _bucket(clean=samples),
        "worst_pairs": [],
        "anomalous_pairs": [],
        "cleanup": {
            "cleanup_success": True,
            "cleanup_verification_inconclusive": False,
            "raw_input_restore_failed": False,
            "pump_thread_failed": False,
        },
    }


def _native_result(*, p99_by_key: dict[str, int] | None = None) -> dict[str, object]:
    p99_by_key = p99_by_key or {}
    pair_buckets = {
        str(polyphony): {
            class_name: _bucket(p99=p99_by_key.get(f"{polyphony}/{class_name}", 6))
            for class_name in ("hot", "cold")
        }
        for polyphony in (1, 5, 15)
    }
    return {
        "version": 14,
        "measurement_protocol_version": 10,
        "evidence_kind": "sender_completion_hold_shrink",
        "source_git_sha": "test-sha",
        "native_build_id": "test-sha",
        "dirty_worktree": False,
        "native_source_fingerprint": "test-fingerprint",
        "rustc_version": "rustc test",
        "host_fingerprint": _host_fingerprint(),
        "scheduling_aids": _scheduling_aids(),
        "configuration": _configuration(polyphonies=[1, 5, 15]),
        "pair_buckets": pair_buckets,
        "anomalous_pairs": {
            str(polyphony): {class_name: [] for class_name in ("hot", "cold")}
            for polyphony in (1, 5, 15)
        },
        "measured_attempted": 600,
        "setup_attempted": 0,
        "setup_anomalous": 0,
        "setup_timed_out": 0,
        "warmup_attempted": 24,
        "warmup_anomalous": 0,
        "warmup_timed_out": 0,
        "measured_anomalous": 0,
        "measured_timed_out": 0,
        "measured_class_mismatch": 0,
        "total_attempted": 624,
        "total_anomalous": 0,
        "total_timed_out": 0,
        "cleanup": {
            "cleanup_success": True,
            "cleanup_verification_inconclusive": False,
            "raw_input_restore_failed": False,
            "pump_thread_failed": False,
        },
    }


def test_protocol_vnext_native_result_accepts_signed_pair_matrix() -> None:
    result = native_calibration._validate_result(_native_result())
    assert result["version"] == 14
    assert result["measurement_protocol_version"] == 10


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("version", 12),
        ("version", 13),
        ("measurement_protocol_version", 7),
        ("measurement_protocol_version", 8),
    ],
)
def test_previous_native_schema_or_protocol_is_rejected(
    field: str, value: int
) -> None:
    result = _native_result()
    result[field] = value
    with pytest.raises(native_calibration.NativeCalibrationError):
        native_calibration._validate_result(result)


def test_native_result_requires_scheduling_aid_provenance() -> None:
    result = _native_result()
    del result["scheduling_aids"]
    with pytest.raises(native_calibration.NativeCalibrationError, match="scheduling_aids"):
        native_calibration._validate_result(result)


def test_native_result_reports_insufficient_clean_pairs_before_null_quantiles() -> None:
    result = _native_result()
    bucket = cast(dict[str, object], result["pair_buckets"]["5"]["cold"])  # type: ignore[index]
    bucket.update(
        {
            "clean": 0,
            "clean_sample_count": 0,
            "rejected": 100,
            "anomaly_count": 100,
            "partial_send": 0,
            "sample_count": 100,
            "pair_sender_hold_shrink_us": None,
        }
    )

    with pytest.raises(
        native_calibration.NativeCalibrationError,
        match=r"5/cold.*insufficient clean pairs.*clean=0.*rejected=100.*class_mismatch_count=0.*anomaly_count=100",
    ):
        native_calibration._validate_result(result)


@pytest.mark.parametrize(
    "counter",
    [
        "timeout_count",
        "class_mismatch_count",
        "partial_send",
    ],
)
def test_native_result_rejects_diagnostic_counter_above_anomaly_count(counter: str) -> None:
    result = _native_result()
    bucket = cast(dict[str, object], result["pair_buckets"]["5"]["hot"])  # type: ignore[index]
    bucket.update(
        {"clean": 98, "clean_sample_count": 98, "rejected": 2, "anomaly_count": 1}
    )
    bucket[counter] = 2
    with pytest.raises(native_calibration.NativeCalibrationError, match="exceeds anomaly_count"):
        native_calibration._validate_result(result)


@pytest.mark.parametrize(
    "counter",
    [
        "timeout_count",
        "partial_send",
    ],
)
def test_publishable_bucket_allows_only_class_mismatch_rejections(counter: str) -> None:
    bucket = _bucket(clean=100, attempted=101)
    bucket["anomaly_count"] = 1
    bucket["class_mismatch_count"] = 0
    bucket[counter] = 1
    with pytest.raises(native_calibration.NativeCalibrationError, match="publishable"):
        native_calibration._validate_pair_bucket(bucket, "5/hot", 100)


def test_publishable_bucket_accepts_bounded_class_mismatch_rejection() -> None:
    bucket = _bucket(clean=100, attempted=101)
    bucket["anomaly_count"] = 1
    bucket["class_mismatch_count"] = 1
    native_calibration._validate_pair_bucket(bucket, "5/hot", 100)


def test_native_signed_and_unsigned_means_are_bounded_and_ordered() -> None:
    signed = _quantiles()
    signed["mean"] = 999_999
    with pytest.raises(native_calibration.NativeCalibrationError, match="quantile"):
        native_calibration._validate_quantiles(signed, "signed")

    unsigned = {"min": 1, "p50": 1, "p90": 2, "p95": 2, "p99": 3, "max": 3, "mean": 999_999}
    with pytest.raises(native_calibration.NativeCalibrationError, match="quantile"):
        native_calibration._validate_unsigned_quantiles(unsigned, "unsigned")


def test_cache_signed_mean_must_lie_between_min_and_max() -> None:
    cache = native_calibration._cache_v6(_native_result())
    cache["pair_buckets"]["1/hot"]["pair_sender_hold_shrink_us"]["mean"] = 999_999  # type: ignore[index]
    with pytest.raises(ValueError, match="ordered"):
        loader.parse_calibration_cache_summary(cache)


def test_cache_rejects_native_schema_13_under_protocol_10() -> None:
    current_cache = native_calibration._cache_v6(_native_result())
    previous_cache = json.loads(json.dumps(current_cache))
    previous_cache["native_calibration_version"] = 13

    loader.parse_calibration_cache_summary(current_cache)
    with pytest.raises(ValueError, match="native calibration schema"):
        loader.parse_calibration_cache_summary(previous_cache)


def test_cache_rejects_native_schema_before_diagnostic_compatibility_floor() -> None:
    cache = native_calibration._cache_v6(_native_result())
    cache["native_calibration_version"] = 12
    with pytest.raises(ValueError, match="native calibration schema"):
        loader.parse_calibration_cache_summary(cache)


def test_native_result_requires_unsigned_call_duration_quantiles() -> None:
    result = _native_result()
    bucket = cast(dict[str, object], result["pair_buckets"]["5"]["hot"])  # type: ignore[index]
    duration = cast(dict[str, int], bucket["down_call_duration_us"])
    duration["min"] = -1
    with pytest.raises(native_calibration.NativeCalibrationError, match="down_call_duration_us"):
        native_calibration._validate_result(result)


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("waiter_mode", "event+timer_resolution_fallback"),
        ("waiter_mode", "timer_resolution_fallback"),
        ("waiter_mode", "high_resolution_timer"),
        ("mmcss_acquired", "thread:time_critical"),
    ],
)
def test_publishable_calibration_rejects_degraded_scheduling_aids(
    field: str, value: str
) -> None:
    result = _native_result()
    cast(dict[str, object], result["scheduling_aids"])[field] = value
    with pytest.raises(native_calibration.NativeCalibrationError, match="publishable"):
        native_calibration._validate_result(result)

    cache = native_calibration._cache_v6(_native_result())
    cast(dict[str, object], cache["scheduling_aids"])[field] = value
    with pytest.raises(ValueError, match="publishable"):
        loader.parse_calibration_cache_summary(cache)
    resolution = loader.load_calibration_resolution(data=cache)
    assert resolution.status is loader.CalibrationStatus.INVALID_CACHE
    assert resolution.resolved_margin_us == 500


def test_native_result_rejects_missing_required_bucket() -> None:
    result = _native_result()
    del result["pair_buckets"]["15"]["cold"]  # type: ignore[index]
    with pytest.raises(native_calibration.NativeCalibrationError, match="incomplete"):
        native_calibration._validate_result(result)


def test_native_result_rejects_schema_v1() -> None:
    result = _native_result()
    result["version"] = 8
    with pytest.raises(native_calibration.NativeCalibrationError, match="schema"):
        native_calibration._validate_result(result)


def test_native_pair_bucket_command_has_no_directional_kind(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    payload = _pair_bucket_result(polyphony=5, class_name="cold")
    captured: dict[str, object] = {}

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        captured["command"] = command
        return subprocess.CompletedProcess(command, 0, json.dumps(payload), "")

    monkeypatch.setattr(native_calibration.subprocess, "run", run)
    result = native_calibration._execute_native_bucket(
        tmp_path / "native.exe",
        class_name="cold",
        polyphony=5,
        samples=100,
        warmup_samples=4,
        budget_seconds=120,
        timeout_seconds=120.0,
        progress=False,
    )
    assert result == payload
    assert "--kind" not in captured["command"]  # type: ignore[operator]


def test_native_stdout_is_bounded_before_json_parsing(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda command, **kwargs: subprocess.CompletedProcess(
            command,
            0,
            "x" * (native_calibration.MAX_NATIVE_CALIBRATION_STDOUT_BYTES + 1),
            "",
        ),
    )
    with pytest.raises(native_calibration.NativeCalibrationError, match="stdout exceeds"):
        native_calibration._execute_native_bucket(
            tmp_path / "native.exe",
            class_name="hot",
            polyphony=5,
            samples=100,
            warmup_samples=4,
            budget_seconds=120,
            timeout_seconds=120.0,
            progress=False,
        )


def test_native_result_rejects_unbounded_anomaly_evidence() -> None:
    result = _native_result()
    entries = cast(dict[str, object], result["anomalous_pairs"]["5"])  # type: ignore[index]
    entries["hot"] = [{} for _ in range(65)]
    with pytest.raises(native_calibration.NativeCalibrationError, match="bounded"):
        native_calibration._validate_result(result)


@pytest.mark.parametrize("attempted", [100, 101, 200])
def test_native_pair_bucket_accepts_bounded_extra_attempts_for_clean_target(
    attempted: int, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    payload = _pair_bucket_result(polyphony=5, class_name="cold")
    payload["attempted_pairs"] = attempted
    bucket = cast(dict[str, object], payload["pair_bucket"])
    bucket.update(
        {
            "attempted": attempted,
            "sample_count": attempted,
            "clean": 100,
            "clean_sample_count": 100,
            "rejected": attempted - 100,
            "anomaly_count": attempted - 100,
            "class_mismatch_count": attempted - 100,
        }
    )

    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda command, **kwargs: subprocess.CompletedProcess(
            command, 0, json.dumps(payload), ""
        ),
    )
    result = native_calibration._execute_native_bucket(
        tmp_path / "native.exe",
        class_name="cold",
        polyphony=5,
        samples=100,
        warmup_samples=4,
        budget_seconds=120,
        timeout_seconds=120.0,
        progress=False,
    )
    assert result["attempted_pairs"] == attempted


def test_native_pair_bucket_rejects_attempts_above_bounded_retry_limit(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    payload = _pair_bucket_result(polyphony=5, class_name="cold")
    payload["attempted_pairs"] = 201
    bucket = cast(dict[str, object], payload["pair_bucket"])
    bucket.update(
        {
            "attempted": 201,
            "sample_count": 201,
            "clean": 100,
            "clean_sample_count": 100,
            "rejected": 101,
        }
    )
    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda command, **kwargs: subprocess.CompletedProcess(
            command, 0, json.dumps(payload), ""
        ),
    )

    with pytest.raises(native_calibration.NativeCalibrationError, match="attempt count"):
        native_calibration._execute_native_bucket(
            tmp_path / "native.exe",
            class_name="cold",
            polyphony=5,
            samples=100,
            warmup_samples=4,
            budget_seconds=120,
            timeout_seconds=120.0,
            progress=False,
        )


def test_native_pair_bucket_rejects_more_clean_pairs_than_target(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    payload = _pair_bucket_result(polyphony=5, class_name="cold")
    payload["attempted_pairs"] = 101
    bucket = cast(dict[str, object], payload["pair_bucket"])
    bucket.update(
        {
            "attempted": 101,
            "sample_count": 101,
            "clean": 101,
            "clean_sample_count": 101,
            "rejected": 0,
        }
    )
    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda command, **kwargs: subprocess.CompletedProcess(
            command, 0, json.dumps(payload), ""
        ),
    )

    with pytest.raises(native_calibration.NativeCalibrationError, match="clean pair count"):
        native_calibration._execute_native_bucket(
            tmp_path / "native.exe",
            class_name="cold",
            polyphony=5,
            samples=100,
            warmup_samples=4,
            budget_seconds=120,
            timeout_seconds=120.0,
            progress=False,
        )


def test_directional_execution_is_rejected() -> None:
    with pytest.raises(native_calibration.NativeCalibrationError, match="directional"):
        native_calibration._execute_native_bucket(
            Path("native.exe"),
            kind="down",
            class_name="hot",
            polyphony=1,
            samples=100,
            warmup_samples=4,
            budget_seconds=120,
            timeout_seconds=120.0,
            progress=False,
        )


def test_cache_v6_uses_max_positive_sender_p99_and_preserves_signed_values() -> None:
    result = _native_result(p99_by_key={"15/cold": 42, "5/hot": -8})
    cache = native_calibration._cache_v6(result)
    summary = loader.parse_calibration_cache_summary(cache)
    assert cache["version"] == 6
    assert summary.sender_hold_shrink_p99_us == 42
    assert summary.worst_bucket == "15/cold"
    assert summary.margin_us == 300
    assert summary.candidate_margin_us == 142
    assert summary.pair_buckets["5/hot"].pair_sender_hold_shrink_us.p99 == -8


def test_cache_v1_is_rejected_and_falls_back() -> None:
    old = {"version": 1, "n": 20, "down_us": {"p50": 1, "p90": 2, "p99": 3}, "up_us": {"p50": 1, "p90": 2, "p99": 3}}
    with pytest.raises(ValueError):
        loader.parse_calibration_cache_summary(old)


def test_previous_measurement_protocol_is_not_reinterpreted() -> None:
    cache = native_calibration._cache_v6(_native_result())
    cache["measurement_protocol_version"] = 5
    with pytest.raises(ValueError, match="measurement protocol"):
        loader.parse_calibration_cache_summary(cache)
    resolution = loader.load_calibration_resolution(data=cache)
    assert resolution.status is loader.CalibrationStatus.INVALID_CACHE
    assert resolution.resolved_margin_us == 500


def test_protocol9_cache_is_rejected_after_protocol10_semantics_bump() -> None:
    cache = native_calibration._cache_v6(_native_result())
    cache["measurement_protocol_version"] = 9
    with pytest.raises(ValueError, match="measurement protocol"):
        loader.parse_calibration_cache_summary(cache)
    resolution = loader.load_calibration_resolution(data=cache)
    assert resolution.status is loader.CalibrationStatus.INCOMPATIBLE
    assert resolution.resolved_margin_us == 500


def test_cache_requires_100_clean_pairs_per_cell() -> None:
    cache = native_calibration._cache_v6(_native_result())
    cache["pair_buckets"]["1/hot"]["clean_pair_count"] = 99  # type: ignore[index]
    cache["pair_buckets"]["1/hot"]["rejected"] = 1  # type: ignore[index]
    with pytest.raises(ValueError, match="clean pairs"):
        loader.parse_calibration_cache_summary(cache)


@pytest.mark.parametrize(
    ("attempted", "clean", "accepted"),
    [(100, 100, True), (200, 100, True), (201, 100, False), (101, 101, False)],
)
def test_cache_enforces_protocol10_bounded_retry_invariant(
    attempted: int, clean: int, accepted: bool
) -> None:
    cache = native_calibration._cache_v6(_native_result())
    for bucket in cast(dict[str, dict[str, object]], cache["pair_buckets"]).values():
        bucket["attempted"] = attempted
        bucket["clean_pair_count"] = clean
        bucket["rejected"] = attempted - clean
    if accepted:
        summary = loader.parse_calibration_cache_summary(cache)
        assert summary.sample_count == 100
    else:
        with pytest.raises(ValueError):
            loader.parse_calibration_cache_summary(cache)


@pytest.mark.parametrize(
    ("p99", "candidate", "status", "margin"),
    [
        (42, 142, loader.CalibrationStatus.VALID, 300),
        (700, 800, loader.CalibrationStatus.VALID, 800),
        (1_900, 2_000, loader.CalibrationStatus.VALID, 2_000),
        (1_901, 2_001, loader.CalibrationStatus.OUT_OF_ENVELOPE, None),
        (12_088, 12_188, loader.CalibrationStatus.OUT_OF_ENVELOPE, None),
    ],
)
def test_qualification_boundaries(
    p99: int,
    candidate: int,
    status: loader.CalibrationStatus,
    margin: int | None,
) -> None:
    qualification = loader.qualify_calibration_margin(p99)
    assert qualification.candidate_margin_us == candidate
    assert qualification.status is status
    assert qualification.applied_margin_us == margin


def test_raw_input_diagnostic_delay_cannot_change_sender_qualification() -> None:
    """Receipt observations are not an input to the protocol-10 formula."""
    for receipt_delay_us in (0, 1_000, 13_000, 100_000):
        result = _native_result(
            p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 780)
        )
        for classes in cast(dict[str, dict[str, dict[str, object]]], result["pair_buckets"]).values():
            for bucket in classes.values():
                bucket["observer_diagnostics"] = {
                    "raw_input_receipt_hold_us": receipt_delay_us,
                    "wm_input_queue_delay_us": receipt_delay_us,
                }
        summary = loader.parse_calibration_cache_summary(
            native_calibration._cache_v6(result)
        )
        assert summary.sender_hold_shrink_p99_us == 780
        assert summary.candidate_margin_us == 880
        assert summary.margin_us == 880


def test_sender_qualification_does_not_sum_component_quantiles() -> None:
    result = _native_result(
        p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 100)
    )
    for classes in cast(dict[str, dict[str, dict[str, object]]], result["pair_buckets"]).values():
        for bucket in classes.values():
            bucket["scheduler_shrink_us"] = _quantiles(10_000)
            bucket["sendinput_shrink_us"] = _quantiles(10_000)
    summary = loader.parse_calibration_cache_summary(
        native_calibration._cache_v6(result)
    )
    assert summary.sender_hold_shrink_p99_us == 100
    assert summary.candidate_margin_us == 200
    assert summary.margin_us == 300


def test_old_raw_input_evidence_and_cache_v5_are_incompatible() -> None:
    result = _native_result()
    result["evidence_kind"] = "injected_raw_input_total_hold_proxy"
    with pytest.raises(native_calibration.NativeCalibrationError):
        native_calibration._validate_result(result)

    cache = native_calibration._cache_v6(_native_result())
    cache["version"] = 5
    with pytest.raises(ValueError):
        loader.parse_calibration_cache_summary(cache)
    resolution = loader.load_calibration_resolution(data=cache)
    assert resolution.status is loader.CalibrationStatus.INCOMPATIBLE
    assert resolution.resolved_margin_us == 500


def test_out_of_envelope_cache_keeps_candidate_and_no_applied_margin() -> None:
    result = _native_result(
        p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 12088)
    )
    cache = native_calibration._cache_v6(result)
    summary = loader.parse_calibration_cache_summary(cache)

    assert summary.margin_us is None
    assert summary.candidate_margin_us == 12_188
    assert summary.status is loader.CalibrationStatus.OUT_OF_ENVELOPE
    assert cache["qualification"]["applied_margin_us"] is None  # type: ignore[index]


@pytest.mark.parametrize(
    "field, value",
    [
        ("candidate_margin_us", 2_000),
        ("worst_bucket", "15/cold"),
        ("sender_hold_shrink_p99_us", 0),
    ],
)
def test_v4_rejects_tampered_qualification(field: str, value: object) -> None:
    cache = native_calibration._cache_v6(_native_result())
    cast(dict[str, object], cache["qualification"])[field] = value
    with pytest.raises(ValueError):
        loader.parse_calibration_cache_summary(cache)


def test_v4_rejects_out_of_envelope_saturated_applied_margin() -> None:
    cache = native_calibration._cache_v6(
        _native_result(
            p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 12088)
        )
    )
    cast(dict[str, object], cache["qualification"])["applied_margin_us"] = 2_000
    with pytest.raises(ValueError):
        loader.parse_calibration_cache_summary(cache)


def test_v4_rejects_wrong_status_and_valid_null_applied_margin() -> None:
    out_cache = native_calibration._cache_v6(
        _native_result(
            p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 12_088)
        )
    )
    out_cache["status"] = "valid"
    with pytest.raises(ValueError):
        loader.parse_calibration_cache_summary(out_cache)

    valid_cache = native_calibration._cache_v6(_native_result())
    cast(dict[str, object], valid_cache["qualification"])["applied_margin_us"] = None
    with pytest.raises(ValueError):
        loader.parse_calibration_cache_summary(valid_cache)


@pytest.mark.parametrize("legacy_version", [2, 3, 4])
def test_legacy_cache_is_rejected_without_reinterpreting_vnext(
    legacy_version: int,
) -> None:
    legacy = native_calibration._cache_v6(_native_result())
    legacy["version"] = legacy_version
    legacy["source_formula_version"] = 3
    with pytest.raises(ValueError, match=r"legacy|unsupported"):
        loader.parse_calibration_cache_summary(legacy)
    resolution = loader.load_calibration_resolution(data=legacy)
    assert resolution.status is loader.CalibrationStatus.INCOMPATIBLE
    assert resolution.resolved_margin_us == 500


def test_load_calibration_resolution_states(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(loader, "_current_host_fingerprint", _host_fingerprint)
    missing = loader.load_calibration_resolution(data=None, cache_path=Path("does-not-exist.json"))
    assert missing.status is loader.CalibrationStatus.UNCALIBRATED
    assert missing.resolved_margin_us == 500
    assert missing.margin_source == loader.SOURCE_DEFAULT_500

    corrupt = loader.load_calibration_resolution(data={"version": 1})
    assert corrupt.status is loader.CalibrationStatus.INVALID_CACHE
    assert corrupt.resolved_margin_us == 500
    assert corrupt.margin_source == loader.SOURCE_INVALID_CACHE_DEFAULT_500

    valid_cache = native_calibration._cache_v6(
        _native_result(p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 700))
    )
    valid = loader.load_calibration_resolution(data=valid_cache)
    assert valid.status is loader.CalibrationStatus.VALID
    assert valid.resolved_margin_us == 800
    assert valid.margin_source == loader.SOURCE_DEVICE_CACHE

    out_cache = native_calibration._cache_v6(
        _native_result(p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 12088))
    )
    out = loader.load_calibration_resolution(data=out_cache)
    assert out.status is loader.CalibrationStatus.OUT_OF_ENVELOPE
    assert out.resolved_margin_us == 500
    assert out.margin_source == loader.SOURCE_OUT_OF_ENVELOPE_DEFAULT_500


def test_host_identity_match_ignores_sampling_time_but_rejects_topology_change(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cache = native_calibration._cache_v6(_native_result())
    current = _host_fingerprint()
    monkeypatch.setattr(loader, "_current_host_fingerprint", lambda: current)
    cache["host_fingerprint"]["sampled_at_us"] = 999_999  # type: ignore[index]
    assert loader.load_calibration_resolution(data=cache).status is loader.CalibrationStatus.VALID

    changed = dict(current)
    changed["cpu_model"] = int(cast(int, changed["cpu_model"])) + 1
    monkeypatch.setattr(loader, "_current_host_fingerprint", lambda: changed)
    result = loader.load_calibration_resolution(data=cache)
    assert result.status is loader.CalibrationStatus.INCOMPATIBLE
    assert result.resolved_margin_us == 500


@pytest.mark.parametrize(
    "field",
    [
        "cpu_family",
        "cpu_stepping",
        "cpu_set_efficiency_classes",
        "win32_build",
        "qpc_frequency_hz",
        "host_fingerprint_version",
    ],
)
def test_each_required_host_identity_field_mismatch_is_incompatible(
    monkeypatch: pytest.MonkeyPatch,
    field: str,
) -> None:
    cache = native_calibration._cache_v6(_native_result())
    current = _host_fingerprint()
    changed = dict(current)
    value = changed[field]
    if isinstance(value, list):
        changed[field] = [8, 16, 16, 16]
    elif isinstance(value, str):
        changed[field] = f"changed-{value}"
    else:
        changed[field] = int(cast(int, value)) + 1

    monkeypatch.setattr(loader, "_current_host_fingerprint", lambda: changed)
    result = loader.load_calibration_resolution(data=cache)

    assert result.status is loader.CalibrationStatus.INCOMPATIBLE
    assert result.resolved_margin_us == 500


def test_diagnostic_run_writes_report_but_never_production_cache(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    payload = _pair_bucket_result(polyphony=1, class_name="hot", samples=5)
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(native_calibration, "_execute_native_bucket", lambda *args, **kwargs: payload)
    report = tmp_path / "diagnostic.json"
    cache = tmp_path / "input_latency.json"
    result = native_calibration.run_diagnostic_calibration(
        class_name="hot", polyphony=1, samples=5, output_path=report
    )
    assert result["acceptance_eligible"] is False
    assert result["version"] == 14
    assert result["measurement_protocol_version"] == 10
    assert result["artifact_schema_version"] == 11
    assert report.is_file()
    assert not cache.exists()


def test_invalid_quick_result_leaves_existing_cache_untouched(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    cache = tmp_path / "input_latency.json"
    cache.write_text("sentinel\n", encoding="utf-8")
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(native_calibration, "_run_process", lambda *args, **kwargs: {"version": 9})
    with pytest.raises(native_calibration.NativeCalibrationError):
        native_calibration.run_native_calibration(cache_path=cache)
    assert cache.read_text(encoding="utf-8") == "sentinel\n"


def test_completed_out_of_envelope_measurement_overwrites_existing_cache(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    cache = tmp_path / "input_latency.json"
    cache.write_text("old cache\n", encoding="utf-8")
    result = _native_result(
        p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 12_088)
    )
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(native_calibration, "_run_process", lambda *args, **kwargs: result)

    native_calibration.run_native_calibration(cache_path=cache)

    summary = loader.parse_calibration_cache_summary(
        json.loads(cache.read_text(encoding="utf-8"))
    )
    assert summary.status is loader.CalibrationStatus.OUT_OF_ENVELOPE
    assert summary.margin_us is None
    assert summary.candidate_margin_us == 12_188


def test_full_configuration_is_six_pair_buckets() -> None:
    assert native_calibration.calibration_bucket_keys() == [
        (1, "hot"), (1, "cold"), (5, "hot"), (5, "cold"), (15, "hot"), (15, "cold")
    ]
    config = native_calibration.full_orchestration_configuration()
    assert config["minimum_clean_pairs_per_bucket"] == 100
    assert "chunk_samples" not in config
    assert native_calibration.full_orchestration_configuration(
        global_budget_seconds=45
    )["global_budget_seconds"] == 45.0


def test_single_process_timeout_has_parent_and_native_exit_reserves() -> None:
    budget, child_timeout = native_calibration._single_process_budget(120.0)
    assert child_timeout == 119.0
    assert 120.0 - child_timeout == native_calibration.PYTHON_PROCESS_EXIT_RESERVE_SECONDS
    assert child_timeout - budget >= native_calibration.NATIVE_PROCESS_EXIT_RESERVE_SECONDS
    assert budget - native_calibration.NATIVE_CLEANUP_RESERVE_SECONDS >= 1


def test_full_child_budget_preserves_publication_and_exit_reserves() -> None:
    budget, child_timeout = native_calibration._native_child_budget(120.0, 0.0)
    assert 120.0 - child_timeout == native_calibration.PUBLICATION_RESERVE_SECONDS
    assert child_timeout - budget >= native_calibration.NATIVE_PROCESS_EXIT_RESERVE_SECONDS
    assert budget - native_calibration.NATIVE_CLEANUP_RESERVE_SECONDS >= 1


def test_timeout_hierarchy_rejects_budget_without_all_reserves() -> None:
    with pytest.raises(native_calibration.NativeCalibrationError, match="at least"):
        native_calibration._finite_timeout(
            native_calibration.MIN_SINGLE_PROCESS_TIMEOUT_SECONDS - 0.01,
            default=120.0,
        )
    with pytest.raises(native_calibration.NativeCalibrationError, match="process-exit"):
        native_calibration._native_child_budget(7.99, 0.0)


def test_checkpoint_sha256_sidecar_is_plain_text_round_trip(tmp_path: Path) -> None:
    artifact_path = tmp_path / "1-hot.json"
    native_calibration._write_json_atomically(artifact_path, {"key": "1/hot"})
    digest = hashlib.sha256(artifact_path.read_bytes()).hexdigest()
    sidecar = artifact_path.with_suffix(".sha256")

    native_calibration._write_text_atomically(sidecar, digest + "\n")

    assert sidecar.read_text(encoding="utf-8") == digest + "\n"
    assert sidecar.read_text(encoding="utf-8").strip() == digest


def _checkpoint_artifacts(tmp_path: Path) -> tuple[dict[str, dict[str, object]], Path]:
    orchestration = native_calibration.full_orchestration_configuration()
    artifacts: dict[str, dict[str, object]] = {}
    for polyphony, class_name in native_calibration.calibration_bucket_keys():
        key = f"{polyphony}/{class_name}"
        artifacts[key] = native_calibration._artifact_from_bucket(
            _pair_bucket_result(polyphony=polyphony, class_name=class_name),
            orchestration=orchestration,
            key=key,
        )
    provenance = native_calibration._provenance_identity(next(iter(artifacts.values())))
    checkpoint = tmp_path / "checkpoint"
    checkpoint.mkdir()
    for key, artifact in artifacts.items():
        polyphony, class_name = key.split("/")
        path = checkpoint / f"{polyphony}-{class_name}.json"
        native_calibration._write_json_atomically(path, artifact)
        native_calibration._write_text_atomically(
            path.with_suffix(".sha256"), hashlib.sha256(path.read_bytes()).hexdigest() + "\n"
        )
    native_calibration._write_json_atomically(
        checkpoint / "MANIFEST.json",
        {
            "orchestration_configuration": orchestration,
            "provenance": provenance,
            "buckets": sorted(artifacts),
        },
    )
    return artifacts, checkpoint


def test_finalizer_rejects_mixed_provenance_before_publication(tmp_path: Path) -> None:
    artifacts, checkpoint = _checkpoint_artifacts(tmp_path)
    tampered = dict(artifacts["15/cold"])
    tampered["rustc_version"] = "rustc mixed provenance"
    artifact_path = checkpoint / "15-cold.json"
    native_calibration._write_json_atomically(artifact_path, tampered)
    native_calibration._write_text_atomically(
        artifact_path.with_suffix(".sha256"), hashlib.sha256(artifact_path.read_bytes()).hexdigest() + "\n"
    )
    output = tmp_path / "published.json"
    cache = tmp_path / "input_latency.json"
    output.write_text("old output\n", encoding="utf-8")
    cache.write_text("old cache\n", encoding="utf-8")

    with pytest.raises(native_calibration.NativeCalibrationError, match="provenance"):
        native_calibration.finalize_native_calibration(
            checkpoint_dir=checkpoint, output_path=output, cache_path=cache
        )

    assert output.read_text(encoding="utf-8") == "old output\n"
    assert cache.read_text(encoding="utf-8") == "old cache\n"


@pytest.mark.parametrize("artifact_schema_version", [None, 9, 10])
def test_finalizer_rejects_wrong_artifact_schema_version(
    tmp_path: Path, artifact_schema_version: int | None
) -> None:
    artifacts, checkpoint = _checkpoint_artifacts(tmp_path)
    tampered = dict(artifacts["15/cold"])
    if artifact_schema_version is None:
        del tampered["artifact_schema_version"]
    else:
        tampered["artifact_schema_version"] = artifact_schema_version
    artifact_path = checkpoint / "15-cold.json"
    native_calibration._write_json_atomically(artifact_path, tampered)
    native_calibration._write_text_atomically(
        artifact_path.with_suffix(".sha256"), hashlib.sha256(artifact_path.read_bytes()).hexdigest() + "\n"
    )

    with pytest.raises(native_calibration.NativeCalibrationError, match="artifact schema"):
        native_calibration.finalize_native_calibration(
            checkpoint_dir=checkpoint,
            output_path=tmp_path / "published.json",
            cache_path=tmp_path / "input_latency.json",
        )


def test_finalizer_validates_cache_before_publishing(tmp_path: Path) -> None:
    _artifacts, checkpoint = _checkpoint_artifacts(tmp_path)
    output = tmp_path / "published.json"
    cache = tmp_path / "input_latency.json"
    final = native_calibration.finalize_native_calibration(
        checkpoint_dir=checkpoint, output_path=output, cache_path=cache
    )

    assert final["acceptance_eligible"] is True
    summary = loader.parse_calibration_cache_summary(json.loads(cache.read_text(encoding="utf-8")))
    assert summary.sample_count == 100
    assert json.loads(output.read_text(encoding="utf-8"))["source_git_sha"] == "test-sha"


def test_finalizer_ignores_only_observation_timestamp_differences(tmp_path: Path) -> None:
    artifacts, checkpoint = _checkpoint_artifacts(tmp_path)
    for index, (key, artifact) in enumerate(artifacts.items(), start=1):
        host = dict(cast(dict[str, Any], artifact["host_fingerprint"]))
        host["sampled_at_us"] = index
        artifact["host_fingerprint"] = host
        polyphony, class_name = key.split("/")
        path = checkpoint / f"{polyphony}-{class_name}.json"
        native_calibration._write_json_atomically(path, artifact)
        native_calibration._write_text_atomically(
            path.with_suffix(".sha256"), hashlib.sha256(path.read_bytes()).hexdigest() + "\n"
        )

    final = native_calibration.finalize_native_calibration(
        checkpoint_dir=checkpoint,
        output_path=tmp_path / "published.json",
        cache_path=tmp_path / "input_latency.json",
    )
    assert final["acceptance_eligible"] is True


def test_published_result_populates_effective_native_floor(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    result = _native_result()
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(native_calibration, "_run_process", lambda *args, **kwargs: result)

    published = native_calibration.run_published_native_calibration(
        output_path=tmp_path / "raw.json", cache_path=tmp_path / "cache.json", fps=60
    )

    assert published.effective_min_hold_us == 16_967
    assert published.status is loader.CalibrationStatus.VALID
    assert published.candidate_margin_us == 106

    stressed = _native_result(p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 700))
    monkeypatch.setattr(native_calibration, "_run_process", lambda *args, **kwargs: stressed)
    stressed_published = native_calibration.run_published_native_calibration(
        output_path=tmp_path / "stressed-raw.json",
        cache_path=tmp_path / "stressed-cache.json",
        fps=60,
        hold_frames=1.0,
    )
    assert stressed_published.margin_us == 800
    assert stressed_published.effective_min_hold_us == 17_467

    out = _native_result(
        p99_by_key=dict.fromkeys(native_calibration.REQUIRED_BUCKETS, 12088)
    )
    monkeypatch.setattr(native_calibration, "_run_process", lambda *args, **kwargs: out)
    out_published = native_calibration.run_published_native_calibration(
        output_path=tmp_path / "out-raw.json",
        cache_path=tmp_path / "out-cache.json",
        fps=60,
        hold_frames=1.0,
    )
    assert out_published.status is loader.CalibrationStatus.OUT_OF_ENVELOPE
    assert out_published.margin_us is None
    assert out_published.candidate_margin_us == 12_188
    assert out_published.effective_min_hold_us is None
