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
        min_release_gap_us=9_134,
        down_late_grace_us=500,
        require_focus=True,
        focus_restore_grace_us=12_345,
        target_hwnd=123,
        telemetry=True,
        profile="production",
    )
    assert config.min_hold_us == 50_000
    assert config.min_release_gap_us == 9_134
    assert config.down_late_grace_us == 500
    assert config.game_fps == 120
    assert config.require_focus is True
    assert config.focus_restore_grace_us == 12_345
    assert config.target_hwnd == 123
    assert config.telemetry is True
    assert config.profile == "production"


def test_session_config_defaults_down_late_grace_to_five_hundred_us() -> None:
    config = sky_player_rs.SessionConfig(game_fps=60)  # type: ignore[attr-defined]

    assert config.down_late_grace_us == 500
    assert config.min_release_gap_us == 16_667


def test_session_config_rejects_down_late_grace_above_min_hold() -> None:
    with pytest.raises(ValueError, match="down_late_grace_us must not exceed min_hold_us"):
        sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=60,
            min_hold_us=100,
            down_late_grace_us=101,
        )


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


def test_native_constructor_rejects_same_key_hold_below_materialized_hold() -> None:
    with pytest.raises(ValueError, match="same-key hold too short"):
        sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [
                (0, "down", 0, [SKY_15_SCAN_CODES[0]], "down"),
                (1, "up", 4_666, [SKY_15_SCAN_CODES[0]], "up"),
            ],
            config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
                game_fps=240,
                min_hold_us=4_667,
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


def test_native_constructor_accepts_exact_explicit_release_gap_boundary() -> None:
    hold_us = 17_467
    release_gap_us = 17_467
    scan_code = SKY_15_SCAN_CODES[0]
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [scan_code], "down-a"),
            (1, "up", hold_us, [scan_code], "up-a"),
            (2, "down", hold_us + release_gap_us, [scan_code], "down-b"),
            (3, "up", hold_us * 2 + release_gap_us, [scan_code], "up-b"),
        ],
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=60,
            min_hold_us=hold_us,
            min_release_gap_us=release_gap_us,
        ),
    )
    assert session is not None


def test_native_constructor_rejects_one_microsecond_short_release_gap() -> None:
    hold_us = 17_467
    release_gap_us = 17_466
    scan_code = SKY_15_SCAN_CODES[0]
    with pytest.raises(ValueError, match="release gap"):
        sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [
                (0, "down", 0, [scan_code], "down-a"),
                (1, "up", hold_us, [scan_code], "up-a"),
                (2, "down", hold_us + release_gap_us, [scan_code], "down-b"),
                (3, "up", hold_us * 2 + release_gap_us, [scan_code], "up-b"),
            ],
            config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
                game_fps=60,
                min_hold_us=hold_us,
                min_release_gap_us=17_467,
            ),
        )


@pytest.mark.parametrize("margin_us", [300, 400, 499, 500])
def test_native_constructor_accepts_python_materialized_calibration_margin(
    margin_us: int,
) -> None:
    # Python authors one-frame hold + static calibrated margin. PyO3 must
    # accept that exact value instead of adding a second frame-relative floor.
    materialized_hold_us = 16_667 + margin_us
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [SKY_15_SCAN_CODES[0]], "down"),
            (1, "up", materialized_hold_us, [SKY_15_SCAN_CODES[0]], "up"),
        ],
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=60,
            min_hold_us=materialized_hold_us,
            require_focus=False,
        ),
    )
    # Construction itself is the pre-start/native validation gate. Do not arm
    # a production session in this contract test merely to inspect telemetry.
    assert session is not None


def test_session_reports_lite_progress_then_one_final_report() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [],
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=60,
            min_hold_us=100,
            down_late_grace_us=0,
            require_focus=False,
            focus_restore_grace_us=1_234,
            telemetry=True,
        ),
    )
    session.arm(0)  # type: ignore[attr-defined]
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
    }
    assert dict(report["snapshot"])["is_finished"] is True
    assert report["effective_config"] == {
        "game_fps": 60,
        "requested_min_hold_us": 100,
        "effective_min_hold_us": 100,
        "min_release_gap_us": 16_667,
        "down_late_grace_us": 0,
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
