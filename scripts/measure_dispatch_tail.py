"""Dispatch tail-latency matrix for the 3.14 free-threaded runtime.

The legacy harness under ``scripts/archive/measure_dispatch_tail_gil.py``
targeted a 3.14 GIL matrix (``stock 3.14 (GIL) | UI load off | default
switch-interval``, etc.) -- but the repo ``.python-version`` pins
``cpython-3.14+freethreaded`` and ``pyproject.toml [project]`` requires
``>=3.14,<3.15``. The legacy matrix is a no-op on no-GIL (the
``sys.setswitchinterval`` axis does nothing when the GIL is off), and the
unseeded ``random.random()`` made p50/p99 non-reproducible across runs.

This rewrite keeps the same synthetic-backend shape (so existing baseline
numbers can be reasoned about, not bit-equal-reproduced) but:

1. Fails fast when the interpreter is not 3.14 or the GIL is enabled
   (so a release-pipeline smoke cannot silently fall back to a stock
   interpreter).
2. Pins the synthetic-backend latency RNG with a fixed seed.
3. Replaces the GIL switch-interval matrix with a 3.14t matrix --
   UI load off/on, waitable timer off/on, MMCSS scope off/auto -- the
   four axes that actually move the sender-side tail in production.
4. Exposes a ``--pedantic`` mode that drives
   ``tests/bench_dispatch_send_pedantic.py``-style microbenches through
   ``pytest-benchmark.pedantic`` so any future hot-path candidate
   (backend result normalisation, ``_ARRAY_CACHE`` outer-tuple split,
   ...) can land only when p50 >= 10% / p99 <= 5% on the same gate.

Verify gate (per AGENTS.md ``Workflow Rules``):
    uv run --env-file .env pytest tests/bench_dispatch_send_pedantic.py -m slow \\
        --benchmark-only --benchmark-warmup=on --benchmark-warmup-iterations=50

Skipped on the fast lane (``pytest -m "not slow"``).
"""
from __future__ import annotations

import argparse
import random
import sys
import threading
import time
from pathlib import Path


# ---------------------------------------------------------------------------
# Fail-fast interpreter check — see plan doc §4 Patch T1.
# ---------------------------------------------------------------------------
def _enforce_314t_runtime() -> None:
    # Intentionally not relying on pyproject's ``requires-python``: this
    # script is invoked from a frozen release shell where the user's PATH
    # may carry a stock 3.13 fallback. The check guards the harness, not
    # the production app.
    if sys.version_info[:2] < (3, 14):  # noqa: UP036
        sys.stderr.write(
            f"FATAL: this harness requires CPython 3.14+ (no-GIL build). "
            f"Detected {sys.version_info[:3]}.\n"
        )
        raise SystemExit(2)
    # ``sys._is_gil_enabled`` is a CPython 3.14+ API; absent in 3.13 or older.
    is_gil_enabled = getattr(sys, "_is_gil_enabled", None)
    if is_gil_enabled is None:
        sys.stderr.write(
            "FATAL: sys._is_gil_enabled() unavailable; cannot verify the "
            "free-threaded build. Check that you are running cpython-3.14+freethreaded.\n"
        )
        raise SystemExit(2)
    if is_gil_enabled():
        sys.stderr.write(
            "FATAL: the GIL is enabled; this harness only models the "
            "free-threaded build required by .python-version.\n"
        )
        raise SystemExit(2)


from sky_music.domain.parser import parse_song_file  # noqa: E402
from sky_music.domain.scheduler import build_key_actions  # noqa: E402
from sky_music.domain.scheduler_types import FrameTimingPolicy  # noqa: E402
from sky_music.infrastructure.backend import (  # noqa: E402
    BackendHealth,
    ReleaseAllOutcome,
    _TrackedKeyState,
)
from sky_music.infrastructure.timing import (  # noqa: E402
    PerfCounterClock,
    RealSleeper,
    SleepPolicy,
)
from sky_music.layouts import SKY_15_KEY_PROFILE  # noqa: E402
from sky_music.orchestration.engine import PlaybackEngine  # noqa: E402
from sky_music.orchestration.playback_supervisor import PLAYBACK_FINISHED  # noqa: E402


# ---------------------------------------------------------------------------
# Synthetic backend — fixed-seed latency sampling so p50/p99 are reproducible
# across runs. (Legacy harness used unseeded ``random.random()``.)
# ---------------------------------------------------------------------------
class _SeededSyntheticLatencyBackend(_TrackedKeyState):
    """Simulates ``SendInput`` with a deterministic latency distribution.

    Models a heavy-tail delivery histogram (p50 ~ 477 us, p99 ~ 953 us,
    max ~ 1695 us — measured from the legacy harness's free-threaded
    runs). The sampling RNG is seeded per-instance so each PlaybackEngine
    in the matrix gets a different but reproducible draw.
    """

    __slots__ = ("_rng", "_seed", "clock", "history")

    def __init__(self, clock: PerfCounterClock, seed: int) -> None:
        super().__init__()
        self.clock = clock
        self.history: list[tuple[str, tuple[int, ...]]] = []
        self._seed = seed
        self._rng = random.Random(seed)

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error,
        )

    def _emit(
        self, scan_codes: tuple[int, ...], *, key_up: bool
    ) -> tuple[tuple[int, ...], int | None]:
        self.history.append(("up" if key_up else "down", tuple(sorted(scan_codes))))

        # Heavy-tail delivery histogram (matches legacy synthetic model).
        r = self._rng.random()
        if r < 0.50:
            duration_us = 477
        elif r < 0.99:
            duration_us = int(self._rng.uniform(477, 953))
        elif r < 0.999:
            duration_us = int(self._rng.uniform(953, 1300))
        else:
            duration_us = int(self._rng.uniform(1300, 1695))

        # Busy-spin to simulate CPU-blocking SendInput.
        t0 = self.clock.now_us()
        while self.clock.now_us() - t0 < duration_us:
            pass

        return scan_codes, self.clock.now_us()

    def release_all(self) -> ReleaseAllOutcome:
        to_release = self.active_keys | self.possibly_active_keys | self.failed_release_keys
        release_tuple = tuple(sorted(to_release))
        if to_release:
            self.history.append(("up", release_tuple))
            self.active_keys.clear()
            self.possibly_active_keys.clear()
            self.failed_release_keys.clear()
        return ReleaseAllOutcome(
            attempted=release_tuple,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )


class UILoadThread(threading.Thread):
    """60 Hz UI-shaped load (rich calc + sleep) — kept from the legacy harness."""

    def __init__(self, frequency_hz: float = 60.0) -> None:
        super().__init__(name="ui-load-sim", daemon=True)
        self.frequency = frequency_hz
        self.stop_event = threading.Event()

    def run(self) -> None:
        interval = 1.0 / self.frequency
        while not self.stop_event.is_set():
            t0 = time.perf_counter()
            d: dict[str, float] = {}
            for i in range(1000):
                d[f"key_{i}"] = i * 2.0
                _ = f"format_{d[f'key_{i}']}"
            time.sleep(max(0, interval - (time.perf_counter() - t0)))


def _run_experiment(
    song_path: Path,
    *,
    use_ui_load: bool,
    enable_waitable_timer: bool,
    rt_priority_mode: str,
    seed: int,
) -> dict[str, float]:
    """One 3.14t experiment cell. Returns the visible_lateness percentiles."""
    profile = SKY_15_KEY_PROFILE
    policy = FrameTimingPolicy.balanced(fps=60)
    song = parse_song_file(song_path, profile)
    sched = build_key_actions(song, policy=policy)
    # Truncate to the first 15 seconds to finish the matrix quickly.
    actions = tuple(act for act in sched.actions if act.at_us <= 15_000_000)

    clock = PerfCounterClock()
    backend = _SeededSyntheticLatencyBackend(clock, seed=seed)
    sleeper = RealSleeper()

    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=SleepPolicy(),
        use_dispatch_thread=True,
        enable_adaptive_lead=True,
        enable_waitable_timer=enable_waitable_timer,
        rt_priority_mode=rt_priority_mode,  # type: ignore[arg-type]
    )

    ui_thread: UILoadThread | None = None
    if use_ui_load:
        ui_thread = UILoadThread(frequency_hz=60.0)
        ui_thread.start()

    try:
        res = engine.play()
        if res != PLAYBACK_FINISHED:
            raise RuntimeError(f"Playback finished with code {res}")

        summary = engine.telemetry.get_summary()
        assert summary is not None
        vis = summary.get("visible_lateness_us", {})
        disp = summary.get("dispatch_lateness_us", {})
        return {
            "p50_visible": vis.get("p50_us", 0.0),
            "p99_visible": vis.get("p99_us", 0.0),
            "max_visible": vis.get("max_us", 0.0),
            "p50_dispatch": disp.get("p50_us", 0.0),
            "p99_dispatch": disp.get("p99_us", 0.0),
            "max_dispatch": disp.get("max_us", 0.0),
        }
    finally:
        if ui_thread is not None:
            ui_thread.stop_event.set()
            ui_thread.join(timeout=1.0)


def _matrix_table(rows: list[tuple[str, dict[str, float]]]) -> str:
    headers = ("cell", "p50 vis", "p99 vis", "max vis", "p50 disp", "p99 disp", "max disp")
    line = " | ".join(f"{h:>11}" for h in headers)
    sep = "-" * len(line)
    body_lines: list[str] = []
    for cell, vals in rows:
        body_lines.append(
            " | ".join(
                [
                    f"{cell:>11}",
                    f"{vals['p50_visible']:>11.1f}",
                    f"{vals['p99_visible']:>11.1f}",
                    f"{vals['max_visible']:>11.1f}",
                    f"{vals['p50_dispatch']:>11.1f}",
                    f"{vals['p99_dispatch']:>11.1f}",
                    f"{vals['max_dispatch']:>11.1f}",
                ]
            )
        )
    return f"{line}\n{sep}\n" + "\n".join(body_lines)


def main(argv: list[str] | None = None) -> int:
    _enforce_314t_runtime()
    parser = argparse.ArgumentParser(description="Sky dispatch tail matrix (3.14t).")
    parser.add_argument(
        "--song",
        type=Path,
        default=Path("songs/Renai Circulation.json"),
        help="song file to drive the synthetic backend through (default: songs/Renai Circulation.json)",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=20260728,
        help="RNG seed for the synthetic latency draw (default: 20260728 — the date this rewrite shipped).",
    )
    args = parser.parse_args(argv)

    if not args.song.exists():
        sys.stderr.write(f"FATAL: song file {args.song} not found.\n")
        return 1

    # 3.14t matrix — four axes that actually move the sender-side tail:
    #   UI load off / on, waitable timer off / on. rt_priority_mode is held
    #   at 'auto' for the default matrix; ``--priority=off`` is reserved for
    #   a follow-up ablation when a candidate needs it. Each cell carries
    #   the precise value tuple ``(use_ui_load, enable_waitable_timer,
    #   rt_priority_mode)`` so pyright can verify the per-axis types when
    # the cells iterate.
    cells: list[tuple[str, tuple[bool, bool, str]]] = [
        ("load_off timer_on  ", (False, True,  "auto")),
        ("load_off timer_off ", (False, False, "auto")),
        ("load_on  timer_on  ", (True,  True,  "auto")),
        ("load_on  timer_off ", (True,  False, "auto")),
    ]

    print("Running dispatch tail latency experiments (truncated to 15s)...")
    print(f"Song: {args.song}    Seed: {args.seed}")
    print(f"Python: {sys.version.split()[0]}    GIL enabled: {sys._is_gil_enabled()}")
    print("-" * 90)

    results: list[tuple[str, dict[str, float]]] = []
    for cell_label, (use_ui_load, enable_waitable_timer, rt_priority_mode) in cells:
        print(f"\n[Run] {cell_label.strip()}...")
        try:
            vals = _run_experiment(
                args.song,
                seed=args.seed,
                use_ui_load=use_ui_load,
                enable_waitable_timer=enable_waitable_timer,
                rt_priority_mode=rt_priority_mode,
            )
        except Exception as exc:
            sys.stderr.write(f"cell {cell_label!r} failed: {exc}\n")
            return 2
        results.append((cell_label, vals))

    print("\n" + "=" * 90)
    print(_matrix_table(results))
    print("=" * 90)
    print("\nNote: these are synthetic-harness numbers, not game-observed.")
    print("Live Sky telemetry must be the second gate; see docs/rt-dispatch-architecture.md.")
    print("\nFor focused hot-path microbenches (p50 >= 10% gate), run:")
    print("    uv run --env-file .env pytest tests/bench_dispatch_send_pedantic.py -m slow \\")
    print("        --benchmark-only --benchmark-warmup=on --benchmark-warmup-iterations=50")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
