"""Differential test suite for Phase 2: Python oracle vs Rust sky_dispatch_core.

Compares Rust simulation traces against Python RuntimeDispatchCoordinator semantics.
"""

from __future__ import annotations

import json
import random
from typing import Any, cast

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]

from sky_music.domain.scheduler_types import (
    ActionKind,
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)


def _py_simulate(actions: tuple[KeyAction, ...], allowed_scan_codes: list[int], min_hold_us: int, send_latency_us: int) -> dict[str, Any]:
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us)

    events: list[dict[str, Any]] = []
    step = 0
    now_us = schedule.batches[0].scheduled_us if schedule.batches else 0

    while not coordinator.is_finished():
        next_dl = coordinator.next_deadline_us(0, 0)
        if next_dl is not None and next_dl > now_us:
            now_us = next_dl

        # 1. Pending releases
        due_pending = coordinator.pop_due_pending(now_us, 0)
        if due_pending:
            scan_codes = tuple(p.scan_code for p in due_pending)
            gen_ids = tuple(p.generation_id for p in due_pending)
            completed_us = now_us + send_latency_us
            coordinator.complete_releases(due_pending, scan_codes, ())

            events.append({
                "step": step,
                "kind": "up",
                "scheduled_us": due_pending[0].scheduled_release_us,
                "actual_us": now_us,
                "completed_us": completed_us,
                "scan_codes": list(scan_codes),
                "generation_ids": list(gen_ids),
                "outcome": "released",
            })
            step += 1
            now_us = completed_us
            continue

        # 2. Authored batch
        popped = coordinator.pop_next_due_authored(now_us, 0)
        if popped is not None:
            batch, _lead = popped
            if batch.kind == "down":
                playable, conflicts = coordinator.split_down_intents(batch.intents)
                if playable:
                    scan_codes = tuple(i.scan_code for i in playable)
                    gen_ids = tuple(i.generation_id for i in playable)
                    completed_us = now_us + send_latency_us
                    coordinator.activate_sent_downs(playable, scan_codes, dispatch_started_us=now_us, dispatch_completed_us=completed_us)

                    events.append({
                        "step": step,
                        "kind": "down",
                        "scheduled_us": batch.scheduled_us,
                        "actual_us": now_us,
                        "completed_us": completed_us,
                        "scan_codes": list(scan_codes),
                        "generation_ids": list(gen_ids),
                        "outcome": "sent",
                    })
                    step += 1
                    now_us = completed_us

                if conflicts:
                    scan_codes = tuple(i.scan_code for i in conflicts)
                    gen_ids = tuple(i.generation_id for i in conflicts)
                    events.append({
                        "step": step,
                        "kind": "down",
                        "scheduled_us": batch.scheduled_us,
                        "actual_us": now_us,
                        "completed_us": now_us,
                        "scan_codes": list(scan_codes),
                        "generation_ids": list(gen_ids),
                        "outcome": "dropped_conflict",
                    })
                    step += 1

            elif batch.kind == "up":
                _requested, suppressed = coordinator.request_releases(batch.intents)
                if suppressed:
                    scan_codes = tuple(i.scan_code for i in suppressed)
                    gen_ids = tuple(i.generation_id for i in suppressed)
                    events.append({
                        "step": step,
                        "kind": "up",
                        "scheduled_us": batch.scheduled_us,
                        "actual_us": now_us,
                        "completed_us": now_us,
                        "scan_codes": list(scan_codes),
                        "generation_ids": list(gen_ids),
                        "outcome": "suppressed_stale_up",
                    })
                    step += 1
        else:
            now_us += 100

    return {
        "events": events,
        "status_counts": coordinator.generation_status_counts(),
        "total_generations": schedule.generation_count,
        "is_finished": coordinator.is_finished(),
    }


def test_rust_differential_basic() -> None:
    actions = (
        KeyAction(at_us=Microseconds(1000), kind=ActionKind("down"), scan_codes=(ScanCode(1), ScanCode(2)), reason="chord"),
        KeyAction(at_us=Microseconds(2000), kind=ActionKind("up"), scan_codes=(ScanCode(1),), reason="rel1"),
        KeyAction(at_us=Microseconds(2100), kind=ActionKind("up"), scan_codes=(ScanCode(2),), reason="rel2"),
    )
    allowed = [1, 2]
    min_hold_us = 50
    send_latency_us = 10

    py_res = _py_simulate(actions, allowed, min_hold_us, send_latency_us)

    rs_inputs = [
        (idx, a.kind, int(a.at_us), list(a.scan_codes), a.reason)
        for idx, a in enumerate(actions)
    ]
    rs_json = cast(str, sky_player_rs.simulate_schedule_rs(rs_inputs, allowed, min_hold_us, send_latency_us))  # type: ignore[attr-defined]
    rs_res = cast(dict[str, Any], json.loads(rs_json))

    assert rs_res == py_res


def test_rust_differential_conflicts_and_stale() -> None:
    actions = (
        KeyAction(at_us=Microseconds(1000), kind=ActionKind("down"), scan_codes=(ScanCode(1),), reason="n1"),
        KeyAction(at_us=Microseconds(1010), kind=ActionKind("down"), scan_codes=(ScanCode(1),), reason="n1_conflict"),
        KeyAction(at_us=Microseconds(1200), kind=ActionKind("up"), scan_codes=(ScanCode(1),), reason="rel1"),
        KeyAction(at_us=Microseconds(1300), kind=ActionKind("up"), scan_codes=(ScanCode(1),), reason="stale_up"),
    )
    allowed = [1]
    min_hold_us = 50
    send_latency_us = 10

    with pytest.raises(ValueError, match="overlapping same-key down actions"):
        _py_simulate(actions, allowed, min_hold_us, send_latency_us)

    rs_inputs = [
        (idx, a.kind, int(a.at_us), list(a.scan_codes), a.reason)
        for idx, a in enumerate(actions)
    ]
    with pytest.raises(ValueError, match="overlapping same-key down actions"):
        sky_player_rs.simulate_schedule_rs(rs_inputs, allowed, min_hold_us, send_latency_us)  # type: ignore[attr-defined]


def test_rust_differential_seeded_schedule_corpus() -> None:
    rng = random.Random(20260730)
    allowed = [1, 2, 3, 4]

    for case_index in range(100):
        at_us = 0
        generated: list[KeyAction] = []
        for action_index in range(rng.randint(1, 40)):
            at_us += rng.randint(0, 2_000)
            scans = tuple(
                ScanCode(value)
                for value in rng.sample(allowed, rng.randint(1, len(allowed)))
            )
            generated.append(
                KeyAction(
                    at_us=Microseconds(at_us),
                    kind=ActionKind(rng.choice(("down", "up"))),
                    scan_codes=scans,
                    reason=f"case-{case_index}-action-{action_index}",
                )
            )
        actions = tuple(generated)
        min_hold_us = rng.randint(0, 5_000)
        send_latency_us = rng.randint(0, 500)

        try:
            py_result = _py_simulate(
                actions,
                allowed,
                min_hold_us,
                send_latency_us,
            )
        except ValueError as e:
            if "overlapping same-key down actions" in str(e):
                py_result = "ValueError"
            else:
                raise

        rust_inputs = [
            (index, action.kind, int(action.at_us), list(action.scan_codes), action.reason)
            for index, action in enumerate(actions)
        ]
        
        try:
            rust_result_json = cast(
                str,
                sky_player_rs.simulate_schedule_rs(  # type: ignore[attr-defined]
                    rust_inputs,
                    allowed,
                    min_hold_us,
                    send_latency_us,
                )
            )
            rust_result = cast(dict[str, Any], json.loads(rust_result_json))
        except ValueError as e:
            if "overlapping same-key down actions" in str(e):
                rust_result = "ValueError"
            else:
                raise

        assert rust_result == py_result, f"differential mismatch in case {case_index}"


def test_rust_simulation_rejects_bool_timing_values() -> None:
    actions = [(0, "down", 0, [1], "note")]
    with pytest.raises(TypeError, match="not bool"):
        sky_player_rs.simulate_schedule_rs(actions, [1], True, 0)  # type: ignore[attr-defined]
    with pytest.raises(TypeError, match="not bool"):
        sky_player_rs.simulate_schedule_rs(actions, [1], 0, True)  # type: ignore[attr-defined]
