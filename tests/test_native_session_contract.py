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
        sky_player_rs.SessionConfig(game_fps=60, profile="mock_test")  # type: ignore[attr-defined]


def test_session_config_validates_target_and_exposes_user_fields() -> None:
    config = sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
        game_fps=120,
        min_hold_us=50_000,
        require_focus=True,
        target_hwnd=123,
        telemetry=True,
        profile="production",
    )
    assert config.min_hold_us == 50_000
    assert config.game_fps == 120
    assert config.require_focus is True
    assert config.target_hwnd == 123
    assert config.telemetry is True
    assert config.profile == "production"


@pytest.mark.parametrize("fps", [14, 241])
def test_session_config_rejects_out_of_range_game_fps(fps: int) -> None:
    with pytest.raises(ValueError, match="game_fps"):
        sky_player_rs.SessionConfig(game_fps=fps)  # type: ignore[attr-defined]


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
            game_fps=60,
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
        "active_count",
        "possibly_active_count",
        "failed_release_count",
        "last_error",
        "keys_dropped",
        "chord_split_events",
        "sendinput_partial_events",
        "sendinput_zero_progress_failures",
        "chords_rejected",
        "authored_conflict_events",
        "authored_chords_rejected",
        "authored_keys_rejected",
        "keys_inserted_before_failure",
        "keys_rolled_back",
        "rollback_residue_keys",
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


def test_health_mapping_rejects_missing_correctness_counter() -> None:
    snapshot = {
        "active_count": 0,
        "possibly_active_count": 0,
        "failed_release_count": 0,
        "last_error": None,
        "keys_dropped": 0,
        "chord_split_events": 0,
        "sendinput_partial_events": 0,
        "sendinput_zero_progress_failures": 0,
        "chords_rejected": 0,
        "authored_conflict_events": 0,
        "authored_chords_rejected": 0,
        "authored_keys_rejected": 0,
        "keys_inserted_before_failure": 0,
        "keys_rolled_back": 0,
    }
    from sky_music.orchestration.native_dispatch import (
        NativeDispatchError,
        RustDispatchRuntime,
    )

    with pytest.raises(NativeDispatchError, match="rollback_residue_keys"):
        RustDispatchRuntime._health(snapshot)
