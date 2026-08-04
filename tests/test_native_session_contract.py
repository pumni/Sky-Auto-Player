"""Contract tests for the small production PyO3 surface."""

from __future__ import annotations

import inspect

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]


def test_production_module_exposes_only_native_session_surface() -> None:
    names = {
        name
        for name in dir(sky_player_rs)
        if not name.startswith("_")
    }
    assert "DispatchSession" in names
    assert "SessionConfig" in names
    assert "build_info" in names
    assert "run_calibration_rs" not in names
    assert "calibration_schema_version" not in names
    assert "native_input_backend" not in names
    assert "Rust" + "InputBackend" not in names
    assert "Rust" + "InputAdapter" not in names


def test_session_config_rejects_test_profile() -> None:
    with pytest.raises(ValueError, match="profile must be"):
        sky_player_rs.SessionConfig(profile="mock_test")  # type: ignore[attr-defined]


def test_session_config_validates_target_and_exposes_user_fields() -> None:
    config = sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
        min_hold_us=50_000,
        require_focus=True,
        target_hwnd=123,
        telemetry=True,
        profile="production",
    )
    assert config.min_hold_us == 50_000
    assert config.require_focus is True
    assert config.target_hwnd == 123
    assert config.telemetry is True
    assert config.profile == "production"


def test_dispatch_constructor_does_not_accept_legacy_backend_knobs() -> None:
    signature = str(inspect.signature(sky_player_rs.DispatchSession))  # type: ignore[attr-defined]
    for retired in (
        "mock_backend",
        "mock_failure_mode",
        "mock_latency_base_us",
        "mock_latency_per_key_us",
        "telemetry_capacity",
        "rt_priority_mode",
    ):
        assert retired not in signature


def test_session_reports_lite_progress_then_one_final_report() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [],
        [21],
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            require_focus=False,
            telemetry=True,
        ),
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    live = dict(session.snapshot_lite())  # type: ignore[attr-defined]
    assert set(live) == {
        "state",
        "elapsed_us",
        "total_us",
        "max_completion_error_us",
        "active_keys",
        "health",
        "is_running",
        "is_finished",
        "is_paused",
        "input_path_degraded",
    }
    assert live["is_finished"] is True

    report = dict(session.session_report())  # type: ignore[attr-defined]
    assert set(report) == {"snapshot", "telemetry_json", "estimator_state_json"}
    assert dict(report["snapshot"])["is_finished"] is True
