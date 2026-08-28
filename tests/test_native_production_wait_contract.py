from __future__ import annotations

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]


@pytest.mark.parametrize("priority_mode", ["off", "mmcss", "highest", "time_critical"])
def test_production_calibrated_test_session_rejects_non_auto_priority(
    priority_mode: str,
) -> None:
    test_session = getattr(sky_player_rs, "TestDispatchSession", None)
    if not callable(test_session):
        pytest.skip("requires the test-support native wheel")

    scan_code = int(sky_player_rs.instrument_scan_codes()[0])
    actions = [
        (0, "down", 100_000, [scan_code], "contract-down"),
        (1, "up", 200_000, [scan_code], "contract-up"),
    ]
    with pytest.raises(ValueError, match="requires rt_priority_mode=auto"):
        test_session(
            actions,
            [scan_code],
            min_hold_us=100,
            min_release_gap_us=16_667,
            game_fps=60,
            rt_priority_mode=priority_mode,
            wait_policy="production_calibrated",
        )


def test_production_calibrated_test_session_accepts_auto_priority() -> None:
    test_session = getattr(sky_player_rs, "TestDispatchSession", None)
    if not callable(test_session):
        pytest.skip("requires the test-support native wheel")

    scan_code = int(sky_player_rs.instrument_scan_codes()[0])
    actions = [
        (0, "down", 100_000, [scan_code], "contract-down"),
        (1, "up", 200_000, [scan_code], "contract-up"),
    ]
    session = test_session(
        actions,
        [scan_code],
        min_hold_us=100,
        min_release_gap_us=16_667,
        game_fps=60,
        rt_priority_mode="auto",
        wait_policy="production_calibrated",
    )
    assert session is not None
