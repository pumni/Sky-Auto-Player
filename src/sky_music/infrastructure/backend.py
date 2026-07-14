import contextlib
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Protocol


@dataclass(frozen=True, slots=True)
class ReleaseAllOutcome:
    attempted: tuple[int, ...]
    released_successfully: bool
    stuck_keys: tuple[int, ...]
    verification_inconclusive: bool

@dataclass(frozen=True, slots=True)
class BackendHealth:
    active_count: int
    possibly_active_count: int
    failed_release_count: int
    last_error: str | None
    min_same_key_up_gap_us: int | None = None
    impossible_same_key_repeats: int = 0
    send_while_unfocused: int = 0


@dataclass(frozen=True, slots=True)
class InputSendResult:
    """Structured result for a single key_down or key_up call."""
    # Scan codes that were sent to the OS (after deduplication)
    sent: tuple[int, ...]
    # Scan codes skipped because they were already in the desired state
    # (duplicate-down protection or release-idempotency)
    skipped_duplicates: tuple[int, ...]
    # Whether the underlying SendInput call succeeded
    success: bool
    # Optional diagnostic message on failure
    error: str | None = None
    # Raw perf_counter µs right after SendInput returned (before backend bookkeeping).
    # None if the backend cannot provide this (e.g. DryRunBackend or no clock available).
    send_completed_us: int | None = None

class InputBackend(Protocol):
    """Protocol interface defining operations for keyboard note key injections."""
    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        """Presses down a set of keyboard keys simultaneously."""
        ...

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        """Releases a set of keyboard keys simultaneously."""
        ...

    def release_all(self) -> ReleaseAllOutcome:
        """Safely releases all currently held keys."""
        ...

    def get_health(self) -> BackendHealth:
        """Returns the current health telemetry of the input backend."""
        ...

    def get_send_diagnostics(self) -> dict[str, int]:
        """Returns partial-send counters since the last reset (chord-atomicity diagnostics)."""
        ...


class _TrackedKeyState(ABC):
    """Key-state tracker with free-threaded-friendly hot paths.

    Phase B (polyphony slope): the dominant cost under no-GIL was per-key Python work —
    building intermediate lists/tuples on every chord and ``set.update`` /
    ``difference_update`` on every send. The decide helpers below reuse the caller's
    tuple whenever the batch is uniform (all free / all held / all already up), allocate
    only on the rare mixed case, and ``key_down``/``key_up`` use scalar ``add``/``discard``
    for the single-key path that carries melodic lines.
    """

    __slots__ = ("active_keys", "failed_release_keys", "last_error", "possibly_active_keys")

    active_keys: set[int]
    possibly_active_keys: set[int]
    failed_release_keys: set[int]
    last_error: str | None

    def __init__(self) -> None:
        self.active_keys: set[int] = set()
        self.possibly_active_keys: set[int] = set()
        self.failed_release_keys: set[int] = set()
        self.last_error: str | None = None

    def _decide_down(self, scan_codes: tuple[int, ...]) -> tuple[tuple[int, ...], tuple[int, ...]]:
        """Return (to_send, duplicates). Reuses *scan_codes* when the batch is uniform."""
        if not scan_codes:
            return (), ()
        active = self.active_keys
        if not active:
            return scan_codes, ()

        n = len(scan_codes)
        if n == 1:
            # Melodic single-note path: zero list/tuple allocation.
            if scan_codes[0] in active:
                return (), scan_codes
            return scan_codes, ()

        # Multi-key: classify without allocating; only split when mixed (rare in real songs).
        any_active = False
        any_free = False
        for sc in scan_codes:
            if sc in active:
                any_active = True
            else:
                any_free = True
            if any_active and any_free:
                to_send: list[int] = []
                duplicates: list[int] = []
                for sc2 in scan_codes:
                    (duplicates if sc2 in active else to_send).append(sc2)
                return tuple(to_send), tuple(duplicates)
        if any_active:
            return (), scan_codes
        return scan_codes, ()

    def _decide_up(self, scan_codes: tuple[int, ...]) -> tuple[tuple[int, ...], tuple[int, ...]]:
        """Return (to_release, already_released). Reuses *scan_codes* when uniform."""
        if not scan_codes:
            return (), ()
        active = self.active_keys
        possibly = self.possibly_active_keys
        if not active and not possibly:
            return (), scan_codes

        n = len(scan_codes)
        if n == 1:
            sc = scan_codes[0]
            if sc in active or sc in possibly:
                return scan_codes, ()
            return (), scan_codes

        any_held = False
        any_free = False
        for sc in scan_codes:
            if sc in active or sc in possibly:
                any_held = True
            else:
                any_free = True
            if any_held and any_free:
                to_release: list[int] = []
                already_released: list[int] = []
                for sc2 in scan_codes:
                    (
                        to_release
                        if (sc2 in active or sc2 in possibly)
                        else already_released
                    ).append(sc2)
                return tuple(to_release), tuple(already_released)
        if any_held:
            return scan_codes, ()
        return (), scan_codes

    def get_send_diagnostics(self) -> dict[str, int]:
        """Default: no partial-send instrumentation (overridden by the real SendInput backend)."""
        return {
            "partial_send_events": 0,
            "chord_split_events": 0,
            "keys_deferred": 0,
            "keys_dropped": 0,
            "keys_retried": 0,
            "zero_progress_retries": 0,
            "send_while_unfocused": 0,
            "impossible_same_key_repeats": 0,
        }

    @abstractmethod
    def _emit(
        self, scan_codes: tuple[int, ...], *, key_up: bool
    ) -> tuple[tuple[int, ...], int | None]:
        """Inject keys. Returns (actually_sent_scan_codes, send_completed_us).

        ``actually_sent_scan_codes`` is a prefix of ``scan_codes`` matching what the OS
        injected. Note-on may be a strict prefix (musical no-retry policy); note-off
        should normally return the full tuple after remainder completion.
        """
        ...

    def _handle_down_error(self, scan_codes: tuple[int, ...], error: Exception) -> None:  # noqa: ARG002
        self.last_error = f"key_down error: {error}"

    def _handle_up_error(self, scan_codes: tuple[int, ...], error: Exception) -> None:
        self.failed_release_keys.update(scan_codes)
        self.last_error = f"key_up error: {error}"

    def _mark_down_pending(self, scan_codes: tuple[int, ...]) -> None:
        if len(scan_codes) == 1:
            self.possibly_active_keys.add(scan_codes[0])
        else:
            self.possibly_active_keys.update(scan_codes)

    def _clear_down_pending(self, scan_codes: tuple[int, ...]) -> None:
        if len(scan_codes) == 1:
            self.possibly_active_keys.discard(scan_codes[0])
        else:
            self.possibly_active_keys.difference_update(scan_codes)

    def _commit_down_sent(self, actually_sent: tuple[int, ...]) -> None:
        if not actually_sent:
            return
        if len(actually_sent) == 1:
            self.active_keys.add(actually_sent[0])
        else:
            self.active_keys.update(actually_sent)

    def _commit_up_sent(self, actually_sent: tuple[int, ...]) -> None:
        if not actually_sent:
            return
        if len(actually_sent) == 1:
            sc = actually_sent[0]
            self.active_keys.discard(sc)
            self.possibly_active_keys.discard(sc)
            self.failed_release_keys.discard(sc)
        else:
            self.active_keys.difference_update(actually_sent)
            self.possibly_active_keys.difference_update(actually_sent)
            self.failed_release_keys.difference_update(actually_sent)

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)

        # Single-key melodic path: skip decide helpers and multi-set bookkeeping entirely.
        if len(scan_codes) == 1:
            sc = scan_codes[0]
            if sc in self.active_keys:
                return InputSendResult(sent=(), skipped_duplicates=scan_codes, success=True)
            self.possibly_active_keys.add(sc)
            try:
                actually_sent, send_completed_us = self._emit(scan_codes, key_up=False)
            except Exception as error:
                self._handle_down_error(scan_codes, error)
                raise
            if actually_sent:
                self.active_keys.add(sc)
            self.possibly_active_keys.discard(sc)
            full = bool(actually_sent)
            return InputSendResult(
                sent=actually_sent,
                skipped_duplicates=(),
                success=full,
                error=None if full else "partial note-on: 0/1",
                send_completed_us=send_completed_us,
            )

        to_send, duplicates = self._decide_down(scan_codes)
        if not to_send:
            return InputSendResult(sent=(), skipped_duplicates=duplicates, success=True)

        self._mark_down_pending(to_send)
        try:
            actually_sent, send_completed_us = self._emit(to_send, key_up=False)
        except Exception as error:
            self._handle_down_error(to_send, error)
            raise

        # Track only keys the OS actually injected. Unsent prefix-tail is dropped by the
        # musical no-retry policy — do not mark them active (would invent stuck state).
        self._commit_down_sent(actually_sent)
        self._clear_down_pending(to_send)
        full = len(actually_sent) == len(to_send)
        return InputSendResult(
            sent=actually_sent,
            skipped_duplicates=duplicates,
            success=full,
            error=None if full else f"partial note-on: {len(actually_sent)}/{len(to_send)}",
            send_completed_us=send_completed_us,
        )

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)

        if len(scan_codes) == 1:
            sc = scan_codes[0]
            if sc not in self.active_keys and sc not in self.possibly_active_keys:
                return InputSendResult(sent=(), skipped_duplicates=scan_codes, success=True)
            try:
                actually_sent, send_completed_us = self._emit(scan_codes, key_up=True)
            except Exception as error:
                self._handle_up_error(scan_codes, error)
                raise
            if actually_sent:
                self.active_keys.discard(sc)
                self.possibly_active_keys.discard(sc)
                self.failed_release_keys.discard(sc)
            elif sc in self.active_keys or sc in self.possibly_active_keys:
                self.failed_release_keys.add(sc)
                self.last_error = "partial note-off: 0/1"
            full = bool(actually_sent)
            return InputSendResult(
                sent=actually_sent,
                skipped_duplicates=(),
                success=full,
                error=None if full else "partial note-off: 0/1",
                send_completed_us=send_completed_us,
            )

        to_release, already_released = self._decide_up(scan_codes)
        if not to_release:
            return InputSendResult(sent=(), skipped_duplicates=already_released, success=True)

        try:
            actually_sent, send_completed_us = self._emit(to_release, key_up=True)
        except Exception as error:
            self._handle_up_error(to_release, error)
            raise

        self._commit_up_sent(actually_sent)
        # If a safety-path emit still left keys out (should be rare), keep them failed
        # so release_all can reclaim them. Emitters return a prefix of to_release.
        if len(actually_sent) < len(to_release):
            self.failed_release_keys.update(to_release[len(actually_sent) :])
            self.last_error = f"partial note-off: {len(actually_sent)}/{len(to_release)}"
        full = len(actually_sent) == len(to_release)
        return InputSendResult(
            sent=actually_sent,
            skipped_duplicates=already_released,
            success=full,
            error=None if full else f"partial note-off: {len(actually_sent)}/{len(to_release)}",
            send_completed_us=send_completed_us,
        )


_watchdog_proc = None
_watchdog_thread = None

def _start_watchdog_once():
    global _watchdog_proc, _watchdog_thread
    if _watchdog_proc is not None:
        return
    import atexit
    import subprocess
    import sys
    import threading

    # CREATE_NO_WINDOW = 0x08000000
    creationflags = 0x08000000 if sys.platform == "win32" else 0
    try:
        _watchdog_proc = subprocess.Popen(
            [sys.executable, "-m", "sky_music.watchdog"],
            stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=creationflags
        )
    except Exception:
        # If watchdog fails to spawn, we just run without it (fallback)
        return

    def heartbeat():
        while True:
            try:
                proc = _watchdog_proc
                if proc is None or proc.poll() is not None:
                    break
                stdin = proc.stdin
                if stdin:
                    stdin.write(b'\x00')
                    stdin.flush()
            except Exception:
                break
            time.sleep(0.5)

    _watchdog_thread = threading.Thread(target=heartbeat, daemon=True)
    _watchdog_thread.start()

    def _cleanup():
        if _watchdog_proc and _watchdog_proc.stdin:
            with contextlib.suppress(Exception):
                _watchdog_proc.stdin.close()
    atexit.register(_cleanup)


class WinSendInputBackend(_TrackedKeyState):
    """Windows-specific SendInput backend wrapper with safety tracking and panic release."""
    __slots__ = ("inputs_module",)

    def __init__(self):
        super().__init__()
        # Dynamically import inputs to avoid cross-import problems
        from sky_music.platform.win32 import inputs
        self.inputs_module = inputs
        _start_watchdog_once()
        
    def get_health(self) -> BackendHealth:
        diag = self.inputs_module.get_send_diagnostics()
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error,
            min_same_key_up_gap_us=diag.get("min_same_key_up_gap_us"),
            impossible_same_key_repeats=diag.get("impossible_same_key_repeats", 0),
            send_while_unfocused=diag.get("send_while_unfocused", 0)
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        return self.inputs_module.get_send_diagnostics()

    def _emit(
        self, scan_codes: tuple[int, ...], *, key_up: bool
    ) -> tuple[tuple[int, ...], int | None]:
        landed = self.inputs_module.send_scan_code_batch_trusted(scan_codes, key_up=key_up)
        completed_us = time.perf_counter_ns() // 1000
        # Mocks/legacy callables may return None; treat as full success.
        if landed is None:
            return scan_codes, completed_us
        sent_n = max(0, min(int(landed), len(scan_codes)))
        if sent_n >= len(scan_codes):
            return scan_codes, completed_us
        if sent_n <= 0:
            return (), completed_us
        return scan_codes[:sent_n], completed_us

    def _handle_down_error(self, scan_codes: tuple[int, ...], error: Exception) -> None:
        self.last_error = f"key_down error: {error}"
        # Best-effort emergency cleanup in case SendInput partially succeeded.
        try:
            self._emit(scan_codes, key_up=True)
            self.possibly_active_keys.difference_update(scan_codes)
        except Exception as cleanup_error:
            self.last_error = f"key_down emergency cleanup failed: {cleanup_error}"
            # Guard against silent stuck keys: if cleanup also fails the keys are
            # physically stuck but escaped all tracking sets (possibly_active_keys
            # still holds them).  Promote into failed_release_keys so release_all
            # picks them up even if the caller swallows the original exception.
            self.failed_release_keys.update(scan_codes)
            
    def release_all(self) -> ReleaseAllOutcome:
        from sky_music.layouts import PHYSICAL_SCAN_CODES, VK_CODES
        
        # Build inverse map from scan code to VK code for active verification
        scan_to_vk = {}
        for char, sc in PHYSICAL_SCAN_CODES.items():
            vk = VK_CODES.get(char)
            if vk is not None:
                scan_to_vk[sc] = vk
                
        # Form union of all tracked potentially active keys
        to_release = self.active_keys | self.possibly_active_keys | self.failed_release_keys
        if not to_release:
            return ReleaseAllOutcome(
                attempted=(),
                released_successfully=True,
                stuck_keys=(),
                verification_inconclusive=False
            )
            
        release_tuple = tuple(to_release)
        released_successfully = False
        
        # Attempt 3-pass release sequence spaced 15ms apart
        for pass_idx in range(3):
            try:
                self.inputs_module.send_scan_code_batch(release_tuple, key_up=True)
                released_successfully = True
                break
            except Exception as e:
                self.last_error = f"release_all pass {pass_idx} error: {e}"
                if pass_idx == 2:
                    self.failed_release_keys.update(to_release)
                    with contextlib.suppress(Exception):
                        self.inputs_module.debug_log(
                            f"[backend] Panic release failed after 3 passes: {e}. "
                            f"Remaining stuck keys: {self.failed_release_keys}"
                        )
                else:
                    time.sleep(0.015)
                    
        # Active verification and emergency retry with backoff if keys are physically stuck
        stuck_scan_codes = []
        for sc in release_tuple:
            vk = scan_to_vk.get(sc)
            if vk is not None:
                try:
                    if self.inputs_module.is_virtual_key_down(vk):
                        stuck_scan_codes.append(sc)
                except Exception:
                    pass
                    
        if stuck_scan_codes:
            with contextlib.suppress(Exception):
                self.inputs_module.debug_log(
                    f"[backend] Stuck keys detected during best-effort verification: {stuck_scan_codes}. "
                    f"Retrying emergency release with backoff..."
                )
                
            for retry_pass in range(2):
                time.sleep(0.050 * (retry_pass + 1))  # Backoff: 50ms, then 100ms
                try:
                    self.inputs_module.send_scan_code_batch(tuple(stuck_scan_codes), key_up=True)
                    # Verify again
                    still_stuck = []
                    for sc in stuck_scan_codes:
                        vk = scan_to_vk.get(sc)
                        if vk is not None and self.inputs_module.is_virtual_key_down(vk):
                            still_stuck.append(sc)
                    if not still_stuck:
                        with contextlib.suppress(Exception):
                            self.inputs_module.debug_log("[backend] Best-effort verification: Emergency release retries succeeded!")
                        self.active_keys.clear()
                        self.possibly_active_keys.clear()
                        self.failed_release_keys.clear()
                        return ReleaseAllOutcome(
                            attempted=release_tuple,
                            released_successfully=True,
                            stuck_keys=(),
                            verification_inconclusive=False
                        )
                    stuck_scan_codes = still_stuck
                except Exception as e:
                    self.last_error = f"Emergency retry {retry_pass} failed: {e}"
                    
            self.failed_release_keys.update(stuck_scan_codes)
            with contextlib.suppress(Exception):
                self.inputs_module.debug_log(
                    f"[backend] CRITICAL (Best-effort Verification): Keys remain stuck after emergency retry: {stuck_scan_codes}"
                )
            return ReleaseAllOutcome(
                attempted=release_tuple,
                released_successfully=False,
                stuck_keys=tuple(sorted(self.failed_release_keys)),
                verification_inconclusive=False
            )
        if released_successfully:
            # Clear tracking sets upon verified release
            self.active_keys.clear()
            self.possibly_active_keys.clear()
            self.failed_release_keys.clear()
            return ReleaseAllOutcome(
                attempted=release_tuple,
                released_successfully=True,
                stuck_keys=(),
                verification_inconclusive=False
            )
        with contextlib.suppress(Exception):
            self.inputs_module.debug_log(
                f"[backend] Verification inconclusive: send failed but no stuck keys detected via GetAsyncKeyState. "
                f"Keys retained as failed releases: {to_release}"
            )
        return ReleaseAllOutcome(
            attempted=release_tuple,
            released_successfully=False,
            stuck_keys=tuple(sorted(self.failed_release_keys)),
            verification_inconclusive=True
        )


class DryRunBackend(_TrackedKeyState):
    """Mock backend useful for timing analysis, safety state validation, and testing."""
    __slots__ = ("history",)

    def __init__(self):
        super().__init__()
        self.history: list[tuple[str, tuple[int, ...]]] = []  # Records (action_type, scan_codes)
        
    def get_health(self) -> BackendHealth:
        diag = self.get_send_diagnostics()
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error,
            min_same_key_up_gap_us=diag.get("min_same_key_up_gap_us"),
            impossible_same_key_repeats=diag.get("impossible_same_key_repeats", 0),
            send_while_unfocused=diag.get("send_while_unfocused", 0)
        )

    def _emit(
        self, scan_codes: tuple[int, ...], *, key_up: bool
    ) -> tuple[tuple[int, ...], int | None]:
        self.history.append(("up" if key_up else "down", tuple(sorted(scan_codes))))
        return scan_codes, None
            
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
            verification_inconclusive=False
        )
