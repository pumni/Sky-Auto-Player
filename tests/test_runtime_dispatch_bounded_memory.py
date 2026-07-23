"""Regression tests verifying RuntimeDispatchCoordinator O(polyphony) memory bound.

The core fix (commit 26d9b00) changed ``status_by_generation`` from O(note_count)
to O(polyphony) by keeping only non-terminal entries in a live dict and folding
terminal states into O(1) running counters (``_terminal_counts``).  These tests
assert that invariant — largely independently of ``PlaybackEngine`` —
by driving ``RuntimeDispatchCoordinator`` directly.
"""
from __future__ import annotations

from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.orchestration.runtime_dispatch import (
    GenerationStatus,
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)


def action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(sc) for sc in scan_codes),
        at_us=Microseconds(at_us),
        reason="test",
    )


def _max_observed(coord: RuntimeDispatchCoordinator, *, send_us: int = 0) -> int:
    """Drive *coord* to completion and return the peak |status_by_generation|."""
    now = 0
    peak = 0
    while not coord.is_finished():
        deadline = coord.next_deadline_us()
        if deadline is None:
            break
        now = max(now, deadline)

        for pending in (coord.pop_due_pending(now),):
            if pending:
                now += send_us
                sent = tuple(r.scan_code for r in pending)
                coord.complete_releases(pending, sent, ())

        for batch, _ in coord.pop_due_authored(now):
            if batch.kind == "up":
                coord.request_releases(batch.intents)
                newly = coord.pop_due_pending(now)
                if newly:
                    now += send_us
                    sent = tuple(r.scan_code for r in newly)
                    coord.complete_releases(newly, sent, ())
            else:
                playable, _ = coord.split_down_intents(batch.intents)
                if playable:
                    now += send_us
                    sent = tuple(i.scan_code for i in playable)
                    coord.activate_sent_downs(
                        playable,
                        sent,
                        dispatch_started_us=now - send_us,
                        dispatch_completed_us=now,
                    )

        size_s = len(coord.status_by_generation)
        peak = max(peak, size_s)
        # Drain-cycle invariant: live entries never exceed combined active + pending.
        assert size_s <= len(coord.active_by_scan_code) + len(coord.pending_by_generation), (
            f"|S|={size_s} > |A|+|P|={len(coord.active_by_scan_code)}"
            f"+{len(coord.pending_by_generation)}"
        )
    return peak


# ---------------------------------------------------------------------------
# Invariant tests  (I1 — I3 from the deep-audit proof)
# ---------------------------------------------------------------------------


def test_only_non_terminal_statuses() -> None:
    """I1: status_by_generation values are always {ACTIVE, RELEASE_PENDING}."""
    actions = (action(0, "down", 21, 22), action(100, "up", 21), action(200, "up", 22))
    sched = compile_runtime_intents(actions)
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=0)
    _max_observed(coord)
    assert not coord.status_by_generation
    # Replay and check mid-flight — use a small min_hold to keep a generation live.
    actions2 = (action(0, "down", 21), action(50_000, "up", 21))
    sched2 = compile_runtime_intents(actions2)
    coord2 = RuntimeDispatchCoordinator(sched2, min_hold_us=10_000)

    deadline = coord2.next_deadline_us()
    assert deadline is not None
    now = deadline
    for batch, _ in coord2.pop_due_authored(now):
        playable, _ = coord2.split_down_intents(batch.intents)
        if playable:
            coord2.activate_sent_downs(
                playable, tuple(i.scan_code for i in playable),
                dispatch_started_us=now, dispatch_completed_us=now,
            )
    # Now the generation is ACTIVE — assert the singleton invariant.
    assert set(coord2.status_by_generation.values()) <= {GenerationStatus.ACTIVE, GenerationStatus.RELEASE_PENDING}


def test_S_equals_active_union_pending() -> None:
    """I2: dom(status_by_generation) == image_g(active_by_scan_code) ∪ dom(pending_by_generation)."""
    actions = (
        action(0, "down", 21),
        action(0, "down", 22),
        action(50, "up", 21),
        action(1_000, "up", 22),
    )
    sched = compile_runtime_intents(actions)
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=0)
    _max_observed(coord)
    # Mid-flight assertion: after the first up, gen0 should be RELEASE_PENDING
    # (active_by_scan_code still has it, pending_by_generation has it).
    actions2 = (action(0, "down", 21), action(50, "up", 21))
    sched2 = compile_runtime_intents(actions2)
    coord2 = RuntimeDispatchCoordinator(sched2, min_hold_us=100)

    now = 0
    for batch, _ in coord2.pop_due_authored(now):
        playable, _ = coord2.split_down_intents(batch.intents)
        if playable:
            coord2.activate_sent_downs(
                playable, tuple(i.scan_code for i in playable),
                dispatch_started_us=now, dispatch_completed_us=now,
            )
    now = 50
    for batch, _ in coord2.pop_due_authored(now):
        if batch.kind == "up":
            coord2.request_releases(batch.intents)

    active_ids = {a.generation_id for a in coord2.active_by_scan_code.values()}
    pending_ids = set(coord2.pending_by_generation)
    s_ids = set(coord2.status_by_generation)
    assert s_ids == (active_ids | pending_ids)
    for g in pending_ids:
        assert coord2.status_by_generation[g] == GenerationStatus.RELEASE_PENDING


def test_pending_ids_present_in_active() -> None:
    """I3: dom(pending_by_generation) ⊆ image_g(active_by_scan_code)."""
    actions = (action(0, "down", 21), action(50_000, "up", 21))
    sched = compile_runtime_intents(actions)
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=1_000)

    now = 0
    for batch, _ in coord.pop_due_authored(now):
        playable, _ = coord.split_down_intents(batch.intents)
        if playable:
            coord.activate_sent_downs(
                playable, tuple(i.scan_code for i in playable),
                dispatch_started_us=now, dispatch_completed_us=now,
            )
    now = 50_000
    for batch, _ in coord.pop_due_authored(now):
        if batch.kind == "up":
            coord.request_releases(batch.intents)

    active_ids = {a.generation_id for a in coord.active_by_scan_code.values()}
    assert set(coord.pending_by_generation) <= active_ids
    for pending in coord.pending_by_generation.values():
        assert pending.scan_code in coord.active_by_scan_code
        assert coord.active_by_scan_code[pending.scan_code].generation_id == pending.generation_id


# ---------------------------------------------------------------------------
# Memory-bound regression tests
# ---------------------------------------------------------------------------


def test_bounded_by_polyphony_under_dense_song() -> None:
    """2000 alternating down/up actions on a 10-key pool: |S| ≤ |A|+|P|, peak ≤ 30."""
    pool = (21, 22, 23, 24, 25, 26, 27, 28, 29, 30)
    actions_list = []
    for i in range(2000):
        sc = pool[(i // 2) % len(pool)]
        actions_list.append(action(i * 20_000, "down" if i % 2 == 0 else "up", sc))
    sched = compile_runtime_intents(tuple(actions_list))
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=0)

    peak = _max_observed(coord)

    assert peak <= 2 * len(pool), f"peak |S| = {peak} exceeds 2 × |pool| = {2 * len(pool)}"
    assert not coord.status_by_generation, (
        f"expected empty live dict post-completion, got {len(coord.status_by_generation)} entries"
    )


def test_bounded_under_repeated_chords() -> None:
    """1000 repetitions of a 6-key chord: peak |S| ≤ 12 (= 2 × max polyphony)."""
    actions_list = []
    for i in range(1000):
        actions_list.append(action(i * 2_000, "down", 21, 22, 23, 24, 25, 26))
        actions_list.append(action(i * 2_000 + 1_000, "up", 21, 22, 23, 24, 25, 26))
    sched = compile_runtime_intents(tuple(actions_list))
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=100)

    peak = _max_observed(coord)

    assert peak <= 12, f"peak |S| = {peak} exceeds 12 (=2 × 6-key chord)"
    assert not coord.status_by_generation


def test_size_independent_of_note_count() -> None:
    """Peak |status_by_generation| for 100 notes == that for 10_000 notes (same polyphony)."""
    def peak_for(n: int) -> int:
        actions_list = []
        for i in range(n):
            actions_list.append(action(i * 20_000, "down", 7))
            actions_list.append(action(i * 20_000 + 1_000, "up", 7))
        sched = compile_runtime_intents(tuple(actions_list))
        coord = RuntimeDispatchCoordinator(sched, min_hold_us=0)
        return _max_observed(coord)

    p100 = peak_for(100)
    p10k = peak_for(10_000)

    assert p100 == p10k, (
        f"peak |S| grows with note_count: {p100} vs {p10k}"
    )
    assert p100 <= 2, f"trivial polyphony-1 song peaked at |S|={p100} > 2"


def test_no_stranded_entries_after_normal_completion() -> None:
    """Post-completion: status_by_generation empty, sum(terminal) == generation_count."""
    actions_list = tuple(
        action(i * 2_000, "down" if i % 2 == 0 else "up", 7)
        for i in range(2000)
    )
    sched = compile_runtime_intents(actions_list)
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=0)

    _max_observed(coord)

    assert not coord.status_by_generation, (
        f"expected empty live dict, got {len(coord.status_by_generation)} entries"
    )
    terminal_total = sum(coord._terminal_counts.values())
    assert terminal_total + len(coord.status_by_generation) == sched.generation_count, (
        f"sum(terminal) + |S| = {terminal_total} + {len(coord.status_by_generation)} "
        f"!= generation_count = {sched.generation_count}"
    )


def test_sum_counts_equals_generation_count() -> None:
    """At every drain cycle, sum(generation_status_counts()) == generation_count."""
    pool = (21, 22, 23)
    actions_list = []
    for i in range(300):
        sc = pool[i % len(pool)]
        actions_list.append(action(i * 20_000, "down", sc))
        actions_list.append(action(i * 20_000 + 10_000, "up", sc))

    sched = compile_runtime_intents(tuple(actions_list))
    coord = RuntimeDispatchCoordinator(sched, min_hold_us=0)

    now = 0
    cnt = 0
    while not coord.is_finished():
        deadline = coord.next_deadline_us()
        if deadline is None:
            break
        now = max(now, deadline)

        for pending in (coord.pop_due_pending(now),):
            if pending:
                sent = tuple(r.scan_code for r in pending)
                coord.complete_releases(pending, sent, ())

        for batch, _ in coord.pop_due_authored(now):
            if batch.kind == "up":
                coord.request_releases(batch.intents)
                newly = coord.pop_due_pending(now)
                if newly:
                    sent = tuple(r.scan_code for r in newly)
                    coord.complete_releases(newly, sent, ())
            else:
                playable, _ = coord.split_down_intents(batch.intents)
                if playable:
                    sent = tuple(i.scan_code for i in playable)
                    coord.activate_sent_downs(
                        playable,
                        sent,
                        dispatch_started_us=now,
                        dispatch_completed_us=now,
                    )

        counts = coord.generation_status_counts()
        total = sum(counts.values())
        assert total == sched.generation_count, (
            f"cycle {cnt}: sum(counts) = {total} != generation_count = {sched.generation_count}"
        )
        cnt += 1
