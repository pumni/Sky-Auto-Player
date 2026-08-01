from __future__ import annotations

import json
from pathlib import Path

import pytest

from sky_music.platform.win32 import native_calibration


def _native_result(*, clean: int = 64, cleanup_success: bool = True) -> dict[str, object]:
    def bucket() -> dict[str, object]:
        return {
            "attempted": clean,
            "clean_sample_count": clean,
            "clean": clean,
            "rejected": 0,
            "sample_count": clean,
            "partial_send": 0,
            "error_count": 0,
            "timeout_count": 0,
            "anomaly_count": 0,
            "class_mismatch_count": 0,
            "first_receipt_us": {"p50": 10, "p90": 20, "p95": 30, "p99": 40},
        }

    measured = clean * 4
    setup_attempted = 20
    warmup_attempted = 10
    return {
        "version": 6,
        "measurement_protocol_version": 3,
        "evidence_kind": "injected_raw_input_delivery_proxy",
        "source_git_sha": "test-sha",
        "native_build_id": "test-sha",
        "dirty_worktree": False,
        "native_source_fingerprint": "test-fingerprint",
        "rustc_version": "rustc 1.97.1",
        "host_fingerprint": {"qpc_frequency_hz": 10_000_000},
        "configuration": {
            "polyphonies": [1],
            "samples_per_hot_bucket": clean,
            "samples_per_cold_bucket": clean,
            "warmup_samples": warmup_attempted,
            "receipt_timeout_ms": 200,
            "hot_gap_target_us": 5_000,
            "cold_idle_gap_us": 100_000,
            "cold_threshold_us": 20_000,
        },
        "warmup_attempted": warmup_attempted,
        "measured_attempted": measured,
        "setup_attempted": setup_attempted,
        "measured_anomalous": 0,
        "warmup_anomalous": 1,
        "setup_anomalous": 0,
        "measured_class_mismatch": 0,
        "total_anomalous": 1,
        "warmup_timed_out": 0,
        "measured_timed_out": 0,
        "setup_timed_out": 0,
        "total_attempted": warmup_attempted + measured + setup_attempted,
        "total_timed_out": 0,
        "buckets": {
            "down": {"1": {"hot": bucket(), "cold": bucket()}},
            "up": {"1": {"hot": bucket(), "cold": bucket()}},
        },
        "cleanup": {
            "cleanup_success": cleanup_success,
            "cleanup_verification_inconclusive": False,
            "raw_input_restore_failed": False,
            "pump_thread_failed": False,
        },
    }


class _FakePopen:
    def __init__(self, _args, *, stdout, stderr, **_kwargs):
        stdout.write(json.dumps(self.result).encode("utf-8"))
        stderr.write(b"diagnostic on stderr\n")

    result: dict[str, object] = {}
    return_code = 0

    def wait(self, *, timeout: float) -> int:
        assert timeout > 0
        return self.return_code

    def kill(self) -> None:
        self.return_code = -9


def test_native_calibration_writes_cache_only_after_valid_clean_result(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    result = _native_result()
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    _FakePopen.result = result
    monkeypatch.setattr(native_calibration.subprocess, "Popen", _FakePopen)

    artifact = tmp_path / "calibration-native.json"
    cache = tmp_path / "input_latency.json"
    returned = native_calibration.run_native_calibration(
        output_path=artifact, cache_path=cache
    )

    assert returned == result
    assert json.loads(artifact.read_text(encoding="utf-8")) == result
    legacy = json.loads(cache.read_text(encoding="utf-8"))
    assert legacy["evidence_kind"] == result["evidence_kind"]
    assert legacy["n"] == 64
    assert legacy["down_us"] == {"p50": 10, "p90": 20, "p99": 40}


@pytest.mark.parametrize(
    "result",
    [
        _native_result(clean=1),
        _native_result(cleanup_success=False),
    ],
)
def test_native_calibration_rejects_untrusted_result_without_mutating_cache(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    result: dict[str, object],
) -> None:
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    _FakePopen.result = result
    monkeypatch.setattr(native_calibration.subprocess, "Popen", _FakePopen)
    cache = tmp_path / "input_latency.json"
    cache.write_text("sentinel\n", encoding="utf-8")

    with pytest.raises(native_calibration.NativeCalibrationError):
        native_calibration.run_native_calibration(
            output_path=tmp_path / "raw.json", cache_path=cache
        )

    assert cache.read_text(encoding="utf-8") == "sentinel\n"


def test_native_calibration_nonzero_process_exit_is_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    class _FailedPopen(_FakePopen):
        return_code = 7

        def __init__(self, _args, *, stdout, stderr, **_kwargs):
            stderr.write(b"cleanup failed")

    monkeypatch.setattr(native_calibration.subprocess, "Popen", _FailedPopen)

    with pytest.raises(native_calibration.NativeCalibrationError, match="7"):
        native_calibration.run_native_calibration(
            output_path=tmp_path / "raw.json", cache_path=tmp_path / "cache.json"
        )


@pytest.mark.parametrize("timeout", [float("nan"), float("inf"), float("-inf"), 0, -1])
def test_native_calibration_rejects_nonfinite_or_nonpositive_timeout(timeout: float) -> None:
    with pytest.raises(native_calibration.NativeCalibrationError, match="finite positive"):
        native_calibration.run_native_calibration(timeout_seconds=timeout)


def test_native_calibration_rejects_legacy_schema_without_mutating_cache(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    result = _native_result()
    result["version"] = 4
    _FakePopen.result = result
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(native_calibration.subprocess, "Popen", _FakePopen)
    cache = tmp_path / "input_latency.json"
    cache.write_text("sentinel\n", encoding="utf-8")

    with pytest.raises(native_calibration.NativeCalibrationError, match="schema"):
        native_calibration.run_native_calibration(cache_path=cache)

    assert cache.read_text(encoding="utf-8") == "sentinel\n"


@pytest.mark.parametrize(
    "mutate",
    [
        lambda result: result["buckets"]["down"]["1"].pop("cold"),
        lambda result: result["buckets"]["up"].pop("1"),
        lambda result: result["buckets"]["down"]["1"].update({"warm": {}}),
        lambda result: result.__setitem__("measured_attempted", 999),
        lambda result: result.__setitem__("measured_timed_out", 1),
        lambda result: result.__setitem__("measured_class_mismatch", 1),
        lambda result: result["buckets"]["down"]["1"]["hot"].__setitem__(
            "sample_count", 0
        ),
        lambda result: result.__setitem__("dirty_worktree", True),
        lambda result: result.__setitem__("native_build_id", "other-sha"),
    ],
    ids=[
        "missing-cold",
        "missing-polyphony",
        "extra-class",
        "attempt-total-mismatch",
        "timeout-total-mismatch",
        "class-mismatch-total-mismatch",
        "sample-count-mismatch",
        "dirty-artifact",
        "source-build-sha-mismatch",
    ],
)
def test_native_calibration_rejects_incomplete_or_untrusted_evidence(
    mutate,
) -> None:
    result = _native_result()
    mutate(result)
    with pytest.raises(native_calibration.NativeCalibrationError):
        native_calibration._validate_result(result)


def test_native_calibration_accepts_complete_matrix() -> None:
    result = _native_result()
    assert native_calibration._validate_result(result) == result
