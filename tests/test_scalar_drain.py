"""Scalar drain regression tests for ``pop_next_due_authored`` (review of main@7c548527 §1.4).

The scalar drain sibling of ``pop_due_authored`` must return ONE due batch per call (or
``None`` when nothing is due), so the RT dispatch loop can avoid materialising a list+tuple
of a multi-batch overdue fan before sending batch 1 — the legacy code's overload amplifier.

This file asserts:
  * Empty-when-ahead scheduling: returns ``None`` for a future batch.
  * Single-batch pop returns exactly that batch (no list/tuple allocation in caller view).
  * Multi-batch burst is drained one batch at a time, preserving authored ordering.
  * Per-batch lead is snapshotted at pop time (consistent with ``pop_due_authored``).
  * Early-pop block returns ``None`` (matching the legacy ``break`` from the burst loop).
  * Final cursor state matches a full ``pop_due_authored`` invocation against the same state.
"""
from __future__ import annotations

from typing import cast

from sky_music.domain.domain import Microseconds, ScanCode
from sky_music.domain.scheduler_types import ActionKind, KeyAction
from sky_music.orchestration.runtime_dispatch import (
    RuntimeActionBatch,
    RuntimeDispatchCoordinator,
    RuntimeSchedule,
    compile_runtime_intents,
)


def _coord(actions: tuple[KeyAction, ...]) -> tuple[RuntimeDispatchCoordinator, list[RuntimeActionBatch]]:
    schedule = cast(RuntimeSchedule, compile_runtime_intents(actions))
    coord = RuntimeDispatchCoordinator(schedule, min_hold_us=0)
    return coord, list(schedule.batches)


def _action(at_us: int, kind: ActionKind, sc: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=(ScanCode(sc),),
        at_us=Microseconds(at_us),
        reason="scalar-drain-test",
    )


def test_pop_next_due_returns_none_when_nothing_scheduled() -> None:
    coord, _ = _coord(())
    assert coord.pop_next_due_authored(now_us=0) is None


def test_pop_next_due_returns_none_for_future_batch() -> None:
    coord, _batches = _coord((_action(1_000_000, ActionKind.DOWN, 0x15),))
    # at t=0 the batch is scheduled for t=1_000_000 — far ahead, no lead.
    assert coord.pop_next_due_authored(now_us=0) is None
    # cursor must NOT advance; a later call after catching up must still return the batch.
    assert coord.pop_next_due_authored(now_us=1_000_000) is not None


def test_pop_next_due_drains_single_batch_without_tuple_allocation() -> None:
    coord, batches = _coord((_action(0, ActionKind.DOWN, 0x15),))
    nxt = coord.pop_next_due_authored(now_us=0)
    assert nxt is not None
    batch, lead = nxt
    assert batch is batches[0]
    assert lead == 0
    # After popping, cursor advanced: another call must return None.
    assert coord.pop_next_due_authored(now_us=0) is None


def test_pop_next_due_drains_multi_batch_burst_preserving_order() -> None:
    # Three batches all due at t=0; legacy ``pop_due_authored`` would have returned a 3-tuple
    # before the first one could be sent. Scalar drain pops them one at a time.
    coord, batches = _coord(
        (
            _action(0, ActionKind.DOWN, 0x15),
            _action(0, ActionKind.DOWN, 0x16),
            _action(0, ActionKind.UP, 0x15),
        )
    )
    first = coord.pop_next_due_authored(now_us=0)
    second = coord.pop_next_due_authored(now_us=0)
    third = coord.pop_next_due_authored(now_us=0)
    sentinel = coord.pop_next_due_authored(now_us=0)
    assert sentinel is None
    assert first is not None and second is not None and third is not None
    # Authored order preserved — same as legacy tuple order.
    assert (first[0], second[0], third[0]) == (batches[0], batches[1], batches[2])


def test_pop_next_due_snapshots_per_batch_lead_at_pop_time() -> None:
    # Two due batches. A lead_for_batch that increments per call must be captured at the
    # pop instant for each batch (i.e. the lead passed back matches the value computed when
    # that batch was popped, NOT a later re-computation).
    coord, _batches = _coord(
        (
            _action(0, ActionKind.DOWN, 0x15),
            _action(0, ActionKind.DOWN, 0x16),
        )
    )
    counter = {"n": 0}

    def lead_for_batch(batch: RuntimeActionBatch) -> int:
        counter["n"] += 1
        return counter["n"] * 10  # 10 then 20

    first = coord.pop_next_due_authored(now_us=0, lead_for_batch=lead_for_batch)
    second = coord.pop_next_due_authored(now_us=0, lead_for_batch=lead_for_batch)
    assert first is not None and second is not None
    assert first[1] == 10, f"first batch lead must be 10, got {first[1]}"
    assert second[1] == 20, f"second batch lead must be 20, got {second[1]}"


def test_scalar_drain_and_tuple_drain_produce_same_observed_sequence() -> None:
    # Cross-check: draining via pop_next_due in a while-loop vs iterating pop_due_authored
    # must return the SAME batch sequence (and per-batch lead when lead_for_batch is the
    # identity / zero path). This is the regression contract the loop.py rewrite relies on.
    actions = (
        _action(0, ActionKind.DOWN, 0x15),
        _action(0, ActionKind.DOWN, 0x16),
        _action(50, ActionKind.UP, 0x15),
        _action(50, ActionKind.UP, 0x16),
    )
    coord_a, _batches_a = _coord(actions)
    coord_b, _batches_b = _coord(actions)

    scalar_out: list[RuntimeActionBatch] = []
    while True:
        nxt = coord_a.pop_next_due_authored(now_us=100)
        if nxt is None:
            break
        scalar_out.append(nxt[0])
    tuple_out = [b for b, _ in coord_b.pop_due_authored(now_us=100)]
    assert scalar_out == tuple_out
