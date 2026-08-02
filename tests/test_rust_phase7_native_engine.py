"""Tests for Phase 7: Native real-time dispatch session engine."""

from __future__ import annotations

import json
import sys
import time
from types import SimpleNamespace
from typing import Any, cast

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]
from rust_contract_helpers import assert_clean_finished

from sky_music.orchestration.native_dispatch import RustDispatchRuntime
from sky_music.platform.win32 import inputs

_TRACE_OUTCOMES = {
    0: "sent",
    1: "deferred_release",
    2: "failed_note_off",
    3: "blocked_unfocused",
    4: "suppressed_stale_up",
    5: "recovered_zero_progress_but_late",
    6: "strict_completion_slo_exceeded",
    7: "chord_integrity_lost",
    8: "aborted",
}


def _trace_outcome(row: dict[str, Any]) -> str:
    return _TRACE_OUTCOMES[row["outcome"]]


def test_native_dispatch_clean_finished_satisfies_full_contract() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15, 0x16], "clean-down"),
            (1, "up", 2_000, [0x15, 0x16], "clean-up"),
        ],
        [0x15, 0x16],
        min_hold_us=0,
        mock_backend=True,
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    assert_clean_finished(cast(dict[str, Any], session.snapshot()))


def test_native_dispatch_session_lifecycle() -> None:
    actions = [
        (0, "down", 1000, [0x15, 0x16], "chord1"),
        (1, "up", 10000, [0x15, 0x16], "rel1"),
        (2, "down", 20000, [0x17], "note2"),
        (3, "up", 30000, [0x17], "rel2"),
    ]
    allowed = [0x15, 0x16, 0x17]

    session = sky_player_rs.NativeDispatchSessionPy(  # pyright: ignore[reportAttributeAccessIssue]
        actions, allowed, min_hold_us=5000, max_lead_us=2000, mock_backend=True
    )

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
    # Manual cancellation releases the live generation and cancels the
    # remaining ledger; the strict success contract therefore rejects
    # `finished`.
    assert snap_end["status"] == "error"
    assert snap_end["outcome"] == "error"
    assert (
        sum(snap_end["generation_status_counts"].values())
        == snap_end["generation_count"]
    )
    assert snap_end["abort_counts_by_reason"] == {
        "error": 1,
        "manual_pause": 1,
    }
    assert snap_end["release_outcome"]["released_successfully"] is True


def test_native_dispatch_session_quit() -> None:
    actions = [
        (0, "down", 1000, [0x15], "n1"),
        (1, "up", 500000, [0x15], "rel1"),
    ]
    allowed = [0x15]

    session = sky_player_rs.NativeDispatchSessionPy(  # pyright: ignore[reportAttributeAccessIssue]
        actions, allowed, min_hold_us=5000, max_lead_us=2000, mock_backend=True
    )
    session.start()
    time.sleep(0.005)

    started = time.perf_counter()
    session.quit()
    assert session.join(timeout_ms=1000) is True
    elapsed = time.perf_counter() - started

    snap_end = cast(dict[str, Any], session.snapshot())
    assert snap_end["is_finished"] is True
    assert elapsed < 0.1


def test_native_dispatch_session_rejects_double_start() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [(0, "down", 500_000, [0x15], "later")],
        [0x15],
        mock_backend=True,
    )
    session.start()
    with pytest.raises(RuntimeError):
        session.start()
    session.quit()
    assert session.join() is True


def test_native_dispatch_rejects_command_before_start() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [(0, "down", 1_000, [0x15], "note")],
        [0x15],
        mock_backend=True,
    )
    with pytest.raises(RuntimeError):
        session.pause()
    with pytest.raises(RuntimeError):
        session.quit()
    with pytest.raises(ValueError):
        session.send_command("typo")


def test_native_dispatch_join_timeout_poison_is_permanent() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [(0, "down", 500_000, [0x15], "later")],
        [0x15],
        mock_backend=True,
    )
    session.start()
    assert session.join(timeout_ms=1) is False
    assert cast(dict[str, Any], session.snapshot())["status"] == "poisoned"
    # The retained handle can still be joined after cooperative shutdown, but
    # the lifecycle must never return to a healthy terminal state.
    session.quit()
    assert session.join(timeout_ms=1_000) is True
    assert cast(dict[str, Any], session.snapshot())["status"] == "poisoned"


def test_native_dispatch_pause_releases_before_marking_paused() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "held"),
            (1, "up", 500_000, [0x15], "release"),
        ],
        [0x15],
        mock_backend=True,
    )
    session.start()
    time.sleep(0.02)
    assert cast(dict[str, Any], session.snapshot())["active_count"] == 1

    session.pause()
    deadline = time.perf_counter() + 0.2
    snap = cast(dict[str, Any], session.snapshot())
    while not snap["is_paused"] and time.perf_counter() < deadline:
        time.sleep(0.002)
        snap = cast(dict[str, Any], session.snapshot())

    assert snap["is_paused"] is True
    assert snap["active_count"] == 0
    session.quit()
    assert session.join() is True


def test_native_dispatch_focus_gate_uses_interruptible_pause() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "held"),
            (1, "up", 500_000, [0x15], "release"),
        ],
        [0x15],
        mock_backend=True,
        require_focus=True,
    )
    session.start()
    time.sleep(0.01)
    snap = cast(dict[str, Any], session.snapshot())
    assert snap["is_paused"] is True
    assert snap["active_count"] == 0

    foreground_raw = inputs.user32.GetForegroundWindow()
    if not foreground_raw:
        pytest.skip("Windows has no foreground window for the strict focus-gate test")
    foreground_hwnd = int(foreground_raw)
    session.update_focus(True, hwnd=foreground_hwnd)
    deadline = time.perf_counter() + 0.2
    while time.perf_counter() < deadline:
        snap = cast(dict[str, Any], session.snapshot())
        if not snap["is_paused"] and snap["active_count"] == 1:
            break
        time.sleep(0.002)
    assert snap["is_paused"] is False
    assert snap["active_count"] == 1

    session.update_focus(False)
    deadline = time.perf_counter() + 0.2
    while time.perf_counter() < deadline:
        snap = cast(dict[str, Any], session.snapshot())
        if snap["is_paused"] and snap["active_count"] == 0:
            break
        time.sleep(0.002)
    assert snap["is_paused"] is True
    assert snap["active_count"] == 0
    session.quit()
    assert session.join() is True


def test_native_dispatch_focus_cycles_preserve_future_generations() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "first"),
            (1, "up", 100_000, [0x15], "first-release"),
            (2, "down", 200_000, [0x16], "future"),
            (3, "up", 300_000, [0x16], "future-release"),
        ],
        [0x15, 0x16],
        mock_backend=True,
        require_focus=True,
        focus_restore_grace_us=1_000,
    )
    session.start()
    time.sleep(0.01)

    foreground_raw = inputs.user32.GetForegroundWindow()
    if not foreground_raw:
        session.quit()
        assert session.join() is True
        pytest.skip("Windows has no foreground window for the focus-cycle test")
    foreground_hwnd = int(foreground_raw)

    session.update_focus(True, hwnd=foreground_hwnd)
    deadline = time.perf_counter() + 0.2
    snap = cast(dict[str, Any], session.snapshot())
    while time.perf_counter() < deadline and snap["active_count"] != 1:
        time.sleep(0.002)
        snap = cast(dict[str, Any], session.snapshot())
    assert snap["active_count"] == 1

    for _ in range(2):
        session.update_focus(False)
        deadline = time.perf_counter() + 0.2
        while time.perf_counter() < deadline:
            snap = cast(dict[str, Any], session.snapshot())
            if snap["is_paused"] and snap["active_count"] == 0:
                break
            time.sleep(0.002)
        assert snap["is_paused"] is True
        assert snap["active_count"] == 0

        session.update_focus(True, hwnd=foreground_hwnd)
        deadline = time.perf_counter() + 0.2
        while time.perf_counter() < deadline:
            snap = cast(dict[str, Any], session.snapshot())
            if not snap["is_paused"]:
                break
            time.sleep(0.002)
        assert snap["is_paused"] is False

    assert session.join(timeout_ms=1_000) is True
    final = cast(dict[str, Any], session.snapshot())
    assert final["status"] == "error"
    assert final["outcome"] == "error"
    assert "clean completion contract failed" in str(final["terminal_error"])
    assert final["terminal_error"] is not None
    assert final["generation_status_counts"]["cancelled"] == 1
    assert final["generation_status_counts"]["released"] == 1


def test_native_dispatch_manual_pause_cancels_live_but_keeps_future_same_key() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "first"),
            (1, "up", 100_000, [0x15], "first-release"),
            (2, "down", 200_000, [0x15], "future"),
            (3, "up", 300_000, [0x15], "future-release"),
        ],
        [0x15],
        mock_backend=True,
    )
    session.start()
    time.sleep(0.01)
    assert cast(dict[str, Any], session.snapshot())["active_count"] == 1

    session.pause()
    deadline = time.perf_counter() + 0.2
    snap = cast(dict[str, Any], session.snapshot())
    while time.perf_counter() < deadline and not snap["is_paused"]:
        time.sleep(0.002)
        snap = cast(dict[str, Any], session.snapshot())
    assert snap["is_paused"] is True
    assert snap["active_count"] == 0

    session.resume()
    assert session.join(timeout_ms=1_000) is True
    final = cast(dict[str, Any], session.snapshot())
    assert final["status"] == "error"
    assert final["outcome"] == "error"
    assert "clean completion contract failed" in str(final["terminal_error"])
    assert final["generation_status_counts"]["cancelled"] == 1
    assert final["generation_status_counts"]["released"] == 1


def test_native_dispatch_telemetry_is_terminal_retain_first_buffer() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "note"),
            (1, "up", 3_000, [0x15], "release"),
        ],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
        telemetry_mode="ring",
        telemetry_capacity=1,
    )
    with pytest.raises(RuntimeError):
        session.take_telemetry_json()
    session.start()
    assert session.join() is True

    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))
    assert output["attempted"] == 2
    assert output["accepted"] == 1
    assert output["dropped"] == 1
    assert output["truncated"] is True
    assert len(output["records"]) == 1
    assert set(output["records"][0]) == {
        "event_index",
        "kind",
        "outcome",
        "polyphony",
        "flags",
        "authored_ticks",
        "effective_deadline_ticks",
        "wake_ticks",
        "send_started_ticks",
        "send_completed_ticks",
        "completion_error_ticks",
        "applied_lead_ticks",
        "win32_error",
    }
    with pytest.raises(RuntimeError):
        session.take_telemetry_json()


def test_native_dispatch_telemetry_does_not_drop_a_late_pulse() -> None:
    expired = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "expired"),
            (1, "up", 1_000, [0x15], "release"),
        ],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
        telemetry_mode="ring",
    )
    expired.start()
    assert expired.join() is True
    assert_clean_finished(cast(dict[str, Any], expired.snapshot()))
    expired_output = cast(
        dict[str, Any],
        json.loads(expired.take_telemetry_json()),
    )
    assert _trace_outcome(expired_output["records"][0]) == "sent"

    stale = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "up", 0, [0x15], "stale")],
        [0x15],
        mock_backend=True,
        telemetry_mode="ring",
    )
    stale.start()
    assert stale.join() is True
    stale_output = cast(dict[str, Any], json.loads(stale.take_telemetry_json()))
    assert [_trace_outcome(row) for row in stale_output["records"]] == [
        "suppressed_stale_up"
    ]


def test_native_dispatch_telemetry_marks_deferred_release() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "down"),
            (1, "up", 1_000, [0x15], "up"),
        ],
        [0x15],
        min_hold_us=10_000,
        mock_backend=True,
        telemetry_mode="ring",
    )
    session.start()
    assert session.join() is True
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))
    assert [_trace_outcome(row) for row in output["records"]] == [
        "sent",
        "deferred_release",
    ]
    assert [row["kind"] for row in output["records"]] == [0, 1]


def test_native_worker_retries_transient_note_off_before_same_key_down() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "down-1"),
            (1, "up", 1_000, [0x15], "up-1"),
            (2, "down", 12_000, [0x15], "down-2"),
            (3, "up", 13_000, [0x15], "up-2"),
        ],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
        mock_failure_mode="transient_release",
        telemetry_mode="ring",
    )
    session.start()
    assert session.join() is True

    snapshot = cast(dict[str, Any], session.snapshot())
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))
    records = output["records"]

    assert snapshot["status"] == "finished"
    assert snapshot["outcome"] == "finished"
    assert snapshot["active_count"] == 0
    assert snapshot["failed_release_count"] == 0
    assert any(_trace_outcome(row) == "failed_note_off" for row in records)
    assert any(
        row["event_index"] == 1 and _trace_outcome(row) in {"sent", "deferred_release"}
        for row in records
    )
    assert any(
        row["event_index"] == 2 and _trace_outcome(row) == "sent" for row in records
    )


def test_native_worker_rejects_zero_progress_retry_that_completes_late() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "down", 0, [0x15], "retry-late")],
        [0x15],
        mock_backend=True,
        mock_failure_mode="zero_progress_down_once",
        mock_latency_base_us=3_000,
        telemetry_mode="ring",
        strict_timing=True,
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    snapshot = cast(dict[str, Any], session.snapshot())
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))

    assert snapshot["status"] == "error"
    assert snapshot["recovered_zero_progress_but_late"] == 1
    assert "zero-progress retry" in snapshot["terminal_error"]
    assert _trace_outcome(output["records"][0]) == "recovered_zero_progress_but_late"


def test_native_worker_stops_when_supervisor_lease_expires() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "down", 1_000_000, [0x15], "lease")],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
        supervisor_lease_timeout_us=50_000,
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    snapshot = cast(dict[str, Any], session.snapshot())
    assert snapshot["status"] == "error"
    assert snapshot["terminal_error"] == "supervisor_lease_expired"
    assert snapshot["release_outcome"]["released_successfully"] is True


def test_native_worker_exhausts_persistent_note_off_and_stops_dispatch() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "down-1"),
            (1, "up", 1_000, [0x15], "up-1"),
            (2, "down", 6_000, [0x15], "down-2"),
        ],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
        mock_failure_mode="persistent_release",
        telemetry_mode="ring",
    )
    session.start()
    assert session.join() is True

    snapshot = cast(dict[str, Any], session.snapshot())
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))

    assert snapshot["status"] == "error"
    assert snapshot["outcome"] == "error"
    assert "note-off recovery exhausted" in snapshot["terminal_error"]
    assert snapshot["release_outcome"]["released_successfully"] is False
    assert not any(
        row["event_index"] == 2 and _trace_outcome(row) == "sent"
        for row in output["records"]
    )


def test_native_adapter_collects_controlled_error_before_returning(monkeypatch) -> None:
    snapshot: dict[str, object] = {
        "is_finished": True,
        "status": "error",
        "outcome": "error",
        "terminal_error": "note-off recovery exhausted",
    }

    class _ErrorSession:
        def __init__(self, *_args, **_kwargs) -> None:
            pass

        def update_focus(self, _active: bool) -> None:
            pass

        def start(self) -> None:
            pass

        def snapshot(self) -> dict[str, object]:
            return snapshot

        def join(self, timeout_ms: int = 5_000) -> bool:
            assert timeout_ms == 5_000
            return True

        def take_telemetry_json(self) -> str:
            return json.dumps(
                {
                    "records": [{"outcome": 2}],
                    "schema_version": 4,
                    "qpc_frequency_hz": 10_000_000,
                }
            )

        def estimator_state_json(self) -> str:
            return "{}"

    monkeypatch.setitem(
        sys.modules, "sky_player_rs", SimpleNamespace(DispatchSession=_ErrorSession)
    )
    runtime = RustDispatchRuntime(
        actions=(),
        song_name="controlled-error",
        min_hold_us=0,
        max_lead_us=2_000,
        focus_restore_grace_us=100_000,
        spin_threshold_us=150,
        telemetry_mode="ring",
        rt_priority_mode="off",
        enable_waitable_timer=True,
        enable_event_wait=True,
        enable_adaptive_spin=False,
        spin_floor_us=700,
        input_path_warn_us=300,
        enable_adaptive_lead=False,
        estimator_state_json=None,
        require_focus=False,
        focus_guard=None,
        controls=None,
        renderer=None,
        poll_s=0.002,
    )

    outcome, latest, telemetry, estimator = runtime.run()
    assert outcome == "error"
    assert latest["terminal_error"] == "note-off recovery exhausted"
    assert telemetry["records"][0]["outcome"] == 2
    assert estimator == "{}"


def test_native_dispatch_fixed_lead_overrides_adaptive_estimator() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "down", 5_000, [0x15], "lead")],
        [0x15],
        min_hold_us=0,
        dispatch_lead_us=1_000,
        mock_backend=True,
        telemetry_mode="ring",
        enable_adaptive_lead=True,
    )
    session.start()
    assert session.join() is True
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))
    assert output["records"][0]["applied_lead_ticks"] == 10_000


def test_native_dispatch_snapshot_publishes_ui_counter_batch() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "down", 1_000, [0x15], "counter")],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
    )
    session.start()
    assert session.join() is True
    snapshot = cast(dict[str, Any], session.snapshot())
    assert snapshot["max_lateness_us"] >= 0
    assert len(snapshot["recent_latencies_us"]) == 1
    assert snapshot["release_max_us"] == 0
    assert snapshot["release_late_2ms"] == 0


def test_native_dispatch_adaptive_probe_publishes_effective_threshold() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [(0, "down", 1_000, [0x15], "note")],
        [0x15],
        mock_backend=True,
        rt_priority_mode="off",
        enable_adaptive_spin=True,
        spin_floor_us=700,
    )
    session.start()
    assert session.join() is True
    snapshot = cast(dict[str, Any], session.snapshot())
    assert 700 <= snapshot["effective_spin_threshold_us"] <= 3_000
    assert snapshot["rt_priority_acquired"] == "off"
    assert snapshot["wait_strategy_acquired"] in {
        "event+high_resolution_timer",
        "event+timer_resolution_fallback",
    }


def test_native_dispatch_estimator_cache_round_trip() -> None:
    seed = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "seed-down"),
            (1, "up", 2_000, [0x15], "seed-up"),
        ],
        [0x15],
        mock_backend=True,
        telemetry_mode="off",
        enable_adaptive_lead=True,
    )
    seed.start()
    assert seed.join() is True
    initial = cast(dict[str, Any], json.loads(seed.estimator_state_json()))

    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "note"),
            (1, "up", 2_000, [0x15], "release"),
        ],
        [0x15],
        mock_backend=True,
        telemetry_mode="off",
        enable_adaptive_lead=True,
        estimator_state_json=json.dumps(initial),
    )
    session.start()
    assert session.join() is True
    exported = cast(dict[str, Any], json.loads(session.estimator_state_json()))
    assert exported["version"] == 8
    initial_down_count = sum(
        int(pair[1])
        for class_name in ("hot_pairs", "cold_pairs")
        for pair in initial["hist_down"][1][class_name]
    )
    exported_down_count = sum(
        int(pair[1])
        for class_name in ("hot_pairs", "cold_pairs")
        for pair in exported["hist_down"][1][class_name]
    )
    assert exported_down_count == initial_down_count + 1


def test_native_dispatch_discards_stale_estimator_cache_and_uses_prior() -> None:
    session = sky_player_rs.NativeDispatchSessionPy(  # type: ignore[attr-defined]
        [
            (0, "down", 1_000, [0x15], "note"),
            (1, "up", 2_000, [0x15], "release"),
        ],
        [0x15],
        mock_backend=True,
        enable_adaptive_lead=True,
        estimator_state_json='{"version":999}',
    )
    session.start()
    assert session.join() is True
    assert cast(dict[str, Any], session.snapshot())["outcome"] == "finished"


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("min_hold_us", True),
        ("max_lead_us", True),
        ("dispatch_lead_us", True),
        ("focus_restore_grace_us", True),
        ("spin_threshold_us", True),
        ("core_warmup_budget_us", True),
        ("telemetry_capacity", True),
        ("spin_floor_us", True),
        ("strict_down_completion_late_us", True),
        ("strict_up_completion_late_us", True),
        ("supervisor_lease_timeout_us", True),
    ],
)
def test_native_dispatch_rejects_bool_for_integer_config(
    field: str,
    value: object,
) -> None:
    with pytest.raises(TypeError, match="not bool"):
        sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [(0, "down", 0, [0x15], "note")],
            [0x15],
            **{field: value},
        )


def test_native_dispatch_rejects_bool_for_runtime_integer_arguments() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "down", 500_000, [0x15], "note")],
        [0x15],
        mock_backend=True,
    )
    with pytest.raises(TypeError, match="not bool"):
        session.set_target_hwnd(True)
    with pytest.raises(TypeError, match="not bool"):
        session.update_focus(True, hwnd=True)
    session.start()
    with pytest.raises(TypeError, match="not bool"):
        session.join(timeout_ms=True)
    session.quit()
    assert session.join() is True


def test_native_dispatch_strict_conflict_is_contained_and_reported() -> None:
    with pytest.raises(ValueError, match="overlapping same-key down actions"):
        sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [
                (0, "down", 0, [0x15], "first"),
                (1, "down", 1_000, [0x15], "overlap"),
            ],
            [0x15],
            mock_backend=True,
        )


def test_native_strict_timing_overrides_default_drop_chord_policy() -> None:
    with pytest.raises(ValueError, match="overlapping same-key down actions"):
        sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [
                (0, "down", 0, [0x15], "first"),
                (1, "down", 1_000, [0x15], "overlap"),
            ],
            [0x15],
            mock_backend=True,
            strict_timing=True,
        )


def test_native_strict_down_completion_slo_rejects_clean_late_send() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, "down", 0, [0x15], "late-down")],
        [0x15],
        mock_backend=True,
        mock_latency_base_us=3_000,
        strict_timing=True,
        strict_down_completion_late_us=2_000,
        telemetry_mode="ring",
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    snapshot = cast(dict[str, Any], session.snapshot())
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))
    assert snapshot["status"] == "error"
    assert "completion SLO" in snapshot["terminal_error"]
    assert _trace_outcome(output["records"][0]) == "strict_completion_slo_exceeded"


def test_native_strict_up_completion_slo_rejects_clean_late_release() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "down"),
            (1, "up", 20_000, [0x15], "late-up"),
        ],
        [0x15],
        min_hold_us=0,
        mock_backend=True,
        mock_latency_base_us=3_000,
        strict_timing=True,
        strict_down_completion_late_us=10_000,
        strict_up_completion_late_us=2_000,
        telemetry_mode="ring",
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    snapshot = cast(dict[str, Any], session.snapshot())
    output = cast(dict[str, Any], json.loads(session.take_telemetry_json()))
    assert snapshot["status"] == "error"
    assert "note-off" in snapshot["terminal_error"]
    assert _trace_outcome(output["records"][-1]) == "strict_completion_slo_exceeded"


def test_native_deferred_release_is_excluded_from_strict_completion_slo() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "down"),
            (1, "up", 1_000, [0x15], "deferred-up"),
        ],
        [0x15],
        min_hold_us=10_000,
        mock_backend=True,
        mock_latency_base_us=3_000,
        strict_timing=True,
        strict_down_completion_late_us=10_000,
        strict_up_completion_late_us=2_000,
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    snapshot = cast(dict[str, Any], session.snapshot())
    assert snapshot["status"] == "finished"


def test_native_non_strict_late_send_is_telemetry_only() -> None:
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [
            (0, "down", 0, [0x15], "late-but-best-effort"),
            (1, "up", 10_000, [0x15], "late-release"),
        ],
        [0x15],
        mock_backend=True,
        mock_latency_base_us=3_000,
        strict_timing=False,
    )
    session.start()
    assert session.join(timeout_ms=5_000) is True

    snapshot = cast(dict[str, Any], session.snapshot())
    assert snapshot["status"] == "finished"


@pytest.mark.parametrize(
    ("actions", "allowed"),
    [
        ([(0, "typo", 0, [0x15], "bad-kind")], [0x15]),
        ([(0, "down", 0, [True], "bool-scan")], [0x15]),
        ([(0, "down", 0, [0x16], "not-allowed")], [0x15]),
        ([(0, "down", 0, [0x15, 0x15], "duplicate")], [0x15]),
        (
            [
                (1, "down", 0, [0x15], "first"),
                (0, "up", 1, [0x15], "index-regression"),
            ],
            [0x15],
        ),
        (
            [
                (0, "down", 2, [0x15], "first"),
                (1, "up", 1, [0x15], "time-regression"),
            ],
            [0x15],
        ),
        ([(0, "down", 0, [0x15], "x" * 129)], [0x15]),
        (
            [(0, "down", 0, [0x15], "duplicate-allowlist")],
            [0x15, 0x15],
        ),
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
