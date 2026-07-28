"""Patch C3: ``QueueCommandSource.poll`` drops redundant ``empty()``.

The legacy implementation called ``queue.empty()`` as a pre-check
before ``get_nowait()`` and still caught ``queue.Empty``. The check is
an advisory snapshot of zero practical value -- the only correct
synchronisation point is the ``Empty`` exception the queue raises
when ``get_nowait()`` lands in the empty window.

Verify:

1. empty queue -> ``poll()`` returns ``None``.
2. one command enqueued -> ``poll()`` returns it; the next ``poll()``
   returns ``None`` (no double-dequeue).
3. concurrent producer / consumer loop: every enqueued command is
   dequeued exactly once, no drop, no dup. The legacy ``empty()``-
   then-``get_nowait()`` shape was equivalent in steady state; the
   simplification is observable only by the absence of the redundant
   synchronized op under no-GIL.
"""

from __future__ import annotations

import queue
import threading

from sky_music.orchestration.playback_supervisor import QueueCommandSource


def test_poll_returns_none_on_empty_queue() -> None:
    q: queue.Queue[str] = queue.Queue()
    src = QueueCommandSource(q)
    assert src.poll() is None


def test_poll_dequeues_exactly_once_per_command() -> None:
    q: queue.Queue[str] = queue.Queue()
    src = QueueCommandSource(q)
    q.put("pause")
    assert src.poll() == "pause"
    assert src.poll() is None
    assert q.qsize() == 0


def test_poll_under_concurrent_producer_consumer() -> None:
    """Producer / consumer race: every enqueued command is delivered exactly
    once. The simplified ``get_nowait()`` only branch must not lose or
    duplicate events against the legacy ``empty()``-then-``get_nowait()``
    shape.
    """
    q: queue.Queue[int] = queue.Queue()
    src = QueueCommandSource(q)
    total = 5_000
    produced: list[int] = []

    def producer() -> None:
        for i in range(total):
            q.put(i)
            produced.append(i)

    consumed: list[int] = []

    def consumer() -> None:
        while True:
            cmd = src.poll()
            if cmd is None:
                # Queue momentarily empty -- brief yield so the producer
                # can race us. ``queue.Empty`` propagates up to here.
                if len(consumed) >= total:
                    return
                continue
            consumed.append(int(cmd))

    t_prod = threading.Thread(target=producer)
    t_cons = threading.Thread(target=consumer)
    t_prod.start()
    t_cons.start()
    t_prod.join(timeout=5.0)
    t_cons.join(timeout=5.0)
    assert len(consumed) == total, (
        f"consumer saw {len(consumed)}/{total} commands -- "
        f"simplified poll dropped events"
    )
    assert sorted(consumed) == list(range(total)), (
        "consumer received commands out of order or with duplicates"
    )


def test_poll_swallows_empty_after_drain() -> None:
    """A queue that drained to empty during polling must not raise -- the
    ``except queue.Empty`` branch must handle the in-flight empty window
    so the supervisor tick continues without an exception escaping.
    """
    q: queue.Queue[str] = queue.Queue()
    src = QueueCommandSource(q)
    q.put("quit")
    assert src.poll() == "quit"
    # Subsequent polls observe the empty queue via the exception path.
    for _ in range(5):
        assert src.poll() is None
