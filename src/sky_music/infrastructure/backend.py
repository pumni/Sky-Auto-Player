from typing import Protocol
from dataclasses import dataclass
import time

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


class _TrackedKeyState:
    active_keys: set[int]
    possibly_active_keys: set[int]
    failed_release_keys: set[int]
    last_error: str | None

    def _decide_down(self, scan_codes: tuple[int, ...]) -> tuple[tuple[int, ...], tuple[int, ...]]:
        unique_scan_codes = tuple(dict.fromkeys(scan_codes))
        duplicates = tuple(sc for sc in unique_scan_codes if sc in self.active_keys)
        to_send = tuple(sc for sc in unique_scan_codes if sc not in self.active_keys)
        return to_send, duplicates

    def _decide_up(self, scan_codes: tuple[int, ...]) -> tuple[tuple[int, ...], tuple[int, ...]]:
        unique_scan_codes = tuple(dict.fromkeys(scan_codes))
        to_release = tuple(
            sc
            for sc in unique_scan_codes
            if sc in self.active_keys or sc in self.possibly_active_keys
        )
        already_released = tuple(
            sc
            for sc in unique_scan_codes
            if sc not in self.active_keys and sc not in self.possibly_active_keys
        )
        return to_release, already_released

    def _emit(self, scan_codes: tuple[int, ...], *, key_up: bool) -> int | None:
        """Returns raw perf_counter µs after send completed, or None if unavailable."""
        raise NotImplementedError

    def _handle_down_error(self, scan_codes: tuple[int, ...], error: Exception) -> None:
        self.last_error = f"key_down error: {error}"

    def _handle_up_error(self, scan_codes: tuple[int, ...], error: Exception) -> None:
        self.failed_release_keys.update(scan_codes)
        self.last_error = f"key_up error: {error}"

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)

        to_send, duplicates = self._decide_down(scan_codes)
        if not to_send:
            return InputSendResult(sent=(), skipped_duplicates=duplicates, success=True)

        self.possibly_active_keys.update(to_send)
        try:
            send_completed_us = self._emit(to_send, key_up=False)
        except Exception as error:
            self._handle_down_error(to_send, error)
            raise

        self.active_keys.update(to_send)
        self.possibly_active_keys.difference_update(to_send)
        return InputSendResult(
            sent=to_send,
            skipped_duplicates=duplicates,
            success=True,
            send_completed_us=send_completed_us,
        )

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)

        to_release, already_released = self._decide_up(scan_codes)
        if not to_release:
            return InputSendResult(sent=(), skipped_duplicates=already_released, success=True)

        try:
            send_completed_us = self._emit(to_release, key_up=True)
        except Exception as error:
            self._handle_up_error(to_release, error)
            raise

        self.active_keys.difference_update(to_release)
        self.possibly_active_keys.difference_update(to_release)
        self.failed_release_keys.difference_update(to_release)
        return InputSendResult(
            sent=to_release,
            skipped_duplicates=already_released,
            success=True,
            send_completed_us=send_completed_us,
        )


class WinSendInputBackend(_TrackedKeyState):
    """Windows-specific SendInput backend wrapper with safety tracking and panic release."""
    def __init__(self):
        # Dynamically import inputs to avoid cross-import problems
        from sky_music.platform.win32 import inputs
        self.inputs_module = inputs
        self._send_fn = getattr(
            inputs, "send_scan_code_batch_trusted", inputs.send_scan_code_batch
        )
        self._send_fn_module = inputs  # track which module _send_fn was resolved from
        self.active_keys = set()
        self.possibly_active_keys = set()
        self.failed_release_keys = set()
        self.last_error: str | None = None
        
    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error
        )

    def _emit(self, scan_codes: tuple[int, ...], *, key_up: bool) -> int | None:
        # Re-resolve send_fn if inputs_module was replaced (e.g. in tests via monkeypatching)
        inputs_module = self.inputs_module
        try:
            cached_module = self._send_fn_module
        except AttributeError:
            cached_module = None

        if cached_module is not inputs_module:
            self._send_fn = getattr(
                inputs_module,
                "send_scan_code_batch_trusted",
                inputs_module.send_scan_code_batch,
            )
            self._send_fn_module = inputs_module
        self._send_fn(scan_codes, key_up=key_up)
        return time.perf_counter_ns() // 1000

    def _handle_down_error(self, scan_codes: tuple[int, ...], error: Exception) -> None:
        self.last_error = f"key_down error: {error}"
        # Best-effort emergency cleanup in case SendInput partially succeeded.
        try:
            self._emit(scan_codes, key_up=True)
            self.possibly_active_keys.difference_update(scan_codes)
        except Exception as cleanup_error:
            # If cleanup fails, keep them in possibly_active_keys to track potential sticking.
            self.last_error = f"key_down emergency cleanup failed: {cleanup_error}"
            
    def release_all(self) -> ReleaseAllOutcome:
        import time
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
                    try:
                        self.inputs_module.debug_log(
                            f"[backend] Panic release failed after 3 passes: {e}. "
                            f"Remaining stuck keys: {self.failed_release_keys}"
                        )
                    except Exception:
                        pass
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
            try:
                self.inputs_module.debug_log(
                    f"[backend] Stuck keys detected during best-effort verification: {stuck_scan_codes}. "
                    f"Retrying emergency release with backoff..."
                )
            except Exception:
                pass
                
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
                        try:
                            self.inputs_module.debug_log("[backend] Best-effort verification: Emergency release retries succeeded!")
                        except Exception:
                            pass
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
            try:
                self.inputs_module.debug_log(
                    f"[backend] CRITICAL (Best-effort Verification): Keys remain stuck after emergency retry: {stuck_scan_codes}"
                )
            except Exception:
                pass
            return ReleaseAllOutcome(
                attempted=release_tuple,
                released_successfully=False,
                stuck_keys=tuple(sorted(self.failed_release_keys)),
                verification_inconclusive=False
            )
        elif released_successfully:
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
        else:
            try:
                self.inputs_module.debug_log(
                    f"[backend] Verification inconclusive: send failed but no stuck keys detected via GetAsyncKeyState. "
                    f"Keys retained as failed releases: {to_release}"
                )
            except Exception:
                pass
            return ReleaseAllOutcome(
                attempted=release_tuple,
                released_successfully=False,
                stuck_keys=tuple(sorted(self.failed_release_keys)),
                verification_inconclusive=True
            )


class DryRunBackend(_TrackedKeyState):
    """Mock backend useful for timing analysis, safety state validation, and testing."""
    def __init__(self):
        self.history = [] # Records tuples of (action_type, scan_codes)
        self.active_keys = set()
        self.possibly_active_keys = set()
        self.failed_release_keys = set()
        self.last_error: str | None = None
        
    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error
        )

    def _emit(self, scan_codes: tuple[int, ...], *, key_up: bool) -> int | None:
        self.history.append(("up" if key_up else "down", tuple(sorted(scan_codes))))
        return None
            
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
