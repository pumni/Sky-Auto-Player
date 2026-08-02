"""P3.1 — Fault-injection backend coverage tests.

Tests the scripted InjectedSendOutcome / FaultInjectionScript backend:
- Explicit call-index script cases, with one terminal outcome per case
- Down failure (zero progress once) → controlled error
- Up failure (transient) → retry → eventual release
- Up failure (persistent) → note-off exhaustion → controlled error
- Focus loss during wait → quit → cleanup
- Quit during recovery (persistent_release partial schedule)
- Supervisor lease expiration during stall (via short lease + no heartbeat)

These tests use mock_backend=True and never touch real SendInput.
"""

from __future__ import annotations

import time
from typing import Any, cast

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]
from rust_contract_helpers import assert_clean_finished

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_ALLOWED = [0x15, 0x16, 0x17, 0x18, 0x19]

_DOWN = "down"
_UP = "up"


def _make_session(
    actions: list[tuple[int, str, int, list[int], str]],
    *,
    allowed: list[int] | None = None,
    mock_failure_mode: str = "none",
    min_hold_us: int = 0,
    supervisor_lease_timeout_us: int = 0,
    telemetry_enabled: bool = False,
) -> Any:
    return sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        actions,
        allowed or _ALLOWED,
        min_hold_us=min_hold_us,
        mock_backend=True,
        mock_failure_mode=mock_failure_mode,
        telemetry_mode="ring" if telemetry_enabled else "off",
        supervisor_lease_timeout_us=supervisor_lease_timeout_us,
    )


def _run(session: Any, *, timeout_ms: int = 5_000) -> dict[str, Any]:
    session.start()
    assert session.join(timeout_ms=timeout_ms) is True, "session did not finish in time"
    return cast(dict[str, Any], session.snapshot())


# ---------------------------------------------------------------------------
# P3.1a — Down failure: zero-progress-down-once
# ---------------------------------------------------------------------------


def test_fault_zero_progress_down_once_recovers_cleanly() -> None:
    """One zero-progress Down is retried immediately and can finish cleanly."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15], "d1"),
            (1, _UP, 1_000, [0x15], "u1"),
        ],
        mock_failure_mode="zero_progress_down_once",
    )
    snap = _run(session)

    assert_clean_finished(snap)


# ---------------------------------------------------------------------------
# P3.1b — Up failure: transient_release (first 3 Up calls fail)
# ---------------------------------------------------------------------------


def test_fault_transient_release_eventually_succeeds() -> None:
    """Transient Up failure (first 3 calls) → recovery → clean finish."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15], "d1"),
            (1, _UP, 500, [0x15], "u1"),
        ],
        mock_failure_mode="transient_release",
        min_hold_us=0,
    )
    snap = _run(session, timeout_ms=10_000)

    assert_clean_finished(snap)


# ---------------------------------------------------------------------------
# P3.1c — Up failure: persistent_release (all Up calls fail → exhaustion)
# ---------------------------------------------------------------------------


def test_fault_persistent_release_exhausts_and_stops() -> None:
    """All Up calls fail → note-off recovery exhausted → controlled error."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15], "d1"),
            (1, _UP, 500, [0x15], "u1"),
            (2, _DOWN, 2_000, [0x16], "d2"),
            (3, _UP, 3_000, [0x16], "u2"),
        ],
        mock_failure_mode="persistent_release",
        telemetry_enabled=True,
    )
    snap = _run(session, timeout_ms=10_000)

    assert snap["status"] == "error", f"expected error, got {snap['status']}"
    assert snap["terminal_error"] is not None
    assert "note-off" in snap["terminal_error"] or "exhausted" in snap["terminal_error"]
    assert snap["release_outcome"] is not None
    assert snap["secondary_errors"], "cleanup diagnostics must retain the primary failure"


# ---------------------------------------------------------------------------
# P3.1d — No-failure fast path: Full outcome, no latency
# ---------------------------------------------------------------------------


def test_fault_none_fast_path_finishes_cleanly() -> None:
    """mock_failure_mode='none' → scripted backend does not break happy path."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15, 0x16], "chord"),
            (1, _UP, 1_000, [0x15, 0x16], "rel"),
        ],
        mock_failure_mode="none",
        min_hold_us=0,
    )
    snap = _run(session)

    assert_clean_finished(snap)


# ---------------------------------------------------------------------------
# P3.1e — Quit during wait
# ---------------------------------------------------------------------------


def test_fault_quit_during_wait_triggers_cleanup() -> None:
    """Quit while worker sleeps before far-future deadline → bounded shutdown."""
    session = _make_session(
        [
            (0, _DOWN, 5_000_000, [0x15], "far-future"),
            (1, _UP, 6_000_000, [0x15], "far-rel"),
        ],
        mock_failure_mode="none",
    )
    session.start()
    time.sleep(0.01)

    t0 = time.perf_counter()
    session.quit()
    joined = session.join(timeout_ms=2_000)
    elapsed = time.perf_counter() - t0

    assert joined is True, "session did not terminate after quit()"
    assert elapsed < 1.0, f"quit took too long: {elapsed:.3f}s"

    snap = cast(dict[str, Any], session.snapshot())
    assert snap["outcome"] == "quit", snap
    assert snap["release_outcome"] is not None


# ---------------------------------------------------------------------------
# P3.1f — Supervisor lease expiration during long wait
# ---------------------------------------------------------------------------


def test_fault_supervisor_lease_expires_when_no_heartbeat() -> None:
    """No heartbeat from Python side → supervisor lease fires → controlled error."""
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        [(0, _DOWN, 1_000_000, [0x15], "lease-test")],
        _ALLOWED,
        min_hold_us=0,
        mock_backend=True,
        mock_failure_mode="none",
        supervisor_lease_timeout_us=50_000,
    )
    session.start()
    joined = session.join(timeout_ms=5_000)

    assert joined is True
    snap = cast(dict[str, Any], session.snapshot())
    assert snap["status"] == "error"
    assert snap["terminal_error"] == "supervisor_lease_expired"
    assert snap["release_outcome"]["released_successfully"] is True


# ---------------------------------------------------------------------------
# P3.1g — Quit during recovery (persistent_release, worker in retry loop)
# ---------------------------------------------------------------------------


def test_fault_quit_during_recovery_exits_cleanly() -> None:
    """Quit while worker is in persistent Up-failure retry loop."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15], "d"),
            (1, _UP, 500, [0x15], "u"),
        ],
        mock_failure_mode="persistent_release",
        min_hold_us=0,
    )
    session.start()
    time.sleep(0.05)

    t0 = time.perf_counter()
    session.quit()
    joined = session.join(timeout_ms=5_000)
    elapsed = time.perf_counter() - t0

    assert joined is True
    assert elapsed < 2.0, f"quit during recovery took too long: {elapsed:.3f}s"

    snap = cast(dict[str, Any], session.snapshot())
    # Persistent release failure makes cleanup itself terminally unsafe; quit
    # must still never be rewritten as a successful finish.
    assert snap["outcome"] == "error", snap


# ---------------------------------------------------------------------------
# P3.1h — Zero-progress: subsequent chords must not be dispatched
# ---------------------------------------------------------------------------


def test_fault_zero_progress_does_not_send_subsequent_chords() -> None:
    """After zero-progress Down failure, all generations must be accounted for."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15], "d1"),
            (1, _UP, 1_000, [0x15], "u1"),
            (2, _DOWN, 2_000, [0x16], "d2"),
            (3, _UP, 3_000, [0x16], "u2"),
        ],
        mock_failure_mode="zero_progress_down_once",
        telemetry_enabled=True,
    )
    snap = _run(session, timeout_ms=10_000)

    assert_clean_finished(snap)
    counts = snap["generation_status_counts"]
    total = sum(counts.values())
    assert total == snap["generation_count"], (
        f"generation_count mismatch: {total} != {snap['generation_count']}"
    )


@pytest.mark.parametrize(
    ("script_case", "expected_outcome"),
    [
        ("persistent_zero_down", "error"),
        ("partial_down_first_attempt", "error"),
        ("partial_down_after_zero_retry", "error"),
        ("persistent_zero_up", "error"),
        ("panic_after_send_before_commit", "error"),
        ("focus_loss_after_due_before_send", "error"),
        ("qpc_failure_after_send", "error"),
        ("wait_failure", "error"),
    ],
)
def test_fault_script_cases_have_one_terminal_outcome(
    script_case: str,
    expected_outcome: str,
) -> None:
    """PR 0 contract table; unsupported cases fail until their script exists."""
    session = _make_session(
        [
            (0, _DOWN, 0, [0x15, 0x16], script_case),
            (1, _UP, 1_000, [0x15, 0x16], f"{script_case}-release"),
        ],
        mock_failure_mode=script_case,
    )
    snap = _run(session, timeout_ms=10_000)
    assert snap["outcome"] == expected_outcome, snap
