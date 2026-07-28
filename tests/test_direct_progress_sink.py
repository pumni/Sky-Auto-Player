"""DirectProgressSink contract: forwards counters to the renderer.

Regression for the ChatGPT-review finding A3: ``DirectProgressSink.publish`` had
``_ = counters`` and never invoked ``update_counters_batch`` on the renderer,
so direct-mode (ablation / non-threaded) playback lost the entire
observability contract that threaded mode honours by routing through the
snapshot sink → supervisor → renderer.update_counters_batch.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from sky_music.orchestration.core.ports import ProgressCounters
from sky_music.orchestration.playback_supervisor import DirectProgressSink


@dataclass
class _SpyRenderer:
    """Records ``render`` + ``update_counters_batch`` calls; both return None."""
    render_calls: list[dict[str, Any]]
    batch_calls: list[ProgressCounters | None]

    def __init__(self) -> None:
        self.render_calls = []
        self.batch_calls = []

    def render(
        self,
        elapsed: float,
        total: float,
        song_name: str,
        *,
        status: str = "playing",
        force: bool = False,
        input_path_degraded: bool = False,
        backend_health: object | None = None,
    ) -> None:
        self.render_calls.append(
            {
                "elapsed": elapsed,
                "total": total,
                "song_name": song_name,
                "status": status,
                "force": force,
                "input_path_degraded": input_path_degraded,
                "backend_health": backend_health,
            }
        )

    def update_counters_batch(self, counters: ProgressCounters) -> None:
        self.batch_calls.append(counters)

    def finish(self, message: str = "") -> None:
        return None


def test_direct_progress_sink_forwards_counters_to_renderer() -> None:
    """publish() with a counters kwarg forwards them to the renderer via
    ``update_counters_batch`` exactly once — mirroring the threaded-path
    contract that the supervisor honours via ``SnapshotProgressSink`` +
    ``_consume_progress_updates``.
    """
    renderer = _SpyRenderer()
    sink = DirectProgressSink(renderer=renderer, song_name="song")

    counters = ProgressCounters(
        max_lateness_us=4_500,
        late_2ms=2,
        late_5ms=1,
        late_10ms=0,
        release_max_us=3_000,
        release_late_2ms=5,
        recent_latencies_us=(4_500,),
    )
    sink.publish(
        elapsed_us=1_600_000,
        total_us=4_000_000,
        status="playing",
        counters=counters,
    )

    assert len(renderer.batch_calls) == 1, (
        f"expected exactly one update_counters_batch call on direct mode, "
        f"got {len(renderer.batch_calls)}"
    )
    assert renderer.batch_calls[0] is counters, (
        "the counters object forwarded must be the same instance that publish() received"
    )
    assert len(renderer.render_calls) == 1, "render() must still be called exactly once"
    assert renderer.render_calls[0]["status"] == "playing"


def test_direct_progress_sink_skips_batch_when_counters_none() -> None:
    """publish() without counters (e.g. supervisor-refocus publish calls that
    carry only status) must not invoke ``update_counters_batch`` and must still
    call ``render()`` exactly once. Mirrors the threaded supervisor path's
    behaviour when a publish omits the counters kwarg.
    """
    renderer = _SpyRenderer()
    sink = DirectProgressSink(renderer=renderer, song_name="song")

    sink.publish(
        elapsed_us=10_000,
        total_us=20_000,
        status="refocus",
        force=True,
    )

    assert renderer.batch_calls == [], (
        f"update_counters_batch must NOT fire when counters is None, got {renderer.batch_calls}"
    )
    assert len(renderer.render_calls) == 1
    assert renderer.render_calls[0]["status"] == "refocus"


def test_direct_progress_sink_skips_batch_when_renderer_lacks_batch_method() -> None:
    """A renderer without ``update_counters_batch`` (legacy console renderer,
    bare render protocol) must not crash: publish() silently skips batch
    forwarding and still calls ``render()``. Mirrors the threaded path's
    ``hasattr(renderer, "update_counters_batch")`` guard.
    """
    class _NoBatchRenderer:
        def __init__(self) -> None:
            self.render_calls = 0

        def render(
            self,
            elapsed: float,
            total: float,
            song_name: str,
            *,
            status: str = "playing",
            force: bool = False,
            input_path_degraded: bool = False,
            backend_health: object | None = None,
        ) -> None:
            self.render_calls += 1

        def finish(self, message: str = "") -> None:
            return None

    renderer = _NoBatchRenderer()
    sink = DirectProgressSink(renderer=renderer, song_name="song")

    counters = ProgressCounters(
        max_lateness_us=0,
        late_2ms=0,
        late_5ms=0,
        late_10ms=0,
        release_max_us=0,
        release_late_2ms=0,
        recent_latencies_us=(),
    )
    sink.publish(
        elapsed_us=0,
        total_us=10_000,
        status="playing",
        counters=counters,
    )

    assert renderer.render_calls == 1, "render() must still be called"
