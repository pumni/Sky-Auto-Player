from typing import Protocol
from dataclasses import dataclass

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

class WinSendInputBackend:
    """Windows-specific SendInput backend wrapper with safety tracking and panic release."""
    def __init__(self):
        # Dynamically import inputs to avoid cross-import problems
        from sky_music.platform.win32 import inputs
        self.inputs_module = inputs
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
        
    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)
        unique_scan_codes = tuple(dict.fromkeys(scan_codes))

        # Duplicate-down protection: skip keys that are already in active_keys
        duplicates = tuple(sc for sc in unique_scan_codes if sc in self.active_keys)
        to_send = tuple(sc for sc in unique_scan_codes if sc not in self.active_keys)

        if not to_send:
            # All keys are already held — nothing to do
            return InputSendResult(sent=(), skipped_duplicates=duplicates, success=True)

        # Add targeted keys to possibly_active_keys before injection
        self.possibly_active_keys.update(to_send)

        try:
            self.inputs_module.send_scan_code_batch(to_send, key_up=False)
            # Acknowledged: move to active_keys and clear from possibly_active_keys
            self.active_keys.update(to_send)
            self.possibly_active_keys.difference_update(to_send)
            return InputSendResult(sent=to_send, skipped_duplicates=duplicates, success=True)
        except Exception as e:
            self.last_error = f"key_down error: {e}"
            # Best-effort emergency cleanup in case SendInput partially succeeded.
            try:
                self.inputs_module.send_scan_code_batch(to_send, key_up=True)
                self.possibly_active_keys.difference_update(to_send)
            except Exception as ex:
                # If cleanup fails, we keep them in possibly_active_keys to track potential sticking
                self.last_error = f"key_down emergency cleanup failed: {ex}"
            raise
        
    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)
        unique_scan_codes = tuple(dict.fromkeys(scan_codes))
        to_release = tuple(sc for sc in unique_scan_codes if sc in self.active_keys or sc in self.possibly_active_keys)
        # Idempotent: skip keys that are already not held
        already_released = tuple(sc for sc in unique_scan_codes if sc not in self.active_keys and sc not in self.possibly_active_keys)
        if to_release:
            try:
                self.inputs_module.send_scan_code_batch(to_release, key_up=True)
                self.active_keys.difference_update(to_release)
                self.possibly_active_keys.difference_update(to_release)
                self.failed_release_keys.difference_update(to_release)
                return InputSendResult(sent=to_release, skipped_duplicates=already_released, success=True)
            except Exception as e:
                # Key up failed: transition keys to failed_release_keys
                self.failed_release_keys.update(to_release)
                self.last_error = f"key_up error: {e}"
                raise
        return InputSendResult(sent=(), skipped_duplicates=already_released, success=True)
            
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


class DryRunBackend:
    """Mock backend useful for timing analysis, safety state validation, and testing."""
    def __init__(self):
        self.history = [] # Records tuples of (action_type, scan_codes)
        self.active_keys = set()
        self.possibly_active_keys = set()
        self.failed_release_keys = set()
        
    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=None
        )
        
    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)
        unique_scan_codes = tuple(dict.fromkeys(scan_codes))

        # Duplicate-down protection
        duplicates = tuple(sc for sc in unique_scan_codes if sc in self.active_keys)
        to_send = tuple(sc for sc in unique_scan_codes if sc not in self.active_keys)

        if not to_send:
            return InputSendResult(sent=(), skipped_duplicates=duplicates, success=True)

        self.possibly_active_keys.update(to_send)
        # Simulate success for dry run
        self.active_keys.update(to_send)
        self.possibly_active_keys.difference_update(to_send)
        self.history.append(("down", tuple(sorted(to_send))))
        return InputSendResult(sent=to_send, skipped_duplicates=duplicates, success=True)
        
    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        if not scan_codes:
            return InputSendResult(sent=(), skipped_duplicates=(), success=True)
        unique_scan_codes = tuple(dict.fromkeys(scan_codes))
        to_release = tuple(sc for sc in unique_scan_codes if sc in self.active_keys or sc in self.possibly_active_keys)
        already_released = tuple(sc for sc in unique_scan_codes if sc not in self.active_keys and sc not in self.possibly_active_keys)
        if to_release:
            self.active_keys.difference_update(to_release)
            self.possibly_active_keys.difference_update(to_release)
            self.failed_release_keys.difference_update(to_release)
            self.history.append(("up", tuple(sorted(to_release))))
            return InputSendResult(sent=to_release, skipped_duplicates=already_released, success=True)
        return InputSendResult(sent=(), skipped_duplicates=already_released, success=True)
            
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
