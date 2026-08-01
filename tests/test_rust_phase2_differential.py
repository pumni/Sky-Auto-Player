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


# ==========================================================================
# P3.3 — Expanded differential oracle scenarios
# ==========================================================================

def _rs_simulate(
    actions: tuple[KeyAction, ...],
    allowed: list[int],
    min_hold_us: int,
    send_latency_us: int,
) -> dict[str, Any] | str:
    """Run Rust simulation, return result dict or 'ValueError' on compile error."""
    rust_inputs = [
        (idx, a.kind, int(a.at_us), list(a.scan_codes), a.reason)
        for idx, a in enumerate(actions)
    ]
    try:
        rs_json = cast(
            str,
            sky_player_rs.simulate_schedule_rs(  # type: ignore[attr-defined]
                rust_inputs, allowed, min_hold_us, send_latency_us
            ),
        )
        return cast(dict[str, Any], json.loads(rs_json))
    except ValueError as e:
        if "overlapping same-key down actions" in str(e):
            return "ValueError"
        raise


def _py_simulate_safe(
    actions: tuple[KeyAction, ...],
    allowed: list[int],
    min_hold_us: int,
    send_latency_us: int,
) -> dict[str, Any] | str:
    """Run Python simulation, return result dict or 'ValueError' on compile error."""
    try:
        return _py_simulate(actions, allowed, min_hold_us, send_latency_us)
    except ValueError as e:
        if "overlapping same-key down actions" in str(e):
            return "ValueError"
        raise


def _assert_differential_match(
    actions: tuple[KeyAction, ...],
    allowed: list[int],
    min_hold_us: int,
    send_latency_us: int,
    label: str = "",
) -> None:
    py = _py_simulate_safe(actions, allowed, min_hold_us, send_latency_us)
    rs = _rs_simulate(actions, allowed, min_hold_us, send_latency_us)
    assert rs == py, (
        f"differential mismatch{' in ' + label if label else ''}\n"
        f"py={py!r}\nrs={rs!r}"
    )


# ---------------------------------------------------------------------------
# P3.3a — Large chord (up to 15 keys)
# ---------------------------------------------------------------------------


def test_differential_large_chord_polyphony() -> None:
    """Chord size up to 15 keys: Python and Rust must agree on generation lifecycle."""
    # Use 15 distinct scan codes for max polyphony.
    allowed = list(range(1, 16))  # 1..15
    actions = (
        KeyAction(at_us=Microseconds(0), kind=ActionKind("down"), scan_codes=tuple(ScanCode(c) for c in allowed), reason="poly15"),
        KeyAction(at_us=Microseconds(500), kind=ActionKind("up"), scan_codes=tuple(ScanCode(c) for c in allowed), reason="rel-poly15"),
    )
    _assert_differential_match(actions, allowed, min_hold_us=0, send_latency_us=10, label="large-chord-15")


def test_differential_large_chord_with_min_hold() -> None:
    """Large chord + min-hold: release deadline must agree in both impls."""
    allowed = list(range(1, 9))
    actions = (
        KeyAction(at_us=Microseconds(0), kind=ActionKind("down"), scan_codes=tuple(ScanCode(c) for c in allowed), reason="chord8"),
        KeyAction(at_us=Microseconds(100), kind=ActionKind("up"), scan_codes=tuple(ScanCode(c) for c in allowed), reason="rel8"),
    )
    _assert_differential_match(actions, allowed, min_hold_us=5_000, send_latency_us=20, label="large-chord-min-hold")


# ---------------------------------------------------------------------------
# P3.3b — Repeated key after release
# ---------------------------------------------------------------------------


def test_differential_repeated_key_after_release() -> None:
    """Same key played twice (Down→Up→Down→Up): life-cycle must match."""
    allowed = [1, 2]
    actions = (
        KeyAction(at_us=Microseconds(0), kind=ActionKind("down"), scan_codes=(ScanCode(1),), reason="d1"),
        KeyAction(at_us=Microseconds(200), kind=ActionKind("up"), scan_codes=(ScanCode(1),), reason="u1"),
        KeyAction(at_us=Microseconds(400), kind=ActionKind("down"), scan_codes=(ScanCode(1),), reason="d2"),
        KeyAction(at_us=Microseconds(600), kind=ActionKind("up"), scan_codes=(ScanCode(1),), reason="u2"),
    )
    _assert_differential_match(actions, allowed, min_hold_us=50, send_latency_us=5, label="repeated-key")


def test_differential_repeated_key_many_cycles() -> None:
    """Key played 5 times in sequence: all cycles tracked correctly."""
    allowed = [3]
    parts: list[KeyAction] = []
    for i in range(5):
        base = i * 300
        parts.append(KeyAction(at_us=Microseconds(base), kind=ActionKind("down"), scan_codes=(ScanCode(3),), reason=f"d{i}"))
        parts.append(KeyAction(at_us=Microseconds(base + 100), kind=ActionKind("up"), scan_codes=(ScanCode(3),), reason=f"u{i}"))
    _assert_differential_match(tuple(parts), allowed, min_hold_us=0, send_latency_us=5, label="repeated-5-cycles")


# ---------------------------------------------------------------------------
# P3.3c — Min-hold stress
# ---------------------------------------------------------------------------


def test_differential_min_hold_exceeds_authored_duration() -> None:
    """When min_hold_us > authored release time, effective release must be delayed."""
    allowed = [1]
    actions = (
        KeyAction(at_us=Microseconds(0), kind=ActionKind("down"), scan_codes=(ScanCode(1),), reason="d"),
        KeyAction(at_us=Microseconds(100), kind=ActionKind("up"), scan_codes=(ScanCode(1),), reason="u"),
    )
    # min_hold_us (10000) >> authored interval (100): effective release moves to ~10000
    _assert_differential_match(actions, allowed, min_hold_us=10_000, send_latency_us=50, label="min-hold-dominates")


def test_differential_min_hold_zero_high_latency() -> None:
    """min_hold_us=0 but send_latency_us very high: effective release moves to latency."""
    allowed = [1, 2]
    actions = (
        KeyAction(at_us=Microseconds(0), kind=ActionKind("down"), scan_codes=(ScanCode(1), ScanCode(2)), reason="d"),
        KeyAction(at_us=Microseconds(50), kind=ActionKind("up"), scan_codes=(ScanCode(1), ScanCode(2)), reason="u"),
    )
    _assert_differential_match(actions, allowed, min_hold_us=0, send_latency_us=5_000, label="high-latency-no-min-hold")


# ---------------------------------------------------------------------------
# P3.3d — Seeded random corpus with large chords (P3.3 extension)
# ---------------------------------------------------------------------------


def test_differential_seeded_large_chord_corpus() -> None:
    """Random corpus with chord sizes up to 15, repeated keys, varied min-hold."""
    rng = random.Random(20260801)
    # Up to 15 distinct keys
    all_allowed = list(range(1, 16))
    failures: list[str] = []

    for case_index in range(50):
        chord_pool_size = rng.randint(1, 15)
        allowed = all_allowed[:chord_pool_size]
        at_us = 0

        # Build a valid authored schedule: Down always followed by Up before next Down
        open_keys: set[int] = set()
        parts: list[KeyAction] = []
        for _ in range(rng.randint(2, 30)):
            at_us += rng.randint(50, 2_000)
            if open_keys and (rng.random() < 0.5 or len(open_keys) >= chord_pool_size):
                # Up for some open keys
                n_release = rng.randint(1, len(open_keys))
                releasing = rng.sample(sorted(open_keys), n_release)
                parts.append(KeyAction(
                    at_us=Microseconds(at_us),
                    kind=ActionKind("up"),
                    scan_codes=tuple(ScanCode(c) for c in releasing),
                    reason=f"u-{case_index}",
                ))
                open_keys -= set(releasing)
            else:
                # Down for keys not currently open
                available = [c for c in allowed if c not in open_keys]
                if not available:
                    continue
                n_down = rng.randint(1, min(len(available), chord_pool_size))
                new_keys = rng.sample(available, n_down)
                parts.append(KeyAction(
                    at_us=Microseconds(at_us),
                    kind=ActionKind("down"),
                    scan_codes=tuple(ScanCode(c) for c in sorted(new_keys)),
                    reason=f"d-{case_index}",
                ))
                open_keys |= set(new_keys)

        # Close any remaining open keys
        if open_keys:
            at_us += rng.randint(50, 500)
            parts.append(KeyAction(
                at_us=Microseconds(at_us),
                kind=ActionKind("up"),
                scan_codes=tuple(ScanCode(c) for c in sorted(open_keys)),
                reason=f"close-{case_index}",
            ))

        if len(parts) < 2:
            continue

        actions = tuple(parts)
        min_hold_us = rng.randint(0, 5_000)
        send_latency_us = rng.randint(0, 1_000)

        py = _py_simulate_safe(actions, allowed, min_hold_us, send_latency_us)
        rs = _rs_simulate(actions, allowed, min_hold_us, send_latency_us)
        if rs != py:
            failures.append(
                f"case {case_index} (chord_pool={chord_pool_size}, min_hold={min_hold_us}, "
                f"latency={send_latency_us}, actions={len(actions)})"
            )

    assert not failures, "differential mismatches:\n" + "\n".join(failures)


# ---------------------------------------------------------------------------
# P3.3e — Terminal state: all generators must be accounted for
# ---------------------------------------------------------------------------


def test_differential_terminal_state_all_generations_counted() -> None:
    """After simulation, both Python and Rust must agree on total_generations
    and that status_counts sum == total_generations."""
    rng = random.Random(77777)
    allowed = [1, 2, 3]

    for case_index in range(30):
        at_us = 0
        open_keys: set[int] = set()
        parts: list[KeyAction] = []

        for _ in range(rng.randint(3, 20)):
            at_us += rng.randint(100, 1_000)
            if open_keys:
                releasing = list(open_keys)
                parts.append(KeyAction(
                    at_us=Microseconds(at_us),
                    kind=ActionKind("up"),
                    scan_codes=tuple(ScanCode(c) for c in sorted(releasing)),
                    reason=f"u",
                ))
                open_keys.clear()
            available = [c for c in allowed if c not in open_keys]
            if available:
                n = rng.randint(1, len(available))
                new_keys = rng.sample(available, n)
                parts.append(KeyAction(
                    at_us=Microseconds(at_us),
                    kind=ActionKind("down"),
                    scan_codes=tuple(ScanCode(c) for c in sorted(new_keys)),
                    reason=f"d",
                ))
                open_keys |= set(new_keys)

        if open_keys:
            at_us += 500
            parts.append(KeyAction(
                at_us=Microseconds(at_us),
                kind=ActionKind("up"),
                scan_codes=tuple(ScanCode(c) for c in sorted(open_keys)),
                reason="close",
            ))

        if len(parts) < 2:
            continue

        actions = tuple(parts)
        py = _py_simulate_safe(actions, allowed, 0, 0)
        rs = _rs_simulate(actions, allowed, 0, 0)

        if isinstance(py, dict) and isinstance(rs, dict):
            # Both must agree on terminal state
            assert rs["is_finished"] == py["is_finished"], f"case {case_index}: is_finished mismatch"
            assert rs["total_generations"] == py["total_generations"], f"case {case_index}: total_generations mismatch"
            rs_sum = sum(rs["status_counts"].values())
            py_sum = sum(py["status_counts"].values())
            assert rs_sum == rs["total_generations"], f"case {case_index}: Rust count sum mismatch"
            assert py_sum == py["total_generations"], f"case {case_index}: Python count sum mismatch"
