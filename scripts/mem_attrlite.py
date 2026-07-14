"""Trace which engine attributes are heavy after play() returns.

A direct-drive coordinator is used for the memory-bound assertion because
``PlaybackEngine`` nulls ``_runtime_coordinator`` post-play (engine.py:814)
and real-time playback of 2000+ notes would require wall-clock seconds.
Instead the coordinator is driven directly with a virtual clock.
"""
from __future__ import annotations

from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.orchestration.runtime_dispatch import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)


def _action(at_us: int, kind: str, scan: int) -> KeyAction:
    return KeyAction(
        kind=kind, scan_codes=(ScanCode(scan),),  # type: ignore[arg-type]
        at_us=Microseconds(at_us), reason="test",
    )


def _drive(coord: RuntimeDispatchCoordinator) -> None:
    """Mirror engine dispatch to completion — virtual clock, no real wait."""
    now = 0
    while not coord.is_finished():
        deadline = coord.next_deadline_us()
        if deadline is None:
            break
        now = max(now, deadline)
        for pending in (coord.pop_due_pending(now),):
            if pending:
                sent = tuple(r.scan_code for r in pending)
                coord.complete_releases(pending, sent, ())
        for batch in coord.pop_due_authored(now):
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
                        playable, sent,
                        dispatch_started_us=now, dispatch_completed_us=now,
                    )


def main() -> None:
    scan_codes_pool = (21, 22, 23, 24, 25, 26, 27, 28, 29, 30)
    actions: list[KeyAction] = []
    for pair_idx in range(1000):
        sc = scan_codes_pool[pair_idx % len(scan_codes_pool)]
        actions.append(_action(pair_idx * 40_000, "down", sc))
        actions.append(_action(pair_idx * 40_000 + 20_000, "up", sc))

    schedule = compile_runtime_intents(tuple(actions))
    coord = RuntimeDispatchCoordinator(schedule, min_hold_us=0)
    _drive(coord)

    bound = 2 * len(scan_codes_pool)
    live_entries = len(coord.status_by_generation)
    terminal_total = sum(coord._terminal_counts.values())
    gen_count = coord._generation_count
    print(f"status_by_generation (live, non-terminal): {live_entries}  (bound: <={bound})")
    print(f"_terminal_counts:                         {dict(coord._terminal_counts)}")
    print(f"generation_count:                         {gen_count}")
    print(f"sum(terminal) + live / generation_count:  {terminal_total + live_entries} / {gen_count}")

    assert live_entries <= bound, (
        f"O(polyphony) bound violated: |S|={live_entries} > {bound}"
    )
    print("[ok] O(polyphony) bound holds.")
    assert terminal_total + live_entries == gen_count, (
        f"counter drift: terminal={terminal_total} + live={live_entries} "
        f"!= generation_count={gen_count}"
    )
    print("[ok] counter invariant holds.")

    # Verify via the public API as cross-check
    counts = coord.generation_status_counts()
    assert sum(counts.values()) == gen_count, (
        f"generation_status_counts sum = {sum(counts.values())} != {gen_count}"
    )
    print("[ok] generation_status_counts sum matches.")


if __name__ == "__main__":
    main()
