"""Tests for Phase 4: Rust hybrid sleeper and timing helpers."""

from __future__ import annotations

from typing import cast

import sky_player_rs  # type: ignore[import-not-found,import-untyped]


def test_qpc_now_rs() -> None:
    t1 = cast(int, sky_player_rs.qpc_now_rs())  # type: ignore[attr-defined]
    assert t1 > 0
    t2 = cast(int, sky_player_rs.qpc_now_rs())  # type: ignore[attr-defined]
    assert t2 >= t1


def test_measure_spin_overhead_rs() -> None:
    overhead = cast(int, sky_player_rs.measure_spin_overhead_rs())  # type: ignore[attr-defined]
    assert overhead >= 1


def test_sleep_until_rs_future() -> None:
    now = cast(int, sky_player_rs.qpc_now_rs())  # type: ignore[attr-defined]
    target = now + 2_000  # 2 ms in future
    overshoot = cast(int, sky_player_rs.sleep_until_rs(target, 200))  # type: ignore[attr-defined]
    end_time = cast(int, sky_player_rs.qpc_now_rs())  # type: ignore[attr-defined]

    assert end_time >= target
    assert overshoot >= 0
    assert abs((target + overshoot) - end_time) <= 100


def test_sleep_until_rs_past() -> None:
    now = cast(int, sky_player_rs.qpc_now_rs())  # type: ignore[attr-defined]
    target = now - 500  # 500 us in past
    overshoot = cast(int, sky_player_rs.sleep_until_rs(target, 200))  # type: ignore[attr-defined]
    assert overshoot >= 500
