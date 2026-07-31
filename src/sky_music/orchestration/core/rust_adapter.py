"""Python Adapter Bridge for Rust sky_player_rs core components.

Wraps native Rust structs/functions (sky_player_rs) behind Python core protocol interfaces
(LeadEstimator, InputBackend, etc.) with automatic fallback to pure Python when unavailable.
"""

from __future__ import annotations

from typing import Any

from sky_music.infrastructure.backend import (
    BackendHealth,
    InputBackend,
    InputSendResult,
    ReleaseAllOutcome,
)


def is_rust_dispatch_available() -> bool:
    """Compatibility alias for the single native-dispatch availability check."""
    from sky_music.orchestration.native_dispatch import (
        is_native_dispatch_available,
        python_dispatch_explicitly_requested,
    )

    return not python_dispatch_explicitly_requested() and is_native_dispatch_available()


def reset_rust_availability_cache() -> None:
    from sky_music.orchestration.native_dispatch import (
        reset_native_dispatch_availability_cache,
    )

    reset_native_dispatch_availability_cache()


class RustInputAdapter(InputBackend):
    """Adapter bridging RustInputBackend (from sky_player_rs) to Python InputBackend protocol."""

    __slots__ = ("_native",)

    def __init__(self, mock: bool = False) -> None:
        import sky_player_rs  # type: ignore[import-not-found]

        self._native = sky_player_rs.RustInputBackend(mock=mock)  # type: ignore[attr-defined]

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        res = self._native.key_down(list(scan_codes))
        return InputSendResult(
            sent=tuple(res["sent"]),
            skipped_duplicates=tuple(res["skipped_duplicates"]),
            success=bool(res["success"]),
            error=res["error"],
            send_completed_us=res.get("send_completed_us"),
            first_win32_error=res.get("first_win32_error"),
            last_win32_error=res.get("last_win32_error"),
            send_attempts=int(res.get("send_attempts", 0)),
            zero_progress_retries=int(res.get("zero_progress_retries", 0)),
        )

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        res = self._native.key_up(list(scan_codes))
        return InputSendResult(
            sent=tuple(res["sent"]),
            skipped_duplicates=tuple(res["skipped_duplicates"]),
            success=bool(res["success"]),
            error=res["error"],
            send_completed_us=res.get("send_completed_us"),
            first_win32_error=res.get("first_win32_error"),
            last_win32_error=res.get("last_win32_error"),
            send_attempts=int(res.get("send_attempts", 0)),
            zero_progress_retries=int(res.get("zero_progress_retries", 0)),
        )

    def release_all(self) -> ReleaseAllOutcome:
        out = self._native.release_all()
        return ReleaseAllOutcome(
            attempted=tuple(out["attempted"]),
            released_successfully=bool(out["released_successfully"]),
            stuck_keys=tuple(out["stuck_keys"]),
            verification_inconclusive=bool(out["verification_inconclusive"]),
        )

    def release_all_full_instrument(self) -> ReleaseAllOutcome:
        out = self._native.release_all_full_instrument()
        return ReleaseAllOutcome(
            attempted=tuple(out["attempted"]),
            released_successfully=bool(out["released_successfully"]),
            stuck_keys=tuple(out["stuck_keys"]),
            verification_inconclusive=bool(out["verification_inconclusive"]),
        )

    def get_health(self) -> BackendHealth:
        h = self._native.get_health()
        return BackendHealth(
            active_count=int(h["active_count"]),
            possibly_active_count=int(h["possibly_active_count"]),
            failed_release_count=int(h["failed_release_count"]),
            last_error=h["last_error"],
            keys_dropped=int(h["keys_dropped"]),
            chord_split_events=int(h["chord_split_events"]),
            sendinput_partial_events=int(h.get("sendinput_partial_events", 0)),
            sendinput_zero_progress_failures=int(
                h.get("sendinput_zero_progress_failures", 0)
            ),
            chords_rejected=int(h.get("chords_rejected", 0)),
            authored_conflict_events=int(h.get("authored_conflict_events", 0)),
            authored_chords_rejected=int(h.get("authored_chords_rejected", 0)),
            authored_keys_rejected=int(h.get("authored_keys_rejected", 0)),
            keys_inserted_before_failure=int(h.get("keys_inserted_before_failure", 0)),
            keys_rolled_back=int(h.get("keys_rolled_back", 0)),
            rollback_residue_keys=int(h.get("rollback_residue_keys", 0)),
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        h = self._native.get_health()
        return {
            "keys_dropped": int(h["keys_dropped"]),
            "chord_split_events": int(h["chord_split_events"]),
            "sendinput_partial_events": int(h.get("sendinput_partial_events", 0)),
            "sendinput_zero_progress_failures": int(
                h.get("sendinput_zero_progress_failures", 0)
            ),
            "chords_rejected": int(h.get("chords_rejected", 0)),
            "authored_conflict_events": int(h.get("authored_conflict_events", 0)),
            "authored_chords_rejected": int(h.get("authored_chords_rejected", 0)),
            "authored_keys_rejected": int(h.get("authored_keys_rejected", 0)),
            "keys_inserted_before_failure": int(h.get("keys_inserted_before_failure", 0)),
            "keys_rolled_back": int(h.get("keys_rolled_back", 0)),
            "rollback_residue_keys": int(h.get("rollback_residue_keys", 0)),
        }

    def set_clock(self, clock: Any) -> None:
        pass
