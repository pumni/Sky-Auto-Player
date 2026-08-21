"""Supervisor tests for terminal native lifecycle transitions."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable
from types import SimpleNamespace
from typing import Any, cast

import pytest

from sky_music.orchestration.native_dispatch import (
    NativeDispatchError,
    RustDispatchRuntime,
)
from sky_music.orchestration.native_models import PlaybackOutcome


def _live(status: str, *, finished: bool, paused: bool = False) -> SimpleNamespace:
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
        pre_roll_remaining_us=0,
        missed_down_boundaries=0,
        missed_down_keys=0,
        missed_hard_late_boundaries=0,
        late_authorized_boundaries=0,
        max_completion_error_us=0,
        late_2ms=0,
        late_5ms=0,
        late_10ms=0,
        release_max_us=0,
        release_late_2ms=0,
        recent_latencies_us=(),
        is_finished=finished,
        is_paused=paused,
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
    runtime._pre_roll_us = 0
    runtime._renderer = None
    runtime._require_focus = False
    runtime._target_hwnd = None
    runtime._sleep_s = 0.002
    runtime._song_name = "test"
    runtime._total_us = 1
    return runtime


class FakeFocusGuard:
    def __init__(
        self,
        *,
        result: bool = False,
        error: Exception | None = None,
        event_log: list[str] | None = None,
    ) -> None:
        self.result = result
        self.error = error
        self.event_log = event_log
        self.focus_calls = 0
        self.on_focus: Callable[[], None] | None = None

    def focus(self) -> bool:
        self.focus_calls += 1
        if self.event_log is not None:
            self.event_log.append("focus")
        if self.on_focus is not None:
            self.on_focus()
        if self.error is not None:
            raise self.error
        return self.result


def _focus_runtime(
    monkeypatch: pytest.MonkeyPatch,
    *,
    has_played: bool = True,
) -> tuple[RustDispatchRuntime, FakeFocusGuard, dict[str, object]]:
    from sky_music.platform.win32 import window_target

    session = FakeSession([_live("finished", finished=True)], _report("finished"))
    runtime = _runtime(session)
    runtime._require_focus = True
    runtime._has_played = has_played
    runtime._last_hwnd = 123
    runtime._target_hwnd = 123
    runtime._last_focus_active = True
    guard = FakeFocusGuard()
    runtime._focus_guard = guard
    state: dict[str, object] = {"hwnd": 123, "focused": False}
    monkeypatch.setattr(window_target, "cached_target_hwnd", lambda: state["hwnd"])
    monkeypatch.setattr(
        window_target,
        "is_foreground_cached_hwnd",
        lambda: bool(state["focused"]),
    )
    monkeypatch.setattr(window_target, "reset_window_cache", lambda: None)
    monkeypatch.setattr(window_target, "is_sky_window_valid", lambda: True)
    return runtime, guard, state


class FakeSession:
    def __init__(
        self,
        snapshots: list[SimpleNamespace],
        report_snapshot: dict[str, Any],
        *,
        event_log: list[str] | None = None,
    ) -> None:
        self._snapshots = snapshots
        self._last_snapshot = snapshots[-1]
        self._report_snapshot = report_snapshot
        self.event_log = event_log if event_log is not None else []
        self.join_calls = 0
        self.report_calls = 0
        self.panic_calls = 0
        self.quit_calls = 0
        self.arm_calls = 0
        self.arm_pre_roll_us: list[int] = []
        self.pause_calls = 0
        self.resume_calls = 0
        self.target_hwnd_calls: list[int] = []
        self.focus_hint_calls: list[bool] = []

    def arm(self, pre_roll_us: int) -> None:
        self.arm_calls += 1
        self.arm_pre_roll_us.append(pre_roll_us)
        self.event_log.append("arm")

    def pause(self) -> None:
        self.pause_calls += 1

    def resume(self) -> None:
        self.resume_calls += 1

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

    outcome, snapshot, telemetry = _runtime(session).run()

    assert outcome == PlaybackOutcome.FINISHED
    assert snapshot["status"] == "finished"
    assert telemetry["attempted"] == 0
    assert session.join_calls == 1
    assert session.report_calls == 1
    assert session.panic_calls == 0
    assert session.quit_calls == 0


def test_runtime_arms_native_before_preroll_reaches_zero() -> None:
    session = FakeSession(
        [
            _live("preroll", finished=False),
            _live("playing", finished=False),
            _live("finished", finished=True),
        ],
        _report("finished"),
    )
    runtime = _runtime(session)
    runtime._pre_roll_us = 3_000_000

    outcome, _snapshot, _telemetry = runtime.run()

    assert outcome == PlaybackOutcome.FINISHED
    assert session.arm_pre_roll_us == [3_000_000]
    assert runtime._has_played is True


def test_runtime_forwards_explicit_down_late_grace_to_native_session(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: dict[str, Any] = {}

    class FakeSessionConfig:
        def __init__(self, **kwargs: Any) -> None:
            captured.update(kwargs)

    class FakeDispatchSession:
        def __init__(self, _actions: Any, *, config: Any) -> None:
            captured["config"] = config

    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(SessionConfig=FakeSessionConfig, DispatchSession=FakeDispatchSession),
    )

    RustDispatchRuntime(
        actions=(),
        song_name="test",
        game_fps=60,
        min_hold_us=17_467,
        down_late_grace_us=500,
        require_focus=False,
        focus_guard=SimpleNamespace(),
        controls=None,
        renderer=None,
        poll_s=0.01,
    )

    assert captured["down_late_grace_us"] == 500


def test_playback_engine_keeps_hold_margin_and_down_late_grace_independent(monkeypatch: pytest.MonkeyPatch) -> None:
    from sky_music.domain import Song
    from sky_music.domain.domain import Microseconds, ScanCode
    from sky_music.domain.scheduler_types import ActionKind, KeyAction
    from sky_music.orchestration import engine as engine_module

    captured: dict[str, Any] = {}

    class FakeRuntime:
        def __init__(self, **kwargs: Any) -> None:
            captured.update(kwargs)

        def run(self) -> tuple[str, dict[str, Any], dict[str, Any]]:
            return engine_module.PLAYBACK_FINISHED, {}, {}

    monkeypatch.setattr(engine_module, "RustDispatchRuntime", FakeRuntime)
    monkeypatch.setattr(engine_module.PlaybackEngine, "_ingest_native_report", lambda *_args: None)
    engine = engine_module.PlaybackEngine(
        song=Song(name="test", notes=()),
        actions=(
            KeyAction(
                kind=ActionKind.DOWN,
                scan_codes=(ScanCode(1),),
                at_us=Microseconds(0),
            ),
        ),
        require_focus=False,
        min_hold_us=17_467,
        min_hold_margin_us=1_800,
        down_late_grace_us=500,
    )

    assert engine._play_native() == engine_module.PLAYBACK_FINISHED
    assert captured["down_late_grace_us"] == 500
    assert captured["down_late_grace_us"] != engine.min_hold_margin_us


def test_runtime_does_not_focus_before_native_worker(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, _guard, _state = _focus_runtime(monkeypatch)
    session = cast(FakeSession, runtime._session)
    runtime.run()

    assert session.arm_calls == 1
    assert not session.focus_hint_calls or session.focus_hint_calls[-1] is False


def test_require_focus_false_skips_startup_focus() -> None:
    session = FakeSession([_live("finished", finished=True)], _report("finished"))
    runtime = _runtime(session)
    guard = FakeFocusGuard()
    runtime._focus_guard = guard

    runtime.run()

    assert guard.focus_calls == 0
    assert session.arm_calls == 1


def test_initial_focus_denial_does_not_retry(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, _state = _focus_runtime(monkeypatch)
    guard.result = False

    runtime.run()
    for _ in range(10):
        runtime._publish_focus()

    assert guard.focus_calls == 0
    assert cast(FakeSession, runtime._session).arm_calls == 1


def test_initial_focus_exception_does_not_retry_or_abort(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, _state = _focus_runtime(monkeypatch)
    guard.error = RuntimeError("foreground denied")

    runtime.run()
    for _ in range(10):
        runtime._publish_focus()

    assert guard.focus_calls == 0
    assert cast(FakeSession, runtime._session).arm_calls == 1


@pytest.mark.parametrize(
    ("focus_result", "actual_foreground"),
    [(True, False), (False, True)],
)
def test_runtime_publishes_actual_foreground_without_focusing(
    monkeypatch: pytest.MonkeyPatch,
    focus_result: bool,
    actual_foreground: bool,
) -> None:
    runtime, guard, state = _focus_runtime(monkeypatch)
    del focus_result
    state["focused"] = actual_foreground

    runtime._set_initial_target()
    runtime._publish_focus()

    assert guard.focus_calls == 0
    assert runtime._last_focus_active is actual_foreground


def test_initial_target_does_not_reenumerate_changed_window(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, state = _focus_runtime(monkeypatch)

    state["hwnd"] = 456
    state["focused"] = True
    runtime._set_initial_target()
    runtime._publish_focus()

    assert guard.focus_calls == 0
    assert runtime._last_hwnd == 123
    assert runtime._last_focus_active is False
    assert cast(FakeSession, runtime._session).target_hwnd_calls[-1] == 123


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


def test_has_played_updates_without_renderer() -> None:
    session = FakeSession(
        [_live("playing", finished=False), _live("finished", finished=True)],
        _report("finished"),
    )
    runtime = _runtime(session)

    runtime.run()

    assert runtime._has_played is True


def test_focus_hint_publishes_foreground_transition_when_hwnd_is_unchanged(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from sky_music.platform.win32 import window_target

    session = FakeSession([_live("finished", finished=True)], _report("finished"))
    runtime = _runtime(session)
    runtime._require_focus = True
    runtime._last_hwnd = 123
    runtime._target_hwnd = 123
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


def test_publish_focus_is_foreground_mutation_free(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, state = _focus_runtime(monkeypatch)
    guard.error = AssertionError("_publish_focus must not call focus")

    for focused in (False, True, False, True):
        state["focused"] = focused
        runtime._publish_focus()

    assert guard.focus_calls == 0


def test_focus_loss_after_startup_never_auto_refocuses(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, _state = _focus_runtime(monkeypatch)

    runtime._publish_focus()
    _state["focused"] = False
    for _ in range(10):
        runtime._publish_focus()

    assert guard.focus_calls == 0


def test_repeated_focus_loss_episodes_never_auto_refocus(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, state = _focus_runtime(monkeypatch)

    runtime._publish_focus()
    for focused in (False, True, False, True, False):
        state["focused"] = focused
        runtime._publish_focus()
        runtime._publish_focus()

    assert guard.focus_calls == 0


def test_pause_and_resume_do_not_auto_focus(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, _state = _focus_runtime(monkeypatch)

    runtime._publish_focus()
    runtime._handle_command("pause")
    runtime._handle_command("pause")

    assert guard.focus_calls == 0
    session = cast(FakeSession, runtime._session)
    assert session.pause_calls == 1
    assert session.resume_calls == 1


def test_manual_refocus_remains_explicit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, _state = _focus_runtime(monkeypatch)

    runtime._publish_focus()
    runtime._handle_command("refocus")

    assert guard.focus_calls == 1


def test_manual_refocus_works_after_failed_startup(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, _state = _focus_runtime(monkeypatch)
    guard.error = RuntimeError("foreground denied")

    runtime._publish_focus()
    guard.error = None
    runtime._handle_command("refocus")

    assert guard.focus_calls == 1


def test_manual_refocus_refreshes_changed_target(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, state = _focus_runtime(monkeypatch)
    state["hwnd"] = 456
    state["focused"] = True
    guard.result = True
    runtime._handle_command("refocus")

    assert guard.focus_calls == 1
    assert runtime._last_hwnd == 456
    assert runtime._last_focus_active is True
    assert cast(FakeSession, runtime._session).target_hwnd_calls[-1] == 456


def test_manual_refocus_does_not_publish_candidate_that_changes_during_focus(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime, guard, state = _focus_runtime(monkeypatch)
    state["focused"] = True

    def recreate_window() -> None:
        state["hwnd"] = 456

    guard.on_focus = recreate_window
    runtime._handle_command("refocus")

    assert guard.focus_calls == 1
    assert runtime._target_hwnd is None
    assert runtime._last_hwnd == 0
    assert cast(FakeSession, runtime._session).target_hwnd_calls[-1] == 0


def test_manual_refocus_resolves_before_focus_and_verifies_same_candidate(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from sky_music.platform.win32 import window_target

    runtime, guard, state = _focus_runtime(monkeypatch)
    state["hwnd"] = 456
    state["focused"] = True
    guard.result = True
    events: list[str] = []
    guard.event_log = events
    monkeypatch.setattr(window_target, "reset_window_cache", lambda: events.append("reset"))
    monkeypatch.setattr(
        window_target,
        "is_sky_window_valid",
        lambda: events.append("resolve") or True,
    )
    monkeypatch.setattr(
        window_target,
        "cached_target_hwnd",
        lambda: events.append("capture") or state["hwnd"],
    )
    monkeypatch.setattr(
        window_target,
        "is_foreground_cached_hwnd",
        lambda: events.append("verify") or bool(state["focused"]),
    )

    runtime._handle_command("refocus")

    assert events[:5] == ["reset", "resolve", "capture", "focus", "capture"]
    assert "verify" in events
    assert runtime._target_hwnd == 456
    assert cast(FakeSession, runtime._session).target_hwnd_calls[-1] == 456
