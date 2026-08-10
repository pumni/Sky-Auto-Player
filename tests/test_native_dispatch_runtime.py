"""Supervisor tests for terminal native lifecycle transitions."""

from __future__ import annotations

import json
from types import SimpleNamespace
from typing import Any

import pytest

from sky_music.orchestration.native_dispatch import (
    NativeDispatchError,
    RustDispatchRuntime,
)
from sky_music.orchestration.native_models import PlaybackOutcome


def _live(status: str, *, finished: bool) -> SimpleNamespace:
    health = SimpleNamespace(
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
        rollback_residue_keys=0,
    )
    return SimpleNamespace(
        elapsed_us=0,
        total_us=1,
        max_completion_error_us=0,
        late_2ms=0,
        late_5ms=0,
        late_10ms=0,
        release_max_us=0,
        release_late_2ms=0,
        recent_latencies_us=(),
        is_finished=finished,
        is_paused=False,
        input_path_degraded=False,
        sendinput_path_degraded=False,
        core_post_send_degraded=False,
        wait_path_degraded=False,
        status=status,
        backend_health=health,
    )


def _runtime(session: Any) -> RustDispatchRuntime:
    runtime = object.__new__(RustDispatchRuntime)
    runtime._session = session
    runtime._controls = None
    runtime._focus_guard = SimpleNamespace()
    runtime._has_played = False
    runtime._last_focus_active = None
    runtime._last_hwnd = None
    runtime._manual_paused = False
    runtime._min_hold_us = 0
    runtime._renderer = None
    runtime._require_focus = False
    runtime._sleep_s = 0.002
    runtime._song_name = "test"
    runtime._total_us = 1
    return runtime


class FakeSession:
    def __init__(self, snapshots: list[SimpleNamespace], report_snapshot: dict[str, Any]) -> None:
        self._snapshots = snapshots
        self._last_snapshot = snapshots[-1]
        self._report_snapshot = report_snapshot
        self.join_calls = 0
        self.report_calls = 0
        self.panic_calls = 0
        self.quit_calls = 0
        self.target_hwnd_calls: list[int] = []
        self.focus_hint_calls: list[bool] = []

    def start(self) -> None:
        return None

    def snapshot_lite(self) -> SimpleNamespace:
        if self._snapshots:
            self._last_snapshot = self._snapshots.pop(0)
        return self._last_snapshot

    def heartbeat(self) -> None:
        return None

    def join(self, *, timeout_ms: int) -> bool:
        self.join_calls += 1
        return True

    def session_report(self) -> dict[str, Any]:
        self.report_calls += 1
        return {
            "snapshot": self._report_snapshot,
            "telemetry_json": json.dumps({"records": [], "attempted": 0}),
            "estimator_state_json": "{}",
        }

    def panic_release(self) -> None:
        self.panic_calls += 1

    def quit(self) -> None:
        self.quit_calls += 1

    def set_target_hwnd(self, hwnd: int) -> None:
        self.target_hwnd_calls.append(hwnd)

    def set_focus_hint(self, active: bool) -> None:
        self.focus_hint_calls.append(active)


def _report(status: str, outcome: str = "finished") -> dict[str, Any]:
    return {
        "status": status,
        "outcome": outcome,
        "terminal_error": None,
        "is_finished": True,
    }


def test_normal_finish_consumes_terminal_snapshot_without_cleanup_race() -> None:
    session = FakeSession(
        [_live("playing", finished=False), _live("finished", finished=True)],
        _report("finished"),
    )

    outcome, snapshot, telemetry, estimator = _runtime(session).run()

    assert outcome == PlaybackOutcome.FINISHED
    assert snapshot["status"] == "finished"
    assert telemetry["attempted"] == 0
    assert estimator == "{}"
    assert session.join_calls == 1
    assert session.report_calls == 1
    assert session.panic_calls == 0
    assert session.quit_calls == 0


def test_terminal_error_report_is_materialized_before_error() -> None:
    report = _report("error", "error")
    report["terminal_error"] = "native failure"
    session = FakeSession(
        [_live("playing", finished=False), _live("error", finished=True)],
        report,
    )

    with pytest.raises(NativeDispatchError, match="native failure") as caught:
        _runtime(session).run()

    assert caught.value.snapshot == report
    assert caught.value.telemetry == {"records": [], "attempted": 0}
    assert session.report_calls == 1
    assert session.panic_calls == 0
    assert session.quit_calls == 0


def test_finished_status_without_finished_flag_fails_closed() -> None:
    session = FakeSession(
        [_live("finished", finished=False)],
        _report("finished"),
    )

    with pytest.raises(NativeDispatchError, match="unexpected live native session status: finished"):
        _runtime(session).run()

    assert session.panic_calls == 1
    assert session.quit_calls == 1


def test_unknown_live_status_has_native_contract_error() -> None:
    session = FakeSession(
        [_live("corrupt", finished=False)],
        _report("corrupt"),
    )

    with pytest.raises(NativeDispatchError, match="unknown native session status: corrupt"):
        _runtime(session).run()


def test_focus_hint_publishes_foreground_transition_when_hwnd_is_unchanged(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from sky_music.platform.win32 import window_target

    session = FakeSession([_live("finished", finished=True)], _report("finished"))
    runtime = _runtime(session)
    runtime._require_focus = True
    runtime._last_hwnd = 123
    runtime._last_focus_active = True

    foreground = True
    monkeypatch.setattr(window_target, "cached_target_hwnd", lambda: 123)
    monkeypatch.setattr(window_target, "is_foreground_cached_hwnd", lambda: foreground)

    runtime._publish_focus()
    assert session.target_hwnd_calls == []
    assert session.focus_hint_calls == []

    foreground = False
    runtime._publish_focus()
    assert session.target_hwnd_calls == []
    assert session.focus_hint_calls == [False]

    runtime._publish_focus()
    assert session.focus_hint_calls == [False]
