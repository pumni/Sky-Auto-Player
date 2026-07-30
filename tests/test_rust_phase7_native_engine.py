"""Tests for Phase 7: Native real-time dispatch session engine."""

from __future__ import annotations

import time
from typing import Any, cast

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]


def test_native_dispatch_session_lifecycle() -> None:
    actions = [
        (0, "down", 1000, [2, 3], "chord1"),
        (1, "up", 10000, [2, 3], "rel1"),
        (2, "down", 20000, [4], "note2"),
        (3, "up", 30000, [4], "rel2"),
    ]
    allowed = [2, 3, 4]

    session = sky_player_rs.NativeDispatchSessionPy(actions, allowed, min_hold_us=5000, max_lead_us=2000, mock_backend=True)  # type: ignore[attr-defined]

    snap0 = cast(dict[str, Any], session.snapshot())
    assert snap0["is_running"] is False
    assert snap0["is_finished"] is False

    session.start()
    time.sleep(0.01)

    snap1 = cast(dict[str, Any], session.snapshot())
    assert snap1["total_us"] == 30000

    session.pause()
    time.sleep(0.01)

    snap_pause = cast(dict[str, Any], session.snapshot())
    assert snap_pause["is_paused"] is True or snap_pause["is_finished"] is True

    session.resume()
    session.join()

    snap_end = cast(dict[str, Any], session.snapshot())
    assert snap_end["is_finished"] is True
    assert snap_end["status"] == "finished"


def test_native_dispatch_session_quit() -> None:
    actions = [
        (0, "down", 1000, [2], "n1"),
        (1, "up", 500000, [2], "rel1"),
    ]
    allowed = [2]

    session = sky_player_rs.NativeDispatchSessionPy(actions, allowed, min_hold_us=5000, max_lead_us=2000, mock_backend=True)  # type: ignore[attr-defined]
    session.start()
    time.sleep(0.005)

    session.quit()
    session.join()

    snap_end = cast(dict[str, Any], session.snapshot())
    assert snap_end["is_finished"] is True


@pytest.mark.parametrize(
    ("actions", "allowed"),
    [
        ([(0, "typo", 0, [2], "bad-kind")], [2]),
        ([(0, "down", 0, [True], "bool-scan")], [2]),
        ([(0, "down", 0, [3], "not-allowed")], [2]),
        ([(0, "down", 0, [2, 2], "duplicate")], [2]),
        (
            [
                (1, "down", 0, [2], "first"),
                (0, "up", 1, [2], "index-regression"),
            ],
            [2],
        ),
        (
            [
                (0, "down", 2, [2], "first"),
                (1, "up", 1, [2], "time-regression"),
            ],
            [2],
        ),
        ([(0, "down", 0, [2], "x" * 129)], [2]),
        ([(0, "down", 0, [2], "duplicate-allowlist")], [2, 2]),
    ],
)
def test_native_dispatch_session_rejects_invalid_prepare_inputs(
    actions: list[tuple[object, object, object, list[object], object]],
    allowed: list[object],
) -> None:
    with pytest.raises((TypeError, ValueError)):
        sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
            actions,
            allowed,
            mock_backend=True,
        )
