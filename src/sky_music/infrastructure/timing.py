from dataclasses import dataclass
from typing import Protocol
import time

class Clock(Protocol):
    def now_us(self) -> int:
        """Returns the current monotonic system or simulation time in microseconds."""
        ...

class Sleeper(Protocol):
    def sleep(self, seconds: float) -> None:
        """Suspends execution for a specified duration in seconds."""
        ...

class PerfCounterClock:
    def now_us(self) -> int:
        return time.perf_counter_ns() // 1000

class RealSleeper:
    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)

@dataclass(frozen=True, slots=True)
class SleepPolicy:
    spin_threshold_us: int = 500
    poll_s: float = 0.025

# Fallback spin threshold when a caller does not pass one explicitly. Kept equal to
# SleepPolicy.spin_threshold_us so the bare-call default and the policy default never disagree;
# the playback engine always forwards the resolved policy value.
_DEFAULT_SPIN_THRESHOLD_US = SleepPolicy.spin_threshold_us

class PreciseSleeper:
    """Standardized high-precision sleeper using hybrid coarse, medium, yield, and spin phases."""
    def sleep_step_towards_us(self, target_us: int, clock: Clock, sleeper: Sleeper, spin_threshold_us: int = _DEFAULT_SPIN_THRESHOLD_US) -> None:
        """Sleeps a single step towards target_us, allowing the caller to perform frequent polling."""
        now = clock.now_us()
        remaining_us = target_us - now
        if remaining_us <= 0:
            return
        
        if remaining_us > 20_000:
            # Coarse sleep: yield to OS but wake up slightly early with a 5ms buffer
            # Cap to 20ms to allow outer loop hotkey/pause polling
            sleep_duration = min(20_000, remaining_us - 5_000)
            sleeper.sleep(sleep_duration / 1_000_000.0)
        elif remaining_us > 5_000:
            # Medium sleep: 1ms ticks
            sleeper.sleep(0.001)
        elif remaining_us > spin_threshold_us:
            # Tiny sleep: yield thread slice
            sleeper.sleep(0.0)
        else:
            # Busy wait / spin
            pass

    def sleep_until_us(self, target_us: int, clock: Clock, sleeper: Sleeper, spin_threshold_us: int = _DEFAULT_SPIN_THRESHOLD_US) -> None:
        """
        Looping wrapper that sleeps until target_us is reached.
        
        WARNING: This is a blocking call. During playback, do NOT use this directly in the main
        playback engine loop since it blocks hotkey polling, pause/resume checks, and state rendering.
        Instead, use PreciseSleeper.sleep_step_towards_us() in a loop alongside runtime control polls.
        This function is intended primarily for tests and simulation environments.
        """
        while clock.now_us() < target_us:
            self.sleep_step_towards_us(target_us, clock, sleeper, spin_threshold_us)


