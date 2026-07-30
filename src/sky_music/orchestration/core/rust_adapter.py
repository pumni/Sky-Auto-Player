"""Python Adapter Bridge for Rust sky_player_rs core components.

Wraps native Rust structs/functions (sky_player_rs) behind Python core protocol interfaces
(LeadEstimator, InputBackend, etc.) with automatic fallback to pure Python when unavailable.
"""

from __future__ import annotations

import os
from typing import Any

from sky_music.infrastructure.backend import (
    BackendHealth,
    InputBackend,
    InputSendResult,
    ReleaseAllOutcome,
)

_RUST_AVAILABLE: bool | None = None


def is_rust_dispatch_available() -> bool:
    """Return True if sky_player_rs is installed and free-threaded build is valid."""
    global _RUST_AVAILABLE
    if _RUST_AVAILABLE is not None:
        return _RUST_AVAILABLE

    env_flag = os.environ.get("SKY_USE_RUST_DISPATCH", "1").lower()
    if env_flag in ("0", "false", "off", "no"):
        _RUST_AVAILABLE = False
        return False

    try:
        import sky_player_rs

        info = sky_player_rs.build_info()  # type: ignore[attr-defined]
        _RUST_AVAILABLE = bool(info.get("free_threaded", False))
    except Exception:
        _RUST_AVAILABLE = False

    return _RUST_AVAILABLE


def reset_rust_availability_cache() -> None:
    """Reset cached availability check for unit testing."""
    global _RUST_AVAILABLE
    _RUST_AVAILABLE = None


class RustInputAdapter(InputBackend):
    """Adapter bridging RustInputBackend (from sky_player_rs) to Python InputBackend protocol."""

    __slots__ = ("_native",)

    def __init__(self, mock: bool = False) -> None:
        import sky_player_rs

        self._native = sky_player_rs.RustInputBackend(mock=mock)  # type: ignore[attr-defined]

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        res = self._native.key_down(list(scan_codes))
        return InputSendResult(
            sent=tuple(res["sent"]),
            skipped_duplicates=tuple(res["skipped_duplicates"]),
            success=bool(res["success"]),
            error=res["error"],
            send_completed_us=res.get("send_completed_us"),
        )

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        res = self._native.key_up(list(scan_codes))
        return InputSendResult(
            sent=tuple(res["sent"]),
            skipped_duplicates=tuple(res["skipped_duplicates"]),
            success=bool(res["success"]),
            error=res["error"],
            send_completed_us=res.get("send_completed_us"),
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
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        h = self._native.get_health()
        return {
            "keys_dropped": int(h["keys_dropped"]),
            "chord_split_events": int(h["chord_split_events"]),
        }

    def set_clock(self, clock: Any) -> None:
        pass
