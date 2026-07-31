from __future__ import annotations

from sky_music.domain.domain import Microseconds, ScanCode
from sky_music.domain.scheduler_types import ActionKind, KeyAction
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)


def test_failed_release_is_requeued_before_same_key_down() -> None:
    schedule = compile_runtime_intents(
        (
            KeyAction(ActionKind.DOWN, (ScanCode(21),), Microseconds(0), "down-1"),
            KeyAction(ActionKind.UP, (ScanCode(21),), Microseconds(1_000), "up-1"),
            KeyAction(ActionKind.DOWN, (ScanCode(21),), Microseconds(4_000), "down-2"),
        )
    )
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=0)

    first_down = coordinator.pop_next_due_authored(0)
    assert first_down is not None
    coordinator.activate_sent_downs(
        first_down[0].intents,
        (21,),
        dispatch_started_us=0,
        dispatch_completed_us=10,
    )
    up = coordinator.pop_next_due_authored(1_000)
    assert up is not None
    pending, suppressed = coordinator.request_releases(up[0].intents)
    assert pending and not suppressed

    due = coordinator.pop_due_pending(1_000)
    assert len(due) == 1
    assert not coordinator.requeue_failed_releases(due, (), (), 1_000)
    assert coordinator.next_pending_release_us() == 3_000
    assert not coordinator.is_finished()

    retry = coordinator.pop_due_pending(3_000)
    assert len(retry) == 1
    coordinator.complete_releases(retry, (21,))
    assert coordinator.active_by_scan_code == {}
    assert coordinator.generation_status_counts()["released"] == 1
    next_down = coordinator.pop_next_due_authored(4_000)
    assert next_down is not None
    playable, conflicts = coordinator.split_down_intents(next_down[0].intents)
    assert tuple(intent.scan_code for intent in playable) == (21,)
    assert conflicts == ()


def test_release_retry_exhaustion_requests_recovery() -> None:
    schedule = compile_runtime_intents(
        (
            KeyAction(ActionKind.DOWN, (ScanCode(21),), Microseconds(0), "down"),
            KeyAction(ActionKind.UP, (ScanCode(21),), Microseconds(1_000), "up"),
        )
    )
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=0)
    down = coordinator.pop_next_due_authored(0)
    assert down is not None
    coordinator.activate_sent_downs(
        down[0].intents,
        (21,),
        dispatch_started_us=0,
        dispatch_completed_us=0,
    )
    up = coordinator.pop_next_due_authored(1_000)
    assert up is not None
    requested, _ = coordinator.request_releases(up[0].intents)
    assert requested

    for attempt in range(8):
        due = coordinator.pop_due_pending(1_000 + attempt * 20_000)
        assert due
        assert not coordinator.requeue_failed_releases(due, (), (), 1_000 + attempt * 20_000)

    due = coordinator.pop_due_pending(200_000)
    assert due
    assert coordinator.requeue_failed_releases(due, (), (), 200_000)
