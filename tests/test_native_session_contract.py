"""Contract tests for the small production PyO3 surface."""

from __future__ import annotations

import inspect

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]

from sky_music.layouts import SKY_15_SCAN_CODES


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
    with pytest.raises(
        ValueError,
        match=r"(profile must be '(production|strict_timing_diagnostic)'|mock_test is available only to Rust test support)",
    ):
        sky_player_rs.SessionConfig(game_fps=60, profile="mock_test")  # type: ignore[attr-defined]


def test_python_and_native_scan_code_registries_match() -> None:
    assert tuple(sky_player_rs.instrument_scan_codes()) == SKY_15_SCAN_CODES





def test_session_config_validates_target_and_exposes_user_fields() -> None:
    config = sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
        game_fps=120,
        min_hold_us=50_000,
        require_focus=True,
        focus_restore_grace_us=12_345,
        target_hwnd=123,
        telemetry=True,
        profile="production",
    )
    assert config.min_hold_us == 50_000
    assert config.game_fps == 120
    assert config.require_focus is True
    assert config.focus_restore_grace_us == 12_345
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


def test_dispatch_constructor_rejects_the_removed_external_allowlist() -> None:
    with pytest.raises(TypeError):
        sky_player_rs.DispatchSession([], list(SKY_15_SCAN_CODES))  # type: ignore[attr-defined]


def test_native_constructor_rejects_same_key_hold_below_effective_floor() -> None:
    with pytest.raises(ValueError, match="same-key hold too short"):
        sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [
                (0, "down", 0, [SKY_15_SCAN_CODES[0]], "down"),
                (1, "up", 100, [SKY_15_SCAN_CODES[0]], "up"),
            ],
            config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
                game_fps=240,
                min_hold_us=100,
            ),
        )


def test_native_constructor_accepts_exact_effective_hold_boundary() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [SKY_15_SCAN_CODES[0]], "down"),
            (1, "up", 4_667, [SKY_15_SCAN_CODES[0]], "up"),
        ],
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=240,
            min_hold_us=4_667,
        ),
    )
    assert session is not None


def test_session_reports_lite_progress_then_one_final_report() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [],
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=60,
            min_hold_us=100,
            require_focus=False,
            focus_restore_grace_us=1_234,
            telemetry=True,
        ),
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    live = session.snapshot_lite()  # type: ignore[attr-defined]
    assert isinstance(live, sky_player_rs.ProgressSnapshot)  # type: ignore[attr-defined]
    assert isinstance(live.backend_health, sky_player_rs.BackendHealthSnapshot)  # type: ignore[attr-defined]
    assert live.is_finished is True
    with pytest.raises(AttributeError):
        live.is_finished = False
    report = dict(session.session_report())  # type: ignore[attr-defined]
    assert set(report) == {
        "snapshot",
        "effective_config",
        "telemetry_json",
        "estimator_state_json",
    }
    assert dict(report["snapshot"])["is_finished"] is True
    assert report["effective_config"] == {
        "game_fps": 60,
        "requested_min_hold_us": 100,
        "effective_min_hold_us": 17_167,
        "require_focus": False,
        "focus_restore_grace_us": 1_234,
        "telemetry_mode": "ring",
        "profile": "production",
    }


def test_health_mapping_rejects_missing_correctness_counter() -> None:
    from types import SimpleNamespace
    from typing import Any, cast

    snapshot = SimpleNamespace(
        active_count=0,
        possibly_active_count=0,
        failed_release_count=0,
        last_error=None,
        keys_dropped=0,
        chord_split_events=0,
        sendinput_partial_events=0,
        sendinput_zero_progress_failures=0,
        chords_rejected=0,
        authored_conflict_events=0,
        authored_chords_rejected=0,
        authored_keys_rejected=0,
        keys_inserted_before_failure=0,
        keys_rolled_back=0,
    )
    from sky_music.orchestration.native_dispatch import (
        NativeDispatchError,
        RustDispatchRuntime,
    )

    with pytest.raises(NativeDispatchError, match="rollback_residue_keys"):
        RustDispatchRuntime._health(cast(Any, snapshot))
