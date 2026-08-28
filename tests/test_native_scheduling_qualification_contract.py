from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType
from typing import Any


def _load_acceptance_module() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "bench_native_acceptance.py"
    spec = importlib.util.spec_from_file_location(
        "native_scheduling_qualification_under_test", path
    )
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


ACCEPTANCE = _load_acceptance_module()


def _run(*, requested: str, acquired: str) -> dict[str, Any]:
    return {
        "requested_rt_priority_mode": requested,
        "acquired_priority": acquired,
        "requested_wait_policy": "production_calibrated",
        "effective_wait_policy": "production_calibrated",
        "spin_threshold_source": "production_startup_calibration",
        "startup_calibration_executed": True,
        "startup_calibration_sample_count": 6,
        "startup_wake_error_p50_us": 100,
        "startup_wake_error_p95_us": 200,
        "startup_wake_error_p99_us": 300,
        "startup_wake_error_max_us": 350,
        "startup_wake_error_robust_us": 400,
        "effective_spin_threshold_us": 450,
        "runtime_provenance": {
            "calibration_provenance_valid": True,
            "legacy_provenance_valid": False,
        },
        "correctness": {},
        "missed_down_boundaries": 0,
        "pre_call_hold_shrink_over_grace_count": 0,
        "_metric_rows": {"pre_call_lateness_us": [("down", 0)]},
    }


def _qualification(*, requested: str, acquired: str) -> dict[str, Any]:
    return ACCEPTANCE._scheduling_qualification(
        [_run(requested=requested, acquired=acquired)],
        backend="mock",
        acceptance_clean=True,
        physical_boundaries=10_000,
    )


def test_auto_mmcss_games_is_production_priority_qualified() -> None:
    result = _qualification(requested="auto", acquired="mmcss:Games")
    assert result["status"] == "qualified"
    assert result["priority_qualification"] is True
    assert result["statistics_eligible"] is True


def test_auto_highest_fallback_is_production_priority_qualified() -> None:
    result = _qualification(requested="auto", acquired="thread:highest")
    assert result["status"] == "qualified"
    assert result["priority_qualification"] is True
    assert result["statistics_eligible"] is True


def test_auto_off_remains_inconclusive() -> None:
    result = _qualification(requested="auto", acquired="off")
    assert result["status"] == "inconclusive_priority_fallback"
    assert result["priority_qualification"] is False
    assert result["statistics_eligible"] is False


def test_forced_highest_cannot_be_relabelled_as_production_equivalent() -> None:
    result = _qualification(requested="highest", acquired="thread:highest")
    assert result["status"] == "inconclusive_nonproduction_priority_request"
    assert result["priority_qualification"] is False
    assert result["statistics_eligible"] is False


def test_forced_time_critical_cannot_be_relabelled_as_production_equivalent() -> None:
    result = _qualification(
        requested="time_critical", acquired="thread:time_critical"
    )
    assert result["status"] == "inconclusive_nonproduction_priority_request"
    assert result["priority_qualification"] is False
    assert result["statistics_eligible"] is False


def test_unexpected_active_auto_acquisition_is_not_production_qualified() -> None:
    result = _qualification(requested="auto", acquired="thread:time_critical")
    assert result["status"] == "inconclusive_nonproduction_priority_acquisition"
    assert result["priority_qualification"] is False
    assert result["statistics_eligible"] is False


def test_runtime_provenance_requires_auto_ladder_for_production_wait() -> None:
    snapshot = {
        "requested_rt_priority_mode": "highest",
        "rt_priority_acquired": "thread:highest",
        "requested_wait_policy": "production_calibrated",
        "effective_wait_policy": "production_calibrated",
        "startup_calibration_executed": True,
        "startup_calibration_sample_count": 6,
        "startup_wake_error_p50_us": 100,
        "startup_wake_error_p95_us": 200,
        "startup_wake_error_p99_us": 300,
        "startup_wake_error_max_us": 350,
        "startup_wake_error_robust_us": 400,
        "effective_spin_threshold_us": 450,
        "spin_threshold_source": "production_startup_calibration",
    }
    provenance = ACCEPTANCE._runtime_provenance(
        snapshot,
        backend="mock",
        requested_wait_policy="production_calibrated",
    )
    assert provenance["priority_active"] is True
    assert provenance["production_priority_valid"] is False
    assert provenance["production_wait_qualification"] is False
