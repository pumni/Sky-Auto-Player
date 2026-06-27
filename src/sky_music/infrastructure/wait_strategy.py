from __future__ import annotations

import time
from typing import Protocol

from sky_music.infrastructure.timing import Clock, Sleeper, SleepPolicy


class WaitStrategy(Protocol):
    def spin_until_us(self, target_system_us: int, clock: Clock) -> None:
        """Busy-wait until target_system_us."""
        ...

    def wait_until_us(
        self,
        target_system_us: int,
        clock: Clock,
        sleeper: Sleeper,
        spin_threshold_us: int,
        policy: SleepPolicy,
        command_event: int | None = None,
    ) -> bool:
        """Suspends execution until target_system_us is reached or command_event is signaled.

        Returns True if interrupted by command_event, False otherwise.
        """
        ...


class HybridWaitStrategy:
    """Consolidated wait strategy managing coarse, medium, yield, spin, and Win32 event waits.

    Test seam: deterministic tests inject a subclass overriding ``spin_until_us`` (e.g. to advance
    a fake clock) via the PlaybackEngine ``wait_strategy`` constructor parameter. Production code
    must never special-case fake clocks here.
    """

    def __init__(self, enable_event_wait: bool = False) -> None:
        self.enable_event_wait = enable_event_wait

    def spin_until_us(self, target_system_us: int, clock: Clock) -> None:
        # Hot path: ns-based clocks avoid the division overhead per iteration by comparing
        # against target_ns directly. Mock clocks for tests set _ns_based = False.
        if getattr(clock, "_ns_based", False):
            tgt_ns = target_system_us * 1000
            # Bind the timer to a local once: this is the tightest loop in the program, so the
            # per-iteration LOAD_GLOBAL/LOAD_ATTR for `time.perf_counter_ns` is pure overhead.
            # Tighter iterations also mean finer-grained deadline detection (less overshoot).
            perf_counter_ns = time.perf_counter_ns
            while perf_counter_ns() < tgt_ns:
                pass
        else:
            now_us = clock.now_us
            while now_us() < target_system_us:
                pass

    def wait_until_us(
        self,
        target_system_us: int,
        clock: Clock,
        sleeper: Sleeper,
        spin_threshold_us: int,
        policy: SleepPolicy,
        command_event: int | None = None,
    ) -> bool:
        now = clock.now_us()
        remaining_us = target_system_us - now
        if remaining_us <= 0:
            return False

        if remaining_us <= spin_threshold_us:
            # Busy-wait phase
            self.spin_until_us(target_system_us, clock)
            return False

        # High-resolution waitable timer path. Capability flag, not class identity: any sleeper
        # that wakes with sub-millisecond accuracy may declare is_high_resolution = True.
        if getattr(sleeper, "is_high_resolution", False):
            guard = spin_threshold_us
            remaining_to_sleep = remaining_us - guard

            timer_handle = getattr(sleeper, "handle", None)
            if self.enable_event_wait and command_event is not None and timer_handle is not None:
                # Event-driven wait: sleep directly to target - guard, waking on timer or command
                # event. The supervisor signals command_event on commands and focus transitions.
                if remaining_to_sleep > 0:
                    from sky_music.platform.win32 import inputs

                    if inputs.set_waitable_timer_relative_us(timer_handle, remaining_to_sleep):
                        res = inputs.wait_for_multiple_objects(
                            (timer_handle, command_event),
                            inputs.INFINITE,
                        )
                        # Woken by command event (WAIT_OBJECT_0 + 1)
                        if res == inputs.WAIT_OBJECT_0 + 1:
                            return True
                    else:
                        sleeper.sleep(remaining_to_sleep / 1_000_000.0)

                # Spin remainder
                self.spin_until_us(target_system_us, clock)
                return False

            # Timer-aware sleep ladder: sleep towards target - guard in 1ms caps so the caller
            # can still poll commands between steps (polled mode).
            if remaining_to_sleep > 0:
                sleep_us = min(remaining_to_sleep, 1_000)
                sleeper.sleep(sleep_us / 1_000_000.0)
            return False

        # Fallback standard sleep ladder
        if remaining_us > policy.coarse_sleep_max_us:
            sleep_duration = min(policy.coarse_sleep_max_us, remaining_us - policy.coarse_sleep_threshold_us)
            sleeper.sleep(sleep_duration / 1_000_000.0)
        elif remaining_us > policy.coarse_sleep_threshold_us:
            sleeper.sleep(policy.medium_sleep_s)
        elif remaining_us > spin_threshold_us:
            sleeper.sleep(0.0)
        else:
            self.spin_until_us(target_system_us, clock)
        return False
